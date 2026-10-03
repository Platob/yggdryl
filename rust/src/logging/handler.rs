//! What a logger hands its records to: Python's `Handler`, its stream and
//! null handlers, and the state every handler shares.

use std::cell::RefCell;
use std::fmt;
use std::io::IsTerminal;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use super::{Formatter, Level, Record};
use crate::Result;

/// A predicate a record must pass to be handled: on a logger, to be handled
/// at all; on a handler, to be emitted by it - Python's filter callables.
pub type Filter = Arc<dyn Fn(&Record<'_>) -> bool + Send + Sync>;

/// What every handler holds: the level it emits from, the formatter it
/// spells a record with and the filters a record must pass - Python's
/// `Handler` attributes, each changed through `&self` because a handler is
/// shared by every logger it is attached to.
#[derive(Default)]
pub struct HandlerState {
    level: AtomicU8,
    formatter: RwLock<Option<Formatter>>,
    filters: RwLock<Arc<[Filter]>>,
}

impl HandlerState {
    /// A handler's state at `NOTSET`, with no formatter and no filter.
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for HandlerState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandlerState")
            .field("level", &Level::new(self.level.load(Ordering::Relaxed)))
            .field("formatter", &*read(&self.formatter))
            .field("filters", &read(&self.filters).len())
            .finish()
    }
}

/// Where a logger's records go: a stream, a file, a host's own logging, a
/// collector - Python's `Handler`.
///
/// An implementation owns its [`HandlerState`] and writes one formatted
/// record in [`Self::emit`]; every other method is provided. A logger hands
/// a handler each record at or above the handler's level, and
/// [`Self::handle`] runs the filters, spells the record with the
/// formatter - the [terminal format](Formatter::terminal) when none is set,
/// coloured where the handler [is](Self::is_colored) - into a buffer the
/// thread reuses, and emits it. A record a handler fails to emit is said on
/// standard error by [`Self::handle_error`] and never reaches the caller:
/// logging does not fail the code that logs.
///
/// ```
/// use std::sync::{Arc, Mutex};
///
/// use yggdryl::logging::{Handler, HandlerState, Level, Record};
///
/// #[derive(Default)]
/// struct Collect {
///     state: HandlerState,
///     lines: Mutex<Vec<String>>,
/// }
///
/// impl Handler for Collect {
///     fn state(&self) -> &HandlerState {
///         &self.state
///     }
///
///     fn emit(&self, _record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
///         self.lines.lock().unwrap().push(line.to_owned());
///         Ok(())
///     }
/// }
///
/// let collect = Collect::default();
/// let record = Record::new("trades", Level::INFO, &"opened")
///     .with_location("src/feed.rs", 7)
///     .with_thread("main")
///     .with_created(1_700_000_000_123_000_000);
/// collect.handle(&record);
/// // No formatter set: the terminal line, plain because nothing is coloured.
/// assert_eq!(
///     *collect.lines.lock().unwrap(),
///     ["2023-11-14 22:13:20,123 • INFO     [main] trades feed:7 › opened"]
/// );
/// ```
pub trait Handler: Send + Sync {
    /// The handler's level, formatter and filters.
    fn state(&self) -> &HandlerState;

