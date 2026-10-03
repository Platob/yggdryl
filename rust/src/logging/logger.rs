//! The tree of named loggers: Python's `Logger` and its manager.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use super::handler::read;
use super::level::{NEVER, Threshold};
use super::repeats::RECORDS;
use super::{Counted, Filter, Handler, Host, Level, Record, Repeat, StreamHandler};

/// A named logger: where code logs and where handlers are attached -
/// Python's `Logger`.
///
/// Loggers form a tree by their dotted names: `trades.feed` is a child of
/// `trades`, whatever order they were first asked for in, and every logger
/// descends from the root, named `root`. A logger stating no level takes
/// its nearest ancestor's, and the root's is `WARNING` until set. A record
/// logged on a logger reaches its handlers, then - while each logger
/// propagates - its ancestors' handlers; a record no handler takes is
/// written by the [last resort](super::last_resort) when it is at or above
/// that handler's level.
///
/// Asking whether a level is enabled is the fast path: the answer is cached
/// on each logger and dropped whenever any level, the process-wide
/// [`disable`](super::disable) or the host changes, as Python's own cache
/// is - so a disabled record costs a few atomic loads, takes no lock and
/// builds nothing.
/// Under a [host](super::Host), a logger is also enabled for a level the
/// host's logger of the same name handles, and the record reaches both.
///
/// ```
/// use std::sync::Arc;
///
/// use yggdryl::holder::Buffer;
/// use yggdryl::logging::{get_logger, FileHandler, Level};
/// use yggdryl::IOBase;
///
/// let feed = get_logger("docs.logger.feed");
/// assert_eq!(feed.parent().map(|parent| parent.name().to_owned()).as_deref(), Some("root"));
/// let docs = get_logger("docs.logger");
/// assert_eq!(feed.parent().as_ref(), Some(&docs));
///
/// docs.set_level(Level::DEBUG);
/// assert_eq!(feed.effective_level(), Level::DEBUG);
/// assert!(feed.is_enabled_for(Level::DEBUG));
///
/// let file = Arc::new(FileHandler::new(Buffer::new()));
/// docs.add_handler(file.clone());
/// feed.info("opened");
/// feed.log(Level::TRACE, "below the logger's level");
/// // The default line: timestamp, level, thread, logger, call site, message.
/// let written = String::from_utf8(file.io().read_all_bytes()?).expect("utf-8");
/// assert_eq!(written.lines().count(), 1);
/// assert!(written.contains(" • INFO     [main] docs.logger.feed "), "{written}");
/// assert!(written.ends_with(" › opened\n"), "{written}");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct Logger(pub(super) Arc<Node>);

/// One logger's state.
pub(super) struct Node {
    pub(super) name: Box<str>,
    parent: RwLock<Option<Arc<Node>>>,
    level: AtomicU8,
    propagating: AtomicBool,
    disabled: AtomicBool,
    /// Whether records logged here are counted and repeats dropped:
    /// [`INHERIT`], [`DEDUPLICATE`] or [`REPEAT`].
    deduplicating: AtomicU8,
    handlers: RwLock<Arc<[Arc<dyn Handler>]>>,
    filters: RwLock<Arc<[Filter]>>,
    /// The tree's threshold for this logger, stamped with the generation it
    /// was computed under.
    native: AtomicU64,
    /// The host's threshold for this logger, stamped the same way.
    hosted: AtomicU64,
    /// Whether this logger deduplicates, as stated here or inherited,
    /// stamped the same way.
    repeats: AtomicU64,
}

/// A logger taking its ancestors' deduplication.
const INHERIT: u8 = 0;
/// A logger dropping repeated records.
const DEDUPLICATE: u8 = 1;
/// A logger passing every record.
const REPEAT: u8 = 2;

/// The tree: every logger by name, and what the whole tree shares.
pub(super) struct Manager {
    root: Arc<Node>,
    loggers: RwLock<HashMap<Box<str>, Arc<Node>>>,
    /// Bumped by every change a cached threshold may depend on.
    generation: AtomicU64,
    disable: AtomicU8,
    /// How many handlers are attached across the tree.
    attached: AtomicUsize,
    last_resort: RwLock<Option<Arc<dyn Handler>>>,
    host: RwLock<Option<Arc<dyn Host>>>,
}

