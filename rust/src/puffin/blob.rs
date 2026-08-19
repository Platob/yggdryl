//! Blob metadata: the footer's description of one blob, and the known types.
//!
//! A [`BlobMetadata`] is one entry of the footer payload's `blobs` list, with
//! exactly the fields the Puffin spec names: `type`, `fields`, `snapshot-id`,
//! `sequence-number`, `offset`, `length`, and the optional
//! `compression-codec` and `properties`. The known blob types are named here
//! as constants, and the `deletion-vector-v1` rules - required properties, no
//! compression, sentinel snapshot identity - are one validation a reader and
//! a writer share.

use smol_str::{SmolStr, format_smolstr};

use crate::{Result, Value};

use super::bitmap::invalid;

/// The Theta sketch blob type: read and carried, never produced here.
pub const APACHE_DATASKETCHES_THETA_V1: &str = "apache-datasketches-theta-v1";

/// The deletion vector blob type Iceberg v3 row-level deletes use.
pub const DELETION_VECTOR_V1: &str = "deletion-vector-v1";

/// The deletion-vector property naming the data file the deletes apply to.
pub const REFERENCED_DATA_FILE_PROPERTY: &str = "referenced-data-file";

/// The deletion-vector property carrying the number of deleted positions.
pub const CARDINALITY_PROPERTY: &str = "cardinality";

/// The sentinel `snapshot-id` and `sequence-number` a deletion vector carries,
/// because neither is known while the Puffin file is being written.
pub const UNASSIGNED_SNAPSHOT: i64 = -1;

/// One blob's entry in the Puffin footer payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobMetadata {
    blob_type: SmolStr,
    fields: Vec<i32>,
    snapshot_id: i64,
    sequence_number: i64,
    offset: u64,
    length: u64,
    compression_codec: Option<SmolStr>,
    properties: Vec<(SmolStr, SmolStr)>,
}

impl BlobMetadata {
    /// Describe a blob before it is appended.
    ///
    /// `offset` and `length` start at zero; the append that stores the blob's
    /// bytes stamps the real values, so whatever a caller sets is replaced.
    pub fn new(
        blob_type: impl Into<SmolStr>,
        fields: Vec<i32>,
        snapshot_id: i64,
        sequence_number: i64,
    ) -> Self {
        Self {
            blob_type: blob_type.into(),
            fields,
            snapshot_id,
            sequence_number,
            offset: 0,
            length: 0,
            compression_codec: None,
            properties: Vec::new(),
        }
    }

    /// Describe a deletion vector for one data file.
    ///
    /// The snapshot id and sequence number are the `-1` sentinels the spec
    /// requires, and the two required properties are filled from the
    /// arguments; the field list is empty because a deletion vector indexes
    /// row positions, not columns.
    pub fn deletion_vector(referenced_data_file: impl Into<SmolStr>, cardinality: u64) -> Self {
        Self::new(
            DELETION_VECTOR_V1,
            Vec::new(),
            UNASSIGNED_SNAPSHOT,
            UNASSIGNED_SNAPSHOT,
        )
        .with_property(REFERENCED_DATA_FILE_PROPERTY, referenced_data_file.into())
        .with_property(CARDINALITY_PROPERTY, format_smolstr!("{cardinality}"))
    }

    /// The blob's type name.
    pub fn blob_type(&self) -> &str {
        &self.blob_type
    }

    /// The field IDs the blob was computed for, in computation order.
    pub fn fields(&self) -> &[i32] {
        &self.fields
    }

    /// The snapshot the blob was computed from.
    pub const fn snapshot_id(&self) -> i64 {
        self.snapshot_id
    }

    /// The sequence number of that snapshot.
    pub const fn sequence_number(&self) -> i64 {
        self.sequence_number
    }

    /// Where the blob's bytes start in the file.
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// The blob's stored length, after compression if compressed.
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// The compression codec, or `None` for uncompressed bytes.
    pub fn compression_codec(&self) -> Option<&str> {
        self.compression_codec.as_deref()
    }

    /// The blob's property entries, in stored order.
    pub fn properties(&self) -> &[(SmolStr, SmolStr)] {
        &self.properties
    }

