//! The process environment, or a stand-in for it.
//!
//! A provider reads its variables through one value so a caller can hand
//! over an environment of their own - a captured one, a subprocess's, a
//! test's - and so every reading trims and treats a blank variable as unset
//! the same way. That is how botocore reads an endpoint variable; a blank
//! *flag* it reads as set and false, where here it is unset and the profile
//! is read next. A variable whose being set is itself the statement - a
//! shared file's path, where `AWS_CONFIG_FILE=` means no file - is read
//! through [`Environment::raw`], which keeps a blank value as the empty text.

#[cfg(feature = "aws")]
use std::collections::BTreeMap;

#[cfg(feature = "aws")]
use crate::boolean::bool_from_text;

/// A non-empty value, trimmed; an empty one is unset.
fn present(value: String) -> Option<String> {
    Some(trimmed(value)).filter(|value| !value.is_empty())
}

/// The value with its surrounding whitespace taken off, reusing its buffer
/// when there is none.
fn trimmed(value: String) -> String {
    if value.trim().len() == value.len() {
        value
    } else {
        value.trim().to_owned()
    }
}

/// Where variables are read from.
#[cfg(feature = "aws")]
#[derive(Clone, Debug)]
pub enum Environment {
    /// The process's own environment.
    Process,
    /// The pairs a caller supplied, and nothing else.
    Given(BTreeMap<String, String>),
}

#[cfg(feature = "aws")]
impl Environment {
    /// A non-empty variable, trimmed: `None` for one that is unset, empty
    /// or blank.
    pub fn get(&self, name: &str) -> Option<String> {
        self.raw(name).filter(|value| !value.is_empty())
    }

    /// A variable as set, trimmed: `Some("")` for one set to the empty or a
    /// blank text, `None` only for one not set at all - or, in the process
    /// environment, set to text that is not Unicode, which [`Self::get`]
    /// reads as unset too.
    ///
    /// What botocore's `EnvironmentProvider` answers, which filters nothing:
    /// a path variable set empty names a file that is not there, so
    /// `AWS_CONFIG_FILE=` read through it is no configuration file, where
    /// read through `get` it is unset and `~/.aws/config` is read instead.
    pub fn raw(&self, name: &str) -> Option<String> {
        let value = match self {
            Self::Process => std::env::var(name).ok()?,
            Self::Given(pairs) => pairs.get(name)?.clone(),
        };
        Some(trimmed(value))
    }

    /// A boolean variable, read through `bool_from_text`: the one table
    /// every flag in the crate reads, `true`, `yes`, `y`, `on`, `1` and their
    /// opposites in any case.
    ///
    /// `None` is a variable that is unset or blank. Text the table does not
    /// spell reads false, as the cloud tools read it: these are toggles read
    /// where nothing can refuse, and one nobody can read is not one that was
    /// set. The table is wider than botocore's, which reads only `true` as
    /// true: `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=1` ignores the configured
    /// endpoints here and not there.
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.get(name)
            .map(|value| bool_from_text(&value).unwrap_or(false))
    }
}

/// A non-empty process environment variable, trimmed: what the HTTP client
/// reads its proxy and certificate names by, and the Google and Azure
/// dialects their own.
pub fn variable(name: &str) -> Option<String> {
    present(std::env::var(name).ok()?)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/auth/environment.rs` pins and a caller cannot reach.
    #[cfg(feature = "aws")]
    pub use super::Environment;
}
