//! The fixed ASCII types: typed markers, and the cast plan around storage.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayRef, BinaryArray, DictionaryArray, FixedSizeBinaryArray, Int32Array, RecordBatch,
    StringArray, StringViewArray, StructArray,
};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields, Schema};
use yggdryl::types::{
    AsciiField, CfiField, CountryField, CurrencyField, FixedAsciiField, MicField, ascii,
};
use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field, FieldScalar};

use super::typed::assert_typed_marker;

#[test]
fn ascii_markers_cover_the_widths_and_the_codes() {
    assert_typed_marker::<ascii::FixedAsciiType>(DataType::FixedAscii(4));
    assert_typed_marker::<ascii::FixedAsciiType>(DataType::FixedAscii(8));
    assert_typed_marker::<ascii::FixedAsciiType>(DataType::FixedAscii(16));
    assert_typed_marker::<ascii::CountryType>(DataType::Country);
    assert_typed_marker::<ascii::CurrencyType>(DataType::Currency);
    assert_typed_marker::<ascii::MicType>(DataType::Mic);
    assert_typed_marker::<ascii::CfiType>(DataType::Cfi);

    // The variable shape is parameterless, so it has a static constructor;
    // the fixed one carries a width and takes its datatype through `try_new`.
    assert_typed_marker::<ascii::AsciiType>(DataType::Ascii);
    assert_eq!(AsciiField::new("note", true).dtype(), &DataType::Ascii);
    let ccy = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), false).unwrap();
    assert_eq!(ccy.dtype(), &DataType::FixedAscii(4));
    assert!(FixedAsciiField::try_new("ccy", DataType::Ascii, false).is_err());
    // The storage says no width, so the declaration is where a width that
    // cannot hold a value is refused - at the Arrow boundary too, not only
    // in the constructor.
    assert!(DataType::ascii(0).is_err());
    assert!(
        Field::new("ccy", DataType::FixedAscii(0), false)
            .into_arrow()
            .is_err()
    );
    assert!(AsciiField::try_new("note", DataType::FixedAscii(4), true).is_err());

    // The code/width boundary is the one the markers exist for: a currency
    // and an `ascii(3)` are the same three bytes and are not each other.
    assert_eq!(
        CurrencyField::new("ccy", false).dtype(),
        &DataType::Currency
    );
    assert_eq!(CountryField::new("iso", true).dtype(), &DataType::Country);
    assert_eq!(MicField::new("venue", true).dtype(), &DataType::Mic);
    assert!(CurrencyField::try_new("ccy", DataType::FixedAscii(3), false).is_err());
    assert!(FixedAsciiField::try_new("ccy", DataType::Currency, false).is_err());
    // Six bytes against eight: the confusion a width/code mix-up produces.
    assert!(CfiField::try_new("code", DataType::FixedAscii(8), false).is_err());

    // The typed value is checked under the one ASCII rule for its width.
    let width = FixedAsciiField::try_new("code", DataType::FixedAscii(8), false).unwrap();
    let code = FieldScalar::new(width.as_field(), "ABC").unwrap();
    assert_eq!(code.as_str(), Some("ABC"));
    assert_eq!(code.value().id(), yggdryl::DataTypeId::FixedAscii);
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

// ---------------------------------------------------------------------------
// The cast plan
// ---------------------------------------------------------------------------

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

/// One column under the root's own Arrow schema, so it carries the
/// extension identity a stored ASCII column carries.
fn batch_of(root: &Field, column: ArrayRef) -> RecordBatch {
    RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column]).unwrap()
}

fn fixed(width: i32, cells: &[Option<&[u8]>]) -> ArrayRef {
    Arc::new(
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(cells.iter().copied(), width).unwrap(),
    )
}

/// Cast one recognized ASCII column to `target` through the batch path.
fn cast_column(batch: RecordBatch, target: Field) -> yggdryl::arrow::Result<ArrayRef> {
    Ok(root([target])
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))?
        .column(0)
        .clone())
}

#[test]
fn text_entering_an_ascii_width_is_validated_and_kept() {
    let field = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), true).unwrap();
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EU"), None]));

    let cast = field
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert_eq!(cast.value(0), "USD");
    assert_eq!(cast.value(1), "EU");
    assert!(cast.is_null(2));

    // The target storage is the source storage, so the cast validated the
    // column and handed back the very array it was given.
    let same = field
        .as_field()
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&same, &source));
}

#[test]
fn a_value_breaking_the_width_rule_is_refused_naming_the_row_and_the_width() {
    let field = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), true).unwrap();
    for (value, fact) in [
        ("EURO!", "5 bytes"),
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
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        assert!(refused.contains(fact), "{refused}");
    }
}

