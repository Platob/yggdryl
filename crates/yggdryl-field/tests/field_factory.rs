//! Integration tests for the [`FieldFactory`] surface: every typed data type builds
//! its field, equal to the one constructed directly — across all eight integers,
//! binary, optional, serie and map.

use yggdryl_field::yggdryl_dtype::{
    self as dtype, BinaryType, Int64Type, TypedMapType, TypedOptionalType, TypedSerieType,
    UInt8Type,
};
use yggdryl_field::{
    BinaryField, Field, FieldFactory, Int64Field, TypedMapField, TypedOptionalField,
    TypedSerieField,
};

// Every integer type is its own field factory; one macro drives one test per width.
macro_rules! int_field_factory_tests {
    ($test:ident, $ty:ident, $field:ident, $native:ty) => {
        #[test]
        fn $test() {
            use dtype::$ty;
            use yggdryl_field::$field;

            // The factory builds the field its `Field` associated type names.
            let built: <$ty as FieldFactory<$native>>::Field = $ty.field("value", true);
            assert_eq!(built, $field::new("value", true));
            assert!(built.is_nullable());
            assert_eq!(built.name(), "value");

            // Nullability is carried through.
            assert!(!$ty.field("value", false).is_nullable());
        }
    };
}

int_field_factory_tests!(int8_builds_its_field, Int8Type, Int8Field, i8);
int_field_factory_tests!(int16_builds_its_field, Int16Type, Int16Field, i16);
int_field_factory_tests!(int32_builds_its_field, Int32Type, Int32Field, i32);
int_field_factory_tests!(int64_builds_its_field, Int64Type, Int64Field, i64);
int_field_factory_tests!(uint8_builds_its_field, UInt8Type, UInt8Field, u8);
int_field_factory_tests!(uint16_builds_its_field, UInt16Type, UInt16Field, u16);
int_field_factory_tests!(uint32_builds_its_field, UInt32Type, UInt32Field, u32);
int_field_factory_tests!(uint64_builds_its_field, UInt64Type, UInt64Field, u64);

#[test]
fn binary_builds_its_field() {
    assert_eq!(
        BinaryType.field("payload", true),
        BinaryField::new("payload", true)
    );
    assert!(!BinaryType.field("payload", false).is_nullable());
}

#[test]
fn parameterised_data_types_build_their_field() {
    // Serie, optional and map each build their typed field through the factory.
    assert_eq!(
        TypedSerieType::new(Int64Type).field("scores", true),
        TypedSerieField::<Int64Type>::new("scores", true)
    );
    assert_eq!(
        TypedOptionalType::new(Int64Type).field("score", true),
        TypedOptionalField::<Int64Type>::new("score", true)
    );
    assert_eq!(
        TypedMapType::new(UInt8Type, Int64Type).field("ranks", true),
        TypedMapField::<UInt8Type, Int64Type>::new("ranks", true)
    );
}

#[test]
fn factories_reach_generic_code() {
    // Generic code bounds on FieldFactory to build a type's field.
    fn field_of<T, D: FieldFactory<T>>(data_type: &D, name: &str, nullable: bool) -> D::Field {
        data_type.field(name, nullable)
    }
    assert_eq!(
        field_of(&Int64Type, "id", false),
        Int64Field::new("id", false)
    );
    assert_eq!(
        field_of(&TypedSerieType::new(Int64Type), "scores", true),
        TypedSerieField::<Int64Type>::new("scores", true)
    );
}
