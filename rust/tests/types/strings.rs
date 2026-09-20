use std::cmp::Ordering;

use arrow_array::{Array, FixedSizeBinaryArray};
use arrow_schema::DataType as ArrowDataType;

use yggdryl::{Charset, DataTypeId, DataTypeKind, StructType};
use yggdryl::{DataType, Str, StringType};
use yggdryl::{Error, Field, Scalar, Scheme};

fn hash_of(value: &DataType) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn stored(array: &dyn Array) -> &FixedSizeBinaryArray {
    array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .expect("fixed-width string storage")
}

/// A US-ASCII string bounded to `max` bytes: the sized leaf.
fn bounded_ascii(max: u32) -> DataType {
    DataType::sized_ascii(max).unwrap()
}

#[test]
fn every_spelling_parses_and_displays_as_its_datatype() {
    for (spelling, dtype) in [
        ("ascii", DataType::ascii()),
        ("ASCII", DataType::ascii()),
        ("string(us-ascii)", DataType::ascii()),
        ("utf8", DataType::utf8()),
        ("string", DataType::utf8()),
        ("large_utf8", DataType::large_utf8()),
        ("utf8_view", DataType::utf8_view()),
        // A number after the plain spelling is a maximum, which is the
        // sized leaf.
        ("ascii(3)", bounded_ascii(3)),
        ("string(us-ascii,3)", bounded_ascii(3)),
        ("sized_ascii(3)", bounded_ascii(3)),
        // A fixed width is exactly what it says, at any length.
        ("fixed_ascii(1)", DataType::fixed_ascii(1).unwrap()),
        ("fixed_ascii(3)", DataType::fixed_ascii(3).unwrap()),
        (
            "fixed_string(us-ascii,6)",
            DataType::fixed_ascii(6).unwrap(),
        ),
        ("fixed_ascii(12)", DataType::fixed_ascii(12).unwrap()),
        ("fixed_ascii(64)", DataType::fixed_ascii(64).unwrap()),
        ("fixed_utf8(8)", DataType::fixed_utf8(8).unwrap()),
        ("char(8)", DataType::fixed_utf8(8).unwrap()),
        ("country", DataType::Country),
        ("Country", DataType::Country),
        ("currency", DataType::Currency),
        ("Currency", DataType::Currency),
        ("mic", DataType::MicCode),
        ("MIC", DataType::MicCode),
        ("Exchange", DataType::MicCode),
        ("cfi", DataType::CfiCode),
        ("CFI", DataType::CfiCode),
        ("MonthYear", DataType::fixed_ascii(8).unwrap()),
    ] {
        let parsed: DataType = spelling
            .parse()
            .unwrap_or_else(|error| panic!("{spelling} must parse: {error}"));
        assert_eq!(parsed, dtype, "{spelling}");
        // One canonical spelling: every alias displays as the datatype,
        // and that display re-parses to the same value.
        assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
    }
    let row: DataType =
        "struct<ccy: currency, isin: fixed_ascii(12), code: cfi, iso: country, name: utf8(32)>"
            .parse()
            .unwrap();
    assert_eq!(
        row.get_field_by_path("ccy").map(Field::dtype),
        Some(&DataType::Currency)
    );
    assert_eq!(
        row.get_field_by_path("isin").map(Field::dtype),
        Some(&DataType::fixed_ascii(12).unwrap())
    );
    assert_eq!(
        row.get_field_by_path("code").map(Field::dtype),
        Some(&DataType::CfiCode)
    );
    assert_eq!(
        row.get_field_by_path("iso").map(Field::dtype),
        Some(&DataType::Country)
    );
    assert_eq!(
        row.get_field_by_path("name")
            .and_then(|field| field.dtype().string_parameters())
            .and_then(StringType::max),
        Some(32)
    );
}

