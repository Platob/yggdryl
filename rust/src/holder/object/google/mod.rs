//! Google Cloud Storage.
//!
//! Google is the store of the three that answers JSON rather than XML and
//! authorizes with a bearer token rather than with a signature over the
//! request, so this dialect owns two things the others do not: where a token
//! comes from, and how the JSON API spells a listing, an object, and a refusal.
//! Everything above it is the backend's.

pub(crate) mod dialect;
pub(crate) mod json;
pub(crate) mod options;
pub(crate) mod token;

pub use options::{DEFAULT_SCOPE, GoogleOptions};
