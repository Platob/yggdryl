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
            Self::Characters(set) => write!(formatter, "{set:?}"),
        }
    }
}

impl std::str::FromStr for Strip {
    type Err = crate::Error;

    /// Read a strip mode, or the exact character set to strip.
    ///
    /// `none`, `whitespace`, and `ascii` name the modes; anything else is the
    /// literal set of characters to strip, so `" \t"` means those two.
    fn from_str(value: &str) -> crate::Result<Self> {
        Ok(match value {
            "none" => Self::None,
            "whitespace" => Self::Whitespace,
            "ascii" => Self::Ascii,
            other => Self::Characters(smol_str::SmolStr::new(other)),
        })
    }
}
