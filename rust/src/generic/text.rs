//! One value naming every structured text format.

use crate::io::IOBase;
use crate::text::{Format, Json, Jsonl, Limits, TextCodec, Toml, Yaml};
use crate::{MediaType, MimeType, Result, Url};

/// A structured text format chosen at runtime.
///
/// [`TextCodec`] is what a *known* format answers; this enum is what a caller
/// holds when the format comes from a name instead of from the code - a
/// filename, a media type, an HTTP header. It implements the same trait, so
/// nothing downstream knows the difference.
///
/// ```
/// use yggdryl::generic::Text;
/// use yggdryl::text::TextCodec;
/// use yggdryl::Url;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // The name decides the format, and the coding rides on the handle.
/// let format = Text::for_url(&Url::from_str("file:///trades.yaml.gz")?)?;
/// assert_eq!(format, Text::Yaml);
///
/// let value = format.from_utf8("symbol: AAPL\n")?;
/// assert_eq!(value.get_key_str("symbol").and_then(yggdryl::Scalar::as_utf8), Some("AAPL"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Text {
    /// One JSON value per document.
    #[default]
    Json,
    /// Newline-delimited JSON.
    Jsonl,
    /// One TOML document.
    Toml,
    /// One or more YAML documents.
    Yaml,
}

impl Text {
    /// Every format in canonical order.
    pub const ALL: [Self; 4] = [Self::Json, Self::Jsonl, Self::Toml, Self::Yaml];

    /// Name the format a [`Format`] value describes.
    pub const fn from_format(format: Format) -> Self {
        match format {
            Format::Json => Self::Json,
            Format::JsonLines => Self::Jsonl,
            Format::Toml => Self::Toml,
            Format::Yaml => Self::Yaml,
        }
    }

    /// Name the format a MIME type describes.
    ///
    /// # Errors
    ///
    /// Returns an error when the MIME type names no structured text format.
    pub fn for_mime_type(mime_type: &MimeType) -> Result<Self> {
        Format::from_mime_type(mime_type).map(Self::from_format)
    }

    /// Name the format a media type describes, ignoring its content codings.
    ///
    /// # Errors
    ///
    /// Returns an error when the media type names no structured text format.
    pub fn for_media_type(media_type: &MediaType) -> Result<Self> {
        Self::for_mime_type(media_type.base())
    }

    /// Name the format a location's compound filename describes.
    ///
    /// # Errors
    ///
    /// Returns an error when the name identifies no structured text format.
    pub fn for_url(url: &Url) -> Result<Self> {
        Self::for_media_type(&url.media_type())
    }

    /// Name the format a handle's own media type describes.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle names no structured text format.
    pub fn for_handle<H: IOBase + ?Sized>(handle: &H) -> Result<Self> {
        Self::for_media_type(handle.media_type())
    }
}

impl TextCodec for Text {
    fn format(&self) -> Format {
        match self {
            Self::Json => Format::Json,
            Self::Jsonl => Format::JsonLines,
            Self::Toml => Format::Toml,
            Self::Yaml => Format::Yaml,
        }
    }

    fn limits(&self) -> Limits {
        Limits::default()
    }
}

impl From<Format> for Text {
    fn from(value: Format) -> Self {
        Self::from_format(value)
    }
}

impl From<Text> for Format {
    fn from(value: Text) -> Self {
        value.format()
    }
}

impl From<Json> for Text {
    fn from(_: Json) -> Self {
        Self::Json
    }
}

impl From<Jsonl> for Text {
    fn from(_: Jsonl) -> Self {
        Self::Jsonl
    }
}

impl From<Toml> for Text {
    fn from(_: Toml) -> Self {
        Self::Toml
    }
}

impl From<Yaml> for Text {
    fn from(_: Yaml) -> Self {
        Self::Yaml
    }
}

#[cfg(test)]
mod tests;
