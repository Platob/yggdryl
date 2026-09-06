//! Flat options for the `text/plain` record encoding.

use std::hash::{Hash, Hasher};

use regex::bytes::Regex;
use smol_str::{SmolStr, format_smolstr};

#[cfg(feature = "arrow")]
use crate::media::IORecordOptions;
use crate::{DataType, Error, Field, Level, Metadata, Result, Timezone};

use super::{LeadingFragment, LineSep};

/// Reserved columns emitted before decoded row-header captures.
pub(crate) const BASE_COLUMNS: [&str; 4] = ["url", "rownum", "body", "dropped_byte_size"];

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
/// Physical-line mode emits one row per line. With `framing` enabled,
/// `rowheader` starts a logical record and following nonmatching lines join its
/// body with normalized `\n` separators. Named captures remain nullable and the
/// complete header match is removed only from the first physical line.
/// `lstrip` and `rstrip` remove edge matches, and `autotype` infers capture
/// datatypes from regex syntax before the resource is read.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextOptions {
    /// Root Field name; [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME) unless set.
    pub name: SmolStr,
    /// Declared root datatype; inferred from text rows when absent.
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
    /// First emitted row number; `None` omits the `rownum` column.
    pub with_rownum: Option<i64>,
    framing: bool,
    leading_fragment: LeadingFragment,
    max_record_byte_size: Option<u64>,
    rowheader: Option<Expression>,
    lstrip: Option<Expression>,
    rstrip: Option<Expression>,
    linesep: Option<LineSep>,
    autotype: bool,
    timezone: Option<Timezone>,
    captures: Vec<Field>,
}

impl TextOptions {
    /// Build text options with flexible line endings and syntax-typed captures.
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
            with_rownum: None,
            framing: false,
            leading_fragment: LeadingFragment::Keep,
            max_record_byte_size: None,
            rowheader: None,
            lstrip: None,
            rstrip: None,
            linesep: None,
            autotype: true,
            timezone: None,
            captures: Vec::new(),
        }
    }

    /// Return whether physical lines are framed into logical records.
    #[must_use]
    pub const fn framing(&self) -> bool {
        self.framing
    }

    /// Enable or disable logical-record framing.
    pub const fn set_framing(&mut self, framing: bool) {
        self.framing = framing;
    }

    /// Return these options with logical-record framing changed.
    #[must_use]
    pub const fn with_framing(mut self, framing: bool) -> Self {
        self.set_framing(framing);
        self
    }

    /// Return how a leading nonmatching fragment is handled while framing.
    #[must_use]
    pub const fn leading_fragment(&self) -> LeadingFragment {
        self.leading_fragment
    }

    /// Set how framing handles physical lines before the first header.
    pub const fn set_leading_fragment(&mut self, treatment: LeadingFragment) {
        self.leading_fragment = treatment;
    }

    /// Return these options with a different leading-fragment treatment.
    #[must_use]
    pub const fn with_leading_fragment(mut self, treatment: LeadingFragment) -> Self {
        self.set_leading_fragment(treatment);
        self
    }

    /// Return the retained decoded-body byte limit for each emitted record.
    #[must_use]
    pub const fn max_record_byte_size(&self) -> Option<u64> {
        self.max_record_byte_size
    }

    /// Set or clear the retained decoded-body byte limit for each record.
    pub const fn set_max_record_byte_size(&mut self, size: Option<u64>) {
        self.max_record_byte_size = size;
    }

    /// Return these options with a decoded-body byte limit for each record.
    #[must_use]
    pub const fn with_max_record_byte_size(mut self, size: u64) -> Self {
        self.set_max_record_byte_size(Some(size));
        self
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
    /// a source or diagnostic column under ASCII case folding.
    pub fn set_rowheader(&mut self, rowheader: Option<&str>) -> Result<()> {
        let expression = rowheader
            .map(|source| Expression::new(source, "$.rowheader"))
            .transpose()?;
        let captures = expression
            .as_ref()
            .map(|expression| {
                DataType::from_regex(expression.source.as_str(), true).and_then(|dtype| {
                    dtype.as_fields().map_or_else(
                        || {
                            Err(Error::InvalidRecord {
                                path: SmolStr::new_static("$.rowheader"),
                                reason: SmolStr::new_static(
                                    "expected regex capture inference to answer a Struct",
                                ),
                            })
                        },
                        |fields| Ok(fields.to_vec()),
                    )
                })
            })
            .transpose()?
            .unwrap_or_default();
        for capture in &captures {
            if BASE_COLUMNS
                .iter()
                .any(|base| base.eq_ignore_ascii_case(capture.name()))
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.rowheader"),
                    reason: format_smolstr!(
                        "expected named captures distinct from url, rownum, body, and dropped_byte_size, got {:?}",
                        capture.name()
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

    /// Return whether named captures are typed from their regex syntax.
    #[must_use]
    pub const fn autotype(&self) -> bool {
        self.autotype
    }

    /// Enable or disable syntax-directed capture typing.
    pub const fn set_autotype(&mut self, autotype: bool) {
        self.autotype = autotype;
    }

    /// Return these options with syntax-directed capture typing changed.
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
        self.captures.iter().map(Field::name)
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

    pub(crate) fn require_framing_rowheader(&self) -> Result<()> {
        if self.framing && self.rowheader.is_none() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.rowheader"),
                reason: SmolStr::new_static(
                    "logical-record framing requires a rowheader expression",
                ),
            });
        }
        Ok(())
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

    /// Build the decoder's source field without reading the resource.
    pub(crate) fn source_field(&self) -> Result<Field> {
        let mut fields = Vec::with_capacity(4 + self.captures.len());
        fields.push(DataType::Utf8.required_field("url"));
        if self.with_rownum.is_some() {
            fields.push(DataType::Int64.required_field("rownum"));
        }
        fields.push(DataType::Binary.required_field("body"));
        if self.max_record_byte_size.is_some() {
            fields.push(DataType::UInt64.nullable_field("dropped_byte_size"));
        }
        fields.extend(self.captures.iter().map(|capture| {
            let dtype = if self.autotype {
                match (capture.dtype(), self.timezone) {
                    (DataType::DateTime64 { unit, timezone }, Some(configured))
                        if timezone.is_naive() =>
                    {
                        DataType::DateTime64 {
                            unit: *unit,
                            timezone: configured,
                        }
                    }
                    (dtype, _) => dtype.clone(),
                }
            } else {
                DataType::Utf8
            };
            dtype.nullable_field(capture.name())
        }));
        Ok(DataType::from_fields(fields)?.required_field(self.name.clone()))
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
