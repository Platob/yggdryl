//! The allocation-bounded glob pattern engine.

/// Match path segments against pattern segments, honouring `**`.
/// Return whether `text` matches `pattern` under the same `.gitignore` rule.
///
/// The rule a listing uses, applied to plain text: a pattern with no separator
/// speaks about the last segment at any depth, and a pattern with one is
/// anchored at the start. Sharing the walk with [`crate::Url::matches_glob`] is what
/// makes `&holder.url glob '**/*.parquet'` and a folder listing agree - the
/// expression layer has no glob of its own.
pub(crate) fn matches_glob_text(text: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let segments: Vec<&str> = text.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() == 1 {
        return segments
            .last()
            .is_some_and(|name| matches_segment(name, parts[0]));
    }
    matches_segments(&segments, &parts)
}

pub(super) fn matches_segments(segments: &[&str], pattern: &[&str]) -> bool {
    match pattern.split_first() {
        // An exhausted pattern matches an exhausted path.
        None => segments.is_empty(),
        Some((&"**", rest)) => {
            // `**` matches any number of segments, including none.
            (0..=segments.len()).any(|skip| matches_segments(&segments[skip..], rest))
        }
        Some((part, rest)) => match segments.split_first() {
            Some((segment, remaining)) if matches_segment(segment, part) => {
                matches_segments(remaining, rest)
            }
            _ => false,
        },
    }
}

/// One step of a segment pattern.
enum Step {
    /// `*`: any run of characters, including none.
    Run,
    /// `?`: exactly one character.
    One,
    /// A literal character.
    Literal(char),
    /// `[a-z0]` or `[!a-z]`: one character from a set.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl Step {
    /// Return whether this single-character step accepts `character`.
    fn accepts(&self, character: char) -> bool {
        match self {
            Self::Run => true,
            Self::One => true,
            Self::Literal(expected) => *expected == character,
            Self::Class { negated, ranges } => {
                let inside = ranges
                    .iter()
                    .any(|(low, high)| *low <= character && character <= *high);
                inside != *negated
            }
        }
    }
}

/// Read a segment pattern as steps, so matching never rescans the syntax.
fn parse_steps(pattern: &str) -> Vec<Step> {
    let mut characters = pattern.chars().peekable();
    let mut steps = Vec::new();
    while let Some(character) = characters.next() {
        match character {
            '*' => steps.push(Step::Run),
            '?' => steps.push(Step::One),
            '[' => {
                let negated = matches!(characters.peek(), Some('!' | '^'));
                if negated {
                    characters.next();
                }
                let mut ranges = Vec::new();
                let mut closed = false;
                while let Some(low) = characters.next() {
                    if low == ']' && !ranges.is_empty() {
                        closed = true;
                        break;
                    }
                    // `a-z` is a range; a bare `-` at the end is a literal.
                    if characters.peek() == Some(&'-') {
                        characters.next();
                        match characters.peek() {
                            Some(&']') | None => ranges.push((low, low)),
                            Some(&high) => {
                                characters.next();
                                ranges.push((low, high));
                                continue;
                            }
                        }
                        ranges.push(('-', '-'));
                        continue;
                    }
                    ranges.push((low, low));
                }
                if closed {
                    steps.push(Step::Class { negated, ranges });
                } else {
                    // An unterminated class is a literal bracket, not an error.
                    steps.push(Step::Literal('['));
                    steps.extend(pattern_tail(&ranges, negated));
                }
            }
            other => steps.push(Step::Literal(other)),
        }
    }
    steps
}

/// Rebuild an unterminated character class as the literals it was written as.
fn pattern_tail(ranges: &[(char, char)], negated: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if negated {
        steps.push(Step::Literal('!'));
    }
    for (low, high) in ranges {
        steps.push(Step::Literal(*low));
        if low != high {
            steps.push(Step::Literal('-'));
            steps.push(Step::Literal(*high));
        }
    }
    steps
}

/// Match one segment against one pattern segment.
pub(super) fn matches_segment(segment: &str, pattern: &str) -> bool {
    let text: Vec<char> = segment.chars().collect();
    let steps = parse_steps(pattern);
    // `table[position]` is whether the steps seen so far match `text[..position]`.
    let mut table = vec![false; text.len() + 1];
    table[0] = true;

    for step in &steps {
        if matches!(step, Step::Run) {
            // A star extends every earlier match to the end of the segment.
            for position in 1..=text.len() {
                table[position] = table[position] || table[position - 1];
            }
            continue;
        }
        // Any other step consumes exactly one character, so the row is rebuilt
        // from the right to keep the previous row readable while it is used.
        for position in (1..=text.len()).rev() {
            table[position] = table[position - 1] && step.accepts(text[position - 1]);
        }
        table[0] = false;
    }

    table[text.len()]
}

#[cfg(test)]
mod tests;
