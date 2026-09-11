//! What every line of one capture is read against.
//!
//! A [`FixCodec`] is the dictionary plus the few facts a whole run shares -
//! the dialect, the version, the spellings that mean nothing was sent - and
//! the constructors on [`FixMsg`] take one. Each of them splits its own
//! dialect, rewrites it into the key forms the one [builder](super::build)
//! understands, and hands the pairs over. None of them parses a message and
//! none of them builds a tree of its own.
//!
//! # Why `parse_*` and not `read_*`
//!
//! `read_*` names an I/O operation in this crate: it asks a handle for bytes
//! and names the core type it answers, as `read_all_bytes` and
//! `read_arrow_reader` do. A codec asks nothing of storage. It is handed
//! bytes, a record or a batch the caller already holds and answers the same
//! rows in the message vocabulary, so the family is named for what it does to
//! its input rather than for where the input came from. The two families then
//! compose without either shadowing the other: a handle's `read_arrow_reader`
//! feeds this codec's [`FixCodec::parse_text_arrow_reader`].
//!
//! # Three verbs, each an iterator, each with an Arrow twin
//!
//! `parse_*` turns what a capture holds into messages: one line of bytes
//! ([`FixCodec::parse_line`]), a stream of them ([`FixCodec::parse_lines`]),
//! one decoded line or a stream of them ([`FixCodec::parse_text_line`],
//! [`FixCodec::parse_text_lines`]), and a stream of Arrow batches of
//! capture rows ([`FixCodec::parse_text_arrow_reader`]). `enrich_*` fills
//! what a message implies ([`FixCodec::enrich_message`],
//! [`FixCodec::enrich_messages`], [`FixCodec::enrich_messages_arrow_reader`]).
//! The two converters between the message and the Arrow shape -
//! [`FixCodec::messages`] and [`FixCodec::arrow_reader`] - are what the Arrow
//! twins compose, and are public so a caller composes the same way. A stage
//! is a call, never a flag: nothing here takes an `enrich` argument, and
//! nothing enriches on the way out of a parse.
//!
//! # A row is a log line
//!
//! A capture line is a message wrapped in whatever the process printed around
//! it - `sending >> 8=FIX.4.2|…|10=203| << queued seq=1092` - so the frame is
//! located first and everything before it is prefix. Reading from byte zero
//! would take `sending >> 8` as the first key and pick the wrong dialect on
//! nearly every real line.
//!
//! The dialect is decided once, from the frame, and never re-sniffed: all
//! ASCII digits before the frame's first `=` means numeric FIX, anything else
//! means a bridge row. Once decided it holds for the whole body, so a `#` or a
//! `<` inside a *value* is part of that value.
//!
//! # Nothing is skipped
//!
//! A row with no message type is built anyway and named `unknown`; a value
//! that will not type is null; a group that will not split stays whole; a
//! document in `XmlData` that will not parse stays the bytes it is. What
//! is left - input that is not a row at all - is an `Err` item carrying it,
//! and the stream continues, because one corrupt line must not end a run over
//! ten million.

use std::borrow::Borrow;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use quick_xml::events::Event;
use smol_str::SmolStr;

use crate::media::text::{TextBytes, TextEntries, TextEntry, TextLine};
use crate::mime_type::line;
use crate::{DataType, Error, Field, Result, Scalar, Version};

use super::build::{
    BEGINSTRING_COLUMN, Builder, CLOCK_COLUMN, DIRECTION_COLUMN, Fill, FixPair, PLUGINID_COLUMN,
    RowExtras, root_name, version_of,
};
use super::memo::Memo;
use super::{FixBranch, FixMessages, FixMsg, FixRegistry};

/// One bridge row read into the pairs a build folds in, beside the message
/// type it declared.
type BridgeRow<'registry> = (Option<&'registry super::MsgType>, Vec<FixPair>);

/// The tier one bridge row's groups are split under: the dialect the row is
/// read under and the message its type names there.
///
/// Resolved once per row and carried together, because every group lookup
/// on the way down a packed occurrence asks both - which dictionary declares
/// the group, and which message says what a shared counter heads.
#[derive(Clone, Copy)]
struct RowTier<'registry> {
    branch: Option<&'registry FixBranch>,
    message: Option<&'registry super::MsgType>,
}

/// One segment of a packed occurrence's value.
///
/// A member pair, or the close a bridge writes where an occurrence it packed
/// inside this one ends: that occurrence's own trailing separator meets the
/// separator of the occurrence around it, and the empty segment between the
/// two is the close.
///
/// Both halves are ranges of the line's own page, so unpacking an occurrence
/// costs the segment list and no byte of what it names.
#[derive(Clone)]
enum Segment {
    Pair(TextBytes, TextBytes),
    Close,
}

/// What the twin judgment made of one `#` key.
enum Hashed {
    /// A second spelling of a bare pair stating the same bytes: one pair.
    Duplicate,
    /// The row's sole spelling: the key the line wrote after its mark.
    Bare,
    /// Beside a bare twin of its key or of its stem, stating other bytes
    /// or numbering other occurrences: the key as it arrived, mark and all.
    Verbatim,
}

/// How deep a packed value nests before the reader stops nesting it.
///
/// The bound a message schema is held to, so a run of openers a bridge never
/// closed cannot recurse the reader off its stack: past it an opener is one
/// more member of the occurrence it is in, its packed value its value.
const PACKED_DEPTH: usize = 64;

/// What separates the members packed inside one bridge group occurrence.
///
/// ULLINK writes EOT then ETX. A bridge relaying into a FIX session writes the
/// protocol's own SOH instead, which is unambiguous inside an occurrence
/// because no FIX value may contain one. Every packed value ends with one,
/// so an occurrence packed inside another closes with two in a row - its own
/// trailing one, then the one between it and the next member - and the empty
/// segment between them is where the bridge says the inner occurrence ends.
const MEMBER_SEPARATORS: [&[u8]; 4] = [
    b"\x04\x03",
    b"\x01",
    // The glyphs a log viewer prints for the two control bytes, which is
    // what an exported log carries in their place.
    "\u{2022}\u{2022}".as_bytes(),
    "\u{25AF}\u{25AF}".as_bytes(),
];

/// FIX's own data fields, whose byte length rides in the pair before them.
///
/// Sorted, so a tag is answered by a binary search. A data value may hold the
/// frame's own separator - a bridge writes a whole row into `XmlData(213)` -
/// so it is read to the length its `Len` field stated rather than split.
const DATA_TAGS: [i32; 21] = [
    89, 91, 96, 213, 349, 351, 353, 355, 357, 359, 361, 363, 365, 446, 619, 622, 1185, 1398, 1402,
    1404, 1469,
];

/// The tag one of FIX's own data fields carries, when the key names one.
///
/// A data tag is at most four digits, so the parse is skipped for every
/// other key before it is attempted.
fn data_tag(key: &[u8]) -> Option<i32> {
    if key.is_empty() || key.len() > 4 || !key[0].is_ascii_digit() {
        return None;
    }
    let tag = std::str::from_utf8(key)
        .ok()
        .and_then(super::field::parse_tag)?;
    DATA_TAGS.binary_search(&tag).ok().map(|_| tag)
}

/// FIX's `XmlData(213)`, the one data field a bridge writes a row into.
const XML_DATA_TAG: i32 = 213;

/// Whether an `XmlData` value is a bridge row rather than a document: it
/// opens with a bridge key - a run of key bytes, `#`-marked or not, closing
/// at an `=` - and not with a tag or a brace.
fn bridge_row(value: &[u8]) -> bool {
    let key = value.strip_prefix(b"#").unwrap_or(value);
    let Some(equals) = memchr::memchr(b'=', key) else {
        return false;
    };
    equals > 0
        && key[..equals]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'[' | b']'))
        && key[0].is_ascii_alphabetic()
}

/// Whether an `XmlData` value is a document rather than a bridge row: it
/// opens a tag, which a row never does.
fn document(value: &[u8]) -> bool {
    line::trim_ascii(value).first() == Some(&b'<')
}

/// The width of the separator standing at `at`, when one does.
///
/// A frame separates its fields with a raw `SOH`, a pipe, or one of the
/// spellings a log escapes a `SOH` in, and one line may mix them. A
/// whitespace run is deliberately not in this vocabulary: a data field's own
/// bytes hold spaces, and a frame that named no separator at all never stated
/// a length worth widening to.
fn separator_width(page: &[u8], at: usize) -> Option<usize> {
    let tail = page.get(at..)?;
    if matches!(tail.first(), Some(&SOH | &b'|')) {
        return Some(1);
    }
    line::SOH_MARKERS
        .iter()
        .find(|marker| tail.starts_with(marker))
        .map(|marker| marker.len())
}

/// Whether `span` ends a segment this frame actually cut.
///
/// A frame's segment ends where its separator begins or where the frame does,
/// and both are readable from what the line already answered: the separator
/// standing at `span`, an entry whose value ends there - which is the same
/// fact for a segment that stated a field - or the data field's own cut,
/// where the stated count and the frame agree. Anything else names a byte in
/// the middle of something, and there the frame's own cut is the only reading
/// that loses nothing.
fn cut_at(entry: &TextEntry, rest: &[TextEntry], span: usize) -> bool {
    entry.value().end() as usize == span
        || rest.iter().any(|held| held.value().end() as usize == span)
        || entry
            .key()
            .page()
            .is_some_and(|page| separator_width(page, span).is_some())
}

