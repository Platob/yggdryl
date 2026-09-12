//! US-ASCII string fields and the code fields: typed markers, the string
//! enum, the value door, and the cast plan around storage.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayRef, BinaryArray, DictionaryArray, FixedSizeBinaryArray, Int32Array, RecordBatch,
    StringArray, StringViewArray, StructArray,
};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields, Schema};
use yggdryl::types::string;
use yggdryl::types::{CfiField, CountryField, CurrencyField, MicField, StringField};
use yggdryl::{
    ArrowCast, ArrowCastOptions, DataType, DataTypeId, Field, FieldScalar, Scalar, StringEnum,
};

use super::typed::assert_typed_marker;

#[test]
fn the_string_marker_covers_us_ascii_and_the_code_markers_their_codes() {
    assert_typed_marker::<string::StringType>(DataType::ascii());
    assert_typed_marker::<string::StringType>(DataType::fixed_ascii(4).unwrap());
    assert_typed_marker::<string::StringType>(DataType::fixed_ascii(16).unwrap());
    assert_typed_marker::<string::StringType>(DataType::from_str("ascii(4)").unwrap());
    assert_typed_marker::<string::StringType>(DataType::utf8());
    assert_typed_marker::<string::StringType>(DataType::large_utf8());
    assert_typed_marker::<string::StringType>(DataType::utf8_view());
    assert_typed_marker::<string::StringType>(DataType::from_str("string(windows-1252)").unwrap());
    assert_typed_marker::<string::CountryType>(DataType::Country);
    assert_typed_marker::<string::CurrencyType>(DataType::Currency);
    assert_typed_marker::<string::MicType>(DataType::Mic);
    assert_typed_marker::<string::CfiType>(DataType::Cfi);

    // Every string is one parameterized datatype, so the field takes it
    // through `try_new`; a code is not a string and is refused by name.
    let note = StringField::try_new("note", DataType::ascii(), true).unwrap();
    assert_eq!(note.dtype(), &DataType::ascii());
    let ccy = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), false).unwrap();
    assert_eq!(ccy.dtype(), &DataType::fixed_ascii(4).unwrap());
    assert!(StringField::try_new("ccy", DataType::Currency, false).is_err());

    // The code/width boundary is the one the markers exist for: a currency
    // and a `fixed_ascii(3)` are the same three bytes and are not each other.
    assert_eq!(
        CurrencyField::new("ccy", false).dtype(),
        &DataType::Currency
    );
    assert_eq!(CountryField::new("iso", true).dtype(), &DataType::Country);
    assert_eq!(MicField::new("venue", true).dtype(), &DataType::Mic);
    assert!(CurrencyField::try_new("ccy", DataType::fixed_ascii(3).unwrap(), false).is_err());
    // Six bytes against eight: the confusion a width/code mix-up produces.
    assert!(CfiField::try_new("code", DataType::fixed_ascii(8).unwrap(), false).is_err());

    // The typed value is checked under the one US-ASCII rule for its width.
    let width = StringField::try_new("code", DataType::fixed_ascii(8).unwrap(), false).unwrap();
    let code = FieldScalar::new(width.as_field(), "ABC").unwrap();
    assert_eq!(code.as_str(), Some("ABC"));
    assert_eq!(code.value().id(), DataTypeId::FixedString);
    assert!(FieldScalar::new(width.as_field(), "ABCDEFGHI").is_err());

    // A typed code value is checked at the width its own standard fixes.
    let ccy = CurrencyField::new("ccy", false);
    assert_eq!(
        FieldScalar::new(ccy.as_field(), "USD").unwrap().as_str(),
        Some("USD")
    );
    assert!(FieldScalar::new(ccy.as_field(), "EURO").is_err());
    let cfi = CfiField::new("classification", false);
    assert!(FieldScalar::new(cfi.as_field(), "ESVUFR").is_ok());
}

#[test]
fn the_value_door_judges_the_repertoire_and_the_bound() {
    // The variable layout trims nothing: NUL and a byte above 0x7F are
    // refused, and everything else is the same value `Scalar::from` builds.
    let note = StringField::try_new("note", DataType::ascii(), true).unwrap();
    let held = FieldScalar::new(note.as_field(), "USD").unwrap();
    assert_eq!(held.value(), &Scalar::from("USD"));
    assert_eq!(held.value().id(), DataTypeId::String);
    for (text, fact) in [("U\0S", "NUL byte"), ("\u{20ac}", "non-ASCII byte")] {
        let refused = FieldScalar::new(note.as_field(), text)
            .unwrap_err()
            .to_string();
        assert!(refused.contains(fact), "{refused}");
    }
    assert!(FieldScalar::new(note.as_field(), "USD\0").is_err());

    // `ascii(4)` is a maximum the value never carries.
    let bounded =
        StringField::try_new("ccy", DataType::from_str("ascii(4)").unwrap(), true).unwrap();
    let held = FieldScalar::new(bounded.as_field(), "EURO").unwrap();
    assert_eq!(held.value().dtype().unwrap(), DataType::ascii());
    let refused = FieldScalar::new(bounded.as_field(), "EUROS")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");

    // The fixed layout trims the padding storage writes and carries its width.
    let fixed = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
    let held = FieldScalar::new(fixed.as_field(), Scalar::from(b"USD\0")).unwrap();
    assert_eq!(held.as_str(), Some("USD"));
    assert_eq!(
        held.value().dtype().unwrap(),
        DataType::fixed_ascii(4).unwrap()
    );
    assert!(FieldScalar::new(fixed.as_field(), Scalar::from(b"US\xC3\xA9")).is_err());
}

