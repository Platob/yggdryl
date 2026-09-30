//! The name of the machine this process runs on, read once.

use std::sync::LazyLock;

use smol_str::{SmolStr, SmolStrBuilder};

/// The host this process runs on, spelled as a URL host: the name the
/// operating system reports, read once on first use and kept for the life of
/// the process.
///
/// Every identity the crate makes up for storage that lives in this process -
/// a [`Buffer`](crate::holder::Buffer), a location bound to an in-process
/// [filesystem](crate::fs::FileSystem) - names this host, so one machine's
/// identity is never another's. The system's name is lower-cased, and a byte
/// no host spells is written as `-`; a system that answers nothing a host can
/// spell is `localhost`, the one name every resolver reads as this machine.
///
/// A `file:` URL names no host - an empty authority is this machine by
/// RFC 8089, and a named one is a share on another - and a remote store's
/// host is the one its location, its environment or its published endpoint
/// names, never this one.
///
/// ```
/// use yggdryl::{Url, HOSTNAME};
///
/// assert!(!HOSTNAME.is_empty());
/// let url = Url::from_str(&format!("memory://{}/trades.parquet", HOSTNAME.as_str()))?;
/// assert_eq!(url.hostname(), Some(HOSTNAME.as_str()));
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

/// The `mem:` identity of bytes held at `address` by process `pid` on this
/// machine: `mem://<host>/<pid>/<address>`, what a buffer answers for a
/// location it does not have.
///
/// Built from its parts rather than parsed from text, the host shared with
/// [`HOSTNAME`] rather than copied, so what an identity costs is the same on
/// every machine whatever its name's length.
pub(crate) fn memory_identity<T>(pid: u32, address: *const T) -> crate::Url {
    let path = smol_str::format_smolstr!("/{pid}/{address:p}");
    crate::UriPath::from_str(&path)
        .and_then(|path| {
            crate::Uri::from_parts(
                crate::Scheme::from_str("mem")?,
                crate::Authority::this_machine(),
                path,
                None,
                None,
            )
        })
        .and_then(crate::Url::from_uri)
        // The host is spelled to parse and the path is digits, so this holds;
        // a diagnostic accessor still never panics.
        .unwrap_or_else(|_| unreachable!("a digit path under this machine's host is a URL"))
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
