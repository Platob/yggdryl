//! `rust/src/timeinforce.rs`: how long an order stands, FIX's
//! `TimeInForce(59)` code set as one enum leaf stored as a `uint8`.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StringArray, UInt8Array};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeKind, Field, Scalar, Serie, StringEnum, TimeInForce,
};

/// Every shipped wire value and the name the standard gives it, as the
/// crate's `config/fix` code set states them.
fn shipped() -> Vec<(String, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config/fix/codesets/timeinforcecodeset.json");
    let shipped: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    shipped["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| {
            (
                code["value"].as_str().unwrap().to_owned(),
                code["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_time_in_force_is_an_enum_leaf_over_uint8_codes() {
    let dtype = DataType::timeinforce();
    assert_eq!(DataType::from_str("timeinforce").unwrap(), dtype);
    assert_eq!(DataType::timeinforce(), dtype);
    assert_eq!(dtype.to_string(), "timeinforce");
    assert_eq!(dtype.kind(), DataTypeKind::Enum);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(dtype.id().as_u8(), 0xc5);

    let field = Field::new("timeinforce", dtype.clone(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(
        arrow.metadata()["ARROW:extension:name"],
        "yggdryl.timeinforce"
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    assert_eq!(std::mem::size_of::<TimeInForce>(), 1);
}

/// One member per wire value of the shipped code set, in wire order: each
/// reads from its wire value and its standard name, and answers the wire
/// value back.
#[test]
fn every_shipped_wire_value_is_one_member() {
    let shipped = shipped();
    assert_eq!(shipped.len(), 13);
    let mut wires: Vec<&str> = shipped.iter().map(|(wire, _)| wire.as_str()).collect();
    wires.sort_unstable();
    assert_eq!(wires.as_slice(), StringEnum::TIMESINFORCE);
    for (at, (wire, name)) in shipped.iter().enumerate() {
        let member = TimeInForce::from_fix(wire);
        assert_eq!(usize::from(member.code()), at + 1, "{wire} in wire order");
        assert_eq!(member.fix_code(), Some(wire.as_str()), "{wire}");
        assert_eq!(TimeInForce::from_spelling(name), Some(member), "{name}");
        assert_eq!(TimeInForce::from_spelling(wire), Some(member), "{wire}");
        assert_eq!(TimeInForce::from_code(member.code()), Some(member));
    }
    // A venue's own value is the catch-all, which stands for no one value.
    assert_eq!(TimeInForce::from_fix("Z"), TimeInForce::Other);
    // A bridge writing a member's spelling where the standard writes its
    // wire value reads as that member.
    assert_eq!(TimeInForce::from_fix("day"), TimeInForce::Day);
    assert_eq!(
        TimeInForce::from_fix("GoodTillCancel"),
        TimeInForce::GoodTillCancel
    );
    assert_eq!(TimeInForce::Other.fix_code(), None);
    assert_eq!(TimeInForce::Unknown.fix_code(), None);
    assert_eq!(TimeInForce::default(), TimeInForce::Unknown);
}

/// The stored name in any case, the standard's name folded, and the wire
/// value unfolded - `A` and `a` differ in FIX - reach one member; a stored
/// code is an integer and never text.
#[test]
fn a_spelling_reads_as_one_member_and_a_stranger_is_refused() {
    for (spelling, member) in [
        ("GTC", TimeInForce::GoodTillCancel),
        ("gtc", TimeInForce::GoodTillCancel),
        ("good_till_cancel", TimeInForce::GoodTillCancel),
        ("GOOD TILL CANCEL", TimeInForce::GoodTillCancel),
        ("1", TimeInForce::GoodTillCancel),
        ("day", TimeInForce::Day),
        ("0", TimeInForce::Day),
        ("C", TimeInForce::GoodForMonth),
        ("GoodForMonth", TimeInForce::GoodForMonth),
        ("other", TimeInForce::Other),
    ] {
        assert_eq!(
            TimeInForce::from_spelling(spelling),
            Some(member),
            "{spelling}"
        );
        assert_eq!(TimeInForce::read(spelling).unwrap(), member, "{spelling}");
    }
    assert_eq!(TimeInForce::from_spelling("c"), None);
    let refused = TimeInForce::read("GTXGTXGT").unwrap_err().to_string();
    assert!(refused.contains("timeinforce"), "{refused}");
    assert!(refused.contains("GTXGTXGT"), "{refused}");
    assert!(TimeInForce::read_code(14).is_err());

    // The value door reads the same spellings and codes.
    assert_eq!(
        DataType::timeinforce().scalar("ioc").unwrap(),
        Scalar::from(TimeInForce::ImmediateOrCancel)
    );
    assert_eq!(
        DataType::timeinforce().scalar(5_i32).unwrap(),
        Scalar::from(TimeInForce::FillOrKill)
    );
}

/// `UKNW` is the zero member's four-letter spelling, and `UNKN`, the
/// spelling it was stored under before, names no time in force at any
/// door, so a document written under it is rebuilt by its writer, never
/// read back as the member; a stored column holds the code `0` and reads
/// unchanged.
#[test]
fn the_retired_spelling_unkn_names_no_time_in_force() {
    assert_eq!(
        TimeInForce::from_spelling("UKNW"),
        Some(TimeInForce::Unknown)
    );
    assert_eq!(
        TimeInForce::from_spelling("uknw"),
        Some(TimeInForce::Unknown)
    );
    for spelling in ["UNKN", "unkn", "Unkn"] {
        assert_eq!(TimeInForce::from_spelling(spelling), None, "{spelling}");
        let refused = TimeInForce::read(spelling).unwrap_err().to_string();
        assert!(refused.contains("timeinforce"), "{refused}");
        assert!(refused.contains(spelling), "{refused}");
    }
    assert!(serde_json::from_str::<TimeInForce>("\"UNKN\"").is_err());
    assert_eq!(
        serde_json::from_str::<TimeInForce>("\"UKNW\"").unwrap(),
        TimeInForce::Unknown
    );
    assert!(
        DataType::timeinforce()
            .scalar(Scalar::from("UNKN"))
            .is_err()
    );
}

/// A text column lands as the codes its spellings name, and the column
/// renders back as the stored names.
#[test]
fn a_text_column_lands_as_codes_and_renders_as_names() {
    let field = Field::new("timeinforce", DataType::timeinforce(), true);
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(StringArray::from(vec!["0", "GTC", "ImmediateOrCancel"])) as ArrayRef,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap();
    let codes = landed.require_arrow_array().unwrap();
    let codes = codes.as_any().downcast_ref::<UInt8Array>().unwrap();
    assert_eq!(codes.values().as_ref(), [1, 2, 4]);
    let names = landed
        .cast(
            &Field::new("x", DataType::utf8(), true),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert_eq!(names.scalar(2).unwrap(), Scalar::from("IOC"));
}

/// The wire contract of `timeinforce` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_timeinforce_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("timeinforce").unwrap();
    let value = dtype.scalar("GTC").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0xc5).unwrap());
    assert_eq!(dtype.id().as_u8(), 0xc5);
    assert_eq!(dtype.id().as_str(), "timeinforce");
    assert_eq!(dtype.id().kind(), DataTypeKind::Enum);
    assert_eq!(dtype.to_string(), "timeinforce");
    assert_eq!(value.kind(), "timeinforce");
    assert_eq!(value.id().as_u8(), 0xc5);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(value.enum_code(), Some(2_u16));
    assert_eq!(value.enum_name(), Some("GTC"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "timeinforce");
    assert_eq!(
        Field::from_str("value timeinforce").unwrap().dtype(),
        &dtype
    );
    assert_eq!(DataType::from_logical_name("timeinforce").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["timeinforce"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(
        dtype.id().arrow_extension_name(),
        Some("yggdryl.timeinforce")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.timeinforce")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(UInt8Array::from(vec![2_u8]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"timeinforce"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"timeinforce"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"timeinforce"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"timeinforce"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"timeinforce","value":"GTC"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"timeinforce","value":"GTC"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 197, 2, 0, 0, 0]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("GTC")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 6704438739160732642);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(6704438739160732642)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(13720035095577081079)
    );
    assert_eq!(dtype.stable_hash(), 15877182161293383149);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `timeinforce`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_timeinforce_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("timeinforce").unwrap();
        let value = dtype.scalar("GTC").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            13668894700351291820
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            6817303614602540286
        );
    }
}
