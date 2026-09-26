//! `Stream`: a response body still on the wire, read forward and re-opened
//! from the byte it stopped at when the transport dies under it.
//!
//! A long transfer over a network dies for reasons that have nothing to do
//! with the resource - a reset connection, an idle timeout, a load balancer
//! recycled - and failing the whole read for one of them means fetching
//! again everything already delivered. This asks for the rest with a `Range`
//! from the delivered cursor and, when the first answer carried a strong
//! `ETag` or a `Last-Modified`, an `If-Range` naming it, so a resource that
//! changed under the transfer is refused rather than spliced. Only a
//! transport failure resumes, only while consecutive failures stay under the
//! client's attempt limit, and only when the first answer said
//! `Accept-Ranges: bytes` or was itself a `206`; a byte arriving resets the
//! failure count, so a transfer that keeps moving survives interruption
//! after interruption while one that cannot deliver a byte stops - up to
//! [`Stream::MAX_RESUMES`] re-opens of one transfer, past which a server cutting
//! every connection short costs a bounded number of requests. A re-opened
//! `206` must state in `Content-Range` that it starts at the cursor, or the
//! bytes are not spliced.
//!
//! As an [`IOBase`] a stream reads forward: a position at or beyond the
//! delivered cursor skips to it, one behind re-opens a range when the
//! resource accepts one and is refused by name otherwise. Writes are refused.
//! The lock over the transfer is held across a read, a resume's pause
//! included, because a transfer is one sequence of bytes and two readers of
//! it cannot both go forward; [`Stream::delivered`] and [`Stream::resumes`]
//! read counters published beside it and never wait on it.
//!
//! A stream stands alone: it carries the request that opened it, and that
//! request its session, so nothing else has to be held for it to go on.
//! [`IOBase::close`] lets go of the live transfer and its connection while
//! keeping the cursor; the next read - or [`IOBase::open`] - re-opens it at
//! the cursor with one ranged `GET` naming the first answer's validator, so
//! a stream parked for a while costs no connection and resumes exactly where
//! it stopped, or is refused if the resource changed meanwhile.

use std::io::{self, Read};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::client::Answer;
use super::retry::is_resumable;
use super::{ContentRange, Headers, Request, Status};
use crate::holder::Holder;
use crate::{ByteStream, Error, IOBase, IOKind, Listing, MediaType, Result, Uri, Url};

/// The scratch a skip or a discard reads into.
const DISCARD_CHUNK: usize = 64 * 1024;

/// The most a whole read reserves on the strength of a stated length alone;
/// past it the buffer grows as the bytes arrive.
const RESERVE_CAP: usize = 64 << 20;

/// A response body read forward, re-opened from the delivered cursor when
/// the transport fails under it.
///
/// ```no_run
/// use std::io::Read;
///
/// use yggdryl::http::Request;
///
/// # fn main() -> yggdryl::Result<()> {
/// let response = Request::get("https://example.com/archive.bin")?.stream()?;
/// let mut stream = response.into_stream()?;
/// let mut head = [0_u8; 1024];
/// stream.read_exact(&mut head)?;
/// // Bytes handed over so far, whatever the transport did underneath.
/// assert_eq!(stream.delivered(), 1024);
/// assert_eq!(stream.resumes(), 0);
/// # Ok(())
/// # }
/// ```
pub struct Stream {
    headers: Headers,
    url: Option<Url>,
    media_type: MediaType,
    total: Option<u64>,
    inner: Mutex<Inner>,
    /// `Inner::delivered` and `Inner::resumes` as last published, read
    /// without the lock.
    delivered: AtomicU64,
    resumes: AtomicU32,
}

/// What a read moves.
struct Inner {
    source: Source,
    /// Bytes handed to the caller: the resume cursor, counted within the
    /// window the stream covers.
    delivered: u64,
    /// Consecutive failures since the last byte arrived.
    failures: u32,
    /// Transfers re-opened so far.
    resumes: u32,
}

/// Where the bytes come from.
enum Source {
    /// A live transfer.
    Wire(Box<Transfer>),
    /// A body already held, streamed over its bytes.
    Bytes(Arc<[u8]>),
}

