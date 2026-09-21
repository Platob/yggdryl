//! Content-based representation inference from leading bytes.
//!
//! Filename inference answers what a location *claims* to be; magic bytes
//! answer what a payload *is*. The two disagree often enough - a `.json` file
//! that is really gzip, a `.bin` that is really Parquet - that a reader with
//! access to the content should prefer this module.
//!
//! Inference is recursive: a gzip payload wrapping JSON reports both codings
//! in application order through [`MediaType`], so one call recovers the whole
//! stack a reader must unwrap.

use crate::{Charset, MediaType, MimeType};

/// Bytes that must be inspected to identify every supported signature.
///
/// A reader that cannot rewind should buffer at least this many bytes before
/// calling [`MimeType::from_magic_bytes`].
pub const MAGIC_PROBE_LEN: usize = 64;

/// How many nested content codings [`MediaType::from_magic_bytes`] will peel.
///
/// A payload nested more deeply than this is reported at the depth reached,
/// which bounds the work an adversarial input can cause.
const MAX_NESTED_CODINGS: usize = 4;

/// One signature: a byte pattern at a fixed offset.
struct Signature {
    offset: usize,
    pattern: &'static [u8],
    mime: fn() -> MimeType,
}

/// Signatures ordered longest-pattern-first so a specific match wins.
///
/// Every entry is a byte-exact prefix documented by its format specification.
static SIGNATURES: &[Signature] = &[
    // Container and columnar formats.
    Signature {
        offset: 0,
        pattern: b"PAR1",
        mime: || MimeType::PARQUET,
    },
    Signature {
        offset: 0,
        pattern: b"ARROW1",
        mime: || MimeType::ARROW_FILE,
    },
    Signature {
        offset: 0,
        pattern: b"Obj\x01",
        mime: || MimeType::AVRO,
    },
    Signature {
        offset: 0,
        pattern: b"ORC",
        mime: || MimeType::ORC,
    },
    Signature {
        offset: 0,
        pattern: b"PFA1",
        mime: || MimeType::PUFFIN,
    },
    Signature {
        offset: 0,
        pattern: b"SQLite format 3\0",
        mime: || MimeType::SQLITE3,
    },
    // Content codings.
    Signature {
        offset: 0,
        pattern: &[0x1F, 0x8B],
        mime: || MimeType::GZIP,
    },
    Signature {
        offset: 0,
        pattern: &[0x28, 0xB5, 0x2F, 0xFD],
        mime: || MimeType::ZSTD,
    },
    // Documents and media.
    Signature {
        offset: 0,
        pattern: b"%PDF-",
        mime: || MimeType::PDF,
    },
    Signature {
        offset: 0,
        pattern: &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        mime: || MimeType::PNG,
    },
    Signature {
        offset: 0,
        pattern: &[0xFF, 0xD8, 0xFF],
        mime: || MimeType::JPEG,
    },
    Signature {
        offset: 0,
        pattern: b"GIF87a",
        mime: || MimeType::GIF,
    },
    Signature {
        offset: 0,
        pattern: b"GIF89a",
        mime: || MimeType::GIF,
    },
    Signature {
        offset: 8,
        pattern: b"WEBP",
        mime: || MimeType::WEBP,
    },
    Signature {
        offset: 0,
        pattern: b"OggS",
        mime: || MimeType::OGG,
    },
    Signature {
        offset: 0,
        pattern: b"fLaC",
        mime: || MimeType::FLAC,
    },
    Signature {
        offset: 0,
        pattern: b"ID3",
        mime: || MimeType::MP3,
    },
    Signature {
        offset: 8,
        pattern: b"WAVE",
        mime: || MimeType::WAV,
    },
    Signature {
        offset: 4,
        pattern: b"ftyp",
        mime: || MimeType::MP4,
    },
    Signature {
        offset: 0,
        pattern: b"wOFF",
        mime: || MimeType::WOFF,
    },
    Signature {
        offset: 0,
        pattern: b"wOF2",
        mime: || MimeType::WOFF2,
    },
    Signature {
        offset: 0,
        pattern: b"OTTO",
        mime: || MimeType::OTF,
    },
    Signature {
        offset: 0,
        pattern: &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        mime: || MimeType::XLS,
    },
];

/// Zlib's first byte encodes a compression method and window size; the
/// two-byte header must also be a multiple of 31.
fn is_zlib_header(input: &[u8]) -> bool {
    let [first, second, ..] = input else {
        return false;
    };
    // Only method 8 (DEFLATE) is defined, and only window sizes up to 32 KiB.
    first & 0x0F == 0x08
        && first >> 4 <= 7
        && (u16::from(*first) * 256 + u16::from(*second)) % 31 == 0
}

