//! The two securities identifiers beside `isin`: `cusip` and `sedol`.
//!
//! Each is a registered code closed by its own check digit, so a value is an
//! identifier or is refused, never a typo stored as a security. What the
//! generic code invariants in `coded` cannot pin is the check itself, the
//! case fold, and the canonical spelling a column is held to; those are
//! here, once per identifier, with the ISIN rule as the reference.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::types::{Cusip, CusipField, Sedol, SedolField};
use yggdryl::{
    ArrowCast, ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Scalar, Term,
};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

fn text(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn cells(array: &ArrayRef) -> Vec<Option<&str>> {
    let text = array.as_any().downcast_ref::<StringArray>().unwrap();
    (0..text.len())
        .map(|index| text.is_valid(index).then(|| text.value(index)))
        .collect()
}

/// The two identifiers: the name, the datatype, the width, one identifier
/// its standard closes, the same one in lower case, the same one with its
/// check digit off by one, and one a character short.
const IDENTIFIERS: [(&str, DataType, usize, &str, &str, &str, &str); 2] = [
    (
        "cusip",
        DataType::Cusip,
        9,
        "38259P508",
        "38259p508",
        "38259P509",
        "38259P50",
    ),
    (
        "sedol",
        DataType::Sedol,
        7,
        "B0YBKJ7",
        "b0ybkj7",
        "B0YBKJ8",
        "B0YBKJ",
    ),
];

#[test]
fn a_cusip_is_nine_characters_closed_by_its_check_digit() {
    // Six of issuer, two of issue, one check digit: the modulus-10
    // double-add-double digit of the eight before it.
    let apple = Cusip::new("037833100").unwrap();
    assert_eq!(apple.as_str(), "037833100");
    assert_eq!(apple.issuer(), "037833");
    assert_eq!(apple.issue(), "10");
    assert_eq!(apple.check_digit(), 0);
    assert_eq!(apple.to_string(), "037833100");
    // A letter reads as ten plus its alphabet position.
    assert_eq!(Cusip::new("38259P508").unwrap().check_digit(), 8);
    assert_eq!(Cusip::new("594918104").unwrap().check_digit(), 4);
    assert_eq!(Cusip::closing_digit("03783310"), Some(0));
    assert_eq!(Cusip::closing_digit("38259P50"), Some(8));
    assert_eq!(Cusip::closing_digit("59491810"), Some(4));
    // The rule answers without building a value, in either case.
    assert!(Cusip::is_valid("037833100"));
    assert!(Cusip::is_valid("38259p508"));
    assert!(Cusip::is_canonical("38259P508"));
    assert!(!Cusip::is_canonical("38259p508"));

    // Lower case is the upper case it spells, and stores as that.
    assert_eq!(
        Cusip::new("38259p508").unwrap(),
        Cusip::new("38259P508").unwrap()
    );
    assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");

    // One digit off is a typo, and the refusal names the identifier.
    let refused = Cusip::new("037833101").unwrap_err().to_string();
    assert!(refused.contains("cusip"), "{refused}");
    assert!(
        refused.contains("check digit does not close the identifier"),
        "{refused}"
    );
    assert!(!Cusip::is_valid("037833101"));
    // The wrong length, in both directions.
    let short = Cusip::new("03783310").unwrap_err().to_string();
    assert!(short.contains("expected nine characters"), "{short}");
    let long = Cusip::new("0378331000").unwrap_err().to_string();
    assert!(long.contains("at most 9 bytes"), "{long}");
    // A character outside the alphanumerics, a closing letter, and a body
    // the rule cannot close.
    let punctuated = Cusip::new("03783*100").unwrap_err().to_string();
    assert!(punctuated.contains("eight alphanumerics"), "{punctuated}");
    let letter = Cusip::new("03783310A").unwrap_err().to_string();
    assert!(letter.contains("closing check digit"), "{letter}");
    assert_eq!(Cusip::closing_digit("03783*10"), None);
    assert_eq!(Cusip::closing_digit("0378331"), None);
    assert_eq!(Cusip::closing_digit("38259p50"), None);
}

#[test]
fn a_sedol_is_seven_characters_closed_by_its_check_digit() {
    // Six alphanumerics weighted 1, 3, 1, 7, 3, 9 and the modulus-10 digit
    // that closes the weighted sum.
    let held = Sedol::new("B0YBKJ7").unwrap();
    assert_eq!(held.as_str(), "B0YBKJ7");
    assert_eq!(held.check_digit(), 7);
    assert_eq!(held.to_string(), "B0YBKJ7");
    assert_eq!(Sedol::new("0263494").unwrap().check_digit(), 4);
    assert_eq!(Sedol::new("B1F3M59").unwrap().check_digit(), 9);
    assert_eq!(Sedol::new("2046251").unwrap().check_digit(), 1);
    assert_eq!(Sedol::closing_digit("B0YBKJ"), Some(7));
    assert_eq!(Sedol::closing_digit("026349"), Some(4));
    assert_eq!(Sedol::closing_digit("B1F3M5"), Some(9));
    assert!(Sedol::is_valid("B0YBKJ7"));
    assert!(Sedol::is_valid("b0ybkj7"));
    assert!(Sedol::is_canonical("B0YBKJ7"));
    assert!(!Sedol::is_canonical("b0ybkj7"));

    // Lower case is the upper case it spells, and stores as that.
    assert_eq!(Sedol::new("b0ybkj7").unwrap(), held);
    assert_eq!(Sedol::new("b0ybkj7").unwrap().as_str(), "B0YBKJ7");

    // One digit off is a typo, and the refusal names the identifier.
    let refused = Sedol::new("B0YBKJ8").unwrap_err().to_string();
    assert!(refused.contains("sedol"), "{refused}");
    assert!(
        refused.contains("check digit does not close the identifier"),
        "{refused}"
    );
    assert!(!Sedol::is_valid("B0YBKJ8"));
    // The wrong length, in both directions.
    let short = Sedol::new("B0YBKJ").unwrap_err().to_string();
    assert!(short.contains("expected seven characters"), "{short}");
    let long = Sedol::new("B0YBKJ70").unwrap_err().to_string();
    assert!(long.contains("at most 7 bytes"), "{long}");
    // A character outside the alphanumerics, a closing letter, and a body
    // the rule cannot close.
    let punctuated = Sedol::new("B0Y-KJ7").unwrap_err().to_string();
    assert!(punctuated.contains("six alphanumerics"), "{punctuated}");
    let letter = Sedol::new("B0YBKJZ").unwrap_err().to_string();
    assert!(letter.contains("closing check digit"), "{letter}");
    assert_eq!(Sedol::closing_digit("B0Y-KJ"), None);
    assert_eq!(Sedol::closing_digit("B0YBK"), None);
    assert_eq!(Sedol::closing_digit("b0ybkj"), None);
}

#[test]
fn each_identifier_is_a_registered_code_that_round_trips_everywhere() {
    for (name, dtype, width, sample, lower, typo, short) in &IDENTIFIERS {
        // The grammar, the identifier and the listing.
        assert_eq!(dtype.to_string(), *name, "{name}");
        assert_eq!(DataType::from_str(name).unwrap(), *dtype, "{name}");
        assert_eq!(DataType::from_str(&name.to_uppercase()).unwrap(), *dtype);
        assert_eq!(DataType::from_logical_name(name).unwrap(), *dtype);
        assert_eq!(DataTypeId::from_str(name).unwrap(), dtype.id(), "{name}");
        assert_eq!(dtype.id().as_str(), *name);
        assert_eq!(dtype.kind(), DataTypeKind::Code);
        assert_eq!(dtype.code_name(), Some(*name));
        assert_eq!(dtype.code_width(), Some(*width), "{name}");
        assert_eq!(dtype.fixed_byte_width(), None);
        assert!(dtype.is_code());
        assert!(!dtype.is_string());
        assert_ne!(*dtype, DataType::fixed_ascii(*width as u32).unwrap());
        assert!(
            DataType::CODES
                .iter()
                .any(|(held, listed, held_width)| held == name
                    && listed == dtype
                    && held_width == width),
            "{name}"
        );

        // Serde of the datatype, in both spellings.
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(json, format!(r#"{{"type":"{name}"}}"#));
        assert_eq!(DataType::from_json(&json).unwrap(), *dtype);
        let rendered = serde_json::to_string(dtype).unwrap();
        assert_eq!(serde_json::from_str::<DataType>(&rendered).unwrap(), *dtype);

        // The value door: the check digit gates the space, the case folds,
        // and the stored value is the code under its own identity.
        let value = dtype.scalar(Scalar::from(*sample)).unwrap();
        assert!(value.is_code(), "{name}");
        assert_eq!(value.kind(), *name);
        assert_eq!(value.as_str(), Some(*sample));
        assert_eq!(value.dtype().unwrap(), *dtype);
        assert_eq!(dtype.scalar(Scalar::from(*lower)).unwrap(), value, "{name}");
        assert_eq!(dtype.scalar(value.clone()).unwrap(), value);
        let refused = dtype.scalar(Scalar::from(*typo)).unwrap_err().to_string();
        assert!(refused.contains("check digit"), "{refused}");
        let refused = dtype.scalar(Scalar::from(*short)).unwrap_err().to_string();
        assert!(refused.contains("characters"), "{refused}");
        // The wire shape is the identity over the text, and lower case on
        // the wire reads as the identifier it spells.
        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, format!(r#"{{"type":"{name}","value":"{sample}"}}"#));
        assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);
        let folded = format!(r#"{{"type":"{name}","value":"{lower}"}}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&folded).unwrap(), value);
        let broken = format!(r#"{{"type":"{name}","value":"{typo}"}}"#);
        assert!(serde_json::from_str::<Scalar>(&broken).is_err(), "{name}");

        // Arrow: the text storage under the code's own extension name, and
        // the identity back from it.
        let field = Field::new(*name, dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            arrow.metadata()["ARROW:extension:name"],
            format!("yggdryl.{name}")
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field);

        // The packing is at the code's own width, and there is no
        // vocabulary to prebuild for an open identifier space.
        assert_eq!(
            dtype.ascii_packed(sample.as_bytes()).unwrap(),
            DataType::fixed_ascii(*width as u32)
                .unwrap()
                .ascii_packed(sample.as_bytes())
                .unwrap()
        );
        assert!(yggdryl::StringEnum::prebuilt_values(name).is_empty());
        assert!(
            yggdryl::StringEnum::from_logical_name(name)
                .unwrap()
                .is_empty()
        );
        // No neutral member: the empty text is not an identifier, so the
        // default is refused rather than invented.
        assert!(dtype.default_value().is_err(), "{name}");
    }
}

#[test]
fn a_cast_into_an_identifier_holds_the_column_to_the_canonical_spelling() {
    for (name, dtype, _, sample, lower, typo, _) in &IDENTIFIERS {
        let field = Field::new("sid", dtype.clone(), true);
        // A column already holding the canonical spelling is the storage
        // itself, and reading it back is the identifier.
        let stored = field
            .cast_arrow_array(text(&[sample]), ArrowCastOptions::new().with_safe(false))
            .unwrap();
        assert_eq!(cells(&stored), vec![Some(*sample)], "{name}");

        // Strict: a typo and a lower-case spelling are refused, naming the
        // row, the column and the rule. A column's bytes are what every
        // reader digests, so the cast lets in the canonical spelling only.
        for refused in [typo, lower] {
            let message = field
                .cast_arrow_array(
                    text(&[sample, refused]),
                    ArrowCastOptions::new().with_safe(false),
                )
                .unwrap_err()
                .to_string();
            assert!(message.contains("canonical spelling"), "{name}: {message}");
            assert!(message.contains("row 1 of column sid"), "{name}: {message}");
        }

        // Safe: the refused cell is null and the rest of the column stands.
        let safe = field
            .cast_arrow_array(
                text(&[sample, typo, lower]),
                ArrowCastOptions::new().with_safe(true),
            )
            .unwrap();
        assert_eq!(cells(&safe), vec![Some(*sample), None, None], "{name}");

        // A fixed-width slot is still a spelling the cast reads, with the
        // slot's padding trimmed.
        let width = i32::try_from(sample.len() + 2).unwrap();
        let padded = format!("{sample}\0\0");
        let slots: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                [Some(padded.as_bytes())].into_iter(),
                width,
            )
            .unwrap(),
        );
        let trimmed = field
            .cast_arrow_array(slots, ArrowCastOptions::new().with_safe(false))
            .unwrap();
        assert_eq!(cells(&trimmed), vec![Some(*sample)]);

        // And back out to text, which is what the column already holds.
        let row = root([Field::new("sid", dtype.clone(), false)]);
        let batch = RecordBatch::try_new(row.into_arrow_schema().unwrap(), vec![stored]).unwrap();
        let as_text = root([DataType::utf8().required_field("sid")]);
        let read = as_text
            .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
            .unwrap();
        assert_eq!(cells(read.column(0)), vec![Some(*sample)], "{name}");
    }
}

#[test]
fn an_expression_cast_answers_the_identifier_and_try_cast_answers_null() {
    let schema = root([DataType::utf8().required_field("sid")]);
    let batch = |values: Vec<&str>| {
        RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(values))],
        )
        .unwrap()
    };
    for (name, _, _, sample, lower, typo, _) in &IDENTIFIERS {
        // The strict cast at the column tier holds to the canonical
        // spelling, exactly as the ISIN cast does: a typo refuses the
        // whole column, and so does a value the width does not fit.
        let strict = format!("cast(sid as {name}) = '{sample}'")
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(strict.filter(&batch(vec![sample])).unwrap().num_rows(), 1);
        let message = strict
            .filter(&batch(vec![sample, typo]))
            .unwrap_err()
            .to_string();
        assert!(message.contains("canonical spelling"), "{name}: {message}");
        assert!(
            strict.filter(&batch(vec![sample, "HIGH_TOUCH"])).is_err(),
            "{name}"
        );
        // The row tier reads a value as the scalar does, folding the case.
        assert!(
            strict
                .matches(&Scalar::from_sequence([Scalar::from(*lower)]))
                .unwrap()
        );

        // The safe cast is what a derivation asks: an identifier the check
        // digit closes answers, and anything else is null rather than a
        // refusal, at both tiers.
        let safe = format!("try_cast(sid as {name}) is not null")
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let kept = safe
            .filter(&batch(vec![sample, typo, "HIGH_TOUCH", lower]))
            .unwrap();
        assert_eq!(cells(kept.column(0)), vec![Some(*sample)], "{name}");
        assert!(
            safe.matches(&Scalar::from_sequence([Scalar::from(*lower)]))
                .unwrap(),
            "{name}"
        );
        assert!(
            !safe
                .matches(&Scalar::from_sequence([Scalar::from(*typo)]))
                .unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_identifiers_are_appended_after_every_earlier_datatype() {
    // The discriminant is a wire contract, so both are appended after the
    // last identifier stated before them and never renumbered.
    assert_eq!(DataTypeId::Cusip.as_u8(), 64);
    assert_eq!(DataTypeId::Sedol.as_u8(), 65);
    assert_eq!(DataTypeId::Bloomberg.as_u8(), 66);
    assert_eq!(
        &DataTypeId::ALL[DataTypeId::ALL.len() - 4..],
        &[
            DataTypeId::MediaType,
            DataTypeId::Cusip,
            DataTypeId::Sedol,
            DataTypeId::Bloomberg
        ]
    );
    // The datatype order is total and appends too, so no earlier pair
    // moved: every code stated before them sorts before them, and the
    // last datatype before them sorts before them as well.
    assert!(DataType::Isin < DataType::Cusip);
    assert!(DataType::Cusip < DataType::Sedol);
    assert!(DataType::MediaType < DataType::Cusip);
    assert!(DataType::TimeInForce < DataType::Cusip);
    let mut shuffled = [
        DataType::Sedol,
        DataType::Isin,
        DataType::Cusip,
        DataType::Country,
        DataType::MediaType,
    ];
    shuffled.sort();
    assert_eq!(
        shuffled,
        [
            DataType::Country,
            DataType::Isin,
            DataType::MediaType,
            DataType::Cusip,
            DataType::Sedol,
        ]
    );

    // A value carries its identity first: a CUSIP and a SEDOL never
    // compare equal, and neither is the string of its characters.
    let cusip = DataType::Cusip.scalar(Scalar::from("037833100")).unwrap();
    let sedol = DataType::Sedol.scalar(Scalar::from("B0YBKJ7")).unwrap();
    assert_ne!(cusip, sedol);
    assert_ne!(cusip.cmp(&sedol), std::cmp::Ordering::Equal);
    assert_ne!(cusip, Scalar::from("037833100"));
    assert_eq!(cusip.id(), DataTypeId::Cusip);
    assert_eq!(sedol.id(), DataTypeId::Sedol);
    // Values of one identifier order by their text.
    assert!(
        DataType::Cusip.scalar(Scalar::from("037833100")).unwrap()
            < DataType::Cusip.scalar(Scalar::from("38259P508")).unwrap()
    );
}

#[test]
fn the_typed_fields_name_their_identifier() {
    let cusip: CusipField = CusipField::new("cusip", true);
    assert_eq!(cusip.as_field().dtype(), &DataType::Cusip);
    let sedol: SedolField = SedolField::new("sedol", false);
    assert_eq!(sedol.as_field().dtype(), &DataType::Sedol);
    assert!(!sedol.as_field().is_nullable());
    // A shared field is kept for each, as for every parameter-free leaf.
    for dtype in [DataType::Cusip, DataType::Sedol] {
        let shared = dtype.shared_field().unwrap();
        assert_eq!(shared.dtype(), &dtype);
        assert!(std::ptr::eq(shared, dtype.shared_field().unwrap()));
    }
}
