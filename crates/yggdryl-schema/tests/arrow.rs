//! Round-trips a [`Field`] through Apache Arrow, exercising the trait-default
//! `to_arrow_field` / `from_arrow_field` that leverage [`DataType`]'s conversion
//! and the metadata strategy for types Arrow cannot represent exactly.
#![cfg(feature = "arrow")]

use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowType;
use yggdryl_schema::{BinaryType, DataType, DataTypeId, Field, Metadata, SchemaError};

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

    fn to_arrow_type(&self) -> ArrowType {
        ArrowType::Int32
    }

    fn from_arrow_type(dtype: &ArrowType, _metadata: &Metadata) -> Result<Self, SchemaError> {
        match dtype {
            ArrowType::Int32 => Ok(Int32),
            other => Err(SchemaError::UnsupportedArrowType(other.clone())),
        }
    }
}

/// A concrete field generic over its data type, supplying only the accessors and
/// `from_parts`.
#[derive(Clone, Debug, PartialEq)]
struct Col<T> {
    name: String,
    dtype: T,
    nullable: bool,
    metadata: Metadata,
}

impl<T: DataType + Clone> Field for Col<T> {
    type Type = T;

    fn name(&self) -> &str {
        &self.name
    }

    fn dtype(&self) -> &T {
        &self.dtype
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn from_parts(name: String, dtype: T, nullable: bool, metadata: Metadata) -> Self {
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

    let arrow = col.to_arrow_field().unwrap();
    assert_eq!(arrow.name(), "id");
    assert_eq!(arrow.data_type(), &ArrowType::Int32);
    assert!(!arrow.is_nullable());
    assert_eq!(
        arrow.metadata().get("unit").map(String::as_str),
        Some("count")
    );

    let back = Col::from_arrow_field(&arrow).unwrap();
    assert_eq!(back, col);
}

#[test]
fn byte_size_cap_round_trips_via_metadata() {
    let col: Col<BinaryType> = Col::from_parts(
        "blob".into(),
        BinaryType::new().with_byte_size(64),
        true,
        Metadata::new(),
    );

    let arrow = col.to_arrow_field().unwrap();
    // The storage type is a plain Binary; the cap lives in reserved metadata.
    assert_eq!(arrow.data_type(), &ArrowType::Binary);
    assert_eq!(
        arrow
            .metadata()
            .get("yggdryl:byte_size")
            .map(String::as_str),
        Some("64")
    );

    let back = Col::<BinaryType>::from_arrow_field(&arrow).unwrap();
    assert_eq!(back.dtype(), &BinaryType::new().with_byte_size(64));
    // The reserved key is consumed by the type rebuild, not surfaced as user metadata.
    assert!(back.metadata().is_empty());
}

#[test]
fn binary_without_cap_metadata_is_unbounded() {
    // A plain Arrow Binary with no reserved metadata rebuilds as an unbounded type.
    let arrow = arrow_schema::Field::new("blob", ArrowType::Binary, true);
    let back = Col::<BinaryType>::from_arrow_field(&arrow).unwrap();
    assert_eq!(back.dtype(), &BinaryType::new());
}

#[test]
fn unsupported_arrow_type_errors() {
    let err = Int32::from_arrow_type(&ArrowType::Utf8, &Metadata::new()).unwrap_err();
    assert!(matches!(err, SchemaError::UnsupportedArrowType(_)));
}

#[test]
fn non_utf8_metadata_errors() {
    let mut metadata = BTreeMap::new();
    metadata.insert(vec![0xff, 0xfe], b"x".to_vec());
    let col = Col::from_parts("c".into(), Int32, true, metadata);
    assert!(matches!(
        col.to_arrow_field().unwrap_err(),
        SchemaError::NonUtf8Metadata
    ));
}
