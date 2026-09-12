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
use yggdryl::types::{CfiField, Code, CountryField, CurrencyField, MicField};
use yggdryl::{
    ArrowCast, ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar,
    StringEnum,
};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

fn text(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// The eight codes, each with its fixed width and one value its standard names.
const CODED: [(&str, DataType, usize, &str); 8] = [
    ("country", DataType::Country, 2, "US"),
    ("currency", DataType::Currency, 3, "USD"),
    ("mic", DataType::Mic, 4, "XPAR"),
    ("cfi", DataType::Cfi, 6, "ESVUFR"),
    ("isin", DataType::Isin, 12, "US0378331005"),
    ("side", DataType::Side, 4, "1"),
    ("state", DataType::State, 10, "20NEW"),
    ("timeinforce", DataType::TimeInForce, 8, "0"),
];

#[test]
fn each_coded_datatype_answers_every_invariant_a_wildcard_would_get_wrong() {
    for (name, dtype, width, sample) in &CODED {
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
        // A code is an identity with a storage, not a string with a charset.
        assert_eq!(dtype.id().as_str(), *name, "{name}");
        assert_eq!(dtype.kind(), DataTypeKind::Code, "{name}");
        assert_eq!(dtype.fixed_byte_width(), Some(*width), "{name}");
        assert_eq!(dtype.id().fixed_byte_width(), Some(*width), "{name}");
        assert!(dtype.is_code(), "{name}");
        assert!(dtype.id().is_string(), "{name}");
        assert!(!dtype.is_string(), "{name}");
        assert_eq!(dtype.string_parameters(), None, "{name}");
        assert_eq!(dtype.charset(), None, "{name}");
        assert!(DataTypeId::ALL.contains(&dtype.id()), "{name}");
        assert!(
            DataType::CODES
                .iter()
                .any(|(held, listed, listed_width)| held == name
                    && listed == dtype
                    && listed_width == width),
            "{name}"
        );

        // Nestedness: a registry places it in the primitive half.
        assert!(!dtype.is_nested(), "{name}");
        assert!(!dtype.id().is_parameterized(), "{name}");

        // Default: what it answers is a value of the datatype, or it answers
        // none at all - never a value the datatype itself refuses.
        if let Ok(default) = dtype.default_value() {
            assert!(!default.is_null(), "{name}");
            dtype
                .scalar(default)
                .unwrap_or_else(|error| panic!("{name} refuses its own default: {error}"));
        }

        // Serde: the serialized shape is the canonical spelling, and it
        // round-trips.
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(json, format!(r#"{{"type":"{name}"}}"#), "{name}");
        assert_eq!(DataType::from_json(&json).unwrap(), *dtype, "{name}");
        let rendered = serde_json::to_string(dtype).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            *dtype,
            "{name}"
        );

        // A value is the code leaf under its own identity, and its wire
        // shape is that identity's name over the text.
        let value = dtype.scalar(Scalar::from(*sample)).unwrap();
        assert!(matches!(value, Scalar::Code(_)), "{name}");
        assert_eq!(value.id(), dtype.id(), "{name}");
        assert_eq!(value.kind(), *name, "{name}");
        assert_eq!(value.as_str(), Some(*sample), "{name}");
        assert_eq!(value.dtype().unwrap(), *dtype, "{name}");
        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(
            wire,
            format!(r#"{{"type":"{name}","value":"{sample}"}}"#),
            "{name}"
        );
        assert_eq!(
            serde_json::from_str::<Scalar>(&wire).unwrap(),
            value,
            "{name}"
        );

        // Arrow: one type and back, losslessly, through a field.
        let field = Field::new(*name, dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{name}");

        // Merge and compatibility: with itself is itself; a number's
        // rendering does not fit a code, so absorbing one is no less than
        // `utf8`; a nested datatype refuses naming both.
        assert_eq!(dtype.merge_with(dtype, false).unwrap(), *dtype, "{name}");
        assert_eq!(
            dtype.merge_with(&DataType::Int64, false).unwrap(),
            DataType::utf8(),
            "{name}"
        );
        let refused = dtype
            .merge_with(
                &DataType::list(DataType::Int64.nullable_field("item")),
                false,
            )
            .unwrap_err();
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
    assert!(matches!(side, Scalar::Code(Code::Side(_))));
    assert_eq!(side.as_str(), Some("1"));
    assert_eq!(DataType::Side.scalar(side.clone()).unwrap(), side);

    // Packing is the crate's fixed-ASCII packing at the fixed width:
    // NUL-padded up to it, the padding gone on the way back.
    assert_eq!(
        DataType::Side.ascii_packed(b"1").unwrap(),
        DataType::fixed_ascii(4)
            .unwrap()
            .ascii_packed(b"1")
            .unwrap()
    );
    for (dtype, value) in [(DataType::Side, "1"), (DataType::Side, "2")] {
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
    let as_text = root([DataType::utf8().required_field("venue")]);
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
    for (dtype, outside) in [(DataType::Side, "Z"), (DataType::TimeInForce, "X")] {
        let stored = dtype.scalar(Scalar::from(outside)).unwrap();
        assert_eq!(stored.as_str(), Some(outside), "{dtype}");
        let packed = dtype.ascii_packed(outside.as_bytes()).unwrap();
        assert_eq!(dtype.ascii_value(packed).unwrap().as_str(), outside);
    }

    // The listing is what a name resolves from, and two readers answer the
    // same members because it is a constant.
    for (name, count) in [
        ("side", StringEnum::SIDES.len()),
        ("timeinforce", StringEnum::TIMESINFORCE.len()),
    ] {
        let built = StringEnum::from_logical_name(name).unwrap();
        assert_eq!(built.len(), count, "{name}");
        assert_eq!(
            built,
            StringEnum::from_logical_name(name).unwrap(),
            "{name}"
        );
    }
    // Every prebuilt member fits the width its own datatype fixes.
    for (name, dtype) in [
        ("side", DataType::Side),
        ("timeinforce", DataType::TimeInForce),
    ] {
        StringEnum::from_logical_name(name)
            .unwrap()
            .into_members(&dtype)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
    // A row whose line does not say a side has none, and the crate already
    // spells "no answer" one way.
    assert!(!StringEnum::SIDES.contains(&"UNKNOWN"));
    assert!(!StringEnum::SIDES.contains(&"NONE"));

    let field = Field::new("side", DataType::Side, true);
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
        DataType::from_fields([Field::new("side", DataType::Side, false)]).unwrap(),
        false,
    );
    assert!(
        required
            .validate_value(&Scalar::from_sequence([Scalar::Null]))
            .is_err()
    );
}

#[test]
fn a_code_carries_its_identity_into_equality_and_order() {
    // Two codes whose bytes agree are two values: the identity compares
    // first, then the text, so a side and a time in force never collide in
    // a set or sort beside each other.
    let side = DataType::Side.scalar(Scalar::from("1")).unwrap();
    let tif = DataType::TimeInForce.scalar(Scalar::from("1")).unwrap();
    assert_eq!(side.as_str(), tif.as_str());
    assert_ne!(side, tif);
    assert_ne!(side.cmp(&tif), std::cmp::Ordering::Equal);
    assert_eq!(side, DataType::Side.scalar(Scalar::from("1")).unwrap());

    // And a code is not the string of the same characters.
    assert_ne!(side, Scalar::from("1"));
    assert_ne!(
        DataType::Currency.scalar(Scalar::from("USD")).unwrap(),
        DataType::fixed_ascii(3).unwrap().scalar("USD").unwrap()
    );
}

#[test]
fn a_coded_column_casts_to_text_and_back_and_refuses_a_number() {
    for (_, dtype, _, value) in &CODED {
        let stored = dtype.scalar(Scalar::from(*value)).unwrap();
        // To text, which is what the value already is.
        let text = DataType::utf8()
            .scalar(Scalar::from(stored.as_str().unwrap()))
            .unwrap();
        assert_eq!(text.as_str(), Some(*value));
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
fn every_code_stores_the_width_its_standard_fixes() {
    for (name, dtype, width, sample) in &CODED {
        let field = Field::new("code", dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();

        assert_eq!(
            arrow.data_type(),
            &ArrowDataType::FixedSizeBinary(i32::try_from(*width).unwrap()),
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

        // A value is stored padded to the width and read back trimmed, and
        // text cast into the column becomes the same cell.
        let value = dtype.scalar(Scalar::from(*sample)).unwrap();
        let stored = scalar_array(&field, &value).unwrap();
        let bytes = stored
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .unwrap();
        assert_eq!(
            bytes.value_length(),
            i32::try_from(*width).unwrap(),
            "{name}"
        );
        let mut padded = sample.as_bytes().to_vec();
        padded.resize(*width, 0);
        assert_eq!(bytes.value(0), padded.as_slice(), "{name}");
        assert_eq!(
            scalar_value(&field, stored.as_ref()).unwrap(),
            value,
            "{name}"
        );
        let cast = field
            .cast_arrow_array(text(&[sample]), ArrowCastOptions::new().with_safe(false))
            .unwrap();
        assert_eq!(cast.as_ref(), stored.as_ref(), "{name}");

        // And the column's own storage ingests without re-padding or
        // refusing: the plan asks `is_code`, so a code added to the listing
        // is planned without being named again.
        let again = field
            .cast_arrow_array(
                Arc::clone(&stored),
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap_or_else(|error| panic!("{name} did not ingest its own bytes: {error}"));
        assert_eq!(again.as_ref(), stored.as_ref(), "{name}");

        // A value past the width is refused at the code's own width, naming
        // the row it was in - one refusal shape for all nine, where the four
        // FIX codes used to fall to Arrow's builder complaint instead.
        let over = "X".repeat(*width + 1);
        let refused = field
            .cast_arrow_array(
                text(&[over.as_str()]),
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains(&format!("at most {width} bytes, got {} bytes", over.len())),
            "{name}: {refused}"
        );
        assert!(
            refused.contains("row 0 of column code"),
            "{name}: {refused}"
        );
    }
}

#[test]
fn a_code_and_the_width_that_holds_it_are_not_the_same_column() {
    let currency = Field::new("ccy", DataType::Currency, false);
    let ascii24 = Field::new("ccy", DataType::fixed_ascii(3).unwrap(), false);

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
        &DataType::fixed_size_binary(3).unwrap()
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
        &DataType::fixed_size_binary(4).unwrap()
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
    assert_eq!(
        DataType::from_str("fixed_ascii(6)").unwrap(),
        DataType::fixed_ascii(6).unwrap()
    );
    assert_ne!(DataType::Cfi, DataType::fixed_ascii(6).unwrap());
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

    // The pairing is the field's value contract, so the text becomes the code
    // leaf on the way in.
    let value = FieldScalar::new(ccy.as_field(), "USD").unwrap();
    assert_eq!(value.dtype(), &DataType::Currency);
    assert_eq!(value.name(), "ccy");
    assert_eq!(value.as_str(), Some("USD"));
    assert_eq!(value.value().id(), DataTypeId::Currency);

    // The marker checks the datatype, so a width is not a code.
    let plain = Field::new("ccy", DataType::fixed_ascii(3).unwrap(), false);
    assert!(
        plain
            .try_into_typed::<yggdryl::types::CurrencyType>()
            .is_err()
    );
    assert!(FieldScalar::new(venue.as_field(), "XPARIS").is_err());
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
        DataType::dictionary(DataType::Int32, DataType::fixed_ascii(3).unwrap()).unwrap(),
        false,
    );
    assert_eq!(
        Field::from_arrow(&width.clone().into_arrow().unwrap()).unwrap(),
        width
    );
    let plain = Field::new(
        "ccy",
        DataType::dictionary(DataType::Int32, DataType::fixed_size_binary(3).unwrap()).unwrap(),
        false,
    );
    assert_eq!(
        Field::from_arrow(&plain.clone().into_arrow().unwrap()).unwrap(),
        plain
    );
}

#[test]
fn a_state_sorts_from_the_first_state_to_the_terminal_ones() {
    use yggdryl::types::State;

    // The stored bytes, sorted by nothing but ASCII. This is the whole claim:
    // whatever sorts the column - a Parquet row group's bounds, an external
    // sort, an ORDER BY in something that never heard of this crate - puts
    // every live state before every ended one.
    let mut held: Vec<&str> = yggdryl::StringEnum::STATES.to_vec();
    held.sort_unstable();
    assert_eq!(
        held.as_slice(),
        yggdryl::StringEnum::STATES,
        "the vocabulary is declared in the order it sorts",
    );

    let ordered = [
        "10PENDING",
        "20NEW",
        "40PARTFILL",
        "60PENDCXL",
        "80FILLED",
        "90CANCELED",
        "95REJECTED",
    ];
    let mut shuffled = [
        "95REJECTED",
        "80FILLED",
        "20NEW",
        "60PENDCXL",
        "10PENDING",
        "90CANCELED",
        "40PARTFILL",
    ];
    shuffled.sort_unstable();
    assert_eq!(shuffled, ordered);

    // The rank is the two leading digits, read as the number they spell.
    for (held, rank) in [
        ("00UNKNOWN", 0),
        ("10PENDING", 10),
        ("40PARTFILL", 40),
        ("80FILLED", 80),
        ("90CANCELED", 90),
        ("95REJECTED", 95),
    ] {
        assert_eq!(State::new(held).unwrap().rank(), Some(rank), "{held}");
    }

    // Every ending is told apart from every other without reading a name.
    for held in [
        "10PENDING",
        "20NEW",
        "40PARTFILL",
        "60PENDCXL",
        "70REPLACED",
        "70RESTATED",
    ] {
        assert!(State::new(held).unwrap().is_live(), "{held}");
    }
    assert!(State::new("80FILLED").unwrap().is_done());
    assert!(State::new("90CANCELED").unwrap().is_cancelled());
    assert!(State::new("95REJECTED").unwrap().is_failed());
    for held in ["80FILLED", "90CANCELED", "95REJECTED"] {
        assert!(!State::new(held).unwrap().is_live(), "{held}");
    }

    // The digits between two shipped ranks are placeholders: a state that
    // belongs between them takes one, and the predicates read the band it
    // falls in rather than the exact rank.
    let between = State::new("85ARCHIVED").unwrap();
    assert_eq!(between.rank(), Some(85));
    assert!(between.is_done());
    assert!(!between.is_live());
    assert!(State::new("92HALTED").unwrap().is_cancelled());
    assert!(State::new("97ABORTED").unwrap().is_failed());

    // A value that opens with anything but two digits has no rank, and so is
    // neither live nor ended.
    let unranked = State::new("FILLED").unwrap();
    assert_eq!(unranked.rank(), None);
    assert!(!unranked.is_live());
    assert!(!unranked.is_done());
    assert_eq!(State::new("8FILLED").unwrap().rank(), None);
}

#[test]
fn a_state_answers_a_fix_code_a_fix_name_and_a_scheduler_word_alike() {
    use yggdryl::types::State;

    // One value, four vocabularies: the wire code an ExecutionReport carries,
    // the specification's name for it, the word a scheduler uses, and the
    // short name a FIX bridge logs.
    for (spelling, expected) in [
        ("0", "20NEW"),
        ("1", "40PARTFILL"),
        ("2", "80FILLED"),
        ("8", "95REJECTED"),
        ("F", "40TRADE"),
        ("New", "20NEW"),
        ("PartiallyFilled", "40PARTFILL"),
        ("DoneForDay", "80DONEDAY"),
        ("done_for_day", "80DONEDAY"),
        ("DONE FOR DAY", "80DONEDAY"),
        ("running", "30RUNNING"),
        ("succeeded", "80SUCCESS"),
        ("timed out", "95TIMEOUT"),
        ("failed", "95FAILED"),
        // The short names a FIX bridge logs fold to the same states.
        ("PartFill", "40PARTFILL"),
        ("PartFilled", "40PARTFILL"),
        ("PendNew", "10PENDNEW"),
        ("PendCancel", "60PENDCXL"),
        ("PendReplace", "60PENDRPL"),
        ("DoneDay", "80DONEDAY"),
        ("Cancel", "90CANCELED"),
        ("Reject", "95REJECTED"),
        // A stored value names itself, so resolving one twice is resolving it
        // once.
        ("80FILLED", "80FILLED"),
    ] {
        let held =
            State::from_spelling(spelling).unwrap_or_else(|| panic!("{spelling} names no state"));
        assert_eq!(held.as_str(), expected, "{spelling}");
        assert_eq!(
            State::from_spelling(held.as_str()).unwrap().as_str(),
            expected,
            "{spelling} resolves to itself",
        );
    }

    // A wire code never folds: `A` is PendingNew and `a` is not a code at all.
    assert_eq!(State::from_spelling("A").unwrap().as_str(), "10PENDNEW");
    assert_eq!(State::from_spelling("a"), None);
    assert_eq!(State::from_spelling("whatever"), None);
    assert_eq!(State::from_spelling(""), None);
}

#[test]
fn the_state_and_time_in_force_codes_are_ordinary_datatypes_everywhere_else() {
    for (name, dtype, width) in [
        ("state", DataType::State, 10),
        ("timeinforce", DataType::TimeInForce, 8),
    ] {
        // Parsed, displayed and round-tripped by the grammar like any other.
        assert_eq!(DataType::from_str(name).unwrap(), dtype);
        assert_eq!(dtype.to_string(), name);
        assert_eq!(dtype.kind(), DataTypeKind::Code);
        assert!(dtype.is_code());
        assert_eq!(dtype.fixed_byte_width(), Some(width));

        // And it crosses Arrow as the fixed width it is, extension name and
        // all, so a column round-trips without becoming plain bytes.
        let field = Field::new(name, dtype.clone(), true);
        let recovered = Field::from_arrow(&field.clone().into_arrow().unwrap()).unwrap();
        assert_eq!(recovered, field);
    }

    // A value wider than the storage is refused by the datatype rather than
    // truncated into something that reads.
    assert!(DataType::State.scalar(Scalar::from("20NEW")).is_ok());
    assert!(DataType::State.scalar(Scalar::from("40PARTFILL")).is_ok());
    assert!(
        DataType::State
            .scalar(Scalar::from("80CALCULATED"))
            .is_err()
    );
    assert!(DataType::TimeInForce.scalar(Scalar::from("0")).is_ok());
}
