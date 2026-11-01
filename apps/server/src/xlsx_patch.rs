//! Edits cells of an xlsx workbook without rewriting it: only the edited `<c>` elements of the sheet
//! XML change, `workbook.xml` gets `fullCalcOnLoad`, and every other zip entry is copied with its
//! compressed bytes unchanged, so formats, charts, macros and everything Orbit does not know stay.
//! Runs in the limited child process of `file_parse`, because the input comes from outside.

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Write};

use calamine::{DataType, Reader, Xlsx};
use chrono::{NaiveDate, NaiveDateTime};
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::file_parse::{ParseError, check_zip_limits};

pub const MAX_EDITS: usize = 5_000;
const MAX_CELL_CHARS: usize = 32_767;
const MAX_ROW: u32 = 1_048_576;
const MAX_COLUMN: u32 = 16_383;
/// Elements of `CT_Workbook` that come after `calcPr`.
const AFTER_CALC_PR: [&str; 9] = [
    "oleSize",
    "customWorkbookViews",
    "pivotCaches",
    "smartTagPr",
    "smartTagTypes",
    "webPublishing",
    "fileRecoveryPr",
    "webPublishObjects",
    "extLst",
];

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct CellEdit {
    /// The cell in A1 style, for example `B2`.
    pub cell: String,
    /// The new value. Empty clears the cell and keeps its format.
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PatchRequest {
    pub sheet: Option<String>,
    pub edits: Vec<CellEdit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum PatchError {
    #[error("cell {0} holds a formula, and formula cells are read-only")]
    Formula(String),
    #[error("cell {0} is a table header, which cannot change here")]
    TableHeader(String),
    #[error("sheet \"{0}\" is protected")]
    Protected(String),
    #[error("\"{0}\" is not a cell")]
    InvalidCell(String),
    #[error("the value for cell {0} is too long or has characters a workbook cannot hold")]
    InvalidValue(String),
    #[error("too many cells in one save")]
    TooManyEdits,
    #[error("the workbook has no such sheet")]
    UnknownSheet,
    #[error("the file is over a limit")]
    Limit,
    #[error("the file cannot be changed: it is damaged, or not an xlsx file")]
    Unreadable,
}

impl From<ParseError> for PatchError {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::Limit => Self::Limit,
            ParseError::UnknownSheet => Self::UnknownSheet,
            ParseError::Unreadable => Self::Unreadable,
        }
    }
}

impl From<quick_xml::Error> for PatchError {
    fn from(_: quick_xml::Error) -> Self {
        Self::Unreadable
    }
}

impl From<zip::result::ZipError> for PatchError {
    fn from(_: zip::result::ZipError) -> Self {
        Self::Unreadable
    }
}

impl From<std::io::Error> for PatchError {
    fn from(_: std::io::Error) -> Self {
        Self::Unreadable
    }
}

/// `B2` as a 1-based row and a 0-based column.
#[must_use]
pub fn parse_cell(cell: &str) -> Option<(u32, u32)> {
    let digits = cell.find(|c: char| c.is_ascii_digit())?;
    let (letters, number) = cell.split_at(digits);
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let row: u32 = number.parse().ok()?;
    let column = letters
        .bytes()
        .fold(0_u32, |sum, b| sum * 26 + u32::from(b - b'A' + 1))
        - 1;
    ((1..=MAX_ROW).contains(&row) && column <= MAX_COLUMN && !number.starts_with('0'))
        .then_some((row, column))
}

/// `(row, column)` as `B2`.
#[must_use]
pub fn cell_name(row: u32, column: u32) -> String {
    let mut letters = Vec::new();
    let mut left = column + 1;
    while left > 0 {
        letters.push(b'A' + u8::try_from((left - 1) % 26).unwrap_or(0));
        left = (left - 1) / 26;
    }
    letters.reverse();
    format!("{}{row}", String::from_utf8_lossy(&letters))
}

type Range = ((u32, u32), (u32, u32));

fn parse_range(range: &str) -> Option<Range> {
    let range = range.replace('$', "");
    match range.split_once(':') {
        Some((from, to)) => Some((parse_cell(from)?, parse_cell(to)?)),
        None => parse_cell(&range).map(|cell| (cell, cell)),
    }
}

fn in_range(((top, left), (bottom, right)): Range, (row, column): (u32, u32)) -> bool {
    (top..=bottom).contains(&row) && (left..=right).contains(&column)
}

fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|value| {
            quick_xml::escape::unescape(&value.value)
                .ok()
                .map(|text| text.into_owned())
        })
}

