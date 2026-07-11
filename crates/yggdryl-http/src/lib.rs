//! **yggdryl-http** — generic HTTP-style header maps.
//!
//! [`Headers`] is an ordered **bytes → bytes** map (an HTTP header block) with byte and
//! UTF-8 string accessors/mutators, zero-copy in-place value mutation
//! ([`get_mut`](Headers::get_mut)), a deterministic byte round-trip codec, and pre-built
//! accessors for the common keys (`name`, `comment`, `content-type`,
//! `content-encoding`). [`HeadersBased`] is the trait a header-carrying type (a field, a
//! buffer) implements — it supplies only the storage slot and gets the whole
//! get / add / update / delete surface, the builder, and the common-key conveniences for
//! free.
//!
//! Dependency-free; the field and buffer layers depend on it.

mod headers;
mod headers_based;
mod headers_error;

pub use headers::Headers;
pub use headers_based::HeadersBased;
pub use headers_error::HeadersError;
