//! Glob patterns and Hive partitions, read off a location.
//!
//! Two conventions turn a path into a query. A glob describes *which* locations
//! a caller means (`data/**/*.parquet`), and a Hive partition describes *what a
//! location holds* (`year=2024/month=01`). Both are read from the URL itself,
//! because every child of every backend has one - so a listing, a filter, and a
//! partition-aware read all work the same way over a local folder or a bucket.

use crate::Url;

/// The characters that make a path segment a pattern rather than a name.
///
/// Only `*` survives URL parsing: `?` opens the query and `[` is reserved for
/// an IPv6 host, so a location that *is* a glob spells it with stars, while the
/// full syntax is available to pattern text passed to [`Url::matches_glob`].
const GLOB_CHARACTERS: [char; 3] = ['*', '?', '['];

impl Url {
    /// Return whether this location is a glob pattern rather than one name.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert!(Url::from_str("file:///data/**/*.parquet")?.is_glob());
    /// assert!(Url::from_str("file:///data/part-*.arrows")?.is_glob());
    /// assert!(!Url::from_str("file:///data/trades.arrows")?.is_glob());
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_glob(&self) -> bool {
        self.path_segments()
            .any(|segment| segment.contains(GLOB_CHARACTERS))
    }

    /// Return whether `text` is a pattern rather than a plain name.
    ///
    /// This is what a walk asks of each pattern segment to decide whether it
    /// can descend into it directly or has to list and filter.
    pub fn is_pattern(text: &str) -> bool {
        text.contains(GLOB_CHARACTERS)
    }

    /// Split a glob into the fixed location it starts from and its pattern.
    ///
    /// The root is every leading segment with no pattern character, which is
    /// the deepest place a listing can start; the pattern is the rest. A
    /// location that is not a glob is its own root with no pattern.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (root, pattern) = Url::from_str("file:///data/year=2024/**/*.parquet")?.glob_parts()?;
    ///
    /// assert_eq!(root.to_string(), "file:///data/year=2024");
    /// assert_eq!(pattern.as_deref(), Some("**/*.parquet"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the fixed prefix cannot be rebuilt as a URL.
    pub fn glob_parts(&self) -> crate::Result<(Self, Option<String>)> {
        let segments: Vec<&str> = self.path_segments().collect();
        let split = segments
            .iter()
            .position(|segment| segment.contains(GLOB_CHARACTERS));
        let Some(split) = split else {
            return Ok((self.clone(), None));
        };

        let mut root = self.clone();
        // Walk up to the deepest fixed segment.
        for _ in split..segments.len() {
            root = root.parent().unwrap_or_else(|| root.clone());
        }
        Ok((root, Some(segments[split..].join("/"))))
    }

    /// Return whether the pattern crosses directory boundaries.
    pub fn is_recursive_glob(&self) -> bool {
        self.path_segments().any(|segment| segment == "**")
    }