/// Where a data field's value ends, when it is read to a length rather than
/// left where the frame cut it.
///
/// Which tags carry a length is the FIX dictionary's fact and no scanner's, so
/// the widening is done here, over the entries the line already answered: the
/// value opens one byte past its key, runs the stated number of bytes, and
/// whatever the widened span swallowed was a reading of the value's own text
/// rather than a field of the frame.
///
/// The stated length is honoured only where it lands on a boundary the frame
/// itself states: the span has to end a segment the frame cut - the data
/// field's own, or one of the segments the widening swallows - and what
/// follows has to be the frame again, either nothing or a field keyed by a
/// tag. Both halves are needed. A count that lands mid-value would otherwise
/// truncate the field and drop the bytes past it from the message and from
/// the wire it re-emits, and a count that lands inside the checksum's own key
/// would swallow the trailer whole, because the entry after any span at all
/// is usually the tag-keyed `10` the frame closes with.
///
/// A stated length that lands nowhere reads the value to the trailer instead,
/// for `XmlData` only, because a bridge writes it last and a log that printed
/// each control byte as a glyph carries more bytes than the bridge counted;
/// any other data field keeps the span the frame cut, which is what the frame
/// itself said and the only reading that loses nothing.
fn data_end(
    entry: &TextEntry,
    stated: Option<&Arrived<'_>>,
    entries: &[TextEntry],
    after: usize,
    xml: bool,
) -> Option<usize> {
    let opens = entry.key().end() as usize + 1;
    let stated = stated
        .and_then(|held| std::str::from_utf8(held.value()).ok())
        .and_then(|text| text.parse::<usize>().ok());
    if let Some(span) = stated.and_then(|length| opens.checked_add(length)) {
        if cut_at(entry, &entries[after..], span) {
            let next = entries[after..]
                .iter()
                .find(|held| held.key().start() as usize >= span);
            match next {
                None => return Some(span),
                Some(held) if tag_keyed(held) && !held.key().is_empty() => return Some(span),
                _ => {}
            }
        }
    }
    if !xml {
        return None;
    }
    // The trailer is the checksum the frame closes with, and the value ends
    // where the separator in front of it begins - whichever spelling the log
    // wrote that separator in.
    let checksum = entries[after..]
        .iter()
        .rev()
        .find(|held| held.key().as_bytes() == b"10")?;
    let at = checksum.key().start() as usize;
    let page = checksum.key().page()?;
    let width = line::SOH_MARKERS
        .iter()
        .find(|marker| page[..at].ends_with(marker))
        .map_or(1, |marker| marker.len());
    at.checked_sub(width).filter(|end| *end >= opens)
}

/// One pair as the line wrote it, before any mark is judged.
///
/// The key range never holds the `#` a bridge marked it with - the scanner
/// strips it so that every reader lifting a bridge key asks for the name the
/// bridge gave the field - and `marked` is that fact kept beside it. What the
/// mark *means* is judged here, which is where the dictionary is.
///
/// Borrowed from the entry the line answered, because judging a row reads
/// its pairs and copies none of them: the one pair that is not the entry's
/// own is a data field read past the separator to the length it stated,
/// which is a new range of the same page and the one place this owns one.
struct Arrived<'entry> {
    key: &'entry TextBytes,
    value: Cow<'entry, TextBytes>,
    marked: bool,
}

impl Arrived<'_> {
    fn key(&self) -> &[u8] {
        self.key.as_bytes()
    }

    fn value(&self) -> &[u8] {
        self.value.as_bytes()
    }

    /// The key as the line wrote it, the mark included.
    ///
    /// A range one byte wider, never a copy: the `#` stands immediately in
    /// front of the key the scanner handed over, because that is what stripping
    /// it means.
    fn written(&self) -> TextBytes {
        if !self.marked {
            return self.key.clone();
        }
        self.key
            .page()
            .and_then(|page| {
                TextBytes::from_page(
                    page,
                    (self.key.start() as usize).checked_sub(1)?,
                    self.key.end() as usize,
                )
                .ok()
            })
            .unwrap_or_else(|| self.key.clone())
    }
}

/// The separator FIX itself writes between two pairs.
pub const SOH: u8 = 0x01;

/// The spellings that mean "nothing was sent", by default.
///
/// A bridge with nothing to say writes one of these, and a reader that keeps
/// them puts the four characters `null` into a column whose answer is no
/// answer. The match is on the raw bytes after the whitespace trim, compared
/// case-insensitively as ASCII - never through the crate's fold, which serves
/// names and code spellings and would match spellings nobody wrote.
pub const DEFAULT_NULL_VALUES: [&str; 3] = ["", "null", "<null>"];

/// The column a payload is read from when nothing names another.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// What one row-header capture states about the line it was read from.
///
/// Resolved once, when the codec is told what a run's captures are called,
/// and read by position afterwards: a line answers its captures in the order
/// the header declares them, so nothing looks a name up per row. A capture
/// naming nothing this codec knows is [`Silent`](Self::Silent), and silence
/// is never an instruction and never an error.
#[derive(Clone)]
enum CaptureRole {
    /// The plugin that logged the line: the dialect it is read under, and the
    /// fill of the crate's own field where the dictionary declares one.
    Plugin(Option<(Field, i32)>),
    /// The version the line is read at.
    Version,
    /// The clock that stamps the message.
    Clock,
    /// The direction the line moved.
    Direction,
    /// A capture whose name reaches a field, beside the field it fills.
    Fill(Field, i32),
    /// A capture this codec has no use for, which is most of them.
    Silent,
}

impl CaptureRole {
    /// What one capture name means to this codec, decided once.
    fn of(name: &str, codec: &FixCodec) -> Self {
        let is = |known: &str| crate::types::folds_equal(known, name);
        if is(PLUGINID_COLUMN) {
            return Self::Plugin(codec.fill_target(name));
        }
        if is(BEGINSTRING_COLUMN) {
            return Self::Version;
        }
        if is(CLOCK_COLUMN) {
            return Self::Clock;
        }
        if is(DIRECTION_COLUMN) {
            return Self::Direction;
        }
        codec
            .fill_target(name)
            .map_or(Self::Silent, |(field, tag)| Self::Fill(field, tag))
    }
}

/// One capture read row by row, holding what is constant across them.
///
/// A capture is millions of lines and calling a singular reader per line
/// re-does per message what is constant for the whole run. Pinning a branch
/// and the two versions skips inference for every row - and a capture is one
/// session, so pinning is the normal case rather than an optimization.
#[derive(Clone)]
pub struct FixCodec {
    registry: Arc<FixRegistry>,
    branch: Option<FixBranch>,
    version: Option<Version>,
    separator: Option<u8>,
    payload_column: SmolStr,
    /// What a run's row-header captures state, resolved in their order.
    ///
    /// Empty until a caller names them, because a codec that was told
    /// nothing reads a line's typed fields and no captures at all.
    captures: Arc<[CaptureRole]>,
    null_values: Vec<String>,
    /// The direction a line with no verb in front of its payload took.
    direction: Option<&'static str>,
    /// The raw bytes one Arrow batch of messages targets.
    batch_byte_size: u64,
    /// The `BeginString` child every built message carries, resolved once:
    /// a bridge row states no version, so every one of them would otherwise
    /// look the field up per line.
    beginstring: Field,
    /// What this run has already asked its dictionary, shared by every
    /// stream the codec is cloned into: a capture asks the same few
    /// questions a million times, and each is answered off the dictionary
    /// once.
    memo: Arc<Memo>,
}

impl FixCodec {
    /// The raw bytes one Arrow batch targets when the caller states none.
    ///
    /// A capture is tens of millions of lines and the row shape varies by
    /// three orders of magnitude between a heartbeat and a market-data
    /// snapshot, so a row bound alone makes memory unpredictable: the same
    /// bound is a few megabytes of one and gigabytes of the other. Targeting
    /// the bytes the rows were read from keeps a batch about the same size
    /// whatever arrived, and 128 MiB is large enough that the per-batch cost
    /// (building the arrays, crossing a reader boundary, writing a row group)
    /// is amortized to nothing, while still leaving several batches in flight
    /// on an ordinary machine.
    ///
    /// It is a target rather than a ceiling, measured on the input rather
    /// than on the batch built: a batch closes after the row that carries
    /// the running total over it, so it always holds at least one row and
    /// one enormous message can never produce an empty batch.
    pub const DEFAULT_BATCH_BYTE_SIZE: u64 = 128 * 1024 * 1024;

    /// Borrows the message type declared by a captured line without parsing a message.
    #[must_use]
    pub fn infer_msgtype_bytes(line: &[u8]) -> Option<&[u8]> {
        line::inspect(line).msgtype()
    }

    /// Borrows the message type declared by a captured text line.
    #[must_use]
    pub fn infer_msgtype_text(line: &str) -> Option<&str> {
        std::str::from_utf8(Self::infer_msgtype_bytes(line.as_bytes())?).ok()
    }

