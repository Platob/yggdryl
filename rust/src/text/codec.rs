//! One natural read/write surface for every structured-text format.

use std::io::{Read, Write};

use super::{Format, Formatting, Limits, Scalar};
use crate::IOBase;
use crate::{Field, Level, MimeType, Result};

/// The read and write surface shared by JSON, JSON Lines, YAML, and TOML.
// The receiver selects the codec; these names intentionally describe the
// conversion direction at the public format boundary.
#[allow(clippy::wrong_self_convention)]
pub trait TextCodec: Copy {
    /// Return this codec's format.
    fn format(&self) -> Format;

    /// Return this codec's parser bounds.
    fn limits(&self) -> Limits;

    /// Return a codec with explicit parser bounds.
    fn with_limits(self, limits: Limits) -> Limited<Self> {
        Limited {
            codec: self,
            limits,
        }
    }

    /// Return this codec's MIME type.
    fn mime_type(&self) -> MimeType {
        self.format().mime_type()
    }

    /// Return whether this format carries several documents.
    fn is_multi_document(&self) -> bool {
        matches!(self.format(), Format::JsonLines | Format::Yaml)
    }

    /// Parse one value from UTF-8.
    fn from_utf8(&self, input: &str) -> Result<Scalar> {
        super::from_utf8_with_limits(input, self.format(), self.limits())
    }

    /// Parse and type one UTF-8 value under `field`.
    fn from_utf8_with_field(&self, input: &str, field: &Field) -> Result<Scalar> {
        super::from_utf8_with_field_and_limits(input, self.format(), field, self.limits())
    }

    /// Parse one value from bytes.
    fn from_bytes(&self, input: &[u8]) -> Result<Scalar> {
        super::from_bytes_with_limits(input, self.format(), self.limits())
    }

    /// Parse and type one byte value under `field`.
    fn from_bytes_with_field(&self, input: &[u8], field: &Field) -> Result<Scalar> {
        super::from_bytes_with_field_and_limits(input, self.format(), field, self.limits())
    }

    /// Parse one value from a standard byte reader.
    fn from_reader<R: Read>(&self, reader: R) -> Result<Scalar> {
        super::from_reader_with_limits(reader, self.format(), self.limits())
    }

    /// Parse and type one reader value under `field`.
    fn from_reader_with_field<R: Read>(&self, reader: R, field: &Field) -> Result<Scalar> {
        super::from_reader_with_field_and_limits(reader, self.format(), field, self.limits())
    }

    /// Parse every UTF-8 document.
    fn from_utf8_all(&self, input: &str) -> Result<Vec<Scalar>> {
        super::from_utf8_all_with_limits(input, self.format(), self.limits())
    }

    /// Parse and type every UTF-8 document under `field`.
    fn from_utf8_all_with_field(&self, input: &str, field: &Field) -> Result<Vec<Scalar>> {
        super::from_utf8_all_with_field_and_limits(input, self.format(), field, self.limits())
    }

    /// Parse every byte document.
    fn from_bytes_all(&self, input: &[u8]) -> Result<Vec<Scalar>> {
        super::from_bytes_all_with_limits(input, self.format(), self.limits())
    }

    /// Parse and type every byte document under `field`.
    fn from_bytes_all_with_field(&self, input: &[u8], field: &Field) -> Result<Vec<Scalar>> {
        super::from_bytes_all_with_field_and_limits(input, self.format(), field, self.limits())
    }

    /// Parse every document from a standard byte reader.
    fn from_reader_all<R: Read>(&self, reader: R) -> Result<Vec<Scalar>> {
        super::from_reader_all_with_limits(reader, self.format(), self.limits())
    }

    /// Parse and type every reader document under `field`.
    fn from_reader_all_with_field<R: Read>(&self, reader: R, field: &Field) -> Result<Vec<Scalar>> {
        super::from_reader_all_with_field_and_limits(reader, self.format(), field, self.limits())
    }

    /// Encode one value as UTF-8.
    fn into_utf8(&self, value: &Scalar) -> Result<String> {
        super::into_utf8(value, self.format())
    }

    /// Encode one value as UTF-8 with explicit formatting.
    fn into_utf8_with_formatting(&self, value: &Scalar, formatting: Formatting) -> Result<String> {
        super::into_utf8_with_formatting(value, self.format(), formatting)
    }

    /// Encode one value as bytes.
    fn into_bytes(&self, value: &Scalar) -> Result<Vec<u8>> {
        super::into_bytes(value, self.format())
    }

    /// Encode one value as bytes with explicit formatting.
    fn into_bytes_with_formatting(
        &self,
        value: &Scalar,
        formatting: Formatting,
    ) -> Result<Vec<u8>> {
        super::into_bytes_with_formatting(value, self.format(), formatting)
    }

