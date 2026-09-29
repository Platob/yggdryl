//! The RFC 4180 tokenizer over any byte stream, and the typed row builder
//! over the records it cuts.
//!
//! The tokenizer reads one record at a time into one reusable buffer and
//! names each cell as a range of it, so a record costs the bytes it holds and
//! nothing per cell. The row builder resolves each column once - which cell
//! it reads, and how the text becomes a value - before the first record, so
//! the per-record path indexes cells and runs the reading already chosen.

use std::collections::VecDeque;
use std::io::Read;

use smol_str::{SmolStr, format_smolstr};

use crate::media::IORecordOptions as _;
use crate::text::expected_got;
use crate::{Charset, DataType, Error, Field, Result, Scalar, StructType, TimeUnit, Timezone, Url};

use super::options::CsvOptions;

/// The window the tokenizer reads the transport through: the fetch layer
/// beneath already holds a whole fetch, so this is a bound on what one
/// scan touches, not on what one read requests.
const WINDOW: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// One cell of a record: where it lies in the record's unquoted bytes, and
/// whether it was quoted - the one fact about a cell that outlives its
/// bytes, because a quoted cell is never an absent value.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    start: usize,
    end: usize,
    quoted: bool,
}

/// The dialect the tokenizer cuts by, read off the options once.
#[derive(Clone, Copy)]
struct Dialect {
    separator: u8,
    quote: Option<u8>,
    escape: Option<u8>,
    comment: Option<u8>,
    trim: bool,
}

impl Dialect {
    fn of(options: &CsvOptions) -> Self {
        Self {
            separator: options.separator(),
            quote: options.quote(),
            escape: options.escape(),
            comment: options.comment(),
            trim: options.trim(),
        }
    }
}

/// How one cell ended.
enum Ending {
    Separator,
    Terminator,
    Eof,
}

/// A streaming record cutter over one decoded byte stream.
///
/// [`next_record`](Self::next_record) fills the held record and answers
/// whether there was one; [`record`](Self::record) then borrows it. Blank
/// records - no byte between two terminators - and comment records are
/// skipped, a quoted cell may hold the separator and line breaks, a quote in
/// an unquoted cell is content, and the last record may lack a terminator.
pub(crate) struct Tokenizer<R> {
    source: R,
    window: Vec<u8>,
    start: usize,
    end: usize,
    eof: bool,
    record: Vec<u8>,
    cells: Vec<Cell>,
    /// The 1-based physical line the byte at `start` is on.
    line: u64,
    /// The physical line the held record began on.
    record_line: u64,
    opened: bool,
    dialect: Dialect,
}

impl<R: Read> Tokenizer<R> {
    /// A cutter over `source` under the dialect `options` state.
    pub(crate) fn new(source: R, options: &CsvOptions) -> Self {
        Self {
            source,
            window: vec![0; WINDOW],
            start: 0,
            end: 0,
            eof: false,
            record: Vec::new(),
            cells: Vec::new(),
            line: 1,
            record_line: 1,
            opened: false,
            dialect: Dialect::of(options),
        }
    }

