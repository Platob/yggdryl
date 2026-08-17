//! Structured-text loading and dumping over [`IOBase`] handles.
//!
//! An [`IOBase`] already carries where its bytes live and what they are, so
//! [`load`] and [`dump`] take one handle rather than a store plus a location.
//! The structured format and the content coding both come from that handle's
//! media type, which a location-addressed implementation infers from its
//! compound filename: `trades.json.gz` decompresses and parses without a
//! caller naming either step, and dumping to the same handle recompresses it.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::text::{dump, load};
//! use yggdryl::{Url, Value};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut handle = Buffer::new().with_media_type(Url::from_str("file:///trades.json.gz")?.media_type());
//!
//! let value = Value::from_mapping([("symbol".into(), Value::from("AAPL"))])?;
//! dump(&mut handle, &value)?;
//!
//! // The stored bytes really are gzip.
//! assert_eq!(&handle.as_slice()[..2], &[0x1F, 0x8B]);
//! assert_eq!(load(&handle)?, value);
//! # Ok(())
//! # }
//! ```

use crate::io::IOBase;
use crate::text::{Format, Limits, Value};
use crate::{Codec, Level};
use crate::{Error, MediaType, MimeType, Result};

/// The structured format and content coding a handle's bytes use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    format: Format,
    codec: Codec,
}

impl Plan {
    /// Pair an explicit format with an explicit coding.
    pub const fn new(format: Format, codec: Codec) -> Self {
        Self { format, codec }
    }

    /// Derive both from a media type.
    ///
    /// # Errors
    ///
    /// Returns an error when the media type names no structured-text format.
    pub fn from_media_type(media_type: &MediaType) -> Result<Self> {
        let format =
            format_from_mime(media_type.base()).ok_or_else(|| unknown_format(media_type))?;
        Ok(Self {
            format,
            codec: Codec::from_media_type(media_type),
        })
    }

    /// Derive both from a handle's declared media type.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle names no structured-text format.
    pub fn infer(handle: &impl IOBase) -> Result<Self> {
        Self::from_media_type(handle.media_type())
    }

    /// Derive the coding from a payload's leading bytes and the format from
    /// the handle's media type.
    ///
    /// The two sources are deliberately not treated the same way. Content
    /// codings have byte signatures, so the payload is authoritative and a
    /// handle labelled JSON whose bytes are really gzip still decodes.
    /// Structured text has no signature and JSON is a subset of YAML, so `{`
    /// identifies neither; the declared media type decides the format and
    /// content is consulted only when it names nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when neither source names a structured-text format.
    pub fn detect(handle: &impl IOBase, head: &[u8]) -> Result<Self> {
        let content = MediaType::from_magic_bytes(head);
        let codec = content.as_ref().map_or(Codec::Identity, |media| {
            // A coding whose payload is unidentifiable is reported as the base
            // type rather than as an encoding, so check both positions.
            match Codec::from_media_type(media) {
                Codec::Identity => Codec::from_mime_type(media.base()),
                coding => coding,
            }
        });
        let declared = handle.media_type();
        let format = format_from_mime(declared.base())
            .or_else(|| {
                content
                    .as_ref()
                    .and_then(|media| format_from_mime(media.base()))
            })
            .ok_or_else(|| unknown_format(declared))?;
        Ok(Self { format, codec })
    }

    /// Return the structured-text format.
    pub const fn format(self) -> Format {
        self.format
    }

    /// Return the content coding.
    pub const fn codec(self) -> Codec {
        self.codec
    }
}

/// Name the format a MIME type describes, through the one shared table.
fn format_from_mime(mime: &MimeType) -> Option<Format> {
    Format::from_mime_type(mime).ok()
}

fn unknown_format(media_type: &MediaType) -> Error {
    Error::Codec {
        format: "text",
        position: 0,
        reason: smol_str::format_smolstr!(
            "expected json, jsonl, yaml, or toml, got {}",
            crate::text::elide_display(media_type)
        ),
    }
}

/// Read one structured value from a handle, decoding any content coding.
///
/// # Errors
///
/// Returns a read, decoding, or parse failure.
pub fn load(source: &impl IOBase) -> Result<Value> {
    load_with_limits(source, Limits::default())
}

/// Read one structured value under explicit parser limits.
///
/// # Errors
///
/// Returns a read, decoding, or parse failure.
pub fn load_with_limits(source: &impl IOBase, limits: Limits) -> Result<Value> {
    let bytes = source.read_all()?;
    let plan = Plan::detect(source, &bytes)?;
    let decoded = plan.codec().load(&bytes)?;
    crate::text::from_slice_with_limits(&decoded, plan.format(), limits)
}

/// Read every structured value from a multi-document handle.
///
/// # Errors
///
/// Returns a read, decoding, or parse failure.
pub fn load_all(source: &impl IOBase) -> Result<Vec<Value>> {
    load_all_with_limits(source, Limits::default())
}

