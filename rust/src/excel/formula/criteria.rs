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
    Char(char),
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

impl Wildcard {
    /// The pattern `pattern` spells, telling case apart when `match_case`,
    /// matching only the whole of a text when `entire`.
    pub(crate) fn new(pattern: &str, match_case: bool, entire: bool) -> Self {
        let mut pieces = Vec::with_capacity(pattern.len());
        let mut chars = pattern.chars();
        while let Some(character) = chars.next() {
            pieces.push(match character {
                '~' => Piece::Char(chars.next().unwrap_or('~')),
                '*' => Piece::Any,
                '?' => Piece::One,
                other => Piece::Char(other),
            });
        }
        Self {
            pieces,
            match_case,
            entire,
        }
    }

    fn same(&self, first: char, second: char) -> bool {
        first == second || (!self.match_case && first.to_lowercase().eq(second.to_lowercase()))
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
    pub(crate) fn find(&self, text: &[char], from: usize) -> Option<(usize, usize)> {
        if from > text.len() || (self.entire && from > 0) {
            return None;
        }
        let states = self.pieces.len() + 1;
        let mut reach: Vec<Option<usize>> = vec![None; states];
        let mut next: Vec<Option<usize>> = vec![None; states];
        let mut found: Option<(usize, usize)> = None;
        for at in from..=text.len() {
            // A match may start here, until one is found: a later start is
            // never the leftmost.
            if found.is_none() && (!self.entire || at == 0) {
                reach[0] = Some(reach[0].map_or(at, |held| held.min(at)));
            }
            self.close(&mut reach);
            if let Some(start) = reach[states - 1] {
                if !self.entire || at == text.len() {
                    found = match found {
                        Some((first, _)) if first < start => found,
                        _ => Some((start, at)),
                    };
                }
            }
            if let Some((first, _)) = found {
                // Only the leftmost start can still grow the match.
                for held in &mut reach {
                    if held.is_some_and(|start| start > first) {
                        *held = None;
                    }
                }
            }
            let Some(&character) = text.get(at) else {
                break;
            };
            next.fill(None);
            let mut alive = false;
            for (state, piece) in self.pieces.iter().enumerate() {
                let Some(start) = reach[state] else {
                    continue;
                };
                let target = match piece {
                    Piece::Any => state,
                    Piece::One => state + 1,
                    Piece::Char(wanted) if self.same(character, *wanted) => state + 1,
                    Piece::Char(_) => continue,
                };
                let held = &mut next[target];
                *held = Some(held.map_or(start, |held| held.min(start)));
                alive = true;
            }
            std::mem::swap(&mut reach, &mut next);
            if !alive && (found.is_some() || self.entire) {
                break;
            }
        }
        found
    }

    /// Whether the pattern matches `text`: anywhere in it, or the whole of
    /// it for an entire-cell pattern.
    pub(crate) fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        self.find(&chars, 0).is_some()
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
        while at <= chars.len() {
            let Some((start, end)) = self.find(&chars, at) else {
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/formula/criteria.rs` pins and a caller cannot
    //! reach: the wildcard matcher Find, Replace and the criteria functions
    //! share.

    use super::Wildcard;

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