    /// Borrow the record the last [`next_record`](Self::next_record) cut.
    pub(crate) fn record(&self) -> Record<'_> {
        Record {
            bytes: &self.record,
            cells: &self.cells,
            line: self.record_line,
        }
    }

    /// Make at least `needed` unread bytes available, or as many as remain.
    ///
    /// The unread tail moves to the front of the window and the rest is
    /// refilled; the window never grows, so a record longer than it is cut
    /// across refills into the record buffer.
    fn fill(&mut self, needed: usize) -> Result<usize> {
        if self.end - self.start >= needed || self.eof {
            return Ok(self.end - self.start);
        }
        if self.start > 0 {
            self.window.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        while self.end - self.start < needed {
            let read = self.source.read(&mut self.window[self.end..])?;
            if read == 0 {
                self.eof = true;
                break;
            }
            self.end += read;
        }
        if !self.opened {
            self.opened = true;
            // A UTF-8 byte-order mark at the very start is framing, not the
            // first cell's first bytes.
            if self.window[self.start..self.end].starts_with(b"\xEF\xBB\xBF") {
                self.start += 3;
                return self.fill(needed);
            }
        }
        Ok(self.end - self.start)
    }

    /// The next unread byte, without taking it.
    fn peek(&mut self) -> Result<Option<u8>> {
        if self.fill(1)? == 0 {
            return Ok(None);
        }
        Ok(Some(self.window[self.start]))
    }

    /// The byte after the next unread one, without taking either.
    fn peek_second(&mut self) -> Result<Option<u8>> {
        if self.fill(2)? < 2 {
            return Ok(None);
        }
        Ok(Some(self.window[self.start + 1]))
    }

    /// Skip to the end of the current physical line, the terminator included.
    fn skip_line(&mut self) -> Result<()> {
        loop {
            if self.fill(1)? == 0 {
                return Ok(());
            }
            let window = &self.window[self.start..self.end];
            match memchr::memchr(b'\n', window) {
                Some(at) => {
                    self.start += at + 1;
                    self.line += 1;
                    return Ok(());
                }
                None => self.start = self.end,
            }
        }
    }

    /// Cut the next record, answering `false` at the end of the stream.
    ///
    /// # Errors
    ///
    /// Returns the transport's read failure.
    pub(crate) fn next_record(&mut self) -> Result<bool> {
        loop {
            self.record.clear();
            self.cells.clear();
            let Some(first) = self.peek()? else {
                return Ok(false);
            };
            self.record_line = self.line;
            // A blank record is a separator between records, not a record.
            if first == b'\n' {
                self.start += 1;
                self.line += 1;
                continue;
            }
            if first == b'\r' && self.peek_second()? == Some(b'\n') {
                self.start += 2;
                self.line += 1;
                continue;
            }
            if self.dialect.comment == Some(first) {
                self.skip_line()?;
                continue;
            }
            loop {
                match self.next_cell()? {
                    Ending::Separator => {}
                    Ending::Terminator | Ending::Eof => return Ok(true),
                }
            }
        }
    }

    /// Cut one cell into the record buffer and say how it ended.
    fn next_cell(&mut self) -> Result<Ending> {
        let start = self.record.len();
        let mut quoted = false;
        if self.dialect.trim {
            while matches!(self.peek()?, Some(b' ' | b'\t')) {
                self.start += 1;
            }
        }
        if let Some(quote) = self.dialect.quote {
            if self.peek()? == Some(quote) {
                quoted = true;
                self.start += 1;
                self.quoted_content(quote)?;
                // The blanks after a closing quote are the cell's framing,
                // as the ones before the opening one were.
                if self.dialect.trim {
                    while matches!(self.peek()?, Some(b' ' | b'\t')) {
                        self.start += 1;
                    }
                }
            }
        }
        let ending = self.unquoted_content()?;
        let mut end = self.record.len();
        let mut cell_start = start;
        if self.dialect.trim && !quoted {
            while cell_start < end && matches!(self.record[cell_start], b' ' | b'\t') {
                cell_start += 1;
            }
            while end > cell_start && matches!(self.record[end - 1], b' ' | b'\t') {
                end -= 1;
            }
        }
        self.cells.push(Cell {
            start: cell_start,
            end,
            quoted,
        });
        Ok(ending)
    }

    /// Copy a quoted cell's content up to its closing quote.
    ///
    /// A doubled quote spells one quote where no escape is set; with one,
    /// the byte after the escape is content whatever it is. A stream ending
    /// inside the quotes ends the cell with what it held.
    fn quoted_content(&mut self, quote: u8) -> Result<()> {
        loop {
            if self.fill(1)? == 0 {
                return Ok(());
            }
            let window = &self.window[self.start..self.end];
            let found = match self.dialect.escape {
                Some(escape) => memchr::memchr2(quote, escape, window),
                None => memchr::memchr(quote, window),
            };
            let Some(at) = found else {
                self.line += memchr::memchr_iter(b'\n', window).count() as u64;
                self.record.extend_from_slice(window);
                self.start = self.end;
                continue;
            };
            let run = &window[..at];
            self.line += memchr::memchr_iter(b'\n', run).count() as u64;
            self.record.extend_from_slice(run);
            let byte = window[at];
            self.start += at + 1;
            if Some(byte) == self.dialect.escape {
                // The escaped byte is content; an escape ending the stream
                // is content too, there being nothing left for it to spell.
                match self.peek()? {
                    Some(next) => {
                        if next == b'\n' {
                            self.line += 1;
                        }
                        self.record.push(next);
                        self.start += 1;
                    }
                    None => self.record.push(byte),
                }
                continue;
            }
            if self.dialect.escape.is_none() && self.peek()? == Some(quote) {
                self.record.push(quote);
                self.start += 1;
                continue;
            }
            return Ok(());
        }
    }

    /// Copy content up to the separator, the terminator or the end.
    ///
    /// What follows a closing quote reads here too: bytes there are content,
    /// the reading every tolerant reader gives them.
    fn unquoted_content(&mut self) -> Result<Ending> {
        loop {
            if self.fill(1)? == 0 {
                return Ok(Ending::Eof);
            }
            let window = &self.window[self.start..self.end];
            let Some(at) = memchr::memchr3(self.dialect.separator, b'\n', b'\r', window) else {
                self.record.extend_from_slice(window);
                self.start = self.end;
                continue;
            };
            self.record.extend_from_slice(&window[..at]);
            let byte = window[at];
            self.start += at;
            if byte == self.dialect.separator {
                self.start += 1;
                return Ok(Ending::Separator);
            }
            if byte == b'\n' {
                self.start += 1;
                self.line += 1;
                return Ok(Ending::Terminator);
            }
            // A `\r` ends a record only with the `\n` after it.
            if self.peek_second()? == Some(b'\n') {
                self.start += 2;
                self.line += 1;
                return Ok(Ending::Terminator);
            }
            self.record.push(b'\r');
            self.start += 1;
        }
    }
}

