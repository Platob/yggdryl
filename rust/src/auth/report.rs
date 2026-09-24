//! What a walk of the sources found wanting.
//!
//! A chain asks each source in turn and stops at the first with an answer.
//! What happens when none has one is the difference between a tool that
//! fails fast and one that tries its best: here every source that was
//! configured and could not answer is kept, every source that was simply
//! not there is kept beside it, and the refusal at the end names them all -
//! or is no refusal at all, when nothing was configured, because an unsigned
//! request against a public resource is a valid thing to send.

use crate::{Error, Result};

/// The failures and absences one walk recorded.
#[derive(Debug)]
pub struct Report {
    /// What was being looked for, for the refusal: `AWS credentials`.
    what: &'static str,
    failed: Vec<String>,
    absent: Vec<String>,
}

impl Report {
    /// An empty report of a walk for `what`.
    pub const fn new(what: &'static str) -> Self {
        Self {
            what,
            failed: Vec::new(),
            absent: Vec::new(),
        }
    }

    /// `source` was configured and could not answer.
    pub fn failed(&mut self, source: &str, error: impl std::fmt::Display) {
        self.failed.push(format!("{source}: {error}"));
    }

    /// `source` was not configured.
    pub fn absent(&mut self, source: &str) {
        self.absent.push(source.to_owned());
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
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            message,
        )))
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/auth/report.rs` pins and a caller cannot reach.
    pub use super::Report;
}
