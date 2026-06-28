//! The [`Field`] — a named [`DataType`] with optional byte-keyed metadata, plus the
//! reserved `comment` / `index_name` / `index_level` metadata accessors.

use std::collections::BTreeMap;

use crate::DataType;

/// Byte metadata key/value map (kept ordered + hashable).
pub type Metadata = BTreeMap<Vec<u8>, Vec<u8>>;

/// The reserved metadata key holding a field's comment.
const COMMENT_KEY: &[u8] = b"comment";
/// The reserved metadata key holding a field's index name.
const INDEX_NAME_KEY: &[u8] = b"index_name";
/// The reserved metadata key holding a field's index level (a `u16`).
const INDEX_LEVEL_KEY: &[u8] = b"index_level";

/// A named, typed schema node: a [`name`](Field::name), a [`dtype`](Field::dtype) and
/// optional byte-keyed [`metadata`](Field::metadata). A few well-known metadata keys
/// have typed accessors — [`comment`](Field::comment), [`index_name`](Field::index_name)
/// and [`index_level`](Field::index_level) — whose setters mutate the metadata map in
/// place.
///
/// ```
/// use yggdryl_schema::{DataType, Field};
/// let mut f = Field::new("id", DataType::int64());
/// f.set_comment(Some("primary key"));
/// f.set_index_level(Some(0));
/// assert_eq!(f.comment().as_deref(), Some("primary key"));
/// assert_eq!(f.index_level(), Some(0));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Field {
    /// The field name.
    pub name: String,
    /// The field's [`DataType`].
    pub dtype: DataType,
    /// Optional byte-keyed metadata.
    pub metadata: Option<Metadata>,
}

impl Field {
    /// A field with the given `name` and `dtype` and no metadata.
    pub fn new(name: impl Into<String>, dtype: DataType) -> Field {
        Field {
            name: name.into(),
            dtype,
            metadata: None,
        }
    }

    // ---- raw metadata ----

    /// The raw metadata value for `key`, if present.
    pub fn get_metadata(&self, key: &[u8]) -> Option<&[u8]> {
        self.metadata.as_ref()?.get(key).map(Vec::as_slice)
    }

    /// Sets a raw metadata `key` to `value`, creating the map if needed (in place).
    pub fn set_metadata(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.metadata
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
    }

    /// Removes a raw metadata `key`, returning its value; clears the map to `None`
    /// when it becomes empty.
    pub fn remove_metadata(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let map = self.metadata.as_mut()?;
        let removed = map.remove(key);
        if map.is_empty() {
            self.metadata = None;
        }
        removed
    }

    /// Sets `key` to a UTF-8 string, or removes it when `value` is `None`.
    fn set_str(&mut self, key: &[u8], value: Option<&str>) {
        match value {
            Some(value) => self.set_metadata(key.to_vec(), value.as_bytes().to_vec()),
            None => {
                self.remove_metadata(key);
            }
        }
    }

    /// Reads `key` as a UTF-8 string, if present and valid.
    fn get_str(&self, key: &[u8]) -> Option<String> {
        std::str::from_utf8(self.get_metadata(key)?)
            .ok()
            .map(str::to_string)
    }

    // ---- reserved typed metadata ----

    /// The field's comment (`comment` metadata), if any.
    pub fn comment(&self) -> Option<String> {
        self.get_str(COMMENT_KEY)
    }

    /// Sets (or clears, with `None`) the field's comment, mutating the metadata.
    pub fn set_comment(&mut self, value: Option<&str>) {
        self.set_str(COMMENT_KEY, value);
    }

    /// The field's index name (`index_name` metadata), if any.
    pub fn index_name(&self) -> Option<String> {
        self.get_str(INDEX_NAME_KEY)
    }

    /// Sets (or clears, with `None`) the field's index name, mutating the metadata.
    pub fn set_index_name(&mut self, value: Option<&str>) {
        self.set_str(INDEX_NAME_KEY, value);
    }

    /// The field's index level (`index_level` metadata) as a `u16`, if present and valid.
    pub fn index_level(&self) -> Option<u16> {
        self.get_str(INDEX_LEVEL_KEY)?.parse().ok()
    }

    /// Sets (or clears, with `None`) the field's index level, mutating the metadata.
    pub fn set_index_level(&mut self, value: Option<u16>) {
        match value {
            Some(level) => {
                self.set_metadata(INDEX_LEVEL_KEY.to_vec(), level.to_string().into_bytes())
            }
            None => {
                self.remove_metadata(INDEX_LEVEL_KEY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_dtype_and_raw_metadata() {
        let mut f = Field::new("id", DataType::int64());
        assert_eq!(f.name, "id");
        assert_eq!(f.dtype, DataType::int64());
        assert!(f.metadata.is_none());

        f.set_metadata(b"unit".to_vec(), b"count".to_vec());
        assert_eq!(f.get_metadata(b"unit"), Some(b"count".as_slice()));
        assert_eq!(f.remove_metadata(b"unit"), Some(b"count".to_vec()));
        assert!(f.metadata.is_none()); // emptied -> None
    }

    #[test]
    fn reserved_typed_accessors_mutate_in_place() {
        let mut f = Field::new("x", DataType::int32());
        assert_eq!(f.comment(), None);
        assert_eq!(f.index_name(), None);
        assert_eq!(f.index_level(), None);

        f.set_comment(Some("a note"));
        f.set_index_name(Some("idx"));
        f.set_index_level(Some(7));
        assert_eq!(f.comment().as_deref(), Some("a note"));
        assert_eq!(f.index_name().as_deref(), Some("idx"));
        assert_eq!(f.index_level(), Some(7));
        // stored under the reserved byte keys.
        assert_eq!(f.get_metadata(b"comment"), Some(b"a note".as_slice()));
        assert_eq!(f.get_metadata(b"index_level"), Some(b"7".as_slice()));

        // clearing removes the key.
        f.set_comment(None);
        f.set_index_level(None);
        assert_eq!(f.comment(), None);
        assert_eq!(f.index_level(), None);
        assert_eq!(f.index_name().as_deref(), Some("idx")); // others untouched
    }
}
