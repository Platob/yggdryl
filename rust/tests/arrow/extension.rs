//! `rust/src/arrow/extension.rs`: arrow-rs's typed `ExtensionType` for the
//! datatypes that ride a name, each answering as the field import does.

use arrow_schema::extension::ExtensionType;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{
    BytesType, CcyType, DataType, DataTypeId, Field, SideType, StateType, StringType, UuidType,
    VersionType,
};

/// The Arrow field the crate writes for one datatype.
fn exported(dtype: DataType) -> ArrowField {
    Field::new("x", dtype, true)
        .into_arrow_field()
        .expect("an Arrow field")
}

#[test]
fn a_marker_reads_the_field_the_crate_writes_and_refuses_another_storage() {
    let ccy = exported(DataType::Ccy);
    assert_eq!(CcyType::NAME, "yggdryl.ccy");
    assert_eq!(
        ccy.try_extension_type::<CcyType>().expect("a currency"),
        CcyType
    );
    assert_eq!(ccy.extension_type_name(), Some(CcyType::NAME));
    // Another name, or the name over a storage the code is not laid out in.
    assert!(ccy.try_extension_type::<SideType>().is_err());
    let mut wrong = ArrowField::new("x", ArrowDataType::Int32, true);
    assert!(wrong.try_with_extension_type(CcyType).is_err());
    // A document a code states none of is refused.
    let mut documented = ccy.metadata().clone();
    documented.insert("ARROW:extension:metadata".to_owned(), "{}".to_owned());
    assert!(
        ccy.clone()
            .with_metadata(documented)
            .try_extension_type::<CcyType>()
            .is_err()
    );
}

#[test]
fn an_arrow_field_typed_by_a_marker_imports_as_its_datatype() {
    for (field, dtype) in [
        (
            ArrowField::new("x", ArrowDataType::Utf8, true).with_extension_type(CcyType),
            DataType::Ccy,
        ),
        (
            ArrowField::new("x", ArrowDataType::UInt16, true).with_extension_type(StateType),
            DataType::State,
        ),
        (
            ArrowField::new("x", ArrowDataType::UInt8, true).with_extension_type(SideType),
            DataType::Side,
        ),
        (
            ArrowField::new("x", ArrowDataType::FixedSizeBinary(16), true)
                .with_extension_type(UuidType),
            DataType::Uuid,
        ),
        (
            ArrowField::new("x", ArrowDataType::Utf8, true).with_extension_type(VersionType),
            DataType::Version,
        ),
    ] {
        let imported = Field::from_arrow_field(&field).expect("the field imports");
        assert_eq!(imported.dtype(), &dtype, "{field:?}");
    }
    // A dictionary of a code is a code.
    let mut dictionary = ArrowField::new(
        "x",
        ArrowDataType::Dictionary(
            Box::new(ArrowDataType::Int32),
            Box::new(ArrowDataType::Utf8),
        ),
        true,
    );
    assert!(dictionary.try_with_extension_type(CcyType).is_ok());
}

#[test]
fn a_string_and_a_bytes_leaf_ride_their_document() {
    let leaf = DataType::fixed_ascii(4)
        .expect("a width")
        .string_parameters()
        .expect("a string leaf");
    let field = exported(DataType::from(leaf));
    let read = field
        .try_extension_type::<StringType>()
        .expect("a string leaf");
    assert_eq!(read, leaf);
    assert_eq!(read.serialize_metadata(), Some(leaf.extension_json()));
    let built = ArrowField::new("x", field.data_type().clone(), true).with_extension_type(leaf);
    assert_eq!(
        Field::from_arrow_field(&built).expect("imports").dtype(),
        &DataType::from(leaf)
    );
    // The leaf over a storage it does not lay out.
    let mut int64 = ArrowField::new("x", ArrowDataType::Int64, true);
    assert!(int64.try_with_extension_type(leaf).is_err());
    let bytes = DataType::sized_binary(16)
        .expect("a maximum")
        .bytes_parameters()
        .expect("a bytes leaf");
    let field = exported(DataType::from(bytes));
    assert_eq!(
        field
            .try_extension_type::<BytesType>()
            .expect("a bytes leaf"),
        bytes
    );
    // A leaf Arrow states alone rides no document.
    let plain = DataType::utf8().string_parameters().expect("a string leaf");
    let mut utf8 = ArrowField::new("x", ArrowDataType::Utf8, true);
    assert!(utf8.try_with_extension_type(plain).is_err());
    let plain = DataType::binary().bytes_parameters().expect("a bytes leaf");
    let mut binary = ArrowField::new("x", ArrowDataType::Binary, true);
    assert!(binary.try_with_extension_type(plain).is_err());
}

#[test]
fn the_names_are_one_per_extension_and_every_datatype_writes_its_own() {
    let names: Vec<&str> = DataTypeId::arrow_extension_names().collect();
    let mut distinct = names.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len(), "{names:?}");
    assert_eq!(names.len(), 30, "{names:?}");
    for id in DataTypeId::ALL {
        if let Some(name) = id.arrow_extension_name() {
            assert!(names.contains(&name), "{id:?}");
        }
    }
    // Each name a datatype writes is its identifier's.
    for dtype in [
        DataType::Ccy,
        DataType::State,
        DataType::Uuid,
        DataType::Decimal,
        DataType::Version,
        DataType::utf8(),
    ] {
        assert_eq!(
            dtype.arrow_extension().map(|(name, _)| name),
            dtype.id().arrow_extension_name(),
            "{dtype}"
        );
    }
}
