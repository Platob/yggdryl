//! The declarative extractor: everything a reader needs, and nothing else.
//!
//! [`TextLineOptions`] is the whole configuration - what opens a record, what
//! counts as its header, what is left as the message, how the message is
//! trimmed, which captures are typed, how batches are bounded, and which zone a
//! naive timestamp is read in. It round-trips through the shared
//! [`Scalar`](crate::generic::Scalar), so a JSON, YAML, or TOML document already
//! readable by this crate **fully defines a reader** - no code, no callbacks,
//! no per-row work in any language.
//!
//! # Regex is the only structurer
//!
//! The transformation from a parsed record to typed columns happens through the
//! pattern and nothing else. Named capture groups become nullable columns in
//! group order - that *is* the custom-field mechanism. There is no format
//! string, no `strptime`, no callback, and no second structuring mechanism. If
//! a shape cannot be expressed as a regex, it is out of scope.

use std::str::FromStr;

use regex::Regex;
use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::generic::Scalar;
use crate::iceberg::PrimitiveType;
use crate::{DataType, Error, Field, Result, TimeUnit, Timezone};

use super::sep::LineSep;
use super::strip::Strip;

/// The columns every line projection opens with, in emission order.
pub(crate) const BASE_COLUMNS: [&str; 10] = [
    "url", "rownum", "date", "time", "unix", "hash", "header", "message", "offset", "lines",
];

/// The columns log mode adds after the base ones, in emission order.
///
/// Fixed and always emitted, never discovered from the data: a line without a
/// level gets `null`, not a missing column, so
/// [`TextLineOptions::field`] still answers from configuration alone.
pub(crate) const LOG_COLUMNS: [&str; 3] = ["level", "logger", "thread"];

/// The name of the root Struct Field the projection reports.
///
/// The record surface infers roots under the same name, so a schema read off
/// this projection and one read off a record encoding compose without rename.
pub(crate) const ROOT_NAME: &str = "row";

/// What opens a record.
///
/// Three states rather than two, because "one record per line" and "a record
/// opens where a timestamp opens" are both defaults something wants, and
/// collapsing them onto `Option<Regex>` would make one of the two surfaces
/// lie: [`read_lines`](crate::io::IOBase::read_lines) means *lines*, and a
/// plain text file must not read as one enormous record just because it
/// carries no timestamps.
#[derive(Clone, Debug, Default)]
pub enum Opening {
    /// Every line opens a record - the plain line surface, and the default.
    #[default]
    EveryLine,
    /// A record opens where a **timestamp** opens: log mode.
    ///
    /// Exact where a regex is approximate, it yields the header span for free,
    /// and it avoids compiling and running an expression against every line of
    /// a stack-trace-heavy log. Reached with no expression written anywhere -
    /// see [`TextLineOptions::for_logs`].
    Timestamp,
    /// A record opens where this expression matches the line.
    Pattern(Regex),
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
enum OpeningIdentity<'a> {
    EveryLine,
    Timestamp,
    Pattern(&'a str),
}

impl Opening {
    fn identity(&self) -> OpeningIdentity<'_> {
        match self {
            Self::EveryLine => OpeningIdentity::EveryLine,
            Self::Timestamp => OpeningIdentity::Timestamp,
            // The expression source is the declarative setting; the compiled
            // automaton is derived state and is deliberately not identity.
            Self::Pattern(pattern) => OpeningIdentity::Pattern(pattern.as_str()),
        }
    }

    /// The expression, when one was supplied.
    #[must_use]
    pub const fn pattern(&self) -> Option<&Regex> {
        match self {
            Self::Pattern(pattern) => Some(pattern),
            _ => None,
        }
    }

    /// Whether records open where a timestamp opens.
    #[must_use]
    pub const fn is_timestamp(&self) -> bool {
        matches!(self, Self::Timestamp)
    }

    /// Return the deterministic hash of the declared opening rule.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(&self.identity())
    }
}

impl PartialEq for Opening {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Opening {}

impl std::hash::Hash for Opening {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

impl Ord for Opening {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl PartialOrd for Opening {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The settings a text-line read or write runs under.
///
/// Construction compiles and validates every expression, so holding a
/// `TextLineOptions` is holding a schema already known to be emittable -
/// [`Self::field`] answers with no resource in sight, which is what a caller
/// creating an Iceberg table before the first log line needs.
///
/// ```
/// use yggdryl::text::TextLineOptions;
///
/// # fn main() -> yggdryl::Result<()> {
/// // Everything is optional. Unset, records open where a timestamp opens.
/// let logs = TextLineOptions::new();
/// assert!(logs.pattern().is_none());
///
/// // A pattern overrides detection, and its captures become columns.
/// let options = TextLineOptions::with_pattern(r"^\[(?<level>[A-Z]+)\]")?;
/// assert_eq!(options.capture_names().collect::<Vec<_>>(), ["level"]);
/// assert_eq!(options.field().name(), "row");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct TextLineOptions {
    /// What opens a record.
    opening: Opening,
    /// What within the opening line is the header. Unset means the pattern is.
    header: Option<Regex>,
    /// The record terminator. Unset means the flexible default.
    linesep: Option<LineSep>,
    lstrip: Strip,
    rstrip: Strip,
    byte_size: Option<usize>,
    batch_size: Option<usize>,
    timestamp_capture: Option<SmolStr>,
    timezone: Option<Timezone>,
    custom_fields: Vec<(SmolStr, Scalar)>,
    capture_types: Vec<(SmolStr, DataType)>,
    /// Every named capture of both expressions, in emission order.
    captures: Vec<SmolStr>,
    /// How many of `captures` come from the opening pattern.
    pattern_captures: usize,
    /// The emitted root, rebuilt on every effective mutation.
    field: Field,
}

/// The complete declared configuration. Captures and the emitted field are
/// derived from these members and therefore cannot create a second identity.
#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct TextLineIdentity<'a> {
    opening: OpeningIdentity<'a>,
    header: Option<&'a str>,
    linesep: Option<&'a LineSep>,
    lstrip: &'a Strip,
    rstrip: &'a Strip,
    byte_size: Option<usize>,
    batch_size: Option<usize>,
    timestamp_capture: Option<&'a SmolStr>,
    timezone: Option<&'a Timezone>,
    custom_fields: &'a [(SmolStr, Scalar)],
    capture_types: &'a [(SmolStr, DataType)],
}

impl TextLineOptions {
    fn identity(&self) -> TextLineIdentity<'_> {
        TextLineIdentity {
            opening: self.opening.identity(),
            header: self.header.as_ref().map(Regex::as_str),
            linesep: self.linesep.as_ref(),
            lstrip: &self.lstrip,
            rstrip: &self.rstrip,
            byte_size: self.byte_size,
            batch_size: self.batch_size,
            timestamp_capture: self.timestamp_capture.as_ref(),
            timezone: self.timezone.as_ref(),
            custom_fields: &self.custom_fields,
            capture_types: &self.capture_types,
        }
    }