#[test]
fn a_width_of_no_bytes_is_refused_by_name() {
    let error = "fixed_ascii(0)"
        .parse::<DataType>()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("expected a width of at least one byte, got 0"),
        "{error}"
    );
    let error = "ascii(0)".parse::<DataType>().unwrap_err().to_string();
    assert!(
        error.contains("expected a width of at least one byte, got 0"),
        "{error}"
    );
    // Bare `ascii` is the variable shape, not a missing width, and empty
    // parentheses are that spelling with punctuation.
    assert_eq!("ascii".parse::<DataType>().unwrap(), DataType::ascii());
    assert_eq!("ascii()".parse::<DataType>().unwrap(), DataType::ascii());
    // A width is what makes a string fixed, so there is no width-free spelling.
    assert!(matches!(
        "fixed_ascii".parse::<DataType>(),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        "fixed_ascii()".parse::<DataType>(),
        Err(Error::Parse { .. })
    ));
    assert!(
        DataType::String(StringType::FixedAsciiString(0))
            .validate()
            .is_err()
    );

    // A width above the packed limit is a legal column and simply has no
    // packed integer.
    assert_eq!(
        DataType::fixed_ascii(17).unwrap(),
        "fixed_ascii(17)".parse::<DataType>().unwrap()
    );
    assert!(
        DataType::fixed_ascii(17)
            .unwrap()
            .ascii_packed(b"USD")
            .is_err()
    );
    assert!(DataType::fixed_ascii(0).is_err());
    assert!(DataType::fixed_utf8(0).is_err());
}

#[test]
fn a_registered_code_is_its_own_datatype_over_its_standard_width() {
    // A code parses to itself and displays as itself: there is no width
    // hiding behind the name, and no second spelling of either.
    for (name, dtype, width) in DataType::CODES {
        assert_eq!(name.parse::<DataType>().unwrap(), *dtype);
        assert_eq!(dtype.to_string(), *name);
        assert_eq!(dtype.name(), *name);
        // The width bounds the value; it is not a layout, so a code names
        // no fixed width.
        assert_eq!(dtype.code_width(), Some(*width));
        assert_eq!(dtype.fixed_byte_width(), None);
        assert_eq!(dtype.kind(), DataTypeKind::Code);
        assert!(dtype.is_code());
        assert!(!dtype.is_string());
        assert_eq!(dtype.string_parameters(), None);
    }
    assert_eq!("currency".parse::<DataType>().unwrap(), DataType::Currency);
    assert_ne!(DataType::Currency, DataType::fixed_ascii(3).unwrap());
    // ISO 10962 is six characters, and `cfi` holds at most those six.
    assert_eq!(DataType::CfiCode.code_width(), Some(6));
    // A width of six bytes is spellable, and it is still not a CFI code.
    assert_ne!(DataType::CfiCode, DataType::fixed_ascii(6).unwrap());
    assert!(!DataType::fixed_ascii(6).unwrap().is_code());
    // ISO 6166 is twelve characters closed by a check digit, and `isin`
    // stores exactly those twelve.
    assert_eq!("isin".parse::<DataType>().unwrap(), DataType::IsinCode);
    assert_eq!(DataType::IsinCode.code_width(), Some(12));
    assert_ne!(DataType::IsinCode, DataType::fixed_ascii(12).unwrap());
    // A CUSIP is nine and a SEDOL seven, each closed by its own check digit,
    // and neither is the ASCII width that would hold the same text.
    assert_eq!("cusip".parse::<DataType>().unwrap(), DataType::CusipCode);
    assert_eq!(DataType::CusipCode.code_width(), Some(9));
    assert_ne!(DataType::CusipCode, DataType::fixed_ascii(9).unwrap());
    assert_eq!("sedol".parse::<DataType>().unwrap(), DataType::SedolCode);
    assert_eq!(DataType::SedolCode.code_width(), Some(7));
    assert_ne!(DataType::SedolCode, DataType::fixed_ascii(7).unwrap());

    // A code name is a grammar keyword like every other, so the parser
    // reads it case-insensitively and trimmed.
    assert_eq!(
        " CURRENCY ".parse::<DataType>().unwrap(),
        DataType::Currency
    );
    // FIGI is its own checked twelve-character code, not a Bloomberg alias.
    assert_eq!(" FIGI ".parse::<DataType>().unwrap(), DataType::FIGICode);
    // The grammar still reports words that name nothing as unknown.
    let error = "figx".parse::<DataType>().unwrap_err().to_string();
    assert!(error.contains("unknown datatype \"figx\""), "{error}");
}