/// One live transfer and what re-opening it needs.
struct Transfer {
    request: Request,
    url: Url,
    reader: Box<dyn Read + Send>,
    /// Where the window starts in the resource.
    start: u64,
    /// The last byte of the window, when bounded.
    last: Option<u64>,
    /// Bytes still to discard before the window: a `200` answered a range.
    skip: u64,
    /// Bytes the window should hold when the headers state it, so a body
    /// ending short of its stated length is a cut transfer and not an end.
    expected: Option<u64>,
    /// The `If-Range` validator, when the first answer carried one.
    validator: Option<String>,
    /// Whether the resource accepts a range, so a failure may re-open one.
    resumable: bool,
    /// Whether the live transfer was let go by a close; the next read
    /// re-opens it at the cursor.
    closed: bool,
}

impl Stream {
    /// The most times one transfer is re-opened after a transport failure,
    /// however many bytes each re-open delivered.
    pub const MAX_RESUMES: u32 = 256;

    /// A stream over `answer`, the body of `request` at `url`, covering the
    /// window from `start` to `last` (`None` for the end).
    ///
    /// A `200` answer to a request that asked for a range from `start`
    /// ignored the range: the stream skips `start` bytes and bounds itself to
    /// `last`. A `206` is trusted to start where it was asked to.
    pub(crate) fn new(
        request: Request,
        url: Url,
        answer: Answer,
        start: u64,
        last: Option<u64>,
    ) -> Self {
        let headers = answer.headers;
        let content_range = headers.content_range().ok().flatten();
        let resumable = headers.accept_ranges() || answer.status == Status::PARTIAL_CONTENT;
        let total = match content_range {
            Some(range) => range.total(),
            None => headers.content_length().ok().flatten(),
        };
        let skip = if answer.status == Status::PARTIAL_CONTENT {
            0
        } else {
            start
        };
        let expected = window_length(&headers, answer.status, start, last);
        let validator = validator_of(&headers);
        let media_type = headers.media_type().unwrap_or_default();
        Self {
            headers,
            url: Some(url.clone()),
            media_type,
            total,
            inner: Mutex::new(Inner {
                source: Source::Wire(Box::new(Transfer {
                    request,
                    url,
                    reader: answer.body,
                    start,
                    last,
                    skip,
                    expected,
                    validator,
                    resumable,
                    closed: false,
                })),
                delivered: 0,
                failures: 0,
                resumes: 0,
            }),
            delivered: AtomicU64::new(0),
            resumes: AtomicU32::new(0),
        }
    }

    /// A materialized body as a stream over its bytes, under `headers`.
    pub(crate) fn over_bytes(bytes: Arc<[u8]>, headers: Headers) -> Self {
        let media_type = headers.media_type().unwrap_or_default();
        Self {
            total: Some(bytes.len() as u64),
            headers,
            url: None,
            media_type,
            inner: Mutex::new(Inner {
                source: Source::Bytes(bytes),
                delivered: 0,
                failures: 0,
                resumes: 0,
            }),
            delivered: AtomicU64::new(0),
            resumes: AtomicU32::new(0),
        }
    }

    /// The same stream, answering `url` as its location.
    pub(crate) fn located_at(mut self, url: Url) -> Self {
        self.url = Some(url);
        self
    }