    /// Return a deterministic hash of every declared extractor setting.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(&self.identity())
    }
}

impl PartialEq for TextLineOptions {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for TextLineOptions {}

impl std::hash::Hash for TextLineOptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

impl Ord for TextLineOptions {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl PartialOrd for TextLineOptions {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Default for TextLineOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl TextLineOptions {
    /// Decoded input bytes per batch when none is declared.
    ///
    /// Byte-driven sizing governs the default read so a corpus of 40-byte lines
    /// and one of 4 KB stack traces produce comparably sized batches - a row
    /// bound alone makes the first batch tiny and the second enormous.
    pub const DEFAULT_BYTE_SIZE: usize = 8 * 1024 * 1024;

    /// Rows per batch when none is declared - the guard beside the byte bound.
    ///
    /// High enough that byte sizing is what normally closes a batch, low enough
    /// that a corpus of very short records still bounds its builders.
    pub const DEFAULT_BATCH_SIZE: usize = 65_536;

    /// The zero-configuration extractor: one record per line, the flexible
    /// terminator, whitespace-trimmed messages.
    ///
    /// This is what [`read_lines`](crate::io::IOBase::read_lines) runs under, so
    /// that method means what its name says. [`Self::for_logs`] is the other
    /// zero-configuration shape, and it also needs no expression written
    /// anywhere.
    #[must_use]
    pub fn new() -> Self {
        let mut options = Self {
            opening: Opening::default(),
            header: None,
            linesep: None,
            lstrip: Strip::default(),
            rstrip: Strip::default(),
            byte_size: None,
            batch_size: None,
            timestamp_capture: None,
            timezone: None,
            custom_fields: Vec::new(),
            capture_types: Vec::new(),
            captures: Vec::new(),
            pattern_captures: 0,
            field: Field::new(ROOT_NAME, DataType::Null, false),
        };
        // The base schema cannot fail: its columns are this module's own.
        options.field =
            options
                .rebuild_schema()
                .unwrap_or(Field::new(ROOT_NAME, DataType::Null, false));
        options
    }

    /// The zero-configuration **log** extractor: records open where a
    /// timestamp opens.
    ///
    /// No expression is written anywhere, and the schema gains the fixed
    /// The fixed log columns - `level`, `logger`, `thread` - always emitted and
    /// always nullable, so it still follows from the options alone.
    #[must_use]
    pub fn for_logs() -> Self {
        let mut options = Self::new();
        // The base and log columns are this module's own, so this cannot fail.
        let _ = options.set_opening(Opening::Timestamp);
        options
    }

    /// Compile a record-opening pattern into options.
    ///
    /// `pattern` opens a record exactly as detection does, and its named
    /// capture groups become nullable columns in group order, typed by the
    /// capture's own sub-pattern where the deterministic table recognizes it -
    /// `(?<thread_id>\d+)` is an `int64` column - and `utf8` otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture group name
    /// collides with a base column or another capture.
    pub fn with_pattern(pattern: &str) -> Result<Self> {
        Self::new().try_with_pattern(pattern)
    }

    /// Borrow the record-opening pattern's source text, when one is set.
    ///
    /// Unset, records open where a timestamp opens - see
    /// [`super::log`](super::log).
    #[must_use]
    pub fn pattern(&self) -> Option<&str> {
        self.opening.pattern().map(Regex::as_str)
    }

    /// What opens a record.
    #[must_use]
    pub const fn opening(&self) -> &Opening {
        &self.opening
    }

    /// Choose what opens a record.
    ///
    /// # Errors
    ///
    /// Returns an error when a capture name collides under the new mode -
    /// switching *into* log mode reserves `level`, `logger`, and `thread`, so a
    /// pattern already capturing one of those is refused rather than shadowed.
    /// Failure leaves the options unchanged.
    pub fn set_opening(&mut self, opening: Opening) -> Result<()> {
        self.replace_expressions(opening, self.header.clone())
    }

    /// Return these options with a different record-opening rule.
    ///
    /// # Errors
    ///
    /// Returns an error when a capture name collides under the new mode.
    pub fn try_with_opening(mut self, opening: Opening) -> Result<Self> {
        self.set_opening(opening)?;
        Ok(self)
    }

    /// Set the record-opening pattern; `None` restores timestamp detection.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides. Failure leaves the options unchanged.
    pub fn set_pattern(&mut self, pattern: Option<&str>) -> Result<()> {
        let opening = match pattern {
            Some(pattern) => Opening::Pattern(compiled(pattern, "pattern")?),
            // Clearing a pattern restores the plain line surface, not log mode:
            // log mode is a deliberate choice, never something a caller falls
            // into by removing an expression.
            None => Opening::EveryLine,
        };
        self.replace_expressions(opening, self.header.clone())
    }

    /// Return these options with a record-opening pattern.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides.
    pub fn try_with_pattern(mut self, pattern: &str) -> Result<Self> {
        self.set_pattern(Some(pattern))?;
        Ok(self)
    }