/// Read every structured value under explicit parser limits.
///
/// # Errors
///
/// Returns a read, decoding, or parse failure.
pub fn load_all_with_limits(source: &impl IOBase, limits: Limits) -> Result<Vec<Value>> {
    let bytes = source.read_all()?;
    let plan = Plan::detect(source, &bytes)?;
    let decoded = plan.codec().load(&bytes)?;
    crate::text::from_slice_all_with_limits(&decoded, plan.format(), limits)
}

/// Write one structured value to a handle, applying its declared coding.
///
/// The handle's existing bytes are replaced only once encoding succeeds, so a
/// failure leaves them untouched.
///
/// # Errors
///
/// Returns an encoding, serialization, or write failure.
pub fn dump(target: &mut impl IOBase, value: &Value) -> Result<()> {
    dump_with_level(target, value, Level::DEFAULT)
}

/// Write one structured value at an explicit compression level.
///
/// # Errors
///
/// Returns an encoding, serialization, or write failure.
pub fn dump_with_level(target: &mut impl IOBase, value: &Value, level: Level) -> Result<()> {
    let plan = Plan::infer(target)?;
    let mut encoded = Vec::new();
    crate::text::to_writer(&mut encoded, value, plan.format())?;
    let encoded = plan.codec().dump_with_level(&encoded, level)?;
    target.write_all_bytes(&encoded)
}

/// Write many structured values to a multi-document handle.
///
/// # Errors
///
/// Returns an encoding, serialization, or write failure.
pub fn dump_all(target: &mut impl IOBase, values: &[Value]) -> Result<()> {
    let plan = Plan::infer(target)?;
    let mut encoded = Vec::new();
    crate::text::to_writer_all(&mut encoded, values, plan.format())?;
    let encoded = plan.codec().dump(&encoded)?;
    target.write_all_bytes(&encoded)
}

#[cfg(test)]
mod tests {
    use super::{Plan, dump, dump_all, load, load_all};
    use crate::Codec;
    use crate::io::Buffer;
    use crate::text::Format;
    use crate::{Url, Value};

    fn sample() -> Value {
        Value::from_mapping([
            ("symbol".into(), Value::from("AAPL")),
            ("quantity".into(), Value::from(100_i64)),
        ])
        .unwrap()
    }

    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    #[test]
    fn plans_are_inferred_from_compound_filenames() {
        let cases = [
            ("a.json", Format::Json, Codec::Identity),
            ("a.json.gz", Format::Json, Codec::Gzip),
            ("a.yaml.zst", Format::Yaml, Codec::Zstd),
            ("a.toml", Format::Toml, Codec::Identity),
            ("a.jsonl.gz", Format::JsonLines, Codec::Gzip),
        ];
        for (name, format, codec) in cases {
            let plan = Plan::infer(&handle(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(plan.format(), format, "{name}");
            assert_eq!(plan.codec(), codec, "{name}");
        }
    }

    #[test]
    fn an_unrecognized_handle_names_what_it_accepts() {
        let message = Plan::infer(&handle("a.parquet")).unwrap_err().to_string();
        assert!(message.contains("json"), "{message}");
        assert!(message.contains("toml"), "{message}");
    }

    #[test]
    fn every_format_and_coding_round_trips() {
        let value = sample();
        for name in [
            "a.json",
            "a.json.gz",
            "a.json.zst",
            "a.yaml",
            "a.yaml.gz",
            "a.toml",
            "a.toml.zst",
        ] {
            let mut target = handle(name);
            dump(&mut target, &value).unwrap_or_else(|error| panic!("{name}: {error}"));
            let loaded = load(&target).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(loaded, value, "{name}");
        }
    }

    #[test]
    fn compressed_output_really_is_compressed() {
        let value = sample();
        let mut plain = handle("a.json");
        let mut gzipped = handle("a.json.gz");
        dump(&mut plain, &value).unwrap();
        dump(&mut gzipped, &value).unwrap();

        assert_eq!(&gzipped.as_slice()[..2], &[0x1F, 0x8B]);
        assert_ne!(plain.as_slice(), gzipped.as_slice());
    }

    #[test]
    fn content_inference_overrides_a_misleading_media_type() {
        let mut honest = handle("real.json.gz");
        dump(&mut honest, &sample()).unwrap();

        // The same bytes behind a handle that claims plain JSON.
        let liar = Buffer::from_bytes(honest.as_slice().to_vec())
            .with_media_type(Url::from_str("file:///liar.json").unwrap().media_type());

        assert_eq!(load(&liar).unwrap(), sample());
    }

    #[test]
    fn a_failed_dump_leaves_the_previous_bytes_intact() {
        let mut target = Buffer::from_bytes(b"previous".to_vec())
            .with_media_type(Url::from_str("file:///never.parquet").unwrap().media_type());
        assert!(dump(&mut target, &sample()).is_err());
        assert_eq!(target.as_slice(), b"previous");
    }

    #[test]
    fn multi_document_handles_round_trip() {
        let values = vec![sample(), sample()];
        for name in ["many.jsonl", "many.jsonl.gz", "many.yaml"] {
            let mut target = handle(name);
            dump_all(&mut target, &values).unwrap_or_else(|error| panic!("{name}: {error}"));
            let loaded = load_all(&target).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(loaded, values, "{name}");
        }
    }
}
