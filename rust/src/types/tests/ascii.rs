use std::cmp::Ordering;

use arrow_array::{Array, FixedSizeBinaryArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::{DataTypeId, DataTypeKind};
use crate::{Error, Field, Scalar, Scheme};

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
        .expect("fixed-width ASCII storage")
}

#[test]
fn every_spelling_parses_and_displays_as_its_datatype() {
    for (spelling, dtype) in [
        ("ascii", DataType::Ascii),
        ("ASCII", DataType::Ascii),
        // A fixed width is exactly what it says, at any length.
        ("ascii(1)", DataType::FixedAscii(1)),
        ("ascii(3)", DataType::FixedAscii(3)),
        ("ascii(6)", DataType::FixedAscii(6)),
        ("ascii(12)", DataType::FixedAscii(12)),
        ("ascii(64)", DataType::FixedAscii(64)),
        ("country", DataType::Country),
        ("Country", DataType::Country),
        ("currency", DataType::Currency),
        ("Currency", DataType::Currency),
        ("mic", DataType::Mic),
        ("MIC", DataType::Mic),
        ("Exchange", DataType::Mic),
        ("cfi", DataType::Cfi),
        ("CFI", DataType::Cfi),
        ("MonthYear", DataType::FixedAscii(8)),
        ("TZTimeOnly", DataType::FixedAscii(16)),
    ] {
        let parsed: DataType = spelling
            .parse()
            .unwrap_or_else(|error| panic!("{spelling} must parse: {error}"));
        assert_eq!(parsed, dtype, "{spelling}");
        // One canonical spelling: every alias displays as the datatype,
        // and that display re-parses to the same value.
        assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
    }
    let row: DataType = "struct<ccy: currency, isin: ascii(12), code: cfi, iso: country>"
        .parse()
        .unwrap();
    assert_eq!(
        row.get_field_by_path("ccy").map(Field::dtype),
        Some(&DataType::Currency)
    );
    assert_eq!(
        row.get_field_by_path("isin").map(Field::dtype),
        Some(&DataType::FixedAscii(12))
    );
    assert_eq!(
        row.get_field_by_path("code").map(Field::dtype),
        Some(&DataType::Cfi)
    );
    assert_eq!(
        row.get_field_by_path("iso").map(Field::dtype),
        Some(&DataType::Country)
    );
}

#[test]
fn a_width_of_no_bytes_is_refused_by_name() {
    let error = "ascii(0)".parse::<DataType>().unwrap_err().to_string();
    assert!(
        error.contains("expected an ASCII width of at least 1 byte, got 0"),
        "{error}"
    );
    // Bare `ascii` is the variable shape, not a missing width.
    assert_eq!("ascii".parse::<DataType>().unwrap(), DataType::Ascii);
    assert!(matches!(
        "ascii()".parse::<DataType>(),
        Err(Error::Parse { .. })
    ));

    // A width above the packed limit is a legal column and simply has no
    // packed integer.
    assert_eq!(DataType::ascii(17).unwrap(), DataType::FixedAscii(17));
    assert!(DataType::FixedAscii(17).ascii_packed(b"USD").is_err());
    assert!(DataType::ascii(-1).is_err());
}

#[test]
fn a_registered_code_is_its_own_datatype_over_its_standard_width() {
    // A code parses to itself and displays as itself: there is no width
    // hiding behind the name, and no second spelling of either.
    for (name, dtype, width) in DataType::CODES {
        assert_eq!(name.parse::<DataType>().unwrap(), *dtype);
        assert_eq!(dtype.to_string(), *name);
        assert_eq!(dtype.name(), *name);
        assert_eq!(dtype.ascii_width(), Some(*width));
        assert_eq!(dtype.kind(), DataTypeKind::Ascii);
        assert!(dtype.is_code());
    }
    assert_eq!("currency".parse::<DataType>().unwrap(), DataType::Currency);
    assert_ne!(DataType::Currency, DataType::FixedAscii(3));
    // ISO 10962 is six characters, and `cfi` stores exactly those six.
    assert_eq!(DataType::Cfi.ascii_width(), Some(6));
    // A width of six bytes is spellable, and it is still not a CFI code.
    assert_eq!(DataType::ascii(6).unwrap(), DataType::FixedAscii(6));
    assert_ne!(DataType::Cfi, DataType::FixedAscii(6));
    assert!(!DataType::FixedAscii(6).is_code());

    // A code name is a grammar keyword like every other, so the parser
    // reads it case-insensitively and trimmed.
    assert_eq!(
        " CURRENCY ".parse::<DataType>().unwrap(),
        DataType::Currency
    );
    // The grammar reports a word that names nothing as unknown.
    let error = "isin".parse::<DataType>().unwrap_err().to_string();
    assert!(error.contains("unknown datatype \"isin\""), "{error}");
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
        DataType::FixedAscii(3).ascii_packed(b"USD").unwrap()
    );
    assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
    assert_eq!(
        DataType::Cfi.ascii_packed(b"ESVUFR").unwrap(),
        0x4553_5655_4652
    );
    let refused = DataType::Country
        .ascii_packed(b"USD")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");

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
            .merge_with(&DataType::FixedAscii(3), true)
            .unwrap(),
        DataType::FixedAscii(3)
    );
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::Country, true)
            .unwrap(),
        DataType::FixedAscii(3)
    );
    assert_eq!(
        DataType::Currency
            .merge_with(&DataType::Utf8, true)
            .unwrap(),
        DataType::Utf8
    );
}

