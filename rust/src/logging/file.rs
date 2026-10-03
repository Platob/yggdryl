//! A handler writing through any storage handle, one append per publish.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use super::{Handler, HandlerState, Level, Record};
use crate::{IOBase, IOMode, Result};

/// The most a [`FileHandler`] keeps of its buffer between publishes when
/// its capacity is smaller: a burst of records held once does not pin its
/// memory for the life of the handler.
const RETAINED: usize = 64 * 1024;

/// The most publishes one call makes: its own lines, then what other
/// threads logged while it published. A thread logging is never kept
/// publishing for every other; what remains goes with the next record,
/// [`Handler::flush`] or [`Handler::close`].
const ROUNDS: usize = 8;

/// Writes each record as one line through a storage handle - a local file,
/// an object in a bucket, a member of an archive, a buffer - Python's
/// `FileHandler` with its `MemoryHandler`'s batching folded in.
///
/// Every record is spelled into the handler's buffer and the buffer is
/// published with one [`IOBase::append_bytes`]: at once by default, as
/// Python's handler flushes each record, or - under
/// [`Self::with_capacity`] - once the buffer holds that many bytes or a
/// record at or above [`Self::with_flush_level`] arrives. The handle is
/// opened on the first publish, so an object store keeps what it learned
/// between appends, and closed by [`Handler::close`].
///
/// | Operation | Calls on the handle |
/// | --- | --- |
/// | construction, a record held back | none |
/// | the first publish | `open` + `append_bytes`, or `write_all_bytes` under [`IOMode::Overwrite`] |
/// | each later publish | `append_bytes` |
/// | `flush` with nothing held | none |
/// | `close` | the publish of what is held - and one more for what that publish raised - then `close` |
///
/// An append is a whole publish on every backend - one `PUT` of the object
/// on a store, a rewrite of the member in an archive - so a handler over a
/// remote store states a capacity. Records are held in memory between
/// publishes; [`shutdown`](super::shutdown), [`Handler::flush`] and a
/// dropped handler publish them, and a handler written to after its
/// [`close`](Handler::close) publishes each record at once, as Python's
/// closed handler does. A publish that fails loses the records it carried
/// and says so on standard error.
///
/// A record never waits on a publish in flight: one logged on another
/// thread meanwhile - a store's own runtime among them - is held, and the
/// publishing thread takes it in before it lets go of the handle, making
/// at most eight publishes for one record; what remains goes with the
/// next. A record the publishing thread logs itself, such as a credential
/// refresh or a retry its publish raises, or one logged under
/// [`Self::io`]'s guard, is carried by the next publish, and `close` gives
/// the lines its own publish raised one more. A `flush` or `close` on the thread holding the
/// handle is refused as a deadlock rather than waiting on itself.
///
/// ```
/// use yggdryl::holder::Buffer;
/// use yggdryl::logging::{FileHandler, Formatter, Handler, Level, Record};
/// use yggdryl::IOBase;
///
/// let handler = FileHandler::new(Buffer::new()).with_capacity(4096);
/// handler.set_formatter(Formatter::from_str("%(levelname)s %(message)s")?);
/// handler.handle(&Record::new("trades", Level::INFO, &"opened"));
/// handler.handle(&Record::new("trades", Level::WARNING, &"closed"));
/// assert_eq!(handler.io().read_all_bytes()?, b"");
/// handler.flush()?;
/// assert_eq!(handler.io().read_all_bytes()?, b"INFO opened\nWARNING closed\n");
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub struct FileHandler<H: IOBase> {
    state: HandlerState,
    mode: IOMode,
    capacity: usize,
    flush_level: Level,
    /// The lines held for the next publish, under a lock never held across
    /// a call on the handle: a record never waits on a publish in flight.
    lines: Mutex<Lines>,
    /// The handle, held by the thread publishing through it.
    io: Mutex<Io<H>>,
    /// The thread holding `io`, `0` for none.
    holder: AtomicU64,
}

