//! What a walk of the sources found wanting.
//!
//! A chain asks each source in turn and stops at the first with an answer.
//! What happens when none has one is the difference between a tool that
//! fails fast and one that tries its best: here every source that was
//! configured and could not answer is kept, every source that was simply
//! not there is kept beside it, and the refusal at the end names them all -
//! or is no refusal at all, when nothing was configured, because an unsigned
//! request against a public resource is a valid thing to send. Each entry
//! is also logged at `DEBUG` under the walking module's logger.

use crate::{Error, Result};

/// The failures and absences one walk recorded.
#[derive(Debug)]
pub struct Report {
    /// What was being looked for, for the refusal: `AWS credentials`.
    what: &'static str,
    /// The `log` target the entries are logged under: the walking module's path.
    target: &'static str,
    failed: Vec<String>,
    absent: Vec<String>,
}

impl Report {
    /// An empty report of a walk for `what`, logged under `target`.
    pub const fn new(what: &'static str, target: &'static str) -> Self {
        Self {
            what,
            target,
            failed: Vec::new(),
            absent: Vec::new(),
        }
    }

    /// `source` was configured and could not answer.
    pub fn failed(&mut self, source: &str, error: impl std::fmt::Display) {
        let entry = format!("{source}: {error}");
        log::debug!(target: self.target, "{}: {entry}", self.what);
        self.failed.push(entry);
    }

    /// `source` was not configured.
    pub fn absent(&mut self, source: &str) {
        log::debug!(target: self.target, "{}: nothing configured in {source}", self.what);
        self.absent.push(source.to_owned());
    }

    /// Every `source: reason` recorded as failed, in the order asked.
    pub fn failures(&self) -> &[String] {
        &self.failed
    }

    /// Nothing answered: a refusal naming what failed, or `None` when
    /// nothing was configured to fail.
    pub fn conclude<T>(self) -> Result<Option<T>> {
        if self.failed.is_empty() {
            return Ok(None);
        }
        let mut message = format!(
            "no {} could be obtained: {}",
            self.what,
            self.failed.join("; ")
        );
        if !self.absent.is_empty() {
            message.push_str(&format!(
                "; nothing configured in: {}",
                self.absent.join(", ")
            ));
        }
        Err(refusal(message))
    }
}

/// A refusal of the credentials a provider looked for: `PermissionDenied`
/// carrying `message`.
pub fn refusal(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message.into(),
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/auth/report.rs` pins and a caller cannot reach.
    pub use super::Report;
}