    /// Opens a codec over one dictionary.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        let beginstring = super::build::beginstring_field(&registry);
        Self {
            registry,
            branch: None,
            version: None,
            separator: None,
            payload_column: SmolStr::new_static(DEFAULT_PAYLOAD_COLUMN),
            captures: Arc::from([]),
            null_values: DEFAULT_NULL_VALUES
                .iter()
                .map(|spelling| (*spelling).to_owned())
                .collect(),
            direction: Some(crate::types::MsgDirection::SENT),
            batch_byte_size: Self::DEFAULT_BATCH_BYTE_SIZE,
            beginstring,
            memo: Arc::new(Memo::new()),
        }
    }

    /// The dictionary every message is read against.
    #[must_use]
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// The version messages are built at, where the caller pinned one.
    #[must_use]
    pub const fn version(&self) -> Option<Version> {
        self.version
    }

    /// The dialect every message is read under, where the caller pinned one.
    #[must_use]
    pub const fn branch(&self) -> Option<&FixBranch> {
        self.branch.as_ref()
    }

    /// The direction a line with no verb in front of its payload takes.
    #[must_use]
    pub const fn direction(&self) -> Option<&'static str> {
        self.direction
    }

    /// The raw bytes one Arrow batch of messages targets.
    #[must_use]
    pub const fn batch_byte_size(&self) -> u64 {
        self.batch_byte_size
    }

    /// The column a record's payload is read from.
    #[must_use]
    pub fn payload_column(&self) -> &str {
        &self.payload_column
    }

    /// The byte a re-emitted line separates its fields with, where the caller
    /// pinned one.
    #[must_use]
    pub const fn separator(&self) -> Option<u8> {
        self.separator
    }

    /// The spellings that mean "nothing was sent".
    #[must_use]
    pub fn null_values(&self) -> &[String] {
        &self.null_values
    }

    /// Pins the dialect, so no row is read under the standard one.
    #[must_use]
    pub fn with_branch(mut self, branch: &FixBranch) -> Self {
        self.branch = Some(branch.clone());
        self
    }

    /// Pins the version the built messages are read at.
    ///
    /// A value is translated through the code spellings that version declares,
    /// and nothing else changes: a tag is one column under the name the
    /// dictionary holds it by, whatever version read it. Unpinned, each row
    /// answers for itself:
    /// `ApplVerID(1128)` first, then `BeginString(8)`, then the dialect's own
    /// default, then the dictionary's newest - which is what a capture
    /// carrying more than one application version needs.
    #[must_use]
    pub const fn with_version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Pins the byte a re-emitted line separates its fields with.
    ///
    /// A write-side statement and nothing else: a line is read by the pairs it
    /// already stated, and which byte separated them is the line's own answer
    /// rather than a caller's. A capture whose frames are written back out
    /// carries no such answer, so this is where a writer says what to spell.
    #[must_use]
    pub const fn with_separator(mut self, separator: u8) -> Self {
        self.separator = Some(separator);
        self
    }

    /// Names the batch column [`Self::parse_text_arrow_reader`] reads the
    /// payload from.
    #[must_use]
    pub fn with_payload_column(mut self, column: impl Into<SmolStr>) -> Self {
        self.payload_column = column.into();
        self
    }

    /// Names what a run's row-header captures are called, resolving each once.
    ///
    /// A line carries its captures by position, in the order its header
    /// declares them, so this is the boundary that decides what each position
    /// means: the plugin that logged the line, the version, the clock, the
    /// direction, a field a capture's name reaches, or nothing. Pass what
    /// [`TextOptions::capture_names`] answers for the options the lines were
    /// read under, and every line of the run is then read without one name
    /// lookup.
    ///
    /// A capture is what the transport wrote around the line, never what the
    /// line itself says: the body is the message, so a fact read out of it is
    /// already a field this codec fills from its tag.
    ///
    /// [`TextOptions::capture_names`]: crate::media::text::TextOptions::capture_names
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::media::text::{TextBytes, TextLine};
    /// # use yggdryl::{FixBranch, FixCodec, FixRegistry};
    /// let mut registry = FixRegistry::new();
    /// registry.set_branch(FixBranch::from_str("venue")?.with_aliases(["vnu"])?)?;
    /// let codec = FixCodec::new(Arc::new(registry)).with_capture_names(["pluginid"]);
    ///
    /// let line = TextLine::new(0, TextBytes::from_bytes(b"8=FIX.4.4|35=D|11=A|10=0|")?)
    ///     .with_captures(vec![Some(TextBytes::from_bytes(b"VNU")?)]);
    /// let message = codec.parse_text_line(&line)?.next().expect("one message")?;
    /// // The alias named the dialect, and the capture filled its own field.
    /// assert_eq!(message.branch().name(), "venue");
    /// assert_eq!(message.by_tag(yggdryl::PLUGINID_TAG)?.as_str(), Some("VNU"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_capture_names<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let roles: Vec<CaptureRole> = names
            .into_iter()
            .map(|name| CaptureRole::of(name.as_ref(), &self))
            .collect();
        self.captures = Arc::from(roles);
        self
    }

    /// Sets the direction a line with no verb in front of its payload takes.
    ///
    /// `SENT` by default: a session's own log is written by the side doing
    /// the sending, so its unmarked lines are the ones it sent and its
    /// inbound lines are the ones it bothered to mark. A capture taken from
    /// the other side sets `RECV`, and one whose silence really means unknown
    /// sets `None`. Any verb a line does carry beats this, and so does a
    /// `direction` column a record or a batch row states.
    ///
    /// The value is stored as the `msgdirection` column holds it, so it is
    /// [`MsgDirection::SENT`](crate::types::MsgDirection::SENT),
    /// [`MsgDirection::RECV`](crate::types::MsgDirection::RECV) or `None`
    /// and nothing else; a spelling from outside the crate crosses
    /// [`MsgDirection::from_spelling`](crate::types::MsgDirection::from_spelling)
    /// first, which is the one place the spellings are listed.
    #[must_use]
    pub const fn with_direction(mut self, direction: Option<&'static str>) -> Self {
        self.direction = direction;
        self
    }

    /// Sets the raw bytes one Arrow batch of messages targets.
    ///
    /// Read by [`Self::parse_text_arrow_reader`] and [`Self::arrow_reader`],
    /// and through it by everything that batches; the default is
    /// [`Self::DEFAULT_BATCH_BYTE_SIZE`]. Zero closes a batch after every
    /// row, since a batch always holds one.
    #[must_use]
    pub const fn with_batch_byte_size(mut self, bytes: u64) -> Self {
        self.batch_byte_size = bytes;
        self
    }

    /// Replaces the spellings that mean "nothing was sent".
    ///
    /// Empty keeps every literal, which is what a venue for whom the text
    /// `null` is a value sets: the default is a convention, and a convention
    /// has to be overridable to be safe.
    #[must_use]
    pub fn with_null_values<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.null_values = spellings.into_iter().map(Into::into).collect();
        self
    }

    /// Whether a column named `name` fills a field under this codec.
    ///
    /// A column is the caller's, not the line's, so it resolves under the
    /// codec's own pin and then the standard branch - the run's tier, asked
    /// once per column rather than once per row. A row's own `pluginid`
    /// names the dialect its *keys* resolve in, which is a different
    /// question: it moves what the line spells, never where a column lands,
    /// so a stream's columns fill the same fields whatever dialect each row
    /// turns out to name.
    pub(super) fn fill_target(&self, name: &str) -> Option<(Field, i32)> {
        let branch = self.branch.as_ref().unwrap_or(FixBranch::standard());
        let (field, tag) = super::build::fill_field(&self.registry, branch, name)?;
        let mut field = field.clone();
        field.set_nullable(false);
        Some((field, tag))
    }

    /// The dialect a row's `pluginid` names: the registered branch whose
    /// name or alias the plugin is spelled as, else nothing.
    ///
    /// A bridge's plugins are named by its operator, so a dictionary that
    /// wants one read under a dialect declares that dialect under the
    /// plugin's name, or with the name as an alias
    /// ([`FixBranch::with_aliases`]); every other plugin reads under the
    /// codec's pin. The registry is asked once per spelling and the answer
    /// kept in the codec's memo, so a capture naming a plugin on every line
    /// resolves it on the first. Empty text names nothing rather than the
    /// standard branch, and text longer than [`FixBranch::MAX_LENGTH`] never
    /// names a branch, since none can be spelled so: both are answered
    /// before the memo is touched.
    pub(super) fn dialect_of(&self, plugin: &str) -> Option<&FixBranch> {
        if plugin.is_empty() || plugin.len() > FixBranch::MAX_LENGTH {
            return None;
        }
        let digest = self.memo.dialect(plugin, || {
            self.registry.branch_named(plugin).map(FixBranch::digest)
        })?;
        self.registry.get_branch_by_digest(super::signed(digest))
    }

    /// The tier one row's keys resolve in: the dialect its `pluginid` named,
    /// else the one the caller pinned, else `None` - the standard namespace
    /// first, as an unpinned codec reads.
    fn tier<'a>(&'a self, row: Option<&'a FixBranch>) -> Option<&'a FixBranch> {
        row.or(self.branch.as_ref())
    }

    /// Whether one raw value is a stated absence rather than a value.
    fn is_absent(&self, value: &[u8]) -> bool {
        let trimmed = line::trim_ascii(value);
        self.null_values
            .iter()
            .any(|spelling| spelling.as_bytes().eq_ignore_ascii_case(trimmed))
    }

    /// Parses one log line into an iterator of messages, selecting the
    /// dialect from its frame. A bulk UL configuration yields one message
    /// per selected MBean; the other dialects yield one message.
    ///
    /// # A message is the frame; the line is still the line
    ///
    /// The line states every pair it wrote, a transport's own prefix and
    /// anything after the checksum included, because what a line said and what
    /// one message said are two facts and only one of them is this reader's.
    /// A message begins where the frame the line carries opens and ends at its
    /// checksum: a log printing `ts=` and `thread=` in front of the frame it
    /// quoted states neither field, and a message that swallowed them would
    /// answer them by name and re-emit them. A line carrying two frames reads
    /// as the first; the rest of the line is not part of that message.
    ///
    /// Which reader owns the frame is one shallow look at what the line
    /// already answered: a frame whose first key is a run of digits is numeric
    /// FIX, and anything else is a bridge row. The decision is made once and
    /// holds for the whole body, so a `#` or a `<` inside a *value* is part of
    /// that value.
    ///
    /// A FIXML row is the one that states no `key=value` frame at all, so the
    /// locator finds nothing to read; a row holding a tag is read as the
    /// document it is rather than as a line that held nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for input that is not a row at all.
    pub fn parse_line(&self, row: &[u8]) -> Result<FixMessages> {
        self.parse_line_with(row, RowExtras::NONE)
    }

    /// Parses a stream of log lines into a stream of messages, lazily.
    ///
    /// The line iterator everything else is built on: each line is read as
    /// [`Self::parse_line`] reads it, a bulk configuration yielding one
    /// message per MBean, and a line that is not a row at all is an `Err`
    /// item - the stream continues past it, because one corrupt line must not
    /// end a run over ten million. Nothing is collected: the iterator is the
    /// stream. Every stage answers an iterator that owns its codec and
    /// borrows nothing, so the stages compose into [`Self::arrow_reader`]
    /// without the codec outliving the stream.
    ///
    /// Composed into [`Self::arrow_reader`], that `Err` item is the batch
    /// reader's error: the completed prefix is yielded, then the error, and
    /// the reader fuses, as every batch reader in the crate does - a
    /// consumer of batches has no row to put a refused line in. A caller
    /// wanting a row per line, refused lines included, reads the capture
    /// through [`Self::parse_text_arrow_reader`], which answers such a line
    /// as a row holding an empty message.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// let codec = FixCodec::new(Arc::new(FixRegistry::new()));
    /// let lines = ["8=FIX.4.4|35=D|11=A|10=0|", "8=FIX.4.4|35=8|37=O1|10=0|"];
    /// let read: Vec<_> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    /// assert_eq!(read.len(), 2);
    /// assert_eq!(read[1].by_tag(37)?.as_str(), Some("O1"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn parse_lines<I>(&self, lines: I) -> impl Iterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        let codec = self.clone();
        lines
            .into_iter()
            .flat_map(move |line| FixMessages::from_result(codec.parse_line(line.as_ref())))
    }

    /// Parses one decoded line into the messages it carries.
    ///
    /// The door every capture should come through, and the only one that
    /// copies nothing: the line already holds its bytes as a range of a page
    /// it owns, so that page is handed over rather than made again, and every
    /// key and value the messages record is a range of it. The pairs are
    /// read off that page here, directly and none descended into, whatever
    /// tree the line was asked for: a message reads a nested value by its
    /// own rules, and a data field to the length it stated.
    ///
    /// What the line states for itself outranks this codec's own pins,
    /// because a line is the more specific statement:
    ///
    /// | the line's | supplies |
    /// | --- | --- |
    /// | [`body`](TextLine::body) | the bytes read, as the range they already are |
    /// | [`timestamp`](TextLine::timestamp) | the clock that stamps the message |
    /// | [`direction`](TextLine::direction) | the direction, before the payload's own verb and this codec's pin |
    /// | [`captures`](TextLine::captures) | the dialect, the version and every field a capture's name reaches, by the positions [`Self::with_capture_names`] resolved |
    ///
    /// A line that states none of them reads exactly as its bytes would,
    /// which is what makes this an entry point and not a second contract.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a line's content cannot provoke.
    pub fn parse_text_line(&self, line: &TextLine) -> Result<FixMessages> {
        // The row's own cells, held while the extras borrow them.
        let stated: Vec<Option<&TextBytes>> = self
            .captures
            .iter()
            .zip(line.captures())
            .map(|(_, held)| held.as_ref())
            .collect();
        let text = |at: usize| {
            stated
                .get(at)
                .copied()
                .flatten()
                .and_then(TextBytes::as_str)
        };
        let mut branch = None;
        let mut version = None;
        let mut stamped = None;
        let mut cells: Vec<(Field, i32, Scalar)> = Vec::new();
        for (at, role) in self.captures.iter().enumerate() {
            match role {
                CaptureRole::Plugin(target) => {
                    let Some(plugin) = text(at) else { continue };
                    branch = self.dialect_of(plugin);
                    if let Some((field, tag)) = target {
                        cells.push((field.clone(), *tag, Scalar::from(plugin)));
                    }
                }
                CaptureRole::Version => version = text(at).and_then(version_of),
                CaptureRole::Clock => stamped = text(at),
                CaptureRole::Fill(field, tag) => {
                    let Some(held) = text(at) else { continue };
                    cells.push((field.clone(), *tag, Scalar::from(held)));
                }
                // A direction has no home on a message: it is a fact about
                // the line, and only the batch reader has a column to put it
                // in. The capture is named so it cannot silently become a
                // fill on a field of that name.
                CaptureRole::Direction | CaptureRole::Silent => {}
            }
        }
        // The line's own clock where the read gave it one - a consumed capture
        // or the object's own time - and the capture it was read from where it
        // did not, which is a column the read emitted rather than consumed.
        let clock = line
            .timestamp()
            .map(Scalar::from)
            .or_else(|| stamped.map(Scalar::from));
        let fills: Vec<Fill<'_>> = cells
            .iter()
            .map(|(field, tag, value)| Fill {
                field,
                tag: *tag,
                value,
            })
            .collect();
        let extras = RowExtras {
            branch,
            version,
            clock: clock.as_ref(),
            fills: &fills,
        };
        let page = line.body();
        if page.is_empty() {
            return Ok(FixMessages::one(self.empty_with(extras)));
        }
        Ok(self
            .parse_page_with(page, extras)
            .unwrap_or_else(|_| FixMessages::one(self.empty_with(extras))))
    }

    /// Parses a stream of decoded lines into a stream of messages, lazily.
    ///
    /// Each line is read as [`Self::parse_text_line`] reads it, and a line
    /// nobody could read is a row holding an empty message rather than the end
    /// of the run - one corrupt line must not end a capture of ten million.
    pub fn parse_text_lines<I>(&self, lines: I) -> impl Iterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator,
        I::Item: Borrow<TextLine>,
    {
        let codec = self.clone();
        lines
            .into_iter()
            .flat_map(move |line| FixMessages::from_result(codec.parse_text_line(line.borrow())))
    }

    /// One payload read under what its row stated, a row of nothing included.
    ///
    /// A row's content can never fail the batch it arrives in: a payload
    /// nobody could read is a row holding an empty message, dated and
    /// versioned by what the row itself said.
    pub(super) fn parse_bytes_with(&self, extras: RowExtras<'_>, bytes: &[u8]) -> FixMessages {
        if bytes.is_empty() {
            return FixMessages::one(self.empty_with(extras));
        }
        self.parse_line_with(bytes, extras)
            .unwrap_or_else(|_| FixMessages::one(self.empty_with(extras)))
    }

    /// A row nobody could read, which is still a row - dated and versioned as
    /// every row is, by what the row itself stated.
    fn empty_with(&self, extras: RowExtras<'_>) -> FixMsg {
        self.build_pairs_with(&[], extras)
            .expect("an empty message builds")
    }

    /// [`Self::parse_line`], with what the row stated beside its line.
    pub(super) fn parse_line_with(&self, row: &[u8], extras: RowExtras<'_>) -> Result<FixMessages> {
        if row.is_empty() {
            return Err(Error::Parse {
                target: "fix",
                position: 0,
                reason: "expected a captured row, got no bytes".into(),
            });
        }
        self.parse_page_with(&TextBytes::from_bytes(row)?, extras)
    }

    /// One line already held as a range of a page, read into its messages.
    ///
    /// Every key and value the messages record is a range of this page, so the
    /// bytes are copied once - into the page, by whoever holds it - rather than
    /// once per pair.
    fn parse_page_with(&self, page: &TextBytes, extras: RowExtras<'_>) -> Result<FixMessages> {
        let row = page.as_bytes();
        // One scan answers the pairs and where the frame opens; a row that
        // carries no payload at all opens past its own end, so the frame
        // reading gets nothing and the document readers below answer.
        let (entries, frame_at) = TextEntries::from_bytes_direct_located(page);
        let entries = entries.unwrap_or_default();
        let opens = line::payload_at_or_document(row, frame_at).unwrap_or(row.len());
        let framed = framed_entries(entries.as_slice(), page.start() as usize + opens);
        if framed.first().is_some_and(tag_keyed) {
            return self.frame_with(framed, extras).map(FixMessages::one);
        }
        // An XML document a transport wrote prose in front of opens before
        // any pair the locator could read as a bridge row, and is read as
        // the document it is, by its attributes.
        if let Some((_, open)) = line::document_behind_prefix(row) {
            return self.fixml_with(&row[open..], extras).map(FixMessages::one);
        }
        let body = &row[opens..];
        // A payload opening with `{` is a bridge configuration document, and
        // nothing else is: the locator points at a key, which starts with a
        // digit or a letter, and points at an object only where it found one.
        // So the test costs one byte rather than a second classification.
        if matches!(body.first(), Some(b'{' | b'[')) {
            return self.ulconfig_with(body, extras);
        }
        // A FIXML row states no `key=value` frame, so the locator finds none
        // and leaves nothing to read. The document is the payload, and it
        // opens at the first tag - which is also how a prefix is dropped from
        // one, since everything before that tag is text the reader skips.
        if body.is_empty() && memchr::memchr(b'<', row).is_some() {
            return self.fixml_with(row, extras).map(FixMessages::one);
        }
        self.bridge_with(framed, extras).map(FixMessages::one)
    }

    /// Parses one numeric FIX frame.
    ///
    /// The frame is read from the pairs the line already answered, so where a
    /// value ends is the line's own reading of the separator it named - a
    /// `SOH` raw or in any of the spellings a log escapes it with, or a pipe -
    /// and this reader neither picks a separator nor splits on one. A data
    /// field is the one exception, and it is FIX's: its value is read to the
    /// length the `Len` field in front of it stated, so a value carrying the
    /// frame's own separator survives whole.
    ///
    /// Nothing after the checksum is part of the message; a pair the line
    /// wrote in front of the frame belongs to the transport and not to it.
    /// Duplicate tags stay in arrival order, and every key and value is a
    /// range of the line, none of it validated as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn parse_fix_line(&self, body: &[u8]) -> Result<FixMsg> {
        let page = TextBytes::from_bytes(body)?;
        let entries = TextEntries::from_bytes_direct(&page).unwrap_or_default();
        self.frame_with(bounded(&page, &entries), RowExtras::NONE)
    }

    /// One numeric frame, from the entries the line answered for it.
    fn frame_with(&self, entries: &[TextEntry], extras: RowExtras<'_>) -> Result<FixMsg> {
        let (arrived, nested) = frame_arrivals(entries);
        let pairs: Vec<FixPair> = self
            .judged_keys(&arrived)
            .into_iter()
            .zip(&arrived)
            .filter_map(|(judged, held)| {
                judged.map(|(key, _)| FixPair::own(key, held.value.as_ref().clone()))
            })
            .collect();
        self.build(&pairs, &nested, extras)
    }

    /// Parses one bridge row of `NAME=VALUE` pairs.
    ///
    /// Where a value ends is the line's own answer: a row that named a `SOH`
    /// or a pipe between its fields keeps the spaces inside its values, and one
    /// that named nothing is read the way a sentence is.
    ///
    /// A key opening with `#` is a bridge's own spelling of a name:
    /// `#SYMBOL=TTF` is the field, `#NOPARTYIDS=1` a counter and
    /// `#NOPARTYIDS[0]=…` one occurrence whose *value* is a run of member
    /// pairs - read into the fields it packs, while the arrival record keeps
    /// the pair the bridge wrote, because a rendered member path names no
    /// range of the line. The mark is judged against the row's bare spellings, under the
    /// FIX name fold every key resolves by, and a bare pair whose value is a
    /// stated absence is no twin, because a key that said nothing was sent
    /// is not a key that was sent:
    ///
    /// | the row states | reads as |
    /// | --- | --- |
    /// | `#ORDERID=123` alone | `OrderID` 123: the mark drops |
    /// | `ORDERID=123\|#ORDERID=123` | `OrderID` 123 once: the marked pair is a second spelling of the same bytes and is dropped, row and entries alike |
    /// | `ORDERID=123\|#ORDERID=345` | `OrderID` 123 beside `#ORDERID` 345: two keys, so the marked one stays verbatim - its own child, its own entry - whichever arrived first |
    /// | `NOPARTYIDS=2\|…\|#NOPARTYIDS=6\|#NOPARTYIDS[0]=…` | the bare group is the dictionary's; every marked key of that group stays verbatim and whole, occurrence and count alike, because a group is one thing however many occurrences it states |
    /// | `NOPARTYIDS=1\|NOPARTYIDS[0]=…\|#NOPARTYIDS=1\|#NOPARTYIDS[0]=…` | the marked count restates the bare one, but a marked group goes only whole: while any marked key of it stays, every one does, count included; a marked group restating the bare group pair for pair goes pair for pair |
    ///
    /// The twin is a spelling, never an identity: a tag and a marked name -
    /// `55=AAPL|#SYMBOL=AAPL` - state two values, exactly as a tag and a bare
    /// name do. Residue that will not split stays as one unknown key,
    /// verbatim: never dropped, never fatal.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn parse_ullink_line(&self, body: &[u8]) -> Result<FixMsg> {
        let page = TextBytes::from_bytes(body)?;
        let entries = TextEntries::from_bytes_direct(&page).unwrap_or_default();
        self.bridge_with(bounded(&page, &entries), RowExtras::NONE)
    }

    /// [`Self::parse_ullink_line`], with what the row stated beside its row.
    fn bridge_with(&self, entries: &[TextEntry], extras: RowExtras<'_>) -> Result<FixMsg> {
        let arrived = arrivals(entries);
        let (_, pairs) = self.bridge_pairs(&arrived, self.tier(extras.branch));
        self.build(&pairs, &[], extras)
    }

    /// One bridge row as the pairs the builder takes: `#` twins judged, and
    /// each packed occurrence read into the member keys it holds.
    ///
    /// Shared by the bridge-row reader and the frame reader, which meets a
    /// bridge row inside a data field and reads it by exactly these rules.
    ///
    /// Answers the message the row declares beside the pairs, because the
    /// splitting already resolved it: a group is split by the members the
    /// row's own type declares, and a caller reading the row into a frame
    /// needs the same answer to resolve the row's own spellings. `branch` is
    /// the dialect the row is read under, which is what declares them.
    ///
    /// A packed occurrence is unpacked into the members that fill the row, and
    /// the pair the bridge wrote is what the arrival record keeps: a key like
    /// `NOPARTYIDS[0].PARTYID` appears nowhere in the line, so it names a
    /// reading and never an arrival.
    fn bridge_pairs<'registry>(
        &'registry self,
        arrived: &[Arrived<'_>],
        branch: Option<&'registry FixBranch>,
    ) -> BridgeRow<'registry> {
        // Every `#` key is judged before the row's type is read, so a type
        // the bridge marked names the message exactly as a bare one does,
        // and one kept verbatim beside a bare type does not. A row with no
        // `#` at all judges nothing.
        let kept: Vec<(TextBytes, &Arrived<'_>, bool)> = self
            .judged_keys(arrived)
            .into_iter()
            .zip(arrived)
            .filter_map(|(judged, held)| judged.map(|(key, whole)| (key, held, whole)))
            .collect();
        let mut resolved: Vec<FixPair> = Vec::with_capacity(kept.len());
        let msgtype = msgtype_of(
            kept.iter()
                .map(|(key, held, _)| (key.as_bytes(), held.value())),
        );
        let tier = RowTier {
            branch,
            message: msgtype
                .as_deref()
                .and_then(|code| self.declared_message(code, branch)),
        };
        // The judged key is the one the pair is built under, so it moves
        // into the pair rather than being counted once more on the way.
        for (key, held, whole) in kept {
            if whole {
                // Verbatim means whole: the twinned `#` key is its own key
                // and the packed value is its value, so no group rendering
                // rewrites either - a group name opening with `#` resolves
                // in no dictionary anyway.
                resolved.push(FixPair::own(key, held.value.as_ref().clone()));
                continue;
            }
            match group_index(key.as_bytes()) {
                Some((group, occurrence)) if memchr::memchr(b'=', held.value()).is_some() => {
                    let declared = self.group_members(group, tier);
                    let mut path = Vec::with_capacity(group.len() + 8);
                    path.extend_from_slice(group);
                    path.extend_from_slice(b"[");
                    path.extend_from_slice(occurrence.to_string().as_bytes());
                    path.extend_from_slice(b"]");
                    let segments = members(&held.value, declared);
                    // The bridge closed what it packed inside this occurrence
                    // where it wrote a close anywhere but at the run's own
                    // end; a run carrying none is bounded by the dictionary.
                    let explicit = segments
                        .iter()
                        .rev()
                        .skip(1)
                        .any(|segment| matches!(segment, Segment::Close));
                    let opened = resolved.len();
                    self.render_members(&path, &segments, tier, explicit, 0, &mut resolved);
                    // The first member read out of the occurrence carries the
                    // record of the pair the bridge actually wrote; the rest
                    // are that same arrival, read further.
                    if let Some(first) = resolved.get_mut(opened) {
                        first.reads(key, held.value.as_ref().clone());
                    }
                }
                _ => resolved.push(FixPair::own(key, held.value.as_ref().clone())),
            }
        }
        (tier.message, resolved)
    }

    /// Every arriving key with its `#` judged: the key to build under and
    /// whether it is kept whole, or `None` for a marked pair that only
    /// restates a bare one.
    ///
    /// Each `#` key is judged against the row's bare spellings, gathered
    /// once: a bridge row is mostly `#` keys, so the probed list stays short.
    /// A row with no `#` at all judges nothing. Then a marked group goes only
    /// whole. Its count and its occurrences are
    /// judged pair by pair, and a count restating the bare one beside
    /// occurrences the bare group never numbered would leave those
    /// occurrences without their count - so where any marked key of a group
    /// stays verbatim, every marked key of that group does.
    fn judged_keys(&self, arrived: &[Arrived<'_>]) -> Vec<Option<(TextBytes, bool)>> {
        // A row that marked nothing judges nothing. The twin probe is one pass
        // over the row's bare spellings per marked key, so a wide frame that
        // marked none would otherwise pay a quadratic walk to learn that.
        if !arrived.iter().any(|held| held.marked) {
            return arrived
                .iter()
                .map(|held| Some((held.key.clone(), false)))
                .collect();
        }
        let bare = self.bare_spellings(arrived);
        let mut judged: Vec<Option<(TextBytes, bool)>> = arrived
            .iter()
            .map(|held| self.hashed_key(held, arrived, &bare))
            .collect();
        fn marked<'held>(held: &'held Arrived<'_>) -> Option<&'held [u8]> {
            held.marked.then(|| line::trim_ascii(held.key()))
        }
        // The stems of the marked groups kept verbatim: a stem some marked
        // key of the row indexes, and some marked key of which was kept.
        let whole: Vec<&[u8]> = arrived
            .iter()
            .zip(&judged)
            .filter(|(_, judged)| matches!(judged, Some((_, true))))
            .filter_map(|(held, _)| marked(held).map(stem_of))
            .filter(|stem| {
                arrived.iter().any(|held| {
                    marked(held).is_some_and(|stripped| {
                        group_index(stripped).is_some() && folds_twin(stem_of(stripped), stem)
                    })
                })
            })
            .collect();
        if whole.is_empty() {
            return judged;
        }
        let promoted: Vec<Option<TextBytes>> = arrived
            .iter()
            .zip(&judged)
            .map(|(held, judged)| {
                let restated = judged.is_none()
                    && marked(held).is_some_and(|stripped| {
                        whole.iter().any(|stem| folds_twin(stem, stem_of(stripped)))
                    });
                restated.then(|| held.written())
            })
            .collect();
        for (key, judged) in promoted.into_iter().zip(&mut judged) {
            if let Some(key) = key {
                *judged = Some((key, true));
            }
        }
        judged
    }

    /// The row's bare spellings: every pair the line did not mark whose value
    /// is not a stated absence.
    fn bare_spellings<'row>(&self, arrived: &'row [Arrived<'_>]) -> Vec<(&'row [u8], &'row [u8])> {
        arrived
            .iter()
            .filter(|held| !held.marked && !self.is_absent(held.value()))
            .map(|held| (held.key(), held.value()))
            .collect()
    }

    /// The key one arriving pair builds under, its `#` judged, and whether it
    /// is kept whole; `None` for a marked pair that only restates a bare one.
    ///
    /// A twin that itself opens with `#` - a `##` key's bare - is not among
    /// the bare spellings, so that one probe falls back to the whole row, read
    /// under the keys the line wrote rather than the ones it was stripped to.
    fn hashed_key(
        &self,
        held: &Arrived<'_>,
        arrived: &[Arrived<'_>],
        bare: &[(&[u8], &[u8])],
    ) -> Option<(TextBytes, bool)> {
        if !held.marked {
            return Some((held.key.clone(), false));
        }
        let stripped = line::trim_ascii(held.key());
        let judged = if stripped.first() == Some(&b'#') {
            let written: Vec<(TextBytes, TextBytes)> = arrived
                .iter()
                .filter(|held| !self.is_absent(held.value()))
                .map(|held| (held.written(), held.value.as_ref().clone()))
                .collect();
            let row: Vec<(&[u8], &[u8])> = written
                .iter()
                .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
                .collect();
            judge_hashed(stripped, held.value(), &row)
        } else {
            judge_hashed(stripped, held.value(), bare)
        };
        match judged {
            Hashed::Duplicate => None,
            Hashed::Bare => Some((held.key.clone(), false)),
            Hashed::Verbatim => Some((held.written(), true)),
        }
    }

    /// One occurrence's member segments rendered under its path, sub-groups
    /// and all.
    ///
    /// A bridge packs a group nested inside an occurrence at the same level
    /// as the occurrence's own members, behind the same separator:
    /// `NOPARTYSUBIDS=1`, then `NOPARTYSUBIDS[0]=PARTYSUBID=a`, then
    /// `PARTYSUBIDTYPE=b`, then the party's own `PARTYID=c`. The
    /// sub-occurrence's key carries its first member packed into its value,
    /// and where the pairs after it stop belonging to it is
    /// [`Self::extent`]'s answer: the close the bridge wrote where it wrote
    /// closes, the dictionary's declaration where it did not - so
    /// `PARTYSUBIDTYPE` rides under the sub-occurrence and `PARTYID` comes
    /// back up to the party. Rendered as
    /// `NOPARTYIDS[0].NOPARTYSUBIDS[0].PARTYSUBID`, the key the builder nests
    /// by, at any depth a bridge packs.
    ///
    /// `explicit` is decided once for the whole packed value, because a
    /// bridge closes every occurrence it packs or none of them, and a nested
    /// slice of a closed run may hold no close of its own. `depth` is how
    /// many occurrences this one is packed inside, held to
    /// [`PACKED_DEPTH`]: past it an opener is one more member, so a run of
    /// openers nothing closed ends in a wide row rather than in the stack.
    fn render_members<'registry>(
        &'registry self,
        path: &[u8],
        segments: &[Segment],
        tier: RowTier<'registry>,
        explicit: bool,
        depth: usize,
        out: &mut Vec<FixPair>,
    ) {
        let mut at = 0;
        while at < segments.len() {
            let Segment::Pair(member, held) = &segments[at] else {
                // The run's own trailing separator, or a close of something
                // this level never opened: nothing to end.
                at += 1;
                continue;
            };
            at += 1;
            let rendered = |member: &[u8]| {
                let mut key = Vec::with_capacity(path.len() + member.len() + 1);
                key.extend_from_slice(path);
                key.extend_from_slice(b".");
                key.extend_from_slice(member);
                key
            };
            match group_index(member.as_bytes()) {
                Some((sub, index))
                    if depth < PACKED_DEPTH && memchr::memchr(b'=', held.as_bytes()).is_some() =>
                {
                    let sub_declared = self.group_members(sub, tier);
                    let mut sub_path = rendered(sub);
                    sub_path.extend_from_slice(b"[");
                    sub_path.extend_from_slice(index.to_string().as_bytes());
                    sub_path.extend_from_slice(b"]");
                    // What the sub-occurrence packed into its own value,
                    // then every following segment up to where it ends.
                    let end = self.extent(segments, at, sub_declared, tier, explicit);
                    let mut nested = members(held, sub_declared);
                    nested.extend_from_slice(&segments[at..end]);
                    self.render_members(&sub_path, &nested, tier, explicit, depth + 1, out);
                    at = end;
                    // The close that ended it is spent with it.
                    if explicit && matches!(segments.get(at), Some(Segment::Close)) {
                        at += 1;
                    }
                }
                // The rendered path is the key the builder nests by and no
                // range of the line names it, which is why it fills a field
                // and records nothing: the pair it was read out of is the
                // arrival, and the caller pins that on the first of these. It
                // is also why it is held as the bytes it is - one allocation
                // for the path, none for a page nothing counts.
                _ => out.push(FixPair::read(rendered(member.as_bytes()), held.clone())),
            }
        }
    }

    /// Where the occurrence opened just before `start` ends: the index of
    /// the first segment past it.
    ///
    /// Explicit, the bridge closed every occurrence it packed inside the run,
    /// so this one reaches the close that balances the ones opened inside it.
    /// Implicit, it reaches as far as the dictionary declares: a member the
    /// group declares, or an occurrence of a group it declares, skipped whole
    /// by the same rule. The first segment it does not declare ends it, as
    /// does a close - the run's own trailing one, in a run that carries no
    /// other - and an undeclared pair ends every enclosing occurrence that
    /// does not declare it either, each judging that segment in turn, so it
    /// lands on the nearest enclosing occurrence that declares it, or on the
    /// packed value's own, and what follows it lands there too.
    fn extent(
        &self,
        segments: &[Segment],
        start: usize,
        declared: &[Field],
        tier: RowTier<'_>,
        explicit: bool,
    ) -> usize {
        let mut at = start;
        let mut depth = 0_usize;
        while let Some(segment) = segments.get(at) {
            match segment {
                Segment::Close => {
                    if !explicit || depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Segment::Pair(key, held) => {
                    let opens = memchr::memchr(b'=', held.as_bytes()).is_some();
                    match group_index(key.as_bytes()) {
                        Some(_) if opens && explicit => depth += 1,
                        Some((sub, _)) if opens => {
                            if !self.declares_group(declared, sub, tier) {
                                break;
                            }
                            let sub_declared = self.group_members(sub, tier);
                            at = self.extent(segments, at + 1, sub_declared, tier, false);
                            continue;
                        }
                        _ if !explicit && !declares(declared, key.as_bytes()) => break,
                        _ => {}
                    }
                }
            }
            at += 1;
        }
        at
    }

    /// Whether the level whose members are `declared` declares the group
    /// `sub` addresses: the group itself, or the counter that heads it.
    fn declares_group(&self, declared: &[Field], sub: &[u8], tier: RowTier<'_>) -> bool {
        let Some(group) = self.group_definition(sub, tier) else {
            return false;
        };
        let counter = group.as_fix().counter().ok().flatten();
        declared.iter().any(|field| {
            crate::types::folds_equal(field.name(), group.name())
                || counter.is_some_and(|counter| {
                    field.as_fix().tag().ok().flatten() == Some(counter)
                        || field.as_fix().counter().ok().flatten() == Some(counter)
                })
        })
    }

    /// Parses one FIXML row: every element's attributes, in document order.
    ///
    /// FIXML spells a field as an XML attribute and a component as a nested
    /// element, so the attributes *are* the pairs and the nesting flattens the
    /// way a bridge occurrence already does. An element name is not a tag and
    /// nothing invents one: `<Order ClOrdID="A"/>` states one field, and one
    /// field is what this states.
    ///
    /// A namespace prefix is dropped from an attribute name, exactly as the
    /// CBlock reader drops one from an element name, because a prefix names a
    /// document's own vocabulary and never the field.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the row is not
    /// well-formed XML, and the builder's refusal otherwise.
    pub fn parse_fixml_line(&self, body: &[u8]) -> Result<FixMsg> {
        self.fixml_with(body, RowExtras::NONE)
    }

    /// [`Self::parse_fixml_line`], with what the row stated beside its document.
    fn fixml_with(&self, body: &[u8], extras: RowExtras<'_>) -> Result<FixMsg> {
        let pairs = own_pairs(
            fixml_pairs(body)?
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice())),
        )?;
        self.build(&pairs, &[], extras)
    }

    /// Fills what one message implies but did not carry.
    ///
    /// An order stating `OrderQty` and `CumQty` has said what `LeavesQty` is,
    /// and a fill stating `LastQty` and `LastPx` has said what it was worth.
    /// The rules are the specification's own tables read as
    /// implications - FIX 4.4's Appendix D for an order's life, 4.2's
    /// Appendix O for what a foreign exchange trade settles on - and a rule
    /// answers only where every input is stated and typed.
    ///
    /// Only the row is filled. The entries are what arrived and are carried
    /// through untouched, so [`FixMsg::into_bytes`] re-emits the received line
    /// byte for byte whether the message was enriched or not. A stated value
    /// is never overwritten, which also makes this idempotent.
    ///
    /// # Errors
    ///
    /// Never fails: a derived value the column refuses is silence rather
    /// than a refusal, and the `Result` is the shape every stage of a
    /// message stream answers in.
    pub fn enrich_message(&self, message: FixMsg) -> Result<FixMsg> {
        Ok(super::enrich::enrich(&self.registry, message))
    }

    /// Fills a stream of messages, lazily.
    ///
    /// Nothing is collected: the iterator is the stream, so a capture of ten
    /// million messages costs one at a time. [`Self::enrich_messages_arrow_reader`]
    /// is the same pass over batches of rows.
    pub fn enrich_messages<I>(&self, messages: I) -> impl Iterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator<Item = FixMsg>,
    {
        let registry = Arc::clone(&self.registry);
        messages
            .into_iter()
            .map(move |message| Ok(super::enrich::enrich(&registry, message)))
    }

    /// Stamps a stream of messages with the identities it implies, in order.
    ///
    /// One [`FixLifecycle`](super::FixLifecycle) over the whole stream: each
    /// message gets its `instid`, its `id` and - where it carries an order
    /// identifier - the `persistentid` of the chain that identifier reaches,
    /// and a terminal state closes the chain. Nothing is collected: the
    /// iterator is the stream, and what is held is the orders still alive.
    pub fn lifecycle<I>(&self, messages: I) -> impl Iterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator<Item = FixMsg>,
    {
        let mut life = super::FixLifecycle::new(Arc::clone(&self.registry));
        messages.into_iter().map(move |message| life.fill(message))
    }

    /// Builds one message from pairs the caller already split.
    ///
    /// The pairs are built as they are: a `#` is judged where a row is
    /// split, never here, so a key carrying one builds as one flat child
    /// under its own spelling, and two pairs are two pairs however alike.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub fn parse_pairs<'a, I>(&self, pairs: I) -> Result<FixMsg>
    where
        I: IntoIterator<Item = (&'a [u8], &'a [u8])>,
    {
        self.build(&own_pairs(pairs)?, &[], RowExtras::NONE)
    }

    /// The build a reader outside this module funnels into.
    ///
    /// The pairs are bytes a reader holds rather than ranges of a line - a
    /// configuration document's fields, or no pairs at all - so they are
    /// copied into one page here, which is what a message owning its own
    /// arrival record costs when nothing owned it already.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub(super) fn build_pairs_with(
        &self,
        pairs: &[(&[u8], &[u8])],
        extras: RowExtras<'_>,
    ) -> Result<FixMsg> {
        self.build(&own_pairs(pairs.iter().copied())?, &[], extras)
    }

    /// The one build every reader funnels into.
    ///
    /// The pairs are the line; `nested` is every bridge row a data field of
    /// the line carried, read after the pairs so the frame's own statements
    /// come first and a row inside it fills only what the frame left unsaid;
    /// `extras` is the row the line came on, applied last for the same
    /// reason.
    fn build(
        &self,
        pairs: &[FixPair],
        nested: &[TextBytes],
        extras: RowExtras<'_>,
    ) -> Result<FixMsg> {
        // The row's own dialect where its `pluginid` named one, else the
        // caller's pin: a capture is one session and the branch is a fact
        // about the run, except where a row says which plugin logged it and
        // the dictionary declares that plugin's own dialect. Nothing is
        // copied for it - the branch lives in the registry, in the codec or
        // in the crate, and each outlives the build. The version goes the
        // same way: the row's, else the pin, else what the line implies.
        let tier = self.tier(extras.branch);
        let branch = tier.unwrap_or(FixBranch::standard());
        let pinned = extras.version.or(self.version);
        let version = pinned.or_else(|| self.infer_version(pairs, branch));
        let msgtype = msgtype_of(pairs.iter().map(|pair| (pair.key(), pair.value())));

        let message = msgtype
            .as_deref()
            .and_then(|code| self.declared_message(code, tier));
        let mut builder = Builder::new(
            &self.registry,
            message,
            &self.beginstring,
            &self.memo,
            branch,
            version,
            pairs.len(),
        );
        // A stated absence produces no field and no entry: the key is read
        // as never having been sent. Filtering happens before typing, so
        // nothing tries to read `<null>` as a price and file the failure.
        builder.push_pairs(pairs, |value| self.is_absent(value));
        for row in nested {
            self.push_nested(&mut builder, row, tier, pinned);
        }
        for fill in extras.fills {
            builder.fill(fill);
        }
        let mut built = builder.finish(root_name(msgtype.as_deref()).as_str(), extras.clock)?;
        built.field.as_fix_mut().set_branch(branch)?;
        FixMsg::from_built(Arc::clone(&self.registry), built)
    }

    /// Reads one row a data field carried into the line it arrived on.
    ///
    /// A row inside a data field is a reading of that field's value, not a
    /// second arrival: it fills the row and records no entry, so the arrival
    /// record and the wire it re-emits stay exact.
    ///
    /// Two shapes, one reading. A bridge row splits into its own pairs; a
    /// document is read by its attributes, exactly as a line carrying one is.
    /// Either is a message of its own type at its own version, so both are
    /// resolved from the row's own pairs before anything is pushed - a
    /// `BeginString` states what the session speaks, and the row inside a data
    /// field is routinely written to a later FIX than that. It states none of
    /// its own, so the inference lands on the dictionary's newest, which is
    /// the best reading of a row nothing dates - unless the line's row or
    /// the caller `pinned` one, which dates the nested row too.
    ///
    /// A document that will not parse fills nothing and the value stays whole,
    /// for the reason a group that will not split stays whole: one
    /// unreadable field is not a reason to lose the line it arrived on.
    fn push_nested<'registry>(
        &'registry self,
        builder: &mut Builder<'registry>,
        row: &TextBytes,
        tier: Option<&'registry FixBranch>,
        pinned: Option<Version>,
    ) {
        if document(row.as_bytes()) {
            let Ok(owned) = fixml_pairs(row.as_bytes()) else {
                return;
            };
            let Ok(held) = own_pairs(
                owned
                    .iter()
                    .map(|(key, value)| (key.as_slice(), value.as_slice())),
            ) else {
                return;
            };
            let declared = msgtype_of(held.iter().map(|pair| (pair.key(), pair.value())))
                .as_deref()
                .and_then(|code| self.declared_message(code, tier));
            self.nest(builder, declared, &held, tier, pinned);
            return;
        }
        // The value's own entries, read in their own scope: what a bridge
        // wrote inside a data field is judged against that row's spellings
        // and not against the frame's.
        let entries = TextEntries::from_bytes_direct(row).unwrap_or_default();
        let arrived = arrivals(entries.as_slice());
        let (declared, held) = self.bridge_pairs(&arrived, tier);
        self.nest(builder, declared, &held, tier, pinned);
    }

    /// One nested row's pairs, bracketed as the nested reading they are.
    fn nest<'registry>(
        &'registry self,
        builder: &mut Builder<'registry>,
        declared: Option<&'registry super::MsgType>,
        pairs: &[FixPair],
        tier: Option<&FixBranch>,
        pinned: Option<Version>,
    ) {
        let dated =
            pinned.or_else(|| self.infer_version(pairs, tier.unwrap_or(FixBranch::standard())));
        builder.begin_nested(declared, dated);
        for pair in pairs {
            if self.is_absent(pair.value()) {
                continue;
            }
            builder.push(pair.key(), pair.value());
        }
        builder.end_nested();
    }

    /// The message definition a row's type names, under the tier every key
    /// resolves by: the row's dialect, then the standard one.
    ///
    /// A dialect declares its own vocabulary and rarely a message of its own,
    /// and the row a bridge writes calls itself by FIX's name - so a pinned
    /// dialect that answered nothing would leave every group the message
    /// declares, `NoLegs` and `NoSides` among them, without the context that
    /// says which group a shared counter heads.
    fn declared_message(&self, code: &str, tier: Option<&FixBranch>) -> Option<&super::MsgType> {
        self.registry.known_msgtype(code, tier)
    }

    /// The direct members the addressed repeating group declares.
    ///
    /// Bridge keys are rendered names even when their bytes are digits. Only
    /// a nested field reached by that name can declare boundaries; an
    /// unresolved group leaves its value whole.
    fn group_members<'registry>(
        &'registry self,
        group: &[u8],
        tier: RowTier<'registry>,
    ) -> &'registry [Field] {
        let Some(field) = self.group_definition(group, tier) else {
            return &[];
        };
        let item = match field.dtype() {
            DataType::List(item) | DataType::LargeList(item) => item,
            _ => return &[],
        };
        item.fields()
    }

    /// The repeating group a bridge key addresses, as the dictionary declares
    /// it: by the group's own name, else by the counter's, under the message
    /// where that counter is shared - all in `tier`, the dialect the row is
    /// read under and the message it declares.
    fn group_definition<'registry>(
        &'registry self,
        group: &[u8],
        tier: RowTier<'registry>,
    ) -> Option<&'registry Field> {
        let Ok(group) = std::str::from_utf8(group) else {
            return None;
        };
        let RowTier { branch, message } = tier;
        let found = self
            .registry
            .get_definition(crate::FixCategory::Groups, group, branch)
            .or_else(|| {
                let counter = if let Some(tag) = super::field::parse_tag(group) {
                    let branch = branch.unwrap_or(FixBranch::standard());
                    self.registry
                        .get_field_by_id(super::FixId::from_parts(branch, tag).ok()?)
                } else {
                    // A dialect that spells no such counter falls back to the
                    // standard branch, where FIX's own fields live.
                    self.registry.get_field_by_name(group, branch).or_else(|| {
                        branch
                            .is_some_and(|held| !held.is_standard())
                            .then(|| {
                                self.registry
                                    .get_field_by_name(group, Some(&FixBranch::STANDARD))
                            })
                            .flatten()
                    })
                }?;
                let id = self.registry.identity_of(counter)?;
                match message.filter(|message| message.has_group_counter(id)) {
                    Some(message) => message.get_group_by_counter(id),
                    None => self.registry.get_group_by_counter(id),
                }
            });
        found.filter(|field| field.dtype().is_nested())
    }

    /// The version an arriving row is written in, when the caller pinned none.
    ///
    /// Each step is a FIX rule rather than a heuristic. `ApplVerID(1128)`
    /// wins, because under FIXT.1.1 the session version says nothing about
    /// the application version. `BeginString(8)` follows, and `FIXT.1.1`
    /// falls through rather than being taken literally. Then the branch's own
    /// default, then the dictionary's real newest - never a sentinel.
    fn infer_version(&self, pairs: &[FixPair], branch: &FixBranch) -> Option<Version> {
        if let Some(value) = value_of(pairs, b"1128") {
            if let Some(version) = appl_ver_id(&String::from_utf8_lossy(value), &self.registry) {
                return Some(version);
            }
        }
        if let Some(value) = value_of(pairs, b"8") {
            let text = String::from_utf8_lossy(value);
            if let Some(rest) = text.strip_prefix("FIX.") {
                if let Ok(version) = rest.parse::<Version>() {
                    return Some(version);
                }
            }
        }
        if branch.version() != Version::MIN {
            return Some(branch.version());
        }
        self.registry.newest().map(|pedigree| pedigree.version())
    }
}