    /// Borrow the header pattern's source text, when one is set.
    ///
    /// Unset - the default - the record-opening pattern *is* the header, which
    /// is what the line projection has always done.
    #[must_use]
    pub fn header(&self) -> Option<&str> {
        self.header.as_ref().map(Regex::as_str)
    }

    /// Set a header pattern separate from the record-opening one.
    ///
    /// This is what lets one extractor read a log whose entries are opened by a
    /// cheap anchored timestamp check but whose header is a richer expression
    /// not worth running against every line: the opening pattern decides *where
    /// a record begins*, the header pattern decides *what within it is the
    /// header*.
    ///
    /// Both are matched against the **opening line only**. A greedy class
    /// matched over the whole record would otherwise cross into continuation
    /// lines and call text a header that the opening line never contained.
    ///
    /// Capture columns are the union of both expressions' named groups -
    /// opening-pattern groups first, header groups after, each in its own group
    /// order. A record whose opening line the header pattern did *not* match
    /// takes the unmatched-preamble shape: null header, null timestamp columns,
    /// null captures, whole record as message. There is one rule for "no header
    /// here", not two.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides with a base column, the opening pattern's captures, or another
    /// header capture. Failure leaves the options unchanged.
    pub fn set_header(&mut self, header: Option<&str>) -> Result<()> {
        let compiled = match header {
            Some(header) => Some(compiled(header, "header")?),
            None => None,
        };
        self.replace_expressions(self.opening.clone(), compiled)
    }

    /// Return these options with a separate header pattern.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides.
    pub fn try_with_header(mut self, header: &str) -> Result<Self> {
        self.set_header(Some(header))?;
        Ok(self)
    }

    /// Return the pinned record terminator, when one is set.
    ///
    /// Unset - the default and the recommended path - the reader accepts `\n`,
    /// `\r\n`, and a lone `\r`, **mixed within one resource**, and a write uses
    /// the platform-neutral `\n`.
    #[must_use]
    pub const fn linesep(&self) -> Option<&LineSep> {
        self.linesep.as_ref()
    }

    /// Pin the record terminator; `None` restores the flexible default.
    pub fn set_linesep(&mut self, linesep: Option<LineSep>) {
        self.linesep = linesep;
    }

    /// Return these options with a pinned record terminator.
    #[must_use]
    pub fn with_linesep(mut self, linesep: LineSep) -> Self {
        self.set_linesep(Some(linesep));
        self
    }

    /// The terminator a write emits.
    ///
    /// **Flexible on read, deterministic on write**: unset means `\n`, never
    /// the host's line ending, because a resource's bytes must not depend on
    /// which machine wrote them.
    #[must_use]
    pub fn write_linesep(&self) -> &[u8] {
        self.linesep.as_ref().map_or(b"\n", LineSep::as_bytes)
    }

    /// How the message's leading edge is trimmed.
    #[must_use]
    pub const fn lstrip(&self) -> &Strip {
        &self.lstrip
    }

    /// How the message's trailing edge is trimmed.
    #[must_use]
    pub const fn rstrip(&self) -> &Strip {
        &self.rstrip
    }

    /// Set how the message's leading edge is trimmed.
    ///
    /// # The hash moves with this
    ///
    /// The `hash` column covers the **message**, so a strip setting changes it.
    /// The hash stays deterministic - but it is now deterministic *given the
    /// options*, and two readers configured differently hash the same log line
    /// differently. A join key that drifts silently with configuration would be
    /// the worst outcome of this option, so it is stated here and on the column.
    pub fn set_lstrip(&mut self, lstrip: Strip) {
        self.lstrip = lstrip;
    }

    /// Set how the message's trailing edge is trimmed.
    ///
    /// See [`Self::set_lstrip`]: this moves the `hash` column too.
    pub fn set_rstrip(&mut self, rstrip: Strip) {
        self.rstrip = rstrip;
    }

    /// Return these options with a leading-edge strip mode.
    #[must_use]
    pub fn with_lstrip(mut self, lstrip: Strip) -> Self {
        self.set_lstrip(lstrip);
        self
    }

    /// Return these options with a trailing-edge strip mode.
    #[must_use]
    pub fn with_rstrip(mut self, rstrip: Strip) -> Self {
        self.set_rstrip(rstrip);
        self
    }

    /// Return the decoded-input-bytes bound a batch closes at, if declared.
    #[must_use]
    pub const fn byte_size(&self) -> Option<usize> {
        self.byte_size
    }

    /// Set the decoded-input-bytes bound; `None` restores the default.
    ///
    /// This measures the **decoded input bytes** of the records appended - each
    /// record's length plus its terminator, accounted as records arrive - not
    /// Arrow buffer memory. It is not an allocation cap, and reading it as one
    /// would be a mistake worth avoiding: a `utf8` column's Arrow buffers are
    /// close to the input size, but a typed capture column is not.
    pub fn set_byte_size(&mut self, byte_size: Option<usize>) {
        self.byte_size = byte_size;
    }

    /// Return these options with a decoded-input-bytes batch bound.
    #[must_use]
    pub fn with_byte_size(mut self, byte_size: usize) -> Self {
        self.set_byte_size(Some(byte_size));
        self
    }

    /// Return the row-per-batch bound, if declared.
    #[must_use]
    pub const fn batch_size(&self) -> Option<usize> {
        self.batch_size
    }

    /// Set the row-per-batch bound; `None` restores the default.
    pub fn set_batch_size(&mut self, batch_size: Option<usize>) {
        self.batch_size = batch_size;
    }

    /// Return these options with a row-per-batch bound.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.set_batch_size(Some(batch_size));
        self
    }

    /// The decoded input bytes a batch is closed at.
    #[must_use]
    pub fn effective_byte_size(&self) -> usize {
        self.byte_size.unwrap_or(Self::DEFAULT_BYTE_SIZE).max(1)
    }

