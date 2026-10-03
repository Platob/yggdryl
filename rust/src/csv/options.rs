//! The settings a CSV read or write takes.

use smol_str::{SmolStr, format_smolstr};

use crate::media::IORecordOptions;
use crate::text::{LineSep, expected_got};
use crate::{Error, Field, Filter, Level, Result, Selector};

/// Bytes per batch a CSV read closes on when the row bound has not: 64 MiB.
pub const DEFAULT_CSV_BATCH_BYTE_SIZE: u64 = 64 * 1024 * 1024;

/// Records sampled to infer a column's datatype when no field is declared.
pub const DEFAULT_CSV_INFER_ROW_SIZE: usize = 1024;

/// The settings a CSV or TSV document is read and written with.
///
/// The shared settings are every record encoding's. CSV adds the dialect:
/// the separator, the quote and how a quote inside a quoted cell is spelled,
/// a comment byte, whether the first record names the columns, the spellings
/// of an absent value, blank trimming, the sample a column's datatype is
/// inferred from, and the record terminator a write ends each record with.
/// Every byte role is one ASCII byte, never a line break, and no two roles
/// share a byte; a setter refuses a value that would, leaving the options as
/// they were.
///
/// ```
/// use yggdryl::csv::CsvOptions;
///
/// # fn main() -> yggdryl::Result<()> {
/// let options = CsvOptions::new().with_separator(b';')?.with_trim(true);
/// assert_eq!(options.separator(), b';');
/// assert_eq!(options.quote(), Some(b'"'));
/// assert!(options.header());
/// assert_eq!(options.null_values(), [""]);
///
/// // A tab-separated document, by name.
/// assert_eq!(CsvOptions::tsv().separator(), b'\t');
///
/// // A role a line break cannot play.
/// assert!(CsvOptions::new().with_separator(b'\n').is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsvOptions {
    /// Root Field name; the declared field's when one is declared.
    pub name: SmolStr,
    /// The declared root; `None` infers the shape from the header and a
    /// sample of the records.
    pub field: Option<Field>,
    /// The rows a read or write keeps.
    pub filter: Filter,
    /// The columns a read or write publishes.
    pub select: Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: Selector,
    /// Whether a nullable column takes a cell it cannot convert as null.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch, when a reader should bound them.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Leading result rows skipped before `max_row_size` counts.
    pub row_offset: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Whole batches published per streamed-write commit, never rows; `None`
    /// is the destination's own cadence: a leaf, a folder and an Iceberg
    /// table publish once, after the source ends - the table holding every
    /// partition's rows under the process spill bound until then - an
    /// overwrite's first commit replacing and every later one appending
    /// while every commit of a merge merges by its key; a write session by
    /// [`DEFAULT_COMMIT_BYTE_SIZE`](crate::media::DEFAULT_COMMIT_BYTE_SIZE).
    /// The commits completed before a later failure stay published. The rule
    /// is [`IORecordOptions::commit_batch_num`](crate::media::IORecordOptions::commit_batch_num)'s.
    pub commit_batch_num: Option<usize>,
    /// The threads a write of several parts runs on at once; `None` is the
    /// destination's own answer.
    pub num_threads: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    separator: u8,
    quote: Option<u8>,
    escape: Option<u8>,
    comment: Option<u8>,
    header: bool,
    null_values: Vec<SmolStr>,
    trim: bool,
    infer_row_size: usize,
    linesep: LineSep,
}

