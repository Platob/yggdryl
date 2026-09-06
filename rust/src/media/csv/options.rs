//! Flat options for the `text/csv` record encoding.

use smol_str::{SmolStr, format_smolstr};

#[cfg(feature = "arrow")]
use crate::media::IORecordOptions;
use crate::media::text::LineSep;
use crate::{DataType, Error, Field, Level, Metadata, Result, Timezone};

use super::scan::Dialect;

/// Rows sampled to infer a column's datatype when the caller declares none.
pub const DEFAULT_INFER_ROW_SIZE: usize = 1024;

/// Settings for CSV rows reached through the ordinary record-media methods.
///
/// The record terminator keeps the plain-text vocabulary - `linesep`, pinned
/// or flexible - and `separator` is the byte between two cells, so the two
/// never read as the same setting. A declared `dtype` turns inference off:
/// cells are read as the declared columns say.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsvOptions {
    /// Root Field name; [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME) unless set.
    pub name: SmolStr,
    /// Declared root datatype; inferred from the header and cells when absent.
    pub dtype: Option<DataType>,
    /// Root metadata; empty unless declared.
    pub metadata: Metadata,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per emitted batch.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Column names forming a write's match key.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by.
    pub filter_partitions: Vec<(String, String)>,
    separator: u8,
    quote: Option<u8>,
    escape: Option<u8>,
    comment: Option<u8>,
    linesep: Option<LineSep>,
    header: bool,
    null: SmolStr,
    trim: bool,
    autotype: bool,
    infer_row_size: Option<usize>,
    timezone: Option<Timezone>,
}

