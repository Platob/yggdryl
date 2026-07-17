//! `uri` — the **URI / URL family**.
//!
//! yggdryl's addressing layer, parsed from scratch (no `url` crate): a generic RFC 3986
//! [`Uri`] that doubles as a POSIX-normalized filesystem path, a [`Url`] (a URI guaranteed to
//! carry a scheme), and the [`Authority`] component (`[user[:password]@]host[:port]`), with
//! [`UriError`] carrying the guided parse failures and [`default_port`] mapping a well-known
//! scheme to its port. Percent-encoding lives in the internal `percent` module.
//!
//! Every type here is a value type — equal, hashable, and byte-serializable identically across
//! the Rust core and the Python / Node extensions.

mod authority;
mod error;
mod generic;
mod percent;
mod scheme;
mod url;

pub use authority::Authority;
pub use error::UriError;
pub use generic::Uri;
pub use scheme::default_port;
pub use url::Url;

/// A zero-allocation [`core::fmt::Write`] that streams formatted output straight into a
/// [`Hasher`](core::hash::Hasher).
///
/// The URI value types hash by their canonical string; this lets them feed that string to
/// the hasher a fragment at a time instead of building a `String` first — the same bytes,
/// no allocation. Paired with a `0xff` terminator it reproduces `str`'s own hash, so equal
/// canonical strings still hash equal.
pub(crate) struct HashWrite<'a, H: core::hash::Hasher>(pub(crate) &'a mut H);

impl<H: core::hash::Hasher> core::fmt::Write for HashWrite<'_, H> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}