impl MimeType {
    /// Identify a representation from a payload's leading bytes.
    ///
    /// Returns `None` when no signature matches, which includes every textual
    /// format; use [`Self::from_text_bytes`] for those. Only the first
    /// [`MAGIC_PROBE_LEN`] bytes are examined.
    pub fn from_magic_bytes(input: &[u8]) -> Option<Self> {
        let probe = &input[..input.len().min(MAGIC_PROBE_LEN)];
        SIGNATURES
            .iter()
            .filter(|signature| {
                probe
                    .get(signature.offset..signature.offset + signature.pattern.len())
                    .is_some_and(|window| window == signature.pattern)
            })
            // Prefer the longest match so `GIF89a` beats a shorter prefix.
            .max_by_key(|signature| signature.pattern.len())
            .map(|signature| (signature.mime)())
            .or_else(|| is_zlib_header(probe).then_some(Self::ZLIB))
    }

    /// Identify a textual representation from a payload's leading bytes.
    ///
    /// Textual formats have no byte signature, so this is a deliberately
    /// conservative structural sniff: it reports a type only when the first
    /// non-whitespace bytes cannot plausibly belong to another text format.
    /// Returns `None` when the payload is not valid UTF-8 or is ambiguous.
    pub fn from_text_bytes(input: &[u8]) -> Option<Self> {
        // A byte-order mark names the encoding of what follows it, and is
        // framing rather than content, so it is taken off before the sniff.
        // Without one the probe is read as UTF-8, which every ASCII-compatible
        // charset agrees with over the handful of bytes a sniff looks at.
        let (charset, mark) = Charset::from_bom(input).unwrap_or((Charset::Utf8, 0));
        let probe = input.get(mark..)?;
        let probe = &probe[..probe.len().min(MAGIC_PROBE_LEN)];
        // A probe cut mid-sequence is read up to the cut rather than refused:
        // what is being looked for is the first scalar, not a whole document.
        let decoded = charset.decode_lossy(probe);
        let text = decoded.trim_start();

        let first = text.as_bytes().first()?;
        match first {
            b'{' | b'[' => Some(Self::JSON),
            b'<' => {
                if text.starts_with("<?xml") || text.starts_with("<!DOCTYPE") {
                    Some(Self::XML)
                } else if text.starts_with("<svg") {
                    Some(Self::SVG)
                } else if text.starts_with("<html") || text.starts_with("<!doctype html") {
                    Some(Self::HTML)
                } else {
                    Some(Self::XML)
                }
            }
            // A YAML document marker is unambiguous; a bare mapping is not.
            b'-' if text.starts_with("---") => Some(Self::YAML),
            b'%' if text.starts_with("%YAML") => Some(Self::YAML),
            _ => None,
        }
    }

    /// Identify a representation from content, falling back to a text sniff.
    ///
    /// This is the accessor a reader with bytes in hand should call.
    pub fn from_bytes(input: &[u8]) -> Option<Self> {
        Self::from_magic_bytes(input).or_else(|| Self::from_text_bytes(input))
    }
}

impl MediaType {
    /// Recover the complete representation and coding stack from content.
    ///
    /// Content codings are peeled recursively and reported in application
    /// order, so a JSON payload compressed with gzip returns a JSON base with
    /// one `gzip` encoding. Peeling stops after a bounded number of layers.
    ///
    /// Returns `None` only when the outermost bytes match nothing at all. A
    /// coding whose payload is unidentifiable is reported as the base type,
    /// because the coding itself is still a fact about the payload.
    pub fn from_magic_bytes(input: &[u8]) -> Option<Self> {
        let mut encodings: Vec<MimeType> = Vec::new();
        let mut current = input.to_vec();

        for _ in 0..MAX_NESTED_CODINGS {
            let Some(mime) = MimeType::from_bytes(&current) else {
                // The innermost payload is opaque, so the coding that wraps it
                // becomes the base rather than being discarded.
                let last = encodings.pop()?;
                return Some(build(last, encodings));
            };
            if !mime.is_encoding() {
                return Some(build(mime, encodings));
            }

            let codec = crate::Codec::from_mime_type(&mime);
            // Decode only the head: identifying the inner type needs a probe,
            // not the whole payload.
            match decode_probe(codec, &current) {
                Some(inner) if !inner.is_empty() => {
                    encodings.push(mime);
                    current = inner;
                }
                // The coding is real but its payload is unreadable or empty;
                // report the coding itself rather than guessing deeper.
                _ => return Some(build(mime, encodings)),
            }
        }

        // The nesting limit was reached; report what the last layer looks like.
        if let Some(mime) = MimeType::from_bytes(&current) {
            return Some(build(mime, encodings));
        }
        let last = encodings.pop()?;
        Some(build(last, encodings))
    }
}

/// Assemble a media type from a base and the codings applied over it.
fn build(base: MimeType, encodings: Vec<MimeType>) -> MediaType {
    MediaType::from(base)
        .try_with_encodings(encodings)
        .unwrap_or_else(|_| MediaType::from(MimeType::OCTET_STREAM))
}

/// Decode enough of a coded payload to identify what is inside it.
fn decode_probe(codec: crate::Codec, input: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read as _;

    let mut probe = vec![0_u8; MAGIC_PROBE_LEN];
    let mut reader = codec.reader(input);
    let mut filled = 0;
    while filled < probe.len() {
        match reader.read(&mut probe[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            // A short or malformed stream still identifies the outer coding.
            Err(_) => break,
        }
    }
    probe.truncate(filled);
    (filled > 0).then_some(probe)
}