    /// Encode one value to a standard byte writer.
    fn into_writer<W: Write>(&self, value: &Scalar, writer: W) -> Result<()> {
        super::into_writer(value, writer, self.format())
    }

    /// Encode one value to a writer with explicit formatting.
    fn into_writer_with_formatting<W: Write>(
        &self,
        value: &Scalar,
        writer: W,
        formatting: Formatting,
    ) -> Result<()> {
        super::into_writer_with_formatting(value, writer, self.format(), formatting)
    }

    /// Encode values as UTF-8.
    fn into_utf8_all(&self, values: &[Scalar]) -> Result<String> {
        super::into_utf8_all(values, self.format())
    }

    /// Encode values as bytes.
    fn into_bytes_all(&self, values: &[Scalar]) -> Result<Vec<u8>> {
        super::into_bytes_all(values, self.format())
    }

    /// Encode values to a standard byte writer.
    fn into_writer_all<W: Write>(&self, values: &[Scalar], writer: W) -> Result<()> {
        super::into_writer_all(values, writer, self.format())
    }

    /// Parse one value from a Yggdryl I/O handle.
    fn from_io<H: IOBase + ?Sized>(&self, handle: &H) -> Result<Scalar> {
        let decoded = super::io::decoded_for_format(handle, self.format())?;
        super::from_reader_with_limits(decoded, self.format(), self.limits())
    }

    /// Parse and type one handle value under `field`.
    fn from_io_with_field<H: IOBase + ?Sized>(&self, handle: &H, field: &Field) -> Result<Scalar> {
        let decoded = super::io::decoded_for_format(handle, self.format())?;
        super::from_reader_with_field_and_limits(decoded, self.format(), field, self.limits())
    }

    /// Parse every value from a Yggdryl I/O handle.
    fn from_io_all<H: IOBase + ?Sized>(&self, handle: &H) -> Result<Vec<Scalar>> {
        let decoded = super::io::decoded_for_format(handle, self.format())?;
        super::from_reader_all_with_limits(decoded, self.format(), self.limits())
    }

    /// Parse and type every handle value under `field`.
    fn from_io_all_with_field<H: IOBase + ?Sized>(
        &self,
        handle: &H,
        field: &Field,
    ) -> Result<Vec<Scalar>> {
        let decoded = super::io::decoded_for_format(handle, self.format())?;
        super::from_reader_all_with_field_and_limits(decoded, self.format(), field, self.limits())
    }

    /// Replace a Yggdryl handle with one encoded value.
    fn into_io<H: IOBase + ?Sized>(&self, value: &Scalar, handle: &mut H) -> Result<()> {
        self.into_io_with_formatting(value, handle, Formatting::default())
    }

    /// Replace a handle with one value at an explicit compression level.
    fn into_io_with_level<H: IOBase + ?Sized>(
        &self,
        value: &Scalar,
        handle: &mut H,
        level: Level,
    ) -> Result<()> {
        self.into_io_with_formatting(value, handle, Formatting::default().with_level(level))
    }

    /// Replace a handle with one value under explicit formatting.
    fn into_io_with_formatting<H: IOBase + ?Sized>(
        &self,
        value: &Scalar,
        handle: &mut H,
        formatting: Formatting,
    ) -> Result<()> {
        let mut encoded = Vec::new();
        {
            let mut writer = handle
                .codec()
                .writer_with_level(&mut encoded, formatting.level());
            self.into_writer_with_formatting(value, &mut writer, formatting)?;
            writer.finish()?;
        }
        handle.write_all_bytes(&encoded)
    }

    /// Replace a handle with encoded values.
    fn into_io_all<H: IOBase + ?Sized>(&self, values: &[Scalar], handle: &mut H) -> Result<()> {
        let mut encoded = Vec::new();
        {
            let mut writer = handle.codec().writer(&mut encoded);
            self.into_writer_all(values, &mut writer)?;
            writer.finish()?;
        }
        handle.write_all_bytes(&encoded)
    }
}

/// A codec with explicit parser limits.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Limited<C: TextCodec> {
    codec: C,
    limits: Limits,
}

impl<C: TextCodec> TextCodec for Limited<C> {
    fn format(&self) -> Format {
        self.codec.format()
    }

    fn limits(&self) -> Limits {
        self.limits
    }
}

macro_rules! text_format {
    ($name:ident, $format:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name;

        impl TextCodec for $name {
            fn format(&self) -> Format {
                $format
            }

            fn limits(&self) -> Limits {
                Limits::default()
            }
        }
    };
}

text_format!(Json, Format::Json, "One JSON value per document.");
text_format!(
    Jsonl,
    Format::JsonLines,
    "Newline-delimited JSON: one value per line."
);
text_format!(Toml, Format::Toml, "One TOML document.");
text_format!(Yaml, Format::Yaml, "One or more YAML documents.");

#[cfg(test)]
mod tests;
