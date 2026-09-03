//! Iceberg's own vocabulary, on the field views that carry it.
//!
//! An Iceberg schema states more about a column than a [`Field`](crate::Field)
//! has structural slots for - the schema identifier, a doc string, the v3
//! defaults, a declared type the physical one cannot distinguish - and a
//! partition tuple states how each of its values was derived. All of it rides
//! as `iceberg:` properties, so these two impls are the one place that
//! vocabulary is spelled, parsed and rendered.
//!
//! The impls live here rather than beside the other protocol views because the
//! property constants belong to the documents they are read from and written
//! to, in `schema.rs` and `partition.rs`, and because the whole vocabulary is
//! gated with the rest of the feature.
//!
//! ```
//! use yggdryl::DataType;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut field = DataType::Int64.required_field("id");
//! field.as_iceberg_mut().set_doc("row identifier")?;
//! field.as_iceberg_mut().set_partition_source_id(3)?;
//!
//! assert_eq!(field.as_iceberg().doc(), Some("row identifier"));
//! assert_eq!(field.as_iceberg().partition_source_id()?, Some(3));
//! assert_eq!(field.get_metadata("iceberg:partition-source-id"), Some("3"));
//! # Ok(())
//! # }
//! ```

use smol_str::{SmolStr, format_smolstr};

use super::Transform;
use super::partition::{SOURCE_ID, SPEC_ID, TRANSFORM};
use super::schema::{DECLARED_TYPE, DOC, IDENTIFIER, INITIAL_DEFAULT, SCHEMA_ID, WRITE_DEFAULT};
use crate::{Error, IcebergField, IcebergFieldMut, Result, Scalar};

impl<'field> IcebergField<'field> {
    /// Parses the identifier of the schema this root is.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text is not a signed 32-bit decimal
    /// integer, which is the width the Iceberg spec types an identifier as.
    /// Every identifier read here fails the same way.
    pub fn schema_id(&self) -> Result<Option<i32>> {
        self.identifier(SCHEMA_ID)
    }

    /// Parses the identifier columns of a schema root.
    ///
    /// An absent property is an empty list: a schema states identifier columns
    /// only when it has them.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text is not a comma-separated list of
    /// identifiers.
    pub fn identifier_field_ids(&self) -> Result<Vec<i32>> {
        let Some(stored) = self.get(IDENTIFIER) else {
            return Ok(Vec::new());
        };
        stored
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| {
                id.parse().map_err(|_| {
                    self.invalid(
                        IDENTIFIER,
                        "a comma-separated list of signed 32-bit decimal integers",
                        stored,
                    )
                })
            })
            .collect()
    }

    /// Returns this column's Iceberg documentation string.
    pub fn doc(&self) -> Option<&'field str> {
        self.get(DOC)
    }

    /// Decodes the v3 `initial-default` this column carries.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text is not one JSON value. Both
    /// default reads fail the same way.
    pub fn initial_default(&self) -> Result<Option<Scalar>> {
        self.decoded(INITIAL_DEFAULT)
    }

    /// Decodes the v3 `write-default` this column carries.
    ///
    /// # Errors
    ///
    /// [`Self::initial_default`] carries the rule.
    pub fn write_default(&self) -> Result<Option<Scalar>> {
        self.decoded(WRITE_DEFAULT)
    }

    /// Returns the Iceberg type spelling the physical datatype cannot keep.
    ///
    /// `uuid` and `fixed[16]` are one physical type, so a schema that said
    /// `uuid` says it again on the next commit instead of quietly demoting the
    /// column.
    pub fn declared_type(&self) -> Option<&'field str> {
        self.get(DECLARED_TYPE)
    }

    /// Parses the identifier of the spec a partition tuple belongs to.
    ///
    /// # Errors
    ///
    /// [`Self::schema_id`] carries the rule.
    pub fn spec_id(&self) -> Result<Option<i32>> {
        self.identifier(SPEC_ID)
    }

    /// Parses the schema column a partition field reads.
    ///
    /// # Errors
    ///
    /// [`Self::schema_id`] carries the rule.
    pub fn partition_source_id(&self) -> Result<Option<i32>> {
        self.identifier(SOURCE_ID)
    }

    /// Parses how this partition field derives its value.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text names no Iceberg transform.
    pub fn transform(&self) -> Result<Option<Transform>> {
        self.get(TRANSFORM).map(Transform::from_str).transpose()
    }

    /// Parse one property as the identifier Iceberg types it as.
    fn identifier(&self, name: &str) -> Result<Option<i32>> {
        self.get(name)
            .map(|stored| {
                stored
                    .parse()
                    .map_err(|_| self.invalid(name, "a signed 32-bit decimal integer", stored))
            })
            .transpose()
    }

    /// Decode one property holding a value that travels as encoded JSON.
    fn decoded(&self, name: &str) -> Result<Option<Scalar>> {
        self.get(name).map(crate::json::from_utf8).transpose()
    }

    /// Name the full key a stored value failed under, and what it should be.
    fn invalid(&self, name: &str, expected: &str, actual: &str) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason: format_smolstr!("expected {expected}, got {actual:?}"),
        }
    }
}