impl CsvOptions {
    /// The default options: comma-separated, double-quoted with a doubled
    /// quote inside a quoted cell, no escape and no comment byte, a header
    /// record, the empty unquoted cell as null, no trimming, a sample of
    /// [`DEFAULT_CSV_INFER_ROW_SIZE`] records, a line feed ending each
    /// written record.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: Filter::always_true(),
            select: Selector::all(),
            merge_by: Selector::all(),
            safe: true,
            batch_byte_size: Some(DEFAULT_CSV_BATCH_BYTE_SIZE),
            batch_row_size: Some(crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE),
            max_row_size: None,
            row_offset: None,
            max_byte_size: None,
            commit_batch_num: None,
            num_threads: None,
            level: Level::DEFAULT,
            separator: b',',
            quote: Some(b'"'),
            escape: None,
            comment: None,
            header: true,
            null_values: vec![SmolStr::new_static("")],
            trim: false,
            infer_row_size: DEFAULT_CSV_INFER_ROW_SIZE,
            linesep: LineSep::LF,
        }
    }

    /// The default options with a tab separator: what a `.tsv` name reads
    /// and writes.
    #[must_use]
    pub fn tsv() -> Self {
        let mut options = Self::new();
        options.separator = b'\t';
        options
    }

    /// The byte between two cells of one record.
    #[must_use]
    pub const fn separator(&self) -> u8 {
        self.separator
    }

    /// Set the byte between two cells.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.separator` when the byte is not ASCII, is a
    /// line break, or is the quote, the escape or the comment byte.
    pub fn set_separator(&mut self, separator: u8) -> Result<()> {
        self.require_role(
            "$.separator",
            "a separator",
            Some(separator),
            Role::Separator,
        )?;
        self.separator = separator;
        Ok(())
    }

    /// Return these options with another byte between two cells.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_separator`](Self::set_separator) does.
    pub fn with_separator(mut self, separator: u8) -> Result<Self> {
        self.set_separator(separator)?;
        Ok(self)
    }

    /// The byte a cell holding the separator, a line break or a quote is
    /// wrapped in; `None` quotes nothing on write and reads a quote as
    /// content.
    #[must_use]
    pub const fn quote(&self) -> Option<u8> {
        self.quote
    }

    /// Set or clear the quote byte.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.quote` when the byte is not ASCII, is a line
    /// break, or is the separator, the escape or the comment byte.
    pub fn set_quote(&mut self, quote: Option<u8>) -> Result<()> {
        self.require_role("$.quote", "a quote", quote, Role::Quote)?;
        self.quote = quote;
        Ok(())
    }

    /// Return these options with another quote byte, or none.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_quote`](Self::set_quote) does.
    pub fn with_quote(mut self, quote: Option<u8>) -> Result<Self> {
        self.set_quote(quote)?;
        Ok(self)
    }

    /// The byte that, inside a quoted cell, spells the byte after it as
    /// content; `None` is RFC 4180, where a quote inside a quoted cell is
    /// doubled.
    #[must_use]
    pub const fn escape(&self) -> Option<u8> {
        self.escape
    }

    /// Set or clear the escape byte.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.escape` when the byte is not ASCII, is a line
    /// break, or is the separator, the quote or the comment byte.
    pub fn set_escape(&mut self, escape: Option<u8>) -> Result<()> {
        self.require_role("$.escape", "an escape", escape, Role::Escape)?;
        self.escape = escape;
        Ok(())
    }

    /// Return these options with another escape byte, or none.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_escape`](Self::set_escape) does.
    pub fn with_escape(mut self, escape: Option<u8>) -> Result<Self> {
        self.set_escape(escape)?;
        Ok(self)
    }

    /// The byte a record is skipped for holding first; `None` skips none.
    #[must_use]
    pub const fn comment(&self) -> Option<u8> {
        self.comment
    }

    /// Set or clear the comment byte.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.comment` when the byte is not ASCII, is a line
    /// break, or is the separator, the quote or the escape byte.
    pub fn set_comment(&mut self, comment: Option<u8>) -> Result<()> {
        self.require_role("$.comment", "a comment byte", comment, Role::Comment)?;
        self.comment = comment;
        Ok(())
    }

    /// Return these options with another comment byte, or none.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_comment`](Self::set_comment) does.
    pub fn with_comment(mut self, comment: Option<u8>) -> Result<Self> {
        self.set_comment(comment)?;
        Ok(self)
    }

    /// Whether the first record names the columns on read and the column
    /// names are written first on write.
    #[must_use]
    pub const fn header(&self) -> bool {
        self.header
    }

    /// Set whether the first record names the columns.
    pub fn set_header(&mut self, header: bool) {
        self.header = header;
    }

    /// Return these options with or without a header record.
    #[must_use]
    pub const fn with_header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// The spellings of an absent value: an unquoted cell spelling one is
    /// null, and a null is written as the first. A quoted cell is never one
    /// of them, so `""` is the empty text whatever they say.
    #[must_use]
    pub fn null_values(&self) -> &[SmolStr] {
        &self.null_values
    }

    /// Set the spellings of an absent value.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.null_values` naming a spelling listed twice.
    pub fn set_null_values<I, S>(&mut self, null_values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        let null_values: Vec<SmolStr> = null_values.into_iter().map(Into::into).collect();
        for (index, spelling) in null_values.iter().enumerate() {
            if null_values[..index].contains(spelling) {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.null_values"),
                    reason: expected_got(
                        "each null spelling once",
                        format_args!("{spelling:?} twice"),
                    ),
                });
            }
        }
        self.null_values = null_values;
        Ok(())
    }

    /// Return these options with other spellings of an absent value.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_null_values`](Self::set_null_values) does.
    pub fn with_null_values<I, S>(mut self, null_values: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.set_null_values(null_values)?;
        Ok(self)
    }

    /// Whether ASCII blanks around an unquoted cell are dropped before the
    /// cell is read.
    #[must_use]
    pub const fn trim(&self) -> bool {
        self.trim
    }

    /// Set whether ASCII blanks around an unquoted cell are dropped.
    pub fn set_trim(&mut self, trim: bool) {
        self.trim = trim;
    }

    /// Return these options trimming, or keeping, the blanks around a cell.
    #[must_use]
    pub const fn with_trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }

    /// The records sampled to infer a column's datatype when no field is
    /// declared.
    #[must_use]
    pub const fn infer_row_size(&self) -> usize {
        self.infer_row_size
    }

    /// Set the records sampled to infer a column's datatype.
    ///
    /// # Errors
    ///
    /// Returns an error at `$.infer_row_size` for zero: a column cannot be
    /// typed from no record.
    pub fn set_infer_row_size(&mut self, infer_row_size: usize) -> Result<()> {
        if infer_row_size == 0 {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.infer_row_size"),
                reason: expected_got("at least one record to infer a column's datatype from", 0),
            });
        }
        self.infer_row_size = infer_row_size;
        Ok(())
    }

    /// Return these options sampling another number of records.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_infer_row_size`](Self::set_infer_row_size) does.
    pub fn with_infer_row_size(mut self, infer_row_size: usize) -> Result<Self> {
        self.set_infer_row_size(infer_row_size)?;
        Ok(self)
    }

    /// The terminator a write ends each record with. A read accepts `\n`
    /// and `\r\n` whatever this says, and a `\r` alone ends nothing.
    #[must_use]
    pub const fn linesep(&self) -> &LineSep {
        &self.linesep
    }

    /// Set the terminator a write ends each record with.
    pub fn set_linesep(&mut self, linesep: LineSep) {
        self.linesep = linesep;
    }

    /// Return these options ending each written record with `linesep`.
    #[must_use]
    pub fn with_linesep(mut self, linesep: LineSep) -> Self {
        self.linesep = linesep;
        self
    }

    /// Whether `other` reads a document exactly as these options do: the
    /// same dialect cutting it and the same sample typing it, so a schema
    /// read under one answers for the other whatever else they say.
    pub(crate) fn reads_as(&self, other: &Self) -> bool {
        self.separator == other.separator
            && self.quote == other.quote
            && self.escape == other.escape
            && self.comment == other.comment
            && self.header == other.header
            && self.null_values == other.null_values
            && self.trim == other.trim
            && self.infer_row_size == other.infer_row_size
    }

    /// Refuse a byte that cannot play `role`: one that is not ASCII, one
    /// that is a line break, and one another role already holds.
    fn require_role(
        &self,
        path: &'static str,
        setting: &str,
        byte: Option<u8>,
        role: Role,
    ) -> Result<()> {
        let Some(byte) = byte else {
            return Ok(());
        };
        let refused = |actual: String| Error::InvalidRecord {
            path: SmolStr::new_static(path),
            reason: expected_got(
                format_args!(
                    "an ASCII byte for {setting}, neither a line break nor a byte another role holds"
                ),
                actual,
            ),
        };
        if !byte.is_ascii() {
            return Err(refused(format!("{byte:#04x}")));
        }
        if byte == b'\n' || byte == b'\r' {
            return Err(refused(format!("{:?}", char::from(byte))));
        }
        let taken = [
            (Role::Separator, Some(self.separator), "the separator"),
            (Role::Quote, self.quote, "the quote"),
            (Role::Escape, self.escape, "the escape"),
            (Role::Comment, self.comment, "the comment byte"),
        ];
        for (held, value, name) in taken {
            if held != role && value == Some(byte) {
                return Err(refused(
                    format_smolstr!("{:?}, which is {name}", char::from(byte)).to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// The four byte roles a dialect assigns, each to at most one byte.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Role {
    Separator,
    Quote,
    Escape,
    Comment,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for CsvOptions {
    crate::record_options_fields!();
}
