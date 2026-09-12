//! Which way a message moved: FIX's own tag 385, read by the registry.
//!
//! A direction is a FIX fact and nothing else in the crate has one
//! (decision 14). The specification publishes it at tag 385, `MsgDirection`,
//! with the code set `R = Receive`, `S = Send`, and the dictionary types the
//! field as it types every coded field. What this module adds is the
//! *reading*: which code the prose a transport wrote in front of a payload
//! names - `sending >>`, `recv`, `[OUT]` - and which code a document that
//! states its own half of an exchange names. The registry answers it through
//! [`FixRegistry::msgdirection`], and a codec compiles it once when it takes
//! its registry, so no row asks the dictionary a question the row before it
//! asked.

use smol_str::SmolStr;

use super::FixRegistry;
use super::MSGDIRECTION_TAG_NAME;
use crate::{DataType, Field};

/// The name tag 385's set gives the code a sent message carries.
const SEND_NAME: &str = "Send";
/// The name tag 385's set gives the code a received message carries.
const RECEIVE_NAME: &str = "Receive";
/// The specification's own code for a sent message, where a dictionary
/// declares no set.
const SEND_CODE: &str = "S";
/// The specification's own code for a received message, where a dictionary
/// declares no set.
const RECEIVE_CODE: &str = "R";

/// The two halves of an exchange the reading can name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Way {
    Sent,
    Recv,
}

/// The registry's reading of tag 385: its code set, and which code the
/// prose in front of a payload names.
///
/// ```
/// use yggdryl::FixRegistry;
///
/// let reading = FixRegistry::new().msgdirection();
/// assert_eq!(reading.sent(), "S");
/// assert_eq!(reading.recv(), "R");
/// assert_eq!(reading.read_bytes(b"sending >> 8=FIX.4.2|35=D|10=203|"), Some("S"));
/// assert_eq!(reading.read_bytes(b"recv 8=FIX.4.4|35=0|10=017|"), Some("R"));
/// // A verb only inside the payload is the payload's word, not a marker.
/// assert_eq!(reading.read_bytes(b"8=FIX.4.4|35=8|58=sent earlier|10=1|"), None);
/// // A spelling of a code - its value, its name - resolves to the code.
/// assert_eq!(reading.code("Send"), Some("S"));
/// assert_eq!(reading.code("R"), Some("R"));
/// assert_eq!(reading.code("sideways"), None);
/// ```
#[derive(Clone, Debug)]
pub struct MsgDirection {
    /// Tag 385 as a built message carries it: the dictionary's field, non-null,
    /// or a text field carrying the tag where the dictionary has none.
    field: Field,
    /// Every code of the set, in the set's order, and the name each carries.
    codes: Vec<(SmolStr, SmolStr)>,
    sent: SmolStr,
    recv: SmolStr,
}

impl MsgDirection {
    /// The reading one registry answers, from what its tag 385 declares.
    ///
    /// A dictionary without the field, or with a set naming neither `Send`
    /// nor `Receive`, answers the specification's own codes for the two
    /// halves it does not name, so a direction is never silently absent.
    pub(super) fn from_registry(registry: &FixRegistry) -> Self {
        let declared = registry.get_field_by_tag(MSGDIRECTION_TAG_NAME.0);
        let field = declared.map_or_else(
            || {
                let mut field =
                    DataType::utf8().required_field(MSGDIRECTION_TAG_NAME.1.to_ascii_lowercase());
                let _ = field.as_fix_mut().set_tag(MSGDIRECTION_TAG_NAME.0);
                field
            },
            super::build::stated,
        );
        let codes: Vec<(SmolStr, SmolStr)> = field
            .as_fix()
            .codes()
            .filter_map(Result::ok)
            .map(|code| (SmolStr::new(code.value()), SmolStr::new(code.name())))
            .collect();
        let named = |name: &str, default: &str| {
            field
                .as_fix()
                .code_value(name)
                .map_or_else(|| SmolStr::new(default), SmolStr::new)
        };
        let sent = named(SEND_NAME, SEND_CODE);
        let recv = named(RECEIVE_NAME, RECEIVE_CODE);
        Self {
            field,
            codes,
            sent,
            recv,
        }
    }

    /// Tag 385 as a built message carries it.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// The code a sent message carries: the set's `Send`, else `S`.
    #[must_use]
    pub fn sent(&self) -> &str {
        &self.sent
    }

    /// The code a received message carries: the set's `Receive`, else `R`.
    #[must_use]
    pub fn recv(&self) -> &str {
        &self.recv
    }

