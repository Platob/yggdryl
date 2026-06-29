//! The streaming [`AsyncBody`] — an [`Io`] handle over a live h2/h3 response body
//! fed from a tokio feeder task via a bounded `tokio::sync::mpsc` channel.
//!
//! HTTP/2 and HTTP/3 bodies arrive as a sequence of DATA frames over the same
//! transport connection; there is no mechanism for range requests within a single
//! stream. [`AsyncBody`] bridges the async frame receiver to the blocking [`Io`]
//! interface: a tokio task feeds chunks into the channel using `.await` (so the
//! tokio worker threads never block), and `read` calls
//! [`Receiver::blocking_recv`](tokio::sync::mpsc::Receiver::blocking_recv) on the
//! caller's thread.
//!
//! A sliding 4 MiB window is kept for short seek-backs within already-delivered
//! data. Seeks backward past the window return [`IoError::Unsupported`]; forward
//! seeks drain intervening chunks into the cache. A positional
//! [`pread`](Io::pread) against cached data is zero-copy (via the default
//! [`as_slice`](Io::as_slice)-free path: seek + read + restore).
//!
//! The [`Receiver`] is not `Sync`, so it is wrapped in a
//! [`Mutex`](std::sync::Mutex) to satisfy `Io: Sync`. Since all `Io` methods take
//! `&mut self`, the mutex is never contended — it exists purely to satisfy the
//! trait bound.

use std::fmt;
use std::sync::Mutex;

use tokio::sync::mpsc::Receiver;
use yggdryl_core::{Io, IoError, IoStats, Kind, Mode, Url, Whence};

use crate::error::HttpError;
use crate::retry::CACHE_LIMIT;
use crate::time::Instant;

/// A streaming [`Io`] over an HTTP/2 or HTTP/3 response body. Sequential
/// [`read`](Io::read) blocks until the next chunk arrives from the feeder task.
/// Short seek-backs are served from the 4 MiB sliding cache; forward seeks drain
/// the stream to the target. Backward seeks past the cache window return an error.
pub(crate) struct AsyncBody {
    /// Receives data chunks from the feeder task. `None` on channel close = clean EOF.
    /// Wrapped in `Mutex` for `Sync` compliance (never actually contended).
    rx: Mutex<Receiver<Result<Vec<u8>, HttpError>>>,
    /// The current buffered chunk and how many bytes of it have been served.
    current: Vec<u8>,
    current_pos: usize,
    /// Sliding cache of recently-delivered bytes for short seek-backs.
    cache: Vec<u8>,
    /// Byte offset in the logical stream of `cache[0]`.
    cache_start: u64,
    /// Logical cursor: bytes delivered to `read` callers so far.
    position: u64,
    /// Total body size, if the server sent `Content-Length`.
    size: Option<u64>,
    url: Url,
    content_type: Option<String>,
    closed: bool,
    /// Shared with the enclosing `HttpResponse`; stamped on EOF or `close`.
    received_at: Instant,
}

impl fmt::Debug for AsyncBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncBody")
            .field("position", &self.position)
            .field("size", &self.size)
            .field("url", &self.url.to_string())
            .finish_non_exhaustive()
    }
}

impl AsyncBody {
    pub(crate) fn new(
        rx: Receiver<Result<Vec<u8>, HttpError>>,
        size: Option<u64>,
        url: Url,
        content_type: Option<String>,
        received_at: Instant,
    ) -> AsyncBody {
        AsyncBody {
            rx: Mutex::new(rx),
            current: Vec::new(),
            current_pos: 0,
            cache: Vec::new(),
            cache_start: 0,
            position: 0,
            size,
            url,
            content_type,
            closed: false,
            received_at,
        }
    }

    /// Pulls the next chunk from the feeder task (blocks until one arrives).
    /// Returns `true` if a chunk was received, `false` on clean EOF (channel closed),
    /// or an error if the feeder sent an error.
    fn next_chunk(&mut self) -> Result<bool, IoError> {
        // Pull the result out of the MutexGuard before calling any `&mut self`
        // method, so the guard is dropped and the borrow ends first.
        let received = self.rx.lock().unwrap().blocking_recv();
        match received {
            Some(Ok(chunk)) if !chunk.is_empty() => {
                self.current = chunk;
                self.current_pos = 0;
                Ok(true)
            }
            Some(Ok(_)) => {
                // Empty chunk — skip and try again.
                self.next_chunk()
            }
            Some(Err(err)) => Err(IoError::Io(err.to_string())),
            None => {
                // Channel closed: clean EOF from feeder.
                self.on_eof();
                Ok(false)
            }
        }
    }

    /// Records end-of-input: stamps `received_at`, finalises `size` if unknown.
    fn on_eof(&mut self) {
        self.closed = true;
        if self.size.is_none() {
            self.size = Some(self.position);
        }
        self.received_at.stamp_once();
    }

