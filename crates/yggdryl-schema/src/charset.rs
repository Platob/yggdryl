//! The [`Charset`] encoding that a string type reads its bytes with.

/// A text encoding. The default is [`Utf8`](Charset::Utf8); the string types carry
/// a charset so the same binary storage can be read as UTF-8, ASCII or Latin-1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Charset {
    /// UTF-8 (the default).
    #[default]
    Utf8,
    /// 7-bit ASCII.
    Ascii,
    /// Latin-1 (ISO-8859-1).
    Latin1,
}

impl Charset {
    /// The canonical charset name, e.g. `"utf8"`.
    ///
    /// ```
    /// use yggdryl_schema::Charset;
    ///
    /// assert_eq!(Charset::default(), Charset::Utf8);
    /// assert_eq!(Charset::Latin1.name(), "latin1");
    /// assert_eq!(Charset::from_name("ascii"), Some(Charset::Ascii));
    /// assert_eq!(Charset::from_name("nope"), None);
    /// ```
    pub fn name(self) -> &'static str {
        match self {
            Charset::Utf8 => "utf8",
            Charset::Ascii => "ascii",
            Charset::Latin1 => "latin1",
        }
    }

    /// Parses a canonical charset name, returning `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "utf8" => Some(Charset::Utf8),
            "ascii" => Some(Charset::Ascii),
            "latin1" => Some(Charset::Latin1),
            _ => None,
        }
    }
}
