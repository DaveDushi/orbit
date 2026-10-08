//! An in-memory S3 server for tests: path-style URLs and only the requests Orbit sends. It checks that requests
//! are signed, not the signature itself (the signer has its own test against AWS's example).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;

use super::S3Config;

/// Starts the server on a free local port and returns a bucket configuration for it (access key `key`).
pub async fn start_fake_s3() -> S3Config {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake S3");
    let address = listener.local_addr().expect("fake S3 address");
    let router = Router::new()
        .fallback(fake_s3)
        .with_state(Objects::default());
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

type Objects = Arc<Mutex<BTreeMap<String, (Vec<u8>, chrono::DateTime<Utc>)>>>;

async fn fake_s3(State(objects): State<Objects>, request: Request) -> Response {
    let method = request.method().clone();
    let headers = request.headers().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    if !headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 Credential=key/"))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let key = path
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, key)| key.to_owned())
        .unwrap_or_default();
    let mut objects = objects
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match method {
        Method::GET if query.contains("list-type=2") => {
            let prefix = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("prefix="))
                .map(|value| value.replace("%2F", "/"))
                .unwrap_or_default();
            let contents: String = objects
                .iter()
                .filter(|(name, _)| name.starts_with(&prefix))
                .map(|(name, (bytes, modified))| {
                    format!(
                        "<Contents><Key>{name}</Key><LastModified>{}</LastModified><Size>{}</Size></Contents>",
                        modified.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        bytes.len()
                    )
                })
                .collect();
            format!(
                "<ListBucketResult><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>"
            )
            .into_response()
        }
        Method::PUT if headers.contains_key("x-amz-copy-source") => match objects.get_mut(&key) {
            Some(object) => {
                object.1 = Utc::now();
                "<CopyObjectResult/>".into_response()
            }
            None => no_such_key(),
        },
        Method::PUT => {
            objects.insert(key, (body.to_vec(), Utc::now()));
            StatusCode::OK.into_response()
        }
        Method::GET | Method::HEAD => match objects.get(&key) {
            Some((bytes, modified)) => {
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::LAST_MODIFIED,
                    modified.to_rfc2822().parse().expect("valid header"),
                );
                (headers, Bytes::from(bytes.clone())).into_response()
            }
            None => no_such_key(),
        },
        Method::DELETE => {
            objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn no_such_key() -> Response {
    (
        StatusCode::NOT_FOUND,
        "<Error><Code>NoSuchKey</Code><Message>missing</Message></Error>",
    )
        .into_response()
}
