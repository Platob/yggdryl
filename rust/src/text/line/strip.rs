//! How a message's edges are trimmed - span arithmetic, never an allocation.
//!
//! A strip narrows a range. It never builds a new string, which is what keeps
//! the message a borrow of the reader's window under every setting.

/// What a message's leading or trailing edge is trimmed of.
///
/// Independent on each side, so "keep the stack trace's leading indentation but
/// drop the trailing newline padding" is expressible.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Strip {
    /// Nothing is stripped: the message keeps its edge exactly.
    None,
    /// Unicode whitespace, as `str::trim` defines it. The default, and what the
    /// line projection has always done.
    #[default]
    Whitespace,
    /// ASCII whitespace only, which is the faster and more predictable rule for
    /// machine-written text.
    Ascii,
    /// Exactly these characters, and nothing else.
    Characters(smol_str::SmolStr),
}

impl Strip {
    /// Narrow `text`'s leading edge, returning how many bytes were dropped.
    #[must_use]
    pub fn lead(&self, text: &str) -> usize {
        match self {
            Self::None => 0,
            Self::Whitespace => text.len() - text.trim_start().len(),
            Self::Ascii => text.len() - text.trim_ascii_start().len(),
            Self::Characters(set) => {
                text.len()
                    - text
                        .trim_start_matches(|character| set.contains(character))
                        .len()
            }
        }
    }

    /// Narrow `text`'s trailing edge, returning how many bytes were dropped.
    #[must_use]
    pub fn trail(&self, text: &str) -> usize {
        match self {
            Self::None => 0,
            Self::Whitespace => text.len() - text.trim_end().len(),
            Self::Ascii => text.len() - text.trim_ascii_end().len(),
            Self::Characters(set) => {
                text.len()
                    - text
                        .trim_end_matches(|character| set.contains(character))
                        .len()
            }
        }
    }

    /// Return whether this strips nothing, so a caller can skip the work.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl std::fmt::Display for Strip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Whitespace => formatter.write_str("whitespace"),
            Self::Ascii => formatter.write_str("ascii"),
            // Prefixed rather than quoted, so the spelling round-trips
            // through `from_str` and cannot be mistaken for a mode name.
            Self::Characters(set) => write!(formatter, "chars:{set}"),
        }
    }
}

impl std::str::FromStr for Strip {
    type Err = crate::Error;

    /// Read a strip mode, or the exact character set to strip.
    ///
    /// `none`, `whitespace`, and `ascii` name the modes; `chars:` followed by
    /// the literal set names exactly those characters, so `chars: \t` strips a
    /// space and a tab. The prefix is what keeps a set of characters from being
    /// mistaken for a mode name, and it is what makes the spelling round-trip
    /// through [`fmt::Display`](std::fmt::Display).
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted spellings for anything else, and
    /// for an empty character set - which would strip nothing and is better
    /// spelled `none`.
    fn from_str(value: &str) -> crate::Result<Self> {
        Ok(match value {
            "none" => Self::None,
            "whitespace" => Self::Whitespace,
            "ascii" => Self::Ascii,
            other => {
                let set = other.strip_prefix("chars:").ok_or_else(|| {
                    crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.strip"),
                        reason: crate::text::expected_got(
                            "\"none\", \"whitespace\", \"ascii\", or \"chars:\" and the set to strip",
                            smol_str::format_smolstr!("{other:?}"),
                        ),
                    }
                })?;
                if set.is_empty() {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.strip"),
                        reason: crate::text::expected_got(
                            "a non-empty character set, or \"none\" to strip nothing",
                            "an empty one",
                        ),
                    });
                }
                Self::Characters(smol_str::SmolStr::new(set))
            }
        })
    }
}
