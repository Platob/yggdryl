//! Excel's wildcard patterns: `*` any run of characters, `?` one character,
//! `~` taking the character after it as it stands (`~*`, `~?`, `~~`) - what
//! Find and Replace read, and what the criteria of `COUNTIF`, `SUMIF` and
//! their kin read.
//!
//! A pattern is matched in one pass over the text: each position of the
//! pattern a prefix of the text reaches is kept with the leftmost start
//! reaching it, so finding the leftmost match - and the longest from there -
//! costs the text's length times the pattern's, never a restart per
//! position. A 32,767-character cell is read once per match.

/// One piece of a pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Piece {
    Char(u32),
    One,
    Any,
}

/// A pattern read once: its pieces, how it compares, and whether it must
/// match the whole text.
#[derive(Clone, Debug)]
pub(crate) struct Wildcard {
    pieces: Vec<Piece>,
    match_case: bool,
    entire: bool,
}

/// One transition owner, borrowed by row matchers or retained by SEARCH.
/// Capacity is the largest compiled pattern encountered by this scratch.
#[derive(Debug, Default)]
struct MatchState {
    reach: Vec<Option<usize>>,
    next: Vec<Option<usize>>,
}

/// Reusable transition state for one borrowed compiled wildcard pattern.
pub(crate) struct Matcher<'a> {
    wildcard: &'a Wildcard,
    state: MatchState,
}

impl Matcher<'_> {
    pub(crate) fn find_iter(&mut self, text: impl Iterator<Item = char>, from: usize) -> Option<(usize,usize)> {
        self.state.find(self.wildcard, text.map(u32::from), from, false)
    }

    pub(crate) fn is_match(&mut self, text: &str) -> bool {
        self.find_iter(text.chars(), 0).is_some()
    }
}

impl MatchState {
    /// The leftmost match at or after `from`, longest at that start. `text`
    /// begins at `from`; the caller has already checked that start's bound.
    fn find(
        &mut self,
        wildcard: &Wildcard,
        text: impl Iterator<Item = u32>,
        from: usize,
        scalar_starts: bool,
    ) -> Option<(usize, usize)> {
        if wildcard.entire && from > 0 {
            return None;
        }
        let states = wildcard.pieces.len() + 1;
        self.reach.resize(states, None);
        self.next.resize(states, None);
        self.reach.fill(None);
        self.next.fill(None);
        let mut chars = text.peekable();
        let mut found: Option<(usize, usize)> = None;
        let mut at = from;
        loop {
            if found.is_none() && (!wildcard.entire || at == 0)
                && (!scalar_starts || chars.peek().is_none_or(|unit| !(0xdc00..=0xdfff).contains(unit))) {
                self.reach[0] = Some(self.reach[0].map_or(at, |held| held.min(at)));
            }
            wildcard.close(&mut self.reach);
            if let Some(start) = self.reach[states - 1] {
                if !wildcard.entire || chars.peek().is_none() {
                    found = match found {
                        Some((first, _)) if first < start => found,
                        _ => Some((start, at)),
                    };
                }
            }
            if let Some((first, _)) = found {
                for held in &mut self.reach {
                    if held.is_some_and(|start| start > first) {
                        *held = None;
                    }
                }
            }
            let Some(character) = chars.next() else {
                break;
            };
            self.next.fill(None);
            let mut alive = false;
            for (state, piece) in wildcard.pieces.iter().enumerate() {
                let Some(start) = self.reach[state] else {
                    continue;
                };
                let target = match piece {
                    Piece::Any => state,
                    Piece::One => state + 1,
                    Piece::Char(wanted) if wildcard.same(character, *wanted) => state + 1,
                    Piece::Char(_) => continue,
                };
                let held = &mut self.next[target];
                *held = Some(held.map_or(start, |held| held.min(start)));
                alive = true;
            }
            std::mem::swap(&mut self.reach, &mut self.next);
            if !alive && (found.is_some() || wildcard.entire) {
                break;
            }
            at += 1;
        }
        found
    }

}

/// One cached SEARCH pattern and reusable transitions. Distinct cell patterns
/// replace this key; no workbook-wide cache or source text is retained.
#[derive(Debug)]
pub(crate) struct Search {
    pattern: Option<crate::Str>,
    wildcard: Wildcard,
    state: MatchState,
}

impl Default for Search {
    fn default() -> Self {
        Self { pattern: None, wildcard: Wildcard::new("", false, false), state: MatchState::default() }
    }
}

