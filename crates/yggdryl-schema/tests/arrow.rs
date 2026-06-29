//! Round-trips a [`Field`] through Apache Arrow, exercising the trait-default
//! `to_arrow` / `from_arrow` that leverage [`DataType`]'s conversion.
#![cfg(feature = "arrow")]

use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowType;
use yggdryl_schema::{DataType, DataTypeId, Field, Metadata, SchemaError};

/// A minimal physical type used to exercise the trait machinery.
#[derive(Clone, Debug, PartialEq)]
struct Int32;

impl DataType for Int32 {
    fn name(&self) -> &'static str {
        "int32"
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Int32
    }

    fn to_arrow(&self) -> ArrowType {
        ArrowType::Int32
    }

    fn from_arrow(dtype: &ArrowType) -> Result<Self, SchemaError> {
        match dtype {
            ArrowType::Int32 => Ok(Int32),
            other => Err(SchemaError::UnsupportedArrowType(other.clone())),
        }
    }
}

/// A concrete field that only supplies the accessors and `from_parts`.
#[derive(Clone, Debug, PartialEq)]
struct Col {
    name: String,
    dtype: Int32,
    nullable: bool,
    metadata: Metadata,
}

impl Field for Col {
    type Type = Int32;

    fn name(&self) -> &str {
        &self.name
    }

    fn dtype(&self) -> &Int32 {
        &self.dtype
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn from_parts(name: String, dtype: Int32, nullable: bool, metadata: Metadata) -> Self {
        Col {
            name,
            dtype,
            nullable,
            metadata,
        }
    }
}

#[test]
fn field_round_trips_through_arrow() {
    let mut metadata = BTreeMap::new();
    metadata.insert(b"unit".to_vec(), b"count".to_vec());
    let col = Col::from_parts("id".into(), Int32, false, metadata);

    let arrow = col.to_arrow().unwrap();
    assert_eq!(arrow.name(), "id");
    assert_eq!(arrow.data_type(), &ArrowType::Int32);
    assert!(!arrow.is_nullable());
    assert_eq!(
        arrow.metadata().get("unit").map(String::as_str),
        Some("count")
    );

    let back = Col::from_arrow(&arrow).unwrap();
    assert_eq!(back, col);
}

#[test]
fn unsupported_arrow_type_errors() {
    let err = Int32::from_arrow(&ArrowType::Utf8).unwrap_err();
    assert!(matches!(err, SchemaError::UnsupportedArrowType(_)));
}

#[test]
fn non_utf8_metadata_errors() {
    let mut metadata = BTreeMap::new();
    metadata.insert(vec![0xff, 0xfe], b"x".to_vec());
    let col = Col::from_parts("c".into(), Int32, true, metadata);
    assert!(matches!(
        col.to_arrow().unwrap_err(),
        SchemaError::NonUtf8Metadata
    ));
}
