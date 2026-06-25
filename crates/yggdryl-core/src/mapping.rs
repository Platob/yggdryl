//! The [`Mapping`] / [`Params`] component maps shared by every type's
//! `from_mapping` parser and query accessors.

use std::collections::BTreeMap;

/// A set of named components, used by each type's `from_mapping` parser.
///
/// Keys are component names (`"scheme"`, `"host"`, `"major"`, …) and values are
/// their string form. Which keys each type understands is documented on its
/// inherent `from_mapping` method.
pub type Mapping = BTreeMap<String, String>;

/// A multi-valued query-parameter map: `key` → list of values, mirroring how a
/// query string may repeat a key (`?a=1&a=2`). Used by [`Uri::params`](crate::Uri::params) /
/// [`Url::params`](crate::Url::params) and friends.
pub type Params = BTreeMap<String, Vec<String>>;
