//! One part of a stored file as rows or text, for the preview panel and the agent. Docs page
//! files and chat files are read the same way: only their access rules differ.

use std::path::PathBuf;
use std::sync::LazyLock;

use axum::extract::Extension;
use axum::http::StatusCode;
use orbit_platform::{RequestId, UploadError};
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::file_parse::{self, FileKind, ParseError, ParseRequest, Parsed};
use crate::task_routes::ApiError;

/// Per server, like every limit of one process: page and chat files share it.
static PARSES: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(file_parse::PARALLEL_PARSES));

/// Which part of a file to read. `offset` and `limit` count rows for spreadsheets and delimited
/// text, and characters otherwise. PDF text is read only with `extract`: the panel shows the pages
/// themselves.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ContentQuery {
    sheet: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
    extract: Option<bool>,
}

#[derive(Debug)]
pub(crate) enum ReadError {
    Parse(ParseError),
    Busy,
    Internal,
    Upload(UploadError),
}

/// Parses files in a child process of `parser`.
#[derive(Clone)]
pub(crate) struct FileReader {
    /// The program that parses in a child process: this one (`orbit parse-file`).
    parser: Option<PathBuf>,
}

impl FileReader {
    pub(crate) fn new() -> Self {
        Self {
            parser: std::env::current_exe().ok(),
        }
    }

    /// Tests run in a test binary, which cannot be the parser.
    pub(crate) fn with_parser(program: impl Into<PathBuf>) -> Self {
        Self {
            parser: Some(program.into()),
        }
    }

    pub(crate) fn parser(&self) -> Result<&std::path::Path, ReadError> {
        self.parser.as_deref().ok_or(ReadError::Internal)
    }

    /// A place in the parse queue, for a parse or a patch.
    pub(crate) async fn permit(&self) -> Result<SemaphorePermit<'static>, ReadError> {
        tokio::time::timeout(file_parse::QUEUE_WAIT, PARSES.acquire())
            .await
            .map_err(|_| ReadError::Busy)?
            .map_err(|_| ReadError::Internal)
    }

    /// One part of a file; `None` for a kind that is not read here (images, other files, and PDF
    /// without `extract`). `bytes` loads the file only when it is read.
    pub(crate) async fn read(
        &self,
        kind: FileKind,
        file_name: &str,
        size_bytes: u64,
        query: ContentQuery,
        bytes: impl Future<Output = Result<Vec<u8>, UploadError>>,
    ) -> Result<Option<Parsed>, ReadError> {
        let parses = match kind {
            FileKind::Image | FileKind::Other => false,
            FileKind::Pdf => query.extract.unwrap_or(false),
            _ => true,
        };
        if !parses {
            return Ok(None);
        }
        if size_bytes > file_parse::MAX_PARSE_BYTES {
            return Err(ReadError::Parse(ParseError::Limit));
        }
        let rows = matches!(
            kind,
            FileKind::Sheet | FileKind::LegacySheet | FileKind::Delimited
        );
        let request = ParseRequest {
            tab_separated: is_tsv(file_name),
            sheet: query.sheet,
            offset: query.offset.unwrap_or(0),
            limit: query
                .limit
                .unwrap_or(if rows { 200 } else { 50_000 })
                .clamp(
                    1,
                    if rows {
                        file_parse::MAX_ROWS_PER_READ
                    } else {
                        file_parse::MAX_CHARS_PER_READ
                    },
                ),
        };
        let _permit = self.permit().await?;
        let bytes = bytes.await.map_err(ReadError::Upload)?;
        file_parse::parse_in_child(self.parser()?, kind, bytes, &request)
            .await
            .map(Some)
            .map_err(ReadError::Parse)
    }
}

pub(crate) fn is_tsv(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".tsv")
}

/// The answer to a [`ReadError`] other than [`ReadError::Upload`], which each caller maps with
/// its own upload errors.
pub(crate) fn read_problem(
    error: ReadError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    match error {
        ReadError::Parse(error) => parse_problem(error, instance, request_id),
        ReadError::Busy => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "file_busy",
            "Try again",
            "The server is reading other files. Try again in a moment.",
            instance,
            request_id,
        ),
        ReadError::Internal | ReadError::Upload(_) => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
            "An unexpected error occurred. Use the request ID when contacting support.",
            instance,
            request_id,
        ),
    }
}

fn parse_problem(
    error: ParseError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    let (code, title, detail) = match error {
        ParseError::Limit => (
            "file_over_limit",
            "File too large to read",
            "This file is too large or too complex to read here. Download it to open it.",
        ),
        ParseError::Unreadable => (
            "file_unreadable",
            "File cannot be read",
            "This file is damaged or protected, or its format is not supported. Download it to open it.",
        ),
        ParseError::UnknownSheet => (
            "file_unreadable",
            "No such sheet",
            "The workbook has no sheet with this name.",
        ),
    };
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        code,
        title,
        detail,
        instance,
        request_id,
    )
}