#[test]
fn a_string_enum_is_accepted_on_fixed_us_ascii_up_to_sixteen_bytes_or_a_code() {
    let sides = StringEnum::from_members("Side", [("BUY", "1"), ("SELL", "2")]).unwrap();

    let mut accepted = DataType::fixed_ascii(4).unwrap().required_field("side");
    accepted.set_string_enum(&sides).unwrap();
    assert_eq!(accepted.string_enum().unwrap().as_ref(), Some(&sides));
    assert_eq!(
        accepted.remove_string_enum().unwrap().as_ref(),
        Some(&sides)
    );
    assert!(
        DataType::fixed_ascii(16)
            .unwrap()
            .required_field("side")
            .try_with_string_enum(&sides)
            .is_ok()
    );
    assert!(
        DataType::Side
            .required_field("side")
            .try_with_string_enum(&sides)
            .is_ok()
    );

    // A maximum is not a width, a wider slot does not pack, and any other
    // charset is not the repertoire the packing reads.
    for refused in [
        DataType::ascii(),
        DataType::from_str("ascii(4)").unwrap(),
        DataType::fixed_ascii(17).unwrap(),
        DataType::fixed_utf8(4).unwrap(),
        DataType::utf8(),
        DataType::fixed_size_binary(4).unwrap(),
    ] {
        let error = refused
            .clone()
            .required_field("side")
            .try_with_string_enum(&sides)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&refused.to_string()), "{refused}: {error}");
    }

    // A member wider than the slot is refused by the width.
    let wide = StringEnum::from_members("Wide", [("LONG", "TOOLONG")]).unwrap();
    assert!(
        DataType::fixed_ascii(4)
            .unwrap()
            .required_field("side")
            .try_with_string_enum(&wide)
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// The cast plan
// ---------------------------------------------------------------------------

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

/// One column under the root's own Arrow schema, so it carries the
/// extension document a stored US-ASCII column carries.
fn batch_of(root: &Field, column: ArrayRef) -> RecordBatch {
    RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column]).unwrap()
}

fn fixed(width: i32, cells: &[Option<&[u8]>]) -> ArrayRef {
    Arc::new(
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(cells.iter().copied(), width).unwrap(),
    )
}

fn fixed_cells(array: &ArrayRef) -> &FixedSizeBinaryArray {
    array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap()
}

/// Cast one recognized US-ASCII column to `target` through the batch path.
fn cast_column(batch: RecordBatch, target: Field) -> yggdryl::arrow::Result<ArrayRef> {
    Ok(root([target])
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))?
        .column(0)
        .clone())
}

#[test]
fn text_entering_an_ascii_width_is_validated_and_padded() {
    let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EU"), None]));

    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = fixed_cells(&cast);
    assert_eq!(cast.value(0), b"USD\0");
    assert_eq!(cast.value(1), b"EU\0\0");
    assert!(cast.is_null(2));
}

#[test]
fn a_value_breaking_the_width_rule_is_refused_naming_the_row_and_the_width() {
    let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
    for (value, fact) in [
        ("EURO!", "at most 4 bytes of us-ascii, got 5"),
        ("\u{20ac}", "non-ASCII byte"),
        ("U\0S", "NUL byte"),
    ] {
        let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some(value)]));
        let refused = field
            .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("\"ccy\""), "{refused}");
        assert!(refused.contains("row 1"), "{refused}");
        assert!(refused.contains(fact), "{refused}");
    }
}

#[test]
fn a_plain_fixed_binary_of_the_width_is_read_strictly() {
    let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), false).unwrap();
    // Bare storage declares nothing, so every cell is read as US-ASCII
    // through the one door bytes take, and written under the target.
    let source = fixed(4, &[Some(b"USD\0"), Some(b"EUR\0")]);
    let cast = field
        .as_field()
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = fixed_cells(&cast);
    assert_eq!(cast.value(0), b"USD\0");
    assert_eq!(cast.value(1), b"EUR\0");

    // The same storage carrying a non-ASCII byte is refused by row, naming
    // the charset that refused it.
    let broken = fixed(4, &[Some(b"USD\0"), Some(b"US\xC3\xA9")]);
    let refused = field
        .as_field()
        .cast_arrow_array(broken, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    assert!(refused.contains("us-ascii"), "{refused}");
}

