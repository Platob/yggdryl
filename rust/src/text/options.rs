//! Flat options for the `text/plain` record encoding.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use regex::bytes::Regex;
use smol_str::{SmolStr, format_smolstr};

use crate::media::IORecordOptions;
use crate::{DataType, Error, Field, Level, Result, Timezone};

use super::{LeadingFragment, LineSep};

/// Reserved columns emitted before decoded row-header captures.
pub(crate) const BASE_COLUMNS: [&str; 2] = ["body", "dropped_byte_size"];

/// The event columns a row-header capture feeds by its exact name: the
/// capture's value is the fact's, read at the fact's own datatype, and the
/// column that states the fact is the event's rather than a capture column
/// beside it. `mtime` feeds the instant the same way, under its own rule.
pub(crate) const EVENT_CAPTURES: [&str; 8] = [
    "state", "creaunix", "execunix", "recdunix", "exprtime", "prevunix", "snapunix", "prevuuid",
];

/// The event columns no capture can feed, because the line derives them -
/// its identity, the chain's, the codes, the names it goes by - or a walk
/// states them: a capture spelled as one is refused.
pub(crate) const DERIVED_EVENT_COLUMNS: [&str; 8] = [
    "currunix",
    "curruuid",
    "crosscode",
    "crossuuid",
    "currhashcode",
    "crosshashcode",
    "srcuuids",
    "seqnum",
];

/// The row-header capture that dates a line, feeding `currunix`.
///
/// Not a reserved name: with `parse_mtime` off the capture is an ordinary
/// one with a column of its own, so the flag alone decides whether the name
/// belongs to the reader or to the expression.
pub(crate) const MTIME_COLUMN: &str = "mtime";

/// The column stating what a line was classified as.
pub(crate) const MIMETYPE_COLUMN: &str = "mimetype";

/// The rows a text batch holds until its byte target binds first: `35 * 1024`.
///
/// A text read batches on two targets, whichever binds first, because a
/// heartbeat capture and a market-data capture differ by orders of
/// magnitude in bytes for one row count: this many rows of short lines, or
/// [`DEFAULT_TEXT_BATCH_BYTE_SIZE`] of long ones.
pub const DEFAULT_TEXT_BATCH_ROW_SIZE: usize = 35 * 1024;

/// The bytes a text batch holds until its row target binds first: `64 MiB`.
pub const DEFAULT_TEXT_BATCH_BYTE_SIZE: u64 = 64 * 1024 * 1024;