/// The process's tree.
pub(super) fn manager() -> &'static Manager {
    static MANAGER: LazyLock<Manager> = LazyLock::new(|| {
        LazyLock::force(&super::START);
        let root = Arc::new(Node::new("root", None));
        root.level.store(Level::WARNING.get(), Ordering::Relaxed);
        let last_resort = StreamHandler::stderr();
        last_resort.set_level(Level::WARNING);
        Manager {
            root,
            loggers: RwLock::default(),
            generation: AtomicU64::new(1),
            disable: AtomicU8::new(0),
            attached: AtomicUsize::new(0),
            last_resort: RwLock::new(Some(Arc::new(last_resort))),
            host: RwLock::new(None),
        }
    });
    &MANAGER
}

/// The logger named `name`, made the first time it is asked for; `""` and
/// `root` are the root - Python's `getLogger`.
///
/// Asking again for a logger that exists allocates nothing.
pub fn get_logger(name: &str) -> Logger {
    let manager = manager();
    if name.is_empty() || name == "root" {
        return Logger(Arc::clone(&manager.root));
    }
    if let Some(node) = read(&manager.loggers).get(name) {
        return Logger(Arc::clone(node));
    }
    let mut loggers = manager
        .loggers
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(node) = loggers.get(name) {
        return Logger(Arc::clone(node));
    }
    let parent = ancestors(name)
        .find_map(|ancestor| loggers.get(ancestor))
        .map_or_else(|| Arc::clone(&manager.root), Arc::clone);
    let node = Arc::new(Node::new(name, Some(parent)));
    // A logger asked for before this one and named below it hung from a
    // farther ancestor until now, as in Python's `_fixupChildren`.
    for child in loggers.values() {
        let below = child
            .name
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('.'));
        if !below {
            continue;
        }
        let mut parent = child.parent.write().unwrap_or_else(PoisonError::into_inner);
        let farther = parent
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &manager.root) || held.name.len() < name.len());
        if farther {
            *parent = Some(Arc::clone(&node));
        }
    }
    loggers.insert(name.into(), Arc::clone(&node));
    Logger(node)
}

/// The dotted ancestors of `name`, nearest first: `a.b` and `a` for `a.b.c`.
/// The logger named `name` when the tree has one, else the nearest
/// ancestor it has, else the root - never a new logger.
pub(super) fn nearest_logger(name: &str) -> Logger {
    let manager = manager();
    let loggers = read(&manager.loggers);
    std::iter::once(name)
        .chain(ancestors(name))
        .find_map(|held| loggers.get(held))
        .map_or_else(
            || Logger(Arc::clone(&manager.root)),
            |node| Logger(Arc::clone(node)),
        )
}

fn ancestors(name: &str) -> impl Iterator<Item = &str> {
    let mut rest = name;
    std::iter::from_fn(move || {
        let (ancestor, _) = rest.rsplit_once('.')?;
        rest = ancestor;
        Some(ancestor)
    })
    .filter(|ancestor| !ancestor.is_empty())
}

impl Node {
    fn new(name: &str, parent: Option<Arc<Self>>) -> Self {
        Self {
            name: name.into(),
            parent: RwLock::new(parent),
            level: AtomicU8::new(Level::NOTSET.get()),
            propagating: AtomicBool::new(true),
            disabled: AtomicBool::new(false),
            deduplicating: AtomicU8::new(INHERIT),
            handlers: RwLock::new(Arc::new([])),
            filters: RwLock::new(Arc::new([])),
            native: AtomicU64::new(0),
            hosted: AtomicU64::new(0),
            repeats: AtomicU64::new(0),
        }
    }

    fn parent(&self) -> Option<Arc<Self>> {
        read(&self.parent).clone()
    }

    /// The level stated here or by the nearest ancestor stating one.
    fn effective_level(&self) -> Level {
        let level = Level::new(self.level.load(Ordering::Relaxed));
        if level != Level::NOTSET {
            return level;
        }
        let mut node = self.parent();
        while let Some(held) = node {
            let level = Level::new(held.level.load(Ordering::Relaxed));
            if level != Level::NOTSET {
                return level;
            }
            node = held.parent();
        }
        Level::NOTSET
    }