/// What a [`FileHandler`] holds between publishes.
struct Lines {
    bytes: Vec<u8>,
    /// Lines the thread holding the handle logged itself - its own publish
    /// raised them, or they were logged under [`FileHandler::io`]'s guard -
    /// carried by the next publish rather than looping this one.
    aside: Vec<u8>,
    /// Whether a line at or above the flush level is held.
    urgent: bool,
    /// Whether the handler was closed: from then on each line publishes at
    /// once.
    closed: bool,
}

impl Lines {
    /// Whether what is held publishes now under `capacity`.
    fn is_due(&self, capacity: usize) -> bool {
        !self.bytes.is_empty() && (self.closed || self.urgent || self.bytes.len() >= capacity)
    }
}

/// The handle and the batch one publish carries.
struct Io<H> {
    handle: H,
    /// Swapped with the held lines, so neither buffer is rebuilt per publish.
    batch: Vec<u8>,
    opened: bool,
    published: bool,
}

/// The handle a [`FileHandler`] writes through, held under its lock; what
/// came due while it was held is published once it drops.
pub struct HandleGuard<'a, H: IOBase> {
    publishing: Option<Publishing<'a, H>>,
    handler: &'a FileHandler<H>,
}

impl<H: IOBase> std::ops::Deref for HandleGuard<'_, H> {
    type Target = H;

    fn deref(&self) -> &H {
        match &self.publishing {
            Some(publishing) => &publishing.io.handle,
            None => unreachable!("the handle is let go only as the guard drops"),
        }
    }
}

impl<H: IOBase> std::ops::DerefMut for HandleGuard<'_, H> {
    fn deref_mut(&mut self) -> &mut H {
        match &mut self.publishing {
            Some(publishing) => &mut publishing.io.handle,
            None => unreachable!("the handle is let go only as the guard drops"),
        }
    }
}

impl<H: IOBase> Drop for HandleGuard<'_, H> {
    fn drop(&mut self) {
        drop(self.publishing.take());
        if let Err(error) = self.handler.publish_due() {
            super::handler::report(&error, format_args!("while publishing held log records"));
        }
    }
}

/// `io` held by this thread, which the holder mark names until it drops.
struct Publishing<'a, H> {
    io: MutexGuard<'a, Io<H>>,
    holder: &'a AtomicU64,
}

impl<H> Drop for Publishing<'_, H> {
    fn drop(&mut self) {
        self.holder.store(0, Ordering::Release);
    }
}

/// What a publish takes from the held lines.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Take {
    /// What is due, with the lines an earlier publish set aside.
    Due,
    /// What is due, leaving aside what this call's own publish raised.
    DueOnly,
    /// Everything held, the lines set aside first.
    All,
}

impl<H: IOBase> FileHandler<H> {
    /// A handler appending each record to `handle` as it arrives.
    pub fn new(handle: H) -> Self {
        Self {
            state: HandlerState::new(),
            mode: IOMode::Append,
            capacity: 0,
            flush_level: Level::ERROR,
            lines: Mutex::new(Lines {
                bytes: Vec::new(),
                aside: Vec::new(),
                urgent: false,
                closed: false,
            }),
            io: Mutex::new(Io {
                handle,
                batch: Vec::new(),
                opened: false,
                published: false,
            }),
            holder: AtomicU64::new(0),
        }
    }

