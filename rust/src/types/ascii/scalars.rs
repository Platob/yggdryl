//! ASCII values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarFamily, ScalarValue, types};

/// Borrowing access shared by every ASCII representation.
pub trait AsciiValue: crate::ScalarValue {
    /// The fixed byte width, or `None` when the datatype carries it.
    const WIDTH: Option<i32>;

    /// Borrow the validated ASCII text.
    fn as_str(&self) -> &str;
    /// Borrow the shared storage behind the validated text.
    ///
    /// The stored text is already trimmed and checked, so a rewrite that
    /// keeps it clones this handle rather than re-validating and copying.
    fn storage(&self) -> &SmolStr;
}

/// Variable-width validated ASCII text.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Ascii(SmolStr);

impl Ascii {
    /// Validate and construct variable-width ASCII text.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_free_text(value.as_ref().as_bytes())?;
        Ok(Self(SmolStr::new(value)))
    }

    /// Borrow the validated text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the text.
    pub fn storage(&self) -> &SmolStr {
        &self.0
    }
}

impl fmt::Display for Ascii {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// ASCII text carrying its fixed padded storage width.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FixedAscii {
    value: SmolStr,
    width: i32,
}

impl FixedAscii {
    /// Validate text against one positive fixed width.
    pub fn new(value: impl AsRef<str>, width: i32) -> Result<Self> {
        let _ = crate::DataType::ascii(width)?;
        let value = types::ascii_text(width, value.as_ref().as_bytes())?;
        Ok(Self {
            value: SmolStr::new(value),
            width,
        })
    }

    /// Borrow the validated text.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    /// Borrow the shared storage without copying the text.
    pub fn storage(&self) -> &SmolStr {
        &self.value
    }

    /// Return the padded storage width.
    pub const fn width(&self) -> i32 {
        self.width
    }
}

impl fmt::Display for FixedAscii {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

macro_rules! ascii_code_leaf {
    ($name:ident, $width:expr) => {
        #[doc = concat!("One validated `", stringify!($name), "` code.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate and construct this registered code width.
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = types::ascii_text($width, value.as_ref().as_bytes())?;
                Ok(Self(SmolStr::new(value)))
            }

            /// Borrow the validated code.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Borrow the shared storage without copying the code.
            pub fn storage(&self) -> &SmolStr {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

ascii_code_leaf!(Country, 2);
ascii_code_leaf!(Currency, 3);
ascii_code_leaf!(Mic, 4);
ascii_code_leaf!(Cfi, 6);
ascii_code_leaf!(Side, 4);
ascii_code_leaf!(MsgType, 8);
ascii_code_leaf!(MsgDirection, 4);
ascii_code_leaf!(State, 10);
ascii_code_leaf!(TimeInForce, 8);

impl State {
    /// The rank a stored state opens with, first to terminal.
    ///
    /// The two leading digits read as the number they spell, `0` to `99`, or
    /// `None` where the value does not open with two digits. A sort of the
    /// raw column is already in this order, because the digits lead and are
    /// fixed at two.
    #[must_use]
    pub fn rank(&self) -> Option<u8> {
        match self.as_str().as_bytes() {
            [tens @ b'0'..=b'9', ones @ b'0'..=b'9', ..] => {
                Some((tens - b'0') * 10 + (ones - b'0'))
            }
            _ => None,
        }
    }

    /// Whether this state can still change.
    ///
    /// Every rank below `80`, the first terminal band. A reader asking "is
    /// this still going" asks this rather than listing names.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.rank().is_some_and(|rank| rank < 80)
    }

    /// Whether this state ended having done what was asked: rank `80`-`89`.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.rank(), Some(80..=89))
    }

    /// Whether this state ended because someone stopped it: rank `90`-`94`.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.rank(), Some(90..=94))
    }

