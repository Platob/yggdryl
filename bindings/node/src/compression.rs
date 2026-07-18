//! The `yggdryl.compression` namespace — the codec contract mirrored as four concrete classes:
//! [`Gzip`], [`Zlib`], [`Zstd`], and [`Lzma`], each compressing / decompressing a `Buffer`.
//!
//! Mirrors `yggdryl_core::compression` (the `compression` feature, which this binding enables,
//! so every codec resolves). The abstract `Compression` trait is not itself a JS type — a codec
//! is a **concrete class** you construct (`new compression.Gzip(level?)`) or resolve from a
//! media type with [`codecFor`](codec_for). Every failing compress / decompress surfaces as a
//! thrown `Error` carrying the core's guided text unchanged.
//!
//! The compression level is a JS `number` (an `i32`, the natural signed JS number): `0`..`9`
//! for gzip / zlib / xz and `1`..`22` for zstd, each clamped by the core to its own range; a
//! negative is clamped to the codec's minimum. Omitting it uses the codec's balanced default.
//! Codecs are stateless config values (no byte identity), so they deliberately carry no
//! `equals` / `hashCode` / `serializeBytes` — only `compress` / `decompress`, the `essence` /
//! `name` getters, and `toString`.

use napi::bindgen_prelude::{Buffer, Either, Either4};
use napi_derive::napi;

use crate::mimetype::MimeType;
use yggdryl_core::compression::{self as core, Compression};

/// Maps any core error to a thrown JS `Error` (its guided text).
pub(crate) fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The JS-facing union of the four concrete codec classes — the return of
/// [`codecFor`](codec_for) and of every source's `compression()` (a codec or `null`). Written
/// out in each napi signature (napi generates `Gzip | Zlib | Zstd | Lzma`); this alias is only
/// used from the io modules, where the `.d.ts` is not derived from it.
pub(crate) type Codec = Either4<Gzip, Zlib, Zstd, Lzma>;

/// The polymorphic `codec` **argument** of every source's `compressWith` / `decompressWith` —
/// a borrow of one of the four codec classes. Written out in each napi signature; this alias is
/// only used from the io modules.
pub(crate) type CodecArg<'a> = Either4<&'a Gzip, &'a Zlib, &'a Zstd, &'a Lzma>;

/// Borrows the concrete core codec behind a JS codec argument as a `&dyn Compression` — the one
/// place the `Either4` of codec classes is collapsed to the trait object the io helpers take.
pub(crate) fn as_dyn<'a>(codec: CodecArg<'a>) -> &'a dyn Compression {
    match codec {
        Either4::A(c) => &c.inner,
        Either4::B(c) => &c.inner,
        Either4::C(c) => &c.inner,
        Either4::D(c) => &c.inner,
    }
}

/// The codec class matching a mime **essence**, or `None` when the essence is not a supported
/// compression. Delegates the "is this a compression?" decision to
/// [`core::codec_for`](yggdryl_core::compression::codec_for) (which honors the `compression`
/// feature), then picks the concrete class by essence — the shared resolution behind
/// [`codecFor`](codec_for) and every source's `compression()`.
pub(crate) fn wrap_codec(essence: &str) -> Option<Codec> {
    core::codec_for(essence)?; // the supported-essence + feature decision, from the core
    Some(match essence {
        "application/gzip" => Either4::A(Gzip {
            inner: core::Gzip::new(),
        }),
        "application/zlib" => Either4::B(Zlib {
            inner: core::Zlib::new(),
        }),
        "application/zstd" => Either4::C(Zstd {
            inner: core::Zstd::new(),
        }),
        // The only remaining supported essences are xz / lzma (both the Lzma codec).
        _ => Either4::D(Lzma {
            inner: core::Lzma::new(),
        }),
    })
}

/// The [`Compression`] codec for a media type, or `null` when it is not a supported
/// compression — the generic, type-inferring entry: a **string** is treated as a mime
/// **essence** (`"application/gzip"`), a [`MimeType`] resolves through its essence. Mirrors
/// `yggdryl_core::compression::codec_for` / `codec_for_mime`.
#[napi(namespace = "compression")]
pub fn codec_for(
    mime_or_essence: Either<String, &MimeType>,
) -> Option<Either4<Gzip, Zlib, Zstd, Lzma>> {
    match mime_or_essence {
        Either::A(essence) => wrap_codec(&essence),
        Either::B(mime) => wrap_codec(mime.inner.essence()),
    }
}

/// **Gzip** (RFC 1952) over the native DEFLATE core. Construct at a level `0`..`9` (default 6).
#[napi(namespace = "compression")]
pub struct Gzip {
    pub(crate) inner: core::Gzip,
}