#[test]
fn a_plain_fixed_binary_is_read_as_the_padded_storage_it_is() {
    let field = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), false).unwrap();
    let source = fixed(4, &[Some(b"USD\0"), Some(b"EUR\0")]);
    let cast = field
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    // A fixed slot pads by construction, so it is read through the width's
    // rule and stored as the text it holds.
    assert_eq!(cast.value(0), "USD");
    assert_eq!(cast.value(1), "EUR");
    assert_eq!(cast.value_data().len(), 6);

    // The same storage carrying a non-ASCII byte is refused by row.
    let broken = fixed(4, &[Some(b"USD\0"), Some(b"US\xC3\xA9")]);
    let refused = field
        .as_field()
        .cast_arrow_array(broken, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    assert!(refused.contains("non-ASCII byte"), "{refused}");
}

#[test]
fn an_ascii_column_renders_as_the_text_it_holds() {
    let source = root([DataType::FixedAscii(4).nullable_field("ccy")]);
    let stored: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EU"), None]));

    let text = cast_column(
        batch_of(&source, Arc::clone(&stored)),
        DataType::Utf8.nullable_field("ccy"),
    )
    .unwrap();
    // The identity changes and the storage does not, so rendering an ASCII
    // column as text is the column itself.
    assert!(Arc::ptr_eq(&text, &stored));
    let text = text.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(text.value(0), "USD");
    assert_eq!(text.value(1), "EU");
    assert!(text.is_null(2));

    let view = cast_column(
        batch_of(&source, stored),
        DataType::Utf8View.nullable_field("ccy"),
    )
    .unwrap();
    let view = view.as_any().downcast_ref::<StringViewArray>().unwrap();
    assert_eq!(view.value(1), "EU");
    assert!(view.is_null(2));
}

#[test]
fn ascii_widths_rebound_between_each_other_without_restoring_a_stride() {
    let narrow = root([DataType::FixedAscii(4).nullable_field("ccy")]);
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));
    let widened = cast_column(
        batch_of(&narrow, Arc::clone(&source)),
        DataType::FixedAscii(8).nullable_field("ccy"),
    )
    .unwrap();
    // Widening changes the bound and not the bytes, so the column is the one
    // that arrived: a wider width is not a wider slot.
    assert!(Arc::ptr_eq(&widened, &source));
    let widened = widened.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(widened.value(0), "USD");
    assert!(widened.is_null(1));

    // Narrowing keeps what fits and refuses what does not, by row and width.
    let wide = root([DataType::FixedAscii(8).nullable_field("ccy")]);
    let narrowed = cast_column(
        batch_of(&wide, Arc::new(StringArray::from(vec![Some("USD")]))),
        DataType::FixedAscii(4).nullable_field("ccy"),
    )
    .unwrap();
    assert_eq!(
        narrowed
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "USD"
    );
    let refused = cast_column(
        batch_of(
            &wide,
            Arc::new(StringArray::from(vec![Some("USD"), Some("EUROS")])),
        ),
        DataType::FixedAscii(4).nullable_field("ccy"),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    assert!(refused.contains("5 bytes"), "{refused}");
}

#[test]
fn an_ascii_column_spells_its_own_bytes_into_a_binary_target() {
    let source = root([DataType::FixedAscii(4).nullable_field("ccy")]);
    let stored: ArrayRef = Arc::new(StringArray::from(vec![Some("USD")]));

    let bytes = cast_column(
        batch_of(&source, Arc::clone(&stored)),
        DataType::Binary.nullable_field("ccy"),
    )
    .unwrap();
    // The storage holds the value, so the bytes a binary target reads are
    // the value's and carry no padding to strip.
    assert_eq!(
        bytes
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"USD"
    );

    // Padding into a fixed slot is that slot's own rule, and a value shorter
    // than the slot does not satisfy it.
    assert!(
        cast_column(
            batch_of(&source, stored),
            DataType::FixedSizeBinary(4).nullable_field("ccy"),
        )
        .is_err()
    );
}

#[test]
fn a_dictionary_of_text_enters_an_ascii_width() {
    let field = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), true).unwrap();
    let keys = Int32Array::from(vec![Some(0), Some(1), None, Some(0)]);
    let values: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR"]));
    let source: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());

    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(cast.value(0), "USD");
    assert_eq!(cast.value(1), "EUR");
    assert!(cast.is_null(2));
    assert_eq!(cast.value(3), "USD");
}

#[test]
fn a_required_ascii_field_fills_nulls_with_the_empty_default() {
    let field = FixedAsciiField::try_new("ccy", DataType::FixedAscii(4), false).unwrap();
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));

    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(cast.null_count(), 0);
    assert_eq!(cast.value(0), "USD");
    assert_eq!(cast.value(1), "");
}

#[test]
fn a_hidden_struct_child_is_neither_validated_nor_copied() {
    let target = root([Field::new(
        "position",
        DataType::from_fields([DataType::FixedAscii(4).required_field("ccy")]).unwrap(),
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
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(ccy.value(0), "USD");
}