    /// The lowest level the tree's own handlers take a record of this
    /// logger at: its effective level raised past the disabled levels, or
    /// none at all when the logger is disabled or no handler could take it.
    fn native_threshold(&self, manager: &Manager) -> Threshold {
        let generation = manager.generation.load(Ordering::SeqCst);
        cached(&self.native, generation, || {
            if self.disabled.load(Ordering::Relaxed) || !manager.is_reachable() {
                return NEVER;
            }
            let disabled = Threshold::from(manager.disable.load(Ordering::Relaxed)) + 1;
            Threshold::from(self.effective_level().get()).max(disabled)
        })
    }

    /// The lowest level the host's logger of this name handles, raised
    /// past the tree's disabled levels, or none when this logger is
    /// disabled: the tree's own switches hold on both sides.
    fn hosted_threshold(&self, manager: &Manager) -> Threshold {
        let generation = manager.generation.load(Ordering::SeqCst);
        cached(&self.hosted, generation, || {
            if self.disabled.load(Ordering::Relaxed) {
                return NEVER;
            }
            let disabled = Threshold::from(manager.disable.load(Ordering::Relaxed)) + 1;
            manager.host().map_or(NEVER, |host| {
                host.threshold(&self.name)
                    .map_or(NEVER, |level| Threshold::from(level.get()).max(disabled))
            })
        })
    }

    /// Which sides take a record at `level`: the tree's handlers, the host.
    pub(super) fn gates(&self, level: Level) -> (bool, bool) {
        let manager = manager();
        let level = Threshold::from(level.get());
        (
            level >= self.native_threshold(manager),
            level >= self.hosted_threshold(manager),
        )
    }

    /// Hands `record` to the tree's handlers when `native`, and to the host
    /// when `hosted`.
    /// A deduplicating logger counts the record first, at its origin, so a
    /// repeat reaches no handler and no host: the first occurrence goes on
    /// as logged, a tenfold one as [`Counted`], any other is dropped.
    pub(super) fn dispatch(self: &Arc<Self>, record: &Record<'_>, native: bool, hosted: bool) {
        if self.is_deduplicating(manager()) {
            match RECORDS.count_record(record) {
                Repeat::First | Repeat::Untracked => {}
                Repeat::Tenfold(count) => {
                    let counted = Counted::new(count, record.message());
                    return self.deliver(&record.with_message(&counted), native, hosted);
                }
                Repeat::Repeated => return,
            }
        }
        self.deliver(record, native, hosted);
    }

    /// Whether records logged here are deduplicated: stated here or by the
    /// nearest ancestor stating it, off at the root unless set.
    fn is_deduplicating(&self, manager: &Manager) -> bool {
        let generation = manager.generation.load(Ordering::SeqCst);
        cached(&self.repeats, generation, || {
            let stated = |node: &Self| match node.deduplicating.load(Ordering::Relaxed) {
                DEDUPLICATE => Some(1),
                REPEAT => Some(0),
                _ => None,
            };
            if let Some(answer) = stated(self) {
                return answer;
            }
            let mut node = self.parent();
            while let Some(held) = node {
                if let Some(answer) = stated(&held) {
                    return answer;
                }
                node = held.parent();
            }
            0
        }) == 1
    }

    fn deliver(self: &Arc<Self>, record: &Record<'_>, native: bool, hosted: bool) {
        // A record handed over directly - `Logger::handle` - passed no gate.
        if self.disabled.load(Ordering::Relaxed) {
            return;
        }
        let manager = manager();
        if native {
            self.call_handlers(manager, record);
        }
        if hosted && let Some(host) = manager.host() {
            host.handle(record);
        }
    }