    /// Bytes handed to the caller so far: the resume cursor.
    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Acquire)
    }

    /// The length the headers stated: the `Content-Range` total, else
    /// `Content-Length`.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// How many times the transfer was re-opened.
    pub fn resumes(&self) -> u32 {
        self.resumes.load(Ordering::Acquire)
    }

    /// The headers of the answer the stream reads.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Read the rest from `position`, `pread`'s rules deciding whether a
    /// position behind the cursor is reachable, holding at most `limit`
    /// bytes.
    ///
    /// # Errors
    ///
    /// A body longer than `limit`, a transport failure that could not be
    /// resumed, a changed resource, or a position behind the cursor of a
    /// resource without ranges.
    pub(crate) fn read_from(&self, position: u64, limit: u64) -> Result<Vec<u8>> {
        let mut inner = self.inner()?;
        // The length the headers state, when they state one inside the
        // bound, is reserved at once: one allocation rather than doublings.
        let stated = self
            .total
            .map(|total| total.saturating_sub(position))
            .filter(|length| *length <= limit)
            .and_then(|length| usize::try_from(length).ok());
        let read = read_rest(
            &mut inner,
            position,
            limit,
            stated.unwrap_or(0),
            self.url.as_ref(),
        );
        self.publish(&inner);
        read
    }

    /// The largest body a whole read holds: the session's `max_body_size`
    /// for a transfer, unbounded for bytes already held.
    fn max_body_size(&self) -> Result<u64> {
        Ok(match &self.inner()?.source {
            Source::Wire(transfer) => transfer.request.session().options().max_body_size(),
            Source::Bytes(_) => u64::MAX,
        })
    }

    fn inner(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| poisoned())
    }

    /// Whether a live transfer is held: a body already held never is.
    pub(crate) fn is_live(&self) -> bool {
        self.inner()
            .is_ok_and(|inner| matches!(&inner.source, Source::Wire(transfer) if !transfer.closed))
    }

    /// Let go of the live transfer and its connection, keeping the cursor.
    pub(crate) fn release(&self) -> Result<()> {
        let mut inner = self.inner()?;
        if let Source::Wire(transfer) = &mut inner.source {
            transfer.reader = Box::new(io::empty());
            transfer.closed = true;
        }
        Ok(())
    }

    /// Re-open a released transfer at the cursor now, rather than on the
    /// next read; a live one is left as it is.
    pub(crate) fn reacquire(&self) -> Result<()> {
        let mut inner = self.inner()?;
        let delivered = inner.delivered;
        if let Source::Wire(transfer) = &mut inner.source {
            if transfer.closed {
                reopen(transfer, delivered).map_err(io_into_error)?;
            }
        }
        Ok(())
    }

    /// Publish the counters a read moved, for the accessors to read.
    fn publish(&self, inner: &Inner) {
        self.delivered.store(inner.delivered, Ordering::Release);
        self.resumes.store(inner.resumes, Ordering::Release);
    }
}

/// Read the rest from `position`, holding at most `limit` bytes, with room
/// for `reserve` of them made first - at most [`RESERVE_CAP`], so a stated
/// length the body does not keep costs no more than that.
///
/// The bytes are read straight into the vector, each region zeroed once and
/// no scratch chunk copied from.
fn read_rest(
    inner: &mut Inner,
    position: u64,
    limit: u64,
    reserve: usize,
    url: Option<&Url>,
) -> Result<Vec<u8>> {
    seek(inner, position)?;
    let mut bytes = vec![0_u8; reserve.clamp(DISCARD_CHUNK, RESERVE_CAP)];
    let mut filled = 0;
    loop {
        if filled == bytes.len() {
            let grow = DISCARD_CHUNK.max(bytes.len() / 2);
            bytes.resize(bytes.len() + grow, 0);
        }
        let read = read_inner(inner, &mut bytes[filled..]).map_err(io_into_error)?;
        if read == 0 {
            bytes.truncate(filled);
            return Ok(bytes);
        }
        filled += read;
        if filled as u64 > limit {
            return Err(oversized_body(limit, url));
        }
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Stream")
            .field("url", &self.url)
            .field("delivered", &self.delivered())
            .field("total", &self.total)
            .field("resumes", &self.resumes())
            .finish()
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let inner = self
            .inner
            .get_mut()
            .map_err(|_| io::Error::other(poisoned()))?;
        let read = read_inner(inner, buffer);
        *self.delivered.get_mut() = inner.delivered;
        *self.resumes.get_mut() = inner.resumes;
        read
    }
}

impl crate::IOMedia for Stream {
    crate::impl_default_iomedia!();
}

