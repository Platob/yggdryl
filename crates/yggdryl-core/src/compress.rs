//! Streaming decompression helpers — one per codec, each behind its own feature gate.
//!
//! `decode_reader` wraps any `Read` in the appropriate streaming decoder for the
//! given `Content-Encoding` value, or returns the reader unchanged when the encoding
//! is `identity`, unknown, or the matching feature is off.
//!
//! ```no_run
//! # #[cfg(feature = "gzip")] {
//! use yggdryl_core::compress;
//! let raw: &[u8] = &[];
//! let mut decoded = compress::decode_reader(raw, "gzip");
//! // `decoded` implements `std::io::Read` and streams decompressed bytes.
//! # }
//! ```

use std::io::Read;

/// Wraps `reader` with a streaming decoder for `encoding`.
///
/// Recognised values (case-insensitive):
/// - `"gzip"` / `"x-gzip"` — requires the `gzip` feature
/// - `"zstd"` — requires the `zstd` feature
/// - `"snappy"` — requires the `snappy` feature
/// - `"br"` / `"brotli"` — requires the `brotli` feature
///
/// Unknown or `"identity"` encodings return the reader unchanged.
/// When a known encoding is requested but the matching feature is off,
/// the reader is also returned unchanged (the response is not decoded).
pub fn decode_reader<R: Read + Send + 'static>(
    reader: R,
    encoding: &str,
) -> Box<dyn Read + Send + 'static> {
    let encoding_lower = encoding.to_ascii_lowercase();
    match encoding_lower.as_str() {
        "gzip" | "x-gzip" => decode_gzip(reader),
        "zstd" => decode_zstd(reader),
        "snappy" => decode_snappy(reader),
        "br" | "brotli" => decode_brotli(reader),
        _ => Box::new(reader),
    }
}

#[cfg(feature = "gzip")]
fn decode_gzip<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    Box::new(flate2::read::GzDecoder::new(reader))
}

#[cfg(not(feature = "gzip"))]
fn decode_gzip<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    crate::log_event!(warn, "gzip Content-Encoding received but the `gzip` feature is off; response body is not decoded");
    Box::new(reader)
}

#[cfg(feature = "zstd")]
fn decode_zstd<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    match zstd::Decoder::new(reader) {
        Ok(dec) => Box::new(dec),
        Err(err) => {
            crate::log_event!(
                warn,
                "zstd decoder init failed: {err}; returning raw reader"
            );
            // Decoder init failed (shouldn't happen for an in-memory decoder);
            // return a reader that will produce an IO error on first read.
            Box::new(ErrorReader(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("zstd init: {err}"),
            )))
        }
    }
}

#[cfg(not(feature = "zstd"))]
fn decode_zstd<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    crate::log_event!(warn, "zstd Content-Encoding received but the `zstd` feature is off; response body is not decoded");
    Box::new(reader)
}

#[cfg(feature = "snappy")]
fn decode_snappy<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    Box::new(snap::read::FrameDecoder::new(reader))
}

#[cfg(not(feature = "snappy"))]
fn decode_snappy<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    crate::log_event!(warn, "snappy Content-Encoding received but the `snappy` feature is off; response body is not decoded");
    Box::new(reader)
}

#[cfg(feature = "brotli")]
fn decode_brotli<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    Box::new(brotli::Decompressor::new(reader, 4096))
}

#[cfg(not(feature = "brotli"))]
fn decode_brotli<R: Read + Send + 'static>(reader: R) -> Box<dyn Read + Send + 'static> {
    crate::log_event!(warn, "brotli Content-Encoding received but the `brotli` feature is off; response body is not decoded");
    Box::new(reader)
}

/// A reader that immediately returns the stored error on any read.
struct ErrorReader(std::io::Error);

impl Read for ErrorReader {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(self.0.kind(), self.0.to_string()))
    }
}
