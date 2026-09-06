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
ascii_code_leaf!(Direction, 4);

impl MsgType {
    /// Reads the message type one captured byte line declares.
    ///
    /// The same shallow scan [`MimeType::infer_bytes`](crate::MimeType) runs,
    /// asked for a different answer: a raw `MSGTYPE=` anywhere in the line is
    /// checked before numeric tag 35 and wins when both are present, because
    /// a bridge writes its own type in front of a frame it relays. FIX's
    /// user-defined `U*` range routes through one dictionary root.
    ///
    /// The answer is a slice of the caller's bytes: no message is parsed and
    /// nothing is allocated. It is deliberately not validated to this type's
    /// width, because a line may carry anything and a classifier must not
    /// refuse what it was asked to read.
    ///
    /// ```
    /// use yggdryl::types::MsgType;
    ///
    /// let line = b"sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092";
    /// assert_eq!(MsgType::infer_bytes(line), Some(&b"D"[..]));
    /// // The user-defined range routes through one root.
    /// assert_eq!(MsgType::infer_bytes(b"35=U7|"), Some(&b"UDF"[..]));
    /// assert_eq!(MsgType::infer_bytes(b"no pairs here"), None);
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

impl Direction {
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
    /// would be a guess.
    ///
    /// ```
    /// use yggdryl::types::Direction;
    ///
    /// assert_eq!(
    ///     Direction::infer_bytes(b"sending >> 8=FIX.4.2|35=D|10=203|"),
    ///     Some(Direction::SENT)
    /// );
    /// assert_eq!(
    ///     Direction::infer_bytes(b"recv 8=FIX.4.4|35=0|10=017|"),
    ///     Some(Direction::RECV)
    /// );
    /// // A verb only inside the payload is the payload's word, not a marker.
    /// assert_eq!(
    ///     Direction::infer_bytes(b"8=FIX.4.4|35=8|58=sent earlier|10=1|"),
    ///     None
    /// );
    /// // English that merely contains the letters is not a marker.
    /// assert_eq!(Direction::infer_bytes(b"sending in session 3"), None);
    /// assert_eq!(Direction::infer_bytes(b"received out of order"), None);
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
    /// use yggdryl::types::Direction;
    ///
    /// let (direction, body) = Direction::split_bytes(b"sending >> 8=FIX.4.2|35=D|");
    /// assert_eq!(direction, Some(Direction::SENT));
    /// assert_eq!(body, b">> 8=FIX.4.2|35=D|");
    ///
    /// // Nothing read is nothing removed.
    /// let (none, whole) = Direction::split_bytes(b"8=FIX.4.4|35=D|");
    /// assert_eq!(none, None);
    /// assert_eq!(whole, b"8=FIX.4.4|35=D|");
    /// ```
    #[must_use]
    pub fn split_bytes(line: &[u8]) -> (Option<&'static str>, &[u8]) {
        let bound = crate::mime_type::line::payload_at(line).unwrap_or(line.len());
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
    (b"sending", Direction::SENT, false),
    (b"sent", Direction::SENT, false),
    (b"send", Direction::SENT, false),
    (b"outbound", Direction::SENT, false),
    (b"outgoing", Direction::SENT, false),
    (b"out", Direction::SENT, true),
    (b"receiving", Direction::RECV, false),
    (b"received", Direction::RECV, false),
    (b"receive", Direction::RECV, false),
    (b"recv", Direction::RECV, false),
    (b"inbound", Direction::RECV, false),
    (b"incoming", Direction::RECV, false),
    (b"in", Direction::RECV, true),
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
    Direction(Direction),
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
            Self::Direction(value) => value.as_str(),
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
    Direction,
    super::DirectionType,
    Direction,
    Direction,
    Direction,
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
            Self::Direction(_) => DataTypeId::Direction,
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
            Self::Direction(_) => Ok(DataType::Direction),
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
    DirectionScalar,
    super::DirectionType,
    "direction",
    crate::DataType::Direction
);
