//! One read/write surface shared by every structured text format.
//!
//! [`Json`], [`Jsonl`], [`Toml`], and [`Yaml`] are the four formats, and
//! [`TextCodec`] is what they all answer: parse a string, parse bytes, parse a
//! reader, parse the bytes a handle holds - and the same four in reverse. The
//! format decides the grammar; nothing else changes.
//!
//! Handle operations go through [`IOBase`], so the handle's own content coding
//! applies: reading `trades.json.gz` decompresses and writing it compresses,
//! with no argument saying so.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::text::{Json, TextCodec, Value};
//! use yggdryl::Url;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let value = Json.loads(r#"{"symbol":"AAPL"}"#)?;
//! assert_eq!(Json.dumps(&value)?, r#"{"symbol":"AAPL"}"#);
//!
//! // The same value through a compressed handle.
//! let mut handle =
//!     Buffer::new().with_media_type(Url::from_str("file:///quote.json.gz")?.media_type());
//! Json.dump(&mut handle, &value)?;
//! assert_eq!(Json.load(&handle)?, value);
//! assert_eq!(&handle.read_range(0, 2)?, &[0x1F, 0x8B]);
//! # Ok(())
//! # }
//! ```

use std::io::{Read, Write};

use super::{Format, Limits, Value};
use crate::io::IOBase;
use crate::{Level, MimeType, Result};

/// The read and write surface every structured text format provides.
///
/// A format value is the configuration: [`Json`] and friends are unit structs
/// with the default limits, and [`Json::with_limits`] returns a configured one.
pub trait TextCodec: Copy {
    /// The format this codec parses and writes.
    fn format(&self) -> Format;

    /// The parser bounds applied to untrusted input.
    fn limits(&self) -> Limits;

    /// Return this codec with explicit parser bounds.
    fn with_limits(self, limits: Limits) -> Limited<Self> {
        Limited {
            codec: self,
            limits,
        }
    }

    /// The MIME type a written value carries.
    fn mime_type(&self) -> MimeType {
        self.format().mime_type()
    }

    /// Return whether this format holds more than one value per document.
    fn is_multi_document(&self) -> bool {
        matches!(self.format(), Format::JsonLines | Format::Yaml)
    }

    /// Parse one value from text.
    ///
    /// # Errors
    ///
    /// Returns the parser failure, with the byte position it stopped at.
    fn loads(&self, text: &str) -> Result<Value> {
        super::from_str_with_limits(text, self.format(), self.limits())
    }

    /// Parse one value from bytes.
    ///
    /// # Errors
    ///
    /// Returns the parser failure, with the byte position it stopped at.
    fn load_slice(&self, bytes: &[u8]) -> Result<Value> {
        super::from_slice_with_limits(bytes, self.format(), self.limits())
    }

    /// Parse one value from a reader.
    ///
    /// # Errors
    ///
    /// Returns a read or parser failure.
    fn read<R: Read>(&self, reader: R) -> Result<Value> {
        super::from_reader_with_limits(reader, self.format(), self.limits())
    }

    /// Parse every value from text.
    ///
    /// A single-document format yields exactly one value.
    ///
    /// # Errors
    ///
    /// Returns the parser failure, with the byte position it stopped at.
    fn loads_all(&self, text: &str) -> Result<Vec<Value>> {
        super::from_str_all_with_limits(text, self.format(), self.limits())
    }

    /// Parse every value from bytes.
    ///
    /// # Errors
    ///
    /// Returns the parser failure, with the byte position it stopped at.
    fn load_slice_all(&self, bytes: &[u8]) -> Result<Vec<Value>> {
        super::from_slice_all_with_limits(bytes, self.format(), self.limits())
    }

    /// Parse every value from a reader.
    ///
    /// # Errors
    ///
    /// Returns a read or parser failure.
    fn read_all<R: Read>(&self, reader: R) -> Result<Vec<Value>> {
        super::from_reader_all_with_limits(reader, self.format(), self.limits())
    }