impl IcebergFieldMut<'_> {
    /// Records the identifier of the schema this root is.
    ///
    /// # Errors
    ///
    /// Returns an error when the property write fails the validation every
    /// metadata write goes through, leaving the field unchanged. Every write
    /// here fails the same way.
    pub fn set_schema_id(&mut self, id: i32) -> Result<()> {
        self.store(SCHEMA_ID, id.to_string())
    }

    /// Records the identifier columns of a schema root.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_identifier_field_ids(&mut self, ids: &[i32]) -> Result<()> {
        let joined: Vec<String> = ids.iter().map(i32::to_string).collect();
        self.store(IDENTIFIER, joined.join(","))
    }

    /// Records this column's Iceberg documentation string.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_doc(&mut self, doc: impl Into<String>) -> Result<()> {
        self.store(DOC, doc)
    }

    /// Records a v3 `initial-default` as encoded JSON.
    ///
    /// The v3 defaults are values, not schema, so they travel as JSON text
    /// rather than as a second parallel value model.
    ///
    /// # Errors
    ///
    /// Returns an error when the value has no JSON representation, or when the
    /// property write fails. Both default writes fail the same way.
    pub fn set_initial_default(&mut self, value: &Scalar) -> Result<()> {
        self.store(INITIAL_DEFAULT, crate::json::into_utf8(value)?)
    }

    /// Records a v3 `write-default` as encoded JSON.
    ///
    /// # Errors
    ///
    /// [`Self::set_initial_default`] carries the rule.
    pub fn set_write_default(&mut self, value: &Scalar) -> Result<()> {
        self.store(WRITE_DEFAULT, crate::json::into_utf8(value)?)
    }

    /// Records the Iceberg type spelling the physical datatype cannot keep.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_declared_type(&mut self, value: impl Into<String>) -> Result<()> {
        self.store(DECLARED_TYPE, value)
    }

    /// Records the identifier of the spec a partition tuple belongs to.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_spec_id(&mut self, id: i32) -> Result<()> {
        self.store(SPEC_ID, id.to_string())
    }

    /// Records the schema column a partition field reads.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_partition_source_id(&mut self, id: i32) -> Result<()> {
        self.store(SOURCE_ID, id.to_string())
    }

    /// Records how this partition field derives its value.
    ///
    /// # Errors
    ///
    /// [`Self::set_schema_id`] carries the rule.
    pub fn set_transform(&mut self, transform: &Transform) -> Result<()> {
        self.store(TRANSFORM, transform.to_string())
    }

    /// Write one property, dropping the prior value a generic insert answers.
    fn store(&mut self, name: &str, value: impl Into<String>) -> Result<()> {
        self.insert(name, value)?;
        Ok(())
    }
}
