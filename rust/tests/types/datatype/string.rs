//! One string family: five layouts, any charset, one bound.

use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::types::{StringLayout, StringParameters};
use yggdryl::{Charset, DataType, DataTypeId, Field, Scalar};

/// Every layout, under both of its two spellings.
const SPELLINGS: [(StringLayout, &str, &str); 5] = [
    (StringLayout::String, "string", "utf8"),
    (StringLayout::FixedString, "fixed_string", "fixed_utf8"),
    (StringLayout::StringView, "string_view", "utf8_view"),
    (StringLayout::LargeString, "large_string", "large_utf8"),
    (
        StringLayout::LargeStringView,
        "large_string_view",
        "large_utf8_view",
    ),
];

#[test]
fn a_plain_utf8_string_is_the_datatype_it_already_was() {
    // Three of the five layouts already have a datatype, so declaring them
    // with nothing else gives that one back rather than a second spelling.
    assert_eq!(DataType::from_str("string").unwrap(), DataType::Utf8);
    assert_eq!(DataType::from_str("utf8").unwrap(), DataType::Utf8);
    assert_eq!(
        DataType::from_str("largestring").unwrap(),
        DataType::LargeUtf8
    );
    assert_eq!(
        DataType::from_str("stringview").unwrap(),
        DataType::Utf8View
    );
    assert_eq!(
        DataType::string(StringParameters::utf8(StringLayout::LargeString)).unwrap(),
        DataType::LargeUtf8
    );

    // The two Arrow has no name for stay their own.
    assert_eq!(
        DataType::from_str("large_utf8_view").unwrap().id(),
        DataTypeId::LargeStringView
    );
    assert_eq!(
        DataType::from_str("fixed_utf8(8)").unwrap().id(),
        DataTypeId::FixedString
    );
}

#[test]
fn us_ascii_text_redirects_into_the_ascii_family_that_owns_it() {
    // US-ASCII text is already a datatype with a value contract of its own,
    // so a string declaring it is that datatype rather than a rival to it.
    assert_eq!(
        DataType::from_str("string(us-ascii)").unwrap(),
        DataType::Ascii
    );
    assert_eq!(
        DataType::from_str("fixedstring(us-ascii,3)").unwrap(),
        DataType::ascii(3).unwrap()
    );

    // And a shape that family has no room for is refused by name rather than
    // becoming a second kind of ASCII column.
    let refusal = DataType::from_str("stringview(us-ascii)").unwrap_err();
    assert!(
        refusal.to_string().contains("ascii"),
        "the refusal should name the family that owns US-ASCII: {refusal}"
    );
}

#[test]
fn every_layout_answers_to_both_spellings_and_renders_as_one() {
    for (layout, general, utf8) in SPELLINGS {
        assert_eq!(StringLayout::from_str(general).unwrap(), layout);
        assert_eq!(StringLayout::from_str(utf8).unwrap(), layout);
        // The fold is the grammar's: case, underscores and hyphens all drop,
        // so the run-together spellings are the same names.
        assert_eq!(
            StringLayout::from_str(&general.to_uppercase()).unwrap(),
            layout
        );
        assert_eq!(
            StringLayout::from_str(&general.replace('_', "")).unwrap(),
            layout
        );
        assert_eq!(
            StringLayout::from_str(&utf8.replace('_', "")).unwrap(),
            layout
        );

        let width = layout.is_fixed().then_some(8);
        for charset in [Charset::Utf8, Charset::Cp1252] {
            let mut parameters = StringParameters::new(layout, charset);
            if let Some(width) = width {
                parameters = parameters.try_with_bound(width).unwrap();
            }
            let dtype = DataType::string(parameters).unwrap();
            let rendered = dtype.to_string();
            // A UTF-8 string renders under the `utf8` name, every other
            // charset under the `string` one, and both read back as itself.
            let expected = match charset.is_utf8() {
                true => utf8,
                false => general,
            };
            assert!(
                rendered.starts_with(expected),
                "{rendered} should be spelled {expected}"
            );
            assert_eq!(DataType::from_str(&rendered).unwrap(), dtype, "{rendered}");
        }
    }
}