    /// The handler under `mode`: [`IOMode::Append`] keeps what the handle
    /// holds, [`IOMode::Overwrite`] replaces it with the first publish -
    /// once per handler, so a handler reopened after a close keeps what it
    /// wrote.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidRecord`] for any other mode.
    pub fn with_mode(mut self, mode: IOMode) -> Result<Self> {
        if !matches!(mode, IOMode::Append | IOMode::Overwrite) {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.mode"),
                reason: smol_str::format_smolstr!(
                    "expected append or overwrite for a log handler, got {mode}"
                ),
            });
        }
        self.mode = mode;
        Ok(self)
    }

    /// The handler holding records back until they reach `capacity` bytes;
    /// `0`, the default, publishes each record as it arrives.
    #[must_use]
    pub const fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// The handler publishing what it holds as soon as a record at or above
    /// `level` arrives; `ERROR` by default, as Python's `MemoryHandler`.
    #[must_use]
    pub const fn with_flush_level(mut self, level: Level) -> Self {
        self.flush_level = level;
        self
    }

    /// The mode the handler writes under.
    pub const fn mode(&self) -> IOMode {
        self.mode
    }

    /// The bytes the handler holds back before publishing.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// The level that publishes what is held at once.
    pub const fn flush_level(&self) -> Level {
        self.flush_level
    }

    /// The handle the handler writes through, locked: a record logged
    /// meanwhile is held and published once the guard drops. Taking it
    /// again on the thread that holds it waits on itself.
    pub fn io(&self) -> HandleGuard<'_, H> {
        let io = self.io.lock().unwrap_or_else(PoisonError::into_inner);
        HandleGuard {
            publishing: Some(self.mark(io)),
            handler: self,
        }
    }

    /// Holds one line logged at `level` - a record a host already spelled -
    /// and publishes when the capacity or the flush level says so.
    ///
    /// # Errors
    ///
    /// Returns the handle's refusal of a publish, the records it carried
    /// dropped.
    pub fn write(&self, line: &str, level: Level) -> Result<()> {
        let mut lines = self.lines();
        if self.holder.load(Ordering::Acquire) == super::formatter::thread_number() {
            lines.aside.extend_from_slice(line.as_bytes());
            lines.aside.push(b'\n');
            return Ok(());
        }
        lines.bytes.reserve(line.len() + 1);
        lines.bytes.extend_from_slice(line.as_bytes());
        lines.bytes.push(b'\n');
        lines.urgent |= level >= self.flush_level;
        let due = lines.is_due(self.capacity);
        drop(lines);
        if due { self.publish_due() } else { Ok(()) }
    }

    /// The held lines, whatever a panicking thread left: they are always
    /// whole lines.
    fn lines(&self) -> MutexGuard<'_, Lines> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `io`, marked as held by this thread.
    fn mark<'a>(&'a self, io: MutexGuard<'a, Io<H>>) -> Publishing<'a, H> {
        self.holder
            .store(super::formatter::thread_number(), Ordering::Release);
        Publishing {
            io,
            holder: &self.holder,
        }
    }

    /// `io` for a flush or a close, waiting on a publish in flight.
    ///
    /// # Errors
    ///
    /// Returns a deadlock refusal on the thread that already holds it -
    /// under [`Self::io`]'s guard, or inside its own publish - instead of
    /// waiting on itself.
    fn acquire(&self) -> Result<Publishing<'_, H>> {
        if self.holder.load(Ordering::Acquire) == super::formatter::thread_number() {
            return Err(crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Deadlock,
                "a log handler's handle is held by the thread flushing or closing it",
            )));
        }
        let io = self.io.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.mark(io))
    }

    /// Publishes what is due unless another thread is publishing: that
    /// thread takes it in before it lets go of the handle. One call makes
    /// at most [`ROUNDS`] publishes - its own lines, then what other
    /// threads logged meanwhile - and leaves the rest to the next.
    fn publish_due(&self) -> Result<()> {
        let mut outcome = Ok(());
        let mut rounds = 0;
        loop {
            let io = match self.io.try_lock() {
                Ok(io) => io,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => return outcome,
            };
            let mut publishing = self.mark(io);
            // Lines an earlier publish set aside ride with this call's first
            // publish, never with the ones after it.
            let take = if rounds == 0 {
                Take::Due
            } else {
                Take::DueOnly
            };
            if self.take(&mut publishing.io, take) {
                rounds += 1;
                let published = self.publish(&mut publishing.io);
                if outcome.is_ok() {
                    outcome = published;
                }
            }
            drop(publishing);
            // What came due while this thread published, or after its last
            // look and before it let go of the handle, found the handle held
            // and was left to this thread.
            if rounds >= ROUNDS || !self.lines().is_due(self.capacity) {
                return outcome;
            }
        }
    }

    /// Moves what `take` asks for from the held lines into `io`'s batch;
    /// answers whether there is anything to publish.
    fn take(&self, io: &mut Io<H>, take: Take) -> bool {
        let mut lines = self.lines();
        let lines = &mut *lines;
        let publishes = match take {
            Take::All => !lines.bytes.is_empty() || !lines.aside.is_empty(),
            Take::Due | Take::DueOnly => lines.is_due(self.capacity),
        };
        if !publishes {
            return false;
        }
        // Lines an earlier publish raised ride with the next one, ahead of
        // what was held since; they never start a publish of their own.
        if take != Take::DueOnly && !lines.aside.is_empty() {
            lines.aside.extend_from_slice(&lines.bytes);
            std::mem::swap(&mut lines.aside, &mut lines.bytes);
            lines.aside.clear();
            if lines.aside.capacity() > RETAINED {
                lines.aside = Vec::new();
            }
        }
        std::mem::swap(&mut lines.bytes, &mut io.batch);
        lines.urgent = false;
        true
    }

    /// Publishes `io`'s batch: one `append_bytes`, or the replacing
    /// `write_all_bytes` an overwriting handler's first publish is.
    fn publish(&self, io: &mut Io<H>) -> Result<()> {
        let published = (|| {
            if !io.opened {
                io.handle.open()?;
                io.opened = true;
            }
            if self.mode == IOMode::Overwrite && !io.published {
                io.handle.write_all_bytes(&io.batch)
            } else {
                io.handle.append_bytes(&io.batch).map(drop)
            }
        })();
        io.published |= published.is_ok();
        io.batch.clear();
        if io.batch.capacity() > self.capacity.max(RETAINED) {
            io.batch = Vec::new();
        }
        published
    }
}

