//! Adapters bridging an [`Io`] body to the transport's [`std::io::Read`], and a
//! deferred-error body for the decoder path.

use yggdryl_core::Io;
#[cfg(feature = "compression")]
use yggdryl_core::{IoError, IoStats, Url, Whence};

/// Bridges an [`Io`] request body to [`std::io::Read`] for the transport, so the
/// bytes stream straight off the handle. The request framing (whether a known
/// length sets `Content-Length`) is decided by the caller, not the bridge.
pub(crate) struct IoBridge(pub(crate) Box<dyn Io>);

impl std::io::Read for IoBridge {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .read(buf)
            .map_err(|err| std::io::Error::other(err.to_string()))
    }
}

/// A body that surfaces a deferred decoder-construction error on first read,
/// keeping [`HttpResponse::reader`](crate::HttpResponse::reader) infallible.
#[cfg(feature = "compression")]
#[derive(Debug)]
pub(crate) struct ErrBody(pub(crate) Option<IoError>);

#[cfg(feature = "compression")]
impl Io for ErrBody {
    fn url(&self) -> Url {
        Url::new("mem", "errbody")
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(0))
    }

    fn seek(&mut self, _offset: i64, _whence: Whence) -> Result<u64, IoError> {
        Err(IoError::Unsupported(
            "seek on a deferred-error body (it only yields its error)".to_string(),
        ))
    }

    fn stream_position(&self) -> u64 {
        0
    }

    /// Yields the deferred error on the first read.
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize, IoError> {
        Err(self.0.take().unwrap_or(IoError::UnexpectedEof))
    }
}