    /// Drains up to `buf.len()` bytes from `self.current` into `buf`, advancing
    /// the cursor and extending the sliding cache. Returns the count copied.
    fn drain_current(&mut self, buf: &mut [u8]) -> usize {
        let remaining = self.current.len() - self.current_pos;
        let take = buf.len().min(remaining);
        let src = &self.current[self.current_pos..self.current_pos + take];
        buf[..take].copy_from_slice(src);
        // Extend the sliding cache, evicting the front if over the limit.
        if self.cache.len() + take > CACHE_LIMIT {
            let evict = self.cache.len() + take - CACHE_LIMIT;
            self.cache.drain(..evict);
            self.cache_start += evict as u64;
        }
        self.cache.extend_from_slice(src);
        self.current_pos += take;
        self.position += take as u64;
        take
    }

    /// Advances `position` to `target` by draining intervening data into the
    /// cache (without returning it to the caller). Used by forward seeks.
    fn skip_to(&mut self, target: u64) -> Result<(), IoError> {
        while self.position < target && !self.closed {
            if self.current_pos < self.current.len() {
                let step = (target - self.position) as usize;
                let available = self.current.len() - self.current_pos;
                let take = step.min(available);
                let src = &self.current[self.current_pos..self.current_pos + take];
                if self.cache.len() + take > CACHE_LIMIT {
                    let evict = self.cache.len() + take - CACHE_LIMIT;
                    self.cache.drain(..evict);
                    self.cache_start += evict as u64;
                }
                self.cache.extend_from_slice(src);
                self.current_pos += take;
                self.position += take as u64;
            } else if !self.next_chunk()? {
                break; // EOF before reaching target
            }
        }
        Ok(())
    }
}

impl Io for AsyncBody {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn mode(&self) -> Mode {
        Mode::Read
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        let size = self.size.unwrap_or(self.position);
        let mut stats = IoStats::new(size).with_kind(Kind::File);
        if let Some(ct) = self.content_type.as_deref() {
            stats = stats.with_content_type(ct);
        }
        Ok(stats)
    }

    fn stream_position(&self) -> u64 {
        self.position
    }

    fn stream_len(&self) -> Option<u64> {
        self.size
    }

    /// Reads bytes from the live h2/h3 body, blocking until a chunk arrives.
    /// A seek-back within the 4 MiB sliding cache is served zero-copy.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() || self.closed {
            return Ok(0);
        }
        // A short backward seek: serve from the sliding cache.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            let available = &self.cache[offset..];
            let count = buf.len().min(available.len());
            buf[..count].copy_from_slice(&available[..count]);
            self.position += count as u64;
            return Ok(count);
        }
        // Serve from the buffered current chunk first.
        if self.current_pos < self.current.len() {
            return Ok(self.drain_current(buf));
        }
        // Pull the next chunk from the feeder (blocks until available).
        if !self.next_chunk()? {
            return Ok(0);
        }
        Ok(self.drain_current(buf))
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base: i64 = match whence {
            Whence::Start => 0,
            Whence::Current => self.position as i64,
            Whence::End => self.size.ok_or_else(|| {
                IoError::Unsupported(
                    "seek from end needs a known size (no Content-Length on this h2/h3 body)"
                        .into(),
                )
            })? as i64,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid("seek offset overflow".into()))?;
        if target < 0 {
            return Err(IoError::Invalid("seek before start".into()));
        }
        let target = target as u64;
        if target == self.position {
            return Ok(self.position);
        }
        if target > self.position {
            self.skip_to(target)?;
        } else {
            // Backward seek: the target must be within the sliding cache.
            if target < self.cache_start {
                return Err(IoError::Unsupported(
                    "seek back past the 4 MiB cache window is not supported on a streaming \
                     h2/h3 body (range requests are not available on the same stream)"
                        .into(),
                ));
            }
            // Data is in cache or in the un-drained tail of current; just move cursor.
            self.position = target;
        }
        Ok(self.position)
    }

    fn close(&mut self) -> Result<(), IoError> {
        self.closed = true;
        self.current = Vec::new();
        self.current_pos = 0;
        // Drain remaining queued chunks so the feeder can finish releasing resources.
        // Non-blocking: ignore data and errors.
        {
            let mut rx = self.rx.lock().unwrap();
            while rx.try_recv().is_ok() {}
        }
        self.received_at.stamp_once();
        Ok(())
    }
}

impl Drop for AsyncBody {
    fn drop(&mut self) {
        // Safety net: stamp received_at even when the body is dropped without being
        // fully read (e.g. an early-abort or a move into `into_io`).
        self.received_at.stamp_once();
    }
}
