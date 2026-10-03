//! The name of the machine this process runs on, read once.

use std::sync::LazyLock;

use smol_str::{SmolStr, SmolStrBuilder};

/// The host this process runs on, spelled as a URL host: the name the
/// operating system reports, read once on first use and kept for the life of
/// the process.
///
/// It is the one spelling of this machine intake reads beside `localhost`
/// ([`Authority::is_this_machine`](crate::Authority::is_this_machine)): on
/// Unix, `file://<HOSTNAME>/x` is the local path `/x`, as `file://localhost/x`
/// is. No URL the crate makes up names it: a
/// [`Buffer`](crate::holder::Buffer) and a location bound to an in-process
/// [filesystem](crate::fs::FileSystem) name `localhost`; a local file names no
/// host at all, `file:///x`, because an empty authority is this machine by
/// RFC 8089 on every platform, where Windows reads a named one as a share; and
/// a remote store's host is the one its location, its environment or its
/// published endpoint names. The system's name is lower-cased, and a byte no
/// host spells is written as `-`; a system that answers nothing a host can
/// spell is `localhost`.
///
/// ```
/// use yggdryl::{Url, HOSTNAME};
///
/// assert!(!HOSTNAME.is_empty());
/// let url = Url::from_str(&format!("file://{}/lake/trades.parquet", HOSTNAME.as_str()))?;
/// assert_eq!(url.hostname(), Some(HOSTNAME.as_str()));
/// // Read as this machine, the way `localhost` is: the local path itself.
/// #[cfg(unix)]
/// assert_eq!(url.into_path()?, std::path::PathBuf::from("/lake/trades.parquet"));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub static HOSTNAME: LazyLock<SmolStr> =
    LazyLock::new(|| host(&gethostname::gethostname().to_string_lossy()));

/// `raw` as a URL host: lower-cased, each byte outside `[a-z0-9._-]` as `-`,
/// trimmed of the dots and hyphens a label cannot open or close with, capped
/// at the 253 bytes a DNS name holds; `localhost` when nothing is left.
fn host(raw: &str) -> SmolStr {
    const MAX: usize = 253;
    let mut spelled = SmolStrBuilder::new();
    for byte in raw.trim().bytes().take(MAX) {
        let byte = byte.to_ascii_lowercase();
        spelled.push(
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
                char::from(byte)
            } else {
                '-'
            },
        );
    }
    let spelled = spelled.finish();
    let trimmed = spelled.trim_matches(['.', '-']);
    match trimmed {
        "" => SmolStr::new_static("localhost"),
        _ if trimmed.len() == spelled.len() => spelled,
        _ => SmolStr::new(trimmed),
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/hostname.rs` pins and a caller cannot reach.
    use smol_str::SmolStr;

    /// `raw` read as [`HOSTNAME`](super::HOSTNAME) reads the system's name.
    pub fn host(raw: &str) -> SmolStr {
        super::host(raw)
    }
}
