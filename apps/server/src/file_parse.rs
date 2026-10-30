//! Reads the content of a Docs page file for the preview panel and the agent. Spreadsheets, PDFs and
//! Word files come from outside and their parsers can use much memory, run for a long time or crash
//! on a crafted file, so all parsing runs in a child process (`orbit parse-file`) without an
//! environment, with memory and CPU limits, and with a time limit that kills it. A crash ends only
//! that one read.

use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use calamine::{DataType, Reader, Xls, Xlsx};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utoipa::ToSchema;

/// Larger files show the name and the download button.
pub const MAX_PARSE_BYTES: u64 = 50 * 1024 * 1024;
/// The panel and the agent edit files up to this size (they send the whole file back).
pub const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
/// How many files one server parses at the same time; the others wait for [`QUEUE_WAIT`].
pub const PARALLEL_PARSES: usize = 2;
pub const QUEUE_WAIT: Duration = Duration::from_secs(10);
const PARSE_TIME: Duration = Duration::from_secs(20);
/// All entries of a zip file (xlsx, docx) together, decompressed: the limit for zip bombs.
const MAX_UNZIPPED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 20_000;
const MAX_CELLS: u64 = 5_000_000;
const MAX_COLUMNS: usize = 1_024;
pub const MAX_ROWS_PER_READ: u64 = 1_000;
pub const MAX_CHARS_PER_READ: u64 = 200_000;
/// One read of rows stops at this size, so the answer stays small whatever the cells hold.
const MAX_ROWS_BYTES: usize = 8 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const CHILD_MEMORY_BYTES: u64 = 1536 * 1024 * 1024;
const CHILD_CPU_SECONDS: u64 = 30;

/// How a file is shown. Taken from the extension and the detected type: the server's detection
/// knows no spreadsheets or text formats.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Sheet,
    Delimited,
    Text,
    Pdf,
    Docx,
    Image,
    Other,
}

impl FileKind {
    /// The panel and the agent can change only these kinds.
    #[must_use]
    pub const fn editable(self) -> bool {
        matches!(self, Self::Delimited | Self::Text)
    }
}

const TEXT_EXTENSIONS: [&str; 28] = [
    "txt", "text", "md", "markdown", "log", "json", "jsonl", "yaml", "yml", "xml", "toml", "ini",
    "cfg", "conf", "sql", "py", "js", "ts", "tsx", "jsx", "css", "sh", "rs", "go", "java", "c",
    "h", "cpp",
];

#[must_use]
pub fn classify(file_name: &str, mime_type: &str) -> FileKind {
    if orbit_platform::UploadService::download_metadata(mime_type, file_name).disposition
        == orbit_platform::ContentDisposition::Inline
    {
        return FileKind::Image;
    }
    // HTML and SVG never open as a page of Orbit's origin, and are not text to edit here.
    if mime_type == "application/pdf" {
        return FileKind::Pdf;
    }
    if mime_type != "application/octet-stream" {
        return FileKind::Other;
    }
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "xlsx" | "xlsm" | "xls" => FileKind::Sheet,
        "csv" | "tsv" => FileKind::Delimited,
        "docx" => FileKind::Docx,
        extension if TEXT_EXTENSIONS.contains(&extension) => FileKind::Text,
        _ => FileKind::Other,
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ParseRequest {
    pub sheet: Option<String>,
    pub offset: u64,
    pub limit: u64,
}

/// One part of a file. `next_offset` is `None` at the end.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Parsed {
    pub sheets: Vec<String>,
    pub sheet: Option<String>,
    pub rows: Option<Vec<Vec<String>>>,
    /// The number of each row in the file, from 1; empty rows of a spreadsheet are left out.
    pub row_numbers: Option<Vec<u64>>,
    pub text: Option<String>,
    pub next_offset: Option<u64>,
    /// A PDF or Word file without text, such as a scan. There is no OCR.
    pub no_text: bool,
    /// False when the bytes are not UTF-8: they show with replacement characters and cannot be
    /// edited.
    pub valid_utf8: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ParseError {
    #[error("the file is over a parse limit")]
    Limit,
    #[error("the file cannot be read")]
    Unreadable,
    #[error("the workbook has no such sheet")]
    UnknownSheet,
}

/// Parses in this process. Only the child process and tests call it.
pub fn parse(kind: FileKind, bytes: &[u8], request: &ParseRequest) -> Result<Parsed, ParseError> {
    match kind {
        FileKind::Sheet => parse_sheet(bytes, request),
        FileKind::Delimited => parse_delimited(bytes, request),
        FileKind::Text => {
            let (text, valid_utf8) = decode(bytes);
            let mut parsed = page_text(&text, request);
            parsed.valid_utf8 = valid_utf8;
            Ok(parsed)
        }
        FileKind::Pdf => {
            let text =
                pdf_extract::extract_text_from_mem(bytes).map_err(|_| ParseError::Unreadable)?;
            Ok(page_text(&text, request))
        }
        FileKind::Docx => Ok(page_text(&docx_text(bytes)?, request)),
        FileKind::Image | FileKind::Other => Ok(Parsed::default()),
    }
}

fn decode(bytes: &[u8]) -> (String, bool) {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => (text.to_owned(), true),
        _ => (String::from_utf8_lossy(bytes).into_owned(), false),
    }
}

