//! The [`Params`] query-parameter map. Component maps used by each type's
//! `from_mapping` parser are a plain `std::collections::BTreeMap<String, String>`.

use std::collections::BTreeMap;

/// A multi-valued query-parameter map: `key` → list of values, mirroring how a
/// query string may repeat a key (`?a=1&a=2`). Used by [`Uri::params`](crate::Uri::params) /
/// [`Url::params`](crate::Url::params) and friends.
pub type Params = BTreeMap<String, Vec<String>>;
