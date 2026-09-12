//! Which way a message moved: FIX's own tag 385, read by the registry.
//!
//! A direction is a FIX fact and nothing else in the crate has one
//! (decision 14). The specification publishes it at tag 385, `MsgDirection`,
//! with the code set `R = Receive`, `S = Send`, and the dictionary types the
//! field as it types every coded field. What this module adds is the
//! *reading*: which code the prose a transport wrote in front of a payload
//! names - `sending >>`, `recv`, `[OUT]`, a Jolokia `Response:`. The rules
//! are the dictionary's, carried on tag 385's field as
//! [`fix:directions`](super::directions) (decision 15), and the defaults
//! below answer where the field carries none. The registry answers the
//! reading through [`FixRegistry::msgdirection`], and a codec compiles it
//! once when it takes its registry, so no row builds a regex and no row asks
//! the dictionary a question the row before it asked.

use regex::bytes::Regex;
use smol_str::SmolStr;

use super::FixRegistry;
use super::MSGDIRECTION_TAG_NAME;
use super::directions::{FixDirection, compile, outside_set, repeated};
use crate::{DataType, Field, FixField};

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

/// The default patterns naming the `Send` code, where tag 385 carries no
/// `fix:directions`.
///
/// Domain knowledge, written out where a reviewer can check it rather than
/// inferred from spelling: the spelled verbs, opened by the start of the
/// prefix, whitespace or a bracket and closed by the end, whitespace or a
/// delimiter; the bare word only where a bracket opens it and a delimiter
/// closes it, because that is the one shape a marker has and none of the
/// shapes the same letters have otherwise - `direct:out` is a route
/// endpoint; and the half a Jolokia exchange states in its prose.
pub const SEND_PATTERNS: [&str; 3] = [
    r"(?i)(?:^|[\s\[(<])(?:sending|sent|send|outbound|outgoing)(?:[\s\])>:,]|$)",
    r"(?i)(?:^|[\[(])out(?:[\]):]|$)",
    r"(?i)(?:^|\s)request:",
];

/// The default patterns naming the `Receive` code, the mirror of
/// [`SEND_PATTERNS`]: `MCFID-IN-XPAR` is a session name and reads nothing.
pub const RECEIVE_PATTERNS: [&str; 3] = [
    r"(?i)(?:^|[\s\[(<])(?:receiving|received|receive|recv|inbound|incoming)(?:[\s\])>:,]|$)",
    r"(?i)(?:^|[\[(])in(?:[\]):]|$)",
    r"(?i)(?:^|\s)response:",
];

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
/// // The rules in force are data: the defaults, where the dictionary
/// // carries none.
/// assert_eq!(reading.directions().len(), 2);
/// assert_eq!(reading.directions()[0].code(), "S");
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
    /// The rules in force, each code resolved to the set's value and every
    /// pattern one the regex crate compiled.
    directions: Vec<FixDirection>,
    /// Every pattern of every rule, compiled once; applying one to a prefix
    /// allocates nothing, which is what a per-row reading has to cost.
    rules: Vec<Regex>,
    /// Which rule the pattern at each index of `rules` belongs to.
    owners: Vec<usize>,
}