/// `ApplVerID`'s numeric and symbolic spellings.
fn appl_ver_id(value: &str, registry: &FixRegistry) -> Option<Version> {
    let spelling = match value {
        "0" | "FIX27" => "2.7",
        "1" | "FIX30" => "3.0",
        "2" | "FIX40" => "4.0",
        "3" | "FIX41" => "4.1",
        "4" | "FIX42" => "4.2",
        "5" | "FIX43" => "4.3",
        "6" | "FIX44" => "4.4",
        "7" | "FIX50" => "5.0",
        "8" | "FIX50SP1" => "5.0.1",
        "9" | "FIX50SP2" => "5.0.2",
        // "FIX Latest" is a moving label and resolves to the pedigree the
        // dictionary actually carries, never to a sentinel.
        "10" | "FIXLatest" => {
            return registry.newest().map(|pedigree| pedigree.version());
        }
        _ => return None,
    };
    spelling.parse().ok()
}

/// The value one tag carries, without building anything.
fn value_of<'a>(pairs: &'a [FixPair], key: &[u8]) -> Option<&'a [u8]> {
    pairs
        .iter()
        .find(|pair| pair.key() == key)
        .map(FixPair::value)
}

/// The message type a row declares, by `35=` or by `MSGTYPE=`.
fn msgtype_of<'a>(pairs: impl IntoIterator<Item = (&'a [u8], &'a [u8])>) -> Option<String> {
    for (key, value) in pairs {
        // The key folds the way every other key folds, so `MSG_TYPE` and
        // `Msg Type` name the type too.
        let folded =
            std::str::from_utf8(key).is_ok_and(|key| crate::types::folds_equal(key, "MsgType"));
        if folded || key == b"35" {
            return Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    None
}

/// One FIXML document as the pairs its attributes are, in document order.
///
/// A namespace prefix is dropped from an attribute name, exactly as the
/// CBlock reader drops one from an element name, because a prefix names a
/// document's own vocabulary and never the field. The values are owned
/// because an escaped attribute is not a slice of the document.
fn fixml_pairs(body: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut reader = quick_xml::Reader::from_reader(body);
    let mut buffer = Vec::new();
    let mut owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let malformed = |reader: &quick_xml::Reader<&[u8]>, reason: String| Error::Parse {
        target: "fixml",
        position: reader.buffer_position() as usize,
        reason: SmolStr::new(reason),
    };
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| malformed(&reader, error.to_string()))?;
        match event {
            Event::Eof => break,
            Event::Start(element) | Event::Empty(element) => {
                for attribute in element.attributes() {
                    let attribute =
                        attribute.map_err(|error| malformed(&reader, error.to_string()))?;
                    owned.push((
                        attribute.key.local_name().as_ref().to_vec(),
                        attribute.value.into_owned(),
                    ));
                }
            }
            _ => {}
        }
        buffer.clear();
    }
    Ok(owned)
}