    /// Render one value as text.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be represented in this format.
    fn dumps(&self, value: &Value) -> Result<String> {
        let bytes = self.dump_vec(value)?;
        String::from_utf8(bytes).map_err(|error| crate::Error::Codec {
            format: self.format().as_str(),
            position: error.utf8_error().valid_up_to(),
            reason: smol_str::SmolStr::new_static("encoded output is not valid UTF-8"),
        })
    }

    /// Render one value as bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be represented in this format.
    fn dump_vec(&self, value: &Value) -> Result<Vec<u8>> {
        super::to_vec(value, self.format())
    }

    /// Write one value to a writer.
    ///
    /// # Errors
    ///
    /// Returns a write or representation failure.
    fn write<W: Write>(&self, writer: W, value: &Value) -> Result<()> {
        super::to_writer(writer, value, self.format())
    }

    /// Render several values as bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the format holds one value per document and more
    /// than one was supplied, or when a value cannot be represented.
    fn dump_vec_all(&self, values: &[Value]) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        super::to_writer_all(&mut bytes, values, self.format())?;
        Ok(bytes)
    }

    /// Write several values to a writer.
    ///
    /// # Errors
    ///
    /// Returns a write or representation failure.
    fn write_all<W: Write>(&self, writer: W, values: &[Value]) -> Result<()> {
        super::to_writer_all(writer, values, self.format())
    }

    /// Parse one value from the bytes a handle holds.
    ///
    /// The handle's content coding is removed first, so a `.json.gz` handle
    /// needs no extra argument. Per the laziness contract, a resource that does
    /// not exist yet is empty rather than an error, which is a parse failure
    /// for a format that requires a value.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or parser failure.
    fn load<H: IOBase + ?Sized>(&self, handle: &H) -> Result<Value> {
        self.load_slice(&decoded(handle)?)
    }

    /// Parse every value from the bytes a handle holds.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or parser failure.
    fn load_all<H: IOBase + ?Sized>(&self, handle: &H) -> Result<Vec<Value>> {
        self.load_slice_all(&decoded(handle)?)
    }

    /// Replace a handle's bytes with one rendered value.
    ///
    /// # Errors
    ///
    /// Returns a representation, encoding, or write failure.
    fn dump<H: IOBase + ?Sized>(&self, handle: &mut H, value: &Value) -> Result<()> {
        let bytes = self.dump_vec(value)?;
        store(handle, &bytes, Level::DEFAULT)
    }

    /// Replace a handle's bytes with several rendered values.
    ///
    /// # Errors
    ///
    /// Returns a representation, encoding, or write failure.
    fn dump_all<H: IOBase + ?Sized>(&self, handle: &mut H, values: &[Value]) -> Result<()> {
        let bytes = self.dump_vec_all(values)?;
        store(handle, &bytes, Level::DEFAULT)
    }

    /// Replace a handle's bytes at an explicit compression level.
    ///
    /// # Errors
    ///
    /// Returns a representation, encoding, or write failure.
    fn dump_with_level<H: IOBase + ?Sized>(
        &self,
        handle: &mut H,
        value: &Value,
        level: Level,
    ) -> Result<()> {
        let bytes = self.dump_vec(value)?;
        store(handle, &bytes, level)
    }
}

/// Read a handle's bytes with any declared content coding removed.
fn decoded<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<u8>> {
    let bytes = handle.read_all()?;
    if bytes.is_empty() {
        return Ok(bytes);
    }
    handle.codec().load(&bytes)
}

/// Apply the handle's content coding and replace its bytes.
fn store<H: IOBase + ?Sized>(handle: &mut H, bytes: &[u8], level: Level) -> Result<()> {
    let encoded = handle.codec().dump_with_level(bytes, level)?;
    handle.write_all_bytes(&encoded)
}

/// A codec with explicit parser bounds.
///
/// [`TextCodec::with_limits`] returns one of these, so the format values stay
/// unit structs that read as the format itself: `Json.loads(...)`.
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

/// Build one unit format type with its [`TextCodec`] implementation.
macro_rules! text_format {
    ($name:ident, $format:expr, $doc:literal) => {
        #[doc = $doc]
        ///
        /// The value uses the default parser bounds;
        /// [`TextCodec::with_limits`] returns one configured for untrusted
        /// input.
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
