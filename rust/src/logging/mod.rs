//! Python's `logging`, owned by the core: named loggers in a dotted tree,
//! numeric levels, handlers and `%`-style formatters, behind the `log`
//! facade every crate in the build already logs through.
//!
//! [`install`] makes the tree the facade's backend, so `log::info!` from
//! this crate, from its dependencies and from the application reaches the
//! logger named after the record's target - `yggdryl::iceberg::table` is
//! `yggdryl.iceberg.table` - and the facade's own ceiling follows the
//! tree's levels, so a record nothing handles is never built.
//! [`basic_config`] is the one call an application makes: it installs, and
//! gives the root a handler on standard error writing the
//! [terminal format](Formatter::terminal) - the time, the level's glyph and
//! name in its colour, the thread, the logger, the call site, the message -
//! coloured where [`is_color_enabled`] says and plain anywhere else. With nothing configured at
//! all, the last resort writes warnings the same way. Code then only logs:
//! `get_logger("trades.feed").info("opened")`.
//!
//! | Python | Here |
//! | --- | --- |
//! | `logging.getLogger(name)` | [`get_logger`] |
//! | `logger.setLevel`, `getEffectiveLevel`, `isEnabledFor` | [`Logger::set_level`], [`Logger::effective_level`], [`Logger::is_enabled_for`] |
//! | `logger.addHandler`, `propagate`, `disabled` | [`Logger::add_handler`], [`Logger::set_propagating`], [`Logger::set_disabled`] |
//! | `Handler`, `StreamHandler`, `FileHandler`, `NullHandler` | [`Handler`], [`StreamHandler`], [`FileHandler`] over any [`IOBase`](crate::IOBase), [`NullHandler`] |
//! | `MemoryHandler` | [`FileHandler::with_capacity`] and [`FileHandler::with_flush_level`] |
//! | `Formatter(fmt, datefmt)` | [`Formatter::from_str`], [`Formatter::with_datefmt`] |
//! | `colorlog`, a modern terminal | [`Formatter::terminal`] - the default of [`basic_config`] and the last resort - and the `%(levelcolor)s`, `%(levelglyph)s`, `%(dim)s`, `%(bold)s`, `%(reset)s` keys, coloured where [`is_color_enabled`] says |
//! | `basicConfig`, `disable`, `shutdown`, `lastResort` | [`basic_config`], [`disable`], [`shutdown`], [`set_last_resort`] |
//!
//! A binding attaches its runtime's own logging as a [`Host`]: the tree
//! then asks the host's logger of the same name what it handles, caches the
//! answer until the host says its levels moved, and hands it every record
//! it handles - the Python binding's records reach `logging`'s own
//! handlers that way, at the levels `logging` itself states.
//!
//! The process ends without dropping what a static holds, so an
//! application calls [`shutdown`] before it returns - Python registers the
//! same call at exit - and a handler holding records back publishes them.
//!
//! ```
//! use std::sync::Arc;
//!
//! use yggdryl::holder::Buffer;
//! use yggdryl::logging::{self, BasicConfig, FileHandler, Formatter, Level};
//! use yggdryl::IOBase;
//!
//! let file = Arc::new(FileHandler::new(Buffer::new()));
//! logging::basic_config(
//!     BasicConfig::new()
//!         .with_level(Level::INFO)
//!         .with_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?)
//!         .with_handler(file.clone()),
//! )?;
//!
//! log::info!(target: "trades::feed", "opened {} venues", 3);
//! log::debug!(target: "trades::feed", "below the root's level");
//! logging::get_logger("trades.book").warning("crossed");
//!
//! logging::shutdown();
//! assert_eq!(
//!     file.io().read_all_bytes()?,
//!     b"INFO trades.feed opened 3 venues\nWARNING trades.book crossed\n"
//! );
//! # Ok::<(), yggdryl::Error>(())
//! ```

mod facade;
mod file;
mod formatter;
mod handler;
mod host;
mod level;
pub(crate) mod logger;
mod record;
mod repeats;
pub(crate) mod terminal;
pub(crate) mod warning;

use std::sync::{Arc, LazyLock};

