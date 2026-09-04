//! Natural structured-text I/O over Yggdryl handles.

use std::io::{Cursor, Read};

use crate::text::{Format, Formatting, Limits, Loading, Scalar};
use crate::{Codec, Error, Field, Level, MediaType, MimeType, Result};
use crate::{DEFAULT_STREAM_BATCH_SIZE, IOBase};

/// The structured format and content coding used by a handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Plan {
    format: Format,
    codec: Codec,
}

impl Plan {
    /// Pair an explicit format with an explicit coding.
    pub const fn new(format: Format, codec: Codec) -> Self {
        Self { format, codec }
    }

    /// Derive a plan from a media type.
    pub fn from_media_type(media_type: &MediaType) -> Result<Self> {
        let format =
            Format::from_mime_type(media_type.base()).map_err(|_| unknown_format(media_type))?;
        Ok(Self {
            format,
            codec: Codec::from_media_type(media_type),
        })
    }

    /// Derive a plan from a handle's media type.
    pub fn infer<H: IOBase + ?Sized>(handle: &H) -> Result<Self> {
        Self::from_media_type(handle.media_type())
    }

    /// Detect coding from bytes and format from the handle, then content.
    pub fn detect<H: IOBase + ?Sized>(handle: &H, head: &[u8]) -> Result<Self> {
        let content = MediaType::from_magic_bytes(head);
        let codec = detected_codec(content.as_ref());
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

    /// Return the structured format.
    pub const fn format(self) -> Format {
        self.format
    }

    /// Return the content coding.
    pub const fn codec(self) -> Codec {
        self.codec
    }
}

/// Recover a content coding from the probed representation, including media
/// types such as `application/gzip` that name the coding as their base.
fn detected_codec(content: Option<&MediaType>) -> Codec {
    content.map_or(Codec::Identity, |media| {
        match Codec::from_media_type(media) {
            Codec::Identity => Codec::from_mime_type(media.base()),
            coding => coding,
        }
    })
}

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

/// Read one natural structured value from a Yggdryl handle.
pub fn from_io<H: IOBase + ?Sized>(source: &H) -> Result<Scalar> {
    from_io_with_limits(source, Limits::default())
}

/// Read one value from a handle with explicit parser limits.
pub fn from_io_with_limits<H: IOBase + ?Sized>(source: &H, limits: Limits) -> Result<Scalar> {
    let (decoded, plan) = decoded(source)?;
    crate::text::from_reader_with_limits(decoded, plan.format(), limits)
}

/// Read one handle value under `field`.
pub fn from_io_with_field<H: IOBase + ?Sized>(source: &H, field: &Field) -> Result<Scalar> {
    from_io_with_field_and_limits(source, field, Limits::default())
}

/// Read one schema-directed handle value with explicit limits.
pub fn from_io_with_field_and_limits<H: IOBase + ?Sized>(
    source: &H,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    let (decoded, plan) = decoded(source)?;
    crate::text::from_reader_with_field_and_limits(decoded, plan.format(), field, limits)
}

/// Read one value from a handle under placeholder loading options.
pub fn from_io_with<H: IOBase + ?Sized>(source: &H, loading: &Loading) -> Result<Scalar> {
    let (decoded, plan) = decoded(source)?;
    crate::text::from_reader_with(decoded, plan.format(), loading)
}

/// Read every natural structured value from a handle.
pub fn from_io_all<H: IOBase + ?Sized>(source: &H) -> Result<Vec<Scalar>> {
    from_io_all_with_limits(source, Limits::default())
}

/// Read every handle value with explicit parser limits.
pub fn from_io_all_with_limits<H: IOBase + ?Sized>(
    source: &H,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    let (decoded, plan) = decoded(source)?;
    crate::text::from_reader_all_with_limits(decoded, plan.format(), limits)
}

/// Replace a handle with one natural structured value.
pub fn into_io<H: IOBase + ?Sized>(value: &Scalar, target: &mut H) -> Result<()> {
    into_io_with_formatting(value, target, Formatting::default())
}

/// Replace a handle with one value at an explicit compression level.
pub fn into_io_with_level<H: IOBase + ?Sized>(
    value: &Scalar,
    target: &mut H,
    level: Level,
) -> Result<()> {
    into_io_with_formatting(value, target, Formatting::default().with_level(level))
}

/// Replace a handle with one value under explicit formatting.
pub fn into_io_with_formatting<H: IOBase + ?Sized>(
    value: &Scalar,
    target: &mut H,
    formatting: Formatting,
) -> Result<()> {
    let plan = Plan::infer(target)?;
    let mut encoded = Vec::new();
    {
        let mut writer = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        crate::text::into_writer_with_formatting(value, &mut writer, plan.format(), formatting)?;
        writer.finish()?;
    }
    target.write_all_bytes(&encoded)
}

/// Replace a handle with encoded values.
pub fn into_io_all<H: IOBase + ?Sized>(values: &[Scalar], target: &mut H) -> Result<()> {
    into_io_all_with_formatting(values, target, Formatting::default())
}

/// Replace a handle with encoded values under explicit formatting.
pub fn into_io_all_with_formatting<H: IOBase + ?Sized>(
    values: &[Scalar],
    target: &mut H,
    formatting: Formatting,
) -> Result<()> {
    let plan = Plan::infer(target)?;
    let mut encoded = Vec::new();
    {
        let mut writer = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        crate::text::into_writer_all_with_formatting(
            values.iter(),
            &mut writer,
            plan.format(),
            formatting,
        )?;
        writer.finish()?;
    }
    target.write_all_bytes(&encoded)
}

/// Open one decoded stream without retaining earlier byte batches.
///
/// The small prefix is replayed after magic detection, so sniffing never drops
/// bytes and the parser still sees the source from position zero. The returned
/// decoder owns the byte stream; its [`Read`] implementation fills parser-owned
/// buffers directly and therefore allocates no iterator `Vec` per batch.
pub(super) fn decoded<H: IOBase + ?Sized>(source: &H) -> Result<(Box<dyn Read + '_>, Plan)> {
    decoded_with_format(source, None)
}

/// Open a decoded transport while letting an explicit codec receiver select
/// the structured format. This preserves `Json.from_io(an_empty_buffer)`'s
/// JSON error semantics even when the buffer itself has no media declaration.
pub(super) fn decoded_for_format<H: IOBase + ?Sized>(
    source: &H,
    format: Format,
) -> Result<Box<dyn Read + '_>> {
    decoded_with_format(source, Some(format)).map(|(reader, _)| reader)
}

