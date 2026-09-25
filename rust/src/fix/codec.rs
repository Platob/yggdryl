//! What every line of one capture is read against.
//!
//! A [`FixCodec`] is the dictionary plus the few facts a whole run shares -
//! the version, the spellings that mean nothing was sent - and the
//! constructors on [`FixMsg`] take one. Each of them splits its own
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
//! `parse_*` turns what a capture holds into messages, each already filled
//! with what it implies: one line of bytes ([`FixCodec::parse_line`]), a
//! stream of them ([`FixCodec::parse_lines`]), one decoded line or a stream
//! of them ([`FixCodec::parse_text_line`], [`FixCodec::parse_text_lines`]),
//! and a stream of Arrow batches of capture rows
//! ([`FixCodec::parse_text_arrow_reader`]). [`FixCodec::lifecycle`] walks a
//! stream of messages as the chains they belong to, stating what each one
//! follows ([`FixCodec::lifecycle_arrow_reader`] over batches). `format_*`
//! answers messages under the field a consumer reads by. The two converters
//! between the message and the Arrow shape - [`FixCodec::messages`] and
//! [`FixCodec::arrow_reader`] - are what the Arrow twins compose, and are
//! public so a caller composes the same way. A stage is a call, never a
//! flag: nothing here takes a `lifecycle` argument, and a parse walks no
//! chain.
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
//! # Nothing is skipped, and nothing is invented
//!
//! A frame, a bridge row or a document with no message type is built anyway
//! and named `unknown`; a value that will not type is null; a group that
//! will not split stays whole; a document in `XmlData` that will not parse
//! stays the bytes it is. A line that states no message at all - no frame,
//! no bridge pair, no document - states none, and reads as no message
//! rather than as an empty one: a sentence carrying an `=` is
//! a sentence. What is left - input that is not a row at all - is an `Err`
//! item carrying it, and the stream continues, because one corrupt line must
//! not end a run over ten million.

use std::borrow::Borrow;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use quick_xml::events::Event;
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::graph::Element as _;
use crate::mime_type::line;
use crate::text::{TextBytes, TextEntries, TextEntry, TextLine, TextOptions};
use crate::{Error, Field, Result, Scalar, Version};

use super::build::{BEGINSTRING_COLUMN, Builder, Fill, FixPair, RowExtras, root_name, version_of};
use super::{FixMessages, FixMsg, FixRegistry};

/// One bridge row read into the pairs a build folds in, beside the message
/// type it declared.
type BridgeRow<'registry> = (
    Option<SmolStr>,
    Declared<'registry>,
    Vec<FixPair>,
    Vec<super::FixAnomaly>,
);

/// One pair a reader outside this module hands the build: the key as it
/// arrived, the value, and the dictionary's name for the field it fills
/// where that is not the key.
pub(super) type SpelledPair<'a> = (&'a [u8], &'a [u8], Option<&'a [u8]>);

/// The message one bridge row's groups are split under: the one its type
/// names, resolved once per row and carried down every packed occurrence,
/// because that is what says which group a shared counter heads.
type Declared<'registry> = Option<&'registry super::MsgType>;

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
    /// A second spelling of a bare pair: one pair, the bare one.
    Duplicate,
    /// The row's sole spelling: the key the line wrote after its mark.
    Bare,
    /// A marked occurrence of a group the row also states bare: one more
    /// occurrence of that group, numbered past the bare ones.
    Reindexed(usize),
}

/// The key one arriving pair builds under, its `#` judged.
enum Judged {
    /// A range of the line: the key as written, or the key after its mark.
    Ranged(TextBytes),
    /// A key the judgement rendered, which names no range of the line: a
    /// marked occurrence numbered past the bare ones.
    Rendered(Vec<u8>),
}

