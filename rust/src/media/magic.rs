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

use crate::{MediaType, MimeType};

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
        let probe = &input[..input.len().min(MAGIC_PROBE_LEN)];
        // A BOM or invalid UTF-8 prefix means this is not text worth sniffing.
        let text = std::str::from_utf8(probe)
            .ok()
            .or_else(|| {
                // A truncated probe may split a character; retry on the valid prefix.
                std::str::from_utf8(probe)
                    .err()
                    .map(|error| error.valid_up_to())
                    .and_then(|valid| std::str::from_utf8(&probe[..valid]).ok())
            })?
            .trim_start_matches('\u{feff}')
            .trim_start();

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

#[cfg(test)]
mod tests {
    use super::MAGIC_PROBE_LEN;
    use crate::coding::{gzip, zstd};
    use crate::{MediaType, MimeType};

    #[test]
    fn container_signatures_are_identified() {
        assert_eq!(
            MimeType::from_magic_bytes(b"PAR1data"),
            Some(MimeType::PARQUET)
        );
        assert_eq!(
            MimeType::from_magic_bytes(b"ARROW1\0\0"),
            Some(MimeType::ARROW_FILE)
        );
        assert_eq!(
            MimeType::from_magic_bytes(b"Obj\x01rest"),
            Some(MimeType::AVRO)
        );
        assert_eq!(MimeType::from_magic_bytes(b"ORCrest"), Some(MimeType::ORC));
        assert_eq!(
            MimeType::from_magic_bytes(b"PFA1rest"),
            Some(MimeType::PUFFIN)
        );
        assert_eq!(
            MimeType::from_magic_bytes(b"SQLite format 3\0rest"),
            Some(MimeType::SQLITE3)
        );
        assert_eq!(MimeType::from_magic_bytes(b"%PDF-1.7"), Some(MimeType::PDF));
    }

    #[test]
    fn offset_signatures_respect_their_offset() {
        // `WEBP` lives at byte 8, after the RIFF header.
        let mut webp = Vec::from(*b"RIFF\0\0\0\0WEBPVP8 ");
        assert_eq!(MimeType::from_magic_bytes(&webp), Some(MimeType::WEBP));

        // The same token at offset 0 must not match.
        webp = Vec::from(*b"WEBPRIFF\0\0\0\0");
        assert_ne!(MimeType::from_magic_bytes(&webp), Some(MimeType::WEBP));
    }

    #[test]
    fn coding_signatures_are_identified() {
        let gzipped = gzip::dump(b"hello").unwrap();
        assert_eq!(MimeType::from_magic_bytes(&gzipped), Some(MimeType::GZIP));

        let zstd_payload = zstd::dump(b"hello").unwrap();
        assert_eq!(
            MimeType::from_magic_bytes(&zstd_payload),
            Some(MimeType::ZSTD)
        );

        let zlib_payload = crate::coding::zlib::dump(b"hello").unwrap();
        assert_eq!(
            MimeType::from_magic_bytes(&zlib_payload),
            Some(MimeType::ZLIB)
        );
    }

    #[test]
    fn textual_formats_are_sniffed_structurally() {
        assert_eq!(
            MimeType::from_text_bytes(b"  {\"a\":1}"),
            Some(MimeType::JSON)
        );
        assert_eq!(MimeType::from_text_bytes(b"[1,2,3]"), Some(MimeType::JSON));
        assert_eq!(
            MimeType::from_text_bytes(b"<?xml version=\"1.0\"?>"),
            Some(MimeType::XML)
        );
        assert_eq!(
            MimeType::from_text_bytes(b"<svg xmlns="),
            Some(MimeType::SVG)
        );
        assert_eq!(
            MimeType::from_text_bytes(b"---\nkey: value"),
            Some(MimeType::YAML)
        );

        // A bare key/value line is ambiguous between YAML and TOML.
        assert_eq!(MimeType::from_text_bytes(b"key = 1"), None);
        assert_eq!(MimeType::from_text_bytes(b"plain prose"), None);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_content() {
        let mut input = Vec::from("\u{feff}".as_bytes());
        input.extend_from_slice(b"{\"a\":1}");
        assert_eq!(MimeType::from_text_bytes(&input), Some(MimeType::JSON));
    }

    #[test]
    fn nested_codings_are_peeled_in_application_order() {
        let json = br#"{"symbol":"AAPL"}"#;
        let gzipped = gzip::dump(json).unwrap();

        let media = MediaType::from_magic_bytes(&gzipped).expect("gzip of json");
        assert_eq!(media.base(), &MimeType::JSON);
        assert_eq!(media.encodings(), &[MimeType::GZIP]);

        // Two layers: gzip over zstd over JSON.
        let doubled = gzip::dump(&zstd::dump(json).unwrap()).unwrap();
        let media = MediaType::from_magic_bytes(&doubled).expect("gzip of zstd of json");
        assert_eq!(media.base(), &MimeType::JSON);
        assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
    }

    #[test]
    fn a_coding_over_an_unknown_payload_reports_the_coding() {
        let opaque = gzip::dump(b"plain prose with no signature").unwrap();
        let media = MediaType::from_magic_bytes(&opaque).expect("gzip of prose");
        assert_eq!(media.base(), &MimeType::GZIP);
    }

    #[test]
    fn nothing_is_inferred_from_an_empty_or_unknown_payload() {
        assert_eq!(MimeType::from_magic_bytes(b""), None);
        assert_eq!(MimeType::from_bytes(b""), None);
        assert_eq!(MimeType::from_bytes(&[0xAB, 0xCD, 0xEF]), None);
        assert_eq!(MediaType::from_magic_bytes(b""), None);
    }

    #[test]
    fn only_the_probe_window_is_examined() {
        // A signature past the probe window must not be found.
        let mut buried = vec![0_u8; MAGIC_PROBE_LEN * 2];
        buried.extend_from_slice(b"PAR1");
        assert_eq!(MimeType::from_magic_bytes(&buried), None);
    }
}
