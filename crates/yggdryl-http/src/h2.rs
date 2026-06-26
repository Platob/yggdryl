//! HTTP/2 (and HTTP/1.1 ALPN fallback) transport via `hyper` + `hyper-util`.
//!
//! [`H2Client`] is a pooling client backed by `hyper_util::client::legacy::Client`
//! with a `hyper-rustls` HTTPS connector — ALPN advertises both `h2` and `http/1.1`
//! so the server picks the highest version it supports. [`H2Stream`] wraps the
//! response body (`hyper::body::Incoming`) as a seekable [`Io`] handle, mirroring
//! [`HttpStream`](crate::HttpStream) for the HTTP/1.1 transport.

use std::fmt;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use yggdryl_core::{Io, IoError, IoStats, Url, Whence};

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::protocol::HttpVersion;
use crate::retry::CACHE_LIMIT;
use crate::time::Instant;

type HyperClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// A pooling HTTP/2+HTTP/1.1 client backed by hyper. Cheap to clone (Arc
/// inside); `HttpSession` holds one behind an `Arc` and shares it with every
/// open [`H2Stream`] for Range re-requests.
pub(crate) struct H2Client {
    inner: HyperClient,
}

impl H2Client {
    /// Builds a client with a TLS connector that advertises both `h2` and
    /// `http/1.1` via ALPN, with a pool of up to `max_pool` idle connections
    /// per host.
    pub(crate) fn new(max_pool: usize) -> H2Client {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();
        let mut builder = Client::builder(TokioExecutor::new());
        builder.pool_max_idle_per_host(max_pool);
        H2Client {
            inner: builder.build(https),
        }
    }

    /// Sends `req` and returns the raw response plus the negotiated protocol.
    pub(crate) fn execute(
        &self,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<(hyper::Response<Incoming>, HttpVersion), HttpError> {
        let resp = crate::runtime::block_on(self.inner.request(req))
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        let version = match resp.version() {
            hyper::Version::HTTP_2 => HttpVersion::H2,
            _ => HttpVersion::H1_1,
        };
        Ok((resp, version))
    }
}

// ---

/// A seekable [`Io`] over an HTTP/2 response body.
///
/// Sequential [`read`](Io::read) pulls frames straight from the `Incoming`
/// body, caching up to 4 MiB for short seek-backs. A [`pread`](Io::pread) or
/// a seek past the cache re-opens a `Range` GET via the same [`H2Client`]
/// connection pool, leaving the sequential cursor untouched for
/// `Whence::Start`/`Whence::End`.
pub(crate) struct H2Stream {
    /// The live response body (`None` = EOF, closed, or awaiting a re-open).
    body: Option<Incoming>,
    /// Unconsumed bytes from the last data frame.
    leftover: Bytes,
    /// Sliding cache of recently-read bytes, never larger than `CACHE_LIMIT`.
    cache: Vec<u8>,
    cache_start: u64,
    position: u64,
    reader_pos: u64,
    size: Option<u64>,
    content_type: Option<String>,
    closed: bool,
    /// Shared client for Range re-requests.
    client: Arc<H2Client>,
    url: Url,
    /// Request headers forwarded into Range re-requests (auth, etc.).
    request_headers: HttpHeaders,
    version: HttpVersion,
    received_at: Instant,
}

// SAFETY: all mutable methods take `&mut self`; `Incoming` is never accessed
// via a shared reference, so concurrent access is structurally impossible even
// though `Incoming: !Sync`.
unsafe impl Sync for H2Stream {}

impl H2Stream {
    /// Wraps a freshly-received `Incoming` body as a seekable `Io`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_response(
        body: Incoming,
        client: Arc<H2Client>,
        url: Url,
        request_headers: HttpHeaders,
        received_at: Instant,
        size: Option<u64>,
        content_type: Option<String>,
        version: HttpVersion,
    ) -> H2Stream {
        H2Stream {
            body: Some(body),
            leftover: Bytes::new(),
            cache: Vec::new(),
            cache_start: 0,
            position: 0,
            reader_pos: 0,
            size,
            content_type,
            closed: false,
            client,
            url,
            request_headers,
            version,
            received_at,
        }
    }

    fn remaining(&self) -> Option<u64> {
        self.size.map(|s| s.saturating_sub(self.position))
    }

    fn on_eof(&mut self) {
        if self.size.is_none() {
            self.size = Some(self.position);
        }
        self.body = None;
        self.received_at.stamp_once();
    }