    /// Whether this state ended because it could not be done: rank `95`-`99`.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self.rank(), Some(95..=99))
    }

    /// The state one spelling names, or `None` where none does.
    ///
    /// Four vocabularies reach one value, because they name one thing:
    ///
    /// - a FIX `OrdStatus(39)` or `ExecType(150)` wire code - `0`, `1`, `F`;
    /// - the specification's own name for it - `PartiallyFilled`, `DoneForDay`;
    /// - the word a scheduler uses - `running`, `succeeded`, `timed out`;
    /// - the short name a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `DoneForDay`,
    /// `done_for_day` and `DONE FOR DAY` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX and a
    /// folded lookup would answer the wrong state for one of them.
    ///
    /// ```
    /// use yggdryl::types::State;
    ///
    /// // The wire code, the specification's name and the scheduler's word.
    /// assert_eq!(State::from_spelling("1").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("running").unwrap().as_str(), "30RUNNING");
    ///
    /// // The stored bytes sort from the first state to the terminal ones,
    /// // which is the whole reason the rank leads.
    /// let mut held = ["80FILLED", "20NEW", "95REJECTED", "40PARTFILL"];
    /// held.sort_unstable();
    /// assert_eq!(held, ["20NEW", "40PARTFILL", "80FILLED", "95REJECTED"]);
    ///
    /// // And the three endings are told apart without reading the name.
    /// assert!(State::from_spelling("New").unwrap().is_live());
    /// assert!(State::from_spelling("Filled").unwrap().is_done());
    /// assert!(State::from_spelling("Rejected").unwrap().is_failed());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        // A stored value names itself, which is what makes reading one back
        // free and the whole mapping idempotent.
        if crate::types::AsciiEnum::STATES.contains(&spelling) {
            return Self::new(spelling).ok();
        }
        if let Some(held) = STATE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, state)| *state)
        {
            return Self::new(held).ok();
        }
        let folded = folded_spelling(spelling);
        STATE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, state)| *state)
            .and_then(|held| Self::new(held).ok())
    }
}

/// One spelling folded the way every name in this crate folds.
fn folded_spelling(spelling: &str) -> SmolStr {
    let mut held = String::with_capacity(spelling.len());
    for byte in spelling.bytes() {
        if matches!(byte, b'_' | b'-' | b' ') {
            continue;
        }
        held.push(char::from(byte.to_ascii_lowercase()));
    }
    SmolStr::new(held)
}

/// FIX's `OrdStatus(39)` and `ExecType(150)` wire codes, unfolded.
///
/// The two code sets agree on every value they share, which is why one table
/// answers both: `0` is New in each, `1` PartiallyFilled, `2` Filled. Where
/// only `ExecType` defines a value - `F` Trade, `L` Triggered - the state is
/// what that report says the order is doing.
static STATE_CODES: &[(&str, &str)] = &[
    ("0", "20NEW"),
    ("1", "40PARTFILL"),
    ("2", "80FILLED"),
    ("3", "80DONEDAY"),
    ("4", "90CANCELED"),
    ("5", "70REPLACED"),
    ("6", "60PENDCXL"),
    ("7", "50STOPPED"),
    ("8", "95REJECTED"),
    ("9", "50SUSPEND"),
    ("A", "10PENDNEW"),
    ("B", "80CALCULAT"),
    ("C", "95EXPIRED"),
    ("D", "20ACCEPTED"),
    ("E", "60PENDRPL"),
    ("F", "40TRADE"),
    ("G", "40TRDCORR"),
    ("H", "40TRDCXL"),
    ("I", "30STATUS"),
    ("J", "40TRDHOLD"),
    ("K", "80TRDRELS"),
    ("L", "30TRIGGER"),
];

/// Every name that reaches a state, folded: FIX's, a scheduler's, and the
/// short names a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`,
/// `Cancel`, `Reject`.
static STATE_NAMES: &[(&str, &str)] = &[
    ("accepted", "20ACCEPTED"),
    ("acceptedforbidding", "20ACCEPTED"),
    ("calculated", "80CALCULAT"),
    ("cancel", "90CANCELED"),
    ("canceled", "90CANCELED"),
    ("cancelled", "90CANCELED"),
    ("complete", "80COMPLETE"),
    ("completed", "80COMPLETE"),
    ("doneday", "80DONEDAY"),
    ("doneforday", "80DONEDAY"),
    ("expired", "95EXPIRED"),
    ("failed", "95FAILED"),
    ("failure", "95FAILED"),
    ("filled", "80FILLED"),
    ("inprogress", "40INPROGR"),
    ("new", "20NEW"),
    ("orderstatus", "30STATUS"),
    ("partfill", "40PARTFILL"),
    ("partfilled", "40PARTFILL"),
    ("partiallyfilled", "40PARTFILL"),
    ("paused", "50PAUSED"),
    ("pendcancel", "60PENDCXL"),
    ("pending", "10PENDING"),
    ("pendingcancel", "60PENDCXL"),
    ("pendingnew", "10PENDNEW"),
    ("pendingreplace", "60PENDRPL"),
    ("pendnew", "10PENDNEW"),
    ("pendreplace", "60PENDRPL"),
    ("queued", "10QUEUED"),
    ("reject", "95REJECTED"),
    ("rejected", "95REJECTED"),
    ("replaced", "70REPLACED"),
    ("restated", "70REPLACED"),
    ("running", "30RUNNING"),
    ("starting", "20STARTING"),
    ("stopped", "50STOPPED"),
    ("submitted", "20SUBMITTD"),
    ("succeeded", "80SUCCESS"),
    ("success", "80SUCCESS"),
    ("suspended", "50SUSPEND"),
    ("timedout", "95TIMEOUT"),
    ("timeout", "95TIMEOUT"),
    ("trade", "40TRADE"),
    ("tradecancel", "40TRDCXL"),
    ("tradecorrect", "40TRDCORR"),
    ("tradehasbeenreleasedtoclearing", "80TRDRELS"),
    ("tradeinaclearinghold", "40TRDHOLD"),
    ("triggeredoractivatedbysystem", "30TRIGGER"),
    ("unknown", "00UNKNOWN"),
];