#[test]
fn serde_and_the_structural_value_round_trip() {
    // The structural encoding names every parameter, so a fixed width
    // carries its width beside the tag, exactly as a fixed binary does.
    for width in [1, 3, 4, 6, 12, 16, 64] {
        let dtype = DataType::ascii(width).unwrap();
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(json, format!(r#"{{"type":"fixed_ascii","width":{width}}}"#));
        assert_eq!(DataType::from_json(&json).unwrap(), dtype);

        let value = dtype.clone().into_value();
        assert_eq!(
            value.get_key_str("type").and_then(Scalar::as_str),
            Some("fixed_ascii")
        );
        assert_eq!(DataType::from_value(value).unwrap(), dtype);
    }

    // The variable shape takes no parameter at all.
    let json = DataType::Ascii.into_json().unwrap();
    assert_eq!(json, r#"{"type":"ascii"}"#);
    assert_eq!(DataType::from_json(&json).unwrap(), DataType::Ascii);
}

#[test]
fn identity_kind_and_widths_answer_for_every_width() {
    // One identifier covers every fixed width, because the width is a
    // parameter of the datatype and not a variant of its own.
    for width in [1, 2, 3, 4, 6, 8, 12, 16, 64] {
        let dtype = DataType::ascii(width).unwrap();
        assert_eq!(dtype.id(), DataTypeId::FixedAscii);
        assert_eq!(dtype.kind(), DataTypeKind::Ascii);
        assert_eq!(dtype.name(), "fixed_ascii");
        assert_eq!(dtype.to_string(), format!("ascii({width})"));
        assert_eq!(dtype.ascii_width(), Some(width));
        assert!(dtype.is_ascii());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();
    }
    // The variable shape is the same family with no width at all.
    assert_eq!(DataType::Ascii.id(), DataTypeId::Ascii);
    assert_eq!(DataType::Ascii.kind(), DataTypeKind::Ascii);
    assert_eq!(DataType::Ascii.ascii_width(), None);
    assert!(DataType::Ascii.is_ascii());
    assert_eq!(DataType::Utf8.ascii_width(), None);
    assert_eq!(DataType::FixedSizeBinary(4).ascii_width(), None);
}

#[test]
fn ordering_and_hashing_are_consistent_for_every_width() {
    // The ASCII family sits after the variable text layouts, and one
    // fixed width orders against another by the bytes it stores.
    assert!(DataType::Utf8View < DataType::Ascii);
    assert!(DataType::Ascii < DataType::FixedAscii(2));
    assert!(DataType::FixedAscii(2) < DataType::FixedAscii(3));
    assert!(DataType::FixedAscii(3) < DataType::FixedAscii(4));
    assert!(DataType::FixedAscii(4) < DataType::FixedAscii(8));
    assert!(DataType::FixedAscii(8) < DataType::FixedAscii(12));
    assert!(DataType::FixedAscii(12) < DataType::FixedAscii(16));
    assert!(DataType::FixedAscii(16) < DataType::list(DataType::Utf8.nullable_field("item")));
    assert_eq!(
        DataType::FixedAscii(8).cmp(&DataType::FixedAscii(8)),
        Ordering::Equal
    );
    assert_eq!(
        hash_of(&DataType::FixedAscii(8)),
        hash_of(&DataType::FixedAscii(8))
    );
    assert_ne!(
        DataType::FixedAscii(4).stable_hash(),
        DataType::FixedAscii(8).stable_hash()
    );
    // A code and the width that holds it are two identities over one
    // storage, and the hash is the only thing telling three of the four
    // pairs apart at all.
    for (code, width) in [
        (DataType::Country, DataType::FixedAscii(2)),
        (DataType::Currency, DataType::FixedAscii(3)),
        (DataType::Mic, DataType::FixedAscii(4)),
        (DataType::Cfi, DataType::FixedAscii(8)),
    ] {
        assert_ne!(code.stable_hash(), width.stable_hash(), "{code}");
        assert_eq!(code.stable_hash(), code.clone().stable_hash(), "{code}");
    }
}

#[test]
fn the_default_is_the_empty_string_stored_as_all_nul() {
    for dtype in [
        DataType::FixedAscii(2),
        DataType::FixedAscii(3),
        DataType::FixedAscii(4),
        DataType::FixedAscii(8),
        DataType::FixedAscii(12),
        DataType::FixedAscii(16),
    ] {
        let exact_empty = dtype.scalar(Scalar::from("")).unwrap();
        assert_eq!(dtype.default_value().unwrap(), exact_empty);
        assert!(dtype.is_default_value(&Scalar::from("")).unwrap());
        assert!(!dtype.is_default_value(&Scalar::from("USD")).unwrap());

        let width = dtype.ascii_width().unwrap();
        let field = dtype.required_field("ccy");
        assert_eq!(field.default_value().unwrap(), exact_empty);
        let array = field.default_arrow_array().unwrap();
        assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(width));
        let stored = stored(array.as_ref());
        assert_eq!(stored.len(), 1);
        assert!(stored.value(0).iter().all(|byte| *byte == 0));
    }
}

#[test]
fn values_validate_and_canonicalize_under_the_one_ascii_rule() {
    let root = DataType::from_fields([DataType::FixedAscii(4).required_field("ccy")])
        .unwrap()
        .required_field("row");
    let row = |value: Scalar| Scalar::from_sequence([value]);
    let canonical = |value: Scalar| root.canonicalize_value(row(value)).unwrap();
    let fixed = |value: &str| {
        Scalar::Ascii(crate::types::AsciiFamily::FixedAscii(
            crate::types::FixedAscii::new(value, 4).unwrap(),
        ))
    };

    // Text inputs canonicalize to the exact fixed-width ASCII leaf.
    assert_eq!(canonical(Scalar::from("USD")), row(fixed("USD")));
    assert_eq!(canonical(Scalar::from("ABCD")), row(fixed("ABCD")));
    assert_eq!(canonical(Scalar::from("")), row(fixed("")));
    let exact = fixed("USD");
    assert_eq!(canonical(exact.clone()), row(exact));
    // Trailing NULs are trimmed, and bytes are rewritten to that leaf.
    assert_eq!(canonical(Scalar::from("USD\0")), row(fixed("USD")));
    assert_eq!(
        canonical(Scalar::from(b"USD\0".to_vec())),
        row(fixed("USD"))
    );
    assert_eq!(canonical(Scalar::from(b"EUR".to_vec())), row(fixed("EUR")));

    // Every refusal names the width and the offending fact.
    for (value, fact) in [
        (Scalar::from("EURO!"), "5 bytes"),
        (Scalar::from(b"ABCDE".to_vec()), "5 bytes"),
        (Scalar::from("\u{20AC}"), "non-ASCII byte 0xE2 at 0"),
        (Scalar::from("U\0D"), "NUL byte at 1"),
    ] {
        let refused = root
            .validate_value(&row(value.clone()))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("ASCII text of at most 4 bytes"),
            "{refused}"
        );
        assert!(refused.contains(fact), "{refused}");
        assert!(refused.contains("ccy"), "{refused}");
        assert!(root.canonicalize_value(row(value)).is_err());
    }
    // Anything that is neither text nor bytes is refused by kind.
    let refused = root
        .validate_value(&row(Scalar::from(7)))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("expected fixed_ascii"), "{refused}");
}

