//! The seekable, connection-holding [`HttpStream`] — the canonical remote [`Io`].

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use yggdryl_io::{Io, IoError, IoStats, Url, Whence};
#[cfg(feature = "media")]
use yggdryl_url::FromInput;

use crate::headers::HttpHeaders;
use crate::retry::{RetryConfig, CACHE_LIMIT};
use crate::time::Instant;

/// A seekable [`Io`] over an HTTP resource that **streams off a held connection**
/// rather than collecting the body: sequential [`read`](Io::read) pulls bytes
/// straight off the socket on demand, keeping only a sliding 4 MiB cache for short
/// seek-backs. Random access ([`pread`](Io::pread), a footer read) or a seek-back
/// past the cache re-opens a `Range` request on a pooled connection. The connection
/// is released to the pool on EOF (or closed, with no keep-alive), and
/// [`close`](Io::close) drops it eagerly. A connection lost mid-stream is
/// reconnected and **resumed from the cursor**.
pub struct HttpStream {
    agent: ureq::Agent,
    url: Url,
    headers: HttpHeaders,
    retry: RetryConfig,
    /// When `false`, requests carry `Connection: close` so the socket is not
    /// pooled (the pool-saturation safeguard sets this on extra streams).
    keep_alive: bool,
    size: Option<u64>,
    content_type: Option<String>,
    /// The live response-body reader, positioned at `reader_pos`. `None` once the
    /// stream is closed, exhausted, or awaiting a (re)open for `position`.
    reader: Option<Box<dyn std::io::Read + Send + Sync>>,
    reader_pos: u64,
    /// A sliding cache of recently-streamed bytes for short seek-backs, never
    /// larger than `CACHE_LIMIT`.
    cache: Vec<u8>,
    cache_start: u64,
    position: u64,
    closed: bool,
    /// Shared count of live streams (held connections) for the pool safeguard.
    held: Arc<AtomicUsize>,
    /// The "connection done" instant, stamped on EOF or [`close`](Io::close) and
    /// shared with the [`HttpResponse`](crate::HttpResponse) that returned this
    /// stream, so its [`received_at`](crate::HttpResponse::received_at) reflects
    /// when the caller finished draining the body.
    received_at: Instant,
}