    /// Return whether this location matches `pattern`.
    ///
    /// The rule is the one `.gitignore` uses, because it is the one people
    /// already have: a pattern with no separator matches the *name* at any
    /// depth, and a pattern with a separator is anchored at the path root.
    /// `**` stands for any number of segments, `*` and `?` stay inside one.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/trades/year=2024/part-0.parquet")?;
    ///
    /// assert!(url.matches_glob("*.parquet"));
    /// assert!(url.matches_glob("lake/**/part-?.parquet"));
    /// assert!(!url.matches_glob("lake/*.parquet"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn matches_glob(&self, pattern: &str) -> bool {
        let parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() == 1 {
            // No separator, so the pattern speaks about the name alone.
            return self
                .file_name()
                .is_some_and(|name| matches_segment(name, parts[0]));
        }
        let segments: Vec<&str> = self.path_segments().collect();
        matches_segments(&segments, &parts)
    }

    /// Return whether this location matches `pattern` relative to `root`.
    ///
    /// This is what a listing filters with: the pattern came from
    /// [`Url::glob_parts`], so it is written relative to the root it was split
    /// from, and it must be anchored there rather than at the path root.
    /// A location outside `root` never matches.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = Url::from_str("file:///lake/trades")?;
    /// let url = Url::from_str("file:///lake/trades/year=2024/part-0.parquet")?;
    ///
    /// assert!(url.matches_glob_under(&root, "**/*.parquet"));
    /// assert!(url.matches_glob_under(&root, "year=2024/*.parquet"));
    /// assert!(!url.matches_glob_under(&root, "*.parquet"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn matches_glob_under(&self, root: &Self, pattern: &str) -> bool {
        let Some(segments) = self.segments_under(root) else {
            return false;
        };
        let parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
        matches_segments(&segments, &parts)
    }

    /// Return this location's path segments below `root`, when it is below it.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = Url::from_str("file:///lake/trades")?;
    /// let url = Url::from_str("file:///lake/trades/year=2024/part-0.parquet")?;
    ///
    /// assert_eq!(url.segments_under(&root), Some(vec!["year=2024", "part-0.parquet"]));
    /// assert_eq!(url.segments_under(&url), Some(Vec::new()));
    /// assert_eq!(root.segments_under(&url), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn segments_under(&self, root: &Self) -> Option<Vec<&str>> {
        if self.scheme() != root.scheme() || self.authority() != root.authority() {
            return None;
        }
        let mut segments = self.path_segments();
        for expected in root.path_segments() {
            if segments.next()? != expected {
                return None;
            }
        }
        Some(segments.collect())
    }

    /// Read the Hive partition pairs a location's path spells out.
    ///
    /// A Hive layout writes one directory per partition column, named
    /// `column=value`. Reading them back is what lets a partitioned read
    /// restore the columns the directory names replaced.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/trades/year=2024/month=01/part-0.parquet")?;
    ///
    /// assert_eq!(
    ///     url.hive_partitions(),
    ///     vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
    /// );
    /// assert!(url.is_hive_partitioned());
    /// # Ok(())
    /// # }
    /// ```
    pub fn hive_partitions(&self) -> Vec<(String, String)> {
        self.path_segments().filter_map(hive_pair).collect()
    }

    /// Read the Hive partition pairs this location spells out *below* `root`.
    ///
    /// A lake is addressed at some level, and only the directories below that
    /// level are its partition columns: `year` is a partition of `/lake` but a
    /// fixed part of the address `/lake/year=2024`. A location outside `root`
    /// spells out nothing.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/year=2024/month=01/part-0.parquet")?;
    ///
    /// assert_eq!(
    ///     url.hive_partitions_under(&Url::from_str("file:///lake")?),
    ///     vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
    /// );
    /// assert_eq!(
    ///     url.hive_partitions_under(&Url::from_str("file:///lake/year=2024")?),
    ///     vec![("month".to_owned(), "01".to_owned())]
    /// );
    /// assert!(url.hive_partitions_under(&Url::from_str("file:///elsewhere")?).is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn hive_partitions_under(&self, root: &Self) -> Vec<(String, String)> {
        self.segments_under(root)
            .map(|segments| segments.into_iter().filter_map(hive_pair).collect())
            .unwrap_or_default()
    }

    /// Return whether any path segment is a Hive partition directory.
    pub fn is_hive_partitioned(&self) -> bool {
        !self.hive_partitions().is_empty()
    }

    /// Return the value of one Hive partition column, when the path carries it.
    pub fn hive_partition(&self, column: &str) -> Option<String> {
        self.hive_partitions()
            .into_iter()
            .find_map(|(key, value)| (key == column).then_some(value))
    }

    /// Extend this location with one Hive partition directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the column or value cannot form a path segment.
    pub fn with_hive_partition(&self, column: &str, value: &str) -> crate::Result<Self> {
        self.joinpath(&format!("{column}={value}"))
    }
}

/// Read the `column=value` pair one path segment spells out.
fn hive_pair(segment: &str) -> Option<(String, String)> {
    let (key, value) = segment.split_once('=')?;
    // A key must look like a column name, not an escaped byte.
    (!key.is_empty() && !key.contains(' ')).then(|| (key.to_owned(), value.to_owned()))
}

/// Match path segments against pattern segments, honouring `**`.
/// Return whether `text` matches `pattern` under the same `.gitignore` rule.
///
/// The rule a listing uses, applied to plain text: a pattern with no separator
/// speaks about the last segment at any depth, and a pattern with one is
/// anchored at the start. Sharing the walk with [`Url::matches_glob`] is what
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

fn matches_segments(segments: &[&str], pattern: &[&str]) -> bool {
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
fn matches_segment(segment: &str, pattern: &str) -> bool {
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