fn page_text(text: &str, request: &ParseRequest) -> Parsed {
    let offset = usize::try_from(request.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    let mut characters = text.chars().skip(offset);
    let part: String = characters.by_ref().take(limit).collect();
    let has_more = characters.next().is_some();
    Parsed {
        no_text: text.trim().is_empty(),
        text: Some(part),
        next_offset: has_more.then(|| request.offset + request.limit),
        valid_utf8: true,
        ..Parsed::default()
    }
}

// Delimited text --------------------------------------------------------------------------------

/// How a csv or tsv file is written, so that a save keeps it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelimitedStyle {
    pub delimiter: u8,
    pub crlf: bool,
    pub bom: bool,
}

/// The delimiter with the most uses in the first line outside quotes.
#[must_use]
pub fn delimited_style(bytes: &[u8]) -> DelimitedStyle {
    let bom = bytes.starts_with(b"\xef\xbb\xbf");
    let body = if bom { &bytes[3..] } else { bytes };
    let (mut counts, mut quoted, mut crlf) = ([0_usize; 4], false, false);
    const DELIMITERS: [u8; 4] = [b',', b';', b'\t', b'|'];
    for (index, &byte) in body.iter().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'\n' if !quoted => {
                crlf = index > 0 && body[index - 1] == b'\r';
                break;
            }
            _ if !quoted => {
                if let Some(at) = DELIMITERS.iter().position(|delimiter| *delimiter == byte) {
                    counts[at] += 1;
                }
            }
            _ => {}
        }
    }
    let mut delimiter = b',';
    let mut most = 0;
    for (at, count) in counts.into_iter().enumerate() {
        if count > most {
            most = count;
            delimiter = DELIMITERS[at];
        }
    }
    DelimitedStyle {
        delimiter,
        crlf,
        bom,
    }
}

