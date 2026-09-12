//! One string family: five layouts, any charset, one bound.

use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::types::{INLINE_CAPACITY, Str, StringLayout, StringParameters};
use yggdryl::{Charset, DataType, DataTypeId, Field, Scalar};

/// Every layout, under its three spellings: general, UTF-8, US-ASCII.
const SPELLINGS: [(StringLayout, &str, &str, &str); 5] = [
    (StringLayout::String, "string", "utf8", "ascii"),
    (
        StringLayout::FixedString,
        "fixed_string",
        "fixed_utf8",
        "fixed_ascii",
    ),
    (
        StringLayout::StringView,
        "string_view",
        "utf8_view",
        "ascii_view",
    ),
    (
        StringLayout::LargeString,
        "large_string",
        "large_utf8",
        "large_ascii",
    ),
    (
        StringLayout::LargeStringView,
        "large_string_view",
        "large_utf8_view",
        "large_ascii_view",
    ),
];

#[test]
fn a_plain_string_is_one_datatype_under_every_spelling() {
    // The `string` name, the `utf8` name and the sugar constructor are one
    // datatype, and its identifier is the layout's.
    assert_eq!(DataType::from_str("string").unwrap(), DataType::utf8());
    assert_eq!(DataType::from_str("utf8").unwrap(), DataType::utf8());
    assert_eq!(
        DataType::from_str("largestring").unwrap(),
        DataType::large_utf8()
    );
    assert_eq!(
        DataType::from_str("stringview").unwrap(),
        DataType::utf8_view()
    );
    assert_eq!(
        DataType::string(StringParameters::utf8(StringLayout::LargeString)).unwrap(),
        DataType::large_utf8()
    );
    assert_eq!(
        DataType::utf8(),
        DataType::String(StringParameters::default())
    );
    assert_eq!(DataType::utf8().id(), DataTypeId::String);
    assert_eq!(DataType::large_utf8().id(), DataTypeId::LargeString);
    assert_eq!(DataType::utf8_view().id(), DataTypeId::StringView);
    assert_eq!(
        DataType::from_str("large_utf8_view").unwrap().id(),
        DataTypeId::LargeStringView
    );
    assert_eq!(
        DataType::from_str("fixed_utf8(8)").unwrap().id(),
        DataTypeId::FixedString
    );
    assert_eq!(
        DataType::fixed_utf8(8).unwrap().to_string(),
        "fixed_utf8(8)"
    );
    // Every string identifier is parameterized: the layout alone is not a
    // datatype.
    for id in [
        DataTypeId::String,
        DataTypeId::FixedString,
        DataTypeId::StringView,
        DataTypeId::LargeString,
        DataTypeId::LargeStringView,
    ] {
        assert!(id.is_parameterized(), "{id}");
        assert!(id.is_string(), "{id}");
        assert_eq!(id.fixed_byte_width(), None, "{id}");
    }
}