impl MsgType {
    /// The alphabet a synthesized message type is rendered in.
    ///
    /// Digits and upper-case letters, minus the four that read as each other
    /// in a log line - `I`/`1`, `O`/`0` - because a synthetic value is read
    /// by people before it is read by anything else. Thirty-two symbols, so
    /// each carries exactly five bits and the rendering is a shift rather
    /// than a division.
    const ALPHABET: &'static [u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

    /// How many symbols a synthesized value spends.
    ///
    /// Seven of the eight bytes, leaving the first to mark it as synthetic.
    /// Seven symbols is thirty-five bits, so two distinct spellings collide
    /// at around a quarter of a million of them - far past what a venue
    /// declares, and the registration refuses a collision anyway rather than
    /// letting one happen quietly.
    const SYNTHETIC_SYMBOLS: usize = 7;

    /// The byte a synthesized value opens with.
    ///
    /// `~` is outside the alphabet and outside every message type FIX
    /// publishes, so a synthetic value is recognisable at a glance and can
    /// never be confused with one a venue actually sent.
    pub const SYNTHETIC_MARK: u8 = b'~';

    /// This spelling as a message type, synthesizing one where it will not fit.
    ///
    /// FIX's own types are one or two characters and this datatype holds
    /// eight, which is enough for every type the specification publishes and
    /// for most a venue invents. It is not enough for the composite keys a
    /// bridge writes - `P Report Ack` is twelve - and a value that does not
    /// fit cannot simply be truncated, because two keys sharing a prefix
    /// would become one message type.
    ///
    /// So a spelling that does not fit is *hashed* into one that does. The
    /// mapping is stable across processes and versions, because it is this
    /// crate's own digest over the exact bytes, and it is one-way: the
    /// spelling it came from is kept by whoever registers it, not recovered
    /// from the value.
    ///
    /// ```
    /// use yggdryl::types::MsgType;
    ///
    /// // What fits is itself, unchanged.
    /// assert_eq!(MsgType::coerce("D").as_str(), "D");
    /// assert_eq!(MsgType::coerce("AB").as_str(), "AB");
    ///
    /// // What does not is stable, marked, and never two things at once.
    /// let held = MsgType::coerce("P Report Ack");
    /// assert_eq!(held, MsgType::coerce("P Report Ack"));
    /// assert_ne!(held, MsgType::coerce("P Report Nack"));
    /// assert!(held.is_synthetic());
    /// assert_eq!(held.as_str().len(), 8);
    /// ```
    #[must_use]
    pub fn coerce(spelling: &str) -> Self {
        if let Ok(held) = Self::new(spelling) {
            return held;
        }
        Self::synthesized(spelling)
    }

    /// The synthetic value one spelling hashes to.
    fn synthesized(spelling: &str) -> Self {
        let digest = crate::digest::DigestAlgorithm::Xxh3
            .digest(spelling.as_bytes())
            .as_u64()
            .unwrap_or_default();
        let mut rendered = [0_u8; 8];
        rendered[0] = Self::SYNTHETIC_MARK;
        for (at, slot) in rendered[1..].iter_mut().enumerate() {
            let shift = 5 * (Self::SYNTHETIC_SYMBOLS - 1 - at);
            let symbol = (digest >> shift) & 0b1_1111;
            *slot = Self::ALPHABET[symbol as usize];
        }
        // Every byte is from the alphabet or the mark, so the width and the
        // ASCII rule both hold by construction.
        Self(SmolStr::new(
            std::str::from_utf8(&rendered).unwrap_or("~UNKNOWN"),
        ))
    }

