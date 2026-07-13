//! `io` — the Apache Arrow-backed physical I/O layer of the core.
//!
//! This module owns yggdryl's *addressing and physical* representation. Its first concern
//! is the URI family: [`Uri`] (a generic RFC 3986 URI that doubles as a POSIX-normalized
//! filesystem path), [`Url`] (a URI guaranteed to carry a scheme), and [`Authority`] (the
//! `[user[:password]@]host[:port]` component), with [`UriError`] carrying the guided parse
//! failures.
//!
//! Per the crate rules, Arrow (`arrow-buffer`) is the physical layer here — its types stay
//! an implementation detail and never appear in a public signature; each public type lives
//! in its own file and is mirrored, thinly, in the Python and Node extensions.

mod authority;
mod percent;
mod uri;
mod uri_error;
mod url;

pub use authority::Authority;
pub use uri::Uri;
pub use uri_error::UriError;
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