/// Writes rows in the file's style. The writer quotes a field only when it must: when the field
/// holds the delimiter, a quote or a line end.
pub fn write_delimited(rows: &[Vec<String>], style: DelimitedStyle) -> Result<Vec<u8>, ParseError> {
    let mut output = Vec::new();
    if style.bom {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    let mut writer = csv::WriterBuilder::new()
        .delimiter(style.delimiter)
        .flexible(true)
        .terminator(if style.crlf {
            csv::Terminator::CRLF
        } else {
            csv::Terminator::Any(b'\n')
        })
        .from_writer(&mut output);
    for row in rows {
        writer
            .write_record(row)
            .map_err(|_| ParseError::Unreadable)?;
    }
    writer.flush().map_err(|_| ParseError::Unreadable)?;
    drop(writer);
    Ok(output)
}

fn parse_delimited(bytes: &[u8], request: &ParseRequest) -> Result<Parsed, ParseError> {
    let style = delimited_style(bytes);
    let (text, valid_utf8) = decode(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(style.delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut parsed = Parsed {
        rows: Some(Vec::new()),
        row_numbers: Some(Vec::new()),
        valid_utf8,
        ..Parsed::default()
    };
    let mut size = 0;
    for (index, record) in (0_u64..).zip(reader.records()) {
        if index < request.offset {
            continue;
        }
        let record = record.map_err(|_| ParseError::Unreadable)?;
        if !push_row(
            &mut parsed,
            index,
            record.iter().map(str::to_owned).collect(),
            &mut size,
            request,
        ) {
            break;
        }
    }
    Ok(parsed)
}

/// Adds a row, or sets `next_offset` and returns false when this read is full. `index` is the
/// position in the row list, which is also the offset of the next read.
fn push_row(
    parsed: &mut Parsed,
    index: u64,
    row: Vec<String>,
    size: &mut usize,
    request: &ParseRequest,
) -> bool {
    let rows = parsed.rows.get_or_insert_default();
    if rows.len() as u64 >= request.limit || (*size >= MAX_ROWS_BYTES && !rows.is_empty()) {
        parsed.next_offset = Some(index);
        return false;
    }
    *size += row.iter().map(String::len).sum::<usize>();
    rows.push(row);
    parsed.row_numbers.get_or_insert_default().push(index + 1);
    true
}

// Spreadsheets ----------------------------------------------------------------------------------

fn parse_sheet(bytes: &[u8], request: &ParseRequest) -> Result<Parsed, ParseError> {
    if bytes.starts_with(b"PK") {
        check_zip_limits(bytes)?;
        let mut workbook: Xlsx<_> =
            Xlsx::new(Cursor::new(bytes)).map_err(|_| ParseError::Unreadable)?;
        let (sheets, sheet) = choose_sheet(workbook.sheet_names(), request)?;
        let mut parsed = Parsed {
            sheets,
            sheet: Some(sheet.clone()),
            rows: Some(Vec::new()),
            row_numbers: Some(Vec::new()),
            valid_utf8: true,
            ..Parsed::default()
        };
        // Cell by cell, not `worksheet_range`: that one allocates the whole rectangle between the
        // first and the last cell, which a crafted file makes as large as it wants.
        let mut cells = workbook
            .worksheet_cells_reader(&sheet)
            .map_err(|_| ParseError::Unreadable)?;
        let mut sheet_rows = SheetRows::new(request);
        let mut count = 0;
        while let Some(cell) = cells.next_cell().map_err(|_| ParseError::Unreadable)? {
            count += 1;
            if count > MAX_CELLS {
                return Err(ParseError::Limit);
            }
            let (row, column) = cell.get_position();
            if !sheet_rows.add(&mut parsed, row, column, cell_text(cell.get_value())) {
                return Ok(parsed);
            }
        }
        sheet_rows.finish(&mut parsed);
        Ok(parsed)
    } else {
        // The legacy format has no streaming reader. The child's memory limit bounds it.
        let mut workbook: Xls<_> =
            Xls::new(Cursor::new(bytes)).map_err(|_| ParseError::Unreadable)?;
        let (sheets, sheet) = choose_sheet(workbook.sheet_names(), request)?;
        let range = workbook
            .worksheet_range(&sheet)
            .map_err(|_| ParseError::Unreadable)?;
        let mut parsed = Parsed {
            sheets,
            sheet: Some(sheet),
            rows: Some(Vec::new()),
            row_numbers: Some(Vec::new()),
            valid_utf8: true,
            ..Parsed::default()
        };
        let mut sheet_rows = SheetRows::new(request);
        let mut count = 0;
        for (row, column, value) in range.used_cells() {
            count += 1;
            if count > MAX_CELLS {
                return Err(ParseError::Limit);
            }
            let (row, column) = (
                u32::try_from(row).unwrap_or(u32::MAX),
                u32::try_from(column).unwrap_or(u32::MAX),
            );
            if !sheet_rows.add(&mut parsed, row, column, cell_text(value)) {
                return Ok(parsed);
            }
        }
        sheet_rows.finish(&mut parsed);
        Ok(parsed)
    }
}

fn choose_sheet(
    sheets: Vec<String>,
    request: &ParseRequest,
) -> Result<(Vec<String>, String), ParseError> {
    let sheet = match &request.sheet {
        Some(wanted) if sheets.contains(wanted) => wanted.clone(),
        Some(_) => return Err(ParseError::UnknownSheet),
        None => sheets.first().cloned().ok_or(ParseError::Unreadable)?,
    };
    Ok((sheets, sheet))
}

/// Groups cells, which arrive in row order, into rows. Rows without a value do not count, so
/// `offset` is a position among the rows with values.
struct SheetRows<'a> {
    request: &'a ParseRequest,
    current: Option<(u32, Vec<String>)>,
    /// How many rows with values came before `current`.
    index: u64,
    size: usize,
}

impl<'a> SheetRows<'a> {
    fn new(request: &'a ParseRequest) -> Self {
        Self {
            request,
            current: None,
            index: 0,
            size: 0,
        }
    }

    fn add(&mut self, parsed: &mut Parsed, row: u32, column: u32, value: String) -> bool {
        if value.is_empty() {
            return true;
        }
        if self.current.as_ref().is_some_and(|(at, _)| *at != row) && !self.flush(parsed) {
            return false;
        }
        let (_, cells) = self.current.get_or_insert_with(|| (row, Vec::new()));
        let column = column as usize;
        if column < MAX_COLUMNS {
            if cells.len() <= column {
                cells.resize(column + 1, String::new());
            }
            cells[column] = value;
        }
        true
    }

    fn flush(&mut self, parsed: &mut Parsed) -> bool {
        let Some((row, cells)) = self.current.take() else {
            return true;
        };
        let index = self.index;
        self.index += 1;
        if index < self.request.offset {
            return true;
        }
        if !push_row(parsed, index, cells, &mut self.size, self.request) {
            return false;
        }
        // `push_row` counts rows by position; a spreadsheet shows its real row numbers.
        if let Some(number) = parsed.row_numbers.as_mut().and_then(|rows| rows.last_mut()) {
            *number = u64::from(row) + 1;
        }
        true
    }

    fn finish(&mut self, parsed: &mut Parsed) {
        self.flush(parsed);
    }
}

fn cell_text<T: DataType>(cell: &T) -> String {
    if cell.is_empty() {
        String::new()
    } else if cell.is_datetime() {
        cell.as_datetime()
            .map(|value| {
                if value.time() == chrono::NaiveTime::MIN {
                    value.format("%Y-%m-%d").to_string()
                } else {
                    value.format("%Y-%m-%d %H:%M:%S").to_string()
                }
            })
            .unwrap_or_default()
    } else if let Some(text) = cell.get_string() {
        text.to_owned()
    } else if let Some(value) = cell.get_int() {
        value.to_string()
    } else if let Some(value) = cell.get_float() {
        value.to_string()
    } else if let Some(value) = cell.get_bool() {
        value.to_string().to_uppercase()
    } else if let Some(value) = cell.get_datetime_iso().or_else(|| cell.get_duration_iso()) {
        value.to_owned()
    } else {
        "#ERROR".to_owned()
    }
}

/// Decompresses every entry and counts the real bytes: the sizes a zip declares can be false. A
/// parser that opens the same entries later cannot get more than this.
fn check_zip_limits(bytes: &[u8]) -> Result<(), ParseError> {
    let mut archive = open_zip(bytes)?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(ParseError::Limit);
    }
    let mut left = MAX_UNZIPPED_BYTES;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| ParseError::Unreadable)?;
        let read = std::io::copy(
            &mut Read::by_ref(&mut entry).take(left + 1),
            &mut std::io::sink(),
        )
        .map_err(|_| ParseError::Unreadable)?;
        if read > left {
            return Err(ParseError::Limit);
        }
        left -= read;
    }
    Ok(())
}

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<impl Read + Seek + '_>, ParseError> {
    zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ParseError::Unreadable)
}