/// The one datatype the `mtime` column is read and stored at.
///
/// Nanoseconds in UTC: a capture's own offset is resolved into it, and a
/// handle's modification time is already counted in it, so the two sources
/// answer one column rather than two resolutions of it.
pub(crate) fn mtime_dtype() -> DataType {
    DataType::DateTime64 {
        unit: crate::TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    }
}

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
/// A new value batches on [`DEFAULT_TEXT_BATCH_ROW_SIZE`] rows or
/// [`DEFAULT_TEXT_BATCH_BYTE_SIZE`] bytes, whichever binds first.
/// Physical-line mode emits one row per line. With `framing` enabled,
/// `rowheader` starts a logical record and following nonmatching lines join its
/// body with normalized `\n` separators. Named captures remain nullable, and
/// the header stays in the body: the first physical line carries it, and a
/// line reads its captures off its own body.
/// `lstrip` and `rstrip` remove edge matches, and `autotype` infers capture
/// datatypes from regex syntax before the resource is read.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextOptions {
    /// Root Field name; the declared field's when one is declared.
    pub name: smol_str::SmolStr,
    /// The declared root; `None` infers the shape.
    pub field: Option<crate::Field>,
    /// The rows a read or write keeps: the `where` clause.
    ///
    /// Answered over the rows the lines become, by the record surface, and
    /// never by the decode: [`read_text_lines`](super::read_text_lines) yields
    /// every line whatever this says, because a `where` may name a column the
    /// `select` builds.
    pub filter: crate::Filter,
    /// The columns a read or write publishes.
    pub select: crate::Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: crate::Selector,
    /// Whether a declared or stored nullable column takes a value it cannot
    /// convert as null, `true` by default; a not-null column refuses it by
    /// name either way.
    pub safe: bool,
    /// Bytes per emitted batch, whichever of this and `batch_row_size` binds
    /// first; [`DEFAULT_TEXT_BATCH_BYTE_SIZE`] as `new()` states it.
    ///
    /// A target rather than a ceiling, and a non-zero bound always yields at
    /// least one row.
    pub batch_byte_size: Option<u64>,
    /// Rows per emitted batch, whichever of this and `batch_byte_size` binds
    /// first; [`DEFAULT_TEXT_BATCH_ROW_SIZE`] as `new()` states it.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// First emitted row number, which `seqnum` counts from; `None` counts
    /// from the zero-based physical index. Each nonnegative row number
    /// supplies `seqnum`; a negative one is refused.
    ///
    /// There is no column of its own: the row number is the event's place in
    /// its chain, and `seqnum` is where an event states that.
    pub start_rownum: Option<i64>,
    /// Whether the row header's `mtime` capture dates the line.
    ///
    /// On by default, because a captured line's own timestamp is the fact a
    /// reader of a capture reaches for first. With it on the capture feeds
    /// `currunix` and has no column beside it, and the handle's own
    /// modification time answers for a line the header did not date; with it
    /// off the capture is an ordinary one, read at its own syntax into its
    /// own column, and the handle's time answers `currunix` alone.
    pub parse_mtime: bool,
    /// Whether to classify each line and emit a `mimetype` column.
    pub parse_mimetype: bool,
    /// Whether to drop a row whose body repeats the row before it.
    ///
    /// A capture tool that published a line twice publishes it twice in a
    /// row, so one previous digest is the whole of what adjacent
    /// deduplication needs - never a set, which would grow without bound over
    /// a day of capture and would also be wrong: two identical heartbeats an
    /// hour apart are two events.
    ///
    /// It is off by default because switching it on deliberately surrenders
    /// row-in / row-out correspondence: the output no longer aligns with its
    /// input by position.
    pub dedup_adjacent: bool,
    /// Emitted name for a column, keyed by its default name.
    ///
    /// Renaming decides what a column is called and never whether one exists:
    /// a key naming no column is refused rather than read as a request to add
    /// one. The row header is the only thing that lifts a column out of a
    /// line, so a name a header does not capture is a mistake here.
    ///
    /// Ordered rather than hashed, because these options are compared, ordered
    /// and hashed, and two equal configurations must have one stored form.
    pub rename_columns: BTreeMap<SmolStr, SmolStr>,
    framing: bool,
    leading_fragment: LeadingFragment,
    max_record_byte_size: Option<u64>,
    rowheader: Option<Expression>,
    lstrip: Vec<Expression>,
    rstrip: Vec<Expression>,
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
            name: smol_str::SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: crate::Filter::always_true(),
            select: crate::Selector::all(),
            merge_by: crate::Selector::all(),
            safe: true,
            batch_byte_size: Some(DEFAULT_TEXT_BATCH_BYTE_SIZE),
            batch_row_size: Some(DEFAULT_TEXT_BATCH_ROW_SIZE),
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            start_rownum: None,
            parse_mtime: true,
            parse_mimetype: false,
            dedup_adjacent: false,
            rename_columns: BTreeMap::new(),
            framing: false,
            leading_fragment: LeadingFragment::Keep,
            max_record_byte_size: None,
            rowheader: None,
            lstrip: Vec::new(),
            rstrip: Vec::new(),
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

    /// Set or clear the retained byte limit for each record, counted past
    /// its row header.
    pub const fn set_max_record_byte_size(&mut self, size: Option<u64>) {
        self.max_record_byte_size = size;
    }

    /// Return these options with a byte limit for each record, counted past
    /// its row header.
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
                .chain(DERIVED_EVENT_COLUMNS.iter())
                .any(|base| base.eq_ignore_ascii_case(capture.name()))
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.rowheader"),
                    reason: format_smolstr!(
                        "expected named captures distinct from body, dropped_byte_size and the event columns the line derives - currunix, curruuid, crosscode, crossuuid, currhashcode, crosshashcode, srcuuids, identifiers, seqnum - got {:?}",
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

    /// Borrow the left-edge trimming patterns, in the order they are applied.
    ///
    /// A capture line routinely carries several layers of prose in front of
    /// its payload - a bridge's arrow, a stage's label, a plugin's name - and
    /// one expression that matches all of them at once is the expression
    /// nobody can read. A sequence strips them one after another, each from
    /// the new left edge, so `After Enrichment --> ` comes off as
    /// `After \w+`, then `-->`, then nothing.
    #[must_use]
    pub fn lstrip(&self) -> impl ExactSizeIterator<Item = &str> {
        self.lstrip
            .iter()
            .map(|expression| expression.source.as_str())
    }

    /// Compile or clear the left-edge trimming sequence atomically.
    ///
    /// The whole sequence is compiled before any of it is installed, so one
    /// bad pattern leaves the options exactly as they were.
    ///
    /// # Errors
    ///
    /// Returns the regex refusal naming `$.lstrip` and the offending pattern.
    pub fn set_lstrip<I, S>(&mut self, patterns: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.lstrip = compile_strips(patterns, "$.lstrip")?;
        Ok(())
    }

    /// Return these options with a left-edge trimming sequence.
    pub fn try_with_lstrip<I, S>(mut self, patterns: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.set_lstrip(patterns)?;
        Ok(self)
    }

    /// Borrow the right-edge trimming patterns, in the order they are applied.
    #[must_use]
    pub fn rstrip(&self) -> impl ExactSizeIterator<Item = &str> {
        self.rstrip
            .iter()
            .map(|expression| expression.source.as_str())
    }

    /// Compile or clear the right-edge trimming sequence atomically.
    ///
    /// # Errors
    ///
    /// Returns the regex refusal naming `$.rstrip` and the offending pattern.
    pub fn set_rstrip<I, S>(&mut self, patterns: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.rstrip = compile_strips(patterns, "$.rstrip")?;
        Ok(())
    }

    /// Return these options with a right-edge trimming sequence.
    pub fn try_with_rstrip<I, S>(mut self, patterns: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.set_rstrip(patterns)?;
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

    /// Borrow the emitted-name overrides.
    #[must_use]
    pub const fn rename_columns(&self) -> &BTreeMap<SmolStr, SmolStr> {
        &self.rename_columns
    }

    /// Return these options with one column renamed.
    #[must_use]
    pub fn with_renamed_column(mut self, from: impl Into<SmolStr>, to: impl Into<SmolStr>) -> Self {
        self.rename_columns.insert(from.into(), to.into());
        self
    }

    /// Compile the column plan these options answer with.
    ///
    /// # Errors
    ///
    /// Returns the refusals the plan states for a rename naming no column or
    /// two columns emitting one name.
    pub(crate) fn line_plan(&self) -> Result<super::plan::TextPlan> {
        super::plan::TextPlan::compile(self)
    }

    /// Return a deterministic hash of the complete flat configuration.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    pub(crate) fn rowheader_regex(&self) -> Option<&Regex> {
        self.rowheader
            .as_ref()
            .map(|expression| &expression.compiled)
    }

    /// Refuse a retained limit that would leave every record with no body.
    ///
    /// The limit bounds what follows the row header, and the header is
    /// always retained - so under a header a record keeps at least the
    /// bytes it was known by, whatever the limit. With no header there is
    /// nothing to keep, and a limit of zero would answer a line with no
    /// body: [`TextLine`](super::TextLine) has none, so the configuration
    /// is refused once here rather than per row.
    pub(crate) fn require_retained_body(&self) -> Result<()> {
        if self.max_record_byte_size == Some(0) && self.rowheader.is_none() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.max_record_byte_size"),
                reason: SmolStr::new_static(
                    "expected a retained record limit that keeps a body, got 0 with no rowheader to retain",
                ),
            });
        }
        Ok(())
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

    pub(crate) fn lstrip_regexes(&self) -> impl ExactSizeIterator<Item = &Regex> {
        self.lstrip.iter().map(|expression| &expression.compiled)
    }

    pub(crate) fn rstrip_regexes(&self) -> impl ExactSizeIterator<Item = &Regex> {
        self.rstrip.iter().map(|expression| &expression.compiled)
    }

    /// Whether any line rewriting is configured at all.
    ///
    /// One question, because every caller asking it is deciding whether the
    /// body it is about to hand on is the bytes it read.
    pub(crate) fn rewrites_body(&self) -> bool {
        !self.lstrip.is_empty() || !self.rstrip.is_empty()
    }

    pub(crate) fn output_linesep(&self) -> &[u8] {
        self.linesep.as_ref().map_or(b"\n", LineSep::as_bytes)
    }

    /// The root a text read answers `schema()` with, built without reading.
    ///
    /// The fixed prefix first - where the line came from, which line it was,
    /// what it was classified as, and the line itself - then one nullable
    /// column per named capture, in the order the row header declares them and
    /// typed by what its syntax can match. Public because a caller composing a
    /// text read with something that reads its payload needs the columns
    /// before there is a resource to read, exactly as
    /// [`fix_schema`](crate::fix_schema) answers the codec's before a byte is
    /// read.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the columns do not make a
    /// struct.
    pub fn source_field(&self) -> Result<Field> {
        self.line_plan()?.field(self.name.clone())
    }

    /// Whether a column the line already states - `mtime`, or one of the
    /// event columns a capture feeds - rather than a column of its own, is
    /// where this capture's value goes.
    ///
    /// One owner per fact: with `parse_mtime` on, a capture spelled `mtime`
    /// dates the line and has no column beside it; with it off, the capture
    /// is an ordinary one with its own column. A capture spelled
    /// as an event fact - `state`, `prevuuid` or one of the lifecycle
    /// instants, by that exact name, as the reading that takes it looks it up,
    /// feeds that fact, and the event column states it at the fact's own
    /// datatype. `seqnum` and `crosscode` are derived from row number and
    /// source URL and therefore cannot be captures.
    pub(crate) fn consumes_capture(&self, index: usize) -> bool {
        let Some(capture) = self.captures.get(index) else {
            return false;
        };
        (self.parse_mtime && capture.name() == MTIME_COLUMN)
            || EVENT_CAPTURES.contains(&capture.name())
    }

    /// The datatype one row-header capture is read at, in regex order.
    ///
    /// The schema and the row decoder ask the same question here rather than
    /// each deriving it, because a decoder that disagreed with the schema
    /// would parse a value into a column that cannot hold it.
    pub(crate) fn capture_dtype(&self, index: usize) -> DataType {
        if self.consumes_capture(index) {
            // The instant's column, or the fact's own reading: no capture
            // column is built, and the datatype answered is the clock's for
            // the one column that is.
            return mtime_dtype();
        }
        let Some(capture) = self.captures.get(index) else {
            return DataType::utf8();
        };
        if !self.autotype {
            return DataType::utf8();
        }
        match (capture.dtype(), self.timezone) {
            (DataType::DateTime64 { unit, timezone }, Some(configured)) if timezone.is_naive() => {
                DataType::DateTime64 {
                    unit: *unit,
                    timezone: configured,
                }
            }
            (dtype, _) => dtype.clone(),
        }
    }
}

impl Default for TextOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for TextOptions {
    crate::record_options_fields!();
}

/// Compile a strip sequence whole, so one bad pattern installs none of it.
fn compile_strips<I, S>(patterns: I, at: &'static str) -> Result<Vec<Expression>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    patterns
        .into_iter()
        .map(|pattern| Expression::new(pattern.as_ref(), at))
        .collect()
}