/// One cut record, borrowed from the tokenizer that holds it.
#[derive(Clone, Copy)]
pub(crate) struct Record<'a> {
    bytes: &'a [u8],
    cells: &'a [Cell],
    line: u64,
}

impl Record<'_> {
    /// How many cells the record holds.
    pub(crate) fn len(&self) -> usize {
        self.cells.len()
    }

    /// The bytes of cell `index` and whether it was quoted.
    pub(crate) fn cell(&self, index: usize) -> (&[u8], bool) {
        let cell = self.cells[index];
        (&self.bytes[cell.start..cell.end], cell.quoted)
    }

    /// The 1-based physical line the record began on.
    pub(crate) fn line(&self) -> u64 {
        self.line
    }

    /// A copy of the record that outlives the tokenizer's buffer.
    pub(crate) fn into_owned(self) -> OwnedRecord {
        OwnedRecord {
            bytes: self.bytes.to_vec(),
            cells: self.cells.to_vec(),
            line: self.line,
        }
    }
}

/// One record held past the tokenizer's buffer: a sampled row emitted after
/// the datatypes it helped infer are settled.
pub(crate) struct OwnedRecord {
    bytes: Vec<u8>,
    cells: Vec<Cell>,
    line: u64,
}

impl OwnedRecord {
    fn as_record(&self) -> Record<'_> {
        Record {
            bytes: &self.bytes,
            cells: &self.cells,
            line: self.line,
        }
    }
}

/// How the text of one column's cells becomes a value: resolved once from
/// the column's datatype, so the per-cell path runs the reading and never
/// chooses it.
enum CellReader {
    /// Plain UTF-8 text, the cell as it stands.
    Text,
    /// `true` or `false` in any ASCII case.
    Boolean,
    /// A signed 64-bit integer.
    Int64,
    /// A finite binary64 float.
    Float64,
    /// `YYYY-MM-DD`.
    Date32,
    /// An ISO 8601 instant read as the column's zone reads one: an offset
    /// or `Z` is the instant it names, a naive spelling a wall clock in the
    /// zone.
    Zoned(DataType),
    /// A nested value spelled as compact JSON.
    Json,
    /// Every other datatype, through the field's own value door.
    Value,
}

impl CellReader {
    fn of(field: &Field) -> Self {
        // A dictionary or a run-end encoding lays a value out and says
        // nothing about how its text reads, so the encoded datatype decides,
        // through the field's own door where the layout is not the leaf's.
        let encoded = super::writer::encoded_dtype(field.dtype());
        if !std::ptr::eq(encoded, field.dtype()) {
            return if encoded.kind() == crate::DataTypeKind::Nested {
                Self::Json
            } else {
                Self::Value
            };
        }
        match field.dtype() {
            DataType::Utf8String => Self::Text,
            DataType::Boolean => Self::Boolean,
            DataType::Int64 => Self::Int64,
            DataType::Float64 => Self::Float64,
            DataType::Date32 => Self::Date32,
            dtype @ DataType::DateTime64 { timezone, .. } if !timezone.is_naive() => {
                Self::Zoned(dtype.clone())
            }
            dtype if dtype.is_nested() => Self::Json,
            _ => Self::Value,
        }
    }
}

/// One column of the row: the field it lands under, how its text reads,
/// and which cell of a record holds it - none where the record has no cell
/// for it, which is an absent value.
struct Column {
    field: Field,
    reader: CellReader,
    at: Option<usize>,
}