// Word ------------------------------------------------------------------------------------------

fn docx_text(bytes: &[u8]) -> Result<String, ParseError> {
    use quick_xml::events::Event;

    check_zip_limits(bytes)?;
    let mut archive = open_zip(bytes)?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| ParseError::Unreadable)?
        .read_to_string(&mut xml)
        .map_err(|_| ParseError::Unreadable)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event().map_err(|_| ParseError::Unreadable)? {
            Event::Start(tag) if tag.local_name().into_inner() == "t" => in_text = true,
            Event::End(tag) => match tag.local_name().into_inner() {
                "t" => in_text = false,
                "p" => text.push('\n'),
                _ => {}
            },
            Event::Empty(tag) => match tag.local_name().into_inner() {
                "tab" => text.push('\t'),
                "br" => text.push('\n'),
                _ => {}
            },
            Event::Text(value) if in_text => text.push_str(&value.xml10_content()),
            Event::GeneralRef(reference) if in_text => {
                text.extend(entity(&reference));
            }
            Event::Eof => return Ok(text),
            _ => {}
        }
    }
}

/// The character of `amp`, `#65` or `#x41`.
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

// The child process -----------------------------------------------------------------------------

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ChildAnswer {
    Parsed(Box<Parsed>),
    Error(ParseError),
}

