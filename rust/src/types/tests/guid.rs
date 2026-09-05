use arrow_array::{Array, FixedSizeBinaryArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::{DataTypeId, DataTypeKind};
use crate::{Field, Scalar};

const TEXT: &str = "01912d68-783e-7c9a-b1f2-0123456789ab";
const PACKED: u128 = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab;

#[test]
fn the_identity_is_the_sixteen_bytes_and_the_spelling_is_a_rendering() {
    let guid = DataType::guid();
    assert_eq!(guid, DataType::Guid);
    assert_eq!(guid.id(), DataTypeId::Guid);
    assert_eq!(guid.kind(), DataTypeKind::Guid);
    assert_eq!(guid.name(), "guid");
    assert_eq!(guid.to_string(), "guid");
    assert_eq!(DataTypeId::Guid.fixed_byte_width(), Some(16));
    assert!(!guid.is_nested());
    guid.validate().unwrap();

    // Both spellings parse to the one type, which displays as `guid`.
    assert_eq!("guid".parse::<DataType>().unwrap(), guid);
    assert_eq!("uuid".parse::<DataType>().unwrap(), guid);
    assert_eq!(guid.to_string().parse::<DataType>().unwrap(), guid);

    // The packed integer is the identifier, not a code for it.
    assert_eq!(guid.guid_packed(TEXT.as_bytes()).unwrap(), PACKED);
    assert_eq!(
        guid.guid_packed(TEXT.to_uppercase().as_bytes()).unwrap(),
        PACKED
    );
    assert_eq!(
        guid.guid_packed(TEXT.replace('-', "").as_bytes()).unwrap(),
        PACKED
    );
    assert_eq!(guid.guid_packed(&PACKED.to_be_bytes()).unwrap(), PACKED);
    assert_eq!(guid.guid_value(PACKED).unwrap(), TEXT);

    // Every accepted rendering canonicalizes to the exact packed GUID leaf.
    let field = guid.clone().required_field("id");
    let row = DataType::from_fields([field.clone()])
        .unwrap()
        .required_field("row");
    let canonical = |value: Scalar| {
        row.canonicalize_value(Scalar::from_sequence([value]))
            .unwrap()
    };
    let exact = Scalar::Guid(crate::types::Guid::new(PACKED));
    let expected = Scalar::from_sequence([exact]);
    assert_eq!(canonical(Scalar::from(TEXT)), expected);
    assert_eq!(canonical(Scalar::from(TEXT.to_uppercase())), expected);
    assert_eq!(
        canonical(Scalar::from(PACKED.to_be_bytes().to_vec())),
        expected
    );
    assert_eq!(
        guid.default_value().unwrap(),
        Scalar::Guid(crate::types::Guid::new(0))
    );
    assert!(
        guid.is_default_value(&Scalar::from([0_u8; 16].to_vec()))
            .unwrap()
    );
}

#[test]
fn storage_is_the_canonical_arrow_uuid_extension_over_sixteen_bytes() {
    let field = Field::new("id", DataType::Guid, false);
    let arrow = field.clone().into_arrow().unwrap();

    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow(&arrow).unwrap(), field);

    // The stored bytes are the identifier; the value reads back exact.
    let array = crate::arrow::scalar_array(&field, &Scalar::from(TEXT)).unwrap();
    let stored = array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(stored.value(0), PACKED.to_be_bytes());
    assert_eq!(
        crate::arrow::scalar_value(&field, array.as_ref()).unwrap(),
        Scalar::Guid(crate::types::Guid::new(PACKED))
    );
}

#[test]
fn what_is_not_an_identifier_is_refused_by_the_one_rule() {
    let guid = DataType::Guid;
    for spelling in [
        "not-a-guid",
        "",
        "01912d68-783e-7c9a-b1f2-0123456789a",
        "01912d68-783e-7c9a-b1f2-0123456789abc",
        "01912d68783e7c9ab1f20123456789ab0",
        "01912d68-783e-7c9a-b1f2+0123456789ab",
        "0191_d68-783e-7c9a-b1f2-0123456789ab",
    ] {
        assert!(guid.guid_packed(spelling.as_bytes()).is_err(), "{spelling}");
    }
    let refused = guid.guid_packed(b"not-a-guid").unwrap_err().to_string();
    assert!(refused.contains("sixteen bytes"), "{refused}");
    assert!(refused.contains("36-character"), "{refused}");

    // The type answers only for itself.
    assert!(DataType::Utf8.guid_packed(TEXT.as_bytes()).is_err());
    assert!(DataType::Utf8.guid_value(PACKED).is_err());
    assert!(
        DataType::FixedSizeBinary(16)
            .guid_packed(TEXT.as_bytes())
            .is_err()
    );
}
