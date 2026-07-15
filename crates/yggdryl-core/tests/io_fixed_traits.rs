//! Tests that exercise the `io::fixed` **generic trait hierarchy** — programming against the
//! root traits (`DataType` / `TypedDataType` / `ScalarType` / `SerieType` / `BufferType` /
//! `FieldType`), the `Fixed*` sub-traits' mutualized default methods, and `&dyn` erasure —
//! rather than the concrete structs directly.

use yggdryl_core::io::fixed::{
    Buffer, Field, FixedBuffer, FixedDataType, FixedField, FixedScalar, FixedSerie, PrimitiveType,
    Scalar, Serie, TypedField,
};
use yggdryl_core::io::{BufferType, DataType, FieldType, ScalarType, SerieType, TypedDataType};

// -------------------------------------------------------------------------------------
// DataType / TypedDataType / FixedDataType
// -------------------------------------------------------------------------------------

/// Generic over any erased descriptor.
fn describe(dt: &dyn DataType) -> (&'static str, usize, bool) {
    (dt.name(), dt.byte_width(), dt.is_fixed_width())
}

#[test]
fn data_type_dyn_and_typed() {
    assert_eq!(describe(&PrimitiveType::<i32>::new()), ("i32", 4, true));
    assert_eq!(describe(&PrimitiveType::<f64>::new()), ("f64", 8, true));

    // TypedDataType carries the element as an associated type.
    fn native_name<D: TypedDataType + Default>() -> &'static str {
        D::default().name()
    }
    assert_eq!(native_name::<PrimitiveType<i8>>(), "i8");

    // FixedDataType default helpers (the "pre-implementations").
    let dt = PrimitiveType::<u16>::new();
    assert_eq!(FixedDataType::native_name(&dt), "u16");
    assert_eq!(FixedDataType::native_width(&dt), 2);
}

// -------------------------------------------------------------------------------------
// ScalarType / FixedScalar
// -------------------------------------------------------------------------------------

fn is_null<S: ScalarType>(scalar: &S) -> bool {
    scalar.is_null()
}

#[test]
fn scalar_type_generic_and_defaults() {
    assert!(is_null(&Scalar::<i32>::null()));
    assert!(!is_null(&Scalar::of(5i32)));

    // FixedScalar: the value + the mutualized-default serialized_width.
    let scalar = Scalar::of(7i64);
    assert_eq!(FixedScalar::value(&scalar), Some(7));
    assert_eq!(FixedScalar::serialized_width(&scalar), 9); // 1 + 8
    assert!(ScalarType::is_valid(&scalar)); // default on the root
    assert_eq!(ScalarType::data_type(&scalar).name(), "i64");
}

// -------------------------------------------------------------------------------------
// SerieType / FixedSerie
// -------------------------------------------------------------------------------------

fn null_count<C: SerieType>(column: &C) -> usize {
    column.null_count()
}

#[test]
fn serie_type_generic_and_defaults() {
    let column = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(null_count(&column), 1);
    assert_eq!(SerieType::len(&column), 3);
    assert!(SerieType::has_nulls(&column)); // default on the root
    assert_eq!(SerieType::get(&column, 0), Some(1));
    // FixedSerie default data_type.
    assert_eq!(<Serie<i32> as FixedSerie>::data_type(&column).name(), "i32");
}

// -------------------------------------------------------------------------------------
// BufferType / FixedBuffer
// -------------------------------------------------------------------------------------

#[test]
fn buffer_type_generic_and_defaults() {
    let buffer = Buffer::<u16>::from_vec(vec![10, 20, 30]);
    assert_eq!(BufferType::count(&buffer), 3);
    assert!(!BufferType::is_empty(&buffer)); // default on the root
    assert_eq!(BufferType::get(&buffer, 1), Some(20));
    assert_eq!(BufferType::as_bytes(&buffer).len(), 6);
    assert_eq!(
        <Buffer<u16> as FixedBuffer>::data_type(&buffer).byte_width(),
        2
    );
}

// -------------------------------------------------------------------------------------
// FieldType / FixedField — erased and typed both implement the root
// -------------------------------------------------------------------------------------

fn field_shape(field: &dyn FieldType) -> (&str, &'static str, usize, bool) {
    (
        field.name(),
        field.type_name(),
        field.byte_width(),
        field.nullable(),
    )
}

#[test]
fn field_type_dyn_erased_and_typed() {
    let typed = TypedField::<f64>::new("price", true);
    assert_eq!(field_shape(&typed), ("price", "f64", 8, true));

    let erased: Field = typed.erase();
    assert_eq!(field_shape(&erased), ("price", "f64", 8, true));

    // FixedField default data_type.
    assert_eq!(
        <TypedField<f64> as FixedField>::data_type(&typed).name(),
        "f64"
    );
}
