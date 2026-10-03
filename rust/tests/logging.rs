//! The logging tree, one test file per file of `rust/src/logging/`.
//!
//! The tree is the process's: a test that dispatches through it, or changes
//! what every logger shares, holds `support::serial` so no other test moves
//! a level under it.

#[path = "support/logging.rs"]
mod support;

#[path = "logging/facade.rs"]
mod facade;
#[path = "logging/file.rs"]
mod file;
#[path = "logging/formatter.rs"]
mod formatter;
#[path = "logging/handler.rs"]
mod handler;
#[path = "logging/host.rs"]
mod host;
#[path = "logging/level.rs"]
mod level;
#[path = "logging/logger.rs"]
mod logger;
#[path = "logging/mod_.rs"]
mod mod_;
#[path = "logging/record.rs"]
mod record;
#[path = "logging/repeats.rs"]
mod repeats;
#[cfg(feature = "internals")]
#[path = "logging/terminal.rs"]
mod terminal;
#[cfg(feature = "internals")]
#[path = "logging/warning.rs"]
mod warning;
