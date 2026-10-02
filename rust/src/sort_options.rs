//! SortOptions: the two facts an ordering states beside its key.
//!
//! An ordering is a key, a direction and where absent rows go. The key is
//! the caller's - a column, a term of the expression grammar - and the two
//! facts are this type, so a `Serie`, a `ChunkedSerie`, a `SerieSlice` and
//! the plan's `order by` key all spell them once: ascending with nulls last
//! unless stated, as the plan's `order by` key and DuckDB default to. That
//! is the opposite of Arrow's own default, which puts nulls first: a caller
//! moving from Arrow's sort with its default options states
//! `with_nulls_first(true)` to keep the order it had.
//!
//! ```
//! use yggdryl::SortOptions;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let options = SortOptions::descending().with_nulls_first(true);
//! assert!(options.is_descending() && options.is_nulls_first());
//! assert_eq!(options.to_string(), " desc nulls first");
//! assert_eq!(" desc nulls first".parse::<SortOptions>()?, options);
//! assert_eq!(SortOptions::default().to_string(), "");
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::str::FromStr;

/// The direction of an ordering and where its absent rows go.
///
/// `Copy`, and `Default` is ascending with nulls last - what the plan's
/// `order by` key and DuckDB default to, and the opposite of
/// `arrow_schema::SortOptions::default()`, which puts nulls first.
/// `Display` writes the suffix the
/// plan's ordering writes after its key - ` desc`, ` nulls first`, both, or
/// nothing for the default - and `FromStr` reads it back.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SortOptions {
    descending: bool,
    nulls_first: bool,
}

impl SortOptions {
    /// Ascending, nulls last: the default.
    pub const fn ascending() -> Self {
        Self {
            descending: false,
            nulls_first: false,
        }
    }

    /// Descending, nulls last.
    pub const fn descending() -> Self {
        Self {
            descending: true,
            nulls_first: false,
        }
    }

    /// The same direction, absent rows first when `nulls_first` and last
    /// otherwise.
    pub const fn with_nulls_first(mut self, nulls_first: bool) -> Self {
        self.nulls_first = nulls_first;
        self
    }

    /// Whether the order is descending.
    pub const fn is_descending(&self) -> bool {
        self.descending
    }

    /// Whether absent rows come first.
    pub const fn is_nulls_first(&self) -> bool {
        self.nulls_first
    }

    /// The same two facts as Arrow's sort kernels spell them; the default
    /// crosses as nulls last, never as Arrow's own default.
    pub(crate) const fn into_arrow(self) -> arrow_schema::SortOptions {
        arrow_schema::SortOptions {
            descending: self.descending,
            nulls_first: self.nulls_first,
        }
    }
}

impl fmt::Display for SortOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.descending {
            formatter.write_str(" desc")?;
        }
        if self.nulls_first {
            formatter.write_str(" nulls first")?;
        }
        Ok(())
    }
}

impl FromStr for SortOptions {
    type Err = crate::Error;

    /// Read the suffix an ordering writes after its key: `asc` or `desc`,
    /// then `nulls first` or `nulls last`, each optional, case folded,
    /// whitespace around and between them ignored; the empty text is the
    /// default.
    ///
    /// # Errors
    ///
    /// Returns an error naming the word that is neither a direction nor a
    /// nulls placement, or a word stated twice.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |reason: String| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.sort_options"),
            reason: smol_str::SmolStr::new(reason),
        };
        let mut options = Self::default();
        let (mut direction, mut placement) = (false, false);
        let mut words = text.split_whitespace();
        while let Some(word) = words.next() {
            let folded = word.to_ascii_lowercase();
            match folded.as_str() {
                "asc" | "desc" => {
                    if direction {
                        return Err(refuse(format!(
                            "the direction is stated twice, at {word:?}"
                        )));
                    }
                    direction = true;
                    options.descending = folded == "desc";
                }
                "nulls" => {
                    if placement {
                        return Err(refuse(format!(
                            "the nulls placement is stated twice, at {word:?}"
                        )));
                    }
                    placement = true;
                    options.nulls_first = match words.next().map(str::to_ascii_lowercase).as_deref()
                    {
                        Some("first") => true,
                        Some("last") => false,
                        other => {
                            return Err(refuse(format!(
                                "expected `nulls first` or `nulls last`, got `nulls {}`",
                                other.unwrap_or("")
                            )));
                        }
                    };
                }
                _ => {
                    return Err(refuse(format!(
                        "expected `asc`, `desc`, `nulls first` or `nulls last`, got {word:?}"
                    )));
                }
            }
        }
        Ok(options)
    }
}