    /// Every code of the set, in the order the dictionary holds it; the two
    /// the specification declares where the dictionary declares none.
    pub fn codes(&self) -> impl Iterator<Item = &str> {
        let declared = self.codes.iter().map(|(value, _)| value.as_str());
        let fallback = [self.sent.as_str(), self.recv.as_str()].into_iter();
        if self.codes.is_empty() {
            Either::Left(fallback)
        } else {
            Either::Right(declared)
        }
    }

    /// The code one spelling names: its value, or the name the set gives it,
    /// ASCII case folded; `None` where the set holds no such code.
    ///
    /// The one place a spelling from outside becomes a code of the set, so a
    /// pin and a stated column resolve exactly alike.
    #[must_use]
    pub fn code(&self, spelling: &str) -> Option<&str> {
        let spelling = spelling.trim();
        if spelling.is_empty() {
            return None;
        }
        if let Some(value) = self.field.as_fix().code_value(spelling) {
            return Some(value);
        }
        [
            (self.sent.as_str(), SEND_NAME),
            (self.recv.as_str(), RECEIVE_NAME),
        ]
        .into_iter()
        .find(|(value, name)| {
            value.eq_ignore_ascii_case(spelling) || name.eq_ignore_ascii_case(spelling)
        })
        .map(|(value, _)| value)
    }

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
    /// its own half of an exchange - a bridge configuration echoing back the
    /// request it answers came back, and one that is a bare request went
    /// out. A verb the transport wrote still wins over what the document
    /// says about itself.
    #[must_use]
    pub fn read_bytes(&self, line: &[u8]) -> Option<&str> {
        let (bound, stated) = crate::mime_type::line::payload(line);
        self.read_prefix(&line[..bound.unwrap_or(line.len())])
            .or_else(|| self.stated(stated))
    }

    /// Reads which way one captured text line moved.
    #[must_use]
    pub fn read_text(&self, line: &str) -> Option<&str> {
        self.read_bytes(line.as_bytes())
    }

    /// The reading, over a prefix the caller has already bounded.
    ///
    /// A reader that located the frame to parse it hands the prose in front
    /// of it here, so the frame is located once.
    #[must_use]
    pub(super) fn read_prefix(&self, prefix: &[u8]) -> Option<&str> {
        let mut found: Option<(Way, bool)> = None;
        for (way, selectable) in markers(prefix) {
            // A bare `in` or `out` conflicts even where it could not be
            // chosen: `sending in session 3` and `received out of order` are
            // English, and a prefix carrying both verbs has none a reading
            // can prefer.
            if found.is_some_and(|(held, _)| held != way) {
                return None;
            }
            match found {
                Some((_, true)) => {}
                _ => found = Some((way, selectable)),
            }
        }
        match found {
            Some((way, true)) => Some(self.of(way)),
            _ => None,
        }
    }

    /// The direction a payload that states its own half of an exchange took.
    pub(super) fn stated(&self, answered: Option<bool>) -> Option<&str> {
        match answered {
            Some(true) => Some(self.recv()),
            Some(false) => Some(self.sent()),
            None => None,
        }
    }

    fn of(&self, way: Way) -> &str {
        match way {
            Way::Sent => self.sent(),
            Way::Recv => self.recv(),
        }
    }
}

/// One of two iterators, so `codes` answers without allocating.
enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<'a, L, R> Iterator for Either<L, R>
where
    L: Iterator<Item = &'a str>,
    R: Iterator<Item = &'a str>,
{
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(left) => left.next(),
            Self::Right(right) => right.next(),
        }
    }
}

/// The verbs a transport marks a line with, longest first inside each
/// direction so `received` is not read as `receive`.
///
/// Domain knowledge, written out where a reviewer can check it rather than
/// inferred from spelling. The third element marks the two bare forms, which
/// match under a stricter rule. Decision 15 moves this table into the
/// dictionary as rules tag 385 carries.
const VERBS: [(&[u8], Way, bool); 13] = [
    (b"sending", Way::Sent, false),
    (b"sent", Way::Sent, false),
    (b"send", Way::Sent, false),
    (b"outbound", Way::Sent, false),
    (b"outgoing", Way::Sent, false),
    (b"out", Way::Sent, true),
    (b"receiving", Way::Recv, false),
    (b"received", Way::Recv, false),
    (b"receive", Way::Recv, false),
    (b"recv", Way::Recv, false),
    (b"inbound", Way::Recv, false),
    (b"incoming", Way::Recv, false),
    (b"in", Way::Recv, true),
];

/// Every direction marker standing in one prefix, and whether it may be
/// chosen.
fn markers(prefix: &[u8]) -> impl Iterator<Item = (Way, bool)> + '_ {
    (0..prefix.len()).filter_map(move |start| {
        VERBS.iter().find_map(|(verb, way, bare)| {
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
            // answer nothing rather than `S`.
            Some((*way, !*bare || bare_marker(prefix, start, end)))
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