    fn update_cache(&mut self, data: &[u8]) {
        self.cache.extend_from_slice(data);
        if self.cache.len() > CACHE_LIMIT {
            let evict = self.cache.len() - CACHE_LIMIT;
            self.cache.drain(..evict);
            self.cache_start += evict as u64;
        }
    }

    /// (Re)opens the body stream from `start` via a Range GET, replacing the
    /// current `Incoming` body.
    fn open_at(&mut self, start: u64) -> Result<(), IoError> {
        let range = format!("bytes={start}-");
        let mut req_builder = hyper::Request::builder()
            .method("GET")
            .uri(self.url.to_string())
            .header("range", &range);
        for (name, value) in self.request_headers.iter() {
            req_builder = req_builder.header(name, value);
        }
        let req = req_builder
            .body(Full::default())
            .map_err(|e| IoError::Io(e.to_string()))?;

        let (resp, version) = self
            .client
            .execute(req)
            .map_err(|e| IoError::Io(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 416 {
            self.body = None;
            return Ok(());
        }
        if status >= 400 {
            return Err(IoError::Io(format!("http status {status} fetching range")));
        }
        if status == 200 && start > 0 {
            return Err(IoError::Unsupported(
                "server ignored the Range request (it does not support range reads)".to_string(),
            ));
        }
        let resp_headers = HttpHeaders::from(resp.headers());
        if self.size.is_none() {
            self.size = resp_headers.content_size();
        }
        self.version = version;
        self.body = Some(resp.into_body());
        self.leftover = Bytes::new();
        self.reader_pos = start;
        self.cache.clear();
        self.cache_start = start;
        Ok(())
    }

    /// Fetches exactly `[start, start+len)` via a one-off `Range` GET without
    /// touching the live reader or the cursor.
    fn fetch_range(&self, start: u64, len: u64) -> Result<Vec<u8>, IoError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let end = start
            .checked_add(len)
            .and_then(|e| e.checked_sub(1))
            .ok_or_else(|| IoError::Invalid("range offset overflow".to_string()))?;
        let range = format!("bytes={start}-{end}");
        let mut req_builder = hyper::Request::builder()
            .method("GET")
            .uri(self.url.to_string())
            .header("range", &range);
        for (name, value) in self.request_headers.iter() {
            req_builder = req_builder.header(name, value);
        }
        let req = req_builder
            .body(Full::default())
            .map_err(|e| IoError::Io(e.to_string()))?;

        let (resp, _) = self
            .client
            .execute(req)
            .map_err(|e| IoError::Io(e.to_string()))?;

        let truncate = resp.status().as_u16() == 200;
        let collected = crate::runtime::block_on(resp.into_body().collect())
            .map_err(|e| IoError::Io(e.to_string()))?;
        let mut out = collected.to_bytes().to_vec();
        if truncate {
            out.truncate(len as usize);
        }
        Ok(out)
    }

    /// Polls the next data frame off the live body, blocking until one arrives.
    /// Returns `None` on EOF (including trailer frames).
    fn next_chunk(&mut self) -> Result<Option<Bytes>, IoError> {
        let Some(body) = self.body.as_mut() else {
            return Ok(None);
        };
        let frame = crate::runtime::block_on(body.frame())
            .transpose()
            .map_err(|e| IoError::Io(e.to_string()))?;
        let Some(frame) = frame else {
            return Ok(None);
        };
        match frame.into_data() {
            Ok(data) => Ok(Some(data)),
            Err(_trailer) => Ok(None), // trailer frame → signal EOF
        }
    }
}

impl Drop for H2Stream {
    fn drop(&mut self) {
        // Drop the live body to release the H2 stream slot back to the server.
        self.body = None;
    }
}

impl fmt::Debug for H2Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("H2Stream")
            .field("url", &self.url.to_string())
            .field("version", &self.version.as_str())
            .field("size", &self.size)
            .field("position", &self.position)
            .field("closed", &self.closed)
            .finish()
    }
}