fn is_true(value: Option<String>) -> bool {
    matches!(value.as_deref(), Some("1" | "true"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn prefix_of(tag: &BytesStart<'_>) -> String {
    let name = tag.name().into_inner();
    name.split_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default()
}

fn valid_value(value: &str) -> bool {
    value.chars().count() <= MAX_CELL_CHARS
        && value.chars().all(|c| {
            (c >= ' ' || matches!(c, '\t' | '\n' | '\r')) && !matches!(c, '\u{fffe}' | '\u{ffff}')
        })
}

/// A number as a spreadsheet writes it. `007` and `+1` stay text.
fn is_number(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let mantissa = digits.split(['e', 'E']).next().unwrap_or("");
    let whole = mantissa.split('.').next().unwrap_or("");
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'))
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'))
        && value.parse::<f64>().is_ok_and(f64::is_finite)
}

/// `2026-10-30` or `2026-10-30 14:00:00` as the serial number of a date cell.
fn date_serial(value: &str, date1904: bool) -> Option<f64> {
    let moment = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
        })?;
    let (year, month, day) = if date1904 {
        (1904, 1, 1)
    } else {
        (1899, 12, 30)
    };
    let epoch = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(0, 0, 0)?;
    let seconds = moment.signed_duration_since(epoch).num_seconds();
    // Before March 1900 the serials of the 1900 system are off by Excel's leap year bug.
    (seconds >= 61 * 86_400).then_some(seconds as f64 / 86_400.0)
}

struct SheetContext {
    prefix: String,
    /// Cells of date format among the edited ones; only they turn a date text into a serial.
    dates: HashSet<(u32, u32)>,
    date1904: bool,
}

/// The new element of a cell. `old_type` is its `t`, and `style` its `s`.
fn cell_xml(
    context: &SheetContext,
    position: (u32, u32),
    style: Option<&str>,
    old_type: Option<&str>,
    value: &str,
) -> String {
    let c = format!("{}c", context.prefix);
    let v = format!("{}v", context.prefix);
    let name = cell_name(position.0, position.1);
    let style = style.map_or(String::new(), |style| format!(" s=\"{}\"", escape(style)));
    if value.is_empty() {
        return if style.is_empty() {
            String::new()
        } else {
            format!("<{c} r=\"{name}\"{style}/>")
        };
    }
    let was_text = matches!(old_type, Some("s" | "str" | "inlineStr"));
    if old_type == Some("b") && matches!(value, "TRUE" | "FALSE") {
        let bit = u8::from(value == "TRUE");
        return format!("<{c} r=\"{name}\"{style} t=\"b\"><{v}>{bit}</{v}></{c}>");
    }
    if context.dates.contains(&position)
        && let Some(serial) = date_serial(value, context.date1904)
    {
        return format!("<{c} r=\"{name}\"{style}><{v}>{serial}</{v}></{c}>");
    }
    if !was_text && is_number(value) {
        return format!("<{c} r=\"{name}\"{style}><{v}>{value}</{v}></{c}>");
    }
    let is = format!("{}is", context.prefix);
    let t = format!("{}t", context.prefix);
    format!(
        "<{c} r=\"{name}\"{style} t=\"inlineStr\"><{is}><{t} xml:space=\"preserve\">{}</{t}></{is}></{c}>",
        escape(value)
    )
}

struct OpenCell {
    position: (u32, u32),
    start: usize,
    style: Option<String>,
    old_type: Option<String>,
    has_formula: bool,
}

fn take_row_before(
    edits: &mut BTreeMap<(u32, u32), String>,
    row: u32,
    before_column: u32,
) -> Vec<((u32, u32), String)> {
    let keys: Vec<_> = edits
        .range((row, 0)..(row, before_column))
        .map(|(key, _)| *key)
        .collect();
    keys.into_iter()
        .filter_map(|key| edits.remove(&key).map(|value| (key, value)))
        .collect()
}

fn new_cells(context: &SheetContext, cells: &[((u32, u32), String)]) -> String {
    cells
        .iter()
        .map(|(position, value)| cell_xml(context, *position, None, None, value))
        .collect()
}

/// New `<row>` elements for the edits in rows before `before_row`.
fn take_rows_before(
    context: &SheetContext,
    edits: &mut BTreeMap<(u32, u32), String>,
    before_row: u32,
) -> String {
    let mut output = String::new();
    while let Some((&(row, _), _)) = edits.range(..(before_row, 0)).next() {
        let cells = new_cells(context, &take_row_before(edits, row, u32::MAX));
        if !cells.is_empty() {
            let name = format!("{}row", context.prefix);
            output.push_str(&format!("<{name} r=\"{row}\">{cells}</{name}>"));
        }
    }
    output
}

/// The opening tag of a row without its `spans`, which an added cell can make false. It is only a
/// hint for readers.
fn without_spans(tag: &str) -> String {
    let Some(at) = tag.find(" spans=") else {
        return tag.to_owned();
    };
    let value = at + " spans=".len();
    let Some(quote) = tag[value..].chars().next() else {
        return tag.to_owned();
    };
    let Some(close) = tag[value + 1..].find(quote) else {
        return tag.to_owned();
    };
    format!("{}{}", &tag[..at], &tag[value + 1 + close + 1..])
}

fn apply(xml: &str, mut splices: Vec<(usize, usize, String)>) -> String {
    splices.sort_by_key(|(start, end, _)| (*start, *end));
    let mut output = String::with_capacity(xml.len() + 256);
    let mut at = 0;
    for (start, end, text) in splices {
        output.push_str(&xml[at..start]);
        output.push_str(&text);
        at = end;
    }
    output.push_str(&xml[at..]);
    output
}

fn patch_sheet(
    xml: &str,
    edits: &BTreeMap<(u32, u32), String>,
    sheet: &str,
    dates: HashSet<(u32, u32)>,
    date1904: bool,
) -> Result<String, PatchError> {
    let mut context = SheetContext {
        prefix: String::new(),
        dates,
        date1904,
    };
    let mut pending = edits.clone();
    let mut splices = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_data = false;
    let mut found_data = false;
    let mut row = 0_u32;
    let mut column = 0_u32;
    let mut in_row = false;
    let mut open: Option<OpenCell> = None;
    let mut formula_ranges = Vec::new();
    let mut protected = false;
    let mut dimension: Option<(usize, usize, Option<Range>)> = None;
    loop {
        let start = usize::try_from(reader.buffer_position()).map_err(|_| PatchError::Limit)?;
        let event = reader.read_event()?;
        let end = usize::try_from(reader.buffer_position()).map_err(|_| PatchError::Limit)?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag) => {
                let empty = matches!(event, Event::Empty(_));
                match tag.local_name().into_inner() {
                    "dimension" if !in_data && empty => {
                        dimension = Some((
                            start,
                            end,
                            attribute(tag, "ref").and_then(|text| parse_range(&text)),
                        ));
                    }
                    "sheetProtection" => protected = is_true(attribute(tag, "sheet")),
                    "sheetData" => {
                        found_data = true;
                        context.prefix = prefix_of(tag);
                        if empty {
                            let name = format!("{}sheetData", context.prefix);
                            let rows = take_rows_before(&context, &mut pending, u32::MAX);
                            splices.push((start, end, format!("<{name}>{rows}</{name}>")));
                        } else {
                            in_data = true;
                        }
                    }
                    "row" if in_data => {
                        let number = attribute(tag, "r")
                            .and_then(|text| text.parse().ok())
                            .unwrap_or(row + 1);
                        if number <= row || number > MAX_ROW {
                            return Err(PatchError::Unreadable);
                        }
                        let rows = take_rows_before(&context, &mut pending, number);
                        if !rows.is_empty() {
                            splices.push((start, start, rows));
                        }
                        row = number;
                        column = 0;
                        let edited = pending.range((row, 0)..=(row, u32::MAX)).next().is_some();
                        if empty {
                            if edited {
                                let cells = new_cells(
                                    &context,
                                    &take_row_before(&mut pending, row, u32::MAX),
                                );
                                let opening = without_spans(&xml[start..end - 2]);
                                let name = tag.name().into_inner();
                                splices.push((start, end, format!("{opening}>{cells}</{name}>")));
                            }
                        } else {
                            in_row = true;
                            if edited {
                                let opening = &xml[start..end];
                                let clean = without_spans(opening);
                                if clean != opening {
                                    splices.push((start, end, clean));
                                }
                            }
                        }
                    }
                    "c" if in_row => {
                        let position = match attribute(tag, "r") {
                            Some(name) => {
                                parse_cell(&name.replace('$', "")).ok_or(PatchError::Unreadable)?
                            }
                            None => (row, column),
                        };
                        if position.0 != row || (position.1 < column && column > 0) {
                            return Err(PatchError::Unreadable);
                        }
                        column = position.1 + 1;
                        let before = take_row_before(&mut pending, row, position.1);
                        if !before.is_empty() {
                            splices.push((start, start, new_cells(&context, &before)));
                        }
                        let cell = OpenCell {
                            position,
                            start,
                            style: attribute(tag, "s"),
                            old_type: attribute(tag, "t"),
                            has_formula: false,
                        };
                        if empty {
                            if let Some(value) = pending.remove(&position) {
                                let text = cell_xml(
                                    &context,
                                    position,
                                    cell.style.as_deref(),
                                    cell.old_type.as_deref(),
                                    &value,
                                );
                                splices.push((start, end, text));
                            }
                        } else {
                            open = Some(cell);
                        }
                    }
                    "f" => {
                        if let Some(cell) = &mut open {
                            cell.has_formula = true;
                            if matches!(attribute(tag, "t").as_deref(), Some("array" | "dataTable"))
                                && let Some(range) =
                                    attribute(tag, "ref").and_then(|text| parse_range(&text))
                            {
                                formula_ranges.push(range);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(tag) => match tag.local_name().into_inner() {
                "c" if in_row => {
                    if let Some(cell) = open.take()
                        && let Some(value) = pending.remove(&cell.position)
                    {
                        if cell.has_formula {
                            return Err(PatchError::Formula(cell_name(
                                cell.position.0,
                                cell.position.1,
                            )));
                        }
                        let text = cell_xml(
                            &context,
                            cell.position,
                            cell.style.as_deref(),
                            cell.old_type.as_deref(),
                            &value,
                        );
                        splices.push((cell.start, end, text));
                    }
                }
                "row" if in_row => {
                    in_row = false;
                    let rest = take_row_before(&mut pending, row, u32::MAX);
                    if !rest.is_empty() {
                        splices.push((start, start, new_cells(&context, &rest)));
                    }
                }
                "sheetData" if in_data => {
                    in_data = false;
                    let rows = take_rows_before(&context, &mut pending, u32::MAX);
                    if !rows.is_empty() {
                        splices.push((start, start, rows));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    if !found_data || !pending.is_empty() {
        return Err(PatchError::Unreadable);
    }
    if protected {
        return Err(PatchError::Protected(sheet.to_owned()));
    }
    for position in edits.keys() {
        if formula_ranges
            .iter()
            .any(|range| in_range(*range, *position))
        {
            return Err(PatchError::Formula(cell_name(position.0, position.1)));
        }
    }
    // The dimension is a hint too, but a false one can hide new cells from some readers.
    if let Some((start, end, Some(((top, left), (bottom, right))))) = dimension {
        let written = edits
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, _)| *key);
        let (mut new_top, mut new_left, mut new_bottom, mut new_right) = (top, left, bottom, right);
        for (row, column) in written {
            new_top = new_top.min(row);
            new_left = new_left.min(column);
            new_bottom = new_bottom.max(row);
            new_right = new_right.max(column);
        }
        if (new_top, new_left, new_bottom, new_right) != (top, left, bottom, right) {
            let prefix = &context.prefix;
            let (from, to) = (
                cell_name(new_top, new_left),
                cell_name(new_bottom, new_right),
            );
            splices.push((
                start,
                end,
                format!("<{prefix}dimension ref=\"{from}:{to}\"/>"),
            ));
        }
    }
    Ok(apply(xml, splices))
}

/// `workbook.xml` that recalculates on open, and whether the workbook uses the 1904 date system.
fn patch_workbook(xml: &str) -> Result<String, PatchError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut depth = 0_u32;
    let mut prefix = String::new();
    let mut insert_at = None;
    let mut calc: Option<(usize, usize)> = None;
    loop {
        let start = usize::try_from(reader.buffer_position()).map_err(|_| PatchError::Limit)?;
        let event = reader.read_event()?;
        let end = usize::try_from(reader.buffer_position()).map_err(|_| PatchError::Limit)?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag) => {
                let name = tag.local_name().into_inner().to_owned();
                if depth == 0 {
                    prefix = prefix_of(tag);
                } else if depth == 1 {
                    if name == "calcPr" {
                        calc = Some((start, end));
                    } else if insert_at.is_none() && AFTER_CALC_PR.contains(&name.as_str()) {
                        insert_at = Some(start);
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 && insert_at.is_none() {
                    insert_at = Some(start);
                }
            }
            _ => {}
        }
    }
    if let Some((start, end)) = calc {
        let tag = &xml[start..end];
        let body = without_attribute(tag, "fullCalcOnLoad");
        let close = if body.ends_with("/>") {
            body.len() - 2
        } else {
            body.len() - 1
        };
        let text = format!("{} fullCalcOnLoad=\"1\"{}", &body[..close], &body[close..]);
        return Ok(apply(xml, vec![(start, end, text)]));
    }
    let at = insert_at.ok_or(PatchError::Unreadable)?;
    Ok(apply(
        xml,
        vec![(at, at, format!("<{prefix}calcPr fullCalcOnLoad=\"1\"/>"))],
    ))
}

fn without_attribute(tag: &str, name: &str) -> String {
    let marker = format!(" {name}=");
    let Some(at) = tag.find(&marker) else {
        return tag.to_owned();
    };
    let value = at + marker.len();
    let Some(quote) = tag[value..].chars().next() else {
        return tag.to_owned();
    };
    let Some(close) = tag[value + 1..].find(quote) else {
        return tag.to_owned();
    };
    format!("{}{}", &tag[..at], &tag[value + 1 + close + 1..])
}

fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<String, PatchError> {
    let mut text = String::new();
    archive.by_name(name)?.read_to_string(&mut text)?;
    Ok(text)
}

/// The XML without a byte order mark, which the reader's byte positions would not count.
fn body(xml: &str) -> &str {
    xml.strip_prefix('\u{feff}').unwrap_or(xml)
}

/// An entry's path from a relationship target of a part in `directory`.
fn resolve(directory: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => format!("{directory}/{target}"),
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(directory, _)| directory)
}

fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((directory, file)) => format!("{directory}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// `(Id, Type, Target)` of a relationships part.
fn relationships(xml: &str) -> Result<Vec<(String, String, String)>, PatchError> {
    let mut reader = quick_xml::Reader::from_str(body(xml));
    let mut found = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Eof => return Ok(found),
            Event::Start(tag) | Event::Empty(tag)
                if tag.local_name().into_inner() == "Relationship" =>
            {
                found.push((
                    attribute(&tag, "Id").unwrap_or_default(),
                    attribute(&tag, "Type").unwrap_or_default(),
                    attribute(&tag, "Target").unwrap_or_default(),
                ));
            }
            _ => {}
        }
    }
}

/// The sheets `(name, relationship id)` and the date system of a workbook.
fn workbook_sheets(xml: &str) -> Result<(Vec<(String, String)>, bool), PatchError> {
    let mut reader = quick_xml::Reader::from_str(body(xml));
    let (mut sheets, mut date1904) = (Vec::new(), false);
    loop {
        match reader.read_event()? {
            Event::Eof => return Ok((sheets, date1904)),
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().into_inner() {
                "workbookPr" => date1904 = is_true(attribute(&tag, "date1904")),
                "sheet" => {
                    let id = tag.attributes().flatten().find_map(|item| {
                        (item.key.into_inner().split(':').next_back() == Some("id"))
                            .then(|| item.value.into_owned())
                    });
                    if let (Some(name), Some(id)) = (attribute(&tag, "name"), id) {
                        sheets.push((name, id));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// The header cells of the tables on a sheet: Excel repairs a file whose header text differs from
/// the table's column names.
fn table_headers(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    sheet_path: &str,
) -> Result<Vec<Range>, PatchError> {
    let Ok(rels) = read_entry(archive, &rels_path(sheet_path)) else {
        return Ok(Vec::new());
    };
    let mut headers = Vec::new();
    for (_, kind, target) in relationships(&rels)? {
        if !kind.ends_with("/table") {
            continue;
        }
        let table = read_entry(archive, &resolve(parent(sheet_path), &target))?;
        let mut reader = quick_xml::Reader::from_str(body(&table));
        loop {
            match reader.read_event()? {
                Event::Eof => break,
                Event::Start(tag) | Event::Empty(tag)
                    if tag.local_name().into_inner() == "table" =>
                {
                    let count = attribute(&tag, "headerRowCount")
                        .and_then(|text| text.parse::<u32>().ok())
                        .unwrap_or(1);
                    if count > 0
                        && let Some(((top, left), (_, right))) =
                            attribute(&tag, "ref").and_then(|text| parse_range(&text))
                    {
                        headers.push(((top, left), (top + count - 1, right)));
                    }
                    break;
                }
                _ => {}
            }
        }
    }
    Ok(headers)
}

/// Which edited cells show as dates: only there a date text means a date.
fn date_cells(
    bytes: &[u8],
    sheet: &str,
    edits: &BTreeMap<(u32, u32), String>,
) -> Result<HashSet<(u32, u32)>, PatchError> {
    let mut dates = HashSet::new();
    if !edits
        .values()
        .any(|value| date_serial(value, false).is_some())
    {
        return Ok(dates);
    }
    let mut workbook: Xlsx<_> =
        Xlsx::new(Cursor::new(bytes)).map_err(|_| PatchError::Unreadable)?;
    let mut cells = workbook
        .worksheet_cells_reader(sheet)
        .map_err(|_| PatchError::Unreadable)?;
    while let Some(cell) = cells.next_cell().map_err(|_| PatchError::Unreadable)? {
        let (row, column) = cell.get_position();
        let position = (row + 1, column);
        if edits.contains_key(&position) && cell.get_value().is_datetime() {
            dates.insert(position);
        }
    }
    Ok(dates)
}

/// The workbook with `request.edits` applied. Every part other than the sheet and `workbook.xml`
/// keeps its stored bytes.
pub fn patch(bytes: &[u8], request: &PatchRequest) -> Result<Vec<u8>, PatchError> {
    if !bytes.starts_with(b"PK") {
        return Err(PatchError::Unreadable);
    }
    if request.edits.len() > MAX_EDITS {
        return Err(PatchError::TooManyEdits);
    }
    let mut edits = BTreeMap::new();
    for edit in &request.edits {
        let position =
            parse_cell(&edit.cell).ok_or_else(|| PatchError::InvalidCell(edit.cell.clone()))?;
        if !valid_value(&edit.value) {
            return Err(PatchError::InvalidValue(edit.cell.clone()));
        }
        edits.insert(position, edit.value.clone());
    }
    check_zip_limits(bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;

    let root = read_entry(&mut archive, "_rels/.rels")?;
    let workbook_path = relationships(&root)?
        .into_iter()
        .find(|(_, kind, _)| kind.ends_with("/officeDocument"))
        .map(|(_, _, target)| resolve("", &target))
        .ok_or(PatchError::Unreadable)?;
    let workbook_xml = read_entry(&mut archive, &workbook_path)?;
    let (sheets, date1904) = workbook_sheets(&workbook_xml)?;
    let (name, id) = match &request.sheet {
        Some(wanted) => sheets
            .iter()
            .find(|(name, _)| name == wanted)
            .ok_or(PatchError::UnknownSheet)?,
        None => sheets.first().ok_or(PatchError::Unreadable)?,
    };
    let workbook_rels = read_entry(&mut archive, &rels_path(&workbook_path))?;
    let sheet_path = relationships(&workbook_rels)?
        .into_iter()
        .find(|(rel, _, _)| rel == id)
        .map(|(_, _, target)| resolve(parent(&workbook_path), &target))
        .ok_or(PatchError::Unreadable)?;

    for range in table_headers(&mut archive, &sheet_path)? {
        if let Some(position) = edits.keys().find(|position| in_range(range, **position)) {
            return Err(PatchError::TableHeader(cell_name(position.0, position.1)));
        }
    }
    let sheet_xml = read_entry(&mut archive, &sheet_path)?;
    let dates = date_cells(bytes, name, &edits)?;
    let new_sheet = patch_sheet(body(&sheet_xml), &edits, name, dates, date1904)?;
    let new_workbook = patch_workbook(body(&workbook_xml))?;

    let mut output = Cursor::new(Vec::with_capacity(bytes.len() + 4096));
    let mut writer = zip::ZipWriter::new(&mut output);
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        let entry_name = entry.name()?.into_owned();
        let new_text = if entry_name == sheet_path {
            Some(&new_sheet)
        } else if entry_name == workbook_path {
            Some(&new_workbook)
        } else {
            None
        };
        let Some(text) = new_text else {
            writer.raw_copy_file(entry)?;
            continue;
        };
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        if let Some(modified) = entry.last_modified() {
            options = options.last_modified_time(modified);
        }
        drop(entry);
        writer.start_file(entry_name, options)?;
        writer.write_all(text.as_bytes())?;
    }
    writer.finish()?;
    let patched = output.into_inner();

    // The result must read back, or it does not replace the file.
    let mut check: Xlsx<_> =
        Xlsx::new(Cursor::new(patched.as_slice())).map_err(|_| PatchError::Unreadable)?;
    let mut cells = check
        .worksheet_cells_reader(name)
        .map_err(|_| PatchError::Unreadable)?;
    while cells
        .next_cell()
        .map_err(|_| PatchError::Unreadable)?
        .is_some()
    {}
    drop(cells);
    Ok(patched)
}

#[cfg(test)]
pub(crate) mod tests {
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::file_parse::{FileKind, ParseRequest, parse};

    const STYLES: &str = "<styleSheet><numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/></numFmts><cellXfs count=\"2\"><xf numFmtId=\"0\"/><xf numFmtId=\"164\"/></cellXfs></styleSheet>";
    const SHEET: &str = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:C4"/><sheetData><row r="1" spans="1:3"><c r="A1" t="s" s="1"><v>0</v></c><c r="B1"><v>2</v></c><c r="C1"><f>B1*2</f><v>4</v></c></row><row r="3"><c r="B3" s="1"><v>46000</v></c></row><row r="4"><c r="A4"><f t="array" ref="A4:B5">SUM(A1:A2)</f><v>1</v></c></row></sheetData><drawing r:id="rId1"/></worksheet>"#;

    pub(crate) fn workbook(sheet: &str, workbook_extra: &str) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut output);
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let entries: Vec<(&str, Vec<u8>, SimpleFileOptions)> = vec![
            ("[Content_Types].xml", b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>".to_vec(), deflated),
            ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec(), deflated),
            ("xl/workbook.xml", format!(r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Plan" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets>{workbook_extra}</workbook>"#).into_bytes(), deflated),
            ("xl/_rels/workbook.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#.to_vec(), deflated),
            ("xl/styles.xml", STYLES.as_bytes().to_vec(), deflated),
            ("xl/sharedStrings.xml", br#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t>Title</t></si></sst>"#.to_vec(), deflated),
            ("xl/worksheets/sheet1.xml", sheet.as_bytes().to_vec(), deflated),
            ("xl/worksheets/sheet2.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec(), deflated),
            ("xl/charts/chart1.xml", b"<c:chartSpace xmlns:c=\"chart\"><c:chart/></c:chartSpace>".to_vec(), deflated),
            ("xl/drawings/drawing1.xml", b"<xdr:wsDr xmlns:xdr=\"d\"/>".to_vec(), stored),
            ("xl/vbaProject.bin", (0_u8..=255).cycle().take(4096).collect(), stored),
        ];
        for (name, body, options) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(&body).unwrap();
        }
        zip.finish().unwrap();
        output.into_inner()
    }

    fn edit(cell: &str, value: &str) -> CellEdit {
        CellEdit {
            cell: cell.to_owned(),
            value: value.to_owned(),
        }
    }

    fn request(edits: &[(&str, &str)]) -> PatchRequest {
        PatchRequest {
            sheet: None,
            edits: edits
                .iter()
                .map(|(cell, value)| edit(cell, value))
                .collect(),
        }
    }

    fn part(bytes: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        read_entry(&mut archive, name).unwrap()
    }

    /// The compressed bytes and the metadata of each entry, as stored in the zip.
    fn raw_entries(bytes: &[u8]) -> Vec<(String, u32, u64, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|index| {
                let mut entry = archive.by_index_raw(index).unwrap();
                let (name, crc, size) = (
                    entry.name().unwrap().into_owned(),
                    entry.crc32(),
                    entry.size(),
                );
                let mut raw = Vec::new();
                entry.read_to_end(&mut raw).unwrap();
                (name, crc, size, raw)
            })
            .collect()
    }

    fn rows_of(bytes: &[u8], sheet: Option<&str>) -> Vec<Vec<String>> {
        let read = ParseRequest {
            tab_separated: false,
            sheet: sheet.map(str::to_owned),
            offset: 0,
            limit: 100,
        };
        parse(FileKind::Sheet, bytes, &read).unwrap().rows.unwrap()
    }

    #[test]
    fn only_the_sheet_and_the_workbook_change_byte_for_byte() {
        let original = workbook(SHEET, "");
        let patched = patch(&original, &request(&[("B1", "9")])).unwrap();
        let before = raw_entries(&original);
        let after = raw_entries(&patched);
        assert_eq!(
            before.iter().map(|entry| &entry.0).collect::<Vec<_>>(),
            after.iter().map(|entry| &entry.0).collect::<Vec<_>>(),
            "same parts in the same order"
        );
        for (old, new) in before.iter().zip(&after) {
            if old.0 == "xl/worksheets/sheet1.xml" || old.0 == "xl/workbook.xml" {
                assert_ne!(old.3, new.3, "{}", old.0);
            } else {
                assert_eq!(old, new, "{} changed", old.0);
            }
        }
        // Within the sheet, only that cell differs.
        let sheet = part(&patched, "xl/worksheets/sheet1.xml");
        assert_eq!(
            sheet,
            SHEET
                .replace(r#"<c r="B1"><v>2</v></c>"#, r#"<c r="B1"><v>9</v></c>"#)
                .replace(r#" spans="1:3""#, "")
        );
    }

    #[test]
    fn a_format_stays_and_a_text_cell_stays_text() {
        let patched = patch(
            &workbook(SHEET, ""),
            &request(&[("A1", "12"), ("B3", "2026-11-05")]),
        )
        .unwrap();
        let sheet = part(&patched, "xl/worksheets/sheet1.xml");
        assert!(
            sheet.contains(
                r#"<c r="A1" s="1" t="inlineStr"><is><t xml:space="preserve">12</t></is></c>"#
            ),
            "{sheet}"
        );
        // A date cell gets the serial number of the date, in its format.
        assert!(
            sheet.contains(r#"<c r="B3" s="1"><v>46331</v></c>"#),
            "{sheet}"
        );
        let rows = rows_of(&patched, None);
        assert_eq!(rows[0][0], "12");
        assert_eq!(rows[1][1], "2026-11-05");
    }

    #[test]
    fn numbers_text_and_markup_are_written_correctly() {
        let patched = patch(
            &workbook(SHEET, ""),
            &request(&[
                ("B1", "<b>&\"x\""),
                ("D1", "12.5"),
                ("E1", "007"),
                ("F1", "+1"),
            ]),
        )
        .unwrap();
        let sheet = part(&patched, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("&lt;b&gt;&amp;&quot;x&quot;"), "{sheet}");
        assert!(sheet.contains(r#"<c r="D1"><v>12.5</v></c>"#), "{sheet}");
        let rows = rows_of(&patched, None);
        assert_eq!(rows[0], ["Title", "<b>&\"x\"", "4", "12.5", "007", "+1"]);
    }

    #[test]
    fn new_cells_and_rows_go_in_order_and_grow_the_dimension() {
        let patched = patch(
            &workbook(SHEET, ""),
            &request(&[
                ("A3", "before"),
                ("D3", "after"),
                ("A2", "new row"),
                ("B9", "last"),
            ]),
        )
        .unwrap();
        let sheet = part(&patched, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains(r#"<dimension ref="A1:D9"/>"#), "{sheet}");
        let rows = rows_of(&patched, None);
        assert_eq!(rows[1][0], "new row");
        assert_eq!(
            rows[2],
            ["before", "46000", "", "after"].map(|text| if text == "46000" {
                "2025-12-09"
            } else {
                text
            })
        );
        assert_eq!(rows.last().unwrap(), &["", "last"]);
        // An empty sheet too.
        let other = PatchRequest {
            sheet: Some("Other".to_owned()),
            edits: vec![edit("A1", "first")],
        };
        let patched = patch(&workbook(SHEET, ""), &other).unwrap();
        assert_eq!(rows_of(&patched, Some("Other")), [["first"]]);
    }

    #[test]
    fn clearing_keeps_the_format_and_formula_cells_refuse() {
        let patched = patch(&workbook(SHEET, ""), &request(&[("B3", "")])).unwrap();
        assert!(part(&patched, "xl/worksheets/sheet1.xml").contains(r#"<c r="B3" s="1"/>"#));
        for (cell, value) in [("C1", "1"), ("C1", ""), ("A4", "1"), ("B5", "1")] {
            assert_eq!(
                patch(&workbook(SHEET, ""), &request(&[(cell, value)])).unwrap_err(),
                PatchError::Formula(cell.to_owned())
            );
        }
    }

    #[test]
    fn a_protected_sheet_and_bad_input_are_refused() {
        let locked = SHEET.replace("</sheetData>", "</sheetData><sheetProtection sheet=\"1\"/>");
        assert_eq!(
            patch(&workbook(&locked, ""), &request(&[("B1", "1")])).unwrap_err(),
            PatchError::Protected("Plan".to_owned())
        );
        let file = workbook(SHEET, "");
        for bad in ["b1", "1", "XFE1", "A0", "A1048577", "A1:B2"] {
            assert_eq!(
                patch(&file, &request(&[(bad, "x")])).unwrap_err(),
                PatchError::InvalidCell(bad.to_owned()),
                "{bad}"
            );
        }
        assert_eq!(
            patch(&file, &request(&[("B1", "a\u{1}b")])).unwrap_err(),
            PatchError::InvalidValue("B1".to_owned())
        );
        assert_eq!(
            patch(b"\xd0\xcf\x11\xe0 xls", &request(&[("B1", "1")])).unwrap_err(),
            PatchError::Unreadable
        );
        let other = PatchRequest {
            sheet: Some("None".to_owned()),
            edits: vec![edit("A1", "x")],
        };
        assert_eq!(patch(&file, &other).unwrap_err(), PatchError::UnknownSheet);
    }

    #[test]
    fn a_table_header_is_refused_and_its_rows_change() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipArchive::new(Cursor::new(workbook(SHEET, ""))).unwrap();
            let mut zip = zip::ZipWriter::new(&mut output);
            for index in 0..archive.len() {
                zip.raw_copy_file(archive.by_index_raw(index).unwrap())
                    .unwrap();
            }
            let options = SimpleFileOptions::default();
            zip.start_file("xl/worksheets/_rels/sheet1.xml.rels", options)
                .unwrap();
            zip.write_all(br#"<Relationships xmlns="r"><Relationship Id="rId1" Type="http://x/relationships/table" Target="../tables/table1.xml"/></Relationships>"#).unwrap();
            zip.start_file("xl/tables/table1.xml", options).unwrap();
            zip.write_all(br#"<table ref="A1:B3" name="T"/>"#).unwrap();
            zip.finish().unwrap();
        }
        let file = output.into_inner();
        assert_eq!(
            patch(&file, &request(&[("B1", "x")])).unwrap_err(),
            PatchError::TableHeader("B1".to_owned())
        );
        assert!(patch(&file, &request(&[("B3", "x")])).is_ok());
    }

    #[test]
    fn recalculation_on_open_is_set_where_the_schema_wants_it() {
        let patched = patch(&workbook(SHEET, ""), &request(&[("B1", "1")])).unwrap();
        assert!(
            part(&patched, "xl/workbook.xml")
                .contains(r#"</sheets><calcPr fullCalcOnLoad="1"/></workbook>"#)
        );
        // Before an element that must follow it.
        let patched = patch(
            &workbook(SHEET, "<definedNames/><extLst/>"),
            &request(&[("B1", "1")]),
        )
        .unwrap();
        assert!(
            part(&patched, "xl/workbook.xml")
                .contains(r#"<definedNames/><calcPr fullCalcOnLoad="1"/><extLst/>"#)
        );
        // An existing one keeps its settings.
        let patched = patch(
            &workbook(
                SHEET,
                r#"<calcPr calcId="191029" fullCalcOnLoad="0" calcMode="manual"/>"#,
            ),
            &request(&[("B1", "1")]),
        )
        .unwrap();
        assert!(
            part(&patched, "xl/workbook.xml")
                .contains(r#"<calcPr calcId="191029" calcMode="manual" fullCalcOnLoad="1"/>"#)
        );
    }

    #[test]
    fn a_byte_order_mark_and_a_prefix_are_kept_in_mind() {
        let prefixed = SHEET
            .replace("<worksheet xmlns=", "\u{feff}<x:worksheet xmlns:x=")
            .replace("</worksheet>", "</x:worksheet>")
            .replace("<dimension", "<x:dimension")
            .replace("sheetData>", "x:sheetData>")
            .replace("<row", "<x:row")
            .replace("</row", "</x:row")
            .replace("<c ", "<x:c ")
            .replace("</c>", "</x:c>")
            .replace("<v>", "<x:v>")
            .replace("</v>", "</x:v>")
            .replace("<f", "<x:f")
            .replace("</f>", "</x:f>");
        let file = workbook(&prefixed, "");
        let patched = patch(&file, &request(&[("D1", "7"), ("A9", "text")])).unwrap();
        let sheet = part(&patched, "xl/worksheets/sheet1.xml");
        assert!(
            sheet.contains(r#"<x:c r="D1"><x:v>7</x:v></x:c>"#),
            "{sheet}"
        );
        assert!(sheet.contains("<x:row r=\"9\">"), "{sheet}");
    }

    #[test]
    fn cell_names_round_trip() {
        for (name, row, column) in [
            ("A1", 1, 0),
            ("Z9", 9, 25),
            ("AA10", 10, 26),
            ("XFD1048576", 1_048_576, 16_383),
        ] {
            assert_eq!(parse_cell(name), Some((row, column)));
            assert_eq!(cell_name(row, column), name);
        }
    }
}