#[test]
fn a_code_packs_and_merges_by_the_ascii_rules() {
    // The packed integer is the value's own storage bytes, exactly as it
    // is for a width: the code is a datatype, not a second encoding.
    assert_eq!(
        DataType::Currency.ascii_packed(b"USD").unwrap(),
        0x0055_5344
    );
    assert_eq!(DataType::Currency.ascii_value(0x0055_5344).unwrap(), "USD");
    assert_eq!(
        DataType::Currency.ascii_packed(b"USD").unwrap(),
        DataType::fixed_ascii(3)
            .unwrap()
            .ascii_packed(b"USD")
            .unwrap()
    );
    assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
    assert_eq!(
        DataType::CfiCode.ascii_packed(b"ESVUFR").unwrap(),
        0x4553_5655_4652
    );
    let refused = DataType::Country
        .ascii_packed(b"USD")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");
    // Only a fixed US-ASCII width packs: a maximum, another charset and a
    // width past sixteen bytes are refused by name.
    assert!(bounded_ascii(3).ascii_packed(b"USD").is_err());
    assert!(
        DataType::fixed_utf8(3)
            .unwrap()
            .ascii_packed(b"USD")
            .is_err()
    );

    // Two schemas that agree on a code keep it; a code reconciled with
    // anything else answers the plain text both fit in.
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::Currency, true)
            .unwrap(),
        DataType::Currency
    );
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::fixed_ascii(3).unwrap(), true)
            .unwrap(),
        DataType::from_str("ascii(3)").unwrap()
    );
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::Country, true)
            .unwrap(),
        DataType::from_str("ascii(3)").unwrap()
    );
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::utf8(), true)
            .unwrap(),
        DataType::utf8()
    );
}