    /// Writes one record, already spelled as `line` without a line ending.
    ///
    /// # Errors
    ///
    /// Returns whatever the destination refused; [`Self::handle`] reports it.
    fn emit(&self, record: &Record<'_>, line: &str) -> Result<()>;

    /// Publishes whatever the handler holds back.
    ///
    /// # Errors
    ///
    /// Returns whatever the destination refused.
    fn flush(&self) -> Result<()> {
        Ok(())
    }

    /// Publishes what the handler holds back and lets go of its destination;
    /// a handler emitting after it reopens the destination.
    ///
    /// # Errors
    ///
    /// Returns whatever the destination refused.
    fn close(&self) -> Result<()> {
        self.flush()
    }

    /// The level the handler emits from.
    fn level(&self) -> Level {
        Level::new(self.state().level.load(Ordering::Relaxed))
    }

    /// Emits records at `level` and above from now on.
    fn set_level(&self, level: Level) {
        self.state().level.store(level.get(), Ordering::Relaxed);
    }

    /// The formatter the handler spells records with: the
    /// [terminal format](Formatter::terminal) when none was set, so every
    /// line carries its timestamp, level, thread and call site.
    fn formatter(&self) -> Formatter {
        read(&self.state().formatter)
            .clone()
            .unwrap_or_else(Formatter::terminal)
    }

    /// Whether a formatter was set, which `basic_config` leaves alone.
    fn has_formatter(&self) -> bool {
        read(&self.state().formatter).is_some()
    }

    /// Spells records with `formatter` from now on.
    fn set_formatter(&self, formatter: Formatter) {
        *self
            .state()
            .formatter
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(formatter);
    }

    /// Adds `filter`: a record it refuses is not emitted.
    fn add_filter(&self, filter: Filter) {
        let mut filters = self
            .state()
            .filters
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *filters = filters.iter().cloned().chain([filter]).collect();
    }

    /// Removes `filter`, the very one added; answers whether it was there.
    fn remove_filter(&self, filter: &Filter) -> bool {
        let mut filters = self
            .state()
            .filters
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let kept: Arc<[Filter]> = filters
            .iter()
            .filter(|held| !Arc::ptr_eq(held, filter))
            .cloned()
            .collect();
        let removed = kept.len() != filters.len();
        *filters = kept;
        removed
    }

    /// Whether `record` passes every filter.
    fn filter(&self, record: &Record<'_>) -> bool {
        let filters = Arc::clone(&read(&self.state().filters));
        filters.iter().all(|filter| filter(record))
    }

    /// Whether the handler writes to a colour terminal, so its formatter's
    /// styles are spelled; `false` unless the handler says otherwise.
    fn is_colored(&self) -> bool {
        false
    }

    /// Spells `record` into `line` with the handler's formatter - the
    /// terminal format when none was set - coloured when the handler
    /// [is](Self::is_colored).
    fn format(&self, record: &Record<'_>, line: &mut String) {
        // Out of its lock before the message renders: a message whose
        // `Display` logs reaches this handler again, and a `set_formatter`
        // queued between the two reads would wait on this one forever.
        let held = read(&self.state().formatter).clone();
        let formatter = held.as_ref().unwrap_or_else(|| Formatter::terminal_ref());
        if self.is_colored() {
            formatter.format_colored_into(record, line);
        } else {
            formatter.format_into(record, line);
        }
    }

    /// Filters, spells and emits `record`; answers whether the filters let
    /// it through. The level is the logger's to check, as in Python.
    fn handle(&self, record: &Record<'_>) -> bool {
        if !self.filter(record) {
            return false;
        }
        let emitted = with_line(|line| {
            self.format(record, line);
            self.emit(record, line)
        });
        if let Err(error) = emitted {
            self.handle_error(record, &error);
        }
        true
    }

    /// Says on standard error that `record` could not be emitted, as
    /// Python's `handleError` does; the code that logged never sees it.
    fn handle_error(&self, record: &Record<'_>, error: &crate::Error) {
        report(
            error,
            format_args!(
                "while emitting a {} record of {}: {}",
                record.level(),
                record.name(),
                record.message()
            ),
        );
    }
}

/// Says a logging failure on standard error under Python's
/// `--- Logging error ---`, `doing` naming what failed. A standard error
/// that cannot take it - closed, a full disk, a reader gone - is left
/// alone: logging does not fail the code that logs, even there.
pub(super) fn report(error: &dyn fmt::Display, doing: fmt::Arguments<'_>) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "--- Logging error ---\n{error}\n{doing}"
    );
}

/// A lock read whatever a panicking writer left behind: the state a lock
/// guards here is always whole.
pub(super) fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

/// The most a thread's line buffer keeps between records: a record longer
/// than this is spelled in a buffer of its own size and the buffer shrinks
/// back.
const LINE_CAPACITY: usize = 64 * 1024;

/// Runs `spell` over the thread's line buffer, cleared; a record logged
/// while the buffer is in use - a handler that logs - or while the thread's
/// storage is torn down - from a thread-local's `Drop` - gets a buffer of
/// its own.
fn with_line<T>(spell: impl FnOnce(&mut String) -> T) -> T {
    thread_local! {
        static LINE: RefCell<String> = const { RefCell::new(String::new()) };
    }
    let mut spell = Some(spell);
    let answered = LINE.try_with(|cell| {
        let mut line = cell.try_borrow_mut().ok()?;
        let spell = spell.take()?;
        line.clear();
        let answer = spell(&mut line);
        if line.capacity() > LINE_CAPACITY {
            *line = String::new();
        }
        Some(answer)
    });
    match (answered, spell) {
        (Ok(Some(answer)), _) => answer,
        (_, Some(spell)) => spell(&mut String::new()),
        (_, None) => unreachable!("`spell` is taken only by the run that answers"),
    }
}

/// Writes each record as one line to standard error, standard output or
/// any writer - Python's `StreamHandler` - flushing after each.
///
/// A handler on standard error or standard output is coloured when that
/// stream is a colour terminal, as [`is_color_enabled`](super::is_color_enabled)
/// decides once, when the handler is built; a writer is plain unless
/// [`Self::set_colored`] says otherwise.
///
/// ```
/// use yggdryl::logging::{Handler, Level, Record, StreamHandler};
///
/// let handler = StreamHandler::new(Vec::new());
/// handler.handle(&Record::new("trades", Level::WARNING, &"late fill"));
/// assert_eq!(handler.level(), Level::NOTSET);
/// ```
pub struct StreamHandler {
    state: HandlerState,
    stream: Stream,
    colored: AtomicBool,
}

