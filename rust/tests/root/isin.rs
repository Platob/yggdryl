//! `rust/src/isin.rs`: the ISO 6166 securities identification number, and
//! how two statements of one instrument's number fold.
//!
//! The shape is what `new` admits - twelve characters, two letters, nine
//! alphanumerics and a digit - and whether the number closes on its check
//! digit and whether its prefix is one an agency numbers under are two
//! readings a value answers rather than two refusals: a masked or mistyped
//! number is a value of a lower rank, which every merge replaces by a real
//! one whatever the order, and only two numbers of one rank fold by the
//! order they were stated in. ISO 6166:2021 gives `ZZ` to derivatives
//! numbered before a country or the DSB's `EZ` numbers them, so a `ZZ`
//! number closes but is listed under no agency: rank one, below a real
//! number and above a number that does not close.

use yggdryl::{CodeValue, Isin};

/// The checksum-valid number eleven leading characters close to.
fn closed(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).expect("two letters and nine alphanumerics");
    Isin::new(format!("{body}{digit}")).unwrap()
}

/// A number eleven leading characters do not close to: the next digit.
fn open(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).expect("two letters and nine alphanumerics");
    Isin::new(format!("{body}{}", (digit + 1) % 10)).unwrap()
}

#[test]
fn the_shape_is_admitted_and_the_closing_and_the_listing_are_readings() {
    // A number that does not close is a value: a masked line's, a typo's.
    let masked = Isin::new("XX0000000001").unwrap();
    assert_eq!(masked.as_str(), "XX0000000001");
    assert!(!Isin::is_closed(masked.as_str()));
    assert!(!Isin::is_listed_prefix(masked.as_str()));
    assert_eq!(masked.rank(), 0);
    assert!(!masked.is_real());
    let typo = Isin::new("us0378331006").unwrap();
    assert_eq!(typo.as_str(), "US0378331006", "folded as every number is");
    assert!(!Isin::is_closed(typo.as_str()));
    assert!(Isin::is_listed_prefix(typo.as_str()));
    assert_eq!(typo.rank(), 1);
    // The shape is the refusal.
    for (text, reason) in [
        ("US037833100", "expected twelve characters"),
        ("U10378331005", "expected a two-letter prefix"),
        ("US03783310*5", "expected nine alphanumerics"),
        ("US037833100A", "expected a closing check digit"),
    ] {
        let refused = Isin::new(text).unwrap_err().to_string();
        assert!(refused.contains(reason), "{text}: {refused}");
        assert_eq!(Isin::rank_of(text), 0, "{text}");
    }

    // A real number closes under a listed prefix: a country's, or an
    // agency's - the DSB's `EZ`, the international `XS`.
    let apple = Isin::new("US0378331005").unwrap();
    assert!(Isin::is_closed(apple.as_str()));
    assert!(Isin::is_listed_prefix(apple.as_str()));
    assert_eq!(apple.rank(), 2);
    assert!(apple.is_real());
    assert_eq!(<Isin as CodeValue>::MAX_RANK, 2);
    for agency in ["EZ1234567AB", "XS020347015", "EU000000000"] {
        let number = closed(agency);
        assert!(Isin::is_listed_prefix(number.as_str()), "{agency}");
        assert_eq!(number.rank(), 2, "{agency}");
    }
    // A `ZZ` number closes but no agency lists it: one of the two.
    let provisional = closed("ZZ000A0B1C2");
    assert!(Isin::is_closed(provisional.as_str()));
    assert!(!Isin::is_listed_prefix(provisional.as_str()));
    assert_eq!(provisional.rank(), 1);
    assert_eq!(Isin::rank_of("ZZ0000000008"), 1);
    // The readings answer the text without building a value, and lower
    // case closes nothing: it is not how a column spells a number.
    assert_eq!(Isin::rank_of("US0378331005"), 2);
    assert_eq!(Isin::rank_of("us0378331005"), 0);
    assert!(!Isin::is_closed("us0378331005"));
}

#[test]
fn the_placeholder_is_the_lowest_number_there_is() {
    assert_eq!(Isin::NONE, "XX0000000000");
    let none = Isin::none();
    assert_eq!(none.as_str(), Isin::NONE);
    assert!(none.is_none());
    assert!(!Isin::new("XX0000000001").unwrap().is_none());
    assert_eq!(none.rank(), 0);
    assert!(!none.is_real());
    assert!(!Isin::is_closed(Isin::NONE));
    assert!(!Isin::is_listed_prefix(Isin::NONE));
    // Any stated number replaces it, whichever leads.
    let masked = Isin::new("XX0000000001").unwrap();
    assert_eq!(
        none.clone().merge_with(&masked),
        none,
        "two of rank zero: this one"
    );
    let typo = open("US037833100");
    assert_eq!(none.clone().merge_with(&typo), typo);
    assert_eq!(typo.clone().merge_with(&none), typo);
}

