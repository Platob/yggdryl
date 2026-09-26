//! A value that is never rendered, and the file only its owner reads.
//!
//! A secret key, a session token, a bearer token, a client secret: each is
//! text the process must hold and must never write - not in `Debug` output,
//! not in an error, not in a log line. Holding one in this type makes that
//! the type's job rather than every holder's, so a struct that carries one
//! can derive `Debug` and print `<redacted>` where the value would be. What
//! a secret is written to on purpose - a cache the tools share - is written
//! by [`write_private`], as a file only its owner can read.

#[cfg(feature = "aws")]
use std::path::Path;

/// Text that renders as `<redacted>`.
///
/// Equality and hashing read the text, so a set of keys is one set whichever
/// holder carries it; [`Self::expose`] is the one door to the text, and it is
/// named so a reader sees where a secret leaves the type.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    /// Hold `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text, for the one place that puts it on the wire.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl From<String> for Secret {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&str> for Secret {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

#[cfg(feature = "aws")]
/// Write `bytes` to `path` as a file only its owner can read, creating the
/// directories above it the same way, which is what the AWS tools do with
/// every cache they keep a secret in.
///
/// # Errors
///
/// The file system's refusal to create the directories or the file.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // A file that already existed keeps the mode it had; state it again.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/auth/secret.rs` pins and a caller cannot reach.
    pub use super::Secret;
    #[cfg(feature = "aws")]
    pub use super::write_private;
}