/// What runs in the child: bytes on stdin, the answer as JSON on stdout.
pub fn run_child(
    kind: FileKind,
    sheet: Option<String>,
    offset: u64,
    limit: u64,
) -> std::io::Result<()> {
    limit_this_process();
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    let request = ParseRequest {
        sheet,
        offset,
        limit,
    };
    let answer = match parse(kind, &bytes, &request) {
        Ok(parsed) => ChildAnswer::Parsed(Box::new(parsed)),
        Err(error) => ChildAnswer::Error(error),
    };
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &answer)?;
    stdout.flush()
}

fn limit_this_process() {
    use rustix::process::{Resource, Rlimit, setrlimit};
    let limit = |value| Rlimit {
        current: Some(value),
        maximum: Some(value),
    };
    // Not supported everywhere (macOS refuses some); the parent's time limit stays.
    let _ = setrlimit(Resource::As, limit(CHILD_MEMORY_BYTES));
    let _ = setrlimit(Resource::Cpu, limit(CHILD_CPU_SECONDS));
    let _ = setrlimit(Resource::Core, limit(0));
}

/// Parses in a child process of `program`. Any failure of the child (killed at a limit, out of
/// memory, a crash) is [`ParseError::Limit`] after the time limit and otherwise
/// [`ParseError::Unreadable`].
pub async fn parse_in_child(
    program: &Path,
    kind: FileKind,
    bytes: Vec<u8>,
    request: &ParseRequest,
) -> Result<Parsed, ParseError> {
    let run = async {
        let kind_argument = clap::ValueEnum::to_possible_value(&kind)
            .map(|value| value.get_name().to_owned())
            .ok_or(ParseError::Unreadable)?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(["parse-file", "--kind", &kind_argument])
            .args(["--offset", &request.offset.to_string()])
            .args(["--limit", &request.limit.to_string()]);
        if let Some(sheet) = &request.sheet {
            command.arg(format!("--sheet={sheet}"));
        }
        let mut child = command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                tracing::warn!(%error, "start file parser");
                ParseError::Unreadable
            })?;
        let mut stdin = child.stdin.take().ok_or(ParseError::Unreadable)?;
        let stdout = child.stdout.take().ok_or(ParseError::Unreadable)?;
        let writer = async move {
            // A child that stops early closes its stdin; the answer below tells what happened.
            let _ = stdin.write_all(&bytes).await;
        };
        let reader = async move {
            let mut output = Vec::new();
            let read = stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut output).await;
            read.map(|_| output)
        };
        let ((), output) = tokio::join!(writer, reader);
        let output = output.map_err(|_| ParseError::Unreadable)?;
        let status = child.wait().await.map_err(|_| ParseError::Unreadable)?;
        if !status.success() {
            return Err(ParseError::Unreadable);
        }
        match serde_json::from_slice(&output) {
            Ok(ChildAnswer::Parsed(parsed)) => Ok(*parsed),
            Ok(ChildAnswer::Error(error)) => Err(error),
            Err(_) => Err(ParseError::Unreadable),
        }
    };
    // Dropping the future kills the child (`kill_on_drop`).
    tokio::time::timeout(PARSE_TIME, run)
        .await
        .unwrap_or(Err(ParseError::Limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(offset: u64, limit: u64) -> ParseRequest {
        ParseRequest {
            sheet: None,
            offset,
            limit,
        }
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut output);
        for (name, bytes) in entries {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        output.into_inner()
    }

    #[test]
    fn a_save_keeps_the_csv_delimiter_quoting_and_line_ends() {
        let original = "\u{feff}name;note\r\n\"a;b\";\"say \"\"hi\"\"\"\r\nplain;\"two\nlines\"\r\n";
        let style = delimited_style(original.as_bytes());
        assert_eq!(
            style,
            DelimitedStyle {
                delimiter: b';',
                crlf: true,
                bom: true
            }
        );
        let parsed = parse(FileKind::Delimited, original.as_bytes(), &request(0, 100)).unwrap();
        let rows = parsed.rows.unwrap();
        assert_eq!(rows[1], ["a;b", "say \"hi\""]);
        assert_eq!(rows[2], ["plain", "two\nlines"]);
        let written = write_delimited(&rows, style).unwrap();
        assert_eq!(String::from_utf8(written).unwrap(), original);
    }

    #[test]
    fn a_tab_in_the_first_line_makes_a_tsv_and_a_quoted_delimiter_does_not_count() {
        assert_eq!(delimited_style(b"a\tb,c\n1\t2\n").delimiter, b'\t');
        assert_eq!(delimited_style(b"\"a;b;c\",d\n").delimiter, b',');
        assert_eq!(delimited_style(b"one\n").delimiter, b',');
    }

    #[test]
    fn rows_and_text_come_in_parts() {
        let csv = "a\nb\nc\n";
        let first = parse(FileKind::Delimited, csv.as_bytes(), &request(0, 2)).unwrap();
        assert_eq!(first.rows.unwrap().len(), 2);
        assert_eq!(first.next_offset, Some(2));
        let last = parse(FileKind::Delimited, csv.as_bytes(), &request(2, 2)).unwrap();
        assert_eq!(last.rows.unwrap(), [["c"]]);
        assert_eq!(last.row_numbers.unwrap(), [3]);
        assert_eq!(last.next_offset, None);

        let text = parse(FileKind::Text, "héllo wörld".as_bytes(), &request(1, 4)).unwrap();
        assert_eq!(text.text.as_deref(), Some("éllo"));
        assert_eq!(text.next_offset, Some(5));
    }

    #[test]
    fn bytes_that_are_not_utf8_show_but_are_marked() {
        let parsed = parse(FileKind::Text, b"caf\xe9", &request(0, 10)).unwrap();
        assert!(!parsed.valid_utf8);
        assert_eq!(parsed.text.as_deref(), Some("caf\u{fffd}"));
    }

    #[test]
    fn a_zip_bomb_stops_at_the_limit() {
        // Over 256 MiB of zeros in about 300 KiB.
        let zeros = vec![0_u8; 300 * 1024 * 1024];
        let bomb = zip_with(&[("xl/workbook.xml", &zeros)]);
        assert!(bomb.len() < 2 * 1024 * 1024);
        assert_eq!(
            parse(FileKind::Sheet, &bomb, &request(0, 10)).unwrap_err(),
            ParseError::Limit
        );
        assert_eq!(
            parse(FileKind::Docx, &bomb, &request(0, 10)).unwrap_err(),
            ParseError::Limit
        );
    }

    #[test]
    fn malformed_files_are_unreadable_not_a_panic() {
        for (kind, bytes) in [
            (FileKind::Sheet, &b"PK\x03\x04 not a zip"[..]),
            (FileKind::Sheet, &b"\xd0\xcf\x11\xe0 not a workbook"[..]),
            (FileKind::Sheet, &b"neither"[..]),
            (FileKind::Docx, &b"PK\x03\x04 not a zip"[..]),
            (FileKind::Pdf, &b"%PDF-1.7 not a pdf"[..]),
        ] {
            assert_eq!(
                parse(kind, bytes, &request(0, 10)).unwrap_err(),
                ParseError::Unreadable,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn docx_text_keeps_paragraphs_and_entities() {
        let document = br#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Fish &amp; chips</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t><w:tab/><w:t>&#65;</w:t></w:r></w:p></w:body></w:document>"#;
        let file = zip_with(&[("word/document.xml", document)]);
        let parsed = parse(FileKind::Docx, &file, &request(0, 100)).unwrap();
        assert_eq!(parsed.text.as_deref(), Some("Fish & chips\nSecond\tA\n"));
    }

    #[test]
    fn the_kind_comes_from_the_detected_type_and_the_extension() {
        let octets = "application/octet-stream";
        assert_eq!(classify("a.XLSX", octets), FileKind::Sheet);
        assert_eq!(classify("a.xls", octets), FileKind::Sheet);
        assert_eq!(classify("a.csv", octets), FileKind::Delimited);
        assert_eq!(classify("a.tsv", octets), FileKind::Delimited);
        assert_eq!(classify("a.txt", octets), FileKind::Text);
        assert_eq!(classify("a.exe", octets), FileKind::Other);
        assert_eq!(classify("a.pdf", "application/pdf"), FileKind::Pdf);
        // A false extension does not make a PDF, and an uploaded page stays out of reach.
        assert_eq!(classify("a.pdf", octets), FileKind::Other);
        assert_eq!(classify("a.txt", "text/html"), FileKind::Other);
        assert_eq!(classify("a.png", "image/svg+xml"), FileKind::Other);
        assert_eq!(classify("a.bin", "image/png"), FileKind::Image);
    }
}