#[test]
fn every_layout_answers_to_three_spellings_and_renders_under_its_charset() {
    for (layout, general, utf8, ascii) in SPELLINGS {
        assert_eq!(layout.as_str(), general);
        assert_eq!(layout.as_utf8_str(), utf8);
        assert_eq!(layout.as_ascii_str(), ascii);
        for spelling in [general, utf8] {
            assert_eq!(StringLayout::from_str(spelling).unwrap(), layout);
            // The fold is the grammar's: case, underscores and hyphens all
            // drop, so the run-together spellings are the same names.
            assert_eq!(
                StringLayout::from_str(&spelling.to_uppercase()).unwrap(),
                layout
            );
            assert_eq!(
                StringLayout::from_str(&spelling.replace('_', "")).unwrap(),
                layout
            );
        }
        // A US-ASCII spelling names a charset a bare layout would drop, so
        // it is refused here and read only where the charset travels with
        // it - the datatype grammar.
        let refused = StringLayout::from_str(ascii).unwrap_err().to_string();
        assert!(refused.contains("us-ascii"), "{refused}");
        let declared = match layout.is_fixed() {
            true => format!("{ascii}(8)"),
            false => ascii.to_owned(),
        };
        let parameters = DataType::from_str(&declared)
            .unwrap()
            .string_parameters()
            .unwrap();
        assert_eq!(
            (parameters.layout(), parameters.charset()),
            (layout, Charset::Ascii)
        );
        assert_eq!(StringLayout::from_id(layout.id()), Some(layout));

        let width = layout.is_fixed().then_some(8);
        for charset in [Charset::Utf8, Charset::Ascii, Charset::Cp1252] {
            let mut parameters = StringParameters::new(layout, charset);
            if let Some(width) = width {
                parameters = parameters.try_with_bound(width).unwrap();
            }
            let dtype = DataType::string(parameters).unwrap();
            let rendered = dtype.to_string();
            // A UTF-8 string renders under the `utf8` name, a US-ASCII one
            // under the `ascii` name, every other charset under the
            // `string` one, and each reads back as itself.
            let expected = match charset {
                Charset::Utf8 => utf8,
                Charset::Ascii => ascii,
                _ => general,
            };
            assert_eq!(parameters.layout_name(), expected);
            assert!(
                rendered.starts_with(expected),
                "{rendered} should be spelled {expected}"
            );
            assert_eq!(DataType::from_str(&rendered).unwrap(), dtype, "{rendered}");
            assert_eq!(dtype.id(), layout.id(), "{rendered}");
            assert_eq!(dtype.string_parameters(), Some(parameters), "{rendered}");
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
fn the_charset_named_spellings_refuse_a_charset_the_name_already_declares() {
    // Every `utf8` and `ascii` spelling, under every punctuation the fold
    // accepts.
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
        "ascii",
        "fixed_ascii",
        "ascii_view",
        "large_ascii",
        "large_ascii_view",
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
    assert_eq!(DataType::from_str("char").unwrap(), DataType::utf8());
}

#[test]
fn every_string_datatype_answers_one_question_about_its_charset() {
    // One accessor for every string, whatever its spelling.
    assert_eq!(DataType::utf8().charset(), Some(Charset::Utf8));
    assert_eq!(DataType::utf8_view().charset(), Some(Charset::Utf8));
    assert_eq!(DataType::ascii().charset(), Some(Charset::Ascii));
    assert_eq!(
        DataType::fixed_ascii(3).unwrap().charset(),
        Some(Charset::Ascii)
    );
    assert_eq!(
        DataType::fixed_ascii(3)
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
    assert!(DataType::utf8().is_string());
}

#[test]
fn a_charset_string_lays_out_as_bytes_and_reads_back_as_itself() {
    // Arrow's string layouts declare UTF-8, so text that is not UTF-8 rides
    // the binary layout beside them and the charset rides the metadata;
    // US-ASCII is UTF-8 and rides the text layouts under the same document.
    let cases: [(&str, ArrowDataType); 9] = [
        ("string(windows-1252)", ArrowDataType::Binary),
        ("largestring(windows-1252)", ArrowDataType::LargeBinary),
        ("stringview(windows-1252)", ArrowDataType::BinaryView),
        ("utf8(32)", ArrowDataType::Utf8),
        ("large_utf8_view", ArrowDataType::Utf8View),
        ("fixed_utf8(8)", ArrowDataType::FixedSizeBinary(8)),
        ("ascii", ArrowDataType::Utf8),
        ("large_ascii(16)", ArrowDataType::LargeUtf8),
        ("ascii_view", ArrowDataType::Utf8View),
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

    // Plain UTF-8 is Arrow's own datatype and crosses bare.
    for dtype in [
        DataType::utf8(),
        DataType::large_utf8(),
        DataType::utf8_view(),
    ] {
        let arrow = dtype.clone().nullable_field("value").into_arrow().unwrap();
        assert!(
            !arrow.metadata().contains_key(EXTENSION_TYPE_NAME_KEY),
            "{dtype} should cross bare"
        );
    }
}

#[test]
fn the_two_view_layouts_share_one_arrow_layout_and_stay_distinct_here() {
    // Arrow has one view layout; this crate declares two, and the difference
    // travels in the metadata rather than in the buffers.
    let view = DataType::utf8_view();
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
fn utf8_reads_bytes_strictly_in_every_layout() {
    // UTF-8 is a validated repertoire: bytes that are not UTF-8 are refused
    // naming the charset, under every spelling of the layout, while a legacy
    // charset transcribes the same bytes.
    let damaged = vec![0x6F_u8, 0x6B, 0x81];
    for spelling in [
        "utf8",
        "utf8(8)",
        "fixed_utf8(8)",
        "large_utf8",
        "utf8_view",
        "large_utf8_view",
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        let refused = dtype
            .scalar(Scalar::from(damaged.clone()))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("utf-8"), "{spelling}: {refused}");
        assert_eq!(
            dtype
                .scalar(Scalar::from("caf\u{e9}".as_bytes().to_vec()))
                .unwrap()
                .as_str(),
            Some("caf\u{e9}"),
            "{spelling}"
        );
    }
    assert!(Str::from_bytes(&damaged, StringParameters::default()).is_err());
    assert_eq!(
        Str::from_bytes(&damaged, StringParameters::from(Charset::Cp1252)).unwrap(),
        "ok\u{0081}"
    );
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
    // Long enough to be heap storage: below the inline buffer a rewrite
    // copies the bytes into the new value and no pointer identity could be
    // observed, so a short value cannot witness this claim at all.
    const LONG: &str = "a value well past the twenty-three byte inline buffer";
    assert!(LONG.len() > INLINE_CAPACITY);

    let source = DataType::utf8().scalar(LONG).unwrap();
    let origin = source.as_str().unwrap().as_ptr();
    let target = DataType::from_str("large_string(windows-1252)").unwrap();
    let restated = target.scalar(source).unwrap();
    assert_eq!(restated.as_str(), Some(LONG));
    assert_eq!(restated.id(), DataTypeId::LargeString);
    assert_eq!(restated.dtype().unwrap(), target);
    // The layout is the offset width, not the bytes: the rewrite adopts the
    // storage handle rather than copying the characters.
    assert!(
        std::ptr::eq(restated.as_str().unwrap().as_ptr(), origin),
        "restating a layout should share its storage"
    );

    let short = DataType::utf8().scalar("AAPL").unwrap();
    assert_eq!(target.scalar(short).unwrap().as_str(), Some("AAPL"));
}

#[test]
fn str_is_the_compact_string_and_compares_by_its_characters() {
    // Short text lives inside the value, a static spelling costs nothing,
    // and longer text is one shared handle.
    let short = Str::new("AAPL");
    assert!(short.is_inline());
    assert!(Str::new("a".repeat(INLINE_CAPACITY)).is_inline());
    assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
    assert_eq!(std::mem::size_of::<Str>(), 32);
    assert_eq!(Str::new_static("AAPL"), short);
    assert_eq!(Str::default(), "");

    // Equality, order and hash read the characters alone, so a value is one
    // value whichever column holds it.
    let latin = short
        .clone()
        .try_with_parameters(StringParameters::new(
            StringLayout::LargeString,
            Charset::Cp1252,
        ))
        .unwrap();
    assert_eq!(latin, short);
    assert_eq!(latin, "AAPL");
    assert_eq!("AAPL", latin);
    assert_eq!(latin, String::from("AAPL"));
    assert_eq!(latin.charset(), Charset::Cp1252);
    assert_eq!(latin.layout(), StringLayout::LargeString);
    assert_ne!(format!("{latin:?}"), format!("{short:?}"));
    assert_eq!(latin.to_string(), "AAPL");
    let mut members = std::collections::BTreeMap::new();
    members.insert(latin, 1);
    assert_eq!(members.get("AAPL"), Some(&1));

    // The conversions every string API leans on.
    assert_eq!(Str::from(String::from("x")), "x");
    assert_eq!(Str::from('x'), "x");
    assert_eq!(String::from(Str::new("x")), "x");
    assert_eq!("a b".parse::<Str>().unwrap(), "a b");
    assert_eq!(["a", "b"].into_iter().collect::<Str>(), "ab");
    assert_eq!(&*Str::new("deref"), "deref");
    assert_eq!(Scalar::from("x"), Scalar::String(Str::new("x")));

    // A maximum is never carried: the value answers its layout alone.
    let bounded = Str::new("USD")
        .try_with_parameters(StringParameters::default().try_with_bound(4).unwrap())
        .unwrap();
    assert_eq!(bounded.parameters(), StringParameters::default());
    assert!(
        Str::new("EURO!")
            .try_with_parameters(StringParameters::default().try_with_bound(4).unwrap())
            .is_err()
    );
    // A fixed width is: the value pads to it on the way out.
    let fixed = Str::new("USD\0")
        .try_with_parameters(
            StringParameters::default()
                .with_layout(StringLayout::FixedString)
                .try_with_bound(4)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(fixed, "USD");
    assert_eq!(fixed.fixed(), Some(4));
    assert_eq!(fixed.encode().unwrap().as_ref(), b"USD\0");
    assert_eq!(fixed.encoded_len(), 4);
    assert_eq!(fixed.dtype().unwrap(), DataType::fixed_utf8(4).unwrap());
    assert!(
        Str::new("x")
            .try_with_parameters(StringParameters::utf8(StringLayout::FixedString))
            .is_err()
    );
}

#[test]
fn a_string_datatype_survives_both_serde_doors() {
    // One `string` tag for every string, with only what it declares beside
    // it: the layout when not `string`, the charset when not UTF-8, and the
    // bound under the reading its layout gives it.
    for (spelling, json) in [
        ("utf8", r#"{"type":"string"}"#),
        ("utf8(32)", r#"{"type":"string","max":32}"#),
        (
            "fixed_utf8(8)",
            r#"{"type":"string","layout":"fixed_string","fixed":8}"#,
        ),
        ("large_utf8", r#"{"type":"string","layout":"large_string"}"#),
        ("utf8_view", r#"{"type":"string","layout":"string_view"}"#),
        (
            "large_utf8_view",
            r#"{"type":"string","layout":"large_string_view"}"#,
        ),
        ("ascii", r#"{"type":"string","charset":"us-ascii"}"#),
        (
            "fixed_ascii(4)",
            r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
        ),
        (
            "string(windows-1252)",
            r#"{"type":"string","charset":"windows-1252"}"#,
        ),
        (
            "fixed_string(windows-1252,8)",
            r#"{"type":"string","layout":"fixed_string","charset":"windows-1252","fixed":8}"#,
        ),
        (
            "string_view(windows-1252,32)",
            r#"{"type":"string","layout":"string_view","charset":"windows-1252","max":32}"#,
        ),
        (
            "large_string(iso-8859-15)",
            r#"{"type":"string","layout":"large_string","charset":"iso-8859-15"}"#,
        ),
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(dtype.clone().into_json().unwrap(), json, "{spelling}");
        assert_eq!(DataType::from_json(json).unwrap(), dtype, "{spelling}");
        assert_eq!(
            serde_json::from_str::<DataType>(&serde_json::to_string(&dtype).unwrap()).unwrap(),
            dtype,
            "{spelling}"
        );

        let value = dtype.clone().into_value();
        assert_eq!(DataType::from_value(value).unwrap(), dtype, "{spelling}");
    }

    // The retired tags are nobody's, and a bound the layout does not read is
    // refused.
    for retired in [
        r#"{"type":"utf8"}"#,
        r#"{"type":"large_utf8"}"#,
        r#"{"type":"utf8_view"}"#,
        r#"{"type":"ascii"}"#,
        r#"{"type":"fixed_ascii","width":4}"#,
        r#"{"type":"string","layout":"fixed_string"}"#,
        r#"{"type":"string","layout":"fixed_string","max":4}"#,
        r#"{"type":"string","fixed":4}"#,
        r#"{"type":"string","max":0}"#,
    ] {
        assert!(DataType::from_json(retired).is_err(), "{retired}");
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

    // The ordinary string still writes its characters and nothing else; a
    // value that declares more writes what it declares, and never a maximum.
    assert_eq!(
        serde_json::to_string(&Scalar::from("AAPL")).unwrap(),
        r#"{"type":"string","value":"AAPL"}"#
    );
    assert_eq!(
        serde_json::to_string(
            &DataType::from_str("utf8(32)")
                .unwrap()
                .scalar("AAPL")
                .unwrap()
        )
        .unwrap(),
        r#"{"type":"string","value":"AAPL"}"#
    );
    assert_eq!(
        serde_json::to_string(&DataType::fixed_ascii(4).unwrap().scalar("USD").unwrap()).unwrap(),
        r#"{"type":"string","value":{"layout":"fixed_string","charset":"us-ascii","fixed":4,"text":"USD"}}"#
    );
    assert_eq!(
        serde_json::to_string(&DataType::ascii().scalar("USD").unwrap()).unwrap(),
        r#"{"type":"string","value":{"layout":"string","charset":"us-ascii","text":"USD"}}"#
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
fn a_cast_into_another_charset_re_encodes_and_refuses_what_it_cannot_spell() {
    use arrow_array::{Array, ArrayRef, BinaryArray, StringArray};
    use std::sync::Arc;
    use yggdryl::{ArrowCast, ArrowCastOptions};

    // Arrow's kernel would hand a UTF-8 buffer to a windows-1252 column and
    // call it a framing change, and the column would read back as mojibake.
    // The string cast re-encodes every cell instead, and a scalar the
    // charset has no byte for is refused naming the charset, the row and
    // the column.
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), None]));
    let target = DataType::from_str("string(windows-1252)")
        .unwrap()
        .nullable_field("value");
    let cast = target
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = cast.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(cast.value(0), &[0x63, 0x61, 0x66, 0xE9]);
    assert!(cast.is_null(1));

    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), Some("東京")]));
    let refusal = target
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("windows-1252"), "{refusal}");
    assert!(refusal.contains("row 1"), "{refusal}");
    assert!(refusal.contains("\"value\""), "{refusal}");
    // Under `safe`, the cell that cannot be spelled becomes null.
    let lenient = target
        .cast_arrow_array(source, ArrowCastOptions::new())
        .unwrap();
    let lenient = lenient.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(lenient.value(0), &[0x63, 0x61, 0x66, 0xE9]);
    assert!(lenient.is_null(1));

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