/// The typed row builder: every column resolved once against the header
/// and the field, and the nulls, the strictness and the location the rows
/// are refused by.
pub(crate) struct Columns {
    columns: Vec<Column>,
    width: usize,
    null_values: Vec<SmolStr>,
    safe: bool,
    url: Option<Url>,
}

impl Columns {
    /// Whether an unquoted cell holding `bytes` spells an absent value.
    fn is_null(&self, bytes: &[u8], quoted: bool) -> bool {
        !quoted
            && self
                .null_values
                .iter()
                .any(|spelling| spelling.as_bytes() == bytes)
    }

    /// One row of the field, in column order, from one record.
    fn row(&self, record: Record<'_>, index: u64) -> Result<Scalar> {
        if record.len() != self.width {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$[{index}]"),
                reason: format_smolstr!(
                    "expected {} cells, got {} in row {} of {}",
                    self.width,
                    record.len(),
                    record.line(),
                    self.location()
                ),
            });
        }
        let mut values = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            let value = match column.at {
                None => column
                    .field
                    .scalar(Scalar::Null)
                    .map_err(|error| self.refused(index, record.line(), column, &error))?,
                Some(at) => {
                    let (bytes, quoted) = record.cell(at);
                    self.cell(column, bytes, quoted, index, record.line())?
                }
            };
            values.push(value);
        }
        Ok(Scalar::from_sequence(values))
    }

    /// One cell as the value its column reads it into.
    fn cell(
        &self,
        column: &Column,
        bytes: &[u8],
        quoted: bool,
        index: u64,
        line: u64,
    ) -> Result<Scalar> {
        if self.is_null(bytes, quoted) {
            return column
                .field
                .scalar(Scalar::Null)
                .map_err(|error| self.refused(index, line, column, &error));
        }
        // The transport decoded a declared charset already; a stray byte in
        // an undeclared one reads as the Windows-1252 character it is, the
        // rule every text line reads by.
        let text = Charset::Utf8.transcribe(bytes);
        match read_cell(&column.reader, &column.field, &text) {
            Ok(value) => Ok(value),
            Err(_) if self.safe && column.field.is_nullable() => Ok(Scalar::Null),
            Err(error) => Err(self.refused(index, line, column, &error)),
        }
    }

    /// The refusal a cell of `column` raised, located by the row it was
    /// read in: the door's own reason, and the path below the column
    /// where the door named one.
    fn refused(&self, index: u64, line: u64, column: &Column, error: &Error) -> Error {
        let reason = match error {
            Error::InvalidRecord { path, reason } => {
                let own = path
                    .strip_prefix("$.")
                    .is_some_and(|rest| rest == column.field.name());
                if own || path == "$" {
                    reason.clone()
                } else {
                    format_smolstr!("{reason} at {path}")
                }
            }
            other => format_smolstr!("{}", crate::text::elide_display(other)),
        };
        Error::InvalidRecord {
            path: format_smolstr!("$[{index}].{}", column.field.name()),
            reason: format_smolstr!("{reason} in row {line} of {}", self.location()),
        }
    }

    fn location(&self) -> SmolStr {
        self.url.as_ref().map_or_else(
            || SmolStr::new_static("<anonymous>"),
            |url| format_smolstr!("{url}"),
        )
    }
}

/// Read `text` the way `reader` says the column reads.
fn read_cell(reader: &CellReader, field: &Field, text: &str) -> Result<Scalar> {
    let invalid = |expected: &str| Error::InvalidRecord {
        path: format_smolstr!("$.{}", field.name()),
        reason: expected_got(
            expected,
            format_args!(
                "{:?}",
                crate::text::elide_to(text, crate::text::ERROR_TEXT_LIMIT)
            ),
        ),
    };
    match reader {
        CellReader::Text => Ok(Scalar::from(text)),
        CellReader::Boolean => read_boolean(text).ok_or_else(|| invalid("true or false")),
        CellReader::Int64 => text
            .parse::<i64>()
            .map(Scalar::from)
            .map_err(|_| invalid("a 64-bit integer")),
        CellReader::Float64 => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Scalar::from)
            .ok_or_else(|| invalid("a finite float")),
        CellReader::Date32 => Scalar::from_temporal_text(&DataType::Date32, text),
        CellReader::Zoned(dtype) => {
            crate::text::arrow::parse_capture(text, dtype, None).map_err(|reason| {
                Error::InvalidRecord {
                    path: format_smolstr!("$.{}", field.name()),
                    reason,
                }
            })
        }
        CellReader::Json => crate::from_json_scalar_with_field(text, field),
        CellReader::Value => crate::text::typed::with_field(Scalar::from(text), field),
    }
}