    /// The row count a batch is closed at.
    #[must_use]
    pub fn effective_batch_size(&self) -> usize {
        self.batch_size.unwrap_or(Self::DEFAULT_BATCH_SIZE).max(1)
    }

    /// Return the capture the entry timestamp is read from, when one is set.
    #[must_use]
    pub fn timestamp_capture(&self) -> Option<&str> {
        self.timestamp_capture.as_deref()
    }

    /// Point the timestamp parser at a named capture group.
    ///
    /// `None` restores the default - the front of the matched header.
    ///
    /// # Errors
    ///
    /// Returns an error naming the available captures when `capture` is not a
    /// named group of either expression. Failure leaves the options unchanged.
    pub fn set_timestamp_capture(&mut self, capture: Option<SmolStr>) -> Result<()> {
        if let Some(name) = &capture {
            if !self.captures.iter().any(|held| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_smolstr!("a named capture group ({:?})", self.captures),
                        format_smolstr!("{name:?}"),
                    ),
                });
            }
        }
        self.timestamp_capture = capture;
        Ok(())
    }

    /// Return these options reading the timestamp from a named capture.
    ///
    /// # Errors
    ///
    /// Returns an error when `capture` is not a named group.
    pub fn try_with_timestamp_capture(mut self, capture: impl Into<SmolStr>) -> Result<Self> {
        self.set_timestamp_capture(Some(capture.into()))?;
        Ok(self)
    }

    /// Return the zone a naive timestamp is read in, when one is set.
    #[must_use]
    pub const fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// Read naive timestamps in `timezone`, so `unix` is a real instant.
    ///
    /// **An offset present in the timestamp text always wins; the option
    /// applies only to timestamps that carry none.** The alternative - the
    /// option overriding the text - would silently rewrite data the log author
    /// was explicit about.
    ///
    /// Unset, the behavior is exactly what it has always been: the civil
    /// reading counted from the epoch, with no zone applied.
    ///
    /// # Errors
    ///
    /// Returns an error naming a zone the registry does not know. The check is
    /// **here**, when the options are built - not at the first row, when a read
    /// is already streaming.
    pub fn set_timezone(&mut self, timezone: Option<Timezone>) -> Result<()> {
        if let Some(zone) = &timezone {
            if !zone.is_known() {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.timezone"),
                    reason: crate::text::expected_got(
                        "a time zone the registry knows, a fixed offset, or UTC",
                        format_smolstr!("{zone}"),
                    ),
                });
            }
        }
        self.timezone = timezone;
        Ok(())
    }

    /// Return these options reading naive timestamps in `timezone`.
    ///
    /// # Errors
    ///
    /// Returns an error naming a zone the registry does not know.
    pub fn try_with_timezone(mut self, timezone: Timezone) -> Result<Self> {
        self.set_timezone(Some(timezone))?;
        Ok(self)
    }

    /// Borrow the named capture groups, in group and column order.
    ///
    /// The union of both expressions': the opening pattern's first, the header
    /// pattern's after.
    pub fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.captures.iter().map(SmolStr::as_str)
    }

    /// Borrow the constant columns appended to every row, in column order.
    #[must_use]
    pub fn custom_fields(&self) -> &[(SmolStr, Scalar)] {
        &self.custom_fields
    }

    /// Replace the constant columns appended to every row.
    ///
    /// Each value's datatype is validated through the strict Iceberg codec
    /// here, at option construction, so a column Iceberg cannot spell fails
    /// before the first batch rather than at table-append time.
    ///
    /// # Errors
    ///
    /// Returns the codec's own rejection under the column's path, or a
    /// collision with a base, capture, or earlier custom column. Failure leaves
    /// the options unchanged.
    pub fn set_custom_fields(&mut self, custom_fields: Vec<(SmolStr, Scalar)>) -> Result<()> {
        self.rebuild_with(|options| &mut options.custom_fields, custom_fields)
    }

    /// Return these options with constant columns appended to every row.
    ///
    /// This is how a caller stamps `source`, `session`, or `venue` onto a
    /// file's rows - and, marked as partition columns in a declared Iceberg
    /// schema, how one file's rows land in one partition.
    ///
    /// # Errors
    ///
    /// Returns an error when a column's datatype has no Iceberg spelling or its
    /// name collides.
    pub fn try_with_custom_fields<I, N>(mut self, custom_fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, Scalar)>,
        N: Into<SmolStr>,
    {
        self.set_custom_fields(
            custom_fields
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        )?;
        Ok(self)
    }

    /// Borrow the declared capture column datatypes, in declaration order.
    #[must_use]
    pub fn capture_types(&self) -> &[(SmolStr, DataType)] {
        &self.capture_types
    }

    /// Declare capture column datatypes, overriding the inferred ones.
    ///
    /// A declared capture parses through the one cast definition as each batch
    /// closes - `(?<price>[0-9.]+)` declared `decimal(9, 2)` lands typed - and a
    /// declaration of `utf8` turns an inferred numeric column back into text.
    /// The cast is **strict**: a captured text the datatype cannot read is an
    /// error naming the row and the column, never a silent null.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is not a capture group, is declared twice,
    /// or names a datatype the created Iceberg tables cannot declare. Failure
    /// leaves the options unchanged.
    pub fn set_capture_types(&mut self, capture_types: Vec<(SmolStr, DataType)>) -> Result<()> {
        for (index, (name, _)) in capture_types.iter().enumerate() {
            if !self.captures.iter().any(|held| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_smolstr!("a named capture group ({:?})", self.captures),
                        format_smolstr!("{name:?}"),
                    ),
                });
            }
            if capture_types[..index].iter().any(|(held, _)| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        "one datatype declaration per capture",
                        format_smolstr!("{name:?} declared twice"),
                    ),
                });
            }
        }
        self.rebuild_with(|options| &mut options.capture_types, capture_types)
    }

    /// Return these options with declared capture column datatypes.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is not a capture group or a datatype has no
    /// Iceberg spelling.
    pub fn try_with_capture_types<I, N>(mut self, capture_types: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, DataType)>,
        N: Into<SmolStr>,
    {
        self.set_capture_types(
            capture_types
                .into_iter()
                .map(|(name, data_type)| (name.into(), data_type))
                .collect(),
        )?;
        Ok(self)
    }

    /// Borrow the non-null Struct root the projection emits.
    ///
    /// Base columns, then - in log mode - the recognized token columns, then one
    /// nullable column per named capture, then the custom constant columns.
    /// Every column passes the strict Iceberg codec unchanged, so this root
    /// creates an Iceberg table as it stands, **with no resource in sight**.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// Consume these options into the root Struct Field they emit.
    #[must_use]
    pub fn into_field(self) -> Field {
        self.field
    }

    /// Return whether records open where a timestamp opens.
    ///
    /// Log mode adds the fixed `level`, `logger`, and `thread` columns to the
    /// schema, always nullable
    /// and always emitted, so the emitted shape still follows from the options
    /// alone - never from what the first batch happened to contain.
    #[must_use]
    pub const fn is_log_mode(&self) -> bool {
        self.opening.is_timestamp()
    }

    /// Borrow the compiled expression the header is matched with.
    ///
    /// The header pattern when one is set, otherwise the opening pattern - the
    /// default in which the two roles are one expression.
    pub(crate) fn header_expression(&self) -> Option<&Regex> {
        self.header.as_ref().or_else(|| self.opening.pattern())
    }

    /// Install a new expression pair, revalidating every derived column.
    fn replace_expressions(&mut self, opening: Opening, header: Option<Regex>) -> Result<()> {
        let (captures, pattern_captures) = union_captures(&opening, header.as_ref())?;
        let held = (
            std::mem::replace(&mut self.opening, opening),
            std::mem::replace(&mut self.header, header),
            std::mem::replace(&mut self.captures, captures),
            std::mem::replace(&mut self.pattern_captures, pattern_captures),
        );
        // A declaration or a timestamp capture may name a group that is gone.
        self.capture_types
            .retain(|(name, _)| self.captures.iter().any(|held| held == name));
        if let Some(name) = &self.timestamp_capture {
            if !self.captures.iter().any(|held| held == name) {
                self.timestamp_capture = None;
            }
        }
        match self.rebuild_schema() {
            Ok(field) => {
                self.field = field;
                Ok(())
            }
            Err(error) => {
                self.opening = held.0;
                self.header = held.1;
                self.captures = held.2;
                self.pattern_captures = held.3;
                Err(error)
            }
        }
    }

    /// Swap one setting in, rebuild the schema, and restore it on failure.
    ///
    /// The transactional half every setter shares: an error leaves the options
    /// exactly as they were.
    fn rebuild_with<T>(&mut self, setting: impl Fn(&mut Self) -> &mut T, value: T) -> Result<()> {
        let held = std::mem::replace(setting(self), value);
        match self.rebuild_schema() {
            Ok(field) => {
                self.field = field;
                Ok(())
            }
            Err(error) => {
                *setting(self) = held;
                Err(error)
            }
        }
    }

    /// Assemble the emitted root after re-running every column validation.
    fn rebuild_schema(&self) -> Result<Field> {
        for (name, data_type) in &self.capture_types {
            iceberg_safe(name, data_type)?;
        }
        for (index, (name, value)) in self.custom_fields.iter().enumerate() {
            if is_reserved_column(name, self.is_log_mode()) {
                return Err(collision(name, "a base column of the line projection"));
            }
            if self
                .captures
                .iter()
                .any(|held| held.eq_ignore_ascii_case(name))
            {
                return Err(collision(name, "a capture group of the pattern"));
            }
            if self.custom_fields[..index]
                .iter()
                .any(|(held, _)| held.eq_ignore_ascii_case(name))
            {
                return Err(collision(name, "another custom field"));
            }
            let data_type = value.data_type().map_err(|error| Error::InvalidRecord {
                path: format_smolstr!("$.{name}"),
                reason: error.to_smolstr(),
            })?;
            iceberg_safe(name, &data_type)?;
        }

        let mut fields = vec![
            DataType::Utf8.required_field("url"),
            DataType::Int64.required_field("rownum"),
            DataType::Date32.nullable_field("date"),
            DataType::Time64(TimeUnit::Microsecond).nullable_field("time"),
            DataType::Int64.nullable_field("unix"),
            DataType::Int64.required_field("hash"),
            DataType::Utf8.nullable_field("header"),
            DataType::Utf8.required_field("message"),
            DataType::Int64.required_field("offset"),
            DataType::Int32.required_field("lines"),
        ];
        if self.is_log_mode() {
            for name in LOG_COLUMNS {
                fields.push(DataType::Utf8.nullable_field(name));
            }
        }
        for data_type in self.resolved_capture_types() {
            let capture = &self.captures[fields.len() - self.leading_column_count()];
            fields.push(data_type.nullable_field(capture.clone()));
        }
        for (name, value) in &self.custom_fields {
            let data_type = value.data_type()?;
            // Only a null constant needs a nullable column; a typed constant is
            // present in every row.
            fields.push(Field::new(
                name.clone(),
                data_type,
                matches!(value, Scalar::Null),
            ));
        }
        Ok(DataType::from_fields(fields)?.required_field(ROOT_NAME))
    }

    /// How many columns precede the capture columns.
    pub(crate) const fn leading_column_count(&self) -> usize {
        if self.is_log_mode() {
            BASE_COLUMNS.len() + LOG_COLUMNS.len()
        } else {
            BASE_COLUMNS.len()
        }
    }

    /// Resolve each capture column's datatype, in capture order.
    ///
    /// A declaration wins; then the deterministic sub-pattern table; then
    /// `utf8`.
    pub(crate) fn resolved_capture_types(&self) -> Vec<DataType> {
        let mut bodies = Vec::new();
        if let Some(pattern) = self.opening.pattern() {
            bodies.extend(named_group_bodies(pattern.as_str()));
        }
        if let Some(header) = &self.header {
            bodies.extend(named_group_bodies(header.as_str()));
        }
        self.captures
            .iter()
            .map(|capture| {
                if let Some((_, data_type)) =
                    self.capture_types.iter().find(|(name, _)| name == capture)
                {
                    return data_type.clone();
                }
                bodies
                    .iter()
                    .find(|(name, _)| name == capture)
                    .and_then(|(_, body)| inferred_capture_type(body))
                    .unwrap_or(DataType::Utf8)
            })
            .collect()
    }
}