impl IOBase for Stream {
    /// Read at `offset`: forward to it when it is at or past the cursor,
    /// re-open a range when it is behind and the resource accepts one.
    ///
    /// # Errors
    ///
    /// A position behind the cursor of a resource without ranges is
    /// [`Error::Unsupported`]; a resumed transfer whose validator changed is
    /// [`Error::Conflict`]; a transport failure past the attempt limit is
    /// [`Error::Io`].
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let mut inner = self.inner()?;
        let read = seek(&mut inner, offset)
            .and_then(|()| read_inner(&mut inner, buffer).map_err(io_into_error));
        self.publish(&inner);
        read
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        ByteStream::from_reader(
            At {
                handle: self,
                position,
            },
            batch_size,
        )
    }

    /// The whole body from its first byte, within the session's
    /// `max_body_size`: a stream already read past its start re-opens a
    /// range for the prefix, and one without ranges is refused by name
    /// rather than answering the rest as if it were the whole.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.read_from(0, self.max_body_size()?)
    }

    fn pwrite(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize> {
        Err(refuse_write())
    }

    fn size(&self) -> u64 {
        self.total.unwrap_or_else(|| self.delivered())
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Err(refuse_write())
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Err(refuse_write())
    }

    fn uri(&self) -> Option<&Uri> {
        self.url.as_ref().map(AsRef::as_ref)
    }

    fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    fn mtime(&self) -> Option<i64> {
        self.headers.last_modified().ok().flatten()
    }

    fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
    }

    fn kind(&self) -> IOKind {
        IOKind::File
    }

    /// Re-open a closed transfer at the cursor: one ranged `GET`.
    fn open(&mut self) -> Result<()> {
        self.reacquire()
    }

    /// Whether the live transfer is held.
    fn opened(&self) -> bool {
        self.is_live()
    }

    /// Let go of the live transfer and its connection; the cursor stays,
    /// and the next read re-opens at it.
    fn close(&mut self) -> Result<()> {
        self.release()
    }

    fn is_container(&self) -> bool {
        false
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Err(Error::unsupported(
            "resolving a child of a response body",
            format!(
                "{}/{path}",
                self.url
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string)
            ),
        ))
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        Listing::empty()
    }
}

/// A reader over one handle from a position, for a bounded stream.
struct At<'handle, H: IOBase + ?Sized> {
    handle: &'handle H,
    position: u64,
}

impl<H: IOBase + ?Sized> Read for At<'_, H> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self
            .handle
            .pread(self.position, buffer)
            .map_err(io::Error::other)?;
        self.position = self.position.saturating_add(read as u64);
        Ok(read)
    }
}

/// A reader over `handle` from `position`, for [`ByteStream::from_reader`].
pub(crate) fn reader_at<H: IOBase + ?Sized>(handle: &H, position: u64) -> impl Read + '_ {
    At { handle, position }
}

/// Move the cursor to `position`: forward by discarding, backward by
/// re-opening a range when the resource accepts one.
fn seek(inner: &mut Inner, position: u64) -> Result<()> {
    if position == inner.delivered {
        return Ok(());
    }
    if let Source::Bytes(_) = inner.source {
        inner.delivered = position;
        return Ok(());
    }
    if position > inner.delivered {
        let mut chunk = vec![0_u8; DISCARD_CHUNK];
        while inner.delivered < position {
            let want = usize::try_from(position - inner.delivered)
                .unwrap_or(usize::MAX)
                .min(chunk.len());
            let read = read_inner(inner, &mut chunk[..want]).map_err(io_into_error)?;
            if read == 0 {
                // Past the end: the position is beyond the body, and a read
                // there is empty, so it is where the cursor now stands.
                inner.delivered = position;
                return Ok(());
            }
        }
        return Ok(());
    }
    let Source::Wire(transfer) = &mut inner.source else {
        return Ok(());
    };
    if !transfer.resumable {
        return Err(Error::unsupported(
            "reading behind the delivered position of a stream without ranges",
            &transfer.url,
        ));
    }
    let resumed = reopen(transfer, position).map_err(io_into_error)?;
    if resumed {
        inner.resumes += 1;
    }
    inner.delivered = position;
    Ok(())
}

