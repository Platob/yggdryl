//! ASCII values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::dtypes::ISIN_WIDTH;
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

/// One validated ISO 6166 international securities identification number.
///
/// Twelve bytes: a two-letter prefix, nine alphanumerics of national number
/// and one check digit, which is the Luhn digit of the eleven before it read
/// with each letter expanded to the two digits of its alphabet position.
/// A spelling whose check digit does not close it is refused, because an
/// identifier that fails its own checksum is not that identifier - it is a
/// typo, and a typo typed as a security joins to the wrong one.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Isin(SmolStr);

impl Isin {
    /// Validate and construct a securities identification number.
    ///
    /// Lower case is read as the upper case it spells, because the number
    /// is case-insensitive by construction: the check digit expands a letter
    /// by its position, which case does not change.
    ///
    /// ```
    /// use yggdryl::types::Isin;
    ///
    /// let apple = Isin::new("US0378331005").unwrap();
    /// assert_eq!(apple.as_str(), "US0378331005");
    /// assert_eq!(apple.prefix(), "US");
    /// assert_eq!(apple.nsin(), "037833100");
    /// assert_eq!(apple.check_digit(), 5);
    /// assert_eq!(Isin::new("us0378331005").unwrap(), apple);
    /// // One digit off is a typo, not a security.
    /// assert!(Isin::new("US0378331006").is_err());
    /// assert!(Isin::new("US037833100").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not twelve ASCII bytes of the
    /// number's shape, or when its check digit does not close it.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_text(ISIN_WIDTH as i32, value.as_ref().as_bytes())?;
        let folded = value.to_ascii_uppercase();
        if let Some(reason) = Self::refusal(&folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "isin",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// Borrow the validated number.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the number.
    pub fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The two-letter prefix: the country of the numbering agency, or one of
    /// the international prefixes such as `XS`.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.0[..2]
    }

    /// The nine-character national securities identifying number.
    #[must_use]
    pub fn nsin(&self) -> &str {
        &self.0[2..11]
    }

    /// The check digit that closes the number.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.0.as_bytes()[11] - b'0'
    }

    /// Whether `text` spells a number this type would accept, in either case.
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        text.len() == ISIN_WIDTH
            && text.is_ascii()
            && Self::refusal(&text.to_ascii_uppercase()).is_none()
    }

    /// Whether `text` is a number exactly as this type stores it: upper
    /// case, and closed by its check digit.
    ///
    /// What a column holds is the canonical spelling, so bytes arriving
    /// through a cast are held to it rather than folded on every read.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == ISIN_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// The check digit that closes eleven leading characters, or `None`
    /// where they are not two letters and nine alphanumerics.
    ///
    /// ISO 6166 reads the eleven as digits - a letter as the two digits of
    /// its position from `A` at ten - and closes them with the Luhn digit,
    /// doubling every second digit from the right.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != ISIN_WIDTH - 1
            || !bytes[..2].iter().all(u8::is_ascii_uppercase)
            || !bytes[2..].iter().all(u8::is_ascii_alphanumeric)
        {
            return None;
        }
        // Eleven characters expand to at most twenty-two digits.
        let mut digits = [0_u8; 2 * (ISIN_WIDTH - 1)];
        let mut held = 0;
        for byte in bytes {
            match byte {
                b'0'..=b'9' => {
                    digits[held] = byte - b'0';
                    held += 1;
                }
                b'A'..=b'Z' => {
                    let position = byte - b'A' + 10;
                    digits[held] = position / 10;
                    digits[held + 1] = position % 10;
                    held += 2;
                }
                _ => return None,
            }
        }
        let mut sum = 0_u32;
        for (from_right, digit) in digits[..held].iter().rev().enumerate() {
            let mut value = u32::from(*digit);
            if from_right % 2 == 0 {
                value *= 2;
                if value > 9 {
                    value -= 9;
                }
            }
            sum += value;
        }
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    /// Why an upper-cased, twelve-byte spelling is not a number, or nothing.
    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != ISIN_WIDTH {
            return Some("expected twelve characters");
        }
        if !bytes[..2].iter().all(u8::is_ascii_uppercase) {
            return Some("expected a two-letter prefix");
        }
        if !bytes[2..11].iter().all(u8::is_ascii_alphanumeric) {
            return Some("expected nine alphanumerics after the prefix");
        }
        if !bytes[11].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        match Self::closing_digit(&folded[..11]) {
            Some(digit) if digit == bytes[11] - b'0' => None,
            _ => Some("the check digit does not close the number"),
        }
    }
}

impl fmt::Display for Isin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
ascii_code_leaf!(Side, 4);
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

    /// The state one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as the value contract reads it: a column typed
    /// `state` holds ranked values only, so text that names no state leaves
    /// the column null rather than storing a value nothing can rank.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "state",
            reason: smol_str::format_smolstr!(
                "expected a state code, name or stored value, got {spelling:?}"
            ),
        })
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
    ("restated", "70RESTATED"),
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
    /// ISO 6166 securities identification number.
    Isin(Isin),
    /// FIX's side of a trade.
    Side(Side),
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
            Self::Isin(value) => value.as_str(),
            Self::Side(value) => value.as_str(),
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
            Self::Isin(value) => value.storage(),
            Self::Side(value) => value.storage(),
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
ascii_value!(Isin, super::IsinType, Isin, Isin, Isin, Some(12));
ascii_value!(Side, super::SideType, Side, Side, Side, Some(4));
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
            Self::Isin(_) => DataTypeId::Isin,
            Self::Side(_) => DataTypeId::Side,
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
            Self::Isin(_) => Ok(DataType::Isin),
            Self::Side(_) => Ok(DataType::Side),
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
define_scalar_type!(IsinScalar, super::IsinType, "isin", crate::DataType::Isin);
define_scalar_type!(SideScalar, super::SideType, "side", crate::DataType::Side);
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