/// Compile one expression, naming which option it belongs to on failure.
fn compiled(pattern: &str, option: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|error| Error::InvalidRecord {
        path: format_smolstr!("$.{option}"),
        reason: format_smolstr!("expected a valid line pattern: {error}"),
    })
}

/// Every named capture of both expressions, opening first, header after.
///
/// Collisions - with a base column, with a log column, with each other, and
/// ASCII-case-insensitive near-collisions - are refused here, once, rather than
/// by a second check per expression.
fn union_captures(opening: &Opening, header: Option<&Regex>) -> Result<(Vec<SmolStr>, usize)> {
    let mut captures: Vec<SmolStr> = Vec::new();
    let mut pattern_captures = 0;
    let log_mode = opening.is_timestamp();
    for (index, expression) in [opening.pattern(), header].into_iter().enumerate() {
        let Some(expression) = expression else {
            continue;
        };
        for name in expression.capture_names().flatten() {
            let name = SmolStr::new(name);
            // The log columns are reserved only where they are emitted: outside
            // log mode `level` is an ordinary and entirely natural capture name.
            if is_reserved_column(&name, log_mode) {
                return Err(collision(&name, "a base column of the line projection"));
            }
            // The regex engine keeps `level` and `LEVEL` distinct; column names
            // resolve ASCII case-insensitively everywhere a cast or a selection
            // matches them, so the pair would be ambiguous.
            if captures.iter().any(|held| held.eq_ignore_ascii_case(&name)) {
                return Err(collision(&name, "another capture group"));
            }
            captures.push(name);
        }
        if index == 0 {
            pattern_captures = captures.len();
        }
    }
    Ok((captures, pattern_captures))
}

