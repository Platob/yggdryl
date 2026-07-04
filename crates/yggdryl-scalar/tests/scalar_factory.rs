//! Integration tests for [`ScalarFactory`] — every typed data type building its
//! scalars (value, null and default), the "data type → scalar" factory of this
//! layer, across all eight integers, binary, optional, serie and map.

use yggdryl_scalar::yggdryl_dtype::{self as dtype, TypedDataType};
use yggdryl_scalar::{
    BinaryScalar, Int64Scalar, Scalar, ScalarFactory, TypedMapScalar, TypedOptionalScalar,
    TypedSerie, UInt8Scalar,
};

type Int64GenericSerie = TypedSerie<dtype::Int64Type, Int64Scalar>;
type RankMap = TypedMapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar>;

// Every integer type is its own scalar factory; one macro drives one test module per
// width over the whole factory surface (value / null / default).
macro_rules! int_factory_tests {
    ($mod:ident, $ty:ident, $scalar:ident, $native:ty) => {
        mod $mod {
            use super::*;
            use dtype::$ty;
            use yggdryl_scalar::$scalar;

            #[test]
            fn builds_value_null_and_default() {
                // scalar(value) holds the native value.
                assert_eq!($ty.scalar(7 as $native), $scalar::new(7 as $native));
                assert_eq!(
                    $ty.scalar(<$native>::MAX),
                    $scalar::new(<$native>::MAX) // the width's extreme survives
                );

                // null_scalar is null; default_scalar holds the type's default (zero).
                assert!($ty.null_scalar().is_null());
                assert_eq!($ty.default_scalar(), $scalar::new(<$native>::default()));
                assert_eq!($ty.default_value(), 0 as $native);

                // The factory's associated Scalar type is the concrete scalar.
                let built: <$ty as ScalarFactory<$native>>::Scalar = $ty.scalar(1 as $native);
                assert_eq!(built, $scalar::new(1 as $native));
            }
        }
    };
}

int_factory_tests!(int8, Int8Type, Int8Scalar, i8);
int_factory_tests!(int16, Int16Type, Int16Scalar, i16);
int_factory_tests!(int32, Int32Type, Int32Scalar, i32);
int_factory_tests!(int64, Int64Type, Int64Scalar, i64);
int_factory_tests!(uint8, UInt8Type, UInt8Scalar, u8);
int_factory_tests!(uint16, UInt16Type, UInt16Scalar, u16);
int_factory_tests!(uint32, UInt32Type, UInt32Scalar, u32);
int_factory_tests!(uint64, UInt64Type, UInt64Scalar, u64);

#[test]
fn binary_factory_builds_value_null_and_default() {
    assert_eq!(
        dtype::BinaryType.scalar(vec![1, 2, 3]),
        BinaryScalar::new(vec![1, 2, 3])
    );
    assert!(dtype::BinaryType.null_scalar().is_null());
    // Binary defaults to the empty byte sequence.
    assert_eq!(
        dtype::BinaryType.default_scalar(),
        BinaryScalar::new(Vec::new())
    );
    assert!(dtype::BinaryType.default_value().is_empty());
}

#[test]
fn optional_factory_wraps_the_value_or_the_null_variant() {
    let optional = dtype::TypedOptionalType::new(dtype::Int64Type);

    // scalar(value) wraps the inner value variant; null_scalar is the null variant.
    assert_eq!(optional.scalar(42).as_i64().unwrap(), 42);
    assert_eq!(optional.scalar(42).scalar(), Some(&Int64Scalar::new(42)));
    assert!(optional.null_scalar().is_null());

    // The optional's scalar models nullness, so its default is the null variant
    // (not a value scalar of the value type's default), matching `Option::default`.
    assert_eq!(optional.default_value(), 0);
    assert_eq!(
        optional.default_scalar(),
        TypedOptionalScalar::<dtype::Int64Type, Int64Scalar>::null()
    );
    assert!(optional.default_scalar().is_null());
}

#[test]
fn serie_factory_builds_from_a_native_sequence() {
    let serie_type = dtype::TypedSerieType::new(dtype::Int64Type);

    // scalar(values) builds the sequence through the value type's own factory.
    let numbers = serie_type.scalar(vec![1, 2, 3]);
    assert_eq!(numbers.len(), 3);
    assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));

    // null_scalar is null; default_scalar is the empty serie (not null).
    assert!(serie_type.null_scalar().is_null());
    let default = serie_type.default_scalar();
    assert!(!default.is_null() && default.is_empty());
    assert_eq!(default, Int64GenericSerie::new(Vec::new()));
    assert_eq!(serie_type.default_value(), Vec::<i64>::new());
}

#[test]
fn map_factory_builds_from_native_entries() {
    let map_type = dtype::TypedMapType::new(dtype::UInt8Type, dtype::Int64Type);

    // scalar(entries) builds each key/value through their own factories.
    let ranks = map_type.scalar(vec![(7u8, 42i64), (8, 43)]);
    assert_eq!(ranks.value().map(<[_]>::len), Some(2));

    // null_scalar is null; default_scalar is the empty map (not null).
    assert!(map_type.null_scalar().is_null());
    let default = map_type.default_scalar();
    assert!(!default.is_null());
    assert_eq!(default, RankMap::default());
    assert_eq!(map_type.default_value(), Vec::<(u8, i64)>::new());
}

#[test]
fn factories_reach_generic_code() {
    // Generic code bounds on ScalarFactory to build a type's scalars.
    fn value_of<T, D: ScalarFactory<T>>(data_type: &D, value: T) -> D::Scalar {
        data_type.scalar(value)
    }
    fn null_of<T, D: ScalarFactory<T>>(data_type: &D) -> D::Scalar {
        data_type.null_scalar()
    }
    fn default_of<T, D: ScalarFactory<T>>(data_type: &D) -> D::Scalar {
        data_type.default_scalar()
    }
    assert_eq!(value_of(&dtype::Int64Type, 7), Int64Scalar::new(7));
    assert!(null_of(&dtype::Int64Type).is_null());
    assert_eq!(default_of(&dtype::Int64Type), Int64Scalar::new(0));
    assert!(!default_of(&dtype::TypedSerieType::new(dtype::Int64Type)).is_null());
    assert!(default_of(&dtype::TypedOptionalType::new(dtype::Int64Type)).is_null());
}
