//! What the core warned about while a command ran, printed once it is done.
//!
//! The FIX reader is lenient: a declaration it cannot keep as a file states
//! it is dropped or read another way, and the reader says so through the
//! `log` facade - one sentence naming what is wrong, where (line, column, the
//! element quoted) and what it did about it. This binary makes the core's
//! logging tree the facade's backend and attaches a handler to its root that
//! keeps those sentences rather than printing them as they arrive: a
//! progress line is being drawn while the files are read, and a warning
//! written through it would tear it. [`report`] prints what was kept once the
//! line is done.

use std::sync::{Arc, Mutex, PoisonError};

use yggdryl::logging::{self, Formatter, Handler, HandlerState, Level, Record};

use crate::style;

/// Every warning kept since the last [`drain`], in the order it arrived.
static HELD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The handler: keeps the core's warnings and errors, and nothing else.
struct Collector {
    state: HandlerState,
}

impl Handler for Collector {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn emit(&self, _record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
        HELD.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.to_owned());
        Ok(())
    }
}

/// Installs the core's logging tree as the process's logger, its root
/// keeping the core's own warnings, before any command runs.
pub fn install() {
    // Installing fails only when another logger is set, and nothing else in
    // this binary sets one.
    if logging::install().is_err() {
        return;
    }
    let collector = Collector {
        state: HandlerState::new(),
    };
    collector.set_level(Level::WARNING);
    // The report prints each sentence under its own marker, so the line is
    // the message alone rather than the terminal layout.
    collector.set_formatter(Formatter::default());
    collector.add_filter(Arc::new(|record: &Record<'_>| {
        record.target().starts_with("yggdryl")
    }));
    logging::get_logger("").add_handler(Arc::new(collector));
}

/// Every warning kept so far, leaving none behind.
pub fn drain() -> Vec<String> {
    std::mem::take(&mut *HELD.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Prints every warning kept so far: nothing when there is none, one
/// workflow warning each under `--annotate`, otherwise a count and one line
/// each.
pub fn report(annotate: bool) {
    let held = drain();
    if held.is_empty() {
        return;
    }
    if annotate {
        for record in &held {
            outln!("::warning title=fix reader::{}", style::annotation(record));
        }
        return;
    }
    style::warn(&format!(
        "{} warning(s) while reading: what a file states that the reader could not keep as stated",
        held.len()
    ));
    for record in &held {
        style::note(record);
    }
}