/// The entries one message is read from: everything from where the frame the
/// line carries opens.
///
/// The line keeps every pair it saw, a transport's own prefix included, because
/// what a line said and what one message said are two facts. A message is
/// bounded by the frame: a log line printing `ts=…` and `thread=…` in front of
/// the frame it quoted states neither field, and a message that swallowed them
/// would answer them by name and re-emit them.
///
/// One bound for every door. The row reader, the frame reader and the bridge
/// reader all take a line, so a prefix means the same thing at each of them,
/// and a body that opens at its own first pair is bounded at zero and reads
/// exactly as it did.
fn bounded<'entries>(page: &TextBytes, entries: &'entries TextEntries) -> &'entries [TextEntry] {
    let opens = line::payload_at(page.as_bytes()).unwrap_or(0);
    framed_entries(entries.as_slice(), page.start() as usize + opens)
}

/// [`bounded`], against a bound a caller already located.
fn framed_entries(entries: &[TextEntry], opens: usize) -> &[TextEntry] {
    let at = entries
        .iter()
        .position(|entry| entry.key().start() as usize >= opens)
        .unwrap_or(entries.len());
    &entries[at..]
}

/// Whether one entry's key is a tag rather than a name, which is what says a
/// frame is numeric FIX rather than a bridge row.
///
/// A key the line marked is a bridge's own spelling and never a tag, even
/// where the bytes after the mark are digits: `#453=1` is a bridge naming the
/// group by its counter, not a frame stating tag 453.
fn tag_keyed(entry: &TextEntry) -> bool {
    !entry.marked() && entry.key().as_bytes().iter().all(u8::is_ascii_digit)
}