#[test]
fn serde_and_the_structural_value_round_trip() {
    // One `string` tag for every string: the leaf is written where it is
    // not `utf8` and the number beside it; the leaf's name says its charset,
    // so a fixed width carries the leaf and the width beside the tag.
    for width in [1, 3, 4, 6, 12, 16, 64] {
        let dtype = DataType::fixed_ascii(width).unwrap();
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(
            json,
            format!(r#"{{"type":"string","layout":"fixed_ascii","fixed":{width}}}"#)
        );
        assert_eq!(DataType::from_json(&json).unwrap(), dtype);

        let value = dtype.clone().into_value();
        assert_eq!(
            value.get_key_str("type").and_then(Scalar::as_str),
            Some("string")
        );
        assert_eq!(
            value.get_key_str("fixed").and_then(Scalar::as_i128),
            Some(i128::from(width))
        );
        assert_eq!(DataType::from_value(value).unwrap(), dtype);
    }

    // The variable shape states its leaf and nothing else; plain UTF-8 is
    // the bare tag.
    for (dtype, json) in [
        (DataType::ascii(), r#"{"type":"string","layout":"ascii"}"#),
        (DataType::utf8(), r#"{"type":"string"}"#),
        (
            DataType::large_utf8(),
            r#"{"type":"string","layout":"large_utf8"}"#,
        ),
        (
            bounded_ascii(3),
            r#"{"type":"string","layout":"sized_ascii","max":3}"#,
        ),
        (
            "fixed_string(windows-1252,8)".parse().unwrap(),
            r#"{"type":"string","layout":"fixed_cp1252","fixed":8}"#,
        ),
    ] {
        assert_eq!(dtype.clone().into_json().unwrap(), json);
        assert_eq!(DataType::from_json(json).unwrap(), dtype);
        assert_eq!(
            DataType::from_value(dtype.clone().into_value()).unwrap(),
            dtype
        );
    }
    // An older document spelled the shape beside a charset, and still
    // reads as the leaf it meant.
    for (document, dtype) in [
        (
            r#"{"type":"string","charset":"us-ascii"}"#,
            DataType::ascii(),
        ),
        (
            r#"{"type":"string","charset":"us-ascii","max":3}"#,
            bounded_ascii(3),
        ),
        (
            r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
            DataType::fixed_ascii(4).unwrap(),
        ),
    ] {
        assert_eq!(DataType::from_json(document).unwrap(), dtype, "{document}");
    }

    // The retired tags name nothing.
    for tag in ["utf8", "large_utf8", "utf8_view", "ascii", "fixed_ascii"] {
        assert!(
            DataType::from_json(&format!(r#"{{"type":"{tag}","width":4}}"#)).is_err(),
            "{tag}"
        );
    }
    // A fixed leaf with no width, and a bound on the wrong side, are refused.
    assert!(DataType::from_json(r#"{"type":"string","layout":"fixed_ascii"}"#).is_err());
    assert!(DataType::from_json(r#"{"type":"string","layout":"fixed_string"}"#).is_err());
    assert!(DataType::from_json(r#"{"type":"string","fixed":4}"#).is_err());
}

#[test]
fn identity_kind_and_widths_answer_for_every_width() {
    // One identifier covers every fixed width, because the width is the
    // leaf's number and not a leaf of its own.
    for width in [1, 2, 3, 4, 6, 8, 12, 16, 64] {
        let dtype = DataType::fixed_ascii(width).unwrap();
        assert_eq!(dtype.id(), DataTypeId::FixedAsciiString);
        assert_eq!(dtype.kind(), DataTypeKind::Text);
        assert_eq!(dtype.name(), "fixed_ascii");
        assert_eq!(dtype.to_string(), format!("fixed_ascii({width})"));
        assert_eq!(dtype.fixed_byte_width(), Some(width as usize));
        assert_eq!(dtype.charset(), Some(Charset::Ascii));
        assert!(dtype.is_string());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();
    }
    // The variable shape is the same family with no width at all.
    assert_eq!(DataType::ascii().id(), DataTypeId::AsciiString);
    assert_eq!(bounded_ascii(3).id(), DataTypeId::SizedAsciiString);
    assert_eq!(DataType::ascii().kind(), DataTypeKind::Text);
    assert_eq!(DataType::ascii().fixed_byte_width(), None);
    assert_eq!(DataType::ascii().charset(), Some(Charset::Ascii));
    assert!(DataType::ascii().is_string());
    assert_eq!(DataType::utf8().fixed_byte_width(), None);
    assert_eq!(DataType::utf8().charset(), Some(Charset::Utf8));
    assert_eq!(bounded_ascii(3).fixed_byte_width(), None);
    assert_eq!(
        DataType::fixed_binary(4).unwrap().fixed_byte_width(),
        Some(4)
    );
    assert_eq!(DataType::binary().charset(), None);
}

#[test]
fn ordering_and_hashing_are_consistent_for_every_width() {
    // Every string is one variant, ordered by leaf - the six UTF-8 shapes,
    // then the six US-ASCII ones, then windows-1252, the number ordering
    // within a leaf; the codes follow the strings and the nested datatypes
    // follow the codes.
    assert!(DataType::utf8() < DataType::large_utf8());
    assert!(DataType::large_utf8() < DataType::utf8_view());
    assert!(DataType::utf8_view() < DataType::fixed_utf8(8).unwrap());
    assert!(DataType::fixed_utf8(8).unwrap() < DataType::ascii());
    assert!(DataType::ascii() < DataType::fixed_ascii(2).unwrap());
    assert!(DataType::fixed_ascii(2).unwrap() < DataType::fixed_ascii(3).unwrap());
    assert!(DataType::fixed_ascii(3).unwrap() < DataType::fixed_ascii(4).unwrap());
    assert!(DataType::fixed_ascii(4).unwrap() < DataType::fixed_ascii(8).unwrap());
    assert!(DataType::fixed_ascii(8).unwrap() < DataType::fixed_ascii(12).unwrap());
    assert!(DataType::fixed_ascii(12).unwrap() < DataType::fixed_ascii(16).unwrap());
    assert!(DataType::fixed_ascii(16).unwrap() < bounded_ascii(3));
    assert!(bounded_ascii(3) < DataType::cp1252());
    assert!(DataType::cp1252() < DataType::Country);
    assert!(DataType::Country < DataType::list(DataType::utf8().nullable_field("item")));
    assert_eq!(
        DataType::fixed_ascii(8)
            .unwrap()
            .cmp(&DataType::fixed_ascii(8).unwrap()),
        Ordering::Equal
    );
    assert_eq!(
        hash_of(&DataType::fixed_ascii(8).unwrap()),
        hash_of(&DataType::fixed_ascii(8).unwrap())
    );
    assert_ne!(
        DataType::fixed_ascii(4).unwrap().stable_hash(),
        DataType::fixed_ascii(8).unwrap().stable_hash()
    );
    // A code and the width that would hold it are two identities over the
    // same characters, and the hash is what tells them apart.
    for (code, width) in [
        (DataType::Country, DataType::fixed_ascii(2).unwrap()),
        (DataType::Currency, DataType::fixed_ascii(3).unwrap()),
        (DataType::MicCode, DataType::fixed_ascii(4).unwrap()),
        (DataType::CfiCode, DataType::fixed_ascii(6).unwrap()),
    ] {
        assert_ne!(code.stable_hash(), width.stable_hash(), "{code}");
        assert_eq!(code.stable_hash(), code.clone().stable_hash(), "{code}");
    }
}

#[test]
fn the_default_is_the_empty_string_stored_as_all_nul() {
    for dtype in [
        DataType::fixed_ascii(2).unwrap(),
        DataType::fixed_ascii(3).unwrap(),
        DataType::fixed_ascii(4).unwrap(),
        DataType::fixed_ascii(8).unwrap(),
        DataType::fixed_ascii(12).unwrap(),
        DataType::fixed_ascii(16).unwrap(),
        DataType::fixed_utf8(8).unwrap(),
    ] {
        let exact_empty = dtype.scalar(Scalar::from("")).unwrap();
        assert_eq!(dtype.default_value().unwrap(), exact_empty);
        assert!(dtype.is_default_value(&Scalar::from("")).unwrap());
        assert!(!dtype.is_default_value(&Scalar::from("USD")).unwrap());
        // The default is the empty text under the column's own parameters,
        // and its stored bytes are the padded slot.
        let Scalar::String(text) = &exact_empty else {
            panic!("a string default");
        };
        assert_eq!(text.parameters(), dtype.string_parameters().unwrap());
        let width = dtype.fixed_byte_width().unwrap();
        assert_eq!(text.encode().unwrap().len(), width);

        let field = dtype.required_field("ccy");
        assert_eq!(field.default_value().unwrap(), exact_empty);
        let array = field.default_arrow_array().unwrap();
        assert_eq!(
            array.data_type(),
            &ArrowDataType::FixedSizeBinary(i32::try_from(width).unwrap())
        );
        let stored = stored(array.as_ref());
        assert_eq!(stored.len(), 1);
        assert!(stored.value(0).iter().all(|byte| *byte == 0));
    }
    // A variable string defaults to the empty text under its charset.
    let empty = DataType::ascii().default_value().unwrap();
    assert_eq!(empty, Scalar::from(""));
    let Scalar::String(text) = &empty else {
        panic!("a string default");
    };
    assert_eq!(text.charset(), Charset::Ascii);
}

#[test]
fn values_validate_and_canonicalize_under_the_one_ascii_rule() {
    let dtype = DataType::fixed_ascii(4).unwrap();
    let root = StructType::from_fields([dtype.clone().required_field("ccy")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let row = |value: Scalar| Scalar::from_sequence([value]);
    let canonical = |value: Scalar| root.canonicalize_value(row(value)).unwrap();
    let fixed = |value: &str| {
        Scalar::String(
            Str::new(value)
                .try_with_parameters(dtype.string_parameters().unwrap())
                .unwrap(),
        )
    };

    // Text inputs canonicalize to the fixed-width US-ASCII value.
    assert_eq!(canonical(Scalar::from("USD")), row(fixed("USD")));
    assert_eq!(canonical(Scalar::from("ABCD")), row(fixed("ABCD")));
    assert_eq!(canonical(Scalar::from("")), row(fixed("")));
    let exact = fixed("USD");
    assert_eq!(canonical(exact.clone()), row(exact));
    // Trailing NULs are trimmed, and bytes are rewritten to that value.
    assert_eq!(canonical(Scalar::from("USD\0")), row(fixed("USD")));
    assert_eq!(
        canonical(Scalar::from(b"USD\0".to_vec())),
        row(fixed("USD"))
    );
    assert_eq!(canonical(Scalar::from(b"EUR".to_vec())), row(fixed("EUR")));
    // The canonical value carries the column's parameters, not the text's.
    let restated = canonical(Scalar::from("USD"));
    let Some(Scalar::String(held)) = restated.get(0) else {
        panic!("a string cell");
    };
    assert_eq!(held.fixed(), Some(4));
    assert_eq!(held.charset(), Charset::Ascii);

    // Every refusal names the offending fact and the column.
    for (value, fact) in [
        (Scalar::from("EURO!"), "at most 4 bytes"),
        (Scalar::from(b"ABCDE".to_vec()), "at most 4 bytes"),
        (Scalar::from("\u{20AC}"), "non-ASCII byte 0xE2 at 0"),
        (Scalar::from("U\0D"), "NUL byte at 1"),
    ] {
        let refused = root
            .validate_value(&row(value.clone()))
            .unwrap_err()
            .to_string();
        assert!(refused.contains(fact), "{refused}");
        assert!(refused.contains("ccy"), "{refused}");
        assert!(root.canonicalize_value(row(value)).is_err());
    }
    // A value that spells no text at all is refused by kind.
    let refused = root
        .validate_value(&row(Scalar::from_sequence([Scalar::from(7)])))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("expected fixed_ascii"), "{refused}");

    // A fixed value carries its own padded width, so one written at another
    // width is restated at the column's rather than kept as it arrived: the
    // column declares the storage, and the value must name the same one.
    let wider = Scalar::String(
        Str::new("USD")
            .try_with_parameters(
                DataType::fixed_ascii(8)
                    .unwrap()
                    .string_parameters()
                    .unwrap(),
            )
            .unwrap(),
    );
    root.validate_value(&row(wider.clone())).unwrap();
    assert_eq!(canonical(wider), row(fixed("USD")));

    // The variable US-ASCII layout keeps a NUL-free value as it is and
    // refuses the repertoire the same way.
    let ascii = DataType::ascii();
    assert_eq!(ascii.scalar("USD").unwrap(), Scalar::from("USD"));
    assert!(ascii.scalar("\u{20AC}").is_err());
    assert!(ascii.scalar("U\0D").is_err());
    assert!(bounded_ascii(3).scalar("EURO").is_err());
    assert_eq!(bounded_ascii(3).scalar("USD").unwrap(), Scalar::from("USD"));
}

#[test]
fn arrow_storage_is_padded_and_reads_back_trimmed() {
    let field = DataType::fixed_ascii(8).unwrap().nullable_field("code");
    let array = yggdryl::arrow::scalar_array(&field, &Scalar::from("ABC")).unwrap();
    assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(8));
    assert_eq!(stored(array.as_ref()).value(0), b"ABC\0\0\0\0\0");
    assert_eq!(
        yggdryl::arrow::scalar_value(&field, array.as_ref()).unwrap(),
        field.dtype().scalar(Scalar::from("ABC")).unwrap()
    );

    // Padded bytes, a null, and the empty string, through the array boundary.
    let values = Scalar::from_sequence([
        Scalar::from(b"XY\0\0\0\0\0\0".to_vec()),
        Scalar::Null,
        Scalar::from(""),
    ]);
    let array = yggdryl::arrow::array_from_value(&field, &values).unwrap();
    let fixed = stored(array.as_ref());
    assert_eq!(fixed.len(), 3);
    assert_eq!(fixed.value(0), b"XY\0\0\0\0\0\0");
    assert!(fixed.is_null(1));
    assert_eq!(fixed.value(2), &[0; 8]);
    // One row at a time through the public boundary: a one-element slice is
    // what `scalar_value` reads, and slicing costs no copy.
    let read = |index: usize| {
        yggdryl::arrow::scalar_value(&field, array.slice(index, 1).as_ref()).unwrap()
    };
    assert_eq!(read(0), field.dtype().scalar(Scalar::from("XY")).unwrap());
    assert_eq!(read(2), field.dtype().scalar(Scalar::from("")).unwrap());

    // What does not fit is refused at this boundary too.
    assert!(yggdryl::arrow::scalar_array(&field, &Scalar::from("ABCDEFGHI")).is_err());

    // The variable US-ASCII layout rides Arrow's own text storage.
    let field = DataType::ascii().nullable_field("code");
    let array = yggdryl::arrow::scalar_array(&field, &Scalar::from("ABC")).unwrap();
    assert_eq!(array.data_type(), &ArrowDataType::Utf8);
}

#[test]
fn compatibility_reads_every_width_as_str() {
    let schema = StructType::from_fields([
        DataType::fixed_ascii(4).unwrap().nullable_field("ccy"),
        DataType::fixed_ascii(16).unwrap().required_field("code"),
        bounded_ascii(3).required_field("bounded"),
        "string(windows-1252)"
            .parse::<DataType>()
            .unwrap()
            .required_field("latin"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    for scheme in [
        Scheme::SPARK,
        Scheme::POLARS,
        Scheme::PANDAS,
        Scheme::ICEBERG,
    ] {
        let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
        assert_eq!(compat["ccy"].dtype(), &DataType::utf8());
        assert_eq!(compat["code"].dtype(), &DataType::utf8());
        assert_eq!(compat["bounded"].dtype(), &DataType::utf8());
        assert_eq!(compat["latin"].dtype(), &DataType::utf8());
        assert!(!compat["code"].is_nullable());
    }
    assert_eq!(
        schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
        schema
    );
    assert_eq!(
        DataType::fixed_ascii(8)
            .unwrap()
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
        DataType::utf8()
    );
    assert_eq!(
        DataType::utf8().into_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::utf8()
    );
}
