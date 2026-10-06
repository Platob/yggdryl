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
    let dtype = DataType::TimeInForce;
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
        DataType::TimeInForce.scalar("ioc").unwrap(),
        Scalar::TimeInForce(TimeInForce::ImmediateOrCancel)
    );
    assert_eq!(
        DataType::TimeInForce.scalar(5_i32).unwrap(),
        Scalar::TimeInForce(TimeInForce::FillOrKill)
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
    assert!(DataType::TimeInForce.scalar(Scalar::from("UNKN")).is_err());
}

/// A text column lands as the codes its spellings name, and the column
/// renders back as the stored names.
#[test]
fn a_text_column_lands_as_codes_and_renders_as_names() {
    let field = Field::new("timeinforce", DataType::TimeInForce, true);
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