pub use facade::{foreign_level, install, is_installed, set_foreign_level};
pub use file::{FileHandler, HandleGuard};
pub use formatter::{Formatter, set_thread_name};
pub use handler::{Filter, Handler, HandlerState, NullHandler, StreamHandler};
pub use host::{Host, has_host, invalidate, set_host};
pub use level::Level;
pub use logger::{Logger, get_logger};
pub use record::Record;
pub use repeats::{Counted, Repeat, Repeats};
pub use terminal::is_color_enabled;

use crate::Result;

/// When the tree first answered: what `relativeCreated` counts from.
pub(crate) static START: LazyLock<i64> = LazyLock::new(record::now);

/// Drops every record at or below `level`, on every logger, whatever its
/// own level; `NOTSET` lifts it - Python's `logging.disable`.
pub fn disable(level: Level) {
    logger::manager().disable(level);
}

/// The handler a record no handler takes is written by: standard error at
/// `WARNING` until replaced, `None` once removed - Python's `lastResort`.
/// Under a host it is never used: the host has its own.
pub fn last_resort() -> Option<Arc<dyn Handler>> {
    logger::manager().last_resort()
}

/// Replaces the last resort; `None` drops a record no handler takes.
pub fn set_last_resort(handler: Option<Arc<dyn Handler>>) {
    logger::manager().set_last_resort(handler);
}

/// Publishes and closes every handler attached anywhere in the tree, and
/// the last resort - Python's `logging.shutdown`, the call an application
/// makes before it exits. A handler that fails says so on standard error
/// and the others still close; a handler logged to again reopens.
pub fn shutdown() {
    for handler in logger::manager().handlers() {
        if let Err(error) = handler.close() {
            handler::report(&error, format_args!("while closing a handler"));
        }
    }
}

/// What [`basic_config`] gives the root: Python's `basicConfig` arguments.
#[derive(Clone, Default)]
pub struct BasicConfig {
    level: Option<Level>,
    formatter: Option<Formatter>,
    handlers: Vec<Arc<dyn Handler>>,
    force: bool,
}

impl BasicConfig {
    /// No level, the [terminal format](Formatter::terminal), a
    /// handler on standard error, and a root that already has handlers
    /// left alone.
    pub fn new() -> Self {
        Self::default()
    }

    /// States `level` on the root.
    #[must_use]
    pub const fn with_level(mut self, level: Level) -> Self {
        self.level = Some(level);
        self
    }

    /// Spells records with `formatter` on every handler that has none.
    #[must_use]
    pub fn with_formatter(mut self, formatter: Formatter) -> Self {
        self.formatter = Some(formatter);
        self
    }

    /// Attaches `handler` to the root in place of standard error; each call
    /// adds one.
    #[must_use]
    pub fn with_handler(mut self, handler: Arc<dyn Handler>) -> Self {
        self.handlers.push(handler);
        self
    }

    /// Closes and removes the root's handlers first, so the configuration
    /// applies to a root that has some.
    #[must_use]
    pub const fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

impl std::fmt::Debug for BasicConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BasicConfig")
            .field("level", &self.level)
            .field("formatter", &self.formatter)
            .field("handlers", &self.handlers.len())
            .field("force", &self.force)
            .finish()
    }
}

/// Installs the tree as the `log` facade's backend and, when the root has
/// no handler - or `force` removed them - attaches the configured handlers
/// (standard error when none is), gives each without a formatter the
/// configured one and states the level - Python's `basicConfig`.
///
/// # Errors
///
/// Returns [`install`]'s refusal, with nothing configured.
pub fn basic_config(config: BasicConfig) -> Result<()> {
    // One configuration at a time: two callers both finding a bare root would
    // both attach.
    static CONFIGURING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    install()?;
    let _configuring = CONFIGURING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = get_logger("");
    if config.force {
        for handler in root.handlers() {
            root.remove_handler(&handler);
            if let Err(error) = handler.close() {
                handler::report(&error, format_args!("while closing a handler"));
            }
        }
    }
    if !root.handlers().is_empty() {
        return Ok(());
    }
    let handlers = if config.handlers.is_empty() {
        vec![Arc::new(StreamHandler::stderr()) as Arc<dyn Handler>]
    } else {
        config.handlers
    };
    let formatter = match config.formatter {
        Some(formatter) => formatter,
        None => Formatter::terminal(),
    };
    for handler in handlers {
        if !handler.has_formatter() {
            handler.set_formatter(formatter.clone());
        }
        root.add_handler(handler);
    }
    if let Some(level) = config.level {
        root.set_level(level);
    }
    Ok(())
}