fn read_boolean(text: &str) -> Option<Scalar> {
    if text.eq_ignore_ascii_case("true") {
        return Some(Scalar::from(true));
    }
    if text.eq_ignore_ascii_case("false") {
        return Some(Scalar::from(false));
    }
    None
}

/// The datatype ladder a column climbs while its cells keep fitting: the
/// first rung every non-null sampled cell fits is the column's datatype,
/// and a column no rung fits - or one with no non-null cell - is text.
const LADDER: [DataType; 4] = [
    DataType::Boolean,
    DataType::Int64,
    DataType::Float64,
    DataType::Date32,
];

/// Which rungs of the ladder a column's sampled cells still fit, and
/// whether any cell was there to fit one: a column of nothing but absent
/// cells fits every rung and is text.
struct Candidates {
    fits: [bool; 5],
    seen: bool,
}

impl Candidates {
    const fn new() -> Self {
        Self {
            fits: [true; 5],
            seen: false,
        }
    }

    /// Narrow the rungs to the ones `text` fits.
    fn narrow(&mut self, text: &str, zoned: &DataType) {
        self.seen = true;
        for (rung, fits) in self.fits.iter_mut().enumerate() {
            if !*fits {
                continue;
            }
            *fits = match rung {
                0 => read_boolean(text).is_some(),
                1 => text.parse::<i64>().is_ok(),
                2 => text.parse::<f64>().is_ok_and(f64::is_finite),
                3 => Scalar::from_temporal_text(&DataType::Date32, text).is_ok(),
                _ => crate::text::arrow::parse_capture(text, zoned, None).is_ok(),
            };
        }
    }

    /// The first rung still fitting, `None` where none is.
    fn datatype(&self, zoned: &DataType) -> Option<DataType> {
        if !self.seen {
            return None;
        }
        let rung = self.fits.iter().position(|fits| *fits)?;
        Some(match LADDER.get(rung) {
            Some(dtype) => dtype.clone(),
            None => zoned.clone(),
        })
    }
}

/// The datatype the instant rung reads: nanoseconds in UTC, a naive
/// spelling read as UTC.
fn zoned_datatype() -> Result<DataType> {
    DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)
}

/// The names the header states, or `column_<i>` for the `width` cells of
/// the first record where there is no header.
fn header_names(
    record: Option<Record<'_>>,
    width: usize,
    location: &SmolStr,
) -> Result<Vec<SmolStr>> {
    let mut names = Vec::with_capacity(width);
    for index in 0..width {
        let name = match record {
            Some(record) => {
                let (bytes, _) = record.cell(index);
                let text = Charset::Utf8.transcribe(bytes);
                if text.is_empty() {
                    format_smolstr!("column_{}", index + 1)
                } else {
                    SmolStr::new(text.as_ref())
                }
            }
            None => format_smolstr!("column_{}", index + 1),
        };
        if names.contains(&name) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected each column name once, got {name:?} twice in the header of {location}"
                ),
            });
        }
        names.push(name);
    }
    Ok(names)
}

/// The rows of one document: the sampled records first, then the rest of
/// the stream, each read into the field's row.
pub(crate) struct Rows<R> {
    sampled: VecDeque<OwnedRecord>,
    tokenizer: Tokenizer<R>,
    columns: Columns,
    index: u64,
    done: bool,
}

impl<R: Read> Iterator for Rows<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let index = self.index;
        let row = match self.sampled.pop_front() {
            Some(held) => self.columns.row(held.as_record(), index),
            None => match self.tokenizer.next_record() {
                Ok(true) => self.columns.row(self.tokenizer.record(), index),
                Ok(false) => {
                    self.done = true;
                    return None;
                }
                Err(error) => Err(error),
            },
        };
        if row.is_err() {
            self.done = true;
        }
        self.index += 1;
        Some(row)
    }
}

impl<R> std::iter::FusedIterator for Rows<R> where R: Read {}

/// A document opened for reading: the field its rows land under and the rows.
pub(crate) struct Opened<R> {
    /// The declared field, or the one the header and the sample state.
    pub(crate) field: Field,
    /// The rows, sampled ones first.
    pub(crate) rows: Rows<R>,
}

