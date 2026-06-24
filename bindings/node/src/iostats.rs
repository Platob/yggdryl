//! The `IoStats` napi class.

use std::time::UNIX_EPOCH;

use napi_derive::napi;
use yggdryl_io::IoStats as CoreIoStats;

use crate::media::MediaType;

/// Lazily-discovered metadata for an IO handle: `size` / `mtime` /
/// `contentType` / `etag` are cheap and eager, while `mediaType` is discovered
/// only when the handle is asked for it.
#[napi(js_name = "IoStats")]
pub struct IoStats {
    pub(crate) inner: CoreIoStats,
}

#[napi]
impl IoStats {
    /// What the resource is: `"missing"`, `"file"`, `"directory"` or `"other"`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().as_str().to_owned()
    }

    /// Whether the resource exists (its `kind` is not `"missing"`).
    #[napi(getter)]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// Whether the resource is a regular file (or in-memory blob).
    #[napi(getter, js_name = "isFile")]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the resource is a directory.
    #[napi(getter, js_name = "isDir")]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// The size in bytes.
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.size() as f64
    }

    /// The last-modified time as a Unix timestamp in seconds, or `null`.
    #[napi(getter)]
    pub fn mtime(&self) -> Option<f64> {
        self.inner
            .mtime()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs_f64())
    }

    /// The transport content type (e.g. a cloud `Content-Type`), or `null`.
    #[napi(getter, js_name = "contentType")]
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_owned)
    }

    /// The backend entity tag (e.g. an object-store `ETag`), or `null`.
    #[napi(getter)]
    pub fn etag(&self) -> Option<String> {
        self.inner.etag().map(str::to_owned)
    }

    /// The discovered `MediaType`, if any has been filled in.
    #[napi(getter, js_name = "mediaType")]
    pub fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|media| MediaType {
            inner: media.clone(),
        })
    }
}
