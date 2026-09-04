//! A registered code keeps its identity across Arrow and its width in storage.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::types::{CfiField, CountryField, CurrencyField, MicField};
use yggdryl::types::{CurrencyScalar, MicScalar};
use yggdryl::{ArrowCast, DataType, Field, Scalar};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

fn text(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
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
fn a_cast_into_a_code_pads_and_reading_it_back_trims() {
    let venue = Field::new("venue", DataType::Mic, false);
    let padded = venue
        .cast_arrow_array(text(&["XPAR", "XLON"]), false)
        .unwrap();
    let bytes = padded
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(bytes.value_length(), 4);
    assert_eq!(bytes.value(0), b"XPAR");

    // A shorter value pads; the column read under `utf8` trims it back.
    let short = venue.cast_arrow_array(text(&["BX"]), false).unwrap();
    let short = short
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(short.value(0), b"BX\0\0");

    let row = root([venue.clone()]);
    let batch = RecordBatch::try_new(row.into_arrow_schema().unwrap(), vec![padded]).unwrap();
    let as_text = root([DataType::Utf8.required_field("venue")]);
    let trimmed = as_text.cast_arrow_batch(batch, false).unwrap();
    let trimmed = trimmed
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(trimmed.value(0), "XPAR");
    assert_eq!(trimmed.value(1), "XLON");

    // The refusal names the code's own width, not the next ASCII one up.
    let refused = venue
        .cast_arrow_array(text(&["XPARIS"]), false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
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
        Scalar::from("ESVUFR")
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
            .try_into_typed::<yggdryl::types::ascii::Currency>()
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
