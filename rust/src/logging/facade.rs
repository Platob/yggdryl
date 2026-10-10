//! The tree as the backend of the `log` facade every crate in the build
//! logs through.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError, RwLock};

use super::handler::read;
use super::logger::manager;
use super::{Level, Logger, Record, get_logger};
use crate::{Error, Result};

/// Whether the tree is the facade's backend.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The lowest level a record from outside the workspace's crates is admitted
/// at.
static FOREIGN: AtomicU8 = AtomicU8::new(0);

/// The most distinct targets the facade makes a logger for and remembers.
/// Past it a target makes none: its records go to the nearest logger the
/// tree has, looked up by name on each record, so the tree stays bounded.
/// A target is a static module path in practice, so code that does not
/// mint targets never reaches the bound.
const TARGETS: usize = 4096;

/// Makes the tree the backend of the [`log`] facade: every `log::info!`,
/// the crate's own included, reaches the logger named after its target,
/// `::` spelled `.` - `yggdryl::iceberg::table` is `yggdryl.iceberg.table`.
/// A module of one of the workspace's crates is named by its module path
/// under `yggdryl` whatever crate holds it - `yggdryl_fix::build` is
/// `yggdryl.fix.build` - because the logger names are the names a Python
/// logger is configured by: one tree, whatever crate a module lives in.
/// The facade's ceiling then follows the tree, so a record no logger
/// handles is refused by the facade before its message is built. Installing
/// twice is one installation.
///
/// # Errors
///
/// Returns [`Error::Conflict`] when another logger already is the facade's
/// backend: the facade takes one per process, and a library does not
/// replace what the application chose.
pub fn install() -> Result<()> {
    static FACADE: Facade = Facade;
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _installing = INSTALLING.lock().unwrap_or_else(PoisonError::into_inner);
    if is_installed() {
        return Ok(());
    }
    log::set_logger(&FACADE)
        .map_err(|_| Error::conflict("logging tree", "logger", "the log facade"))?;
    INSTALLED.store(true, Ordering::Release);
    manager().changed();
    Ok(())
}

/// Whether the tree is the backend of the `log` facade.
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Admits a facade record whose target is outside the workspace's crates -
/// a dependency of the build, or the application's own crates - only at
/// `level` and above, whatever its logger's level; `NOTSET`, the default,
/// admits what the tree enables. The bindings state `WARNING`, so the
/// crates the build depends on say what went wrong and never what they did.
pub fn set_foreign_level(level: Level) {
    FOREIGN.store(level.get(), Ordering::Relaxed);
}

/// The level a facade record from outside the workspace's crates is
/// admitted from.
pub fn foreign_level() -> Level {
    Level::new(FOREIGN.load(Ordering::Relaxed))
}

/// The workspace's crates whose records the tree names as its own, each
/// with the logger its crate root is named: a logger is named by its module
/// path under `yggdryl`, whatever crate holds the module: the market
/// crate's modules log under `yggdryl` itself, so `yggdryl_market::graph`
/// is `yggdryl.graph`, and every other crate under its own name, so
/// `yggdryl_fix::build` is `yggdryl.fix.build`. The logger names are the
/// names a Python logger is configured by, one tree whatever crate a module
/// lives in. A crate this table does not name - `yggdryl_cli`, a binding, a
/// dependency - is foreign, its target's `::` spelled `.`.
const CRATES: [(&str, &str); 8] = [
    ("yggdryl", "yggdryl"),
    ("yggdryl_avro", "yggdryl.avro"),
    ("yggdryl_excel", "yggdryl.excel"),
    ("yggdryl_fix", "yggdryl.fix"),
    ("yggdryl_iceberg", "yggdryl.iceberg"),
    ("yggdryl_market", "yggdryl"),
    ("yggdryl_parquet", "yggdryl.parquet"),
    ("yggdryl_xmla", "yggdryl.xmla"),
];

