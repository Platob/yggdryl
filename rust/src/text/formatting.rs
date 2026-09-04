//! How a dump is laid out - the one formatting value all three formats share.
//!
//! Formatting changes *bytes*, never meaning. Parsing any formatting of the
//! same value yields an equal value, in every format, and dumping the same
//! value under the same formatting twice is byte-identical. That invariant is
//! the whole contract: a knob that quietly altered what round-trips would be
//! worse than no knob at all.
//!
//! The type is deliberately not called `Format` - [`crate::text::Format`]
//! already names the `Json`/`Yaml`/`Toml` enum, and a second type by that name
//! beside it would be genuinely confusing.
//!
//! ```
//! use yggdryl::Scalar;
//! use yggdryl::text::{Formatting, Indent};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let value = Scalar::from_record([("id", Scalar::U64(1))])?;
//!
//! // The zero-configuration path is unchanged output.
//! assert_eq!(yggdryl::text::json::into_bytes(&value)?, br#"{"id":1}"#);
//!
//! // Indented on request.
//! let pretty = yggdryl::text::json::into_bytes_with_formatting(&value, Formatting::indented(2))?;
//! assert_eq!(pretty, b"{\n  \"id\": 1\n}");
//!
//! // And parsing either spelling gives the same value back.
//! assert_eq!(yggdryl::text::json::from_utf8("{\n  \"id\": 1\n}")?, value);
//! # Ok(())
//! # }
//! ```

/// How many columns one nesting level is indented by, if any.
///
/// Three states, because "the format's own default" and "explicitly none" are
/// genuinely different requests: YAML's default is a two-space *block*, while
/// no indent at all means flow style.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Indent {
    /// The format's own default, and what every existing dump method uses.
    ///
    /// - **JSON**: compact - no whitespace between tokens.
    /// - **YAML**: block style, two spaces per level.
    /// - **TOML**: table bodies and array-of-table entries flush left.
    #[default]
    Default,
    /// No indentation at all.
    ///
    /// - **JSON**: compact, the same as [`Self::Default`].
    /// - **YAML**: *flow* style - `{a: 1, b: 2}` on one line, which is valid
    ///   YAML and round-trips. This is an explicitly requested opt-in and never
    ///   what a caller gets by accident; a schema dump's block style is the
    ///   default precisely so nobody has to ask for it.
    /// - **TOML**: table bodies flush left, the same as [`Self::Default`].
    None,
    /// This many spaces per nesting level.
    ///
    /// - **JSON**: pretty-printed, exactly as `json.dumps(indent=n)` reads.
    /// - **YAML**: block style with this width instead of two.
    /// - **TOML**: nested table bodies and array-of-table entries indented by
    ///   this much per level. TOML's whitespace is largely insignificant, so
    ///   this affects readability and nothing else - the parse is identical.
    Spaces(u8),
    /// One tab per nesting level, wherever the format's grammar permits it.
    ///
    /// TOML and YAML forbid tabs in some positions; where a tab is not legal
    /// the writer falls back to the format's default width rather than
    /// emitting a document that will not parse.
    Tabs,
}

impl Indent {
    /// The literal bytes one nesting level costs, or `None` for no indent.
    ///
    /// `Default` answers `None` here: each format resolves its own default
    /// before asking, so this reports only what was explicitly requested.
    #[must_use]
    pub const fn unit(self) -> Option<&'static [u8]> {
        match self {
            Self::Default | Self::None => None,
            Self::Tabs => Some(b"\t"),
            Self::Spaces(width) => Some(spaces(width)),
        }
    }

    /// Return whether this asks for no layout at all.
    #[must_use]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    /// Return whether this defers to the format's own default.
    #[must_use]
    pub const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }
}

/// Up to 16 spaces, sliced without allocating.
const SPACES: &[u8; 16] = b"                ";

/// `width` spaces, clamped to what one nesting level can sensibly cost.
const fn spaces(width: u8) -> &'static [u8] {
    let width = if width as usize > SPACES.len() {
        SPACES.len()
    } else {
        width as usize
    };
    SPACES.split_at(width).0
}

/// How a dump is laid out, shared by JSON, YAML, and TOML.
///
/// One options value rather than a naming cross-product: two orthogonal knobs
/// today become three tomorrow, and `dump_with_level_and_formatting` is not a
/// method name anyone should have to type. Every existing dump method delegates
/// here with [`Formatting::default`], so no existing output changes a byte
/// unless a caller asks.
///
/// ```
/// use yggdryl::text::{Formatting, Indent};
///
/// assert_eq!(Formatting::default().indent(), Indent::Default);
/// assert_eq!(Formatting::indented(4).indent(), Indent::Spaces(4));
/// assert_eq!(Formatting::compact().indent(), Indent::None);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Formatting {
    indent: Indent,
    level: crate::Level,
}

impl Formatting {
    /// The format's own default layout, and no content coding.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lay out nested structure with `width` spaces per level.
    #[must_use]
    pub fn indented(width: u8) -> Self {
        Self::new().with_indent(Indent::Spaces(width))
    }

    /// Ask for no layout: compact JSON, flow-style YAML, flat TOML.
    #[must_use]
    pub fn compact() -> Self {
        Self::new().with_indent(Indent::None)
    }

    /// Return the indentation this asks for.
    #[must_use]
    pub const fn indent(&self) -> Indent {
        self.indent
    }

    /// Set the indentation.
    pub const fn set_indent(&mut self, indent: Indent) {
        self.indent = indent;
    }

    /// Return this value with a different indentation.
    #[must_use]
    pub const fn with_indent(mut self, indent: Indent) -> Self {
        self.indent = indent;
        self
    }

    /// Return the content-coding level a redirected dump encodes at.
    ///
    /// Carried here rather than as a second parameter so `dump` keeps one
    /// options companion instead of growing a name per knob combination.
    #[must_use]
    pub const fn level(&self) -> crate::Level {
        self.level
    }

    /// Set the content-coding level a redirected dump encodes at.
    pub const fn set_level(&mut self, level: crate::Level) {
        self.level = level;
    }

    /// Return this value with a different content-coding level.
    #[must_use]
    pub const fn with_level(mut self, level: crate::Level) -> Self {
        self.level = level;
        self
    }
}
