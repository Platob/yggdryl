//! Type and field metadata, and the strategy for round-tripping yggdryl types
//! that Apache Arrow cannot represent exactly.
//!
//! Arrow's type system is narrower than ours, so [`DataType::to_arrow_type`] is
//! sometimes lossy (a `MaxedSizeBinaryType` becomes a plain `Binary`). The lost
//! information is stored under [reserved](reserved_key) metadata keys, so the
//! exact type can be rebuilt from the Arrow type **plus** the metadata via
//! [`DataType::from_arrow_type`].
//!
//! [`DataType::to_arrow_type`]: crate::DataType::to_arrow_type
//! [`DataType::from_arrow_type`]: crate::DataType::from_arrow_type

use std::collections::BTreeMap;

/// Column and type metadata: an ordered map of opaque byte-string key/value pairs.
///
/// `BTreeMap` keeps the entries ordered so the map hashes and serializes
/// deterministically.
pub type Metadata = BTreeMap<Vec<u8>, Vec<u8>>;

/// Prefix marking yggdryl's own reserved metadata keys, kept distinct from user
/// metadata. Entries under this prefix carry the information Apache Arrow drops, so
/// the exact yggdryl type can be rebuilt from the Arrow type plus the metadata.
pub const RESERVED_PREFIX: &[u8] = b"yggdryl:";

/// The reserved key under which every data type records its canonical name.
pub const TYPE_KEY: &str = "type";

/// Builds the reserved metadata key for `name` (e.g. `yggdryl:byte_size`).
pub fn reserved_key(name: &str) -> Vec<u8> {
    [RESERVED_PREFIX, name.as_bytes()].concat()
}

/// Whether `key` is one of yggdryl's reserved metadata keys.
pub fn is_reserved(key: &[u8]) -> bool {
    key.starts_with(RESERVED_PREFIX)
}

/// The base metadata recording a type's identity — its canonical `name` under the
/// reserved [`TYPE_KEY`]. Every [`DataType`](crate::DataType) contributes this (and
/// may add parameters) so the exact yggdryl type is recoverable from an Arrow
/// field's metadata.
pub fn type_metadata(name: &str) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert(reserved_key(TYPE_KEY), name.as_bytes().to_vec());
    metadata
}