impl Search {
    pub(crate) fn find(&mut self, pattern: &crate::Str, text: &str, from: usize, scalar_starts: bool) -> Option<(usize,usize)> {
        if self.pattern.as_ref() != Some(pattern) {
            self.wildcard.compile(pattern.as_str(), true, false);
            self.pattern = Some(pattern.clone());
        }
        self.state.find(&self.wildcard, text.encode_utf16().skip(from).map(u32::from), from, scalar_starts)
    }
}

impl Wildcard {
    /// Borrow this pattern with transition buffers reusable across rows.
    pub(crate) fn matcher(&self) -> Matcher<'_> {
        Matcher { wildcard: self, state: MatchState::default() }
    }

    /// The pattern `pattern` spells, telling case apart when `match_case`,
    /// matching only the whole of a text when `entire`.
    pub(crate) fn new(pattern: &str, match_case: bool, entire: bool) -> Self {
        let mut result = Self { pieces: Vec::with_capacity(pattern.len()), match_case, entire };
        result.compile(pattern, false, false);
        result
    }

    /// The same wildcard grammar emits scalar or UTF-16 literal units. SEARCH
    /// ignores a final unescaped tilde; ordinary Find retains it literally.
    fn compile(&mut self, pattern: &str, utf16: bool, criterion: bool) {
        self.pieces.clear();
        // Excel compares a criterion with no wildcard operator as literal
        // text, including every tilde. Its wildcard path treats a terminal
        // tilde as absent. SEARCH/FIND retain their separate proven intake.
        let wildcard_criterion = criterion && pattern.contains(['*', '?']);
        let mut chars = pattern.chars().peekable();
        while let Some(character) = chars.next() {
            let literal = match character {
                '~' if criterion && !wildcard_criterion => '~',
                '~' if criterion => match chars.peek().copied() {
                    Some('*' | '?' | '~') => chars.next().expect("peeked next character"),
                    Some(_) => '~',
                    None => break,
                },
                '~' => match chars.next() { Some(next) => next, None if utf16 => break, None => '~' },
                '*' => { self.pieces.push(Piece::Any); continue; }
                '?' => { self.pieces.push(Piece::One); continue; }
                other => other,
            };
            if utf16 {
                let mut buffer = [0; 2];
                for unit in literal.encode_utf16(&mut buffer) { self.pieces.push(Piece::Char(u32::from(*unit))); }
            } else { self.pieces.push(Piece::Char(u32::from(literal))); }
        }
    }

    /// Compile a criterion through the same transition engine. Tildes are
    /// literal unless the pattern enters Excel's wildcard path.
    fn criterion(pattern: &str) -> Self {
        let mut result = Self {
            pieces: Vec::with_capacity(pattern.len()),
            match_case: false,
            entire: true,
        };
        result.compile(pattern, false, true);
        result
    }

    fn same(&self, first: u32, second: u32) -> bool {
        first == second || (!self.match_case && match (char::from_u32(first), char::from_u32(second)) {
            (Some(first),Some(second)) => first.to_lowercase().eq(second.to_lowercase()),
            _ => false,
        })
    }

    /// Each state `reach` holds - a count of the pieces matched, with the
    /// leftmost start reaching it - carried through the `*` pieces that
    /// may match nothing.
    fn close(&self, reach: &mut [Option<usize>]) {
        for (at, piece) in self.pieces.iter().enumerate() {
            if *piece == Piece::Any {
                if let Some(start) = reach[at] {
                    let next = &mut reach[at + 1];
                    *next = Some(next.map_or(start, |held| held.min(start)));
                }
            }
        }
    }

    /// The leftmost match in `text` starting at `from` or later, and the
    /// longest from that start: its start and its end, in characters.
    #[cfg(feature = "internals")]
    pub(crate) fn find(&self, text: &[char], from: usize) -> Option<(usize, usize)> {
        if from > text.len() { return None; }
        self.matcher().find_iter(text[from..].iter().copied(), from)
    }

    /// Whether the pattern matches `text`: anywhere in it, or the whole of
    /// it for an entire-cell pattern.
    pub(crate) fn is_match(&self, text: &str) -> bool {
        self.matcher().is_match(text)
    }

    /// `text` with every match replaced by `replacement`, left to right and
    /// none overlapping, each the longest from its start - an empty match
    /// right where another ended being none, as Replace All reads `*`;
    /// `None` when nothing matches.
    pub(crate) fn replace(&self, text: &str, replacement: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut replaced = String::with_capacity(text.len());
        let mut at = 0;
        let mut matched = false;
        let mut matcher = self.matcher();
        while at <= chars.len() {
            let Some((start, end)) = matcher.find_iter(chars[at..].iter().copied(), at) else {
                break;
            };
            if matched && start == end && start == at {
                // Nothing between this match and the one before.
                match chars.get(at) {
                    Some(character) => {
                        replaced.push(*character);
                        at += 1;
                        continue;
                    }
                    None => break,
                }
            }
            matched = true;
            replaced.extend(&chars[at..start]);
            replaced.push_str(replacement);
            if end == start {
                // An empty match moves one character on.
                if let Some(character) = chars.get(start) {
                    replaced.push(*character);
                }
                at = start + 1;
            } else {
                at = end;
            }
            if self.entire {
                break;
            }
        }
        if !matched {
            return None;
        }
        if at < chars.len() {
            replaced.extend(&chars[at..]);
        }
        Some(replaced)
    }
}

