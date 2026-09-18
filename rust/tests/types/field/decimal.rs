//! The decimal family's field: one marker over every backing width.

use yggdryl::types::{DataTypeValue, DecimalField, DecimalType};
use yggdryl::{DataType, DataTypeId, Scalar};

use super::typed::assert_typed_marker;

#[test]
fn the_decimal_marker_covers_every_backing_width() {
    for dtype in [
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, 2).unwrap(),
        DataType::decimal128(38, 2).unwrap(),
        DataType::decimal256(76, 2).unwrap(),
    ] {
        assert_typed_marker::<DecimalType>(dtype);
    }
}

#[test]
fn a_leaf_is_the_width_and_the_family_is_what_the_number_means() {
    // The leaf says which integer holds the coefficient; the precision and
    // the scale say what the column means, and both are read without a match.
    let narrow = DecimalType::Decimal32 {
        precision: 9,
        scale: 2,
    };
    assert_eq!(narrow.precision(), 9);
    assert_eq!(narrow.scale(), 2);
    assert_eq!(narrow.maximum(), 9);
    assert_eq!(narrow.id(), DataTypeId::Decimal32);
    assert_eq!(narrow.to_string(), "decimal32(9,2)");

    // `decimal` picks the narrowest width that holds the digits, and the four
    // named constructors are the same rule with the width already chosen.
    for (precision, width) in [
        (1_u8, DataTypeId::Decimal32),
        (9, DataTypeId::Decimal32),
        (10, DataTypeId::Decimal64),
        (18, DataTypeId::Decimal64),
        (19, DataTypeId::Decimal128),
        (38, DataTypeId::Decimal128),
        (39, DataTypeId::Decimal256),
        (76, DataTypeId::Decimal256),
    ] {
        assert_eq!(DataType::decimal(precision, 0).unwrap().id(), width);
        assert_eq!(DecimalType::narrowest(precision, 0).id(), width);
    }

    // Each width states its own maximum, and a precision past it is refused
    // by the name of the width that could not hold it.
    assert!(DataType::decimal32(10, 0).is_err());
    assert!(DataType::decimal64(19, 0).is_err());
    assert!(DataType::decimal128(39, 0).is_err());
    assert!(DataType::decimal256(77, 0).is_err());
    let refused = DataType::decimal32(10, 0).unwrap_err().to_string();
    assert!(refused.contains("Decimal32"), "{refused}");
    // A positive scale cannot exceed the digits it is taken out of.
    assert!(DataType::decimal128(4, 5).is_err());
    // The variants stay public, so an unchecked one is caught at the boundary.
    assert!(
        DataTypeValue::validate(&DecimalType::Decimal32 {
            precision: 10,
            scale: 0
        })
        .is_err()
    );
}

#[test]
fn a_decimal_field_holds_its_own_leaf_and_the_root_redirects_to_it() {
    let field = DecimalField::try_new("price", DataType::decimal128(38, 18).unwrap(), false)
        .unwrap();
    assert_eq!(field.typed_dtype_ref().precision(), 38);
    assert_eq!(field.typed_dtype_ref().scale(), 18);
    assert_eq!(field.dtype(), &DataType::decimal128(38, 18).unwrap());
    assert_eq!(field.dtype().id(), DataTypeId::Decimal128);

    // A datatype from another family is refused by name.
    assert!(DecimalField::try_new("price", DataType::Int64, false).is_err());

    // The value door is the family's, whichever width the column is.
    let held = field.to_field().scalar(Scalar::from("1.5")).unwrap();
    assert_eq!(held.id(), DataTypeId::Decimal128);
}