/// Where a [`StreamHandler`] writes.
enum Stream {
    Stderr,
    Stdout,
    Writer(Writer),
}

/// A caller's writer, and what it logged while it wrote: standard error and
/// standard output lock re-entrantly, a writer's lock does not.
struct Writer {
    writer: Mutex<Box<dyn Write + Send>>,
    /// The thread writing, `0` for none.
    writing: AtomicU64,
    /// Lines the writer logged into this handler while it wrote, written
    /// after the line it was writing.
    aside: Mutex<Vec<u8>>,
}

/// The writer marked as written by this thread until the mark drops.
struct Writing<'a>(&'a AtomicU64);

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

impl Writer {
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        let me = super::formatter::thread_number();
        if self.writing.load(Ordering::Acquire) == me {
            let mut aside = self.aside.lock().unwrap_or_else(PoisonError::into_inner);
            aside.extend_from_slice(line.as_bytes());
            aside.push(b'\n');
            return Ok(());
        }
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.writing.store(me, Ordering::Release);
        let _writing = Writing(&self.writing);
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        // What writing that line logged, once: what writing these logs
        // waits for the next line.
        let aside = std::mem::take(&mut *self.aside.lock().unwrap_or_else(PoisonError::into_inner));
        if !aside.is_empty() {
            writer.write_all(&aside)?;
        }
        writer.flush()
    }

    fn flush(&self) -> std::io::Result<()> {
        if self.writing.load(Ordering::Acquire) == super::formatter::thread_number() {
            // The writer asked for it while writing: the line it is writing
            // flushes it.
            return Ok(());
        }
        self.writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}

impl StreamHandler {
    /// Writes to the process's standard error, Python's default stream.
    pub fn stderr() -> Self {
        Self {
            state: HandlerState::new(),
            stream: Stream::Stderr,
            colored: AtomicBool::new(super::is_color_enabled(std::io::stderr().is_terminal())),
        }
    }

    /// Writes to the process's standard output.
    pub fn stdout() -> Self {
        Self {
            state: HandlerState::new(),
            stream: Stream::Stdout,
            colored: AtomicBool::new(super::is_color_enabled(std::io::stdout().is_terminal())),
        }
    }

    /// Writes to `writer`.
    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self {
            state: HandlerState::new(),
            stream: Stream::Writer(Writer {
                writer: Mutex::new(Box::new(writer)),
                writing: AtomicU64::new(0),
                aside: Mutex::default(),
            }),
            colored: AtomicBool::new(false),
        }
    }

    /// Spells the formatter's styles, or stops spelling them.
    pub fn set_colored(&self, colored: bool) {
        self.colored.store(colored, Ordering::Relaxed);
    }
}

impl Handler for StreamHandler {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn is_colored(&self) -> bool {
        self.colored.load(Ordering::Relaxed)
    }

    fn emit(&self, _record: &Record<'_>, line: &str) -> Result<()> {
        fn write_line(mut writer: impl Write, line: &str) -> std::io::Result<()> {
            writer.write_all(line.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()
        }
        match &self.stream {
            Stream::Stderr => write_line(std::io::stderr().lock(), line)?,
            Stream::Stdout => write_line(std::io::stdout().lock(), line)?,
            Stream::Writer(writer) => writer.write_line(line)?,
        }
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        match &self.stream {
            Stream::Stderr => std::io::stderr().flush()?,
            Stream::Stdout => std::io::stdout().flush()?,
            Stream::Writer(writer) => writer.flush()?,
        }
        Ok(())
    }
}

impl fmt::Debug for StreamHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stream = match self.stream {
            Stream::Stderr => "stderr",
            Stream::Stdout => "stdout",
            Stream::Writer(_) => "writer",
        };
        formatter
            .debug_struct("StreamHandler")
            .field("stream", &stream)
            .field("colored", &self.is_colored())
            .field("state", &self.state)
            .finish()
    }
}

/// Takes every record and writes none - Python's `NullHandler`: attached to
/// a library's logger, it keeps the last resort from speaking for it.
#[derive(Debug, Default)]
pub struct NullHandler {
    state: HandlerState,
}

impl NullHandler {
    /// A handler that writes nothing.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Handler for NullHandler {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn emit(&self, _record: &Record<'_>, _line: &str) -> Result<()> {
        Ok(())
    }

    /// Takes the record without spelling it.
    fn handle(&self, _record: &Record<'_>) -> bool {
        true
    }
}
