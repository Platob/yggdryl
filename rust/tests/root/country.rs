//! `rust/src/country.rs`: ISO 3166-1's two-letter country code, held by
//! its shape and ranked by the listing.

use yggdryl::{Ccy, CodeValue, Country, DataType, Scalar, StringEnum};

#[test]
fn a_country_is_at_most_two_ascii_bytes_and_the_listing_is_a_rank() {
    assert_eq!(<Country as CodeValue>::WIDTH, 2);
    assert_eq!(DataType::Country.code_width(), Some(2));
    let listed = Country::new("CH").unwrap();
    assert!(listed.is_listed());
    assert_eq!(listed.rank(), 1);
    assert!(listed.is_real());
    assert_eq!(<Country as CodeValue>::MAX_RANK, 1);
    // A code the registry does not assign - the user-assigned `XX`, the
    // transitional `AN` - is a value of rank zero, never a refusal.
    for unlisted in ["XX", "AN", "ZZ", ""] {
        let held = Country::new(unlisted).unwrap();
        assert_eq!(held.as_str(), unlisted);
        assert!(!held.is_listed(), "{unlisted}");
        assert_eq!(held.rank(), 0, "{unlisted}");
        assert_eq!(
            DataType::Country.scalar(unlisted).unwrap(),
            Scalar::from(held),
            "{unlisted}"
        );
    }
    // Every listed code ranks one, and the listing is the sorted one.
    for code in StringEnum::COUNTRIES {
        assert!(Country::new(code).unwrap().is_listed(), "{code}");
    }
    // The width is the refusal.
    let refused = Country::new("CHE").unwrap_err().to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");
}

/// A listed country replaces an unlisted one whichever leads; two listed
/// ones keep the leading one.
#[test]
fn a_listed_country_replaces_an_unlisted_one_whichever_leads() {
    let listed = Country::new("CH").unwrap();
    let other = Country::new("US").unwrap();
    let masked = Country::new("XX").unwrap();
    let other_masked = Country::new("ZZ").unwrap();
    assert_eq!(masked.clone().merge_with(&listed), listed);
    assert_eq!(listed.clone().merge_with(&masked), listed);
    assert_eq!(listed.clone().merge_with(&other), listed);
    assert_eq!(other.clone().merge_with(&listed), other);
    assert_eq!(masked.clone().merge_with(&other_masked), masked);
}

/// A country answers the one legal tender ISO 4217 list one gives it: a
/// fund code never, and where list one gives two tenders the one the
/// generator's override table names.
#[test]
fn a_country_answers_the_one_legal_tender_list_one_gives_it() {
    for (country, currency) in [
        ("US", "USD"),
        ("CH", "CHF"),
        ("DE", "EUR"),
        ("GB", "GBP"),
        ("JP", "JPY"),
        // A territory using another country's tender.
        ("LI", "CHF"),
        ("AX", "EUR"),
        // Two tenders in list one: the override table's choice.
        ("SV", "USD"),
        ("PA", "PAB"),
        ("LS", "LSL"),
        ("VE", "VES"),
    ] {
        assert_eq!(
            Country::new(country).unwrap().currency(),
            Some(Ccy::new(currency).unwrap()),
            "{country}"
        );
    }
    // None where list one gives no currency: the user-assigned codes, an
    // agency prefix, a country with no universal currency, a spelling no
    // code is.
    for none in ["XX", "ZZ", "XS", "EU", "AQ", "PS", "GS", "ch", ""] {
        assert_eq!(Country::new(none).unwrap().currency(), None, "{none}");
    }
    // Every listed country has a tender but the three list one gives no
    // universal currency.
    let unanswered: Vec<&str> = StringEnum::COUNTRIES
        .iter()
        .copied()
        .filter(|code| Country::new(code).unwrap().currency().is_none())
        .collect();
    assert_eq!(unanswered, ["AQ", "GS", "PS"]);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::country::country_currency;
    use yggdryl::{Ccy, Country, StringEnum};

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_country_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("country").unwrap();
        let value = dtype.scalar("FR").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            11399152777318276282
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            16434823388470558038
        );
    }

    /// The generated table is sorted by the alpha-2 code with each code once,
    /// what the binary search needs; every code is listed, every currency is
    /// three upper-case letters ISO 4217 lists, and every row is what
    /// `currency` answers.
    #[test]
    fn the_generated_currency_table_is_sorted_unique_and_what_currency_answers() {
        let rows = country_currency();
        assert!(
            rows.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "sorted by the alpha-2 code, each code once"
        );
        for &(country, currency) in rows {
            let code = Country::new(country).unwrap();
            assert!(code.is_listed(), "{country}");
            assert!(
                currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()),
                "{country}: {currency}"
            );
            assert!(
                StringEnum::CURRENCIES.contains(&currency),
                "{country}: {currency}"
            );
            assert_eq!(
                code.currency(),
                Some(Ccy::new(currency).unwrap()),
                "{country}"
            );
        }
    }
}

/// The wire contract of `country` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_country_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("country").unwrap();
    let value = dtype.scalar("FR").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x71).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x71);
    assert_eq!(dtype.id().as_str(), "country");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "country");
    assert_eq!(value.kind(), "country");
    assert_eq!(value.id().as_u8(), 0x71);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(2));
    assert_eq!(value.code_storage().map(|held| held.as_str()), Some("FR"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "country");
    assert_eq!(Field::from_str("value country").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("country").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["country"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.country"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.country")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["FR"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"country"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"country"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"country"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"country"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"country","value":"FR"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"country","value":"FR"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 113, 0, 2, 70, 82]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("FR")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 7299401977006624818);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(7299401977006624818)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(11123011540160199249)
    );
    assert_eq!(dtype.stable_hash(), 749182048769006606);
}