#[test]
fn a_bound_is_the_width_on_a_fixed_layout_and_the_maximum_on_the_others() {
    let bounded = DataType::from_str("string(windows-1252,32)").unwrap();
    let parameters = bounded.string_parameters().unwrap();
    assert_eq!(parameters.max(), Some(32));
    assert_eq!(parameters.fixed(), None);
    assert_eq!(parameters.bound(), Some(32));
    assert_eq!(bounded.to_string(), "string(windows-1252,32)");

    let fixed = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
    let parameters = fixed.string_parameters().unwrap();
    assert_eq!(parameters.fixed(), Some(8));
    assert_eq!(parameters.max(), None);
    assert_eq!(fixed.to_string(), "fixed_string(windows-1252,8)");

    // A fixed layout with no width is a question, not a declaration.
    assert!(DataType::from_str("fixedstring").is_err());
    assert!(DataType::from_str("fixedutf8").is_err());
    // And a bound of no bytes is a column of one value.
    assert!(DataType::from_str("utf8(0)").is_err());
}

#[test]
fn the_utf8_spellings_refuse_a_charset_the_name_already_declares() {
    // Every `utf8` spelling, under every punctuation the fold accepts.
    for spelling in [
        "utf8",
        "fixed_utf8",
        "fixedutf8",
        "utf8_view",
        "utf8view",
        "large_utf8",
        "largeutf8",
        "large_utf8_view",
        "largeutf8view",
    ] {
        let refusal = DataType::from_str(&format!("{spelling}(windows-1252)")).unwrap_err();
        assert!(
            refusal.to_string().contains("string"),
            "{spelling} should name the spelling that takes a charset: {refusal}"
        );
    }
    let refusal = DataType::from_str("utf8(windows-1252)").unwrap_err();
    assert!(
        refusal.to_string().contains("string"),
        "the refusal should name the spelling that takes a charset: {refusal}"
    );
    assert_eq!(
        DataType::from_str("string(windows-1252)")
            .unwrap()
            .charset(),
        Some(Charset::Cp1252)
    );
}

#[test]
fn sql_reads_its_own_two_string_shapes() {
    // `varchar(n)` bounds; `char(n)` is blank-padded to exactly n.
    assert_eq!(
        DataType::from_str("varchar(32)").unwrap().to_string(),
        "utf8(32)"
    );
    assert_eq!(
        DataType::from_str("character varying(32)")
            .unwrap()
            .to_string(),
        "utf8(32)"
    );
    assert_eq!(
        DataType::from_str("char(8)").unwrap().to_string(),
        "fixed_utf8(8)"
    );
    // A width is what makes a string fixed, so a bare `char` is not.
    assert_eq!(DataType::from_str("char").unwrap(), DataType::Utf8);
}

#[test]
fn every_string_datatype_answers_one_question_about_its_charset() {
    // The unification: the plain layouts and the ASCII family answer the same
    // accessor the parameterized strings do.
    assert_eq!(DataType::Utf8.charset(), Some(Charset::Utf8));
    assert_eq!(DataType::Utf8View.charset(), Some(Charset::Utf8));
    assert_eq!(DataType::Ascii.charset(), Some(Charset::Ascii));
    assert_eq!(DataType::ascii(3).unwrap().charset(), Some(Charset::Ascii));
    assert_eq!(
        DataType::ascii(3)
            .unwrap()
            .string_parameters()
            .unwrap()
            .fixed(),
        Some(3)
    );
    assert_eq!(
        DataType::from_str("large_string(iso-8859-15)")
            .unwrap()
            .charset(),
        Some(Charset::Latin9)
    );

    // A code is an identity with a storage, not a string with a charset.
    assert_eq!(DataType::Currency.charset(), None);
    assert!(!DataType::Currency.is_string());
    assert!(DataType::Utf8.is_string());
}