use super::number;
use super::value::Operand;
use crate::excel::cell::ExcelError;
use crate::excel::entry;
use crate::expression::Comparison;

/// One resolved criterion. Excel applies operator-specific type rules: a
/// numeric equality sees numeric text, but numeric inequality and ordering
/// retain the source kind. A formula error in the criterion is data here.
pub(crate) struct Criterion {
    relation: Comparison,
    value: CriterionValue,
}

enum CriterionValue {
    Blank,
    Number(f64),
    Boolean(bool),
    Error(ExcelError),
    Text(Wildcard),
}

/// Formula-local transition scratch; one NFA state allocation per criterion,
/// then no wildcard allocation per source row.
pub(crate) struct CriterionMatcher<'a> {
    criterion: &'a Criterion,
    wildcard: Option<Matcher<'a>>,
}

impl Criterion {
    /// Resolve the scalar criterion once. Unsupported relational text is held
    /// until its locale collation has a native contract.
    pub(crate) fn new(value: Operand) -> Option<Self> {
        match value {
            Operand::Blank => Some(Self { relation: Comparison::Eq, value: CriterionValue::Blank }),
            Operand::Number(value) if value.is_finite() => Some(Self {
                relation: Comparison::Eq, value: CriterionValue::Number(value),
            }),
            Operand::Boolean(value) => Some(Self {
                relation: Comparison::Eq, value: CriterionValue::Boolean(value),
            }),
            Operand::Error(value) => Some(Self {
                relation: Comparison::Eq, value: CriterionValue::Error(value),
            }),
            Operand::Text(value) => {
                let text = value.as_str();
                let (relation, rest) = if let Some(rest) = text.strip_prefix("<>") {
                    (Comparison::NotEq, rest)
                } else if let Some(rest) = text.strip_prefix("<=") {
                    (Comparison::LtEq, rest)
                } else if let Some(rest) = text.strip_prefix(">=") {
                    (Comparison::GtEq, rest)
                } else if let Some(rest) = text.strip_prefix('<') {
                    (Comparison::Lt, rest)
                } else if let Some(rest) = text.strip_prefix('>') {
                    (Comparison::Gt, rest)
                } else if let Some(rest) = text.strip_prefix('=') {
                    (Comparison::Eq, rest)
                } else {
                    (Comparison::Eq, text)
                };
                let error = ExcelError::from_text(rest);
                let value = if rest.is_empty() {
                    if !matches!(relation, Comparison::Eq | Comparison::NotEq) { return None; }
                    CriterionValue::Blank
                } else if error != ExcelError::Unrecognized {
                    if !matches!(relation, Comparison::Eq | Comparison::NotEq) { return None; }
                    CriterionValue::Error(error)
                } else if let Some(number) = entry::number(rest) {
                    CriterionValue::Number(number.value)
                } else {
                    if !matches!(relation, Comparison::Eq | Comparison::NotEq) || !rest.is_ascii() {
                        return None;
                    }
                    CriterionValue::Text(Wildcard::criterion(rest))
                };
                Some(Self { relation, value })
            }
            Operand::Reference(_) => None,
            Operand::Number(_) => None,
        }
    }

    pub(crate) fn matcher(&self) -> CriterionMatcher<'_> {
        CriterionMatcher {
            criterion: self,
            wildcard: match &self.value {
                CriterionValue::Text(value) => Some(value.matcher()),
                _ => None,
            },
        }
    }
}