/// Read into `buffer`, resuming a failed transfer while it may.
fn read_inner(inner: &mut Inner, buffer: &mut [u8]) -> io::Result<usize> {
    if buffer.is_empty() {
        return Ok(0);
    }
    match &mut inner.source {
        Source::Bytes(bytes) => {
            let Ok(position) = usize::try_from(inner.delivered) else {
                return Ok(0);
            };
            if position >= bytes.len() {
                return Ok(0);
            }
            let count = (bytes.len() - position).min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[position..position + count]);
            inner.delivered += count as u64;
            Ok(count)
        }
        Source::Wire(transfer) => loop {
            if transfer.closed {
                // Closed by the caller: re-opened here, at the cursor, which
                // is no failure and spends none of the resume budget.
                if !reopen(transfer, inner.delivered)? {
                    return Ok(0);
                }
            }
            if let Some(last) = transfer.last {
                let window = last.saturating_sub(transfer.start).saturating_add(1);
                if inner.delivered >= window {
                    return Ok(0);
                }
            }
            match read_window(transfer, inner.delivered, buffer) {
                Ok(read) => {
                    inner.delivered = inner.delivered.saturating_add(read as u64);
                    if read > 0 {
                        inner.failures = 0;
                    }
                    return Ok(read);
                }
                Err(error) => {
                    let client = transfer.request.session().client_ref();
                    if !transfer.resumable
                        || !is_resumable(&error)
                        || inner.failures.saturating_add(1) >= client.max_attempts()
                        || inner.resumes >= Stream::MAX_RESUMES
                    {
                        return Err(error);
                    }
                    inner.failures += 1;
                    client.pause(inner.failures, None);
                    match reopen(transfer, inner.delivered) {
                        Ok(true) => inner.resumes += 1,
                        // Everything asked for arrived; the failure was the
                        // end of the body announcing itself badly.
                        Ok(false) => return Ok(0),
                        Err(reopen_error) => {
                            // A refusal is the caller's to hear; a second
                            // transport failure leaves the first one to tell.
                            return Err(if carries_refusal(&reopen_error) {
                                reopen_error
                            } else {
                                error
                            });
                        }
                    }
                }
            }
        },
    }
}

/// One read within the window, the skip before it taken first.
fn read_window(transfer: &mut Transfer, delivered: u64, buffer: &mut [u8]) -> io::Result<usize> {
    while transfer.skip > 0 {
        let mut chunk = [0_u8; 4096];
        let want = usize::try_from(transfer.skip)
            .unwrap_or(usize::MAX)
            .min(chunk.len());
        let read = transfer.reader.read(&mut chunk[..want])?;
        if read == 0 {
            return Ok(0);
        }
        transfer.skip -= read as u64;
    }
    let want = match transfer.last {
        Some(last) => {
            let window = last.saturating_sub(transfer.start).saturating_add(1);
            usize::try_from(window.saturating_sub(delivered))
                .unwrap_or(usize::MAX)
                .min(buffer.len())
        }
        None => buffer.len(),
    };
    let read = transfer.reader.read(&mut buffer[..want])?;
    if read == 0
        && transfer
            .expected
            .is_some_and(|expected| delivered < expected)
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "the transfer ended after {delivered} of {} bytes",
                transfer.expected.unwrap_or(0)
            ),
        ));
    }
    Ok(read)
}

/// How many bytes the window holds, when the answer states it: a `206`'s
/// `Content-Range`, a `200`'s `Content-Length` past the skip, either
/// bounded by `last`.
fn window_length(headers: &Headers, status: Status, start: u64, last: Option<u64>) -> Option<u64> {
    let stated = if status == Status::PARTIAL_CONTENT {
        match headers.content_range().ok().flatten() {
            Some(ContentRange::Bytes { start, end, .. }) => Some(end.saturating_sub(start) + 1),
            _ => None,
        }
    } else {
        headers
            .content_length()
            .ok()
            .flatten()
            .map(|length| length.saturating_sub(start))
    };
    let bound = last.map(|last| last.saturating_sub(start) + 1);
    match (stated, bound) {
        (Some(stated), Some(bound)) => Some(stated.min(bound)),
        (stated, None) => stated,
        (None, _) => None,
    }
}

