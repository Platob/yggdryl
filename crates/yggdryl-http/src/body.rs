//! Request and response body abstractions.
//!
//! `RequestBody` carries the data for an outgoing request; `ResponseBody`
//! wraps the incoming stream and handles streaming decompression when the
//! `compression` feature is on.

use std::io::{self, Read};

use yggdryl_core::Io;

/// The data for an outgoing HTTP request body.
pub enum RequestBody {
    /// Raw bytes (sent with `Content-Length: N`).
    Bytes(Vec<u8>),
    /// A streaming `Io` handle; `Content-Length` is set from `io.size()`.
    Io(Box<dyn Io + Send + 'static>),
}

impl RequestBody {
    /// Length hint in bytes, if known.
    pub fn len(&self) -> Option<u64> {
        match self {
            RequestBody::Bytes(b) => Some(b.len() as u64),
            RequestBody::Io(io) => Some(io.size()),
        }
    }

    /// Whether the body is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
}

impl std::fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestBody::Bytes(b) => write!(f, "RequestBody::Bytes({} bytes)", b.len()),
            RequestBody::Io(io) => write!(f, "RequestBody::Io({} bytes)", io.size()),
        }
    }
}

/// A streaming response body.
///
/// Wraps a `Box<dyn Read + Send>` — which may be a decompressor, a socket
/// reader, or an in-memory cursor — and exposes it as a lazy stream. Draining
/// methods (`drain_bytes`, `drain_text`) consume `self` so the body can only
/// be read once (as with a streaming `requests.Response`).
pub(crate) struct ResponseBody {
    inner: Box<dyn Read + Send + 'static>,
}

impl ResponseBody {
    /// Wraps any `Read + Send + 'static` as a response body.
    pub(crate) fn new<R: Read + Send + 'static>(reader: R) -> Self {
        ResponseBody {
            inner: Box::new(reader),
        }
    }

    /// Wraps a boxed reader.
    pub(crate) fn from_box(reader: Box<dyn Read + Send + 'static>) -> Self {
        ResponseBody { inner: reader }
    }

    /// Wraps the reader with a `Content-Encoding` decoder when the
    /// `compression` feature is on and the encoding is recognised.
    #[allow(unused_variables)]
    pub(crate) fn with_encoding(self, encoding: &str) -> Self {
        #[cfg(feature = "compression")]
        {
            ResponseBody {
                inner: yggdryl_core::compress::decode_reader(self.inner, encoding),
            }
        }
        #[cfg(not(feature = "compression"))]
        {
            self
        }
    }

    /// Drains the body into a `Vec<u8>`.
    pub(crate) fn drain_bytes(mut self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.inner.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Drains the body and decodes as UTF-8 text.
    pub(crate) fn drain_text(self) -> io::Result<String> {
        let bytes = self.drain_bytes()?;
        String::from_utf8(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}

impl Read for ResponseBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}