    /// Whether this value was synthesized rather than sent.
    #[must_use]
    pub fn is_synthetic(&self) -> bool {
        self.as_str().as_bytes().first() == Some(&Self::SYNTHETIC_MARK)
    }

    /// Reads the message type one captured byte line declares.
    ///
    /// The same shallow scan [`MimeType::infer_bytes`](crate::MimeType) runs,
    /// asked for a different answer: a raw `MSGTYPE=` anywhere in the line is
    /// checked before numeric tag 35 and wins when both are present, because
    /// a bridge writes its own type in front of a frame it relays. FIX's
    /// user-defined `U*` range routes through one dictionary root.
    ///
    /// A [bridge configuration](crate::MimeType::ULCONFIG) document declares
    /// its own, and only where the line wrote neither of those. The ObjectName
    /// of the first MBean it names carries a `type=` segment - `Plugin` or
    /// `ConfigurationPlugin` - which is what that entry *is*; failing one, the
    /// Jolokia request's own `type` is the operation the document came from.
    ///
    /// The answer is a slice of the caller's bytes: no message is parsed and
    /// nothing is allocated. It is deliberately not validated to this type's
    /// width, because a line may carry anything and a classifier must not
    /// refuse what it was asked to read - [`Self::coerce`] is what makes a
    /// reading fit a column.
    ///
    /// ```
    /// use yggdryl::types::MsgType;
    ///
    /// let line = b"sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092";
    /// assert_eq!(MsgType::infer_bytes(line), Some(&b"D"[..]));
    /// // The user-defined range routes through one root.
    /// assert_eq!(MsgType::infer_bytes(b"35=U7|"), Some(&b"UDF"[..]));
    /// assert_eq!(MsgType::infer_bytes(b"no pairs here"), None);
    ///
    /// // A bridge configuration answers what the entry is, and `plugin-type=`
    /// // is not that segment however alike its last five bytes look.
    /// let entry = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"},"status":200}"#;
    /// assert_eq!(MsgType::infer_bytes(entry), Some(&b"Plugin"[..]));
    /// // A wildcard read names no type of its own, so the operation answers.
    /// let wildcard = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"status":200}"#;
    /// assert_eq!(MsgType::infer_bytes(wildcard), Some(&b"read"[..]));
    /// ```
    #[must_use]
    pub fn infer_bytes(line: &[u8]) -> Option<&[u8]> {
        crate::mime_type::line::inspect(line).msgtype()
    }

    /// Reads the message type one captured text line declares.
    ///
    /// Answers nothing where the bytes it found are not text, because a
    /// message type that cannot be spelled is not one a caller can use.
    #[must_use]
    pub fn infer_text(line: &str) -> Option<&str> {
        std::str::from_utf8(Self::infer_bytes(line.as_bytes())?).ok()
    }
}

impl MsgDirection {
    /// The direction a line moved when nothing in it says otherwise.
    ///
    /// A session's own log is written by the side doing the sending, so its
    /// unmarked lines are the ones it sent and its inbound lines are the ones
    /// it bothered to mark.
    pub const SENT: &'static str = "SENT";

    /// The direction of a line the transport marked as arriving.
    pub const RECV: &'static str = "RECV";