impl MsgDirection {
    /// The reading one registry answers, from what its tag 385 declares.
    ///
    /// A dictionary without the field, or with a set naming neither `Send`
    /// nor `Receive`, answers the specification's own codes for the two
    /// halves it does not name, so a direction is never silently absent. A
    /// field carrying no `fix:directions` reads by the defaults, keyed by
    /// those two codes; one carrying the property reads by what it states.
    /// A rule naming no code of the set, a second rule naming a code
    /// already named under another spelling, a pattern the regex crate
    /// refuses, or an entry the document grammar refuses, is dropped with a
    /// warning that is the refusal
    /// [`set_directions`](crate::FixFieldMut::set_directions) would have
    /// raised: the setter is the door, and a dictionary edited by hand
    /// degrades to fewer rules - down to none - rather than to a wrong
    /// reading.
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
        let mut reading = Self {
            field,
            codes,
            sent,
            recv,
            directions: Vec::new(),
            rules: Vec::new(),
            owners: Vec::new(),
        };
        let stated: Vec<FixDirection> = {
            let walk = reading.field.as_fix().directions();
            if walk.is_stated() {
                walk.filter_map(|entry| match entry {
                    Ok(entry) => Some(FixDirection::from(entry)),
                    Err(error) => {
                        warned(&error);
                        None
                    }
                })
                .collect()
            } else {
                // The defaults are what an absent property reads by; a
                // property the field carries reads by what it states,
                // however little of it survives the drops warned about here
                // and below.
                vec![
                    FixDirection::new(reading.sent.clone(), SEND_PATTERNS),
                    FixDirection::new(reading.recv.clone(), RECEIVE_PATTERNS),
                ]
            }
        };
        reading.compile(stated);
        reading
    }

    /// Compiles the rules, resolving each code against the set and keeping
    /// the patterns the regex crate accepts.
    ///
    /// Every drop is warned about with the refusal the setter raises for
    /// the same input, so a hand-edited dictionary and a refused edit say
    /// one thing.
    fn compile(&mut self, rules: Vec<FixDirection>) {
        let mut directions: Vec<FixDirection> = Vec::with_capacity(rules.len());
        let mut compiled = Vec::new();
        let mut owners = Vec::new();
        for rule in rules {
            let Some(code) = self.code(rule.code()).map(SmolStr::new) else {
                warned(&outside_set(rule.code(), self.codes()));
                continue;
            };
            if directions.iter().any(|held| held.code() == code) {
                warned(&repeated(rule.code(), &code));
                continue;
            }
            let mut kept: Vec<&str> = Vec::with_capacity(rule.patterns().len());
            for pattern in rule.patterns() {
                match compile(pattern) {
                    Ok(regex) => {
                        kept.push(pattern);
                        compiled.push(regex);
                        owners.push(directions.len());
                    }
                    Err(error) => warned(&error),
                }
            }
            if kept.is_empty() {
                continue;
            }
            directions.push(FixDirection::new(code, kept));
        }
        self.rules = compiled;
        self.owners = owners;
        self.directions = directions;
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

    /// The rules in force, as data: the field's `fix:directions` with each
    /// code resolved to the set's value and only the patterns that
    /// compiled, or the defaults where the field carries none.
    #[must_use]
    pub fn directions(&self) -> &[FixDirection] {
        &self.directions
    }

    /// The code one spelling names: its value, or the name the set gives it,
    /// ASCII case folded; `None` where the set holds no such code.
    ///
    /// The one resolution a spelling from outside goes through to become a
    /// code of the set, so a pin, a stated column, a rule's code and the
    /// setter that admits one resolve exactly alike.
    #[must_use]
    pub fn code(&self, spelling: &str) -> Option<&str> {
        resolve(&self.field.as_fix(), spelling)
    }

    /// Reads which way one captured byte line moved.
    ///
    /// The rules are applied **in front of the payload**, never inside it.
    /// Where a message starts is where the transport's own prose stops, so a
    /// `sent` inside a FIX `Text(58)`, a bridge value spelled `OUT=1`, an XML
    /// payload's own wording, or the `send-test-request` a configuration
    /// document names never becomes a direction.
    ///
    /// A prefix two codes match, and one no code matches, both answer
    /// nothing: there is no code the reading can prefer, and inventing one
    /// would be a guess. A document states nothing of itself; the prose in
    /// front of it does, `Response:` and `Request:` under the defaults.
    #[must_use]
    pub fn read_bytes(&self, line: &[u8]) -> Option<&str> {
        let bound = crate::mime_type::line::payload_at(line).unwrap_or(line.len());
        self.read_prefix(&line[..bound])
    }

    /// Reads which way one captured text line moved.
    #[must_use]
    pub fn read_text(&self, line: &str) -> Option<&str> {
        self.read_bytes(line.as_bytes())
    }

    /// The reading, over a prefix the caller has already bounded.
    ///
    /// A reader that located the frame to parse it hands the prose in front
    /// of it here, so the frame is located once. Every compiled pattern is
    /// applied to it, allocating nothing, and the rules the matches belong
    /// to decide: one names its code, two stop the reading.
    #[must_use]
    pub(super) fn read_prefix(&self, prefix: &[u8]) -> Option<&str> {
        let mut named: Option<usize> = None;
        for (index, rule) in self.rules.iter().enumerate() {
            if !rule.is_match(prefix) {
                continue;
            }
            let owner = self.owners[index];
            match named {
                Some(held) if held != owner => return None,
                _ => named = Some(owner),
            }
        }
        named.map(|owner| self.directions[owner].code())
    }
}

/// The code one spelling names in one field's set: its value, or the name
/// the set gives it, ASCII case folded; else the specification's two halves
/// under the codes the set gives them, `S` and `R` where it names neither.
///
/// The one place a spelling from outside becomes a code of the set: the
/// reading resolves a pin, a stated column and a rule's code through it,
/// and [`set_directions`](crate::FixFieldMut::set_directions) admits a
/// rule's code through it, so what the door lets in is what the reading
/// answers.
pub(super) fn resolve<'field>(field: &FixField<'field>, spelling: &str) -> Option<&'field str> {
    let spelling = spelling.trim();
    if spelling.is_empty() {
        return None;
    }
    if let Some(value) = field.code_value(spelling) {
        return Some(value);
    }
    let sent = field.code_value(SEND_NAME).unwrap_or(SEND_CODE);
    let recv = field.code_value(RECEIVE_NAME).unwrap_or(RECEIVE_CODE);
    [(sent, SEND_NAME), (recv, RECEIVE_NAME)]
        .into_iter()
        .find(|(value, name)| {
            value.eq_ignore_ascii_case(spelling) || name.eq_ignore_ascii_case(spelling)
        })
        .map(|(value, _)| value)
}

/// Warns about one rule the reading dropped, in the setter's words.
fn warned(error: &crate::Error) {
    log::warn!("tag {} fix:directions: {error}", MSGDIRECTION_TAG_NAME.0);
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
