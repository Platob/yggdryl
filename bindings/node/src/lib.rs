//! Node extension for yggdryl — a thin napi wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example, plus the `yggdryl.io` namespace
//! (the root value types: the `Headers` metadata map and the `IOMode` / `IOKind` int enums),
//! the `yggdryl.uri` namespace (RFC 3986 URIs, absolute URLs, and authorities), mirroring
//! `yggdryl_core::uri`, the `yggdryl.mimetype` namespace (the `MimeType` media type and its
//! `MimeCatalog` registry) and the `yggdryl.mediatype` namespace (the layered `MediaType`),
//! mirroring `yggdryl_core::mimetype` / `yggdryl_core::mediatype`, the `yggdryl.compression`
//! namespace (the `Gzip` / `Zlib` / `Zstd` / `Lzma` codecs), mirroring
//! `yggdryl_core::compression`, the `yggdryl.memory` namespace (the in-heap `Heap` byte source
//! and the `Whence` seek anchor), mirroring `yggdryl_core::io::memory`, and the `yggdryl.local`
//! namespace (the local-filesystem family: the lazy `LocalIO` single access point and the raw
//! memory-mapped `Mmap` it builds on), mirroring `yggdryl_core::io::local`.

#[macro_use]
extern crate napi_derive;

pub mod compression;
pub mod headers;
pub mod io;
pub mod mediatype;
pub mod mimetype;
pub mod uri;

use napi::bindgen_prelude::{Either, Either4, Uint8Array};

use crate::io::local::LocalIO;
use crate::io::memory::Heap;
use crate::uri::{Uri, Url};
use yggdryl_core::io::{open as core_open, open_str as core_open_str, AnyIO};

/// The library version string — delegates to [`yggdryl_core::version`].
#[napi]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}

/// Maps any core error to a thrown JS `Error` (its guided text).
fn open_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Redirects an opened [`AnyIO`] to the concrete binding class the scheme selected — a
/// [`Heap`] for `mem://`, a [`LocalIO`] for `file://` / a plain path.
fn any_to_source(any: AnyIO) -> Either<Heap, LocalIO> {
    match any.into_memory() {
        Ok(inner) => Either::A(Heap { inner }),
        Err(any) => Either::B(LocalIO {
            inner: any
                .into_local()
                .expect("an AnyIO is local when it is not in-memory"),
        }),
    }
}

/// **Opens** `target` into its concrete source — the project's `open()`. Dispatches by runtime
/// type and scheme, returning the concrete binding class:
///
/// - a **string** — a `mem://` URI opens a [`Heap`]; a `file://` URI or a plain path opens a
///   [`LocalIO`] (an unsupported scheme throws the core's guided `Error`);
/// - a **`Buffer` / `Uint8Array`** opens an in-memory [`Heap`] over a copy of its bytes;
/// - a **[`Uri`]** / **[`Url`]** dispatches on its scheme exactly like the string form.
#[napi(
    ts_args_type = "target: string | Uint8Array | uri.Uri | uri.Url",
    ts_return_type = "memory.Heap | local.LocalIO"
)]
pub fn open(
    target: Either4<String, Uint8Array, &Uri, &Url>,
) -> napi::Result<Either<Heap, LocalIO>> {
    let any = match target {
        Either4::A(target) => core_open_str(&target).map_err(open_error)?,
        Either4::B(bytes) => {
            return Ok(Either::A(Heap {
                inner: yggdryl_core::io::memory::Heap::from_slice(bytes.as_ref()),
            }))
        }
        Either4::C(uri) => core_open(&uri.inner).map_err(open_error)?,
        Either4::D(url) => core_open(url.inner.as_uri()).map_err(open_error)?,
    };
    Ok(any_to_source(any))
}