#[test]
fn arrow_storage_is_padded_and_reads_back_trimmed() {
    let field = DataType::FixedAscii(8).nullable_field("code");
    let array = crate::arrow::scalar_array(&field, &Scalar::from("ABC")).unwrap();
    assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(8));
    assert_eq!(stored(array.as_ref()).value(0), b"ABC\0\0\0\0\0");
    assert_eq!(
        crate::arrow::scalar_value(&field, array.as_ref()).unwrap(),
        field.dtype().scalar(Scalar::from("ABC")).unwrap()
    );

    // Padded bytes, a null, and the empty string, through the array boundary.
    let values = Scalar::from_sequence([
        Scalar::from(b"XY\0\0\0\0\0\0".to_vec()),
        Scalar::Null,
        Scalar::from(""),
    ]);
    let array = crate::arrow::array_from_value(&field, &values).unwrap();
    let fixed = stored(array.as_ref());
    assert_eq!(fixed.len(), 3);
    assert_eq!(fixed.value(0), b"XY\0\0\0\0\0\0");
    assert!(fixed.is_null(1));
    assert_eq!(fixed.value(2), &[0; 8]);
    let read = |index: usize| {
        crate::arrow::value::value_from_array(field.dtype(), array.as_ref(), index).unwrap()
    };
    assert_eq!(read(0), field.dtype().scalar(Scalar::from("XY")).unwrap());
    assert_eq!(read(2), field.dtype().scalar(Scalar::from("")).unwrap());

    // What does not fit is refused at this boundary too.
    assert!(crate::arrow::scalar_array(&field, &Scalar::from("ABCDEFGHI")).is_err());
}

#[test]
fn compatibility_reads_every_width_as_utf8() {
    let schema = DataType::from_fields([
        DataType::FixedAscii(4).nullable_field("ccy"),
        DataType::FixedAscii(16).required_field("code"),
    ])
    .unwrap()
    .required_field("row");
    for scheme in [
        Scheme::SPARK,
        Scheme::POLARS,
        Scheme::PANDAS,
        Scheme::ICEBERG,
    ] {
        let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
        assert_eq!(compat["ccy"].dtype(), &DataType::Utf8);
        assert_eq!(compat["code"].dtype(), &DataType::Utf8);
        assert!(!compat["code"].is_nullable());
    }
    assert_eq!(
        schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
        schema
    );
    assert_eq!(
        DataType::FixedAscii(8)
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
        DataType::Utf8
    );
}