    /// Python's `Logger.handle` and `callHandlers`: this logger's filters,
    /// then every handler up the propagating chain at or below the record's
    /// level, then the last resort when no handler was found.
    fn call_handlers(self: &Arc<Self>, manager: &Manager, record: &Record<'_>) {
        if self.disabled.load(Ordering::Relaxed) {
            return;
        }
        let filters = Arc::clone(&read(&self.filters));
        if !filters.iter().all(|filter| filter(record)) {
            return;
        }
        let mut found = 0_usize;
        let mut node = Arc::clone(self);
        loop {
            let handlers = Arc::clone(&read(&node.handlers));
            for handler in handlers.iter() {
                found += 1;
                if record.level() >= handler.level() {
                    handler.handle(record);
                }
            }
            if !node.propagating.load(Ordering::Relaxed) {
                break;
            }
            match node.parent() {
                Some(parent) => node = parent,
                None => break,
            }
        }
        if found == 0
            && manager.host().is_none()
            && let Some(last_resort) = manager.last_resort()
            && record.level() >= last_resort.level()
        {
            last_resort.handle(record);
        }
    }
}

/// A threshold cached in `cell` under `generation`, computed when the cell
/// holds another generation's.
fn cached(cell: &AtomicU64, generation: u64, compute: impl FnOnce() -> Threshold) -> Threshold {
    let held = cell.load(Ordering::Relaxed);
    if held >> 16 == generation {
        return Threshold::try_from(held & 0xFFFF).unwrap_or(NEVER);
    }
    let threshold = compute();
    cell.store(generation << 16 | u64::from(threshold), Ordering::Relaxed);
    threshold
}

impl Manager {
    pub(super) fn host(&self) -> Option<Arc<dyn Host>> {
        read(&self.host).clone()
    }

    pub(super) fn set_host(&self, host: Option<Arc<dyn Host>>) {
        *self.host.write().unwrap_or_else(PoisonError::into_inner) = host;
        self.changed();
    }

    pub(super) fn last_resort(&self) -> Option<Arc<dyn Handler>> {
        read(&self.last_resort).clone()
    }

    pub(super) fn set_last_resort(&self, handler: Option<Arc<dyn Handler>>) {
        *self
            .last_resort
            .write()
            .unwrap_or_else(PoisonError::into_inner) = handler;
        self.changed();
    }

    pub(super) fn disable(&self, level: Level) {
        self.disable.store(level.get(), Ordering::Relaxed);
        self.changed();
    }

    /// Whether the tree's handlers can take any record: one is attached, or
    /// the last resort speaks because no host does.
    fn is_reachable(&self) -> bool {
        self.attached.load(Ordering::Relaxed) > 0
            || (read(&self.host).is_none() && read(&self.last_resort).is_some())
    }

    /// Every logger, the root first.
    pub(super) fn nodes(&self) -> Vec<Arc<Node>> {
        std::iter::once(Arc::clone(&self.root))
            .chain(read(&self.loggers).values().cloned())
            .collect()
    }

    /// Every handler attached anywhere in the tree, each once, and the last
    /// resort.
    pub(super) fn handlers(&self) -> Vec<Arc<dyn Handler>> {
        let mut handlers: Vec<Arc<dyn Handler>> = Vec::new();
        let attached = self
            .nodes()
            .into_iter()
            .flat_map(|node| read(&node.handlers).iter().cloned().collect::<Vec<_>>())
            .chain(self.last_resort());
        for handler in attached {
            if !handlers.iter().any(|held| Arc::ptr_eq(held, &handler)) {
                handlers.push(handler);
            }
        }
        handlers
    }

    /// Drops every cached threshold and, while the tree is the facade's
    /// backend, moves the facade's ceiling to the most verbose level any
    /// logger now handles. No lock is held while the host is asked.
    pub(super) fn changed(&self) {
        let mut generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if !super::facade::is_installed() {
            return;
        }
        loop {
            log::set_max_level(Level::filter_for(self.lowest_threshold()));
            // A change racing this one may have read the old state; whoever
            // sees the generation move reads it again.
            let current = self.generation.load(Ordering::SeqCst);
            if current == generation {
                break;
            }
            generation = current;
        }
    }