    /// Reads which way one captured byte line moved.
    ///
    /// The verb is read **in front of the payload**, never inside it. Where a
    /// message starts is where the transport's own prose stops, so a `sent`
    /// inside a FIX `Text(58)`, a bridge value spelled `OUT=1`, or an XML
    /// payload's own wording never becomes a direction.
    ///
    /// A prefix carrying both verbs, and one carrying neither, both answer
    /// nothing: there is no verb the reading can prefer, and inventing one
    /// would be a guess. Except where the payload is a document that states
    /// its own half of an exchange - a
    /// [bridge configuration](crate::MimeType::ULCONFIG) echoing back the
    /// request it answers came back, and one that is a bare request went out.
    /// A verb the transport wrote still wins over what the document says
    /// about itself.
    ///
    /// ```
    /// use yggdryl::types::MsgDirection;
    ///
    /// assert_eq!(
    ///     MsgDirection::infer_bytes(b"sending >> 8=FIX.4.2|35=D|10=203|"),
    ///     Some(MsgDirection::SENT)
    /// );
    /// assert_eq!(
    ///     MsgDirection::infer_bytes(b"recv 8=FIX.4.4|35=0|10=017|"),
    ///     Some(MsgDirection::RECV)
    /// );
    /// // A verb only inside the payload is the payload's word, not a marker.
    /// assert_eq!(
    ///     MsgDirection::infer_bytes(b"8=FIX.4.4|35=8|58=sent earlier|10=1|"),
    ///     None
    /// );
    /// // English that merely contains the letters is not a marker.
    /// assert_eq!(MsgDirection::infer_bytes(b"sending in session 3"), None);
    /// assert_eq!(MsgDirection::infer_bytes(b"received out of order"), None);
    ///
    /// // A document answered by a status came back, and the words its own
    /// // payload spells - `send-test-request` here - are never the marker.
    /// let answered = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"name":"send-test-request"},"status":200}"#;
    /// assert_eq!(MsgDirection::infer_bytes(answered), Some(MsgDirection::RECV));
    /// let asked = br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#;
    /// assert_eq!(MsgDirection::infer_bytes(asked), Some(MsgDirection::SENT));
    /// ```
    #[must_use]
    pub fn infer_bytes(line: &[u8]) -> Option<&'static str> {
        Self::split_bytes(line).0
    }

    /// Reads which way one captured text line moved.
    #[must_use]
    pub fn infer_text(line: &str) -> Option<&'static str> {
        Self::infer_bytes(line.as_bytes())
    }

    /// Reads the direction and answers the line with its marker removed.
    ///
    /// Stripping is what makes the reading free downstream: the verb is
    /// transport prose rather than payload, so a body that keeps it carries a
    /// word no protocol sent. What is removed is the marker and the
    /// whitespace after it, never the payload.
    ///
    /// ```
    /// use yggdryl::types::MsgDirection;
    ///
    /// let (direction, body) = MsgDirection::split_bytes(b"sending >> 8=FIX.4.2|35=D|");
    /// assert_eq!(direction, Some(MsgDirection::SENT));
    /// assert_eq!(body, b">> 8=FIX.4.2|35=D|");
    ///
    /// // Nothing read is nothing removed.
    /// let (none, whole) = MsgDirection::split_bytes(b"8=FIX.4.4|35=D|");
    /// assert_eq!(none, None);
    /// assert_eq!(whole, b"8=FIX.4.4|35=D|");
    /// ```
    #[must_use]
    pub fn split_bytes(line: &[u8]) -> (Option<&'static str>, &[u8]) {
        let (bound, stated) = crate::mime_type::line::payload(line);
        let (marked, rest) = Self::split_within(line, bound.unwrap_or(line.len()));
        // A document states its own half of an exchange, and states it with no
        // marker to take off: what is read there leaves the line whole.
        (marked.or_else(|| Self::stated(stated)), rest)
    }

    /// The direction a payload that states its own half of an exchange took.
    ///
    /// The reading is the document's, so the vocabulary stays here: the scan
    /// answers which half it is and this names the half.
    const fn stated(answered: Option<bool>) -> Option<&'static str> {
        match answered {
            Some(true) => Some(Self::RECV),
            Some(false) => Some(Self::SENT),
            None => None,
        }
    }

    /// Reads which way a line moved, given where its payload starts.
    ///
    /// The reading is the same; what this adds is that the caller already
    /// knows the offset. A reader has located the frame to parse it, and
    /// locating it twice is the only cost the bounded reading has.
    ///
    /// The default fills silence and never overrides a statement: a line
    /// carrying a verb answers that verb, a payload that states its own half
    /// of an exchange answers that, and only a line stating neither - or
    /// carrying both verbs, which is a line no reading can prefer one of -
    /// takes the default. FIX parsing passes [`MsgDirection::SENT`], because a
    /// session's own log is written by the side doing the sending and its
    /// unmarked lines are the ones it sent.
    ///
    /// ```
    /// use yggdryl::types::MsgDirection;
    ///
    /// let line = b"sending >> 8=FIX.4.2|35=D|58=received out of order|10=0|";
    /// let at = 11; // where the reader found the frame
    /// assert_eq!(
    ///     MsgDirection::at_payload(line, at, Some(MsgDirection::SENT)),
    ///     Some(MsgDirection::SENT),
    ///     "the verb inside Text(58) is payload, not prose",
    /// );
    ///
    /// // A line the transport did not mark takes the default, and a line
    /// // with no default takes nothing.
    /// let bare = b"8=FIX.4.2|35=D|10=0|";
    /// assert_eq!(MsgDirection::at_payload(bare, 0, Some(MsgDirection::SENT)), Some(MsgDirection::SENT));
    /// assert_eq!(MsgDirection::at_payload(bare, 0, None), None);
    /// ```
    #[must_use]
    pub fn at_payload(
        line: &[u8],
        payload_at: usize,
        default: Option<&'static str>,
    ) -> Option<&'static str> {
        Self::split_within(line, payload_at.min(line.len()))
            .0
            .or_else(|| Self::stated(crate::mime_type::line::payload(line).1))
            .or(default)
    }

    /// The reading, over a prefix the caller has already bounded.
    fn split_within(line: &[u8], bound: usize) -> (Option<&'static str>, &[u8]) {
        let prefix = &line[..bound];
        let mut found: Option<(&'static str, usize)> = None;
        for (start, end, direction, selectable) in markers(prefix) {
            // A bare `in` or `out` conflicts even where it could not be
            // chosen: `sending in session 3` and `received out of order` are
            // English, and a prefix carrying both verbs has none a reading
            // can prefer.
            if found.is_some_and(|(held, _)| held != direction) {
                return (None, line);
            }
            if selectable {
                // A bracketed marker owns its bracket, so the pair goes
                // together; an unbracketed one owns only itself, which is why
                // an arrow after it survives for an lstrip pattern to take.
                let closed = matches!(
                    (prefix.get(start.wrapping_sub(1)), line.get(end)),
                    (Some(b'['), Some(b']')) | (Some(b'('), Some(b')')) | (Some(b'<'), Some(b'>'))
                );
                found.get_or_insert((direction, end + usize::from(closed)));
            } else if found.is_none() {
                found = Some((direction, usize::MAX));
            }
        }
        match found {
            Some((direction, end)) if end != usize::MAX => {
                (Some(direction), trim_start(&line[end..]))
            }
            _ => (None, line),
        }
    }

    /// Reads the direction and answers the text with its marker removed.
    #[must_use]
    pub fn split_text(line: &str) -> (Option<&'static str>, &str) {
        let (direction, rest) = Self::split_bytes(line.as_bytes());
        // The split lands after an ASCII verb, so the tail is still text.
        (direction, std::str::from_utf8(rest).unwrap_or(line))
    }
}

