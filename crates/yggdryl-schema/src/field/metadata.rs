//! Field [`Metadata`].

use std::collections::BTreeMap;

/// Byte-keyed, byte-valued metadata attached to a [`Field`](crate::Field). A
/// `BTreeMap` keeps a deterministic order, so equal fields hash equally.
pub type Metadata = BTreeMap<Vec<u8>, Vec<u8>>;
