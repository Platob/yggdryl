//! Internal transport abstraction.
//!
//! `Transport` is the single seam between the public `HttpSession` API and
//! the three underlying protocol implementations. Adding a new protocol means
//! adding a new `impl Transport` — the session and stream code stay unchanged.

use std::io::Read;
use std::time::Duration;

#[cfg(any(feature = "http2", feature = "http3"))]
use std::sync::OnceLock;

use crate::body::ResponseBody;
use crate::error::HttpError;
use crate::version::HttpVersion;

pub(crate) mod h1;

#[cfg(feature = "http2")]
pub(crate) mod h2;

/// Process-global async runtime shared by the HTTP/2 and HTTP/3 transports.
///
/// Initialized lazily on first use; the thread pool is named `yggdryl-http-async`
/// so it appears distinctly in profilers and thread dumps.
#[cfg(any(feature = "http2", feature = "http3"))]
pub(super) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("yggdryl-http-async")
            .build()
            .expect("failed to create yggdryl-http async runtime")
    })
}

#[cfg(feature = "http3")]
pub(crate) mod h3;

/// Configuration for a single send operation (merged from session + request).
#[derive(Clone, Debug)]
pub(crate) struct SendConfig {
    pub timeout: Option<Duration>,
    pub redirect_limit: usize,
    /// Target HTTP version for this request; used by H2/H3 transports to
    /// assert the negotiated protocol. H1 ignores it.
    #[allow(dead_code)]
    pub version: HttpVersion,
}

/// The raw, pre-processed response returned by a transport.
pub(crate) struct RawResponse {
    pub status: u16,
    /// Lower-cased header name + value pairs.
    pub headers: Vec<(String, String)>,
    pub version: HttpVersion,
    pub content_length: Option<u64>,
    pub body: Box<dyn Read + Send + 'static>,
}

impl RawResponse {
    /// Drains the body into a `ResponseBody`, optionally wrapping with a
    /// `Content-Encoding` decoder.
    pub fn into_body(self) -> ResponseBody {
        let body = ResponseBody::from_box(self.body);
        #[cfg(feature = "compression")]
        if let Some(enc) = self
            .headers
            .iter()
            .find(|(k, _)| k == "content-encoding")
            .map(|(_, v)| v.clone())
        {
            if enc != "identity" {
                return body.with_encoding(&enc);
            }
        }
        body
    }
}

/// Protocol transport — sends a single request and returns the raw response.
///
/// Implementations: [`H1Transport`](h1::H1Transport) (HTTP/1.1, always),
/// optionally [`H2Transport`](h2::H2Transport) (`http2` feature), and
/// [`H3Transport`](h3::H3Transport) (`http3` feature).
pub(crate) trait Transport: Send + Sync {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError>;

    /// Sends with a streaming request body.
    ///
    /// The default implementation buffers `body_reader` up to 64 MiB. Overrides
    /// can stream it without buffering.
    fn send_streaming(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        mut body_reader: Box<dyn Read + Send + 'static>,
        body_len: Option<u64>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError> {
        // Fallback: buffer the reader and delegate to `send`.
        let mut buf = Vec::with_capacity(body_len.unwrap_or(0).min(64 * 1024 * 1024) as usize);
        body_reader
            .read_to_end(&mut buf)
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        self.send(method, url, headers, Some(&buf), config)
    }
}

/// Picks the best `Transport` for the requested version; errors when the
/// required cargo feature is off.
pub(crate) fn for_version(version: HttpVersion) -> Result<Box<dyn Transport>, HttpError> {
    crate::log_event!(debug, "selecting transport for version={version}");
    match version {
        HttpVersion::Auto | HttpVersion::Http1_1 => Ok(Box::new(h1::H1Transport::new())),
        HttpVersion::Http2 => {
            #[cfg(feature = "http2")]
            return Ok(Box::new(h2::H2Transport::new()));
            #[cfg(not(feature = "http2"))]
            return Err(HttpError::VersionUnavailable {
                version: HttpVersion::Http2,
                feature: "http2",
            });
        }
        HttpVersion::Http3 => {
            #[cfg(feature = "http3")]
            return Ok(Box::new(h3::H3Transport::new()));
            #[cfg(not(feature = "http3"))]
            return Err(HttpError::VersionUnavailable {
                version: HttpVersion::Http3,
                feature: "http3",
            });
        }
    }
}