/// Whether `name` spells a reserved column, compared as every cast matches.
fn is_reserved_column(name: &str, log_mode: bool) -> bool {
    BASE_COLUMNS
        .iter()
        .chain(log_mode.then_some(LOG_COLUMNS.iter()).into_iter().flatten())
        .any(|base| base.eq_ignore_ascii_case(name))
}

/// Report a column name already taken by `taken_by`.
fn collision(name: &str, taken_by: &str) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason: crate::text::expected_got(
            "a distinct column name",
            format_smolstr!("{name:?}, already {taken_by}"),
        ),
    }
}

/// Refuse a column datatype the created Iceberg tables cannot declare.
///
/// The strict codec is the arbiter: what it refuses here would have been
/// refused at table-append time, when rows already streamed. The v3-only types
/// are refused too, because the tables this crate creates are format v2 and a
/// column they cannot legally declare must fail before the metadata is
/// committed.
fn iceberg_safe(name: &str, data_type: &DataType) -> Result<()> {
    let primitive =
        PrimitiveType::from_data_type(data_type).map_err(|error| Error::InvalidRecord {
            path: format_smolstr!("$.{name}"),
            reason: error.to_smolstr(),
        })?;
    if matches!(
        primitive,
        PrimitiveType::Unknown | PrimitiveType::TimestampNs | PrimitiveType::TimestamptzNs
    ) {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{name}"),
            reason: crate::text::expected_got(
                "a column every Iceberg format version spells (microsecond timestamps; no null \
                 constants)",
                format_smolstr!("{primitive}, which Iceberg adds in format v3"),
            ),
        });
    }
    Ok(())
}

/// The deterministic sub-pattern table behind capture type inference.
///
/// A capture whose *whole* body is one of these exact spellings names a numeric
/// column; every other body - however numeric it looks - stays text, because
/// inference is a closed table, never a guess about what a regex might match. A
/// declaration overrides in both directions.
fn inferred_capture_type(body: &str) -> Option<DataType> {
    match body {
        r"\d+" | "[0-9]+" | r"-?\d+" | r"[+-]?\d+" => Some(DataType::Int64),
        r"\d+\.\d+" | r"-?\d+\.\d+" | r"[+-]?\d+\.\d+" => Some(DataType::Float64),
        _ => None,
    }
}