#[test]
fn a_charset_string_lays_out_as_bytes_and_reads_back_as_itself() {
    // Arrow's string layouts declare UTF-8, so text that is not UTF-8 rides
    // the binary layout beside them and the charset rides the metadata.
    let cases: [(&str, ArrowDataType); 6] = [
        ("string(windows-1252)", ArrowDataType::Binary),
        ("largestring(windows-1252)", ArrowDataType::LargeBinary),
        ("stringview(windows-1252)", ArrowDataType::BinaryView),
        ("utf8(32)", ArrowDataType::Utf8),
        ("large_utf8_view", ArrowDataType::Utf8View),
        ("fixed_utf8(8)", ArrowDataType::FixedSizeBinary(8)),
    ];
    for (spelling, storage) in cases {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(dtype.clone().into_arrow().unwrap(), storage, "{spelling}");

        let field = dtype.clone().nullable_field("value");
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            Some("yggdryl.string"),
            "{spelling}"
        );
        assert!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_METADATA_KEY)
                .is_some_and(|document| document.contains("\"layout\"")),
            "{spelling} should declare its layout"
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{spelling}");
    }
}

#[test]
fn the_two_view_layouts_share_one_arrow_layout_and_stay_distinct_here() {
    // Arrow has one view layout; this crate declares two, and the difference
    // travels in the metadata rather than in the buffers.
    let view = DataType::Utf8View;
    let large = DataType::from_str("large_utf8_view").unwrap();
    assert_ne!(view, large);
    assert_eq!(
        view.clone().into_arrow().unwrap(),
        large.clone().into_arrow().unwrap()
    );
    let field = large.clone().nullable_field("value");
    assert_eq!(
        Field::from_arrow(&field.clone().into_arrow().unwrap()).unwrap(),
        field
    );
}

#[test]
fn a_string_column_stores_the_bytes_its_charset_writes() {
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype.scalar("Grüße").unwrap();
    let text = value.as_str().unwrap();
    assert_eq!(text, "Grüße");
    assert_eq!(value.id(), DataTypeId::String);
    assert_eq!(value.dtype().unwrap(), dtype);

    // The characters are UTF-8 in memory and windows-1252 on the way out.
    let encoded = Charset::Cp1252.encode("Grüße").unwrap();
    assert_eq!(encoded.len(), 5);
    assert_eq!(encoded.as_ref(), &[0x47, 0x72, 0xFC, 0xDF, 0x65]);
}

#[test]
fn bytes_arriving_at_a_string_column_are_read_through_its_charset() {
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype
        .scalar(Scalar::from(vec![0x47_u8, 0x72, 0xFC, 0xDF, 0x65]))
        .unwrap();
    assert_eq!(value.as_str(), Some("Grüße"));

    // And a byte the charset leaves unassigned is transcribed rather than
    // refusing the whole value: `0x81` is a C1 control in ISO 8859-1, which
    // is what the WHATWG index maps it to too.
    let recovered = dtype
        .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
        .unwrap();
    assert_eq!(recovered.as_str(), Some("ok\u{0081}"));
}

#[test]
fn a_maximum_is_counted_in_the_bytes_the_charset_stores() {
    let dtype = DataType::from_str("string(windows-1252,4)").unwrap();
    // Five scalars, five stored bytes: over the bound.
    assert!(dtype.scalar("Grüße").is_err());
    // Four scalars in one byte each: within it.
    assert_eq!(dtype.scalar("Grüß").unwrap().as_str(), Some("Grüß"));

    // UTF-8 counts the bytes it stores, which is not the scalar count.
    let utf8 = DataType::from_str("utf8(4)").unwrap();
    assert!(utf8.scalar("Grüß").is_err());
    assert_eq!(utf8.scalar("Grü").unwrap().as_str(), Some("Grü"));
}

#[test]
fn a_fixed_string_pads_its_storage_and_reads_back_trimmed() {
    let dtype = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
    let value = dtype.scalar("café").unwrap();
    assert_eq!(value.as_str(), Some("café"));
    assert_eq!(value.dtype().unwrap(), dtype);
    // Four stored bytes in windows-1252, so it fits eight; five in UTF-8
    // would too, and neither is the scalar count.
    assert!(
        DataType::from_str("fixedstring(windows-1252,3)")
            .unwrap()
            .scalar("café")
            .is_err()
    );
}