impl CsvOptions {
    /// Build comma-separated options with a header row and inferred columns.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            dtype: None,
            metadata: Metadata::new(),
            safe: false,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            separator: b',',
            quote: Some(b'"'),
            escape: None,
            comment: None,
            linesep: None,
            header: true,
            null: SmolStr::new_static(""),
            trim: false,
            autotype: true,
            infer_row_size: Some(DEFAULT_INFER_ROW_SIZE),
            timezone: None,
        }
    }

    /// Return the byte that delimits two cells of one record.
    #[must_use]
    pub const fn separator(&self) -> u8 {
        self.separator
    }

    /// Set the cell separator.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte is already spoken for - the quote, the
    /// escape, the comment marker, or a byte of the record terminator - since
    /// one byte cannot mean two things in the same resource.
    pub fn set_separator(&mut self, separator: u8) -> Result<()> {
        let candidate = Self {
            separator,
            ..self.clone()
        };
        candidate.validate("$.separator")?;
        self.separator = separator;
        Ok(())
    }

    /// Return these options with a different cell separator.
    ///
    /// # Errors
    ///
    /// Returns the same collision failure as [`Self::set_separator`].
    pub fn try_with_separator(mut self, separator: u8) -> Result<Self> {
        self.set_separator(separator)?;
        Ok(self)
    }

    /// Return the byte that opens and closes a quoted cell.
    #[must_use]
    pub const fn quote(&self) -> Option<u8> {
        self.quote
    }

    /// Set or clear the quote byte; clearing reads every cell literally.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte collides with another role.
    pub fn set_quote(&mut self, quote: Option<u8>) -> Result<()> {
        let candidate = Self {
            quote,
            ..self.clone()
        };
        candidate.validate("$.quote")?;
        self.quote = quote;
        Ok(())
    }

    /// Return these options with a different quote byte.
    ///
    /// # Errors
    ///
    /// Returns the same collision failure as [`Self::set_quote`].
    pub fn try_with_quote(mut self, quote: u8) -> Result<Self> {
        self.set_quote(Some(quote))?;
        Ok(self)
    }

    /// Return the byte that escapes the next byte inside a quoted cell.
    ///
    /// `None` is RFC 4180: a quote inside a quoted cell is written twice.
    #[must_use]
    pub const fn escape(&self) -> Option<u8> {
        self.escape
    }

    /// Set or clear the escape byte.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte collides with another role.
    pub fn set_escape(&mut self, escape: Option<u8>) -> Result<()> {
        let candidate = Self {
            escape,
            ..self.clone()
        };
        candidate.validate("$.escape")?;
        self.escape = escape;
        Ok(())
    }

    /// Return these options with an explicit escape byte.
    ///
    /// # Errors
    ///
    /// Returns the same collision failure as [`Self::set_escape`].
    pub fn try_with_escape(mut self, escape: u8) -> Result<Self> {
        self.set_escape(Some(escape))?;
        Ok(self)
    }

    /// Return the byte a line must open with to carry no record.
    #[must_use]
    pub const fn comment(&self) -> Option<u8> {
        self.comment
    }

    /// Set or clear the comment marker.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte collides with another role.
    pub fn set_comment(&mut self, comment: Option<u8>) -> Result<()> {
        let candidate = Self {
            comment,
            ..self.clone()
        };
        candidate.validate("$.comment")?;
        self.comment = comment;
        Ok(())
    }

    /// Return these options with a comment marker.
    ///
    /// # Errors
    ///
    /// Returns the same collision failure as [`Self::set_comment`].
    pub fn try_with_comment(mut self, comment: u8) -> Result<Self> {
        self.set_comment(Some(comment))?;
        Ok(self)
    }

    /// Borrow the pinned record terminator; `None` accepts LF, CRLF, or CR.
    #[must_use]
    pub const fn linesep(&self) -> Option<&LineSep> {
        self.linesep.as_ref()
    }

    /// Pin or clear the record terminator.
    ///
    /// # Errors
    ///
    /// Returns an error when a terminator byte is already spoken for.
    pub fn set_linesep(&mut self, linesep: Option<LineSep>) -> Result<()> {
        let candidate = Self {
            linesep: linesep.clone(),
            ..self.clone()
        };
        candidate.validate("$.linesep")?;
        self.linesep = linesep;
        Ok(())
    }

    /// Return these options with a pinned record terminator.
    ///
    /// # Errors
    ///
    /// Returns the same collision failure as [`Self::set_linesep`].
    pub fn try_with_linesep(mut self, linesep: LineSep) -> Result<Self> {
        self.set_linesep(Some(linesep))?;
        Ok(self)
    }

    /// Return whether the first record names the columns.
    #[must_use]
    pub const fn header(&self) -> bool {
        self.header
    }

    /// Set whether the first record names the columns.
    ///
    /// Without a header the columns are named `column_1`, `column_2`, and so
    /// on, in the order the cells appear.
    pub const fn set_header(&mut self, header: bool) {
        self.header = header;
    }

    /// Return these options with a different header setting.
    #[must_use]
    pub const fn with_header(mut self, header: bool) -> Self {
        self.set_header(header);
        self
    }

    /// Borrow the cell text that spells an absent value.
    ///
    /// The default is the empty cell. A quoted empty cell is still the empty
    /// string, so a resource can spell both.
    #[must_use]
    pub fn null(&self) -> &str {
        self.null.as_str()
    }

    /// Set the cell text that spells an absent value.
    pub fn set_null(&mut self, null: impl Into<SmolStr>) {
        self.null = null.into();
    }

    /// Return these options with a different absent-value spelling.
    #[must_use]
    pub fn with_null(mut self, null: impl Into<SmolStr>) -> Self {
        self.set_null(null);
        self
    }

    /// Return whether unquoted cells drop their edge ASCII whitespace.
    #[must_use]
    pub const fn trim(&self) -> bool {
        self.trim
    }

    /// Set whether unquoted cells drop their edge ASCII whitespace.
    pub const fn set_trim(&mut self, trim: bool) {
        self.trim = trim;
    }

    /// Return these options with a different trimming setting.
    #[must_use]
    pub const fn with_trim(mut self, trim: bool) -> Self {
        self.set_trim(trim);
        self
    }

    /// Return whether column datatypes are inferred from the cells.
    #[must_use]
    pub const fn autotype(&self) -> bool {
        self.autotype
    }

    /// Enable or disable value-directed column typing; disabled reads text.
    pub const fn set_autotype(&mut self, autotype: bool) {
        self.autotype = autotype;
    }

    /// Return these options with value-directed column typing changed.
    #[must_use]
    pub const fn with_autotype(mut self, autotype: bool) -> Self {
        self.set_autotype(autotype);
        self
    }

    /// Return how many rows inference reads; `None` reads every row.
    #[must_use]
    pub const fn infer_row_size(&self) -> Option<usize> {
        self.infer_row_size
    }

    /// Set how many rows inference reads; `None` reads every row.
    pub const fn set_infer_row_size(&mut self, infer_row_size: Option<usize>) {
        self.infer_row_size = infer_row_size;
    }

    /// Return these options with a different inference sample size.
    #[must_use]
    pub const fn with_infer_row_size(mut self, infer_row_size: usize) -> Self {
        self.set_infer_row_size(Some(infer_row_size));
        self
    }

    /// Borrow the timezone applied to inferred offset-free timestamps.
    #[must_use]
    pub const fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// Set or clear the timezone for inferred offset-free timestamps.
    pub fn set_timezone(&mut self, timezone: Option<Timezone>) {
        self.timezone = timezone;
    }

    /// Return these options with an inference timezone.
    #[must_use]
    pub fn with_timezone(mut self, timezone: Timezone) -> Self {
        self.set_timezone(Some(timezone));
        self
    }

    /// Return a deterministic hash of the complete flat configuration.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Build the byte-level dialect the scanner and renderer both read.
    pub(crate) fn dialect(&self) -> Dialect {
        Dialect {
            separator: self.separator,
            quote: self.quote,
            escape: self.escape,
            comment: self.comment,
            linesep: self.linesep.clone(),
            trim: self.trim,
        }
    }

    /// Return the terminator a write ends each record with.
    pub(crate) fn output_linesep(&self) -> &[u8] {
        self.linesep.as_ref().map_or(b"\n", LineSep::as_bytes)
    }

    /// Refuse a configuration where one byte would carry two meanings.
    fn validate(&self, path: &'static str) -> Result<()> {
        let mut roles: Vec<(&'static str, u8)> = vec![("the separator", self.separator)];
        if let Some(quote) = self.quote {
            roles.push(("the quote", quote));
        }
        if let Some(escape) = self.escape {
            // An escape equal to the quote is RFC 4180's doubling, spelled the
            // other way round, so it is a configuration rather than a clash.
            if Some(escape) != self.quote {
                roles.push(("the escape", escape));
            }
        }
        if let Some(comment) = self.comment {
            roles.push(("the comment marker", comment));
        }
        for (index, (role, byte)) in roles.iter().enumerate() {
            for (other_role, other) in &roles[index + 1..] {
                if byte == other {
                    return Err(collision(path, role, other_role, *byte));
                }
            }
            let terminator = self
                .linesep
                .as_ref()
                .map_or(b"\r\n".as_slice(), LineSep::as_bytes);
            if terminator.contains(byte) {
                return Err(collision(path, role, "the record terminator", *byte));
            }
        }
        Ok(())
    }
}

/// Name both roles a byte was asked to play.
fn collision(path: &'static str, role: &str, other: &str, byte: u8) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static(path),
        reason: format_smolstr!(
            "expected {role} to differ from {other}, got {:?} for both",
            char::from(byte)
        ),
    }
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "arrow")]
impl IORecordOptions for CsvOptions {
    crate::record_options_fields!();
}

/// The column name a headerless resource gives its `index`-th cell.
pub(crate) fn positional_name(index: usize) -> SmolStr {
    format_smolstr!("column_{}", index + 1)
}

/// Build the canonical root Field from column fields under one root name.
pub(crate) fn root_field(name: &SmolStr, fields: Vec<Field>) -> Result<Field> {
    Ok(DataType::from_fields(fields)?.required_field(name.clone()))
}