/// The body text of each named group in an already-compiled pattern.
///
/// A byte scan, not a second regex grammar: it only pairs parentheses -
/// skipping escapes and character classes, where a parenthesis is a literal -
/// and remembers where a `(?<name>` or `(?P<name>` group's body spans. It runs
/// only on patterns the engine has already compiled, and every index it slices
/// at is an ASCII byte, which UTF-8 guarantees is a character boundary.
fn named_group_bodies(pattern: &str) -> Vec<(SmolStr, SmolStr)> {
    let bytes = pattern.as_bytes();
    let mut bodies = Vec::new();
    // One entry per open group: the name and body start when it is named.
    let mut open: Vec<Option<(SmolStr, usize)>> = Vec::new();
    let mut in_class = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' if !in_class => {
                in_class = true;
                index += 1;
            }
            b']' if in_class => {
                in_class = false;
                index += 1;
            }
            b'(' if !in_class => {
                let rest = &pattern[index..];
                let name_start = if rest.starts_with("(?P<") {
                    Some(index + 4)
                } else if rest.starts_with("(?<")
                    && !rest.starts_with("(?<=")
                    && !rest.starts_with("(?<!")
                {
                    Some(index + 3)
                } else {
                    None
                };
                if let Some(start) = name_start {
                    if let Some(close) = pattern[start..].find('>') {
                        open.push(Some((
                            SmolStr::new(&pattern[start..start + close]),
                            start + close + 1,
                        )));
                        index = start + close + 1;
                        continue;
                    }
                }
                open.push(None);
                index += 1;
            }
            b')' if !in_class => {
                if let Some(Some((name, body_start))) = open.pop() {
                    bodies.push((name, SmolStr::new(&pattern[body_start..index])));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    bodies
}

impl FromStr for TextLineOptions {
    type Err = Error;

    /// Read options from a record-opening pattern.
    fn from_str(value: &str) -> Result<Self> {
        Self::with_pattern(value)
    }
}

// ---------------------------------------------------------------------------
// The declarative round trip.
//
// A JSON, YAML, or TOML document already readable by this crate fully defines
// a reader. That is the point of the whole options value: a caller configures
// an optimized Arrow reader with *configuration only* - no Rust, no Python
// callbacks, no per-row code in any language.
// ---------------------------------------------------------------------------

impl TextLineOptions {
    /// Project these options onto the shared structural [`Scalar`].
    ///
    /// Only what is actually set is emitted, so a default extractor is an empty
    /// mapping and a document round-trips without accumulating noise. The keys
    /// are the option names, in a fixed order.
    ///
    /// ```
    /// use yggdryl::generic::Scalar;
    /// use yggdryl::text::TextLineOptions;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let options = TextLineOptions::with_pattern(r"^\[(?<level>[A-Z]+)\]")?
    ///     .with_byte_size(1 << 20);
    /// let value = options.into_value();
    ///
    /// assert_eq!(
    ///     value.get_key_str("pattern").and_then(Scalar::as_utf8),
    ///     Some(r"^\[(?<level>[A-Z]+)\]"),
    /// );
    /// assert_eq!(TextLineOptions::from_value(value)?.byte_size(), Some(1 << 20));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn into_value(&self) -> Scalar {
        let key = |name: &str| Scalar::String(SmolStr::new(name));
        let mut entries: Vec<(Scalar, Scalar)> = Vec::new();
        match &self.opening {
            Opening::EveryLine => {}
            Opening::Timestamp => entries.push((key("opening"), key("timestamp"))),
            Opening::Pattern(pattern) => {
                entries.push((
                    key("pattern"),
                    Scalar::String(SmolStr::new(pattern.as_str())),
                ));
            }
        }
        if let Some(header) = &self.header {
            entries.push((key("header"), Scalar::String(SmolStr::new(header.as_str()))));
        }
        if let Some(linesep) = &self.linesep {
            entries.push((key("linesep"), Scalar::String(escaped(linesep.as_bytes()))));
        }
        if !matches!(self.lstrip, Strip::Whitespace) {
            entries.push((key("lstrip"), Scalar::String(self.lstrip.to_smolstr())));
        }
        if !matches!(self.rstrip, Strip::Whitespace) {
            entries.push((key("rstrip"), Scalar::String(self.rstrip.to_smolstr())));
        }
        if let Some(byte_size) = self.byte_size {
            entries.push((key("byte_size"), Scalar::U64(byte_size as u64)));
        }
        if let Some(batch_size) = self.batch_size {
            entries.push((key("batch_size"), Scalar::U64(batch_size as u64)));
        }
        if let Some(capture) = &self.timestamp_capture {
            entries.push((key("timestamp_capture"), Scalar::String(capture.clone())));
        }
        if let Some(timezone) = &self.timezone {
            entries.push((
                key("timezone"),
                Scalar::String(SmolStr::new(timezone.as_str())),
            ));
        }
        if !self.capture_types.is_empty() {
            entries.push((
                key("capture_types"),
                Scalar::from_mapping(
                    self.capture_types.iter().map(|(name, data_type)| {
                        (key(name), Scalar::String(data_type.to_smolstr()))
                    }),
                )
                .unwrap_or(Scalar::Null),
            ));
        }
        if !self.custom_fields.is_empty() {
            entries.push((
                key("custom_fields"),
                Scalar::from_mapping(
                    self.custom_fields
                        .iter()
                        .map(|(name, value)| (key(name), value.clone())),
                )
                .unwrap_or(Scalar::Null),
            ));
        }
        Scalar::from_mapping(entries).unwrap_or(Scalar::Null)
    }

    /// Read options back from the shared structural [`Scalar`].
    ///
    /// This is what makes a reader **fully specifiable from a document**: parse
    /// a config file with [`text::from_utf8`](crate::text::from_utf8), hand the
    /// value here, and the reader is built - pattern, header, terminator,
    /// batch bounds, typed captures, constant columns, timestamp capture, and
    /// zone. Every value is validated exactly as the setters validate it, so a
    /// bad document fails here rather than at the first row.
    ///
    /// # Errors
    ///
    /// Returns an error naming the key and the expectation when the value is
    /// not a mapping, a key is unknown, or a setting does not validate.
    pub fn from_value(value: Scalar) -> Result<Self> {
        let entries = named_entries(&value, "an options record", "$")?;
        let mut options = Self::new();
        // A pattern and an explicit opening are the same setting spelled two
        // ways, so whichever the document carries is applied once.
        let mut opening: Option<Opening> = None;
        let mut header: Option<&str> = None;
        // The capture-dependent settings are held aside rather than written
        // into the options: installing the expressions *retains* only the
        // declarations whose captures still exist, which is right when a
        // caller changes a pattern and wrong here - a document naming a
        // capture that does not exist must be refused, not quietly dropped.
        let mut capture_types: Vec<(SmolStr, DataType)> = Vec::new();
        let mut custom_fields: Vec<(SmolStr, Scalar)> = Vec::new();
        let mut timestamp_capture: Option<SmolStr> = None;

        for (name, held) in entries {
            match name {
                "pattern" => opening = Some(Opening::Pattern(compiled(text(held, name)?, name)?)),
                "opening" => {
                    opening = Some(match text(held, name)? {
                        "every_line" => Opening::EveryLine,
                        "timestamp" => Opening::Timestamp,
                        other => {
                            return Err(unexpected(
                                name,
                                "\"every_line\" or \"timestamp\"",
                                format_smolstr!("{other:?}"),
                            ));
                        }
                    });
                }
                "header" => header = Some(text(held, name)?),
                "linesep" => options.set_linesep(Some(text(held, name)?.parse()?)),
                "lstrip" => options.set_lstrip(text(held, name)?.parse()?),
                "rstrip" => options.set_rstrip(text(held, name)?.parse()?),
                "byte_size" => options.set_byte_size(Some(count(held, name)?)),
                "batch_size" => options.set_batch_size(Some(count(held, name)?)),
                "timestamp_capture" => timestamp_capture = Some(SmolStr::new(text(held, name)?)),
                "timezone" => options.set_timezone(Some(text(held, name)?.parse()?))?,
                "capture_types" => {
                    let pairs = named_entries(
                        held,
                        "a record of capture names to datatypes",
                        "capture_types",
                    )?;
                    let mut declared = Vec::with_capacity(pairs.len());
                    for (capture, data_type) in pairs {
                        let spelled = data_type.as_str().ok_or_else(|| {
                            unexpected(name, "datatype expressions", data_type.kind())
                        })?;
                        declared.push((SmolStr::new(capture), DataType::from_str(spelled)?));
                    }
                    capture_types = declared;
                }
                "custom_fields" => {
                    let pairs = named_entries(
                        held,
                        "a record of column names to constants",
                        "custom_fields",
                    )?;
                    let mut customs = Vec::with_capacity(pairs.len());
                    for (column, constant) in pairs {
                        customs.push((SmolStr::new(column), constant.clone()));
                    }
                    custom_fields = customs;
                }
                other => {
                    return Err(unexpected(
                        other,
                        "a known option (pattern, opening, header, linesep, lstrip, rstrip, \
                         byte_size, batch_size, timestamp_capture, timezone, capture_types, \
                         custom_fields)",
                        format_smolstr!("{other:?}"),
                    ));
                }
            }
        }

        // The expressions settle together, because the capture columns are
        // their union and a collision has to be caught once.
        let compiled_header = match header {
            Some(header) => Some(compiled(header, "header")?),
            None => None,
        };
        options.replace_expressions(opening.unwrap_or(Opening::EveryLine), compiled_header)?;
        // Applied against the settled captures, through the same setters a
        // Rust caller uses - so a document is validated exactly as code is.
        options.set_capture_types(capture_types)?;
        options.set_custom_fields(custom_fields)?;
        if timestamp_capture.is_some() {
            options.set_timestamp_capture(timestamp_capture)?;
        }
        Ok(options)
    }
}