/// Open the stream `bytes` as the document it holds.
///
/// The first record names the columns when the options say so; a declared
/// field reads its columns by name from that header - positionally where
/// there is no header - and a column the header does not state is absent,
/// null where nullable and refused by name otherwise. With no declared
/// field, up to `infer_row_size` records are sampled and each column's
/// datatype inferred over their non-null cells; the sampled records are the
/// first rows out.
///
/// `None` is a document holding no record at all.
///
/// # Errors
///
/// Returns the transport's read failure, a header naming a column twice, and
/// a required declared column the header does not state.
pub(crate) fn open<R: Read>(
    bytes: R,
    options: &CsvOptions,
    declared: Option<&Field>,
    url: Option<Url>,
) -> Result<Option<Opened<R>>> {
    let location = url.as_ref().map_or_else(
        || SmolStr::new_static("<anonymous>"),
        |url| format_smolstr!("{url}"),
    );
    let mut tokenizer = Tokenizer::new(bytes, options);
    if !tokenizer.next_record()? {
        return Ok(None);
    }
    let mut sampled = VecDeque::new();
    let width = tokenizer.record().len();
    let names = if options.header() {
        header_names(Some(tokenizer.record()), width, &location)?
    } else {
        sampled.push_back(tokenizer.record().into_owned());
        header_names(None, width, &location)?
    };
    let null_values = options.null_values().to_vec();
    let columns = match declared {
        Some(declared) => {
            let mut columns = Vec::with_capacity(declared.field_len());
            for (position, child) in declared.fields().iter().enumerate() {
                let at = if options.header() {
                    names.iter().position(|name| name == child.name())
                } else {
                    (position < width).then_some(position)
                };
                if at.is_none() && !child.is_nullable() {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.header"),
                        reason: expected_got(
                            format_args!(
                                "a column named {:?} for the declared not-null field",
                                child.name()
                            ),
                            format_args!("{names:?} in the header of {location}"),
                        ),
                    });
                }
                columns.push(Column {
                    field: child.clone(),
                    reader: CellReader::of(child),
                    at,
                });
            }
            columns
        }
        None => {
            let zoned = zoned_datatype()?;
            let mut candidates: Vec<Candidates> = (0..width).map(|_| Candidates::new()).collect();
            let nulls = Columns {
                columns: Vec::new(),
                width,
                null_values: null_values.clone(),
                safe: options.safe(),
                url: None,
            };
            let mut narrow = |record: Record<'_>| {
                if record.len() != width {
                    return;
                }
                for (index, candidate) in candidates.iter_mut().enumerate() {
                    let (bytes, quoted) = record.cell(index);
                    if nulls.is_null(bytes, quoted) {
                        continue;
                    }
                    candidate.narrow(&Charset::Utf8.transcribe(bytes), &zoned);
                }
            };
            for held in &sampled {
                narrow(held.as_record());
            }
            while sampled.len() < options.infer_row_size() && tokenizer.next_record()? {
                narrow(tokenizer.record());
                sampled.push_back(tokenizer.record().into_owned());
            }
            names
                .iter()
                .zip(&candidates)
                .enumerate()
                .map(|(at, (name, candidate))| {
                    let dtype = candidate.datatype(&zoned).unwrap_or(DataType::utf8());
                    let field = dtype.nullable_field(name.clone());
                    Column {
                        reader: CellReader::of(&field),
                        field,
                        at: Some(at),
                    }
                })
                .collect()
        }
    };
    let field = match declared {
        Some(declared) => declared.clone(),
        None => DataType::from(StructType::from_fields(
            columns.iter().map(|column| column.field.clone()),
        )?)
        .required_field(options.name()),
    };
    Ok(Some(Opened {
        field,
        rows: Rows {
            sampled,
            tokenizer,
            columns: Columns {
                columns,
                width,
                null_values,
                safe: options.safe(),
                url,
            },
            index: 0,
            done: false,
        },
    }))
}

/// Count the records of `bytes`, the header left out, reading no cell.
///
/// # Errors
///
/// Returns the transport's read failure.
pub(crate) fn count<R: Read>(bytes: R, options: &CsvOptions) -> Result<u64> {
    let mut tokenizer = Tokenizer::new(bytes, options);
    let mut records = 0_u64;
    while tokenizer.next_record()? {
        records = records.checked_add(1).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("logical row count exceeds u64::MAX"),
        })?;
    }
    if options.header() {
        records = records.saturating_sub(1);
    }
    Ok(records)
}