/// The logger `target`'s crate root is named and the module path below it,
/// `::` first - empty for the crate root itself - or `None` where the
/// target's crate is none of [`CRATES`].
fn own_crate(target: &str) -> Option<(&'static str, &str)> {
    let (name, path) = target.split_at(target.find("::").unwrap_or(target.len()));
    CRATES
        .iter()
        .find(|(held, _)| *held == name)
        .map(|(_, root)| (*root, path))
}

/// Whether `target` lies outside the workspace's crates.
fn is_foreign(target: &str) -> bool {
    own_crate(target).is_none()
}

/// Hands `piece` the logger name `target`'s records carry, in order and
/// building no text: its crate's logger then its module path, every `::`
/// spelled `.`.
fn name_pieces(target: &str, mut piece: impl FnMut(&str)) {
    let (root, path) = own_crate(target).unwrap_or(("", target));
    piece(root);
    for (index, step) in path.split("::").enumerate() {
        if index > 0 {
            piece(".");
        }
        piece(step);
    }
}

/// The logger name `target`'s records carry.
fn logger_name(target: &str) -> String {
    let mut name = String::with_capacity(target.len());
    name_pieces(target, |piece| name.push_str(piece));
    name
}

/// Feeds `state` the logger name `target`'s records carry, building no
/// text: what a warning raised at `target` is deduplicated under, so a
/// module keeps its key whatever crate holds it.
pub(super) fn write_logger_name(target: &str, state: &mut impl std::hash::Hasher) {
    name_pieces(target, |piece| state.write(piece.as_bytes()));
}

/// Whether the foreign floor admits a record at `level` from `target`.
fn admitted(target: &str, level: Level) -> bool {
    level.get() >= FOREIGN.load(Ordering::Relaxed) || !is_foreign(target)
}

/// The logger a target's records are logged on, and - past the bound - the
/// name they carry instead of its own.
fn logger_for(target: &str) -> (Logger, Option<String>) {
    static LOGGERS: LazyLock<RwLock<HashMap<Box<str>, Logger>>> = LazyLock::new(RwLock::default);
    static FULL: AtomicBool = AtomicBool::new(false);
    if let Some(logger) = read(&LOGGERS).get(target) {
        return (logger.clone(), None);
    }
    let name = logger_name(target);
    if !FULL.load(Ordering::Relaxed) {
        let mut loggers = LOGGERS.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(logger) = loggers.get(target) {
            return (logger.clone(), None);
        }
        if loggers.len() < TARGETS {
            let logger = get_logger(&name);
            loggers.insert(target.into(), logger.clone());
            return (logger, None);
        }
        FULL.store(true, Ordering::Relaxed);
    }
    // Past the bound a target makes no logger, so targets minted per tenant
    // or per request cannot grow the tree: its records go to the nearest
    // logger the tree has - its own, once asked for by name - under their
    // own name.
    (super::logger::nearest_logger(&name), Some(name))
}

/// The facade's view of the tree.
struct Facade;

impl log::Log for Facade {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        let level = Level::from(metadata.level());
        admitted(metadata.target(), level) && logger_for(metadata.target()).0.is_enabled_for(level)
    }

    fn log(&self, record: &log::Record<'_>) {
        let level = Level::from(record.level());
        let target = record.target();
        if !admitted(target, level) {
            return;
        }
        let (logger, named) = logger_for(target);
        let (native, hosted) = logger.0.gates(level);
        if !native && !hosted {
            return;
        }
        let message = record.args();
        let name = named.as_deref().unwrap_or_else(|| logger.name());
        let mut logged =
            Record::new(name, level, message).with_target(target, record.module_path());
        if let (Some(file), Some(line)) = (record.file(), record.line()) {
            logged = logged.with_location(file, line);
        }
        logger.0.dispatch(&logged, native, hosted);
    }

    fn flush(&self) {
        for handler in manager().handlers() {
            if let Err(error) = handler.flush() {
                super::handler::report(&error, format_args!("while flushing a handler"));
            }
        }
    }
}
