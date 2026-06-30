//! [`HttpStream`] — a seekable byte stream over an HTTP resource.
//!
//! `HttpStream` wraps a URL and a transport handle; each positional read issues
//! a `Range: bytes=offset-end` request so callers can seek freely without
//! fetching the whole body. The content length is resolved lazily on first
//! `size()` call via `HEAD` and then cached.

use std::sync::{Arc, Mutex};

use yggdryl_core::{Buffer, Io, IoError, Whence};

use crate::transport::{SendConfig, Transport};

/// A seekable, position-based byte stream backed by HTTP Range requests.
///
/// ```no_run
/// use yggdryl_http::HttpSession;
/// use yggdryl_core::{Io, IoError};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let session = HttpSession::new();
/// let mut stream = session.stream("https://example.com/file.bin")?;
/// let chunk = stream.pread(0, 16)?; // random-access without buffering the whole file
/// # Ok(())
/// # }
/// ```
pub struct HttpStream {
    url: String,
    transport: Arc<dyn Transport>,
    config: SendConfig,
    pos: u64,
    /// Cached `Content-Length` from the initial `HEAD` request.
    cached_size: Arc<Mutex<Option<u64>>>,
}

impl HttpStream {
    /// Creates a new `HttpStream` backed by `transport` for `url`.
    pub(crate) fn new(url: String, transport: Arc<dyn Transport>, config: SendConfig) -> Self {
        HttpStream {
            url,
            transport,
            config,
            pos: 0,
            cached_size: Arc::new(Mutex::new(None)),
        }
    }

    fn fetch_size(&self) -> Result<u64, IoError> {
        if let Ok(g) = self.cached_size.lock() {
            if let Some(n) = *g {
                return Ok(n);
            }
        }
        let raw = self
            .transport
            .send("HEAD", &self.url, &[], None, &self.config)
            .map_err(|e| IoError::Remote(e.to_string()))?;
        let len = raw
            .content_length
            .ok_or_else(|| IoError::Remote("HEAD response missing Content-Length".to_string()))?;
        if let Ok(mut g) = self.cached_size.lock() {
            *g = Some(len);
        }
        Ok(len)
    }
}

impl Io for HttpStream {
    fn size(&self) -> u64 {
        self.fetch_size().unwrap_or(0)
    }

    fn tell(&self) -> u64 {
        self.pos
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base: i64 = match whence {
            Whence::Start => 0,
            Whence::Current => self.pos as i64,
            Whence::End => self.fetch_size()? as i64,
        };
        let new_pos = base
            .checked_add(offset)
            .ok_or_else(|| IoError::InvalidSeek("seek offset overflowed i64".to_string()))?;
        if new_pos < 0 {
            return Err(IoError::InvalidSeek(format!(
                "seek resolved to negative position {new_pos}"
            )));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }

    fn pread_into(&self, offset: u64, dst: &mut [u8]) -> Result<usize, IoError> {
        if dst.is_empty() {
            return Ok(0);
        }
        let end = offset + dst.len() as u64 - 1;
        let range = format!("bytes={offset}-{end}");
        let raw = self
            .transport
            .send(
                "GET",
                &self.url,
                &[("range".to_string(), range)],
                None,
                &self.config,
            )
            .map_err(|e| IoError::Remote(e.to_string()))?;

        if raw.status != 206 && raw.status != 200 {
            return Err(IoError::Remote(format!(
                "Range GET returned status {} (expected 206)",
                raw.status
            )));
        }

        let body = raw.into_body();
        let bytes = body
            .drain_bytes()
            .map_err(|e| IoError::Remote(e.to_string()))?;
        let n = bytes.len().min(dst.len());
        dst[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }

    fn pread(&self, offset: u64, len: usize) -> Result<Buffer, IoError> {
        if len == 0 {
            return Ok(Buffer::from_slice(&[]));
        }
        let mut buf = vec![0u8; len];
        let n = self.pread_into(offset, &mut buf)?;
        buf.truncate(n);
        Ok(Buffer::from_vec(buf))
    }

    fn pwrite(&mut self, _offset: u64, _src: &[u8]) -> Result<usize, IoError> {
        Err(IoError::Unsupported("pwrite"))
    }
}

impl std::fmt::Debug for HttpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpStream")
            .field("url", &self.url)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}
