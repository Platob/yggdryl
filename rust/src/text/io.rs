//! Natural structured-text I/O over Yggdryl handles.

use std::io::{Cursor, Read, Write};

use crate::text::{Format, Formatting, Limits, Loading, Scalar};
use crate::{Charset, Codec, Error, Field, Level, MediaType, MimeType, Result};
use crate::{DEFAULT_STREAM_BATCH_SIZE, IOBase};

/// The structured format, content coding, and charset used by a handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Plan {
    format: Format,
    codec: Codec,
    charset: Charset,
}

impl Plan {
    /// Pair an explicit format with an explicit coding, read as UTF-8.
    pub const fn new(format: Format, codec: Codec) -> Self {
        Self {
            format,
            codec,
            charset: Charset::Utf8,
        }
    }

    /// Return this plan with a different charset.
    #[must_use]
    pub const fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Derive a plan from a media type.
    pub fn from_media_type(media_type: &MediaType) -> Result<Self> {
        let format =
            Format::from_mime_type(media_type.base()).map_err(|_| unknown_format(media_type))?;
        Ok(Self {
            format,
            codec: Codec::from_media_type(media_type),
            charset: Charset::from_media_type(media_type),
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
        Ok(Self {
            format,
            codec,
            charset: Charset::from_media_type(declared),
        })
    }

    /// Return the structured format.
    pub const fn format(self) -> Format {
        self.format
    }

    /// Return the content coding.
    pub const fn codec(self) -> Codec {
        self.codec
    }

    /// Return the charset the decoded bytes are read in.
    pub const fn charset(self) -> Charset {
        self.charset
    }

    /// Write what a reader of these bytes needs before the document.
    ///
    /// XML assumes UTF-8, so a document written in any other charset opens
    /// with the declaration naming it - and, in a UTF-16 form, with the byte
    /// order mark XML 1.0 requires of one, written through the charset writer
    /// so it is the mark of that form; every other format, and UTF-8, writes
    /// nothing. This is the write half of the mark and the declaration read in
    /// [`from_io`], and it sits with the charset because the charset is the
    /// transport's decision.
    pub(crate) fn write_prolog<W: Write>(self, writer: &mut W) -> Result<()> {
        if self.format == Format::Xml && !self.charset.is_utf8() {
            if self.charset.is_unicode() {
                write!(writer, "\u{FEFF}")?;
            }
            write!(
                writer,
                "<?xml version=\"1.0\" encoding=\"{}\"?>",
                self.charset.as_str()
            )?;
        }
        Ok(())
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
            "expected json, jsonl, yaml, toml, or xml, got {}",
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
        let mut coded = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        {
            // Rendered text is encoded in the declared charset, and only then
            // compressed: the coding applies to the bytes a reader will meet.
            let mut writer = plan.charset().writer(&mut coded);
            plan.write_prolog(&mut writer)?;
            crate::text::into_writer_with_formatting(
                value,
                &mut writer,
                plan.format(),
                formatting,
            )?;
            writer.finish()?;
        }
        coded.finish()?;
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
        let mut coded = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        {
            let mut writer = plan.charset().writer(&mut coded);
            plan.write_prolog(&mut writer)?;
            crate::text::into_writer_all_with_formatting(
                values.iter(),
                &mut writer,
                plan.format(),
                formatting,
            )?;
            writer.finish()?;
        }
        coded.finish()?;
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
    let mut head = Vec::with_capacity(crate::media::MAGIC_PROBE_LEN);
    {
        let mut probe = std::io::Read::take(&mut encoded, crate::media::MAGIC_PROBE_LEN as u64);
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
    let mut decompressed = plan.codec().reader(replayed);

    // The mark sits under the coding, so it is looked for here rather than in
    // the probe above, which is still compressed. Precedence is the one every
    // intake follows: what the handle declared first, then this content read -
    // the mark, and for XML the declaration, which is the one document that
    // states its own charset. Either way the framing comes off the stream: a
    // parser that met `U+FEFF` before the first token would refuse it, and the
    // declaration is read as any other processing instruction is.
    let mut head = vec![0_u8; MARK_LEN];
    let mut filled = fill(&mut decompressed, &mut head)?;
    let declared = source.media_type().charset();
    let (charset, mark) = match Charset::from_bom(&head[..filled]) {
        Some((found, length)) => (declared.unwrap_or(found), length),
        None => match declared {
            Some(declared) => (declared, 0),
            None if plan.format() == Format::Xml => {
                head.resize(DECLARATION_PROBE_LEN, 0);
                filled += fill(&mut decompressed, &mut head[filled..])?;
                (
                    crate::xml::declared_charset(&head[..filled])?.unwrap_or_default(),
                    0,
                )
            }
            None => (Charset::default(), 0),
        },
    };
    head.truncate(filled);
    head.drain(..mark);
    let replayed = Cursor::new(head).chain(decompressed);
    Ok((charset.reader(replayed), plan.with_charset(charset)))
}

/// The longest byte-order mark, which bounds the replayed prefix.
const MARK_LEN: usize = crate::charset::MARK_LEN;

/// How far into an XML document its declaration can reach: `<?xml`, a
/// version, an encoding name and a standalone flag, quoted and spaced, fit
/// well within it.
const DECLARATION_PROBE_LEN: usize = 128;

/// Read until `target` is full or the source ends, answering what was filled.
///
/// `Read::read` may answer short for reasons of its own, so a mark split
/// across two reads would otherwise go unrecognized.
fn fill(source: &mut impl Read, target: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < target.len() {
        let read = source.read(&mut target[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
