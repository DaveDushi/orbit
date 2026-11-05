//! An in-memory S3 server for tests: path-style URLs and only the requests Orbit sends. It checks that requests
//! are signed (by header or presigned query), not the signature itself (the signer has its own tests against
//! AWS's examples). Presigned URLs are refused after they expire, and multipart uploads follow S3's rules on
//! part order, ETags and the smallest part size, so part bookkeeping bugs show up in tests. Every response
//! allows any origin and exposes `ETag`, like the CORS rules a browser upload needs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::S3Config;
use super::s3::S3_MIN_PART_BYTES;

/// Starts the server on a free local port and returns a bucket configuration for it (access key `key`).
pub async fn start_fake_s3() -> S3Config {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake S3");
    let address = listener.local_addr().expect("fake S3 address");
    let router = Router::new()
        .fallback(fake_s3)
        .with_state(Arc::new(Mutex::new(Bucket::default())));
    tokio::spawn(async move { axum::serve(listener, router).await });
    S3Config {
        endpoint: format!("http://{address}"),
        region: "us-east-1".to_owned(),
        bucket: "orbit".to_owned(),
        prefix: String::new(),
        access_key_id: "key".to_owned(),
        secret_access_key: "secret".to_owned(),
        path_style: true,
    }
}

#[derive(Default)]
struct Bucket {
    objects: BTreeMap<String, Object>,
    uploads: BTreeMap<String, Upload>,
}

struct Object {
    bytes: Vec<u8>,
    modified: DateTime<Utc>,
}

struct Upload {
    key: String,
    /// Part number → (ETag, bytes).
    parts: BTreeMap<u32, (String, Vec<u8>)>,
}

type Shared = Arc<Mutex<Bucket>>;

async fn fake_s3(State(bucket): State<Shared>, request: Request) -> Response {
    let mut response = if request.method() == Method::OPTIONS {
        preflight()
    } else {
        handle(&bucket, request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("ETag"),
    );
    response
}

fn preflight() -> Response {
    (
        [
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, PUT, HEAD"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "*"),
            (header::ACCESS_CONTROL_MAX_AGE, "3600"),
        ],
        StatusCode::OK,
    )
        .into_response()
}

async fn handle(bucket: &Shared, request: Request) -> Response {
    let method = request.method().clone();
    let headers = request.headers().clone();
    let path = request.uri().path().to_owned();
    let query: BTreeMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    if !authorized(&headers, &query) {
        return error(StatusCode::FORBIDDEN, "AccessDenied");
    }
    let key = path
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, key)| percent_decode(key))
        .unwrap_or_default();
    let mut bucket = bucket
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let upload_id = query.get("uploadId").cloned();
    match (method, upload_id) {
        (Method::POST, None) if query.contains_key("uploads") => {
            let upload_id = Uuid::now_v7().to_string();
            bucket.uploads.insert(
                upload_id.clone(),
                Upload {
                    key,
                    parts: BTreeMap::new(),
                },
            );
            format!(
                "<InitiateMultipartUploadResult><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>"
            )
            .into_response()
        }
        (Method::PUT, Some(upload_id)) => {
            let Some(upload) = upload_of(&mut bucket, &upload_id, &key) else {
                return no_such_upload();
            };
            let Some(number) = query
                .get("partNumber")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|number| (1..=10_000).contains(number))
            else {
                return error(StatusCode::BAD_REQUEST, "InvalidArgument");
            };
            let etag = etag(&body);
            upload.parts.insert(number, (etag.clone(), body.to_vec()));
            ([(header::ETAG, etag)], StatusCode::OK).into_response()
        }
        (Method::GET, Some(upload_id)) => {
            let Some(upload) = upload_of(&mut bucket, &upload_id, &key) else {
                return no_such_upload();
            };
            let parts: String = upload
                .parts
                .iter()
                .map(|(number, (etag, bytes))| {
                    format!(
                        "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag><Size>{}</Size></Part>",
                        etag.replace('"', "&quot;"),
                        bytes.len()
                    )
                })
                .collect();
            format!("<ListPartsResult><IsTruncated>false</IsTruncated>{parts}</ListPartsResult>")
                .into_response()
        }
        (Method::POST, Some(upload_id)) => {
            let Some(upload) = upload_of(&mut bucket, &upload_id, &key) else {
                return no_such_upload();
            };
            let bytes = match join_parts(upload, &String::from_utf8_lossy(&body)) {
                Ok(bytes) => bytes,
                Err(code) => return error(StatusCode::BAD_REQUEST, code),
            };
            bucket.uploads.remove(&upload_id);
            let etag = etag(&bytes);
            bucket.objects.insert(
                key,
                Object {
                    bytes,
                    modified: Utc::now(),
                },
            );
            format!(
                "<CompleteMultipartUploadResult><ETag>{}</ETag></CompleteMultipartUploadResult>",
                etag.replace('"', "&quot;")
            )
            .into_response()
        }
        (Method::DELETE, Some(upload_id)) => {
            if upload_of(&mut bucket, &upload_id, &key).is_none() {
                return no_such_upload();
            }
            bucket.uploads.remove(&upload_id);
            StatusCode::NO_CONTENT.into_response()
        }
        (Method::GET, None) if query.get("list-type").is_some_and(|value| value == "2") => {
            let prefix = query.get("prefix").cloned().unwrap_or_default();
            let contents: String = bucket
                .objects
                .iter()
                .filter(|(name, _)| name.starts_with(&prefix))
                .map(|(name, object)| {
                    format!(
                        "<Contents><Key>{name}</Key><LastModified>{}</LastModified><Size>{}</Size></Contents>",
                        object
                            .modified
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        object.bytes.len()
                    )
                })
                .collect();
            format!(
                "<ListBucketResult><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>"
            )
            .into_response()
        }
        (Method::PUT, None) if headers.contains_key("x-amz-copy-source") => {
            let source = headers
                .get("x-amz-copy-source")
                .and_then(|value| value.to_str().ok())
                .map(|value| percent_decode(value.trim_start_matches('/')))
                .and_then(|value| value.split_once('/').map(|(_, key)| key.to_owned()))
                .unwrap_or_default();
            let Some(bytes) = bucket
                .objects
                .get(&source)
                .map(|object| object.bytes.clone())
            else {
                return no_such_key();
            };
            bucket.objects.insert(
                key,
                Object {
                    bytes,
                    modified: Utc::now(),
                },
            );
            "<CopyObjectResult/>".into_response()
        }
        (Method::PUT, None) => {
            let etag = etag(&body);
            bucket.objects.insert(
                key,
                Object {
                    bytes: body.to_vec(),
                    modified: Utc::now(),
                },
            );
            ([(header::ETAG, etag)], StatusCode::OK).into_response()
        }
        (method @ (Method::GET | Method::HEAD), None) => match bucket.objects.get(&key) {
            Some(object) => {
                let mut response_headers = HeaderMap::new();
                response_headers.insert(
                    header::LAST_MODIFIED,
                    object.modified.to_rfc2822().parse().expect("valid header"),
                );
                if let Some(value) = query
                    .get("response-content-disposition")
                    .and_then(|value| HeaderValue::from_str(value).ok())
                {
                    response_headers.insert(header::CONTENT_DISPOSITION, value);
                }
                if method == Method::HEAD {
                    response_headers.insert(header::CONTENT_LENGTH, object.bytes.len().into());
                    return (response_headers, StatusCode::OK).into_response();
                }
                (response_headers, Bytes::from(object.bytes.clone())).into_response()
            }
            None => no_such_key(),
        },
        (Method::DELETE, None) => {
            bucket.objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// A signed `Authorization` header, or a presigned query that has not expired.
fn authorized(headers: &HeaderMap, query: &BTreeMap<String, String>) -> bool {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 Credential=key/"))
    {
        return true;
    }
    let presigned = query
        .get("X-Amz-Credential")
        .is_some_and(|value| value.starts_with("key/"))
        && query.contains_key("X-Amz-Signature");
    if !presigned {
        return false;
    }
    let signed_at = query
        .get("X-Amz-Date")
        .and_then(|value| NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ").ok())
        .map(|time| time.and_utc());
    let lifetime = query
        .get("X-Amz-Expires")
        .and_then(|value| value.parse::<i64>().ok());
    matches!((signed_at, lifetime), (Some(signed_at), Some(lifetime))
        if Utc::now() <= signed_at + chrono::Duration::seconds(lifetime))
}

/// The upload, when it exists and belongs to `key`.
fn upload_of<'a>(bucket: &'a mut Bucket, upload_id: &str, key: &str) -> Option<&'a mut Upload> {
    bucket
        .uploads
        .get_mut(upload_id)
        .filter(|upload| upload.key == key)
}