/// The verbs a transport marks a line with, longest first inside each
/// direction so `received` is not read as `receive`.
///
/// Domain knowledge, written out where a reviewer can check it rather than
/// inferred from spelling. The third element marks the two bare forms, which
/// match under a stricter rule.
const VERBS: [(&[u8], &str, bool); 13] = [
    (b"sending", MsgDirection::SENT, false),
    (b"sent", MsgDirection::SENT, false),
    (b"send", MsgDirection::SENT, false),
    (b"outbound", MsgDirection::SENT, false),
    (b"outgoing", MsgDirection::SENT, false),
    (b"out", MsgDirection::SENT, true),
    (b"receiving", MsgDirection::RECV, false),
    (b"received", MsgDirection::RECV, false),
    (b"receive", MsgDirection::RECV, false),
    (b"recv", MsgDirection::RECV, false),
    (b"inbound", MsgDirection::RECV, false),
    (b"incoming", MsgDirection::RECV, false),
    (b"in", MsgDirection::RECV, true),
];

/// Every direction marker standing in one prefix, with its bounds.
fn markers(prefix: &[u8]) -> impl Iterator<Item = (usize, usize, &'static str, bool)> + '_ {
    (0..prefix.len()).filter_map(move |start| {
        VERBS.iter().find_map(|(verb, direction, bare)| {
            let end = start + verb.len();
            if prefix.len() < end || !prefix[start..end].eq_ignore_ascii_case(verb) {
                return None;
            }
            if !opens_marker(prefix, start) || !closes_marker(prefix, end) {
                return None;
            }
            // A bare `in` or `out` is *chosen* only where a bracket opens it
            // and a delimiter closes it, because that is the one shape a
            // marker has and none of the shapes the same letters have
            // otherwise: `direct:out` is a route endpoint and
            // `MCFID-IN-XPAR` is a session name. It still counts against an
            // opposite verb, which is what makes `sending in session 3`
            // answer nothing rather than `SENT`.
            Some((
                start,
                end,
                *direction,
                !*bare || bare_marker(prefix, start, end),
            ))
        })
    })
}

/// Whether a marker may open at `start`.
fn opens_marker(prefix: &[u8], start: usize) -> bool {
    start == 0
        || prefix
            .get(start - 1)
            .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'[' | b'(' | b'<'))
}

