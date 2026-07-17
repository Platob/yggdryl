//! `io::uri` — the **URI / URL family**.
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
