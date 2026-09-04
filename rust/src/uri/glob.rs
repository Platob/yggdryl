//! Glob detection, decomposition, and URL matching.

use super::pattern::{matches_segment, matches_segments};
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
}
