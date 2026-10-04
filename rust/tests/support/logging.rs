//! What the logging tests share: the lock every test touching the process's
//! tree holds, and a handler that keeps what it is handed.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yggdryl::logging::{self, Formatter, Handler, HandlerState, Level, Record, StreamHandler};

/// Holds the process's tree for one test, restored to what a fresh process
/// has: the root at `WARNING` with no handler, nothing disabled, no host,
/// the last resort on standard error, no foreign floor.
pub fn serial() -> MutexGuard<'static, ()> {
    static TREE: Mutex<()> = Mutex::new(());
    let guard = TREE.lock().unwrap_or_else(PoisonError::into_inner);
    let root = logging::get_logger("");
    for handler in root.handlers() {
        root.remove_handler(&handler);
    }
    root.set_level(Level::WARNING);
    logging::disable(Level::NOTSET);
    logging::set_host(None);
    logging::set_foreign_level(Level::NOTSET);
    let last_resort = StreamHandler::stderr();
    last_resort.set_level(Level::WARNING);
    logging::set_last_resort(Some(Arc::new(last_resort)));
    guard
}

/// One record as a [`Collect`] kept it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Kept {
    pub name: String,
    pub level: Level,
    pub line: String,
    pub target: String,
    pub file: Option<String>,
    pub at: Option<u32>,
}

/// A handler keeping every record it is handed, spelled.
#[derive(Default)]
pub struct Collect {
    state: HandlerState,
    kept: Mutex<Vec<Kept>>,
}

impl Collect {
    /// A collector shared as a handler, keeping each record's message alone.
    pub fn shared() -> Arc<Self> {
        let collect = Self::default();
        collect.set_formatter(Formatter::default());
        Arc::new(collect)
    }

    /// A collector stating no formatter, so it spells the handler default.
    pub fn bare() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Every record kept so far, leaving none.
    pub fn take(&self) -> Vec<Kept> {
        std::mem::take(&mut *self.kept.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The lines kept so far, leaving none.
    pub fn lines(&self) -> Vec<String> {
        self.take().into_iter().map(|kept| kept.line).collect()
    }
}

impl Handler for Collect {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn emit(&self, record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Kept {
                name: record.name().to_owned(),
                level: record.level(),
                line: line.to_owned(),
                target: record.target().to_owned(),
                file: record.file().map(str::to_owned),
                at: record.line(),
            });
        Ok(())
    }
}