    /// The lowest level any logger handles, on either side.
    fn lowest_threshold(&self) -> Threshold {
        let native = if self.is_reachable() {
            let disabled = Threshold::from(self.disable.load(Ordering::Relaxed)) + 1;
            // A disabled logger refuses its own records, but its children -
            // some not asked for yet - inherit its level, so it counts.
            self.nodes()
                .iter()
                .map(|node| Threshold::from(node.effective_level().get()).max(disabled))
                .min()
                .unwrap_or(NEVER)
        } else {
            NEVER
        };
        let hosted = self.host().map_or(NEVER, |host| {
            host.lowest_threshold()
                .map_or(NEVER, |level| Threshold::from(level.get()))
        });
        native.min(hosted)
    }
}

impl Logger {
    /// The logger's dotted name; `root` for the root.
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The logger this one hangs from; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(Self)
    }

    /// The logger named `suffix` below this one - Python's `getChild`.
    pub fn child(&self, suffix: &str) -> Self {
        if self.0.parent().is_none() {
            return get_logger(suffix);
        }
        get_logger(&format!("{}.{suffix}", self.0.name))
    }

    /// The level stated on this logger; `NOTSET` when it takes its
    /// ancestors'.
    pub fn level(&self) -> Level {
        Level::new(self.0.level.load(Ordering::Relaxed))
    }

    /// States `level` on this logger; `NOTSET` takes the ancestors' again.
    pub fn set_level(&self, level: Level) {
        self.0.level.store(level.get(), Ordering::Relaxed);
        manager().changed();
    }

    /// The level this logger handles from: its own, or the nearest
    /// ancestor's that states one.
    pub fn effective_level(&self) -> Level {
        self.0.effective_level()
    }

    /// Whether a record at `level` would be handled here, by the tree's
    /// handlers or the host's.
    pub fn is_enabled_for(&self, level: Level) -> bool {
        let (native, hosted) = self.0.gates(level);
        native || hosted
    }

    /// Whether records go on to the ancestors' handlers after this logger's.
    pub fn is_propagating(&self) -> bool {
        self.0.propagating.load(Ordering::Relaxed)
    }

    /// Sends records on to the ancestors' handlers, or stops them here.
    pub fn set_propagating(&self, propagating: bool) {
        self.0.propagating.store(propagating, Ordering::Relaxed);
    }

    /// Whether the logger drops every record logged on it.
    pub fn is_disabled(&self) -> bool {
        self.0.disabled.load(Ordering::Relaxed)
    }

    /// Drops every record logged on this logger, or stops dropping them.
    pub fn set_disabled(&self, disabled: bool) {
        self.0.disabled.store(disabled, Ordering::Relaxed);
        manager().changed();
    }

    /// Whether this logger states its own deduplication: `Some(true)` drops
    /// repeated records, `Some(false)` passes every record, `None` takes
    /// the nearest ancestor's.
    pub fn deduplicating(&self) -> Option<bool> {
        match self.0.deduplicating.load(Ordering::Relaxed) {
            DEDUPLICATE => Some(true),
            REPEAT => Some(false),
            _ => None,
        }
    }

    /// States this logger's deduplication; `None` takes the ancestors'
    /// again. A deduplicating logger counts each record by its
    /// [`stable_hash`](Record::stable_hash) - the logger, the level and the
    /// message - where it is logged: the first occurrence goes on as logged,
    /// the 10th, 100th, 1000th as `message (seen N times)`, and every other
    /// reaches no handler and no host. Off at the root unless set.
    pub fn set_deduplicating(&self, deduplicating: Option<bool>) {
        let stated = match deduplicating {
            Some(true) => DEDUPLICATE,
            Some(false) => REPEAT,
            None => INHERIT,
        };
        self.0.deduplicating.store(stated, Ordering::Relaxed);
        manager().changed();
    }

    /// Whether records logged here are deduplicated: as stated here or by
    /// the nearest ancestor stating it.
    pub fn is_deduplicating(&self) -> bool {
        self.0.is_deduplicating(manager())
    }

    /// The handlers attached to this logger, in the order they were added.
    pub fn handlers(&self) -> Vec<Arc<dyn Handler>> {
        read(&self.0.handlers).to_vec()
    }

    /// Attaches `handler`, once: a handler already attached stays where it is.
    pub fn add_handler(&self, handler: Arc<dyn Handler>) {
        let mut handlers = self
            .0
            .handlers
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if handlers.iter().any(|held| Arc::ptr_eq(held, &handler)) {
            return;
        }
        *handlers = handlers.iter().cloned().chain([handler]).collect();
        drop(handlers);
        manager().attached.fetch_add(1, Ordering::Relaxed);
        manager().changed();
    }

    /// Detaches `handler`, the very one added; answers whether it was
    /// attached. The handler is not closed.
    pub fn remove_handler(&self, handler: &Arc<dyn Handler>) -> bool {
        let mut handlers = self
            .0
            .handlers
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let kept: Arc<[Arc<dyn Handler>]> = handlers
            .iter()
            .filter(|held| !Arc::ptr_eq(held, handler))
            .cloned()
            .collect();
        if kept.len() == handlers.len() {
            return false;
        }
        *handlers = kept;
        drop(handlers);
        manager().attached.fetch_sub(1, Ordering::Relaxed);
        manager().changed();
        true
    }

    /// Whether this logger or an ancestor its records propagate to has a
    /// handler - Python's `hasHandlers`.
    pub fn has_handlers(&self) -> bool {
        let mut node = Some(Arc::clone(&self.0));
        while let Some(held) = node {
            if !read(&held.handlers).is_empty() {
                return true;
            }
            if !held.propagating.load(Ordering::Relaxed) {
                return false;
            }
            node = held.parent();
        }
        false
    }

    /// Adds `filter`: a record it refuses reaches none of the tree's handlers.
    pub fn add_filter(&self, filter: Filter) {
        let mut filters = self
            .0
            .filters
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *filters = filters.iter().cloned().chain([filter]).collect();
    }

    /// Removes `filter`, the very one added; answers whether it was there.
    pub fn remove_filter(&self, filter: &Filter) -> bool {
        let mut filters = self
            .0
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

    /// Logs `message` at `level`, dated now and located at the caller,
    /// when the level is enabled; the message is rendered only by a handler
    /// that writes it.
    #[track_caller]
    pub fn log(&self, level: Level, message: impl fmt::Display) {
        let (native, hosted) = self.0.gates(level);
        if !native && !hosted {
            return;
        }
        let location = std::panic::Location::caller();
        let record = Record::new(&self.0.name, level, &message)
            .with_location(location.file(), location.line());
        self.0.dispatch(&record, native, hosted);
    }

    /// Logs `message` at `DEBUG`.
    #[track_caller]
    pub fn debug(&self, message: impl fmt::Display) {
        self.log(Level::DEBUG, message);
    }

    /// Logs `message` at `INFO`.
    #[track_caller]
    pub fn info(&self, message: impl fmt::Display) {
        self.log(Level::INFO, message);
    }

    /// Logs `message` at `WARNING`.
    #[track_caller]
    pub fn warning(&self, message: impl fmt::Display) {
        self.log(Level::WARNING, message);
    }

    /// Logs `message` at `ERROR`.
    #[track_caller]
    pub fn error(&self, message: impl fmt::Display) {
        self.log(Level::ERROR, message);
    }

    /// Logs `message` at `CRITICAL`.
    #[track_caller]
    pub fn critical(&self, message: impl fmt::Display) {
        self.log(Level::CRITICAL, message);
    }

    /// Hands `record` to this logger's handlers, its ancestors' and the
    /// host, whatever its level - Python's `Logger.handle`, for a record a
    /// caller built and checked itself.
    pub fn handle(&self, record: &Record<'_>) {
        self.0.dispatch(record, true, true);
    }
}

impl PartialEq for Logger {
    /// The same logger: one name is one logger.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Logger {}

impl fmt::Debug for Logger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Logger")
            .field("name", &self.name())
            .field("level", &self.level())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Logger {
    /// Python's `repr` without the brackets: the name and effective level.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.name(), self.effective_level())
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/logging/logger.rs` pins and a caller cannot reach.

    /// The name of the logger a facade record is logged on once the facade
    /// makes no more loggers: `name`'s own when the tree has it, else its
    /// nearest ancestor's, else `root`.
    #[must_use]
    pub fn nearest_logger(name: &str) -> String {
        super::nearest_logger(name).name().to_owned()
    }
}
