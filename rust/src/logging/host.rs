//! A host's own logging, mirrored by the tree: what a binding attaches so
//! records reach the runtime's loggers of the same names.

use std::sync::Arc;

use super::logger::manager;
use super::{Level, Record};

/// A logging system the tree hands records to beside its own handlers - the
/// Python binding's `logging` - read as loggers of the same dotted names.
///
/// The tree asks a host two questions and caches both answers on each
/// logger until [`invalidate`] says the host's levels moved: the threshold
/// of one logger, read the first time a level is asked of it, and the
/// lowest threshold across all of them, which becomes the `log` facade's
/// ceiling. A record at or above a logger's host threshold is handed to
/// [`Self::handle`], which the host dispatches through its own loggers,
/// filters and handlers. Neither question is asked, and no record handed
/// over, while the tree holds a lock: a host may wait on its runtime.
pub trait Host: Send + Sync {
    /// The lowest level the host's logger `name` handles - its effective
    /// level, raised past any level the host disables - or `None` when it
    /// handles none.
    fn threshold(&self, name: &str) -> Option<Level>;

    /// The lowest level any of the host's loggers handles, `None` when none
    /// handles anything.
    fn lowest_threshold(&self) -> Option<Level>;

    /// Hands `record` to the host's logger of the record's name.
    fn handle(&self, record: &Record<'_>);
}

/// Attaches `host`, replacing any other; `None` detaches it. Every cached
/// threshold is dropped, and while a host is attached the last resort is
/// the host's own.
pub fn set_host(host: Option<Arc<dyn Host>>) {
    manager().set_host(host);
}

/// Whether a host is attached.
pub fn has_host() -> bool {
    manager().host().is_some()
}

/// Drops every cached threshold and reads the facade's ceiling again: what
/// a host calls whenever its own levels change.
pub fn invalidate() {
    manager().changed();
}
