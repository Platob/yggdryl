//! The [`Schema`] type — an ordered list of [`Field`]s with metadata: the header
//! shared by every [`Frame`](crate::Frame), mirroring `arrow_schema::Schema`.

use std::collections::BTreeMap;
use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::parse::split_top_level;
use crate::{Field, FieldError};

/// Error returned when a [`Schema`] cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// One of the comma-separated fields was invalid.
    Field(FieldError),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Field(err) => write!(f, "schema field: {err}"),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<FieldError> for SchemaError {
    fn from(err: FieldError) -> SchemaError {
        SchemaError::Field(err)
    }
}

/// An ordered collection of [`Field`]s plus string key/value metadata — the shape
/// of a table. Mirrors `arrow_schema::Schema`; metadata is kept in a [`BTreeMap`]
/// for deterministic ordering.
///
/// ```
/// use yggdryl_saga::Schema;
///
/// let schema = Schema::from_str("ts: timestamp(ns, UTC) not null, px: float64").unwrap();
/// assert_eq!(schema.len(), 2);
/// assert_eq!(schema.index_of("px"), Some(1));
/// assert_eq!(schema.to_str(), "ts: timestamp(ns, UTC) not null, px: float64");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Schema {
    fields: Vec<Field>,
    metadata: BTreeMap<String, String>,
}

impl Schema {
    /// Creates a schema from its fields, with no metadata.
    pub fn new(fields: Vec<Field>) -> Schema {
        Schema {
            fields,
            metadata: BTreeMap::new(),
        }
    }

    /// The empty schema (no fields, no metadata).
    pub fn empty() -> Schema {
        Schema::new(Vec::new())
    }

    /// Returns a copy with the metadata replaced.
    pub fn with_metadata(mut self, metadata: BTreeMap<String, String>) -> Schema {
        self.metadata = metadata;
        self
    }

    /// The fields, in order.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The field at `index`, if any.
    pub fn field(&self, index: usize) -> Option<&Field> {
        self.fields.get(index)
    }

    /// The first field named `name`, if any.
    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name() == name)
    }

    /// The position of the first field named `name`, if any.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name() == name)
    }

    /// The field names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.fields.iter().map(Field::name).collect()
    }

    /// The number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the schema has no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The metadata map (possibly empty).
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Parses a comma-separated list of `name: type` fields (each as
    /// [`Field::from_str`]). An empty string is the empty schema. Metadata is not
    /// part of the string form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Schema, SchemaError> {
        log_event!(trace, "Schema::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(Schema::empty());
        }
        let fields = split_top_level(trimmed, ',')
            .into_iter()
            .map(|s| Field::from_str(s).map_err(SchemaError::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Schema::new(fields))
    }

    /// Renders the fields as a comma-separated `name: type` list — the inverse of
    /// [`from_str`](Schema::from_str). Metadata is not rendered.
    pub fn to_str(&self) -> String {
        self.fields
            .iter()
            .map(Field::to_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Converts to an `arrow_schema::Schema` (infallible), carrying fields and
    /// metadata.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Schema {
        let metadata: std::collections::HashMap<String, String> = self
            .metadata
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let fields: Vec<arrow_schema::Field> =
            self.fields.iter().map(arrow_schema::Field::from).collect();
        arrow_schema::Schema::new_with_metadata(fields, metadata)
    }

    /// Builds a [`Schema`] from an `arrow_schema::Schema` (infallible).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(schema: &arrow_schema::Schema) -> Schema {
        Schema {
            fields: schema
                .fields()
                .iter()
                .map(|f| Field::from(f.as_ref()))
                .collect(),
            metadata: schema
                .metadata()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataType, PrimitiveType};

    #[test]
    fn build_and_query() {
        let schema = Schema::new(vec![
            Field::new("id", PrimitiveType::Int64.into(), false),
            Field::new("name", PrimitiveType::Utf8.into(), true),
        ]);
        assert_eq!(schema.len(), 2);
        assert!(!schema.is_empty());
        assert_eq!(schema.names(), ["id", "name"]);
        assert_eq!(schema.index_of("name"), Some(1));
        assert_eq!(schema.index_of("nope"), None);
        assert_eq!(
            schema.field_by_name("id").unwrap().data_type(),
            &DataType::from(PrimitiveType::Int64)
        );
    }

    #[test]
    fn string_round_trips() {
        let s = "id: int64 not null, tags: list<item: utf8>";
        let schema = Schema::from_str(s).unwrap();
        assert_eq!(schema.to_str(), s);
        // The empty schema round-trips through the empty string.
        assert!(Schema::from_str("").unwrap().is_empty());
        assert_eq!(Schema::empty().to_str(), "");
    }

    #[test]
    fn errors_propagate_from_fields() {
        assert!(matches!(
            Schema::from_str("ok: int64, bad: notatype"),
            Err(SchemaError::Field(_))
        ));
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn arrow_round_trips() {
        let mut meta = BTreeMap::new();
        meta.insert("source".to_string(), "feed".to_string());
        let schema = Schema::from_str("id: int64 not null, px: float64")
            .unwrap()
            .with_metadata(meta);

        let arrow = schema.to_arrow();
        assert_eq!(arrow.fields().len(), 2);
        assert_eq!(arrow.metadata().get("source"), Some(&"feed".to_string()));
        assert_eq!(Schema::from_arrow(&arrow), schema);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trips() {
        let schema = Schema::from_str("a: int64, b: utf8 not null").unwrap();
        let json = serde_json::to_string(&schema).unwrap();
        assert_eq!(serde_json::from_str::<Schema>(&json).unwrap(), schema);
    }
}