impl CriterionMatcher<'_> {
    /// None means an unproved source spelling, never a false match.
    pub(crate) fn matches(&mut self, candidate: &Operand) -> Option<bool> {
        use CriterionValue as V;
        let criterion = self.criterion;
        let relation = criterion.relation;
        match &criterion.value {
            V::Blank => Some(match relation {
                Comparison::Eq => matches!(candidate, Operand::Blank)
                    || matches!(candidate, Operand::Text(text) if text.as_str().is_empty()),
                Comparison::NotEq => !matches!(candidate, Operand::Blank),
                _ => return None,
            }),
            V::Number(number) => match relation {
                Comparison::Eq => match candidate {
                    Operand::Number(value) => number::compare(*value, *number).ok()
                        .map(|order| relation.answers(order)),
                    Operand::Text(text) => entry::number(text.as_str()).map_or(Some(false), |parsed| {
                        number::compare(parsed.value, *number).ok()
                            .map(|order| relation.answers(order))
                    }),
                    Operand::Reference(_) => None,
                    _ => Some(false),
                },
                Comparison::NotEq => match candidate {
                    Operand::Number(value) => number::compare(*value, *number).ok()
                        .map(|order| relation.answers(order)),
                    Operand::Reference(_) => None,
                    _ => Some(true),
                },
                _ => match candidate {
                    Operand::Number(value) => number::compare(*value, *number).ok()
                        .map(|order| relation.answers(order)),
                    Operand::Reference(_) => None,
                    _ => Some(false),
                },
            },
            V::Boolean(value) if relation == Comparison::Eq => Some(matches!(candidate,
                Operand::Boolean(actual) if actual == value)),
            V::Error(value) if matches!(relation, Comparison::Eq | Comparison::NotEq) => {
                let equal = matches!(candidate, Operand::Error(actual) if actual == value);
                Some(if relation == Comparison::Eq { equal } else { !equal })
            }
            V::Text(_) if matches!(relation, Comparison::Eq | Comparison::NotEq) => {
                let equal = match candidate {
                    Operand::Text(text) if text.as_str().is_ascii() =>
                        self.wildcard.as_mut().expect("text has compiled matcher")
                            .is_match(text.as_str()),
                    Operand::Text(_) | Operand::Reference(_) => return None,
                    _ => false,
                };
                Some(if relation == Comparison::Eq { equal } else { !equal })
            }
            _ => None,
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/formula/criteria.rs` pins and a caller cannot
    //! reach: the wildcard matcher Find, Replace and the criteria functions
    //! share.

    use super::Wildcard;


    /// Match through SEARCH's UTF-16 unit owner; returned offsets are units.
    #[must_use]
    pub fn search_utf16(pattern: &str, text: &str, from: usize, scalar_starts: bool) -> Option<(usize,usize)> {
        super::Search::default().find(&crate::Str::new(pattern), text, from, scalar_starts)
    }

    /// Count whole-text wildcard matches through one compiled pattern.
    #[must_use]
    pub fn count_whole_matches(pattern: &str, texts: &[&str]) -> usize {
        let wildcard = Wildcard::new(pattern, false, true);
        let mut matcher = wildcard.matcher();
        texts.iter().filter(|text| matcher.is_match(text)).count()
    }

    /// Count source texts through Criterion's own compiled text intake.
    /// The matcher and transition buffers are retained across source rows.
    #[must_use]
    pub fn count_text_criterion_matches(pattern: &str, texts: &[&str]) -> usize {
        let criterion = super::Criterion::new(super::Operand::Text(crate::Str::new(pattern)))
            .expect("text criterion");
        let mut matcher = criterion.matcher();
        let wildcard = matcher.wildcard.as_mut().expect("text criterion matcher");
        texts.iter().filter(|text| wildcard.is_match(text)).count()
    }

    /// The leftmost, longest match of `pattern` in `text` from character
    /// `from`: its start and end, in characters.
    #[must_use]
    pub fn find(
        pattern: &str,
        match_case: bool,
        entire: bool,
        text: &str,
        from: usize,
    ) -> Option<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        Wildcard::new(pattern, match_case, entire).find(&chars, from)
    }

    /// `text` with every match of `pattern` replaced by `replacement`.
    #[must_use]
    pub fn replace(
        pattern: &str,
        match_case: bool,
        entire: bool,
        text: &str,
        replacement: &str,
    ) -> Option<String> {
        Wildcard::new(pattern, match_case, entire).replace(text, replacement)
    }
}