/// One entry as the pair it arrived as.
fn arrival(entry: &TextEntry) -> Arrived<'_> {
    Arrived {
        key: entry.key(),
        value: Cow::Borrowed(entry.value()),
        marked: entry.marked(),
    }
}

/// Every entry as the pair it arrived as.
fn arrivals(entries: &[TextEntry]) -> Vec<Arrived<'_>> {
    entries.iter().map(arrival).collect()
}

/// One frame's arriving pairs, its data fields read to the length their `Len`
/// field stated, beside the rows those data fields carried.
///
/// The widening is where FIX and the scanner part, and it parts exactly here:
/// the scanner cut the value at the frame's separator because that is all a
/// frame states, and a data field says how long its value is instead. What the
/// widened span swallowed was never a field of the frame, so it goes.
fn frame_arrivals(entries: &[TextEntry]) -> (Vec<Arrived<'_>>, Vec<TextBytes>) {
    let mut arrived: Vec<Arrived<'_>> = Vec::with_capacity(entries.len());
    // The data values that are bridge rows, read after the frame's own pairs
    // so the frame's statements come first.
    let mut nested: Vec<TextBytes> = Vec::new();
    let mut at = 0;
    while let Some(entry) = entries.get(at) {
        at += 1;
        let mut held = arrival(entry);
        if let Some(tag) = data_tag(held.key()) {
            let xml = tag == XML_DATA_TAG;
            if let Some(end) = data_end(entry, arrived.last(), entries, at, xml) {
                if let Some(widened) = entry.key().page().and_then(|page| {
                    TextBytes::from_page(page, entry.key().end() as usize + 1, end).ok()
                }) {
                    held.value = Cow::Owned(widened);
                    while entries
                        .get(at)
                        .is_some_and(|swallowed| (swallowed.key().start() as usize) < end)
                    {
                        at += 1;
                    }
                }
            }
            // `XmlData` is the field a bridge writes a whole message into,
            // and it writes one two ways: a row of its own pairs, and the
            // FIXML the tag is named for. Both are read into the line rather
            // than left as bytes nobody can address, so a frame carrying
            // either answers by tag and by name like any other. Anything else
            // in it is a value and stays one.
            if xml && (bridge_row(held.value()) || document(held.value())) {
                nested.push(held.value.as_ref().clone());
            }
        }
        // A key the bridge marked is the bridge's own, whatever it spells: a
        // frame ends at the checksum it wrote, not at a restatement of one.
        let checksum = !held.marked && held.key() == b"10";
        arrived.push(held);
        if checksum {
            // Nothing after the checksum is part of the message.
            break;
        }
    }
    (arrived, nested)
}

