//! Integration tests for [`ScalarFactory`] — a typed data type building its scalars
//! (value, null and default), the "data type → scalar" factory of this layer.

use yggdryl_scalar::yggdryl_dtype::{self as dtype, TypedDataType};
use yggdryl_scalar::{
    BinaryScalar, Int64Scalar, MapScalar, OptionalScalar, Scalar, ScalarFactory, Serie, UInt8Scalar,
};

type Int64GenericSerie = Serie<dtype::Int64Type, Int64Scalar>;
type RankMap = MapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar>;

#[test]
fn a_data_type_builds_its_scalars() {
    // The typed data type is the factory: it builds a value scalar or the null one.
    assert_eq!(dtype::Int64Type.scalar(42), Int64Scalar::new(42));
    assert!(dtype::Int64Type.null_scalar().is_null());
    assert_eq!(dtype::UInt8Type.scalar(7), UInt8Scalar::new(7));
    assert!(dtype::UInt8Type.null_scalar().is_null());

    // Binary builds from owned bytes.
    assert_eq!(
        dtype::BinaryType.scalar(vec![1, 2, 3]),
        BinaryScalar::new(vec![1, 2, 3])
    );
    assert!(dtype::BinaryType.null_scalar().is_null());

    // The optional wraps the inner scalar as its value variant, or the null variant.
    let optional = dtype::OptionalType::new(dtype::Int64Type);
    assert_eq!(optional.scalar(42).as_i64().unwrap(), 42);
    assert!(optional.null_scalar().is_null());
}

#[test]
fn defaults_flow_through_the_typed_layer() {
    // Integers default to zero, held in a value scalar.
    assert_eq!(dtype::Int64Type.default_value(), 0);
    assert_eq!(dtype::Int64Type.default_scalar(), Int64Scalar::new(0));

    // The optional's scalar models nullness: its default is the null variant.
    let optional = dtype::OptionalType::new(dtype::Int64Type);
    assert_eq!(optional.default_value(), 0);
    assert_eq!(
        optional.default_scalar(),
        OptionalScalar::<dtype::Int64Type, Int64Scalar>::null()
    );

    // Sequences default to empty, not null.
    assert_eq!(
        dtype::SerieType::new(dtype::Int64Type).default_scalar(),
        Int64GenericSerie::new(Vec::new())
    );
    assert_eq!(
        dtype::MapType::new(dtype::UInt8Type, dtype::Int64Type).default_scalar(),
        RankMap::default()
    );

    // Binary defaults to the empty byte sequence.
    assert_eq!(
        dtype::BinaryType.default_scalar(),
        BinaryScalar::new(Vec::new())
    );
}

#[test]
fn factories_reach_generic_code() {
    // Generic code bounds on ScalarFactory to build a type's scalars.
    fn value_of<T, D: ScalarFactory<T>>(data_type: &D, value: T) -> D::Scalar {
        data_type.scalar(value)
    }
    fn default_of<T, D: ScalarFactory<T>>(data_type: &D) -> D::Scalar {
        data_type.default_scalar()
    }
    assert_eq!(value_of(&dtype::Int64Type, 7), Int64Scalar::new(7));
    assert_eq!(default_of(&dtype::Int64Type), Int64Scalar::new(0));
    assert!(!default_of(&dtype::SerieType::new(dtype::Int64Type)).is_null());
    assert!(default_of(&dtype::OptionalType::new(dtype::Int64Type)).is_null());
}
