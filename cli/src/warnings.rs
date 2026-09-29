//! What the core warned about while a command ran, printed once it is done.
//!
//! The FIX reader is lenient: a declaration it cannot keep as a file states
//! it is dropped or read another way, and the reader says so through the
//! `log` facade - one sentence naming what is wrong, where (line, column, the
//! element quoted) and what it did about it. The core emits nothing unless
//! the host installs a logger, so this binary installs one that keeps those
//! sentences rather than printing them as they arrive: a progress line is
//! being drawn while the files are read, and a warning written through it
//! would tear it. [`report`] prints what was kept once the line is done.

use std::sync::{Mutex, PoisonError};

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::style;

/// Every warning kept since the last [`drain`], in the order it arrived.
static HELD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The logger: keeps the core's warnings and errors, and nothing else.
struct Collector;

impl Log for Collector {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Warn && metadata.target().starts_with("yggdryl")
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            HELD.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Installs the collector as the process's logger, before any command runs.
pub fn install() {
    // Setting a logger fails only when one is set already, and nothing else
    // in this binary sets one.
    if log::set_logger(&Collector).is_ok() {
        log::set_max_level(LevelFilter::Warn);
    }
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