impl HttpStream {
    /// Builds a stream from a freshly-received response, holding its live body as
    /// the connection at offset 0.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_response(
        response: ureq::http::Response<ureq::Body>,
        agent: ureq::Agent,
        url: Url,
        headers: HttpHeaders,
        retry: RetryConfig,
        keep_alive: bool,
        held: Arc<AtomicUsize>,
        received_at: Instant,
        size: Option<u64>,
        content_type: Option<String>,
    ) -> HttpStream {
        let reader: Box<dyn std::io::Read + Send + Sync> =
            Box::new(response.into_body().into_reader());
        held.fetch_add(1, Ordering::SeqCst);
        HttpStream {
            agent,
            url,
            headers,
            retry,
            keep_alive,
            size,
            content_type,
            reader: Some(reader),
            reader_pos: 0,
            cache: Vec::new(),
            cache_start: 0,
            position: 0,
            closed: false,
            held,
            received_at,
        }
    }

    /// The total size in bytes, if the server reported it.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// The `Content-Type`, if the server reported it.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Closes the held connection eagerly (idempotent); further reads return EOF.
    pub fn close(&mut self) {
        self.reader = None;
        self.closed = true;
        self.received_at.stamp_once();
    }

    /// The number of bytes still readable from `position`, if the size is known.
    fn remaining(&self) -> Option<u64> {
        self.size.map(|size| size.saturating_sub(self.position))
    }

    /// Records end-of-input on an unknown-size stream and releases the socket.
    fn on_eof(&mut self) {
        if self.size.is_none() {
            self.size = Some(self.position);
        }
        self.reader = None; // exhausted: return the connection to the pool
        self.received_at.stamp_once();
    }

    /// (Re)opens a `Range` request from `start`, replacing the live reader — used
    /// to seek back past the cache, jump forward, or resume after a drop.
    fn open_at(&mut self, start: u64) -> Result<(), IoError> {
        let (size, reader) = self.request_reader(start)?;
        if self.size.is_none() {
            self.size = size;
        }
        self.reader = reader;
        self.reader_pos = start;
        self.cache.clear();
        self.cache_start = start;
        Ok(())
    }

    /// Issues a ranged `GET <url>` (with `Connection: close` when `close`),
    /// retrying transient statuses and reconnecting on a transport error. Returns
    /// the response, or `None` at a clean EOF (`416`). `start` is the range's first
    /// byte, used only to reject a server that ignores the range (`200` to a
    /// non-zero offset). The single place a `Range` request is built and classified.
    fn ranged_get(
        &self,
        range: &str,
        start: u64,
        close: bool,
    ) -> Result<Option<ureq::http::Response<ureq::Body>>, IoError> {
        let url = self.url.to_string();
        let mut attempt = 0u32;
        loop {
            let mut builder = ureq::http::Request::builder()
                .method("GET")
                .uri(url.as_str());
            for (name, value) in self.headers.iter() {
                builder = builder.header(name, value);
            }
            builder = builder.header("range", range);
            if close {
                builder = builder.header("connection", "close");
            }
            let outcome = builder
                .body(ureq::SendBody::none())
                .map_err(|err| IoError::Io(err.to_string()))
                .and_then(|request| {
                    self.agent
                        .run(request)
                        .map_err(|err| IoError::Io(err.to_string()))
                });
            match outcome {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if attempt < self.retry.max_retries && self.retry.retryable_status(status) {
                        let delay = self
                            .retry
                            .backoff(attempt, HttpHeaders::from(response.headers()).retry_after());
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    if status == 416 {
                        return Ok(None); // range past the end: clean EOF
                    }
                    if status >= 400 {
                        return Err(IoError::Io(format!(
                            "http status {status} fetching a range (check the URL and that the resource still exists)"
                        )));
                    }
                    if status == 200 && start > 0 {
                        return Err(IoError::Unsupported(
                            "server ignored the Range request (it does not support range reads)"
                                .to_string(),
                        ));
                    }
                    // A 206 must resume from exactly the byte we asked for; a server
                    // that answers a different range would corrupt the stream.
                    if status == 206 {
                        if let Some(got) =
                            HttpHeaders::from(response.headers()).content_range_start()
                        {
                            if got != start {
                                return Err(IoError::Io(format!(
                                    "server resumed at byte {got}, not the requested {start} (range reads are unreliable here)"
                                )));
                            }
                        }
                    }
                    return Ok(Some(response));
                }
                Err(error) => {
                    if attempt < self.retry.max_retries {
                        let delay = self.retry.backoff(attempt, None);
                        log_event!(warn, "HttpStream reconnect after error: {error}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// Opens a streaming reader from `start` via an open-ended `Range` (and
    /// `Connection: close` when not keep-alive). Returns the total size (if newly
    /// learnt) and the live reader, or `None` reader at a clean EOF (`416`).
    #[allow(clippy::type_complexity)]
    fn request_reader(
        &self,
        start: u64,
    ) -> Result<(Option<u64>, Option<Box<dyn std::io::Read + Send + Sync>>), IoError> {
        match self.ranged_get(&format!("bytes={start}-"), start, !self.keep_alive)? {
            None => Ok((None, None)),
            Some(response) => {
                let size = HttpHeaders::from(response.headers()).content_size();
                let reader: Box<dyn std::io::Read + Send + Sync> =
                    Box::new(response.into_body().into_reader());
                Ok((size, Some(reader)))
            }
        }
    }

    /// Reads off the live connection into `buf`, reconnecting (resuming the range
    /// from `reader_pos`) if the connection drops, up to the retry limit.
    fn read_live(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let mut attempt = 0u32;
        loop {
            let result = match self.reader.as_mut() {
                Some(reader) => std::io::Read::read(reader, buf),
                None => return Ok(0),
            };
            match result {
                Ok(count) => return Ok(count),
                Err(_error) if attempt < self.retry.max_retries => {
                    attempt += 1;
                    std::thread::sleep(self.retry.backoff(attempt - 1, None));
                    log_event!(
                        warn,
                        "HttpStream reconnect mid-stream at {} (attempt {attempt})",
                        self.reader_pos
                    );
                    self.open_at(self.reader_pos)?; // resume from the cursor
                    continue;
                }
                Err(error) => return Err(IoError::from(error)),
            }
        }
    }

    /// Fetches exactly `[start, start+len)` into a fresh `Vec` via a one-off
    /// `Range` request — used by [`pread`](Io::pread), leaving the live reader
    /// untouched.
    fn fetch_range(&self, start: u64, len: u64) -> Result<Vec<u8>, IoError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let end = start
            .checked_add(len)
            .and_then(|end| end.checked_sub(1))
            .ok_or_else(|| IoError::Invalid("range offset overflow".to_string()))?;
        let range = format!("bytes={start}-{end}");
        let mut attempt = 0u32;
        loop {
            let Some(mut response) = self.ranged_get(&range, start, false)? else {
                return Ok(Vec::new()); // 416: range past the end
            };
            let truncate = response.status().as_u16() == 200; // a non-range body needs trimming
            let mut out = Vec::with_capacity(len.min(CACHE_LIMIT as u64) as usize);
            let mut reader = response.body_mut().as_reader();
            match std::io::Read::read_to_end(&mut reader, &mut out) {
                Ok(_) => {
                    if truncate {
                        out.truncate(len as usize);
                    }
                    return Ok(out);
                }
                // The connection dropped mid-body: re-issue the whole range request.
                Err(_error) if attempt < self.retry.max_retries => {
                    attempt += 1;
                    std::thread::sleep(self.retry.backoff(attempt - 1, None));
                }
                Err(error) => return Err(IoError::from(error)),
            }
        }
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        self.held.fetch_sub(1, Ordering::SeqCst);
    }
}

impl fmt::Debug for HttpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpStream")
            .field("url", &self.url.to_string())
            .field("size", &self.size)
            .field("position", &self.position)
            .field("reader_pos", &self.reader_pos)
            .field("cache", &(self.cache_start, self.cache.len()))
            .field("closed", &self.closed)
            .finish()
    }
}

impl Io for HttpStream {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        let mut stats = IoStats::new(self.size.unwrap_or(0)).with_kind(yggdryl_io::Kind::File);
        if let Some(content_type) = &self.content_type {
            stats = stats.with_content_type(content_type.clone());
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

    /// Reads off the live connection (or the sliding cache) into `buf`, advancing
    /// the cursor — the streamed read primitive.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() || self.closed || self.remaining() == Some(0) {
            // A fully-drained known-size body is done: mark the connection done.
            if self.remaining() == Some(0) {
                self.received_at.stamp_once();
            }
            return Ok(0);
        }
        // A short seek-back is served from the sliding cache.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            let available = &self.cache[offset..];
            let count = buf.len().min(available.len());
            buf[..count].copy_from_slice(&available[..count]);
            self.position += count as u64;
            return Ok(count);
        }
        // Otherwise stream off the live connection (re-opening if the cursor moved
        // off the reader).
        if self.reader.is_none() || self.reader_pos != self.position {
            self.open_at(self.position)?;
        }
        let count = self.read_live(buf)?;
        if count == 0 {
            self.on_eof();
            return Ok(0);
        }
        self.cache.extend_from_slice(&buf[..count]);
        if self.cache.len() > CACHE_LIMIT {
            let evict = self.cache.len() - CACHE_LIMIT;
            self.cache.drain(..evict);
            self.cache_start += evict as u64;
        }
        self.reader_pos += count as u64;
        self.position += count as u64;
        Ok(count)
    }

    /// Drains the rest of the stream straight into `out`, reading whole chunks off
    /// the connection into `out`'s own buffer (one copy), reconnecting on a drop.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        if self.closed {
            return Ok(0);
        }
        let start_len = out.len();
        // Serve any cached tail first.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            out.extend_from_slice(&self.cache[offset..]);
            self.position += (self.cache.len() - offset) as u64;
        }
        if self.remaining() != Some(0)
            && (self.reader.is_none() || self.reader_pos != self.position)
        {
            self.open_at(self.position)?;
        }
        while self.remaining() != Some(0) {
            let base = out.len();
            out.resize(base + 64 * 1024, 0);
            let count = self.read_live(&mut out[base..])?;
            out.truncate(base + count);
            if count == 0 {
                self.on_eof();
                break;
            }
            self.reader_pos += count as u64;
            self.position += count as u64;
        }
        // The loop exits once the body is fully drained (known size) or hit EOF:
        // either way the connection is done.
        self.received_at.stamp_once();
        Ok(out.len() - start_len)
    }

    /// Releases the held connection (idempotent); further reads return EOF.
    fn close(&mut self) -> Result<(), IoError> {
        HttpStream::close(self);
        Ok(())
    }

    /// A positional read via a one-off `Range` request, leaving the live reader
    /// and — for [`Whence::Start`]/[`Whence::End`] — the cursor untouched, so a
    /// footer can be read without disturbing a sequential scan. With
    /// [`Whence::Current`] the cursor is the base and is advanced by what was read.
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
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        let content_type = self.content_type.as_ref()?;
        yggdryl_media::MimeType::from_str(content_type)
            .ok()
            .map(|mime| yggdryl_media::MediaType::new(vec![mime]))
    }
}
