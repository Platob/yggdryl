//! The `Compression` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::Compression as CoreCompression;

use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::mime::MimeType;

/// A byte-stream compression codec — `gzip`, `deflate` (zlib), `zstd`, `snappy`
/// or `brotli` (or `none`, the identity codec) — that compresses and decompresses
/// bytes. The backends are optional Cargo features in the core, so a codec may
/// parse and name itself yet report `isAvailable` `false` when its backend was
/// not compiled in.
#[napi]
pub struct Compression {
    pub(crate) inner: CoreCompression,
}

#[napi]
impl Compression {
    /// Parse a codec name — `none` / `identity` / `store`, `gzip` / `gz`,
    /// `deflate` / `zlib` / `zz`, `zstd` / `zst`, `snappy` / `snap` / `sz`,
    /// `brotli` / `br` — throwing on an unknown one.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreCompression::from_str(&value)
            .map(|inner| Compression { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        Compression::new(value)
    }

    /// Infer the codec from a file `extension` (`gz` / `zst` / `sz`, with or
    /// without a leading dot), or `null` if it names no known codec.
    #[napi(js_name = "fromExtension")]
    pub fn from_extension(extension: String) -> Option<Compression> {
        CoreCompression::from_extension(&extension).map(|inner| Compression { inner })
    }

    /// Infer the codec from a `MimeType` (e.g. `application/gzip` → `gzip`), or
    /// `null` if the MIME names no supported codec.
    #[napi(js_name = "fromMime")]
    pub fn from_mime(mime: &MimeType) -> Option<Compression> {
        CoreCompression::from_mime(&mime.inner).map(|inner| Compression { inner })
    }

    /// Infer the codec from a layered `MediaType` stack — its outermost (container)
    /// MIME, e.g. `gzip` for `data.csv.gz` — or `null`.
    #[napi(js_name = "fromMedia")]
    pub fn from_media(media: &MediaType) -> Option<Compression> {
        CoreCompression::from_media(&media.inner).map(|inner| Compression { inner })
    }

    /// Infer the codec from an `IoStats` — its discovered media type first, then
    /// its transport content type — or `null` if neither names a codec.
    #[napi(js_name = "fromStats")]
    pub fn from_stats(stats: &IoStats) -> Option<Compression> {
        CoreCompression::from_stats(&stats.inner).map(|inner| Compression { inner })
    }

    /// The `MimeType` this codec is carried as — the inverse of `fromMime`, used to
    /// add an encoding layer to a media type. `null` for the identity codec and
    /// `deflate` / `snappy` (which have no registered MIME).
    #[napi]
    pub fn mime(&self) -> Option<MimeType> {
        self.inner.mime().map(|inner| MimeType { inner })
    }

    /// The canonical codec name (`"none"` / `"gzip"` / `"deflate"` / `"zstd"` /
    /// `"snappy"` / `"brotli"`).
    #[napi(getter)]
    pub fn name(&self) -> &'static str {
        self.inner.as_str()
    }

    /// The conventional file extension (`"gz"` / `"zz"` / `"zst"` / `"sz"` /
    /// `"br"`), or `null` for the identity codec.
    #[napi(getter)]
    pub fn extension(&self) -> Option<&'static str> {
        self.inner.extension()
    }

    /// Whether this codec's backend is compiled in, so `compress` / `decompress`
    /// will work. `none` is always available.
    #[napi(getter, js_name = "isAvailable")]
    pub fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    /// Compress `data` in full and return the encoded bytes. Throws if this
    /// codec's backend is not available.
    #[napi]
    pub fn compress(&self, data: Buffer) -> Result<Buffer> {
        self.inner
            .compress(data.as_ref())
            .map(Buffer::from)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Decompress `data` in full and return the decoded bytes. Throws if this
    /// codec's backend is not available or the data is malformed.
    #[napi]
    pub fn decompress(&self, data: Buffer) -> Result<Buffer> {
        self.inner
            .decompress(data.as_ref())
            .map(Buffer::from)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialise to JSON as the codec name (used by `JSON.stringify`). `fromJSON`
    /// is the inverse.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.as_str().to_string()
    }

    /// Reconstruct from the value produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        Compression::new(value)
    }
}