fn decoded_with_format<H: IOBase + ?Sized>(
    source: &H,
    format: Option<Format>,
) -> Result<(Box<dyn Read + '_>, Plan)> {
    let mut encoded = source.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?;
    let mut head = Vec::with_capacity(crate::generic::MAGIC_PROBE_LEN);
    {
        let mut probe = std::io::Read::take(&mut encoded, crate::generic::MAGIC_PROBE_LEN as u64);
        probe.read_to_end(&mut head)?;
    }
    let plan = match format {
        Some(format) => Plan::new(
            format,
            detected_codec(MediaType::from_magic_bytes(&head).as_ref()),
        ),
        None => Plan::detect(source, &head)?,
    };
    let replayed = Cursor::new(head).chain(encoded);
    Ok((plan.codec().reader(replayed), plan))
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::{Plan, from_io, from_io_all, into_io, into_io_all};
    use crate::IOBase;
    use crate::holder::Buffer;
    use crate::text::Format;
    use crate::{Codec, Scalar, Url};

    #[test]
    fn plans_have_complete_value_traits() {
        fn assert_traits<T: Copy + Eq + Ord + Hash>() {}
        assert_traits::<Plan>();
        let plan = Plan::new(Format::Json, Codec::Gzip);
        assert_eq!(plan, plan.clone());
        assert_eq!(crate::stable_hash_of(&plan), crate::stable_hash_of(&plan));
    }

    fn sample() -> Scalar {
        Scalar::from_record([
            ("quantity", Scalar::I64(100)),
            ("symbol", Scalar::from("AAPL")),
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
    fn plans_follow_compound_filenames() {
        let cases = [
            ("a.json", Format::Json, Codec::Identity),
            ("a.json.gz", Format::Json, Codec::Gzip),
            ("a.yaml.zst", Format::Yaml, Codec::Zstd),
            ("a.toml", Format::Toml, Codec::Identity),
        ];
        for (name, format, codec) in cases {
            let plan = Plan::infer(&handle(name)).unwrap();
            assert_eq!((plan.format(), plan.codec()), (format, codec));
        }
    }

    #[test]
    fn formats_and_codings_round_trip() {
        for name in ["a.json", "a.json.gz", "a.yaml.zst", "a.toml"] {
            let mut target = handle(name);
            into_io(&sample(), &mut target).unwrap();
            assert_eq!(from_io(&target).unwrap(), sample(), "{name}");
        }
    }

    #[test]
    fn multi_document_handles_round_trip() {
        let values = [sample(), sample()];
        for name in ["many.jsonl", "many.yaml"] {
            let mut target = handle(name);
            into_io_all(&values, &mut target).unwrap();
            assert_eq!(from_io_all(&target).unwrap(), values, "{name}");
        }
    }

    #[test]
    fn structured_parsers_cross_stream_batches_under_every_coding() {
        let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut state = 0xA537_1D09_u32;
        let message = (0..256 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                alphabet[state as usize % alphabet.len()] as char
            })
            .collect::<String>();
        let value = Scalar::from_record([
            ("quantity", Scalar::I64(100)),
            ("message", Scalar::from(message)),
        ])
        .unwrap();
        let cases = [
            (Format::Json, "json"),
            (Format::Yaml, "yaml"),
            (Format::Toml, "toml"),
        ];
        let codings = [
            (Codec::Gzip, "gz"),
            (Codec::Zlib, "zz"),
            (Codec::Zstd, "zst"),
        ];

        for (format, extension) in cases {
            let plain = crate::text::into_bytes(&value, format).unwrap();
            assert!(plain.len() > crate::DEFAULT_STREAM_BATCH_SIZE);
            for (codec, suffix) in codings {
                // Deliberately declare only the structured format: the bounded
                // replay prefix must still preserve magic-based coding
                // detection before the parser crosses several decoded chunks.
                let mut source = handle(&format!("large.{extension}"));
                let encoded = codec.dump(&plain).unwrap();
                assert!(
                    encoded.len() > crate::DEFAULT_STREAM_BATCH_SIZE,
                    "{format}/{codec}"
                );
                source.write_all_bytes(&encoded).unwrap();
                assert_eq!(
                    from_io(&source).unwrap(),
                    value,
                    "{format}/{codec}/{suffix}"
                );
            }
        }
    }
}