/// Whether a marker may close at `end`.
fn closes_marker(prefix: &[u8], end: usize) -> bool {
    prefix.get(end).is_none_or(|byte| {
        byte.is_ascii_whitespace() || matches!(byte, b']' | b')' | b'>' | b':' | b',')
    })
}

/// Whether a bare `in` or `out` stands as a marker rather than as English.
fn bare_marker(prefix: &[u8], start: usize, end: usize) -> bool {
    let opened = start == 0
        || prefix
            .get(start - 1)
            .is_some_and(|byte| matches!(byte, b'[' | b'('));
    let closed = prefix
        .get(end)
        .is_none_or(|byte| matches!(byte, b']' | b')' | b':'));
    opened && closed
}

/// The bytes with leading ASCII whitespace removed.
fn trim_start(line: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < line.len() && line[start].is_ascii_whitespace() {
        start += 1;
    }
    &line[start..]
}

/// One exact ASCII storage or registered-code representation.
///
/// `AsciiFamily` carries the suffix because Rust cannot place the family enum
/// and its required `Ascii` leaf in the same type namespace.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum AsciiFamily {
    /// Variable-width ASCII.
    Ascii(Ascii),
    /// Fixed-width ASCII.
    FixedAscii(FixedAscii),
    /// ISO 3166-1 alpha-2 country code.
    Country(Country),
    /// ISO 4217 currency code.
    Currency(Currency),
    /// ISO 10383 market identifier code.
    Mic(Mic),
    /// ISO 10962 classification code.
    Cfi(Cfi),
    /// FIX's side of a trade.
    Side(Side),
    /// FIX's message type, case-bearing.
    MsgType(MsgType),
    /// Which way a captured line moved.
    MsgDirection(MsgDirection),
    /// What state one thing is in, ranked so the bytes sort by lifecycle.
    State(State),
    /// How long an order stands.
    TimeInForce(TimeInForce),
}

impl AsciiFamily {
    /// Borrow the validated text independently of its storage identity.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ascii(value) => value.as_str(),
            Self::FixedAscii(value) => value.as_str(),
            Self::Country(value) => value.as_str(),
            Self::Currency(value) => value.as_str(),
            Self::Mic(value) => value.as_str(),
            Self::Cfi(value) => value.as_str(),
            Self::Side(value) => value.as_str(),
            Self::MsgType(value) => value.as_str(),
            Self::MsgDirection(value) => value.as_str(),
            Self::State(value) => value.as_str(),
            Self::TimeInForce(value) => value.as_str(),
        }
    }

    /// Borrow the shared storage independently of the storage identity.
    ///
    /// Every member holds the same trimmed, validated text, so a rewrite
    /// between two of them clones this handle instead of the characters.
    pub fn storage(&self) -> &SmolStr {
        match self {
            Self::Ascii(value) => value.storage(),
            Self::FixedAscii(value) => value.storage(),
            Self::Country(value) => value.storage(),
            Self::Currency(value) => value.storage(),
            Self::Mic(value) => value.storage(),
            Self::Cfi(value) => value.storage(),
            Self::Side(value) => value.storage(),
            Self::MsgType(value) => value.storage(),
            Self::MsgDirection(value) => value.storage(),
            Self::State(value) => value.storage(),
            Self::TimeInForce(value) => value.storage(),
        }
    }
}

impl fmt::Display for AsciiFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for AsciiFamily {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for AsciiFamily {}

impl PartialOrd for AsciiFamily {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AsciiFamily {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for AsciiFamily {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

macro_rules! ascii_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $dtype:ident, $width:expr) => {
        impl ScalarValue for $leaf {
            type Family = AsciiFamily;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Ascii;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$dtype)
            }