impl<H: IOBase> Handler for FileHandler<H> {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn emit(&self, record: &Record<'_>, line: &str) -> Result<()> {
        self.write(line, record.level())
    }

    /// Publishes everything held, waiting on a publish in flight.
    fn flush(&self) -> Result<()> {
        let flushed = {
            let mut publishing = self.acquire()?;
            if self.take(&mut publishing.io, Take::All) {
                self.publish(&mut publishing.io)
            } else {
                Ok(())
            }
        };
        flushed.and(self.publish_due())
    }

    /// Publishes everything held - and once more, the lines that publish
    /// raised about itself, a credential refresh or a retry on a store -
    /// then closes the handle.
    fn close(&self) -> Result<()> {
        let mut publishing = self.acquire()?;
        let io = &mut *publishing.io;
        let mut outcome = Ok(());
        for _ in 0..2 {
            if self.take(io, Take::All) {
                let published = self.publish(io);
                if outcome.is_ok() {
                    outcome = published;
                }
            }
        }
        self.lines().closed = true;
        if io.opened {
            io.opened = false;
            let closed = io.handle.close();
            if outcome.is_ok() {
                outcome = closed;
            }
        }
        outcome
    }
}

impl<H: IOBase> Drop for FileHandler<H> {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            super::handler::report(&error, format_args!("while closing a file handler"));
        }
    }
}

impl<H: IOBase> fmt::Debug for FileHandler<H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never waits on a publish in flight, nor on this thread's own guard.
        let url = match self.io.try_lock() {
            Ok(io) => io.handle.url().map(ToString::to_string),
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().handle.url().map(ToString::to_string)
            }
            Err(TryLockError::WouldBlock) => None,
        };
        let pending = self.lines().bytes.len();
        formatter
            .debug_struct("FileHandler")
            .field("url", &url)
            .field("mode", &self.mode)
            .field("capacity", &self.capacity)
            .field("flush_level", &self.flush_level)
            .field("pending", &pending)
            .field("state", &self.state)
            .finish()
    }
}