impl Judged {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Ranged(key) => key.as_bytes(),
            Self::Rendered(key) => key,
        }
    }

    /// The pair this key builds, over `value`.
    fn pair(self, value: TextBytes) -> FixPair {
        match self {
            Self::Ranged(key) => FixPair::own(key, value),
            Self::Rendered(key) => FixPair::read(key, value),
        }
    }
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
    entry.value_bytes().end() as usize == span
        || rest
            .iter()
            .any(|held| held.value_bytes().end() as usize == span)
        || entry
            .key_bytes()
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
    stated: Option<&[u8]>,
    entries: &[TextEntry],
    after: usize,
    xml: bool,
) -> Option<usize> {
    let opens = entry.key_bytes().end() as usize + 1;
    let stated = stated
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|text| text.parse::<usize>().ok());
    if let Some(span) = stated.and_then(|length| opens.checked_add(length)) {
        if cut_at(entry, &entries[after..], span) {
            let next = entries[after..]
                .iter()
                .find(|held| held.key_bytes().start() as usize >= span);
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
        .find(|held| held.key_bytes().as_bytes() == b"10")?;
    let at = checksum.key_bytes().start() as usize;
    let page = checksum.key_bytes().page()?;
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
/// A bridge with nothing to say writes empty text, `null`, `<null>`, `none`,
/// or `n/a` (with or without brackets); these spellings are omitted from parsed fields and entries. The
/// raw ASCII bytes are trimmed and compared case-insensitively. A custom
/// [`FixCodec::with_null_values`] replaces this set.
pub const DEFAULT_NULL_VALUES: [&str; 6] = ["", "null", "<null>", "none", "n/a", "[n/a]"];

/// The column a payload is read from when nothing names another.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// The message types a codec refuses until a caller says otherwise: the
/// session's own keepalives and a row that states no type at all.
///
/// A capture is mostly `Heartbeat` and `TestRequest` - a quiet session
/// writes one every thirty seconds and says nothing else - and neither
/// states anything about a market: no instrument, no order, no price. A row
/// that states no type is the third: a line a transport wrote that carries
/// no message this dictionary knows, read as `unknown`. Refusing the three
/// by default means the common reading of a capture is the messages that
/// say something, and the cost of the rest is one look at the type rather
/// than a parse.
///
/// Spelled as the wire spells them, because that is what a frame carries;
/// [`FixCodec::with_exclude_msgtypes`] takes any spelling the dictionary
/// resolves.
pub const DEFAULT_REFUSED_MSGTYPES: [&str; 3] = ["0", "1", super::build::UNKNOWN_MSGTYPE];

/// What one row-header capture states about the line it was read from.
///
/// Resolved once, when the codec is told what a run's captures are called,
/// and read by position afterwards: a line answers its captures in the order
/// the header declares them, so nothing looks a name up per row. A capture
/// naming nothing this codec knows is [`Silent`](Self::Silent), and silence
/// is never an instruction and never an error.
#[derive(Clone)]
enum CaptureRole {
    /// The version the line is read at.
    Version,
    /// A capture whose name reaches a field, beside the field it fills.
    /// A capture named `msgdirection` is one of these: tag 385 is a field
    /// like any other, and a fill never overrides what the
    /// line stated.
    Fill(Field, i32),
    /// A capture this codec has no use for, which is most of them.
    Silent,
}

impl CaptureRole {
    /// What one capture name means to this codec, decided once.
    ///
    /// A capture named for the capture's own column - `sourceurl` - is
    /// silent: what a reader says about a line is not something the message
    /// it holds says, so it fills no field here and is stated on the row by
    /// whoever read it. A capture named for one of the sixteen event columns
    /// is silent too: it is the line's own fact - the place, the state, the
    /// instant the line reads off it - and the line states its identity as
    /// the message's source, which is all a line says about a message; the
    /// batch door reads a carrier's event columns the same way.
    fn of(name: &str, codec: &FixCodec) -> Self {
        let is = |known: &str| crate::folds_equal(known, name);
        if is(BEGINSTRING_COLUMN) {
            return Self::Version;
        }
        if crate::graph::EventColumn::of_name(name).is_some() {
            return Self::Silent;
        }
        match codec.fill_target(name) {
            Some((_, tag)) if super::identity::is_capture_tag(tag) => Self::Silent,
            Some((field, tag)) => Self::Fill(field, tag),
            None => Self::Silent,
        }
    }
}

/// One capture read row by row, holding what is constant across them.
///
/// A capture is millions of lines and calling a singular reader per line
/// re-does per message what is constant for the whole run. Pinning the
/// version skips inference for every row - and a capture is one session, so
/// pinning is the normal case rather than an optimization.
#[derive(Clone)]
pub struct FixCodec {
    registry: Arc<FixRegistry>,
    /// Already validated; absence defers the one now read until intake.
    default_sending_time: Option<Scalar>,
    separator: Option<u8>,
    payload_column: SmolStr,
    /// What a run's row-header captures state, resolved in their order.
    ///
    /// Empty until a caller names them, because a codec that was told
    /// nothing reads a line's typed fields and no captures at all.
    captures: Arc<[CaptureRole]>,
    /// The spellings a value states an absence with, shared: a codec is
    /// cloned into every stream and every row of several messages, and a
    /// clone is reference counts and nothing else.
    null_values: Arc<[String]>,
    /// The code a line with no verb in front of its payload takes on the
    /// batch door: a code of tag 385's set, or none.
    direction: Option<SmolStr>,
    /// The registry's reading of tag 385, its rules compiled once.
    msgdirection: Arc<super::MsgDirection>,
    /// The message types this run reads, resolved to their wire codes, and
    /// the ones it refuses. Empty inclusions read every type the exclusions
    /// leave; `exclude_stated` remembers whether the refusals are this
    /// crate's own default or a caller's own list.
    include_msgtypes: Arc<[SmolStr]>,
    exclude_msgtypes: Arc<[SmolStr]>,
    exclude_stated: bool,
    /// The raw bytes one Arrow batch of messages targets.
    batch_byte_size: u64,
    /// The rows one Arrow batch of messages targets, whichever bound the
    /// batch reaches first.
    batch_row_size: usize,
    /// The threads the line and row doors read on. A new codec uses the
    /// available CPU count, falling back to one where it is unavailable.
    threads: usize,
    /// The lifecycle snapshot grid in nanoseconds; nonpositive disables it.
    snapshot_ns: i64,
    /// How far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, in milliseconds; nonpositive
    /// leaves only a clock equal to it.
    official_time_delay_ms: i64,
    /// The `BeginString` child every built message carries, resolved once:
    /// a bridge row states no version, so every one of them would otherwise
    /// look the field up per line.
    beginstring: Field,
}

/// The source a message parsed out of `line` states: the line's identity,
/// and none where the line has none - an instant no UUIDv7 holds is the nil
/// identity, and nil names no element.
fn source_of(line: &TextLine) -> Option<crate::Uuid> {
    Some(line.get_curruuid()).filter(|uuid| !uuid.is_nil())
}

/// One line's bytes as the page its messages are ranges of, or the refusal
/// a row of no bytes at all earns.
fn paged(row: &[u8]) -> Result<TextBytes> {
    if row.is_empty() {
        return Err(Error::Parse {
            target: "fix",
            position: 0,
            reason: "expected a captured row, got no bytes".into(),
        });
    }
    TextBytes::from_bytes(row)
}

/// One stream of messages, read where it stands or spread over threads.
pub(super) enum Spread<S, T> {
    Sequential(S),
    Threaded(T),
}

impl<S, T, M> Iterator for Spread<S, T>
where
    S: Iterator<Item = M>,
    T: Iterator<Item = M>,
{
    type Item = M;

    fn next(&mut self) -> Option<M> {
        match self {
            Self::Sequential(held) => held.next(),
            Self::Threaded(held) => held.next(),
        }
    }
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

    /// The rows one Arrow batch targets when the caller states none.
    ///
    /// The other half of the bound, and the one that matters at the small
    /// end: a stream of heartbeats is a few dozen bytes a row, so a batch
    /// bounded by bytes alone would hold millions of them and a consumer
    /// would wait for the whole capture before seeing a batch. Thirty-two
    /// thousand rows is large enough that the per-batch cost is amortized
    /// and small enough that a reader sees rows while the capture is still
    /// being read. A batch closes on whichever bound it reaches first.
    pub const DEFAULT_BATCH_ROW_SIZE: usize = 32 * 1024;

    /// How far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, when the caller states none.
    ///
    /// The venue's clock and the session's are two clocks, and what stands
    /// between them is the hop: a transaction stamped when it happened
    /// reaches the wire microseconds later out of a matching engine and
    /// tens of milliseconds later through a bridge, so a clock that close
    /// is the same event said twice and the more exact saying of it is the
    /// venue's. A `TransactTime(60)` a whole second off the sending clock
    /// is a different event of the session's day - a resend of an older
    /// order, a report batched behind the trades it covers, a clock nobody
    /// disciplined - and dating the message by it would move it out of the
    /// order it was sent in. One second is wide enough to hold every hop a
    /// capture actually shows and narrow enough that nothing else crosses.
    pub const DEFAULT_OFFICIAL_TIME_DELAY_MS: i64 = 1_000;

    /// [`Self::DEFAULT_OFFICIAL_TIME_DELAY_MS`] as the nanosecond distance a
    /// dating compares, for the doors that build a message without a codec
    /// to state one.
    pub(super) const DEFAULT_OFFICIAL_TIME_DELAY_NS: i64 =
        Self::DEFAULT_OFFICIAL_TIME_DELAY_MS * 1_000_000;

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
        let msgdirection = registry.msgdirection();
        let direction = Some(SmolStr::new(msgdirection.sent()));
        Self {
            registry,
            default_sending_time: None,
            separator: None,
            payload_column: SmolStr::new_static(DEFAULT_PAYLOAD_COLUMN),
            captures: Arc::from([]),
            null_values: DEFAULT_NULL_VALUES
                .iter()
                .map(|spelling| (*spelling).to_owned())
                .collect(),
            direction,
            msgdirection: Arc::new(msgdirection),
            include_msgtypes: Arc::from([]),
            exclude_msgtypes: DEFAULT_REFUSED_MSGTYPES
                .iter()
                .map(|spelling| SmolStr::new_static(spelling))
                .collect(),
            exclude_stated: false,
            batch_byte_size: Self::DEFAULT_BATCH_BYTE_SIZE,
            batch_row_size: Self::DEFAULT_BATCH_ROW_SIZE,
            threads: std::thread::available_parallelism().map_or(1, usize::from),
            snapshot_ns: 0,
            official_time_delay_ms: Self::DEFAULT_OFFICIAL_TIME_DELAY_MS,
            beginstring,
        }
    }

    /// The dictionary every message is read against.
    #[must_use]
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// The exact nanosecond/UTC clock used when neither message nor carrier
    /// states SendingTime - a carrier being a row cell reaching tag 52, or
    /// the `currunix` of the [`TextLine`] the message was read out of.
    /// `None` reads UTC now lazily at fresh intake.
    #[must_use]
    pub const fn default_sending_time(&self) -> Option<&Scalar> {
        self.default_sending_time.as_ref()
    }

    /// Sets the fallback SendingTime without coercing a different layout.
    ///
    /// # Errors
    ///
    /// Refuses null or any value other than DateTime64(ns, UTC), atomically.
    pub fn set_default_sending_time(&mut self, value: Option<Scalar>) -> Result<()> {
        if let Some(value) = &value {
            super::identity::validate_value(
                "default_sending_time",
                &super::schema::CLOCK_DATATYPE,
                value,
            )?;
        }
        self.default_sending_time = value;
        Ok(())
    }

    /// [`Self::set_default_sending_time`], consuming the codec.
    ///
    /// # Errors
    ///
    /// Returns the setter's exact-layout refusal.
    pub fn try_with_default_sending_time(mut self, value: Option<Scalar>) -> Result<Self> {
        self.set_default_sending_time(value)?;
        Ok(self)
    }

    /// The code a line with no verb in front of its payload takes on the
    /// batch door, a code of tag 385's set; `None` is no pin.
    #[must_use]
    pub fn direction(&self) -> Option<&str> {
        self.direction.as_deref()
    }

    /// The registry's reading of tag 385, as this codec compiled it.
    #[must_use]
    pub fn msgdirection(&self) -> &super::MsgDirection {
        &self.msgdirection
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
    /// means: the version, a field a capture's
    /// name reaches, or nothing. Pass what
    /// [`TextOptions::capture_names`] answers for the options the lines were
    /// read under, and every line of the run is then read without one name
    /// lookup.
    ///
    /// A capture is what the transport wrote around the line, never what the
    /// line itself says: the body is the message, so a fact read out of it is
    /// already a field this codec fills from its tag.
    ///
    /// [`TextOptions::capture_names`]: crate::text::TextOptions::capture_names
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::text::{TextBytes, TextLine};
    /// # use yggdryl::{FixCodec, FixRegistry, MSGPLUGINID_TAG_NAME};
    /// let codec = FixCodec::new(Arc::new(FixRegistry::new())).with_capture_names(["msgpluginid"]);
    ///
    /// let options = Arc::new(yggdryl::text::TextOptions::new());
    /// let line = TextLine::from_bytes(0, TextBytes::from_bytes(b"8=FIX.4.4|35=D|11=A|10=0|")?, options)?
    ///     .with_captures(vec![Some(TextBytes::from_bytes(b"VNU")?)])?;
    /// let message = codec.parse_text_line(&line)?.next().expect("one message")?;
    /// // The capture filled the crate's own field.
    /// assert_eq!(message.by_tag(MSGPLUGINID_TAG_NAME.0)?.as_str(), Some("VNU"));
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

    /// Sets the code a line with no verb in front of its payload takes on
    /// the batch door.
    ///
    /// The set's `Send` code by default - `S` in the specification's set:
    /// a session's own log is written by the side doing the sending, so its
    /// unmarked lines are the ones it sent and its inbound lines are the
    /// ones it bothered to mark. A capture taken from the other side pins
    /// the `Receive` code, and one whose silence really means unknown pins
    /// `None` or the empty text. Any verb a line does carry beats this, and
    /// so does a `msgdirection` column a batch row states.
    ///
    /// `code` is any spelling of a code of tag 385's set - its value, its
    /// name, an alias - resolved once here through
    /// [`MsgDirection::code`](super::MsgDirection::code); the pin is stored
    /// as the code itself.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the spelling and the set when the
    /// set holds no such code.
    pub fn try_with_direction(mut self, code: Option<&str>) -> Result<Self> {
        let Some(spelling) = code.map(str::trim).filter(|held| !held.is_empty()) else {
            self.direction = None;
            return Ok(self);
        };
        let Some(code) = self.msgdirection.code(spelling) else {
            let codes: Vec<&str> = self.msgdirection.codes().collect();
            return Err(Error::Parse {
                target: "msgdirection",
                position: 0,
                reason: crate::text::expected_got(
                    format_args!("one of {}", codes.join(", ")),
                    format_args!("{spelling:?}"),
                ),
            });
        };
        self.direction = Some(SmolStr::new(code));
        Ok(self)
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

    /// Sets the rows one Arrow batch of messages targets.
    ///
    /// The second of the two bounds, and a batch closes on whichever it
    /// reaches first; the default is [`Self::DEFAULT_BATCH_ROW_SIZE`]. Zero
    /// leaves the bytes to decide, since a batch always holds one row.
    #[must_use]
    pub const fn with_batch_row_size(mut self, rows: usize) -> Self {
        self.batch_row_size = rows;
        self
    }

    /// The rows one Arrow batch of messages targets.
    #[must_use]
    pub const fn batch_row_size(&self) -> usize {
        self.batch_row_size
    }

    /// Sets the threads the line and row doors read on.
    ///
    /// One reads a stream where it stands, a line at a time. A new codec uses
    /// the available CPU count, falling back to one. More threads read line,
    /// message-row and write doors ahead in chunks of [`Self::PARALLEL_CHUNK`]
    /// lines, two chunks per thread; the Arrow capture doors instead hand one
    /// whole input batch to each worker, at most one batch per worker ahead.
    /// Each job is parsed on the thread it was handed to and every message is
    /// answered in the lines' order, so
    /// [`Self::parse_lines`], [`Self::parse_text_lines`],
    /// [`Self::parse_arrow_messages`] and [`Self::messages`] answer exactly
    /// what one thread answers, sooner, and [`Self::arrow_reader`] and
    /// [`Self::parse_text_arrow_reader`] fill their rows the same way. The
    /// threads live for the stream, so what a thread learns of the
    /// dictionary it keeps, and the thread that pulls reads the next chunk
    /// while they work the ones before it. A [`lifecycle`](Self::lifecycle)
    /// is one walk and reads on the thread that pulls it whatever this
    /// says. Zero reads as one.
    #[must_use]
    pub const fn with_threads(mut self, threads: usize) -> Self {
        self.set_threads(threads);
        self
    }

    /// [`Self::with_threads`], in place.
    pub const fn set_threads(&mut self, threads: usize) {
        self.threads = if threads == 0 { 1 } else { threads };
    }

    /// The threads the line and row doors read on.
    #[must_use]
    pub const fn threads(&self) -> usize {
        self.threads
    }

    /// Sets the epoch-aligned lifecycle snapshot grid in nanoseconds. A
    /// nonpositive width disables snapshots, which is the default.
    #[must_use]
    pub const fn with_snapshot_ns(mut self, snapshot_ns: i64) -> Self {
        self.snapshot_ns = snapshot_ns;
        self
    }

    /// The lifecycle snapshot grid in nanoseconds, where enabled.
    #[must_use]
    pub const fn snapshot_ns(&self) -> Option<i64> {
        if self.snapshot_ns > 0 {
            Some(self.snapshot_ns)
        } else {
            None
        }
    }

    /// Sets how far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, in milliseconds.
    ///
    /// The sending clock is the reference every parse dates against, and the
    /// message is dated by the best official clock standing within this
    /// distance of it, on either side. The `TransactTime(60)` the message
    /// states outranks everything; below it stand the `TrdRegTimestamp(769)`
    /// occurrences of a `TrdRegTimestamps(768)` group, ranked by what their
    /// `TrdRegTimestampType(770)` says each one is - the event itself before
    /// a hop the message crossed, and a stamp about the trade's afterlife
    /// never a clock at all. Two of one rank are decided by the nearer, and
    /// the sending clock dates the message where none stands that near.
    /// The default is [`Self::DEFAULT_OFFICIAL_TIME_DELAY_MS`]; a nonpositive
    /// delay admits only a clock equal to the sending clock, which is the
    /// reading that dates nothing the sending clock did not already date.
    #[must_use]
    pub const fn with_official_time_delay_ms(mut self, official_time_delay_ms: i64) -> Self {
        self.official_time_delay_ms = official_time_delay_ms;
        self
    }

    /// How far from `SendingTime(52)` an official transaction clock may stand
    /// and still date the message, in milliseconds.
    #[must_use]
    pub const fn official_time_delay_ms(&self) -> i64 {
        self.official_time_delay_ms
    }

    /// The delay as the nanosecond distance a dating compares against: never
    /// negative, and saturating rather than wrapping on a delay stated in
    /// milliseconds no span of nanoseconds can hold.
    pub(super) const fn official_time_delay_ns(&self) -> i64 {
        if self.official_time_delay_ms <= 0 {
            return 0;
        }
        self.official_time_delay_ms.saturating_mul(1_000_000)
    }

    /// The lines one chunk holds where the doors read on several threads:
    /// what one hand-over to a thread carries, and with the two chunks a
    /// thread holds, what bounds the read-ahead.
    pub const PARALLEL_CHUNK: usize = 64;

    /// The lines one chunk holds: one thread reads none ahead.
    pub(super) const fn chunk(&self) -> usize {
        if self.threads == 1 {
            1
        } else {
            Self::PARALLEL_CHUNK
        }
    }

    /// The message types this codec reads, named in any spelling the
    /// dictionary resolves.
    ///
    /// Empty - the default - reads every type the refusals leave. Naming
    /// even one makes this the whole answer: only these are read, and the
    /// default refusals step aside, because a caller that says what it wants
    /// has already said what it does not. A caller that wants both states
    /// both, and the refusal wins where they disagree.
    ///
    /// A spelling is resolved once, here: `Heartbeat`, `HEARTBEAT` and `0`
    /// are one type, and a code no dictionary knows is kept as it was
    /// written, so a venue's own type is named the way the venue names it.
    /// The word `unknown` names a row that states no type at all.
    ///
    /// The filter is applied to the type a row *states*, before the message
    /// is built, so a refused type costs one look rather than a parse.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&yggdryl::local::LocalFolder::new(root)?)?);
    /// // Any spelling the dictionary resolves: `NewOrderSingle` is `35=D`.
    /// let orders = FixCodec::new(Arc::clone(&registry)).with_include_msgtypes(["NewOrderSingle"]);
    /// let lines = ["8=FIX.4.4|35=D|11=A|10=0|", "8=FIX.4.4|35=8|37=O1|10=0|"];
    /// let read: Vec<_> = orders.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    /// assert_eq!(read.len(), 1);
    /// assert_eq!(read[0].by_tag(11)?.as_str(), Some("A"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_include_msgtypes<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.include_msgtypes = self.resolved_msgtypes(spellings);
        if !self.exclude_stated {
            self.exclude_msgtypes = Arc::from([]);
        }
        self
    }

    /// The message types this codec refuses, named in any spelling the
    /// dictionary resolves.
    ///
    /// Replaces [`DEFAULT_REFUSED_MSGTYPES`] - the session's keepalives and
    /// the untyped row - so an empty list reads everything, which is what a
    /// caller auditing a session sets. Spellings resolve as
    /// [`Self::with_include_msgtypes`] resolves them, and a type named here
    /// is refused whether or not it is included.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// # let registry = Arc::new(FixRegistry::new());
    /// let lines = ["8=FIX.4.4|35=0|10=0|", "8=FIX.4.4|35=D|11=A|10=0|"];
    /// // The keepalive is refused by default and read where nothing is.
    /// let quiet = FixCodec::new(Arc::clone(&registry));
    /// assert_eq!(quiet.parse_lines(lines).count(), 1);
    /// let everything = quiet.clone().with_exclude_msgtypes::<[&str; 0], &str>([]);
    /// assert_eq!(everything.parse_lines(lines).count(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_exclude_msgtypes<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.exclude_msgtypes = self.resolved_msgtypes(spellings);
        self.exclude_stated = true;
        self
    }

    /// The types this codec reads, as it resolved them; empty reads every
    /// type [`Self::exclude_msgtypes`] leaves.
    #[must_use]
    pub fn include_msgtypes(&self) -> &[SmolStr] {
        &self.include_msgtypes
    }

    /// The types this codec refuses, as it resolved them.
    #[must_use]
    pub fn exclude_msgtypes(&self) -> &[SmolStr] {
        &self.exclude_msgtypes
    }

    /// Whether a message of this type is read, by any spelling of it.
    ///
    /// What the parse asks of every row before it builds one, and what a
    /// walk asks of every message it is handed. The word `unknown` - or an
    /// empty spelling - asks about a row that states no type.
    #[must_use]
    pub fn reads_msgtype(&self, spelling: &str) -> bool {
        self.reads_code(&self.resolve_msgtype(spelling))
    }

    /// One configured spelling as the code it names.
    fn resolve_msgtype(&self, spelling: &str) -> SmolStr {
        if spelling.is_empty() || crate::folds_equal(spelling, super::build::UNKNOWN_MSGTYPE) {
            return SmolStr::new_static(super::build::UNKNOWN_MSGTYPE);
        }
        self.registry.get_msgtype(spelling).map_or_else(
            || SmolStr::new(spelling),
            |held| SmolStr::new(held.as_str()),
        )
    }

    /// Every configured spelling as the codes they name, each once.
    fn resolved_msgtypes<I, S>(&self, spellings: I) -> Arc<[SmolStr]>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut held: Vec<SmolStr> = Vec::new();
        for spelling in spellings {
            let code = self.resolve_msgtype(spelling.as_ref());
            if !held.contains(&code) {
                held.push(code);
            }
        }
        Arc::from(held)
    }

    /// Whether one resolved code is read: refused where the refusals name
    /// it, else read where nothing is included or the inclusions name it.
    fn reads_code(&self, code: &SmolStr) -> bool {
        if self.exclude_msgtypes.contains(code) {
            return false;
        }
        self.include_msgtypes.is_empty() || self.include_msgtypes.contains(code)
    }

    /// Whether this codec reads every type: nothing included and nothing
    /// refused, which is the common configuration and worth not paying a
    /// read of the type for.
    fn reads_all(&self) -> bool {
        self.include_msgtypes.is_empty() && self.exclude_msgtypes.is_empty()
    }

    /// Whether a row stating `stated` is read, the type taken off the pairs
    /// before anything is built; a row stating none asks about `unknown`.
    fn reads_stated(&self, stated: Option<&str>) -> bool {
        if self.reads_all() {
            return true;
        }
        self.reads_msgtype(stated.unwrap_or_default())
    }

    /// Whether a run of pairs stating its type among them is read: the type
    /// is taken off the run only where a refusal could name it.
    fn reads_run(&self, run: &[TextEntry]) -> bool {
        self.reads_all() || self.reads_stated(stated_type(run).as_deref())
    }

    /// Whether a document stating these pairs is read.
    fn reads_pairs(&self, pairs: &[(Vec<u8>, Vec<u8>)]) -> bool {
        if self.reads_all() {
            return true;
        }
        let stated = msgtype_of(
            pairs
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice())),
        );
        self.reads_stated(stated.as_deref())
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
    /// A column is the caller's, not the line's, so it resolves once per
    /// column rather than once per row, and a stream's columns fill the same
    /// fields whatever each row turns out to say.
    pub(super) fn fill_target(&self, name: &str) -> Option<(Field, i32)> {
        let (field, tag) = super::build::fill_field(&self.registry, name)?;
        let mut field = field.clone();
        field.set_nullable(false);
        Some((field, tag))
    }

    /// Whether one raw value is a stated absence rather than a value.
    fn is_absent(&self, value: &[u8]) -> bool {
        let trimmed = line::trim_ascii(value);
        self.null_values
            .iter()
            .any(|spelling| spelling.as_bytes().eq_ignore_ascii_case(trimmed))
    }

    fn entries(&self, page: &TextBytes) -> (Option<TextEntries>, line::Located) {
        TextEntries::from_bytes_direct_located(page)
    }

    /// Parses one log line into an iterator of the messages it carries,
    /// selecting the dialect from its frame.
    ///
    /// A row yields none, one or many: one per frame the line
    /// holds, one per configuration a bulk UL answer named, and none at all
    /// for a line that states no message - which a document naming no
    /// configuration is, as much as a sentence is.
    ///
    /// # A message is the frame; the line is still the line
    ///
    /// The line states every pair it wrote, a transport's own prefix and
    /// anything after the checksum included, because what a line said and what
    /// one message said are two facts and only one of them is this reader's.
    /// A message begins where the frame the line carries opens and ends at its
    /// checksum: a log printing `ts=` and `thread=` in front of the frame it
    /// quoted states neither field, and a message that swallowed them would
    /// answer them by name and re-emit them.
    ///
    /// The pairs behind that checksum begin the next message where they open
    /// a frame of their own - an unmarked `8=`, and where the rest states
    /// none, an unmarked `35=` - so a line carrying two frames reads as two,
    /// each re-emitting its own bytes. A frame that stated no checksum ends
    /// where the next `8=` opens, and the tail of the last frame is the last
    /// frame's. A bridge row is one message and a frame behind it a second;
    /// a tag run behind a bridge row that opens no frame stays part of it,
    /// which is the mixed form a bridge writes. A key the bridge marked is
    /// the bridge's own spelling, so a `#8=` opens nothing and a `#10=`
    /// closes nothing.
    ///
    /// A row that opens no frame, states no bridge pair and carries no
    /// document yields nothing: `unknown` names a frame, a bridge row or a
    /// document that stated no type, never a line that stated no frame. What
    /// makes a run of named pairs a bridge row rather than prose carrying an
    /// `=` is the separator the line named for it - a pipe, a `SOH`, or one
    /// of the spellings a log escapes it with, never whitespace - or a key
    /// the bridge marked. A numeric frame needs neither.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// let codec = FixCodec::new(Arc::new(FixRegistry::new()));
    /// let both = b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O|10=002|";
    /// let read: Vec<Vec<u8>> = codec
    ///     .parse_line(both)?
    ///     .map(|message| message.map(|message| message.into_bytes(b'|')))
    ///     .collect::<yggdryl::Result<_>>()?;
    /// assert_eq!(read.len(), 2, "a line of two frames is two messages");
    /// assert_eq!(read[0], b"8=FIX.4.4|35=D|11=A|10=001|", "each its own bytes");
    /// assert_eq!(read[1], b"8=FIX.4.4|35=8|37=O|10=002|");
    /// // A sentence states no message, whatever `=` it happens to hold.
    /// assert!(codec.parse_line(b"heartbeat emitted seq=7")?.next().is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    /// [`Self::parse_line`] reads it - one message per frame, one per
    /// configuration a bulk answer named, none for a line that states no
    /// message - and
    /// a line that is not a row at all is an `Err` item; the stream
    /// continues past it, because one corrupt line must not end a run over
    /// ten million. Nothing is collected: the iterator is the
    /// stream. Every stage answers an iterator that owns its codec and
    /// borrows nothing, so the stages compose into [`Self::arrow_reader`]
    /// without the codec outliving the stream.
    ///
    /// Composed into [`Self::arrow_reader`], that `Err` item is the batch
    /// reader's error: the completed prefix is yielded, then the error, and
    /// the reader fuses, as every batch reader in the crate does - a
    /// consumer of batches has no row to put a refused line in. A caller
    /// wanting a row per *line* reads the capture through the text reader;
    /// a FIX batch answers one row per message, so a line stating none
    /// answers no row.
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
        let threads = self.threads();
        // Each line's bytes are made a page on the thread that pulls,
        // exactly as [`Self::parse_line`] makes them one: the page is what
        // crosses to a thread, and every key and value the messages record
        // is a range of it.
        let pages = lines.into_iter().map(|line| paged(line.as_ref()));
        crate::parallel::ordered(
            pages,
            threads,
            self.chunk(),
            move |page: Result<TextBytes>| {
                let messages = FixMessages::from_result(
                    page.and_then(|page| codec.parse_page_with(&page, RowExtras::NONE)),
                );
                if threads > 1 {
                    messages.collected()
                } else {
                    messages
                }
            },
        )
        .flatten()
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
    /// | [`captures`](TextLine::captures) | the version and every field a capture's name reaches, by the positions [`Self::with_capture_names`] resolved |
    ///
    /// A line that states none of them reads exactly as its bytes would,
    /// which is what makes this an entry point and not a second contract.
    ///
    /// # The line is the source, and its own fields reach no message
    ///
    /// Every message the line carries states the line's identity -
    /// [`Element::get_curruuid`](crate::graph::Element::get_curruuid), which
    /// the line derives from its instant and its bytes - as its one source,
    /// [`Element::get_srcuuids`](crate::graph::Element::get_srcuuids): the
    /// same line parsed again states the same source, and a message parsed
    /// from raw bytes states none. The line's `mtime` - its `currunix` -
    /// fills the message's `recdunix` as the carrier's recording clock, and
    /// dates a message stating no `SendingTime(52)` that no capture reaching
    /// tag 52 dated either: it is the sending clock such a message is read
    /// against, ahead of [`Self::default_sending_time`] and of now, and is
    /// never a fact of the message, so it reaches neither the wire nor the
    /// row's tag-52 column. Nothing else a [`TextLine`] holds is
    /// communicated: not the object it names, not its media type or its
    /// place in that object, not the body itself as a value - and a capture
    /// named for one of [the capture's own columns](FixMsg::from_row) fills
    /// nothing either. The answer is
    /// the message the line's bytes parsed to and no more. Where a line came
    /// from is the reader's to state, on the row, which is what
    /// [`Self::parse_text_arrow_reader`] does with the columns the batch
    /// already carries.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a line's content cannot provoke.
    pub fn parse_text_line(&self, line: &TextLine) -> Result<FixMessages> {
        // The row's own cells, read by the position the codec resolved.
        let text = |at: usize| line.capture(at);
        let mut version = None;
        // The field is the codec's own and outlives this call, so a cell
        // borrows it rather than cloning a name, a datatype, a metadata
        // handle and an Arrow cache once per filled capture per line. Only
        // the value is new, and only it is owned here - on the stack while
        // the header fills at most eight fields.
        let mut cells: SmallVec<[(&Field, i32, Scalar); 8]> = SmallVec::new();
        for (at, role) in self.captures.iter().enumerate() {
            match role {
                CaptureRole::Version => version = text(at).and_then(version_of),
                CaptureRole::Fill(field, tag) => {
                    let Some(held) = text(at) else { continue };
                    cells.push((field, *tag, Scalar::from(held)));
                }
                CaptureRole::Silent => {}
            }
        }
        let fills: SmallVec<[Fill<'_>; 8]> = cells
            .iter()
            .map(|(field, tag, value)| Fill {
                field,
                tag: *tag,
                value,
            })
            .collect();
        let extras = RowExtras {
            version,
            fills: &fills,
            direction: None,
            direction_pin: None,
            source: source_of(line),
            recdunix: line.mtime()?,
            originator: None,
            conversation: None,
        };
        let page = line.body_bytes();
        if page.is_empty() {
            // A row with no payload at all carries no message.
            return Ok(FixMessages::none());
        }
        self.parse_page_with(page, extras)
            .or_else(|_| self.empty_with(extras).map(FixMessages::one))
    }

    /// Parses a stream of decoded lines into a stream of messages, lazily.
    ///
    /// Each line is read as [`Self::parse_text_line`] reads it - none, one or
    /// many messages a line - and a payload nobody could read is an empty
    /// message rather than the end of the run: one corrupt line must not end
    /// a capture of ten million. Owned and borrowed lines, or their fallible
    /// counterparts, compose directly. Source errors move through unchanged;
    /// an owned line is never cloned, a borrowed one only to cross to a
    /// worker thread, and source exhaustion is fused.
    pub fn parse_text_lines<I, L>(
        &self,
        lines: I,
    ) -> impl Iterator<Item = Result<FixMsg>> + use<I, L>
    where
        I: IntoIterator,
        I::Item: Into<Result<L>>,
        L: Borrow<TextLine> + Into<TextLine>,
    {
        let codec = self.clone();
        let lines = lines.into_iter().map(Into::<Result<L>>::into);
        if self.threads() == 1 {
            // Where it stands: a borrowed line is read borrowed.
            return Spread::Sequential(lines.flat_map(move |line| {
                FixMessages::from_result(
                    line.and_then(|line: L| codec.parse_text_line(line.borrow())),
                )
            }));
        }
        // A line crosses to a thread owned: an owned one moves, and a
        // borrowed one is made the door's own first - a reference count per
        // page it is a range of, never a byte - and read where it lands.
        let owned = lines.map(|line| line.map(Into::<TextLine>::into));
        Spread::Threaded(
            crate::parallel::ordered(
                owned,
                self.threads(),
                self.chunk(),
                move |line: Result<TextLine>| {
                    FixMessages::from_result(line.and_then(|line| codec.parse_text_line(&line)))
                        .collected()
                },
            )
            .flatten(),
        )
    }

    /// One row's payload read under what the row stated, a row of nothing
    /// included, as the text line the row was cut from.
    ///
    /// The payload is the line: it is read into a [`TextLine`] dated by the
    /// row's own `mtime`, so what the messages state as their source is the
    /// identity the line door states for the same line, their `recdunix` is
    /// that recording clock - and the sending clock of a message stating
    /// none, as the line door reads it - and the text the codec reads is the
    /// text a line is. A row's content can never fail
    /// the batch it arrives in: a payload nobody could read is a row
    /// holding an empty message, dated and versioned by what the row itself
    /// said. A row that carried no message to read is a different fact and
    /// answers no message at all.
    pub(super) fn parse_row_with(
        &self,
        extras: RowExtras<'_>,
        payload: &[u8],
        mtime: Option<i64>,
        options: &Arc<TextOptions>,
    ) -> FixMessages {
        if payload.is_empty() {
            // A row with no payload at all carries no message.
            return FixMessages::none();
        }
        // Page capacity is a materialization bound, never malformed syntax.
        let line = TextBytes::from_bytes(payload)
            .and_then(|page| TextLine::from_bytes(0, page, Arc::clone(options)));
        let line = match line {
            Ok(mut line) => {
                line.set_handle_mtime(mtime);
                line
            }
            Err(error) => return FixMessages::from_result(Err(error)),
        };
        // The row's own identity where the carrier stated one, else the
        // identity its bytes and its instant derive.
        let extras = RowExtras {
            source: extras.source.or_else(|| source_of(&line)),
            recdunix: mtime,
            ..extras
        };
        FixMessages::from_result(
            self.parse_page_with(line.body_bytes(), extras)
                .or_else(|_| self.empty_with(extras).map(FixMessages::one)),
        )
    }

    /// What a byte door states beside the line it was handed whole: the
    /// direction the reading names, and nothing else.
    ///
    /// The single-dialect doors take one frame as bytes and locate it
    /// themselves, so the reading locates it once more here; they are not
    /// the per-row path, which resolves the direction where it located the
    /// frame.
    fn read_extras(&self, body: &[u8]) -> RowExtras<'_> {
        RowExtras {
            direction: self.msgdirection.read_bytes(body),
            ..RowExtras::NONE
        }
    }

    /// A payload nobody could read, which is still a row - dated and
    /// versioned as every row is, by what the row itself stated.
    ///
    /// A row that carried nothing to read is not this: it answers no message.
    /// This is the payload that was there and would not
    /// parse, which a batch must not fail on.
    fn empty_with(&self, extras: RowExtras<'_>) -> Result<FixMsg> {
        self.build_pairs_with(&[], extras)
    }

    /// [`Self::parse_line`], with what the row stated beside its line.
    pub(super) fn parse_line_with(&self, row: &[u8], extras: RowExtras<'_>) -> Result<FixMessages> {
        self.parse_page_with(&paged(row)?, extras)
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
        let (entries, located) = self.entries(page);
        let entries = entries.unwrap_or_default();
        let frame_at = located.frame_at;
        // A row that located no frame may carry a JSON document instead, and
        // the scan that finds one is run here and nowhere else: the span is
        // kept whole, so where the payload opens is the one answer.
        let document = frame_at.is_none().then(|| line::json_span(row)).flatten();
        let opens = frame_at
            .or_else(|| document.as_ref().map(|span| span.start))
            .unwrap_or(row.len());
        // The row's stated direction, else the reading over the prose in
        // front of the payload, else the pin the door supplied. Resolved
        // here, where the payload was located, so the frame is located once.
        let direction = extras
            .direction
            .or_else(|| self.msgdirection.read_prefix(&row[..opens.min(row.len())]));
        // What the same prose says about where the message came from, read
        // where it was located: the plugin a bridge's own sentence names it
        // arriving through - on a `Receiving :` line, the one that logged it
        // - and the conversation it files it under.
        let prose = &row[..opens.min(row.len())];
        let msgpluginid = extras
            .fills
            .iter()
            .find(|fill| fill.tag == super::MSGPLUGINID_TAG_NAME.0)
            .and_then(|fill| fill.value.as_str());
        let extras = RowExtras {
            direction: direction.or(extras.direction_pin),
            direction_pin: None,
            originator: extras
                .originator
                .or_else(|| super::ulbridge::originator(prose, msgpluginid)),
            conversation: extras
                .conversation
                .or_else(|| super::ulbridge::conversation(prose)),
            ..extras
        };
        let framed = framed_entries(entries.as_slice(), page.start() as usize + opens);
        // Where the payload opens among the row's own entries, and where the
        // row's first message does: the bridge's own row in front of a frame
        // is a message of its own where the bridge marked one of its keys,
        // and the transport's prose otherwise.
        let payload = entries.as_slice().len() - framed.len();
        let opened = entries.as_slice()[..payload]
            .iter()
            .position(TextEntry::marked)
            .unwrap_or(payload);
        if framed.first().is_some_and(tag_keyed) {
            let end = payload + frame_end(framed);
            if opened == payload && next_frame(entries.as_slice(), end).is_none() {
                // One frame and nothing in front of it: the row is read
                // where it stands, which is what a row of one message costs.
                let run = &entries.as_slice()[payload..end];
                if !self.reads_run(run) {
                    return Ok(FixMessages::none());
                }
                return Ok(FixMessages::from_result(
                    self.frame_with(run, extras).map(FixMessages::one),
                ));
            }
            let stamp = super::build::RowStamp::retained(extras);
            return Ok(FixMessages::frames(self.clone(), entries, opened, stamp));
        }
        // An XML document a transport wrote prose in front of opens before
        // any pair the locator could read as a bridge row, and is read as
        // the document it is, by its attributes.
        if let Some((_, open)) = line::document_behind_prefix(row) {
            let pairs = fixml_pairs(&row[open..])?;
            if !self.reads_pairs(&pairs) {
                return Ok(FixMessages::none());
            }
            return Ok(FixMessages::from_result(
                self.fixml_with(&pairs, extras).map(FixMessages::one),
            ));
        }
        // A JSON document is a body this codec does not read: the row said
        // something, and what it said is one message stating no type and
        // no entries - `unknown` names it - carrying what the row stated
        // beside it, its clock and its own columns.
        if document.is_some() {
            // A body this codec does not read states no type, so it is the
            // row read as `unknown` and the refusals answer for it.
            if !self.reads_stated(None) {
                return Ok(FixMessages::none());
            }
            return Ok(FixMessages::from_result(
                self.build_pairs_with(&[], extras).map(FixMessages::one),
            ));
        }
        let body = &row[opens..];
        // A FIXML row states no `key=value` frame, so the locator finds none
        // and leaves nothing to read. The document is the payload, and it
        // opens at the first tag - which is also how a prefix is dropped from
        // one, since everything before that tag is text the reader skips.
        if body.is_empty() && memchr::memchr(b'<', row).is_some() {
            let pairs = fixml_pairs(row)?;
            if !self.reads_pairs(&pairs) {
                return Ok(FixMessages::none());
            }
            return Ok(FixMessages::from_result(
                self.fixml_with(&pairs, extras).map(FixMessages::one),
            ));
        }
        // A run of named pairs is a bridge row where the line named a
        // separator for it or the bridge marked one of its keys, and prose
        // carrying an `=` where it did neither: a row that opens no frame,
        // states no bridge pair and carries no document yields nothing at
        // all.
        // The row is the whole run of pairs the line held: a key the bridge
        // marked is the bridge's own spelling, so a `#8=` the scanner read
        // as a tag relocated the payload past pairs that are the row's.
        // Whether the line named a separator is what the one scan above
        // already answered; the marks are read off the entries it cut.
        let held = entries.as_slice();
        if framed.is_empty() || !(held.iter().any(TextEntry::marked) || located.separated) {
            return Ok(FixMessages::none());
        }
        if next_frame(held, 0).is_none() {
            if !self.reads_run(held) {
                return Ok(FixMessages::none());
            }
            return Ok(FixMessages::from_result(
                self.bridge_with(held, extras).map(FixMessages::one),
            ));
        }
        // A frame opens behind the row, so the row is one message and the
        // frame the next.
        let stamp = super::build::RowStamp::retained(extras);
        Ok(FixMessages::frames(self.clone(), entries, 0, stamp))
    }

    /// The message opening at `at` among a row's entries, and where the
    /// row's next one opens.
    ///
    /// The one step the lazy source takes: a run opening on a tag is a frame,
    /// bounded by [`frame_end`]; a run opening on a name is the bridge row in
    /// front of the next frame. Every message of the row is built over the
    /// one page behind those entries and owns only its own ranges, so each
    /// re-emits its own bytes.
    pub(super) fn message_at(
        &self,
        entries: &TextEntries,
        at: usize,
        stamp: Option<&Arc<super::build::RowStamp>>,
    ) -> Option<RowMessage> {
        let held = entries.as_slice();
        let mut at = at;
        loop {
            let run = held.get(at..).filter(|run| !run.is_empty())?;
            let framed = tag_keyed(&run[0]);
            let end = if framed {
                at + frame_end(run)
            } else {
                next_frame(held, at).unwrap_or(held.len())
            };
            let next = if framed {
                next_frame(held, end).unwrap_or(held.len())
            } else {
                end
            };
            // The type the run states, read off the pairs: a type this codec
            // refuses costs one look rather than a build, an enrichment and a
            // settling, and a row of several frames still answers the ones it
            // does read.
            if !self.reads_run(&held[at..end]) {
                at = next;
                continue;
            }
            let fills = stamp.map(|stamp| stamp.fills()).unwrap_or_default();
            let extras = super::build::RowStamp::held(stamp, &fills);
            let message = if framed {
                self.frame_with(&held[at..end], extras)
            } else {
                self.bridge_with(&held[at..end], extras)
            };
            return Some(RowMessage { message, next });
        }
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
        let entries = self.entries(&page).0.unwrap_or_default();
        let framed = bounded(&page, &entries);
        if let Some(at) = second_frame_at(framed) {
            return Err(second_frame("fix", at));
        }
        let end = frame_end(framed);
        self.frame_with(&framed[..end], self.read_extras(body))
    }

    /// One numeric frame, from the entries the line answered for it.
    fn frame_with(&self, entries: &[TextEntry], extras: RowExtras<'_>) -> Result<FixMsg> {
        // A frame that marked no key judges none, and every key of it is
        // then its own: asking for the judgement would build a vector of the
        // row's whole width only to hand each key straight back, and the
        // `filter_map` that read it would drop the width the collect could
        // have reserved from. A wire frame marks nothing, so this is the
        // shape every captured line takes, and it is walked straight into
        // its pairs; an arrival is a subset of the entries, so a frame whose
        // entries mark nothing holds no marked arrival either.
        let mut conflicts = Vec::new();
        let (pairs, nested) = if entries.iter().any(TextEntry::marked) {
            let (arrived, nested) = frame_arrivals(entries, |held| held, Arrived::value);
            let pairs: Vec<FixPair> = if arrived.iter().any(|held| held.marked) {
                self.judged_keys(&arrived, &mut conflicts)
                    .into_iter()
                    .zip(&arrived)
                    .filter_map(|(judged, held)| {
                        judged.map(|key| key.pair(held.value.as_ref().clone()))
                    })
                    .collect()
            } else {
                arrived
                    .iter()
                    .map(|held| FixPair::own(held.key.clone(), held.value.as_ref().clone()))
                    .collect()
            };
            (pairs, nested)
        } else {
            frame_arrivals(
                entries,
                |held| FixPair::own(held.key.clone(), held.value.into_owned()),
                FixPair::value,
            )
        };
        let (stated, message) = self.declared_of(&pairs);
        self.build(
            &pairs,
            stated.as_deref(),
            message,
            &nested,
            extras,
            conflicts,
        )
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
    /// | `ORDERID=123\|#ORDERID=123` | `OrderID` 123 once: the marked pair restates the bare one and is dropped |
    /// | `ORDERID=123\|#ORDERID=345` | `OrderID` 123: the bare pair is the wire's word and the marked restatement is dropped |
    /// | `NOPARTYIDS=2\|NOPARTYIDS[0]=…\|NOPARTYIDS[1]=…\|#NOPARTYIDS=6\|#NOPARTYIDS[0]=…` | one `Parties` group: the bare occurrences, then the marked ones numbered past them; an occurrence restating another is dropped and the group counts what is left, sorted |
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
        let entries = self.entries(&page).0.unwrap_or_default();
        // Judged over the row's own entries rather than the bounded run: a
        // frame behind the row is where the scanner would have opened the
        // payload, so bounding first would read the frame and refuse
        // nothing.
        if let Some(at) = second_frame_at(entries.as_slice()) {
            return Err(second_frame("ullink", at));
        }
        self.bridge_with(bounded(&page, &entries), self.read_extras(body))
    }

    /// [`Self::parse_ullink_line`], with what the row stated beside its row.
    fn bridge_with(&self, entries: &[TextEntry], extras: RowExtras<'_>) -> Result<FixMsg> {
        let arrived = arrivals(entries);
        // The row's type was read while its groups were split under it.
        let (stated, message, pairs, conflicts) = self.bridge_pairs(&arrived);
        self.build(&pairs, stated.as_deref(), message, &[], extras, conflicts)
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
    /// needs the same answer to resolve the row's own spellings.
    ///
    /// A packed occurrence is unpacked into the members that fill the row, and
    /// the pair the bridge wrote is what the arrival record keeps: a key like
    /// `NOPARTYIDS[0].PARTYID` appears nowhere in the line, so it names a
    /// reading and never an arrival.
    fn bridge_pairs<'registry>(&'registry self, arrived: &[Arrived<'_>]) -> BridgeRow<'registry> {
        // Every `#` key is judged before the row's type is read, so a type
        // the bridge marked names the message exactly as a bare one does,
        // and one kept verbatim beside a bare type does not. A row with no
        // `#` at all judges nothing.
        let mut conflicts = Vec::new();
        // Sized to the row once: a judged key is at most one per arrival,
        // and a filtered zip states no count of its own.
        let mut kept: Vec<(Judged, &Arrived<'_>)> = Vec::with_capacity(arrived.len());
        kept.extend(
            self.judged_keys(arrived, &mut conflicts)
                .into_iter()
                .zip(arrived)
                .filter_map(|(judged, held)| judged.map(|key| (key, held))),
        );
        let mut resolved: Vec<FixPair> = Vec::with_capacity(kept.len());
        let msgtype = msgtype_of(
            kept.iter()
                .map(|(key, held)| (key.as_bytes(), held.value())),
        );
        let message: Declared<'registry> = msgtype
            .as_deref()
            .and_then(|code| self.declared_message(code));
        // The judged key is the one the pair is built under, so it moves
        // into the pair rather than being counted once more on the way.
        for (key, held) in kept {
            match group_index(key.as_bytes()) {
                Some((group, occurrence)) if memchr::memchr(b'=', held.value()).is_some() => {
                    let declared = self.group_members(group, message);
                    let mut path = Vec::with_capacity(group.len() + 8);
                    path.extend_from_slice(group);
                    path.extend_from_slice(b"[");
                    // Rendered in place: a byte vector never refuses a write.
                    let _ = std::io::Write::write_fmt(&mut path, format_args!("{occurrence}"));
                    path.extend_from_slice(b"]");
                    let segments = members(&held.value, declared, 0);
                    // The bridge closed what it packed inside this occurrence
                    // where it wrote a close anywhere but at the run's own
                    // end; a run carrying none is bounded by the dictionary.
                    let explicit = segments
                        .iter()
                        .rev()
                        .skip(1)
                        .any(|segment| matches!(segment, Segment::Close));
                    let opened = resolved.len();
                    self.render_members(&path, &segments, message, explicit, 0, &mut resolved);
                    // The first member read out of the occurrence carries the
                    // record of the pair the bridge actually wrote; the rest
                    // are that same arrival, read further.
                    if let Some(first) = resolved.get_mut(opened) {
                        first.reads();
                    }
                }
                _ => resolved.push(key.pair(held.value.as_ref().clone())),
            }
        }
        (msgtype, message, resolved, conflicts)
    }

    /// Every arriving key with its `#` judged: the key to build under, or
    /// `None` for a marked pair that restates a bare one.
    ///
    /// Each `#` key is judged against the row's bare spellings, gathered
    /// once: a bridge row is mostly `#` keys, so the probed list stays short.
    /// A row with no `#` at all judges nothing.
    ///
    /// A marked twin stating something else than its bare key is dropped all
    /// the same - the bare key always stands - and kept in `conflicts`.
    fn judged_keys(
        &self,
        arrived: &[Arrived<'_>],
        conflicts: &mut Vec<super::FixAnomaly>,
    ) -> Vec<Option<Judged>> {
        // A row that marked nothing judges nothing. The twin probe is one pass
        // over the row's bare spellings per marked key, so a wide frame that
        // marked none would otherwise pay a quadratic walk to learn that.
        if !arrived.iter().any(|held| held.marked) {
            return arrived
                .iter()
                .map(|held| Some(Judged::Ranged(held.key.clone())))
                .collect();
        }
        let bare = self.bare_spellings(arrived);
        arrived
            .iter()
            .map(|held| self.hashed_key(held, arrived, &bare, conflicts))
            .collect()
    }

    /// The row's bare spellings: every pair the line did not mark whose value
    /// is not a stated absence, each under the digest of its stem.
    fn bare_spellings<'row>(&self, arrived: &'row [Arrived<'_>]) -> Vec<Twin<'row>> {
        // Sized to the row once: a filter states no count of its own.
        let mut bare = Vec::with_capacity(arrived.len());
        bare.extend(
            arrived
                .iter()
                .filter(|held| !held.marked && !self.is_absent(held.value()))
                .map(|held| twin(held.key(), held.value())),
        );
        bare
    }

    /// The key one arriving pair builds under, its `#` judged; `None` for a
    /// marked pair that restates a bare one.
    ///
    /// A twin that itself opens with `#` - a `##` key's bare - is not among
    /// the bare spellings, so that one probe falls back to the whole row, read
    /// under the keys the line wrote rather than the ones it was stripped to.
    fn hashed_key(
        &self,
        held: &Arrived<'_>,
        arrived: &[Arrived<'_>],
        bare: &[Twin<'_>],
        conflicts: &mut Vec<super::FixAnomaly>,
    ) -> Option<Judged> {
        if !held.marked {
            return Some(Judged::Ranged(held.key.clone()));
        }
        let stripped = line::trim_ascii(held.key());
        let judged = if stripped.first() == Some(&b'#') {
            let written: Vec<(TextBytes, TextBytes)> = arrived
                .iter()
                .filter(|held| !self.is_absent(held.value()))
                .map(|held| (held.written(), held.value.as_ref().clone()))
                .collect();
            let row: Vec<Twin<'_>> = written
                .iter()
                .map(|(key, value)| twin(key.as_bytes(), value.as_bytes()))
                .collect();
            judge_hashed(stripped, &row)
        } else {
            judge_hashed(stripped, bare)
        };
        match judged {
            Hashed::Duplicate => {
                let marked = line::trim_ascii(held.value());
                let digest = stem_digest(stem_of(stripped));
                // A marked counter counts the marked occurrences, which are
                // numbered past the bare ones, so it says nothing against the
                // bare count.
                let counts_marked = arrived.iter().any(|other| {
                    other.marked
                        && group_index(line::trim_ascii(other.key()))
                            .is_some_and(|(group, _)| folds_twin(group, stripped))
                });
                if counts_marked {
                    return None;
                }
                if let Some((_, key, value)) = bare.iter().find(|(held_digest, key, _)| {
                    *held_digest == digest && folds_twin(key, stripped)
                }) {
                    let value = line::trim_ascii(value);
                    if value != marked {
                        conflicts.push(super::FixAnomaly::new(
                            super::build::folded_key(key),
                            format!(
                                "a #-marked twin states {:?} where the bare key states {:?}",
                                String::from_utf8_lossy(marked),
                                String::from_utf8_lossy(value)
                            ),
                        ));
                    }
                }
                None
            }
            Hashed::Bare => Some(Judged::Ranged(held.key.clone())),
            Hashed::Reindexed(offset) => {
                let (stem, index) = group_index(stripped)?;
                let mut key = Vec::with_capacity(stripped.len() + 4);
                key.extend_from_slice(stem);
                key.extend_from_slice(b"[");
                // Rendered in place: a byte vector never refuses a write.
                let _ = std::io::Write::write_fmt(&mut key, format_args!("{}", offset + index));
                key.extend_from_slice(b"]");
                Some(Judged::Rendered(key))
            }
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
        message: Declared<'registry>,
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
                    let sub_declared = self.group_members(sub, message);
                    let mut sub_path = rendered(sub);
                    sub_path.extend_from_slice(b"[");
                    // Rendered in place: a byte vector never refuses a write.
                    let _ = std::io::Write::write_fmt(&mut sub_path, format_args!("{index}"));
                    sub_path.extend_from_slice(b"]");
                    // What the sub-occurrence packed into its own value,
                    // then every following segment up to where it ends.
                    let end = self.extent(segments, at, sub_declared, message, explicit);
                    let mut nested = members(held, sub_declared, end - at);
                    nested.extend_from_slice(&segments[at..end]);
                    self.render_members(&sub_path, &nested, message, explicit, depth + 1, out);
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
        message: Declared<'_>,
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
                            if !self.declares_group(declared, sub, message) {
                                break;
                            }
                            let sub_declared = self.group_members(sub, message);
                            at = self.extent(segments, at + 1, sub_declared, message, false);
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
    fn declares_group(&self, declared: &[Field], sub: &[u8], message: Declared<'_>) -> bool {
        let Some(group) = self.group_definition(sub, message) else {
            return false;
        };
        let counter = group.as_fix().counter().ok().flatten();
        declared.iter().any(|field| {
            crate::folds_equal(field.name(), group.name())
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
        if let Some(at) = second_root(body)? {
            return Err(second_frame("fixml", at));
        }
        self.fixml_with(&fixml_pairs(body)?, self.read_extras(body))
    }

    /// Materializes attributes already parsed from one XML document.
    /// Syntax stays outside the message result, as it does for lazy frames.
    fn fixml_with(
        &self,
        attributes: &[(Vec<u8>, Vec<u8>)],
        extras: RowExtras<'_>,
    ) -> Result<FixMsg> {
        let pairs = own_pairs(
            attributes
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice())),
        )?;
        let (stated, message) = self.declared_of(&pairs);
        self.build(&pairs, stated.as_deref(), message, &[], extras, Vec::new())
    }

    /// Chains a finite capture of messages: the lifecycle.
    ///
    /// The capture is collected and stably sorted by event time before the
    /// one walk,
    /// [`EventIterator`](crate::graph::EventIterator), states each message
    /// as the one after the live message it follows - the last message of
    /// its chain, under the cross identity its cross code derives, still
    /// alive - so a chained message carries its predecessor's identity and
    /// instant, its place in the chain, the predecessor as a parent and the
    /// lifecycle carried forward, and is settled again around them.
    /// [`Self::lifecycle_arrow_reader`] is the same walk over batches of
    /// rows. Intake errors are retained in source order and yielded before
    /// the sorted messages; they never advance the walk, and exhaustion is
    /// fused.
    ///
    /// Two observations with the same complete nonempty capture
    /// `(msgtype, msgsessionid, msgctxid, msgseqnum)` - one
    /// [`FixCapture::msgsesseventid`](super::FixCapture::msgsesseventid) -
    /// are fully merged before the walk rather than stated as successive
    /// events. The observation with the latest `recdunix` is the reference
    /// message, a stated one leading an unstated one and the later `currunix`
    /// closing a tie; it is chosen once over every observation of the event,
    /// because a merge keeps the earliest recording either side knows and
    /// would rank what it folded by that. The graph merge unions the other
    /// observations into it, while `execunix` and `recdunix` retain the
    /// earliest precise facts, and the reference source leads provenance
    /// order. An incomplete key proves no equivalence.
    ///
    /// Exact republications and flagged FIX retransmissions are removed by a
    /// delivery set over session, sequence, original time and the recorded
    /// canonical content code. That code survives a semantic row round trip,
    /// so this walk and [`Self::lifecycle_arrow_reader`] remove the same
    /// deliveries. A row without a complete session header keeps the stricter
    /// event identity, capture context, direction and sequence in its key. The
    /// set is bounded by the number of distinct deliveries in the finite
    /// capture. Distinct deliveries with equal business content remain
    /// distinct. Missing instrument codes may be learned from earlier messages
    /// of this lifecycle only, after sorting, and never overwrite a stated
    /// fact. Finite expirations emit at their exact deadline. Where
    /// [`Self::snapshot_ns`] is set, separate owned views of every living
    /// identity are emitted on that epoch-aligned grid without advancing its
    /// chain.
    ///
    /// The walk reads the structured message before it walks: a message
    /// whose sending clock the parse supplied rather than read is dated by
    /// the `TransactTime(60)` it states, [`FixMsg::dated_by_transaction`],
    /// so a capture whose frames state no `SendingTime(52)` still orders,
    /// expires and folds by when its transactions happened.
    ///
    /// A message whose type this codec refuses never enters the walk: a
    /// keepalive belongs to the session rather than to a chain, and a
    /// message handed here from somewhere other than this codec's own parse
    /// is held to the same reading. The refusal is by
    /// [`Self::reads_msgtype`], and a codec refusing nothing walks
    /// everything it is given.
    pub fn lifecycle<I>(&self, messages: I) -> impl Iterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let codec = self.clone();
        let walked = messages
                .into_iter()
                .fuse()
                .map(Into::into)
                .filter(move |held: &Result<FixMsg>| match held {
                    // A failure is never filtered: what a stream could not read
                    // has no type to refuse it by, and swallowing it here would
                    // lose the one report of it.
                    Err(_) => true,
                    Ok(message) => codec.reads_msgtype(message.header().msgtype()),
                })
                // The walk reads the structured message: a frame the parse
                // dated by a stand-in clock is dated by its transaction.
                .map(|held: Result<FixMsg>| held.and_then(FixMsg::dated_by_transaction));
        super::enrich::Walked::new(walked, self.snapshot_ns)
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
        let pairs = own_pairs(pairs)?;
        // The pairs have no line behind them, so the byte a refusal names is
        // the offset in the page they were copied into, which is where the
        // arrival record holds the key.
        if let Some(at) = second_pair_frame(&pairs) {
            return Err(second_frame("fix", at));
        }
        let (stated, message) = self.declared_of(&pairs);
        self.build(
            &pairs,
            stated.as_deref(),
            message,
            &[],
            RowExtras::NONE,
            Vec::new(),
        )
    }

    /// The build a reader outside this module funnels into.
    ///
    /// The pairs are bytes a reader holds rather than ranges of a line - a
    /// configuration document's fields, or no pairs at all - so they are
    /// copied into one page here, which is what a message owning its own
    /// arrival record costs when nothing owned it already. A pair may name
    /// the dictionary field it fills beside the key it arrived under, where
    /// the document spells a field otherwise than the dictionary does: the
    /// entry keeps the arrival, the child takes the name.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub(super) fn build_pairs_with(
        &self,
        pairs: &[SpelledPair<'_>],
        extras: RowExtras<'_>,
    ) -> Result<FixMsg> {
        let pairs = spelled_pairs(pairs.iter().copied())?;
        let (stated, message) = self.declared_of(&pairs);
        self.build(&pairs, stated.as_deref(), message, &[], extras, Vec::new())
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
        stated: Option<&str>,
        message: Declared<'_>,
        nested: &[TextBytes],
        extras: RowExtras<'_>,
        conflicts: Vec<super::FixAnomaly>,
    ) -> Result<FixMsg> {
        // The version is the row's own, else what the line implies. A row
        // states one where the transport knew it and the frame did not, which
        // is what a bridge log carries in its `beginstring` capture.
        let stated_version = extras.version;
        let version = stated_version.or_else(|| self.infer_version(pairs));
        let mut builder = Builder::new(
            &self.registry,
            message,
            &self.beginstring,
            self.registry.memo(),
            version,
            pairs.len(),
        );
        builder.note(conflicts);
        // A stated absence produces no field and no entry: the key is read
        // as never having been sent. Filtering happens before typing, so
        // nothing tries to read `<null>` as a price and file the failure.
        builder.push_pairs(pairs, |_, value| self.is_absent(value));
        for row in nested {
            self.push_nested(&mut builder, row, stated_version);
        }
        for fill in extras.fills {
            if fill.tag != 52 {
                builder.fill(fill);
            }
        }
        // The carrier clock is a fallback, applied after the row's explicit
        // cells and after the message itself: either can state `recdunix`
        // directly, and the builder never overwrites a stated value.
        if let Some(unix) = extras.recdunix {
            let field = self
                .registry
                .get_field_by_tag(super::RECDUNIX_TAG_NAME.0)
                .ok_or_else(|| Error::absent("FIX crate field", super::RECDUNIX_TAG_NAME.0))?;
            let value =
                Scalar::datetime64(unix, crate::TimeUnit::Nanosecond, crate::Timezone::UTC)?;
            builder.fill(&Fill {
                field,
                tag: super::RECDUNIX_TAG_NAME.0,
                value: &value,
            });
        }
        // Where the line's prose says the message came from, as fills: a
        // `CONVERSATIONID` the message stated stands.
        for (tag, text) in [
            (super::MSGORIGINATOR_TAG_NAME.0, extras.originator),
            (super::CONVERSATIONID_TAG_NAME.0, extras.conversation),
        ] {
            let Some(text) = text else {
                continue;
            };
            let field = self
                .registry
                .get_field_by_tag(tag)
                .ok_or_else(|| Error::absent("FIX crate field", tag))?;
            let value = Scalar::from(text);
            builder.fill(&Fill {
                field,
                tag,
                value: &value,
            });
        }
        // Tag 385 as a built child, where the line stated none of its own:
        // a fill, so a `385=` on the wire or a stated column stands.
        if let Some(code) = extras.direction {
            let value = Scalar::from(code);
            builder.fill(&Fill {
                field: self.msgdirection.field(),
                tag: super::MSGDIRECTION_TAG_NAME.0,
                value: &value,
            });
        }
        // The root is named for the message type the row states once every
        // nested row has been read: a frame carrying a message in its data
        // field is named for that message.
        let name = builder
            .stated_msgtype()
            .or_else(|| stated.map(SmolStr::new));
        self.finish(builder.finish(root_name(name.as_deref()).as_str())?, extras)
    }

    fn finish(&self, mut built: super::build::Built, extras: RowExtras<'_>) -> Result<FixMsg> {
        // What the row stated under a namespace of its own is what the row
        // stated, so it is read here rather than left to the enriching pass:
        // a child the dictionary does not name has no column, so one read
        // any later would be invisible to the batch door.
        built.compose(&self.registry)?;
        // A conversation the message states outranks the one its line names,
        // and the two disagreeing is kept beside the message.
        if let Some(named) = extras.conversation {
            let held = built
                .index_of_tag(super::CONVERSATIONID_TAG_NAME.0, &self.registry)
                .and_then(|at| built.value.get(at))
                .and_then(|value| value.as_str().map(str::to_owned));
            if let Some(held) = held.filter(|held| held != named) {
                built.anomalies.push(super::FixAnomaly::new(
                    super::CONVERSATIONID_TAG_NAME.1,
                    format!("states {held} where its line names {named}"),
                ));
            }
        }
        let stated = built
            .index_of_tag(52, &self.registry)
            .and_then(|at| built.value.get(at))
            .is_some_and(|value| !value.is_null());
        // A message stating no sending clock is dated by its carrier: a row
        // cell reaching `SendingTime(52)`, else the line's own `currunix` -
        // the instant the line was recorded at, which is nearer the send
        // than any pin - and only then by the codec's default or now.
        let carrier = if stated {
            None
        } else {
            let filled = extras
                .fills
                .iter()
                .find(|fill| fill.tag == 52 && !fill.value.is_null())
                .map(|fill| {
                    super::build::typed_fill(&self.registry, fill.field, fill.tag, fill.value)
                })
                .transpose()?
                .filter(|value| !value.is_null());
            // The epoch is silence here as on the batch door, where a line
            // nothing dated reads as the epoch: an undated carrier dates no
            // message, on either door.
            match (filled, extras.recdunix.filter(|unix| *unix != 0)) {
                (Some(value), _) => Some(value),
                (None, Some(unix)) => Some(Scalar::datetime64(
                    unix,
                    crate::TimeUnit::Nanosecond,
                    crate::Timezone::UTC,
                )?),
                (None, None) => None,
            }
        };
        let message = FixMsg::from_built(
            Arc::clone(&self.registry),
            built,
            carrier.as_ref().or(self.default_sending_time.as_ref()),
            extras.source,
            self.official_time_delay_ns(),
        )?;
        super::enrich::enrich(&self.registry, message)
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
    /// its own, so the inference lands on the crate's own default, which is
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
                .and_then(|code| self.declared_message(code));
            self.nest(builder, declared, &held, pinned);
            return;
        }
        // The value's own entries, read in their own scope: what a bridge
        // wrote inside a data field is judged against that row's spellings
        // and not against the frame's.
        let entries = self.entries(row).0.unwrap_or_default();
        let arrived = arrivals(entries.as_slice());
        // A twin conflict inside a data field is that field's reading, and
        // the field's own value stands whole on the arrival record.
        let (_, declared, held, _) = self.bridge_pairs(&arrived);
        self.nest(builder, declared, &held, pinned);
    }

    /// One nested row's pairs, bracketed as the nested reading they are.
    fn nest<'registry>(
        &'registry self,
        builder: &mut Builder<'registry>,
        declared: Declared<'registry>,
        pairs: &[FixPair],
        pinned: Option<Version>,
    ) {
        let dated = pinned.or_else(|| self.infer_version(pairs));
        builder.begin_nested(declared, dated);
        for pair in pairs {
            if self.is_absent(pair.value()) {
                continue;
            }
            builder.push(pair.key(), pair.value());
        }
        builder.end_nested();
    }

    /// The message definition a row's type names, which is what says which
    /// group a shared counter - `NoLegs`, `NoSides` - heads in it.
    /// What the payload spelled, read off its pairs once: the code decides
    /// the message and names the root, and a payload spelling none is named
    /// `unknown`.
    fn declared_of(&self, pairs: &[FixPair]) -> (Option<SmolStr>, Declared<'_>) {
        let stated = msgtype_of(pairs.iter().map(|pair| (pair.key(), pair.value())));
        let message = stated
            .as_deref()
            .and_then(|code| self.declared_message(code));
        (stated, message)
    }

    fn declared_message(&self, code: &str) -> Option<&super::MsgType> {
        self.registry.get_msgtype(code)
    }

    /// The direct members the addressed repeating group declares.
    ///
    /// Bridge keys are rendered names even when their bytes are digits. Only
    /// a nested field reached by that name can declare boundaries; an
    /// unresolved group leaves its value whole.
    fn group_members<'registry>(
        &'registry self,
        group: &[u8],
        message: Declared<'registry>,
    ) -> &'registry [Field] {
        let Some(field) = self.group_definition(group, message) else {
            return &[];
        };
        super::schema::item_fields(field).unwrap_or_default()
    }

    /// The repeating group a bridge key addresses, as the dictionary declares
    /// it: by the group's own name, else by the counter's, under `message`
    /// where that counter is shared.
    fn group_definition<'registry>(
        &'registry self,
        group: &[u8],
        message: Declared<'registry>,
    ) -> Option<&'registry Field> {
        let Ok(group) = std::str::from_utf8(group) else {
            return None;
        };
        let found = self
            .registry
            .get_definition(crate::FixCategory::Groups, group)
            .or_else(|| {
                let counter = if let Some(tag) = super::field::parse_tag(group) {
                    self.registry.get_field_by_tag(tag)
                } else {
                    self.registry.get_field_by_name(group)
                }?;
                let (tag, _) = self.registry.identity_of(counter)?;
                match message.filter(|message| message.has_group_tag(tag)) {
                    Some(message) => message.get_group_by_tag(tag),
                    None => self.registry.get_group_by_tag(tag),
                }
            });
        found.filter(|field| field.dtype().is_nested())
    }

    /// The version an arriving row is written in, when the caller pinned none.
    ///
    /// Each step is a FIX rule rather than a heuristic. `ApplVerID(1128)`
    /// wins, because under FIXT.1.1 the session version says nothing about
    /// the application version. `BeginString(8)` follows, and `FIXT.1.1`
    /// falls through rather than being taken literally. A line that states
    /// neither is undated: the dictionary is version-blind and has no version
    /// of its own to lend, and a caller reading an undated row gets the
    /// crate's stated default rather than a guess dressed as a fact.
    fn infer_version(&self, pairs: &[FixPair]) -> Option<Version> {
        if let Some(value) = value_of(pairs, b"1128") {
            if let Some(version) = appl_ver_id(&String::from_utf8_lossy(value)) {
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
        None
    }
}

/// `ApplVerID`'s numeric and symbolic spellings.
fn appl_ver_id(value: &str) -> Option<Version> {
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
        // "FIX Latest" is a moving label rather than a version. A
        // version-blind dictionary states none to resolve it against, so it
        // names nothing here and the next rule answers.
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
fn msgtype_of<'a>(pairs: impl IntoIterator<Item = (&'a [u8], &'a [u8])>) -> Option<SmolStr> {
    for (key, value) in pairs {
        // The key folds the way every other key folds, so `MSG_TYPE` and
        // `Msg Type` name the type too.
        let folded = std::str::from_utf8(key).is_ok_and(|key| crate::folds_equal(key, "MsgType"));
        if folded || key == b"35" {
            return Some(SmolStr::new(String::from_utf8_lossy(value)));
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
        .position(|entry| entry.key_bytes().start() as usize >= opens)
        .unwrap_or(entries.len());
    &entries[at..]
}

/// What a byte door answers a body holding a second message.
///
/// The doors take one frame; a caller holding two holds a row, and a row is
/// what [`FixCodec::parse_line`] reads. `Error::Parse` prints
/// the byte itself, so the reason names what was expected and nothing else.
fn second_frame(target: &'static str, position: usize) -> Error {
    Error::Parse {
        target,
        position,
        reason: crate::text::expected_got("one frame", "a second"),
    }
}

/// The byte a second frame opens at among one body's entries, where it holds
/// one.
///
/// The first message is bounded the way the door reads it - a run opening on
/// a tag is a frame, a run opening on a name is a bridge row - and anything
/// opening a frame behind it is the second.
fn second_frame_at(entries: &[TextEntry]) -> Option<usize> {
    let from = if entries.first().is_some_and(tag_keyed) {
        frame_end(entries)
    } else {
        0
    };
    let second = next_frame(entries, from)?;
    Some(entries[second].key_bytes().start() as usize)
}

/// One message of a row, and where the row's next one opens.
pub(super) struct RowMessage {
    /// The message, or the builder's refusal, which fuses the row.
    pub(super) message: Result<FixMsg>,
    /// The entry the next message opens at; the run's end where the row
    /// holds no more.
    pub(super) next: usize,
}

/// Where the frame opening at the first of these entries ends.
///
/// One past its checksum, which is what closes a frame; else where the next
/// frame opens, because a frame stating no checksum ends where the next one
/// begins; else the run's end. Only an unmarked `8=` closes an
/// open frame: a frame's own `35=` stands behind its `8=`, so a second one
/// inside it is a duplicate tag and not a new message.
fn frame_end(entries: &[TextEntry]) -> usize {
    for (at, entry) in entries.iter().enumerate() {
        if !tag_keyed(entry) {
            continue;
        }
        match entry.key_bytes().as_bytes() {
            b"10" => return at + 1,
            b"8" if at > 0 => return at,
            _ => {}
        }
    }
    entries.len()
}

/// Where the next frame opens at or after `from`, where the run holds one.
///
/// The rule the scanner locates a line's first frame by, read over the row's
/// own entries so a key the bridge marked is the bridge's own: an unmarked
/// `8=`, and where the rest states none, an unmarked `35=`.
fn next_frame(entries: &[TextEntry], from: usize) -> Option<usize> {
    let rest = entries.get(from..)?;
    let opens = |key: &[u8]| {
        rest.iter()
            .position(|entry| tag_keyed(entry) && entry.key_bytes().as_bytes() == key)
    };
    opens(b"8").or_else(|| opens(b"35")).map(|at| from + at)
}

/// Whether one entry's key is a tag rather than a name, which is what says a
/// frame is numeric FIX rather than a bridge row.
///
/// A key the line marked is a bridge's own spelling and never a tag, even
/// where the bytes after the mark are digits: `#453=1` is a bridge naming the
/// group by its counter, not a frame stating tag 453.
/// The message type one run of pairs states, read off the pairs rather than
/// off a message: `35` in a FIX frame, a key folding to `MsgType` in a bridge
/// row. `None` where the run states none, which is the row read as `unknown`.
fn stated_type(run: &[TextEntry]) -> Option<SmolStr> {
    msgtype_of(
        run.iter()
            .map(|entry| (entry.key_bytes().as_bytes(), entry.value_bytes().as_bytes())),
    )
}

fn tag_keyed(entry: &TextEntry) -> bool {
    !entry.marked() && entry.key_bytes().as_bytes().iter().all(u8::is_ascii_digit)
}

/// One entry as the pair it arrived as.
fn arrival(entry: &TextEntry) -> Arrived<'_> {
    Arrived {
        key: entry.key_bytes(),
        value: Cow::Borrowed(entry.value_bytes()),
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
///
/// Each arrival is emitted as the caller keeps it - the arrival itself where
/// a mark is still to be judged, the pair it builds where none is - and
/// `stated` reads back the value the arrival before a data field stated, so
/// one walk cuts the frame whatever it is cut into.
fn frame_arrivals<'entry, T>(
    entries: &'entry [TextEntry],
    emit: impl Fn(Arrived<'entry>) -> T,
    stated: impl Fn(&T) -> &[u8],
) -> (Vec<T>, Vec<TextBytes>) {
    let mut arrived: Vec<T> = Vec::with_capacity(entries.len());
    // The data values that are bridge rows, read after the frame's own pairs
    // so the frame's statements come first.
    let mut nested: Vec<TextBytes> = Vec::new();
    let mut at = 0;
    while let Some(entry) = entries.get(at) {
        at += 1;
        let mut held = arrival(entry);
        if let Some(tag) = data_tag(held.key()) {
            let xml = tag == XML_DATA_TAG;
            if let Some(end) = data_end(entry, arrived.last().map(&stated), entries, at, xml) {
                if let Some(widened) = entry.key_bytes().page().and_then(|page| {
                    TextBytes::from_page(page, entry.key_bytes().end() as usize + 1, end).ok()
                }) {
                    held.value = Cow::Owned(widened);
                    while entries
                        .get(at)
                        .is_some_and(|swallowed| (swallowed.key_bytes().start() as usize) < end)
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
        arrived.push(emit(held));
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
/// The byte a second frame opens at among pairs a caller handed in.
///
/// A second `8=`, or a `35=` behind a checksum: the openers a line's own
/// frames are found by, over pairs that never were a line.
fn second_pair_frame(pairs: &[FixPair]) -> Option<usize> {
    let mut at = 0;
    let mut opened = false;
    let mut closed = false;
    for pair in pairs {
        let key = pair.key();
        if !key.is_empty() && key.iter().all(u8::is_ascii_digit) {
            match key {
                b"8" if opened => return Some(at),
                b"35" if closed => return Some(at),
                b"10" => closed = true,
                _ => {}
            }
            opened = true;
        }
        at += key.len() + pair.value().len();
    }
    None
}

/// The byte a second root element opens at, where the document holds one.
///
/// A FIXML body states one document; two roots are two messages, which is a
/// row rather than the one frame this door takes.
///
/// # Errors
///
/// Returns [`Error::Parse`] naming the byte position when the body is not
/// well-formed XML, exactly as reading it does.
fn second_root(body: &[u8]) -> Result<Option<usize>> {
    let mut reader = quick_xml::Reader::from_reader(body);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut roots = 0_usize;
    loop {
        let opens = reader.buffer_position() as usize;
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| Error::Parse {
                target: "fixml",
                position: reader.buffer_position() as usize,
                reason: SmolStr::new(error.to_string()),
            })?;
        match event {
            Event::Eof => break,
            Event::Start(_) | Event::Empty(_) => {
                if depth == 0 {
                    roots += 1;
                    if roots > 1 {
                        return Ok(Some(opens));
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn own_pairs<'a>(pairs: impl IntoIterator<Item = (&'a [u8], &'a [u8])>) -> Result<Vec<FixPair>> {
    spelled_pairs(pairs.into_iter().map(|(key, value)| (key, value, None)))
}

/// [`own_pairs`], where a pair may also name the dictionary field it fills.
///
/// A pair naming one is recorded under the key it arrived with and built
/// under the name, so a row spelling an alternate name of a field keeps that
/// spelling in its arrival record and the field's own name in its row; a pair
/// naming none is its own record.
fn spelled_pairs<'a>(pairs: impl IntoIterator<Item = SpelledPair<'a>>) -> Result<Vec<FixPair>> {
    let mut page = Vec::new();
    let mut spans = Vec::new();
    for (key, value, name) in pairs {
        let opens = page.len();
        page.extend_from_slice(key);
        let split = page.len();
        page.extend_from_slice(value);
        spans.push((opens, split, page.len(), name.map(<[u8]>::to_vec)));
    }
    let page = TextBytes::from_whole_page(std::sync::Arc::new(page))?;
    spans
        .into_iter()
        .map(|(opens, split, ends, name)| {
            let key = page.slice(opens, split)?;
            let value = page.slice(split, ends)?;
            Ok(match name {
                Some(name) => FixPair::spelled(name, value),
                None => FixPair::own(key, value),
            })
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
        crate::folds_equal(field.name(), key)
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
/// stays out. `spare` is room for the segments a caller appends behind them.
fn members(value: &TextBytes, declared: &[Field], spare: usize) -> Vec<Segment> {
    let parts = split_members(value.as_bytes(), declared);
    // Sized once: a segment is at most one per part, and a filtered collect
    // states no count of its own.
    let mut segments = Vec::with_capacity(parts.len() + spare);
    segments.extend(parts.into_iter().filter_map(|part| {
        if part.is_empty() {
            return Some(Segment::Close);
        }
        member_pair(value, part).map(|(key, value)| Segment::Pair(key, value))
    }));
    segments
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
    // One searcher for the whole run, built for the separator that won.
    for at in memchr::memmem::Finder::new(separator).find_iter(held) {
        parts.push(start..at);
        start = at + separator.len();
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
fn judge_hashed(stripped: &[u8], twins: &[Twin<'_>]) -> Hashed {
    let stem = stem_of(stripped);
    let digest = stem_digest(stem);
    let mut twinned = false;
    let mut occurrences = 0;
    for &(held_digest, held, _) in twins {
        // The digest says which twins can fold equal; the fold says which do.
        if held_digest != digest || !folds_twin(stem_of(held), stem) {
            continue;
        }
        twinned = true;
        if let Some((_, index)) = group_index(held) {
            occurrences = occurrences.max(index + 1);
        }
    }
    if !twinned {
        Hashed::Bare
    } else if group_index(stripped).is_some() {
        Hashed::Reindexed(occurrences)
    } else {
        Hashed::Duplicate
    }
}

/// The group an indexed key addresses, or the key itself.
fn stem_of(key: &[u8]) -> &[u8] {
    group_index(key).map_or(key, |(group, _)| group)
}

/// One spelling a `#` key may restate: the digest of its stem under the
/// fold [`folds_twin`] compares by, the key and the value.
type Twin<'row> = (u64, &'row [u8], &'row [u8]);

fn twin<'row>(key: &'row [u8], value: &'row [u8]) -> Twin<'row> {
    (stem_digest(stem_of(key)), key, value)
}

/// A digest of one stem under the fold [`folds_twin`] compares by: two
/// stems that fold equal digest equal, so a row of a hundred marked keys
/// probes its bare spellings by digest and folds only the ones that can
/// match.
fn stem_digest(stem: &[u8]) -> u64 {
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in stem {
        if name_separator(*byte) {
            continue;
        }
        digest ^= u64::from(byte.to_ascii_lowercase());
        digest = digest.wrapping_mul(0x0100_0000_01b3);
    }
    digest
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/codec.rs` pins and a caller cannot reach.
    //!
    //! The delay a caller states is milliseconds, and the nanosecond distance
    //! a dating actually compares is the step inside that statement: it is
    //! `pub(super)`, so it is forwarded here rather than published.
    use super::FixCodec;

    /// [`FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_MS`] as the nanosecond distance
    /// a dating compares.
    #[must_use]
    pub const fn default_official_time_delay_ns() -> i64 {
        FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_NS
    }

    /// One codec's delay as the nanosecond distance a dating compares.
    #[must_use]
    pub const fn official_time_delay_ns(codec: &FixCodec) -> i64 {
        codec.official_time_delay_ns()
    }
}