#[test]
fn a_string_restates_into_another_layout_without_copying_its_characters() {
    let source = DataType::Utf8.scalar("AAPL").unwrap();
    let target = DataType::from_str("largestring(windows-1252)").unwrap();
    let restated = target.scalar(source).unwrap();
    assert_eq!(restated.as_str(), Some("AAPL"));
    assert_eq!(restated.id(), DataTypeId::LargeString);
    assert_eq!(restated.dtype().unwrap(), target);
}

#[test]
fn a_string_datatype_survives_both_serde_doors() {
    for spelling in [
        "string(windows-1252)",
        "fixed_string(windows-1252,8)",
        "string_view(windows-1252,32)",
        "large_string(iso-8859-15)",
        "large_utf8_view",
        "utf8(32)",
        "fixed_utf8(8)",
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(DataType::from_json(&json).unwrap(), dtype, "{spelling}");

        let value = dtype.clone().into_value();
        assert_eq!(DataType::from_value(value).unwrap(), dtype, "{spelling}");
    }
}

#[test]
fn a_string_value_survives_the_scalar_wire_format() {
    for spelling in [
        "string(windows-1252)",
        "fixed_string(windows-1252,8)",
        "large_utf8_view",
        "utf8(32)",
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        let value = dtype.scalar("café").unwrap();
        let json = serde_json::to_string(&value).unwrap();
        let read: Scalar = serde_json::from_str(&json).unwrap();
        assert_eq!(read, value, "{spelling}");
        // A value declares the layout and charset it is stored in; the bound
        // is the column's rule about values, not part of one.
        assert_eq!(read.id(), value.id(), "{spelling}");
        assert_eq!(dtype.scalar(read).unwrap(), value, "{spelling}");
    }

    // The ordinary string still writes its characters and nothing else.
    assert_eq!(
        serde_json::to_string(&Scalar::from("AAPL")).unwrap(),
        r#"{"type":"string","value":"AAPL"}"#
    );
}

#[test]
fn a_hand_built_string_with_no_width_is_refused_before_a_boundary() {
    // The variant is public, so a caller can build what the constructor would
    // have refused; `validate` is where that stops.
    let unwidened = DataType::String(StringParameters::new(
        StringLayout::FixedString,
        Charset::Cp1252,
    ));
    assert!(unwidened.validate().is_err());
    assert!(unwidened.clone().into_arrow().is_err());
}

#[test]
fn a_cast_that_would_have_to_re_encode_is_refused_rather_than_reinterpreted() {
    use arrow_array::{ArrayRef, StringArray};
    use std::sync::Arc;
    use yggdryl::{ArrowCast, ArrowCastOptions};

    // Arrow's kernel would happily hand a UTF-8 buffer to a windows-1252
    // column and call it a framing change; the column would read back as
    // mojibake, so the pair is refused by name.
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café")]));
    let target = DataType::from_str("string(windows-1252)")
        .unwrap()
        .nullable_field("value");
    let refusal = target
        .cast_arrow_array(source, ArrowCastOptions::default())
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("re-encode"), "{refusal}");

    // A field already in that exact string reads back as itself.
    let arrow = target.clone().into_arrow().unwrap();
    assert_eq!(Field::from_arrow(&arrow).unwrap(), target);
}

#[test]
fn a_charset_a_column_cannot_write_is_refused_where_the_bytes_are_written() {
    // The value door counts rather than judges, because the permissive read
    // recovers damage precisely by answering scalars the charset does not
    // assign. The write is where a scalar with no byte is refused, by name.
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype.scalar("東京").unwrap();
    assert_eq!(value.as_str(), Some("東京"));

    let field = dtype.nullable_field("value");
    let refusal = yggdryl::FieldScalar::new(&field, value)
        .unwrap()
        .into_arrow_array()
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("windows-1252"), "{refusal}");

    // A recovered byte survives the round trip the strict door would refuse.
    let damaged = DataType::from_str("string(windows-1252)")
        .unwrap()
        .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
        .unwrap();
    assert_eq!(damaged.as_str(), Some("ok\u{0081}"));
}