impl Io for H2Stream {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        let mut stats = IoStats::new(self.size.unwrap_or(0)).with_kind(yggdryl_core::Kind::File);
        if let Some(ct) = &self.content_type {
            stats = stats.with_content_type(ct.clone());
        }
        #[cfg(feature = "media")]
        if let Some(media_type) = self.media_type() {
            stats = stats.with_media_type(media_type);
        }
        Ok(stats)
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self.size.ok_or_else(|| {
                IoError::Unsupported(
                    "seek from end needs a known size (the server sent no Content-Length)"
                        .to_string(),
                )
            })? as i64,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid("seek offset overflow".to_string()))?;
        if target < 0 {
            return Err(IoError::Invalid("seek before start".to_string()));
        }
        self.position = target as u64;
        Ok(self.position)
    }

    fn stream_position(&self) -> u64 {
        self.position
    }

    fn stream_len(&self) -> Option<u64> {
        self.size
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() || self.closed || self.remaining() == Some(0) {
            if self.remaining() == Some(0) {
                self.received_at.stamp_once();
            }
            return Ok(0);
        }

        // Serve a short seek-back from the sliding cache.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            let available = &self.cache[offset..];
            let count = buf.len().min(available.len());
            buf[..count].copy_from_slice(&available[..count]);
            self.position += count as u64;
            return Ok(count);
        }

        // Position moved off the reader — reopen via Range.
        if self.body.is_none() || self.reader_pos != self.position {
            self.open_at(self.position)?;
        }

        // Drain leftover bytes from the previous frame first.
        if !self.leftover.is_empty() {
            let n = buf.len().min(self.leftover.len());
            buf[..n].copy_from_slice(&self.leftover[..n]);
            self.leftover = self.leftover.slice(n..);
            self.update_cache(&buf[..n]);
            self.reader_pos += n as u64;
            self.position += n as u64;
            return Ok(n);
        }

        // Pull the next data frame.
        match self.next_chunk()? {
            None => {
                self.body = None;
                self.on_eof();
                Ok(0)
            }
            Some(data) if data.is_empty() => Ok(0),
            Some(data) => {
                let n = buf.len().min(data.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    self.leftover = data.slice(n..);
                }
                self.update_cache(&buf[..n]);
                self.reader_pos += n as u64;
                self.position += n as u64;
                Ok(n)
            }
        }
    }

    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        if self.closed {
            return Ok(0);
        }
        let start_len = out.len();

        // Serve any cached tail.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            out.extend_from_slice(&self.cache[offset..]);
            self.position += (self.cache.len() - offset) as u64;
        }

        if self.remaining() != Some(0) && (self.body.is_none() || self.reader_pos != self.position)
        {
            self.open_at(self.position)?;
        }

        // Drain leftover first.
        if !self.leftover.is_empty() {
            out.extend_from_slice(&self.leftover);
            self.position += self.leftover.len() as u64;
            self.reader_pos += self.leftover.len() as u64;
            self.leftover = Bytes::new();
        }

        // Drain remaining frames.
        while self.remaining() != Some(0) {
            match self.next_chunk()? {
                None => {
                    self.on_eof();
                    break;
                }
                Some(data) if data.is_empty() => break,
                Some(data) => {
                    out.extend_from_slice(&data);
                    self.reader_pos += data.len() as u64;
                    self.position += data.len() as u64;
                }
            }
        }
        self.received_at.stamp_once();
        Ok(out.len() - start_len)
    }

    fn close(&mut self) -> Result<(), IoError> {
        self.body = None;
        self.leftover = Bytes::new();
        self.closed = true;
        self.received_at.stamp_once();
        Ok(())
    }

    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self.size.ok_or_else(|| {
                IoError::Unsupported(
                    "pread from end needs a known size (the server sent no Content-Length)"
                        .to_string(),
                )
            })? as i64,
        };
        let start = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid("pread offset overflow".to_string()))?;
        if start < 0 {
            return Err(IoError::Invalid("pread before start".to_string()));
        }
        let start = start as u64;
        if self.size.is_some_and(|size| start >= size) {
            return Ok(0);
        }
        let want = self.size.map_or(buf.len() as u64, |size| {
            (buf.len() as u64).min(size - start)
        });
        let bytes = self.fetch_range(start, want)?;
        let count = buf.len().min(bytes.len());
        buf[..count].copy_from_slice(&bytes[..count]);
        if matches!(whence, Whence::Current) {
            self.position = start + count as u64;
        }
        Ok(count)
    }

    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_core::MediaType> {
        let ct = self.content_type.as_ref()?;
        yggdryl_core::MimeType::from_str(ct)
            .ok()
            .map(|mime| yggdryl_core::MediaType::new(vec![mime]))
    }
}