/// A higher rank wins whatever leads; among equals the leading one stands.
#[test]
fn a_real_number_replaces_a_lower_one_whichever_leads_and_equals_keep_the_leading_one() {
    let apple = Isin::new("US0378331005").unwrap();
    let microsoft = Isin::new("US5949181045").unwrap();
    let provisional = closed("ZZ000A0B1C2");
    let other_provisional = closed("ZZ000000001");
    let dsb = closed("EZ1234567AB");
    let typo = open("US037833100");
    let masked = Isin::new("XX0000000001").unwrap();
    assert_eq!(provisional.prefix(), "ZZ");
    assert_eq!(dsb.prefix(), "EZ");

    // The placeholder takes the real number, whichever real number it is.
    assert_eq!(provisional.clone().merge_with(&apple), apple);
    assert_eq!(provisional.clone().merge_with(&dsb), dsb);
    // A real number never takes the placeholder.
    assert_eq!(apple.clone().merge_with(&provisional), apple);
    assert_eq!(dsb.clone().merge_with(&provisional), dsb);
    // Two placeholders, or two real numbers, are two statements and this one
    // stands: a DSB number and a country's are both real.
    assert_eq!(
        provisional.clone().merge_with(&other_provisional),
        provisional
    );
    assert_eq!(apple.clone().merge_with(&microsoft), apple);
    assert_eq!(dsb.clone().merge_with(&apple), dsb);
    assert_eq!(apple.clone().merge_with(&dsb), apple);
    // A typo under a listed prefix yields to the real number in either
    // order, and a masked number - closing nowhere, listed nowhere - yields
    // to a typo, to a provisional number and to a real one.
    for (lower, higher) in [
        (&typo, &apple),
        (&masked, &typo),
        (&masked, &provisional),
        (&masked, &apple),
    ] {
        assert_eq!(lower.clone().merge_with(higher), *higher);
        assert_eq!(higher.clone().merge_with(lower), *higher);
    }
    // Two of one rank: this one. A typo under a listed prefix and a closing
    // `ZZ` number each have one of the two readings, so neither leads the
    // other by rank and the order decides.
    let other_typo = open("US594918104");
    assert_eq!(typo.clone().merge_with(&other_typo), typo);
    assert_eq!(typo.clone().merge_with(&provisional), typo);
    assert_eq!(provisional.clone().merge_with(&typo), provisional);
}

#[test]
fn intake_folds_the_case_and_a_column_holds_the_upper_case_shape() {
    let provisional = closed("ZZ000000001");
    assert!(Isin::is_canonical(provisional.as_str()));
    assert_eq!(
        Isin::new(provisional.as_str().to_ascii_lowercase()).unwrap(),
        provisional
    );
    // Canonical is the spelling a column holds - upper case, the shape -
    // and says nothing of the check digit, which is `closes`' question.
    assert!(Isin::is_canonical("XX0000000001"));
    assert!(!Isin::is_canonical("xx0000000001"));
    assert!(!Isin::is_canonical("XX000000000"));
    assert!(!Isin::is_canonical("US037833100A"));
}

/// The wire contract of `isin` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_isin_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("isin").unwrap();
    let value = dtype.scalar("US0378331005").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x78).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x78);
    assert_eq!(dtype.id().as_str(), "isin");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "isin");
    assert_eq!(value.kind(), "isin");
    assert_eq!(value.id().as_u8(), 0x78);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(12));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("US0378331005")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "isin");
    assert_eq!(Field::from_str("value isin").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("isin").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["isin"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.isin"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.isin")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["US0378331005"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"isin"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"isin"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"isin"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"isin"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"isin","value":"US0378331005"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"isin","value":"US0378331005"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(
        bytes,
        [
            0, 120, 0, 12, 85, 83, 48, 51, 55, 56, 51, 51, 49, 48, 48, 53
        ]
    );
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("US0378331005"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 10610353165993888946);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(10610353165993888946)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(7045775035903662255)
    );
    assert_eq!(dtype.stable_hash(), 17548004354234589734);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `isin`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_isin_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("isin").unwrap();
        let value = dtype.scalar("US0378331005").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            2004564884531700211
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            1090472907199677283
        );
    }
}