#[test]
fn an_ascii_column_renders_as_trimmed_text() {
    let source = root([DataType::fixed_ascii(4).unwrap().nullable_field("ccy")]);
    let stored = fixed(4, &[Some(b"USD\0"), Some(b"EU\0\0"), None]);

    let text = cast_column(
        batch_of(&source, Arc::clone(&stored)),
        DataType::utf8().nullable_field("ccy"),
    )
    .unwrap();
    let text = text.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(text.value(0), "USD");
    assert_eq!(text.value(1), "EU");
    assert!(text.is_null(2));

    let view = cast_column(
        batch_of(&source, stored),
        DataType::utf8_view().nullable_field("ccy"),
    )
    .unwrap();
    let view = view.as_any().downcast_ref::<StringViewArray>().unwrap();
    assert_eq!(view.value(1), "EU");
    assert!(view.is_null(2));
}

#[test]
fn ascii_widths_re_pad_between_each_other() {
    let narrow = root([DataType::fixed_ascii(4).unwrap().nullable_field("ccy")]);
    let widened = cast_column(
        batch_of(&narrow, fixed(4, &[Some(b"USD\0"), None])),
        DataType::fixed_ascii(8).unwrap().nullable_field("ccy"),
    )
    .unwrap();
    let widened = fixed_cells(&widened);
    assert_eq!(widened.value(0), b"USD\0\0\0\0\0");
    assert!(widened.is_null(1));

    // Narrowing keeps what fits and refuses what does not, by row and width.
    let wide = root([DataType::fixed_ascii(8).unwrap().nullable_field("ccy")]);
    let narrowed = cast_column(
        batch_of(&wide, fixed(8, &[Some(b"USD\0\0\0\0\0")])),
        DataType::fixed_ascii(4).unwrap().nullable_field("ccy"),
    )
    .unwrap();
    assert_eq!(fixed_cells(&narrowed).value(0), b"USD\0");
    let refused = cast_column(
        batch_of(
            &wide,
            fixed(8, &[Some(b"USD\0\0\0\0\0"), Some(b"EUROS\0\0\0")]),
        ),
        DataType::fixed_ascii(4).unwrap().nullable_field("ccy"),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    assert!(refused.contains("got 5"), "{refused}");
}

#[test]
fn an_ascii_column_keeps_its_padding_into_a_binary_target() {
    let source = root([DataType::fixed_ascii(4).unwrap().nullable_field("ccy")]);
    let stored = fixed(4, &[Some(b"USD\0")]);

    let bytes = cast_column(
        batch_of(&source, Arc::clone(&stored)),
        DataType::binary().nullable_field("ccy"),
    )
    .unwrap();
    assert_eq!(
        bytes
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"USD\0"
    );

    // The fixed binary of the same width is the storage itself.
    let same = cast_column(
        batch_of(&source, Arc::clone(&stored)),
        DataType::fixed_size_binary(4)
            .unwrap()
            .nullable_field("ccy"),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&same, &stored));
}

#[test]
fn a_dictionary_of_text_enters_an_ascii_width() {
    let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
    let keys = Int32Array::from(vec![Some(0), Some(1), None, Some(0)]);
    let values: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR"]));
    let source: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());

    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = fixed_cells(&cast);
    assert_eq!(cast.value(0), b"USD\0");
    assert_eq!(cast.value(1), b"EUR\0");
    assert!(cast.is_null(2));
    assert_eq!(cast.value(3), b"USD\0");
}

#[test]
fn a_required_ascii_field_fills_nulls_with_the_all_nul_default() {
    let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), false).unwrap();
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));

    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = fixed_cells(&cast);
    assert_eq!(cast.null_count(), 0);
    assert_eq!(cast.value(0), b"USD\0");
    assert_eq!(cast.value(1), b"\0\0\0\0");
}

#[test]
fn a_hidden_struct_child_is_neither_validated_nor_copied() {
    let target = root([Field::new(
        "position",
        DataType::from_fields([DataType::fixed_ascii(4).unwrap().required_field("ccy")]).unwrap(),
        true,
    )]);
    // Row 1 is null at the struct level; its child slot holds a value that
    // breaks the width rule, which a hidden slot never has to satisfy.
    let ccy: ArrayRef = Arc::new(StringArray::from(vec!["USD", "not a currency"]));
    let position = StructArray::new(
        Fields::from(vec![ArrowField::new("ccy", ArrowDataType::Utf8, true)]),
        vec![ccy],
        Some(NullBuffer::from(vec![true, false])),
    );
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "position",
        position.data_type().clone(),
        true,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(position)]).unwrap();

    let cast = target
        .cast_arrow_batch(batch, ArrowCastOptions::new())
        .unwrap();
    let position = cast
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    assert!(position.is_valid(0));
    assert!(position.is_null(1));
    let ccy = position
        .column(0)
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(ccy.value(0), b"USD\0");
}