/// The object's bytes from the parts that a CompleteMultipartUpload body lists, checked as S3 does; the S3
/// error code otherwise.
fn join_parts(upload: &Upload, body: &str) -> Result<Vec<u8>, &'static str> {
    let listed: Vec<(u32, String)> = body
        .split("<Part>")
        .skip(1)
        .map(|part| {
            let value = |name: &str| {
                part.split_once(&format!("<{name}>"))
                    .and_then(|(_, rest)| rest.split_once(&format!("</{name}>")))
                    .map(|(value, _)| value.replace("&quot;", "\"").replace("&amp;", "&"))
                    .unwrap_or_default()
            };
            (value("PartNumber").parse().unwrap_or(0), value("ETag"))
        })
        .collect();
    if listed.is_empty() {
        return Err("MalformedXML");
    }
    if listed.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err("InvalidPartOrder");
    }
    let mut bytes = Vec::new();
    for (index, (number, etag)) in listed.iter().enumerate() {
        let Some((stored_etag, part)) = upload.parts.get(number) else {
            return Err("InvalidPart");
        };
        if stored_etag.trim_matches('"') != etag.trim_matches('"') {
            return Err("InvalidPart");
        }
        if index + 1 < listed.len() && (part.len() as u64) < S3_MIN_PART_BYTES {
            return Err("EntityTooSmall");
        }
        bytes.extend_from_slice(part);
    }
    Ok(bytes)
}

fn etag(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("\"{hex}\"")
}

fn percent_decode(value: &str) -> String {
    url::form_urlencoded::parse(format!("v={}", value.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

fn error(status: StatusCode, code: &str) -> Response {
    (
        status,
        format!("<Error><Code>{code}</Code><Message>{code}</Message></Error>"),
    )
        .into_response()
}

fn no_such_key() -> Response {
    error(StatusCode::NOT_FOUND, "NoSuchKey")
}

fn no_such_upload() -> Response {
    error(StatusCode::NOT_FOUND, "NoSuchUpload")
}