/// Pairs a caller holds as loose bytes, copied into one page.
///
/// The door for input that never was a line: pairs handed in by a caller, and
/// the attributes a FIXML document escaped, which are not slices of the
/// document that carried them. A message owns the bytes its entries name, so
/// they are copied once into one page here rather than once per half of every
/// pair.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] when the pairs are wider than one page can
/// address.
fn own_pairs<'a>(pairs: impl IntoIterator<Item = (&'a [u8], &'a [u8])>) -> Result<Vec<FixPair>> {
    let mut page = Vec::new();
    let mut spans = Vec::new();
    for (key, value) in pairs {
        let opens = page.len();
        page.extend_from_slice(key);
        let split = page.len();
        page.extend_from_slice(value);
        spans.push((opens, split, page.len()));
    }
    let page = TextBytes::from_whole_page(std::sync::Arc::new(page))?;
    spans
        .into_iter()
        .map(|(opens, split, ends)| {
            Ok(FixPair::own(
                page.slice(opens, split)?,
                page.slice(split, ends)?,
            ))
        })
        .collect()
}

/// One range narrowed to what it holds without ASCII whitespace at its ends.
fn trimmed(bytes: &TextBytes, start: usize, end: usize) -> TextBytes {
    let held = bytes.as_bytes();
    let mut start = start;
    let mut end = end;
    while start < end && held[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && held[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    bytes.slice(start, end).unwrap_or_default()
}

/// Whether a group declares a member spelled `key`, by name or by tag.
fn declares(declared: &[Field], key: &[u8]) -> bool {
    let Ok(key) = std::str::from_utf8(key) else {
        return false;
    };
    let key = key.trim();
    let tag = super::field::parse_tag(key);
    declared.iter().any(|field| {
        crate::types::folds_equal(field.name(), key)
            || (tag.is_some() && field.as_fix().tag().ok().flatten() == tag)
    })
}

/// The group and occurrence a `NAME[0]` key addresses.
fn group_index(key: &[u8]) -> Option<(&[u8], usize)> {
    let open = memchr::memchr(b'[', key)?;
    let close = memchr::memchr(b']', &key[open..])? + open;
    let index = std::str::from_utf8(&key[open + 1..close]).ok()?;
    Some((&key[..open], index.parse().ok()?))
}

/// The segments packed inside one occurrence's value: its member pairs, and
/// the closes of the occurrences packed inside it.
///
/// ULLINK separates members with EOT then ETX, and sometimes omits the
/// separator. An explicit spelling is authoritative. With neither spelling
/// present, only direct members declared by the addressed group can begin
/// another pair. An empty segment - two separators in a row, nothing at all
/// between them - is a close, kept for the renderer to end a nested
/// occurrence on; a segment that is neither a pair nor empty is residue and
/// stays out.
fn members(value: &TextBytes, declared: &[Field]) -> Vec<Segment> {
    split_members(value.as_bytes(), declared)
        .into_iter()
        .filter_map(|part| {
            if part.is_empty() {
                return Some(Segment::Close);
            }
            member_pair(value, part).map(|(key, value)| Segment::Pair(key, value))
        })
        .collect()
}

/// One packed member - the `part` of `value` - split at its **first** `=`,
/// both halves trimmed.
///
/// A member's own reading, never a line's: what separates two members is the
/// bridge's vocabulary and nothing a line ever named, so the pair scanner has
/// no answer here and this is the one place FIX cuts a value of its own.
/// `Text=a;b` is one value with a semicolon, not two fields.
fn member_pair(value: &TextBytes, part: Range<usize>) -> Option<(TextBytes, TextBytes)> {
    let at = part.start + memchr::memchr(b'=', &value.as_bytes()[part.clone()])?;
    let key = trimmed(value, part.start, at);
    if key.is_empty() {
        return None;
    }
    Some((key, trimmed(value, at + 1, part.end)))
}

/// Where one occurrence's value splits on the bridge's member separator,
/// empty segments included.
///
/// The first explicit spelling the run actually carries wins, and only that
/// one splits it. With neither present, declared member names are boundaries;
/// an unresolved run remains one segment. Every part is a range of the value
/// and nothing more, so a wide occurrence costs the list, no byte, and no
/// count of the page until a part is read as a pair.
fn split_members(held: &[u8], declared: &[Field]) -> Vec<Range<usize>> {
    let Some(separator) = MEMBER_SEPARATORS
        .into_iter()
        .find(|marker| memchr::memmem::find(held, marker).is_some())
    else {
        return split_on_declared_members(held, declared);
    };
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(at) = memchr::memmem::find(&held[start..], separator) {
        parts.push(start..start + at);
        start += at + separator.len();
    }
    parts.push(start..held.len());
    parts
}

/// Splits a separator-less run at names declared directly by its group.
///
/// The first `KEY=` starts the run. After that, the earliest declared member
/// spelling followed by `=` starts the next pair. Matching uses the FIX name
/// fold, and the longest declared match at one byte wins. Bytes that match no
/// declared member remain verbatim in the surrounding pair.
fn split_on_declared_members(held: &[u8], declared: &[Field]) -> Vec<Range<usize>> {
    let mut parts = Vec::new();
    let Some(first_equals) = memchr::memchr(b'=', held) else {
        parts.push(0..held.len());
        return parts;
    };
    let mut start = 0;
    let mut at = first_equals + 1;
    while at < held.len() {
        let Some(prefix_len) = declared_member_prefix(&held[at..], declared) else {
            at += 1;
            continue;
        };
        parts.push(start..at);
        start = at;
        at += prefix_len;
    }
    parts.push(start..held.len());
    parts
}

/// The length through `=` of the longest declared member at this byte.
fn declared_member_prefix(value: &[u8], declared: &[Field]) -> Option<usize> {
    declared
        .iter()
        .filter_map(|field| {
            let name = field.name();
            let prefix = folded_name_prefix(value, name)?;
            let length = name.bytes().filter(|byte| !name_separator(*byte)).count();
            Some((length, prefix))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, prefix)| prefix)
}

/// The byte length through `=` when `value` opens with one folded FIX name.
fn folded_name_prefix(value: &[u8], name: &str) -> Option<usize> {
    if !name.is_ascii() {
        return None;
    }
    let mut at = 0;
    let mut matched = false;
    for expected in name.bytes().filter(|byte| !name_separator(*byte)) {
        let actual = *value.get(at)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            return None;
        }
        matched = true;
        at += 1;
        while value.get(at).is_some_and(|byte| name_separator(*byte)) {
            at += 1;
        }
    }
    (matched && value.get(at) == Some(&b'=')).then_some(at + 1)
}

/// One ASCII separator ignored by the FIX name fold.
const fn name_separator(byte: u8) -> bool {
    matches!(byte, b'_' | b'-' | b' ')
}

/// Judges one `#` key, stripped of its mark, against `twins` - the spellings
/// it may restate, absences already left out.
///
/// The twin is judged on the stem - the group a `NAME[0]` key addresses, or
/// the whole key - so a bare group claims every marked occurrence of it,
/// however many the two state. A twin whose whole key, not only its stem,
/// folds equal to the key's and that states the same bytes makes the marked
/// pair a duplicate; the bytes are compared trimmed of the space the row put
/// around them and never folded, because a value is a value and `abc` is
/// not `ABC`.
fn judge_hashed(stripped: &[u8], value: &[u8], twins: &[(&[u8], &[u8])]) -> Hashed {
    let stem = stem_of(stripped);
    let value = line::trim_ascii(value);
    let mut twinned = false;
    for &(held, held_value) in twins {
        if folds_twin(held, stripped) && line::trim_ascii(held_value) == value {
            return Hashed::Duplicate;
        }
        twinned |= folds_twin(stem_of(held), stem);
    }
    if twinned {
        Hashed::Verbatim
    } else {
        Hashed::Bare
    }
}

/// The group an indexed key addresses, or the key itself.
fn stem_of(key: &[u8]) -> &[u8] {
    group_index(key).map_or(key, |(group, _)| group)
}

/// Whether two key spellings name one field under the FIX name fold.
///
/// The identity a `#` key's twin is judged by - whether the mark drops,
/// stays, or the pair goes as a duplicate - because case and separators fold
/// away, and `OrderId=1|#ORDERID=2` merges under one name exactly as the
/// same-cased pair would.
fn folds_twin(left: &[u8], right: &[u8]) -> bool {
    let mut left = left.iter().filter(|byte| !name_separator(**byte));
    let mut right = right.iter().filter(|byte| !name_separator(**byte));
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(one), Some(other)) if one.eq_ignore_ascii_case(other) => {}
            _ => return false,
        }
    }
}