#[napi(namespace = "compression")]
impl Gzip {
    /// A gzip codec at compression `level` (`0` fastest/none … `9` smallest); the balanced
    /// default (6) when omitted. A negative level is clamped to `0`.
    #[napi(constructor)]
    pub fn new(level: Option<i32>) -> Self {
        Self {
            inner: match level {
                Some(level) => core::Gzip::with_level(level.max(0) as u32),
                None => core::Gzip::new(),
            },
        }
    }

    /// The codec's mime **essence** (`"application/gzip"`).
    #[napi(getter)]
    pub fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The codec's short **name** (`"gzip"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Compresses `data` into a new `Buffer`.
    #[napi]
    pub fn compress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .compress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// Decompresses `data` (a gzip stream) into a new `Buffer`, or throws a guided `Error` on
    /// corrupt / truncated input.
    #[napi]
    pub fn decompress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .decompress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// The codec's short name (`"gzip"`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.name().to_string()
    }
}

/// **Zlib** (RFC 1950) over the native DEFLATE core. Construct at a level `0`..`9` (default 6).
#[napi(namespace = "compression")]
pub struct Zlib {
    pub(crate) inner: core::Zlib,
}

#[napi(namespace = "compression")]
impl Zlib {
    /// A zlib codec at compression `level` (`0` … `9`); the balanced default (6) when omitted.
    /// A negative level is clamped to `0`.
    #[napi(constructor)]
    pub fn new(level: Option<i32>) -> Self {
        Self {
            inner: match level {
                Some(level) => core::Zlib::with_level(level.max(0) as u32),
                None => core::Zlib::new(),
            },
        }
    }

    /// The codec's mime **essence** (`"application/zlib"`).
    #[napi(getter)]
    pub fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The codec's short **name** (`"zlib"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Compresses `data` into a new `Buffer`.
    #[napi]
    pub fn compress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .compress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// Decompresses `data` (a zlib stream) into a new `Buffer`, or throws a guided `Error` on
    /// corrupt / truncated input.
    #[napi]
    pub fn decompress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .decompress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// The codec's short name (`"zlib"`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.name().to_string()
    }
}

/// **Zstandard** over the native `libzstd` core. Construct at a level `1`..`22` (default 3).
#[napi(namespace = "compression")]
pub struct Zstd {
    pub(crate) inner: core::Zstd,
}

#[napi(namespace = "compression")]
impl Zstd {
    /// A zstd codec at compression `level` (`1` fastest … `22` smallest); the balanced default
    /// (3) when omitted. The level is clamped to `1`..`22`.
    #[napi(constructor)]
    pub fn new(level: Option<i32>) -> Self {
        Self {
            inner: match level {
                Some(level) => core::Zstd::with_level(level),
                None => core::Zstd::new(),
            },
        }
    }

    /// The codec's mime **essence** (`"application/zstd"`).
    #[napi(getter)]
    pub fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The codec's short **name** (`"zstd"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Compresses `data` into a new `Buffer`.
    #[napi]
    pub fn compress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .compress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// Decompresses `data` (a zstd stream) into a new `Buffer`, or throws a guided `Error` on
    /// corrupt / truncated input.
    #[napi]
    pub fn decompress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .decompress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// The codec's short name (`"zstd"`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.name().to_string()
    }
}

/// **LZMA / XZ** over the native `liblzma` core. Construct at a preset `0`..`9` (default 6).
#[napi(namespace = "compression")]
pub struct Lzma {
    pub(crate) inner: core::Lzma,
}

#[napi(namespace = "compression")]
impl Lzma {
    /// An xz codec at compression `level` / preset (`0` fastest … `9` smallest); the balanced
    /// default (6) when omitted. A negative level is clamped to `0`.
    #[napi(constructor)]
    pub fn new(level: Option<i32>) -> Self {
        Self {
            inner: match level {
                Some(level) => core::Lzma::with_level(level.max(0) as u32),
                None => core::Lzma::new(),
            },
        }
    }

    /// The codec's mime **essence** (`"application/x-xz"`).
    #[napi(getter)]
    pub fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The codec's short **name** (`"xz"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Compresses `data` into a new `Buffer`.
    #[napi]
    pub fn compress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .compress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// Decompresses `data` (an xz stream) into a new `Buffer`, or throws a guided `Error` on
    /// corrupt / truncated input.
    #[napi]
    pub fn decompress(&self, data: Buffer) -> napi::Result<Buffer> {
        self.inner
            .decompress(data.as_ref())
            .map(Into::into)
            .map_err(to_error)
    }

    /// The codec's short name (`"xz"`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.name().to_string()
    }
}