            fn into_family(self) -> Self::Family {
                AsciiFamily::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    AsciiFamily::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Ascii(AsciiFamily::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Ascii(AsciiFamily::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl AsciiValue for $leaf {
            const WIDTH: Option<i32> = $width;

            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }

            fn storage(&self) -> &SmolStr {
                <$leaf>::storage(self)
            }
        }
    };
}

ascii_value!(Ascii, super::AsciiType, Ascii, Ascii, Ascii, None);
ascii_value!(
    Country,
    super::CountryType,
    Country,
    Country,
    Country,
    Some(2)
);
ascii_value!(
    Currency,
    super::CurrencyType,
    Currency,
    Currency,
    Currency,
    Some(3)
);
ascii_value!(Mic, super::MicType, Mic, Mic, Mic, Some(4));
ascii_value!(Cfi, super::CfiType, Cfi, Cfi, Cfi, Some(6));
ascii_value!(Side, super::SideType, Side, Side, Side, Some(4));
ascii_value!(
    MsgType,
    super::MsgTypeType,
    MsgType,
    MsgType,
    MsgType,
    Some(8)
);
ascii_value!(
    MsgDirection,
    super::MsgDirectionType,
    MsgDirection,
    MsgDirection,
    MsgDirection,
    Some(4)
);

impl ScalarValue for FixedAscii {
    type Family = AsciiFamily;
    type Type = super::FixedAsciiType;

    const ID: DataTypeId = DataTypeId::FixedAscii;
    const KIND: DataTypeKind = DataTypeKind::Ascii;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::FixedAscii(self.width()))
    }

    fn into_family(self) -> Self::Family {
        AsciiFamily::FixedAscii(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            AsciiFamily::FixedAscii(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Ascii(AsciiFamily::FixedAscii(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Ascii(AsciiFamily::FixedAscii(value)) => Some(value),
            _ => None,
        }
    }
}

impl AsciiValue for FixedAscii {
    const WIDTH: Option<i32> = None;

    fn as_str(&self) -> &str {
        Self::as_str(self)
    }

    fn storage(&self) -> &SmolStr {
        Self::storage(self)
    }
}

impl ScalarFamily for AsciiFamily {
    const KIND: DataTypeKind = DataTypeKind::Ascii;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Ascii(_) => DataTypeId::Ascii,
            Self::FixedAscii(_) => DataTypeId::FixedAscii,
            Self::Country(_) => DataTypeId::Country,
            Self::Currency(_) => DataTypeId::Currency,
            Self::Mic(_) => DataTypeId::Mic,
            Self::Cfi(_) => DataTypeId::Cfi,
            Self::Side(_) => DataTypeId::Side,
            Self::MsgType(_) => DataTypeId::MsgType,
            Self::MsgDirection(_) => DataTypeId::MsgDirection,
            Self::State(_) => DataTypeId::State,
            Self::TimeInForce(_) => DataTypeId::TimeInForce,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Ascii(_) => Ok(DataType::Ascii),
            Self::FixedAscii(value) => ScalarValue::dtype(value),
            Self::Country(_) => Ok(DataType::Country),
            Self::Currency(_) => Ok(DataType::Currency),
            Self::Mic(_) => Ok(DataType::Mic),
            Self::Cfi(_) => Ok(DataType::Cfi),
            Self::Side(_) => Ok(DataType::Side),
            Self::MsgType(_) => Ok(DataType::MsgType),
            Self::MsgDirection(_) => Ok(DataType::MsgDirection),
            Self::State(_) => Ok(DataType::State),
            Self::TimeInForce(_) => Ok(DataType::TimeInForce),
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Ascii(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Ascii(value) => Some(value),
            _ => None,
        }
    }
}

define_scalar_type!(
    AsciiScalar,
    super::AsciiType,
    "ascii",
    crate::DataType::Ascii
);
define_scalar_type!(FixedAsciiScalar, super::FixedAsciiType, "fixed_ascii");
define_scalar_type!(
    CountryScalar,
    super::CountryType,
    "country",
    crate::DataType::Country
);
define_scalar_type!(
    CurrencyScalar,
    super::CurrencyType,
    "currency",
    crate::DataType::Currency
);
define_scalar_type!(MicScalar, super::MicType, "mic", crate::DataType::Mic);
define_scalar_type!(CfiScalar, super::CfiType, "cfi", crate::DataType::Cfi);
define_scalar_type!(SideScalar, super::SideType, "side", crate::DataType::Side);
define_scalar_type!(
    MsgTypeScalar,
    super::MsgTypeType,
    "msgtype",
    crate::DataType::MsgType
);
define_scalar_type!(
    MsgDirectionScalar,
    super::MsgDirectionType,
    "direction",
    crate::DataType::MsgDirection
);
define_scalar_type!(
    StateScalar,
    super::StateType,
    "state",
    crate::DataType::State
);
define_scalar_type!(
    TimeInForceScalar,
    super::TimeInForceType,
    "timeinforce",
    crate::DataType::TimeInForce
);
