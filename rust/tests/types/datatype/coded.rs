//! The coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::types::{AsciiFamily, CfiField, CountryField, CurrencyField, MicField};
use yggdryl::types::{CurrencyScalar, MicScalar};
use yggdryl::{
    ArrowCast, ArrowCastOptions, AsciiEnum, DataType, DataTypeId, DataTypeKind, Field, Scalar,
};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

fn text(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// Coded datatypes with their fixed widths and published vocabulary.
const CODED: [(&str, DataType, i32); 2] = [
    ("side", DataType::Side, 4),
    ("msgdirection", DataType::MsgDirection, 4),
];

#[test]
fn each_coded_datatype_answers_every_invariant_a_wildcard_would_get_wrong() {
    for (name, dtype, width) in &CODED {
        // Naming: one canonical spelling, and the grammar round-trips it.
        assert_eq!(dtype.to_string(), *name, "{name}");
        assert_eq!(DataType::from_str(name).unwrap(), *dtype, "{name}");
        assert_eq!(dtype.name(), *name, "{name}");
        assert_eq!(dtype.code_name(), Some(*name), "{name}");
        // The fold reaches it, exactly as it reaches every other name.
        assert_eq!(DataType::from_logical_name(name).unwrap(), *dtype, "{name}");
        assert_eq!(
            DataType::from_str(&name.to_uppercase()).unwrap(),
            *dtype,
            "{name}"
        );

        // Identity: the discriminant, the family, the width, and the listing.
        assert_eq!(dtype.id().as_str(), *name, "{name}");
        assert_eq!(dtype.kind(), DataTypeKind::Ascii, "{name}");
        assert_eq!(dtype.ascii_width(), Some(*width), "{name}");
        assert_eq!(
            dtype.id().fixed_byte_width(),
            usize::try_from(*width).ok(),
            "{name}"
        );
        assert!(dtype.is_code(), "{name}");
        assert!(dtype.is_ascii(), "{name}");
        assert!(DataTypeId::ALL.contains(&dtype.id()), "{name}");
        assert!(
            DataType::CODES.iter().any(|(held, _, _)| held == name),
            "{name}"
        );

        // Nestedness: a registry places it in the primitive half.
        assert!(!dtype.is_nested(), "{name}");
        assert!(!dtype.id().is_parameterized(), "{name}");

        // Default: it answers one rather than falling through to one.
        let default = dtype.default_value().unwrap();
        assert!(!default.is_null(), "{name}");
        dtype.scalar(default.clone()).unwrap();

        // Serde: the serialized shape is the canonical spelling, and it
        // round-trips.
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(DataType::from_json(&json).unwrap(), *dtype, "{name}");
        let rendered = serde_json::to_string(dtype).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            *dtype,
            "{name}"
        );

        // Arrow: one type and back, losslessly, through a field.
        let field = Field::new(*name, dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{name}");

        // Merge and compatibility: with itself is itself, and a foreign
        // datatype refuses naming both.
        assert_eq!(dtype.merge_with(dtype, false).unwrap(), *dtype, "{name}");
        let refused = dtype.merge_with(&DataType::Int64, false).unwrap_err();
        let message = refused.to_string();
        assert!(message.contains(name), "{message}");
        assert!(message.contains("int64"), "{message}");
    }
}

#[test]
fn a_coded_value_is_checked_rewritten_and_packed_at_its_own_width() {
    // The value contract accepts the text, rewrites it into the declared
    // representation, and answers an unchanged value untouched.
    let side = DataType::Side.scalar(Scalar::from("1")).unwrap();
    assert!(matches!(side, Scalar::Ascii(AsciiFamily::Side(_))));
    assert_eq!(side.as_str(), Some("1"));
    assert_eq!(DataType::Side.scalar(side.clone()).unwrap(), side);

    // Packing is the crate's existing fixed-ASCII packing at the fixed width:
    // NUL-padded up to it, the padding gone on the way back.
    assert_eq!(
        DataType::Side.ascii_packed(b"1").unwrap(),
        DataType::FixedAscii(4).ascii_packed(b"1").unwrap()
    );
    for (dtype, value) in [
        (DataType::Side, "1"),
        (DataType::MsgDirection, "SENT"),
        (DataType::MsgDirection, "RECV"),
    ] {
        let packed = dtype.ascii_packed(value.as_bytes()).unwrap();
        let read = dtype.ascii_value(packed).unwrap();
        assert_eq!(read.as_str(), value, "{dtype} {value}");
    }

    // A value longer than the width is the refusal any fixed-ASCII field
    // gives, and it names the type.
    let refused = DataType::Side.scalar(Scalar::from("TOOLONG")).unwrap_err();
    assert!(refused.to_string().contains("4 bytes"), "{refused}");
}

#[test]
fn a_cast_into_a_code_pads_and_reading_it_back_trims() {
    let venue = Field::new("venue", DataType::Mic, false);
    let padded = venue
        .cast_arrow_array(
            text(&["XPAR", "XLON"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    let bytes = padded
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(bytes.value_length(), 4);
    assert_eq!(bytes.value(0), b"XPAR");

    // A shorter value pads; the column read under `utf8` trims it back.
    let short = venue
        .cast_arrow_array(text(&["BX"]), ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let short = short
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(short.value(0), b"BX\0\0");

    let row = root([venue.clone()]);
    let batch = RecordBatch::try_new(row.into_arrow_schema().unwrap(), vec![padded]).unwrap();
    let as_text = root([DataType::Utf8.required_field("venue")]);
    let trimmed = as_text
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let trimmed = trimmed
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(trimmed.value(0), "XPAR");
    assert_eq!(trimmed.value(1), "XLON");

    // The refusal names the code's own width, not the next ASCII one up.
    let refused = venue
        .cast_arrow_array(text(&["XPARIS"]), ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
}

#[test]
fn a_listing_is_a_vocabulary_and_never_a_gate_on_the_value() {
    // These declare a vocabulary exactly as `Mic` does: a venue's own message
    // type, and a side no version defines, are held rather than refused.
    for (dtype, outside) in [(DataType::Side, "Z"), (DataType::MsgDirection, "BOTH")] {
        let stored = dtype.scalar(Scalar::from(outside)).unwrap();
        assert_eq!(stored.as_str(), Some(outside), "{dtype}");
        let packed = dtype.ascii_packed(outside.as_bytes()).unwrap();
        assert_eq!(dtype.ascii_value(packed).unwrap().as_str(), outside);
    }

    // The listing is what a name resolves from, and two readers answer the
    // same members because it is a constant.
    for (name, count) in [
        ("side", AsciiEnum::SIDES.len()),
        ("msgdirection", AsciiEnum::DIRECTIONS.len()),
    ] {
        let built = AsciiEnum::from_logical_name(name).unwrap();
        assert_eq!(built.len(), count, "{name}");
        assert_eq!(built, AsciiEnum::from_logical_name(name).unwrap(), "{name}");
    }
    assert_eq!(AsciiEnum::DIRECTIONS, &["RECV", "SENT"][..]);
    // Every prebuilt member fits the width its own datatype fixes.
    for (name, dtype) in [
        ("side", DataType::Side),
        ("msgdirection", DataType::MsgDirection),
    ] {
        AsciiEnum::from_logical_name(name)
            .unwrap()
            .into_members(&dtype)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
    // A row whose line does not say which way it moved has no direction, and
    // the crate already spells "no answer" one way.
    assert!(!AsciiEnum::DIRECTIONS.contains(&"UNKNOWN"));
    assert!(!AsciiEnum::DIRECTIONS.contains(&"NONE"));

    let field = Field::new("direction", DataType::MsgDirection, true);
    let row = Field::new(
        "row",
        DataType::from_fields([field.clone()]).unwrap(),
        false,
    );
    let value = row
        .canonicalize_value(Scalar::from_sequence([Scalar::Null]))
        .unwrap();
    row.validate_value(&value).unwrap();
    assert!(value.as_sequence().unwrap()[0].is_null());
    // A required one refuses the same null, so the nullability is the field's
    // and not the datatype's.
    let required = Field::new(
        "row",
        DataType::from_fields([Field::new("direction", DataType::MsgDirection, false)]).unwrap(),
        false,
    );
    assert!(
        required
            .validate_value(&Scalar::from_sequence([Scalar::Null]))
            .is_err()
    );
}

#[test]
fn a_coded_column_casts_to_text_and_back_and_refuses_a_number() {
    for (dtype, value) in [(DataType::Side, "1"), (DataType::MsgDirection, "SENT")] {
        let stored = dtype.scalar(Scalar::from(value)).unwrap();
        // To text, which is what the value already is.
        let text = DataType::Utf8
            .scalar(Scalar::from(stored.as_str().unwrap()))
            .unwrap();
        assert_eq!(text.as_str(), Some(value));
        // And back, through the same contract.
        assert_eq!(dtype.scalar(text.clone()).unwrap(), stored);
        // A number is not one of these, and the refusal names the type.
        let refused = dtype.scalar(Scalar::from(7_i64)).unwrap_err();
        assert!(
            refused.to_string().contains(&dtype.to_string()),
            "{refused}"
        );
    }
}

#[test]
fn a_direction_is_the_verb_in_front_of_the_payload_and_nothing_else() {
    use yggdryl::types::MsgDirection;

    // Read, with the marker taken off the body.
    for (line, direction, body) in [
        (
            "sending >> 8=FIX.4.2|9=176|35=D|10=203|",
            Some(MsgDirection::SENT),
            ">> 8=FIX.4.2|9=176|35=D|10=203|",
        ),
        (
            "recv 8=FIX.4.4|35=0|10=017|",
            Some(MsgDirection::RECV),
            "8=FIX.4.4|35=0|10=017|",
        ),
        (
            "Receiving XmlApi: <Execution ExecID='E1'/>",
            Some(MsgDirection::RECV),
            "XmlApi: <Execution ExecID='E1'/>",
        ),
        (
            "[OUT] 8=FIX.4.4|35=D|",
            Some(MsgDirection::SENT),
            "8=FIX.4.4|35=D|",
        ),
        ("(in) ACCOUNT=A1", Some(MsgDirection::RECV), "ACCOUNT=A1"),
    ] {
        assert_eq!(MsgDirection::infer_text(line), direction, "{line}");
        assert_eq!(MsgDirection::split_text(line).1, body, "{line}");
    }

    // Nothing read is nothing removed, and these are the shapes that must
    // read nothing.
    for line in [
        // English that merely contains the letters.
        "sending in session 3",
        "received out of order",
        // A route endpoint and a session name, where a word boundary alone
        // would have been enough to get it wrong.
        "direct:out 8=FIX.4.4|35=D|",
        "MCFID-IN-XPAR 8=FIX.4.4|35=D|",
        // Both verbs in one prefix: none a reading can prefer.
        "sending and receiving 8=FIX.4.4|35=D|",
        // A verb only inside the payload is the payload's word.
        "8=FIX.4.4|35=8|58=sent earlier|10=1|",
        "ACCOUNT=A1|TEXT=received late|",
        // No verb at all.
        "8=FIX.4.4|35=D|",
        "no level printed by this plugin",
    ] {
        assert_eq!(MsgDirection::infer_text(line), None, "{line}");
        assert_eq!(MsgDirection::split_text(line).1, line, "{line}");
    }
}

#[test]
fn every_code_stores_the_width_its_standard_fixes() {
    for (name, dtype, width) in DataType::CODES {
        let field = Field::new("code", dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();

        assert_eq!(
            arrow.data_type(),
            &ArrowDataType::FixedSizeBinary(*width),
            "{name}"
        );
        assert_eq!(
            arrow.metadata()["ARROW:extension:name"],
            format!("yggdryl.{name}"),
            "{name}"
        );
        assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "", "{name}");
        // The identity round-trips: the same bytes come back the same code.
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{name}");
    }
}

#[test]
fn a_code_and_the_width_that_holds_it_are_not_the_same_column() {
    let currency = Field::new("ccy", DataType::Currency, false);
    let ascii24 = Field::new("ccy", DataType::FixedAscii(3), false);

    // Identical storage, different identity, so neither imports as the other.
    let currency_arrow = currency.clone().into_arrow().unwrap();
    let ascii_arrow = ascii24.clone().into_arrow().unwrap();
    assert_eq!(currency_arrow.data_type(), ascii_arrow.data_type());
    assert_ne!(currency_arrow.metadata(), ascii_arrow.metadata());
    assert_eq!(Field::from_arrow(&currency_arrow).unwrap(), currency);
    assert_eq!(Field::from_arrow(&ascii_arrow).unwrap(), ascii24);

    // The same three bytes under no extension at all stay a fixed binary.
    let plain = arrow_schema::Field::new("ccy", ArrowDataType::FixedSizeBinary(3), false);
    assert_eq!(
        Field::from_arrow(&plain).unwrap().dtype(),
        &DataType::FixedSizeBinary(3)
    );

    // A code's own name over the wrong width is not that code either.
    let mismatched = arrow_schema::Field::new("ccy", ArrowDataType::FixedSizeBinary(4), false)
        .with_metadata(
            [
                (
                    "ARROW:extension:name".to_owned(),
                    "yggdryl.currency".to_owned(),
                ),
                ("ARROW:extension:metadata".to_owned(), String::new()),
            ]
            .into_iter()
            .collect(),
        );
    assert_eq!(
        Field::from_arrow(&mismatched).unwrap().dtype(),
        &DataType::FixedSizeBinary(4)
    );
}

#[test]
fn a_cfi_stores_six_bytes_rather_than_padding_into_eight() {
    let cfi = Field::new("classification", DataType::Cfi, false);
    let stored = scalar_array(&cfi, &Scalar::from("ESVUFR")).unwrap();
    let bytes = stored
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();

    assert_eq!(bytes.value_length(), 6);
    assert_eq!(bytes.value(0), b"ESVUFR");
    assert_eq!(
        scalar_value(&cfi, stored.as_ref()).unwrap(),
        DataType::Cfi.scalar(Scalar::from("ESVUFR")).unwrap()
    );
    // A width of six bytes is spellable and is still not a CFI code.
    assert_eq!(DataType::ascii(6).unwrap(), DataType::FixedAscii(6));
    assert_ne!(DataType::Cfi, DataType::FixedAscii(6));
}

#[test]
fn the_typed_field_and_scalar_aliases_name_their_code() {
    let ccy = CurrencyField::new("ccy", false);
    let venue = MicField::new("venue", true);
    let iso = CountryField::new("iso", true);
    let cfi = CfiField::new("classification", true);

    assert_eq!(ccy.as_field().dtype(), &DataType::Currency);
    assert_eq!(venue.as_field().dtype(), &DataType::Mic);
    assert_eq!(iso.as_field().dtype(), &DataType::Country);
    assert_eq!(cfi.as_field().dtype(), &DataType::Cfi);

    let value = CurrencyScalar::new(Scalar::from("USD")).unwrap();
    assert_eq!(value.dtype(), &DataType::Currency);
    assert_eq!(value.value(), &Scalar::from("USD"));

    // The marker checks the datatype, so a width is not a code.
    let plain = Field::new("ccy", DataType::FixedAscii(3), false);
    assert!(
        plain
            .try_into_typed::<yggdryl::types::ascii::CurrencyType>()
            .is_err()
    );
    assert!(MicScalar::new(Scalar::from("XPARIS")).is_err());
}

#[test]
fn a_dictionary_encoded_code_keeps_its_identity_across_arrow() {
    // Arrow's dictionary holds a bare datatype for its values, so the field
    // is the only place the identity can ride - and a low-cardinality code
    // column is exactly the one a writer dictionary-encodes.
    for (name, dtype, _) in DataType::CODES {
        let encoded = DataType::dictionary(DataType::Int32, dtype.clone()).unwrap();
        let field = Field::new("code", encoded.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();

        assert_eq!(
            arrow.metadata()["ARROW:extension:name"],
            format!("yggdryl.{name}"),
            "{name}"
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{name}");
    }

    // The width keeps its own identity the same way, and a dictionary of
    // anonymous bytes stays anonymous.
    let width = Field::new(
        "ccy",
        DataType::dictionary(DataType::Int32, DataType::FixedAscii(3)).unwrap(),
        false,
    );
    assert_eq!(
        Field::from_arrow(&width.clone().into_arrow().unwrap()).unwrap(),
        width
    );
    let plain = Field::new(
        "ccy",
        DataType::dictionary(DataType::Int32, DataType::FixedSizeBinary(3)).unwrap(),
        false,
    );
    assert_eq!(
        Field::from_arrow(&plain.clone().into_arrow().unwrap()).unwrap(),
        plain
    );
}