    /// Return one property value by key.
    pub fn get_property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }

    /// Return this metadata with a compression codec named.
    #[must_use]
    pub fn with_compression_codec(mut self, codec: impl Into<SmolStr>) -> Self {
        self.compression_codec = Some(codec.into());
        self
    }

    /// Return this metadata with one property set, replacing a prior value.
    #[must_use]
    pub fn with_property(mut self, key: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        let key = key.into();
        let value = value.into();
        if let Some(entry) = self.properties.iter_mut().find(|(name, _)| *name == key) {
            entry.1 = value;
        } else {
            self.properties.push((key, value));
        }
        self
    }

    /// Stamp the location an append decided.
    pub(crate) const fn set_location(&mut self, offset: u64, length: u64) {
        self.offset = offset;
        self.length = length;
    }

    /// Read one `BlobMetadata` object from the footer payload.
    ///
    /// # Errors
    ///
    /// Returns an error when a required field is missing or not the type the
    /// Puffin spec declares, or when `offset` or `length` is negative.
    pub fn from_json(document: &Value) -> Result<Self> {
        let blob_type = document
            .get_key_str("type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(SmolStr::new_static("expected a blob \"type\" string")))?;
        let fields_json = document
            .get_key_str("fields")
            .and_then(Value::as_sequence)
            .ok_or_else(|| invalid(SmolStr::new_static("expected a blob \"fields\" list")))?;
        let mut fields = Vec::with_capacity(fields_json.len());
        for field in fields_json {
            let wide = field
                .as_i64()
                .ok_or_else(|| invalid(SmolStr::new_static("expected an integer blob field ID")))?;
            let id = i32::try_from(wide).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a 32-bit blob field ID, got {wide}"
                ))
            })?;
            fields.push(id);
        }
        let long = |key: &'static str| {
            document
                .get_key_str(key)
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid(format_smolstr!("expected a blob \"{key}\" integer")))
        };
        let snapshot_id = long("snapshot-id")?;
        let sequence_number = long("sequence-number")?;
        let unsigned = |key: &'static str| {
            let value = long(key)?;
            u64::try_from(value).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative blob \"{key}\", got {value}"
                ))
            })
        };
        let offset = unsigned("offset")?;
        let length = unsigned("length")?;
        let compression_codec = match document.get_key_str("compression-codec") {
            None => None,
            Some(codec) => Some(SmolStr::new(codec.as_str().ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a blob \"compression-codec\" string",
                ))
            })?)),
        };
        let properties = match document.get_key_str("properties") {
            None => Vec::new(),
            Some(properties) => super::format::string_entries(properties, "blob \"properties\"")?,
        };
        Ok(Self {
            blob_type: SmolStr::new(blob_type),
            fields,
            snapshot_id,
            sequence_number,
            offset,
            length,
            compression_codec,
            properties,
        })
    }

    /// Write this metadata as the object the footer payload holds.
    ///
    /// # Errors
    ///
    /// Returns an error when `offset` or `length` exceeds what a JSON long
    /// can carry, or when the mapping cannot be built.
    pub fn to_json(&self) -> Result<Value> {
        let long = |name: &'static str, value: u64| {
            i64::try_from(value).map(Value::from).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a blob \"{name}\" within a signed 64-bit integer, got {value}"
                ))
            })
        };
        let mut entries = vec![
            (Value::from("type"), Value::from(self.blob_type.clone())),
            (
                Value::from("fields"),
                Value::from_sequence(self.fields.iter().map(|id| Value::from(i64::from(*id)))),
            ),
            (Value::from("snapshot-id"), Value::from(self.snapshot_id)),
            (
                Value::from("sequence-number"),
                Value::from(self.sequence_number),
            ),
            (Value::from("offset"), long("offset", self.offset)?),
            (Value::from("length"), long("length", self.length)?),
        ];
        if let Some(codec) = &self.compression_codec {
            entries.push((Value::from("compression-codec"), Value::from(codec.clone())));
        }
        if !self.properties.is_empty() {
            entries.push((
                Value::from("properties"),
                Value::from_mapping(
                    self.properties
                        .iter()
                        .map(|(key, value)| (Value::from(key.clone()), Value::from(value.clone()))),
                )?,
            ));
        }
        Value::from_mapping(entries)
    }

    /// Validate the `deletion-vector-v1` metadata rules the spec states.
    ///
    /// Each rule is checked and reported by name: the blob type, the two
    /// required properties, the ban on compression, and the `-1` sentinels for
    /// `snapshot-id` and `sequence-number`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first broken rule and the value breaking it.
    pub fn validate_deletion_vector(&self) -> Result<()> {
        if self.blob_type != DELETION_VECTOR_V1 {
            return Err(invalid(format_smolstr!(
                "expected a {DELETION_VECTOR_V1:?} blob, got {:?}",
                self.blob_type
            )));
        }
        if let Some(codec) = &self.compression_codec {
            return Err(invalid(format_smolstr!(
                "expected an uncompressed {DELETION_VECTOR_V1} blob, got compression-codec {codec:?}"
            )));
        }
        if self.snapshot_id != UNASSIGNED_SNAPSHOT {
            return Err(invalid(format_smolstr!(
                "expected snapshot-id {UNASSIGNED_SNAPSHOT} on a {DELETION_VECTOR_V1} blob, got {}",
                self.snapshot_id
            )));
        }
        if self.sequence_number != UNASSIGNED_SNAPSHOT {
            return Err(invalid(format_smolstr!(
                "expected sequence-number {UNASSIGNED_SNAPSHOT} on a {DELETION_VECTOR_V1} blob, got {}",
                self.sequence_number
            )));
        }
        for required in [REFERENCED_DATA_FILE_PROPERTY, CARDINALITY_PROPERTY] {
            if self.get_property(required).is_none() {
                return Err(invalid(format_smolstr!(
                    "expected a {required:?} property on a {DELETION_VECTOR_V1} blob, got none"
                )));
            }
        }
        Ok(())
    }

    /// Read the required `cardinality` property as the count it declares.
    ///
    /// # Errors
    ///
    /// Returns an error when the property is absent or not a non-negative
    /// decimal integer.
    pub fn cardinality(&self) -> Result<u64> {
        let text = self.get_property(CARDINALITY_PROPERTY).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a {CARDINALITY_PROPERTY:?} property on a {DELETION_VECTOR_V1} blob, got none"
            ))
        })?;
        text.parse().map_err(|_| {
            invalid(format_smolstr!(
                "expected a non-negative integer {CARDINALITY_PROPERTY}, got {text:?}"
            ))
        })
    }
}