impl From<&TextLineOptions> for Scalar {
    fn from(value: &TextLineOptions) -> Self {
        value.into_value()
    }
}

fn named_entries<'value>(
    value: &'value Scalar,
    expectation: &str,
    path: &str,
) -> Result<Vec<(&'value str, &'value Scalar)>> {
    match value {
        Scalar::Record(entries) => Ok(entries
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect()),
        Scalar::Mapping(entries) => entries
            .iter()
            .map(|(name, value)| {
                name.as_str()
                    .map(|name| (name, value))
                    .ok_or_else(|| Error::InvalidRecord {
                        path: SmolStr::new(path),
                        reason: crate::text::expected_got("string names", name.kind()),
                    })
            })
            .collect(),
        _ => Err(Error::InvalidRecord {
            path: SmolStr::new(path),
            reason: crate::text::expected_got(expectation, value.kind()),
        }),
    }
}

impl TryFrom<Scalar> for TextLineOptions {
    type Error = Error;

    fn try_from(value: Scalar) -> Result<Self> {
        Self::from_value(value)
    }
}

/// Read a string option, or report what it was.
fn text<'value>(held: &'value Scalar, name: &str) -> Result<&'value str> {
    held.as_str()
        .ok_or_else(|| unexpected(name, "a string", held.kind()))
}

/// Read a count option, accepting every width a document may carry it in.
fn count(held: &Scalar, name: &str) -> Result<usize> {
    let value = match held {
        Scalar::U8(value) => u64::from(*value),
        Scalar::U16(value) => u64::from(*value),
        Scalar::U32(value) => u64::from(*value),
        Scalar::U64(value) => *value,
        Scalar::I8(value) => nonnegative(i64::from(*value), name)?,
        Scalar::I16(value) => nonnegative(i64::from(*value), name)?,
        Scalar::I32(value) => nonnegative(i64::from(*value), name)?,
        Scalar::I64(value) => nonnegative(*value, name)?,
        other => return Err(unexpected(name, "a count", other.kind())),
    };
    usize::try_from(value)
        .map_err(|_| unexpected(name, "a count this process can hold", "a larger one"))
}

/// Refuse a negative count, naming the option.
fn nonnegative(value: i64, name: &str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| unexpected(name, "a non-negative count", format_smolstr!("{value}")))
}

/// A typed refusal naming the option, the expectation, and the actual.
fn unexpected(
    name: &str,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason: crate::text::expected_got(expected.to_string(), actual.to_string()),
    }
}

/// A terminator's bytes as the escaped text `LineSep::from_str` reads back.
fn escaped(bytes: &[u8]) -> SmolStr {
    let mut spelled = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        match byte {
            b'\n' => spelled.push_str(r"\n"),
            b'\r' => spelled.push_str(r"\r"),
            b'\t' => spelled.push_str(r"\t"),
            0 => spelled.push_str(r"\0"),
            b'\\' => spelled.push_str(r"\\"),
            byte if byte.is_ascii_graphic() || *byte == b' ' => spelled.push(*byte as char),
            byte => {
                use std::fmt::Write as _;
                let _ = write!(spelled, "\\x{byte:02x}");
            }
        }
    }
    SmolStr::new(spelled)
}