/// Re-open the transfer at `delivered` bytes into the window.
///
/// Answers `Ok(true)` with the reader replaced, `Ok(false)` when the window
/// is already complete, and the refusal otherwise: a `412` is the resource
/// having changed, any other status a remote refusal.
fn reopen(transfer: &mut Transfer, delivered: u64) -> io::Result<bool> {
    let from = transfer.start.saturating_add(delivered);
    if transfer.last.is_some_and(|last| from > last)
        || transfer
            .expected
            .is_some_and(|expected| delivered >= expected)
    {
        return Ok(false);
    }
    let (answer, url) = transfer
        .request
        .exchange_range(from, transfer.last, transfer.validator.as_deref())
        .map_err(io::Error::other)?;
    transfer.request.session().client_ref().record_resume();
    match answer.status {
        Status::PARTIAL_CONTENT => {
            // Splicing bytes whose place is unknown corrupts the body, so a
            // `206` is taken only when it says it starts at the cursor.
            match answer.headers.content_range().map_err(io::Error::other)? {
                Some(ContentRange::Bytes { start, .. }) if start == from => {}
                Some(ContentRange::Bytes { start, .. }) => {
                    return Err(misplaced(format!(
                        "resumed {url} from byte {from}, got a range starting at {start}"
                    )));
                }
                _ => {
                    return Err(misplaced(format!(
                        "resumed {url} from byte {from}, got a 206 stating no Content-Range"
                    )));
                }
            }
            transfer.reader = answer.body;
            transfer.closed = false;
            transfer.skip = 0;
            Ok(true)
        }
        Status::OK => {
            // A whole answer where a range named a validator is the resource
            // having changed when its validator moved (RFC 9110 answers the
            // whole representation rather than a 412 there); otherwise the
            // server ignored the range and the delivered prefix is skipped.
            if let (Some(sent), Some(now)) =
                (transfer.validator.as_deref(), validator_of(&answer.headers))
            {
                if sent != now {
                    return Err(io::Error::other(Error::conflict(
                        "resource",
                        "changed resource",
                        &url,
                    )));
                }
            }
            transfer.reader = answer.body;
            transfer.closed = false;
            transfer.skip = from;
            Ok(true)
        }
        Status::PRECONDITION_FAILED => Err(io::Error::other(Error::conflict(
            "resource",
            "changed resource",
            &url,
        ))),
        Status::RANGE_NOT_SATISFIABLE => {
            let total = answer
                .headers
                .content_range()
                .ok()
                .flatten()
                .and_then(|range| range.total());
            if total.is_some_and(|total| from >= total) {
                return Ok(false);
            }
            Err(io::Error::other(refusal(&answer, &url)))
        }
        _ => Err(io::Error::other(refusal(&answer, &url))),
    }
}

/// A resumed range whose bytes have no known place in the body: a refusal,
/// so it is the caller's to hear rather than the transport failure before it.
fn misplaced(message: String) -> io::Error {
    io::Error::other(Error::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        message,
    )))
}

/// A status that refuses a resume, as the remote refusal it is.
fn refusal(answer: &Answer, url: &Url) -> Error {
    Error::remote(
        "http",
        "GET",
        answer.status.code(),
        answer.status.reason(),
        "",
        url,
    )
}

/// The `If-Range` validator the first answer offers: a strong `ETag`, else
/// `Last-Modified` as written.
fn validator_of(headers: &Headers) -> Option<String> {
    if let Ok(Some(etag)) = headers.etag() {
        if !etag.is_weak() {
            return Some(etag.to_string());
        }
    }
    headers.get("last-modified").map(str::to_owned)
}

/// Whether an I/O error wraps a crate refusal - a changed resource, a
/// status - rather than reporting the transport.
fn carries_refusal(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Error>())
}

/// The crate error an I/O error carries, or the I/O error as one.
pub(crate) fn io_into_error(error: io::Error) -> Error {
    if error.get_ref().is_some_and(|inner| inner.is::<Error>()) {
        if let Some(inner) = error.into_inner() {
            if let Ok(inner) = inner.downcast::<Error>() {
                return *inner;
            }
        }
        return Error::Io(io::Error::other("an HTTP stream failure lost its cause"));
    }
    Error::Io(error)
}

/// A body longer than the caller holds.
pub(crate) fn oversized_body(limit: u64, url: Option<&Url>) -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "response body of {} exceeds max_body_size {limit} bytes",
            url.map_or_else(|| "<unlocated>".to_owned(), ToString::to_string)
        ),
    ))
}

/// The one refusal every write meets.
pub(crate) fn refuse_write() -> Error {
    Error::unsupported("writing a response body", "http")
}

fn poisoned() -> Error {
    Error::Io(io::Error::other(
        "an HTTP stream lock was poisoned by a panicking reader",
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/stream.rs` pins and a caller cannot reach.
    //!
    //! The ranged door `Request`'s `IOBase` leaf reads through: one ranged
    //! `GET` answered as a `Stream` over the asked window, whether the server
    //! honoured the range or ignored it.
    use crate::Result;
    use crate::http::{Request, Stream};

    /// One ranged `GET` of `request` from `start` to `last`, as a stream
    /// over that window.
    pub fn stream_range(request: &Request, start: u64, last: Option<u64>) -> Result<Stream> {
        let (answer, url) = request.exchange_range(start, last, None)?;
        Ok(Stream::new(request.clone(), url, answer, start, last))
    }
}
