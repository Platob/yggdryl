//! The process environment, or a stand-in for it.
//!
//! A provider reads its variables through one value so a caller can hand
//! over an environment of their own - a captured one, a subprocess's, a
//! test's - and so every reading trims and treats an empty variable as unset
//! the same way, which is what every cloud's own tools do.

#[cfg(feature = "aws")]
use std::collections::BTreeMap;

/// A non-empty value, trimmed; an empty one is unset.
fn present(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
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
    /// A non-empty variable, trimmed.
    pub fn get(&self, name: &str) -> Option<String> {
        let value = match self {
            Self::Process => std::env::var(name).ok()?,
            Self::Given(pairs) => pairs.get(name)?.clone(),
        };
        present(value)
    }

    /// A boolean variable, in the spellings the cloud tools accept.
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.get(name).map(|value| is_true(&value))
    }
}

/// Whether `value` spells true the way the cloud tools read one.
#[cfg(feature = "aws")]
pub fn is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
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

    /// Whether `value` spells true the way the cloud tools read one.
    #[cfg(feature = "aws")]
    pub fn is_true(value: &str) -> bool {
        super::is_true(value)
    }
}
