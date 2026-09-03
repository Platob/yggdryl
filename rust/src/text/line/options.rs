//! Flat options for the `text/plain` record encoding.

use std::hash::{Hash, Hasher};

use regex::bytes::Regex;
use smol_str::{SmolStr, format_smolstr};

#[cfg(feature = "arrow")]
use crate::generic::IORecordOptions;
use crate::{DataType, Error, Field, Level, Metadata, Result, Timezone};

use super::LineSep;

/// Columns every decoded text row starts with, in emission order.
pub(crate) const BASE_COLUMNS: [&str; 3] = ["url", "rownum", "body"];

/// A regex whose source, rather than its compiled automaton, is value identity.
#[derive(Clone, Debug)]
struct Expression {
    source: SmolStr,
    compiled: Regex,
}

impl Expression {
    fn new(source: &str, path: &'static str) -> Result<Self> {
        let compiled = Regex::new(source).map_err(|error| Error::InvalidRecord {
            path: SmolStr::new_static(path),
            reason: format_smolstr!("expected a valid byte regex, got {source:?}: {error}"),
        })?;
        Ok(Self {
            source: SmolStr::new(source),
            compiled,
        })
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Expression {}

impl Hash for Expression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

impl Ord for Expression {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.source.cmp(&other.source)
    }
}

impl PartialOrd for Expression {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Settings for text rows reached through the ordinary record-media methods.
///
/// Each physical line is one row. `rowheader`, when set, is searched once in the
/// line; its named captures become nullable columns and its complete match is
/// removed from `body`. `lstrip` and `rstrip` then remove a match only when it
/// touches the corresponding edge. With `autotype`, capture datatypes are
/// inferred from the first row batch and fixed for the rest of the reader.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextOptions {
    /// Root Field name; [`DEFAULT_ROOT_NAME`](crate::generic::DEFAULT_ROOT_NAME) unless set.
    pub name: SmolStr,
    /// Declared root datatype; inferred from text rows when absent.
    pub dtype: Option<DataType>,
    /// Root metadata; empty unless declared.
    pub metadata: Metadata,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch, and the capture-type sample size when autotyping.
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
    rowheader: Option<Expression>,
    lstrip: Option<Expression>,
    rstrip: Option<Expression>,
    linesep: Option<LineSep>,
    autotype: bool,
    timezone: Option<Timezone>,
    captures: Vec<SmolStr>,
}

impl TextOptions {
    /// Build text options with flexible line endings and adaptive captures.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::generic::DEFAULT_ROOT_NAME),
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
            rowheader: None,
            lstrip: None,
            rstrip: None,
            linesep: None,
            autotype: true,
            timezone: None,
            captures: Vec::new(),
        }
    }

    /// Borrow the row-header regex source.
    #[must_use]
    pub fn rowheader(&self) -> Option<&str> {
        self.rowheader
            .as_ref()
            .map(|expression| expression.source.as_str())
    }

    /// Compile or clear the row-header regex atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed syntax or a named capture colliding with
    /// `url`, `rownum`, or `body` under ASCII case folding.
    pub fn set_rowheader(&mut self, rowheader: Option<&str>) -> Result<()> {
        let expression = rowheader
            .map(|source| Expression::new(source, "$.rowheader"))
            .transpose()?;
        let captures = expression
            .as_ref()
            .map(|expression| {
                expression
                    .compiled
                    .capture_names()
                    .flatten()
                    .map(SmolStr::new)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for capture in &captures {
            if BASE_COLUMNS
                .iter()
                .any(|base| base.eq_ignore_ascii_case(capture))
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.rowheader"),
                    reason: format_smolstr!(
                        "expected named captures distinct from url, rownum, and body, got {capture:?}"
                    ),
                });
            }
        }
        self.rowheader = expression;
        self.captures = captures;
        Ok(())
    }

    /// Return these options with a compiled row-header regex.
    ///
    /// # Errors
    ///
    /// Returns the same validation failures as [`Self::set_rowheader`].
    pub fn try_with_rowheader(mut self, rowheader: &str) -> Result<Self> {
        self.set_rowheader(Some(rowheader))?;
        Ok(self)
    }

    /// Borrow the left-edge trimming regex source.
    #[must_use]
    pub fn lstrip(&self) -> Option<&str> {
        self.lstrip
            .as_ref()
            .map(|expression| expression.source.as_str())
    }

    /// Compile or clear the left-edge trimming regex atomically.
    pub fn set_lstrip(&mut self, lstrip: Option<&str>) -> Result<()> {
        self.lstrip = lstrip
            .map(|source| Expression::new(source, "$.lstrip"))
            .transpose()?;
        Ok(())
    }

    /// Return these options with a left-edge trimming regex.
    pub fn try_with_lstrip(mut self, lstrip: &str) -> Result<Self> {
        self.set_lstrip(Some(lstrip))?;
        Ok(self)
    }

    /// Borrow the right-edge trimming regex source.
    #[must_use]
    pub fn rstrip(&self) -> Option<&str> {
        self.rstrip
            .as_ref()
            .map(|expression| expression.source.as_str())
    }

    /// Compile or clear the right-edge trimming regex atomically.
    pub fn set_rstrip(&mut self, rstrip: Option<&str>) -> Result<()> {
        self.rstrip = rstrip
            .map(|source| Expression::new(source, "$.rstrip"))
            .transpose()?;
        Ok(())
    }

    /// Return these options with a right-edge trimming regex.
    pub fn try_with_rstrip(mut self, rstrip: &str) -> Result<Self> {
        self.set_rstrip(Some(rstrip))?;
        Ok(self)
    }

    /// Borrow the pinned line terminator; `None` accepts LF, CRLF, or CR.
    #[must_use]
    pub const fn linesep(&self) -> Option<&LineSep> {
        self.linesep.as_ref()
    }

    /// Pin or clear the line terminator.
    pub fn set_linesep(&mut self, linesep: Option<LineSep>) {
        self.linesep = linesep;
    }

    /// Return these options with a pinned line terminator.
    #[must_use]
    pub fn with_linesep(mut self, linesep: LineSep) -> Self {
        self.set_linesep(Some(linesep));
        self
    }

    /// Return whether named captures are typed from the first batch.
    #[must_use]
    pub const fn autotype(&self) -> bool {
        self.autotype
    }

    /// Enable or disable adaptive capture typing.
    pub const fn set_autotype(&mut self, autotype: bool) {
        self.autotype = autotype;
    }

    /// Return these options with adaptive capture typing changed.
    #[must_use]
    pub const fn with_autotype(mut self, autotype: bool) -> Self {
        self.set_autotype(autotype);
        self
    }

    /// Borrow the timezone applied to autotyped offset-free timestamps.
    #[must_use]
    pub const fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// Set or clear the timezone for autotyped offset-free timestamps.
    pub fn set_timezone(&mut self, timezone: Option<Timezone>) {
        self.timezone = timezone;
    }

    /// Return these options with an autotype timezone.
    #[must_use]
    pub fn with_timezone(mut self, timezone: Timezone) -> Self {
        self.set_timezone(Some(timezone));
        self
    }

    /// Iterate named row-header captures in regex order.
    pub fn capture_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.captures.iter().map(SmolStr::as_str)
    }

    /// Return a deterministic hash of the complete flat configuration.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    pub(crate) fn rowheader_regex(&self) -> Option<&Regex> {
        self.rowheader
            .as_ref()
            .map(|expression| &expression.compiled)
    }

    pub(crate) fn lstrip_regex(&self) -> Option<&Regex> {
        self.lstrip.as_ref().map(|expression| &expression.compiled)
    }

    pub(crate) fn rstrip_regex(&self) -> Option<&Regex> {
        self.rstrip.as_ref().map(|expression| &expression.compiled)
    }

    pub(crate) fn output_linesep(&self) -> &[u8] {
        self.linesep.as_ref().map_or(b"\n", LineSep::as_bytes)
    }

    /// Build the decoder's source field from capture datatypes.
    pub(crate) fn source_field(&self, capture_dtypes: &[DataType]) -> Result<Field> {
        if capture_dtypes.len() != self.captures.len() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.rowheader"),
                reason: format_smolstr!(
                    "expected {} capture datatypes, got {}",
                    self.captures.len(),
                    capture_dtypes.len()
                ),
            });
        }
        let mut fields = Vec::with_capacity(BASE_COLUMNS.len() + self.captures.len());
        fields.push(DataType::Utf8.required_field("url"));
        fields.push(DataType::Int64.required_field("rownum"));
        fields.push(DataType::Binary.required_field("body"));
        fields.extend(
            self.captures
                .iter()
                .zip(capture_dtypes)
                .map(|(name, dtype)| dtype.clone().nullable_field(name.clone())),
        );
        Ok(DataType::from_fields(fields)?.required_field(self.name.clone()))
    }

    /// Build the schema available without sampling data.
    pub(crate) fn fallback_field(&self) -> Result<Field> {
        self.source_field(&vec![DataType::Utf8; self.captures.len()])
    }
}

impl Default for TextOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "arrow")]
impl IORecordOptions for TextOptions {
    crate::record_options_fields!();
}
