//! `rust/src/code.rs`: the coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod datatypes {
    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use std::sync::Arc;
    use yggdryl::FieldValue as _;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar, Serie,
        StringEnum, StructType,
    };
    use yggdryl::{
        CcyField, CfiCodeField, CountryField, DxFeedExchangeFeed, MicCode, MicCodeField,
    };

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    fn text(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    /// The ten codes, each with its width and one value its standard names.
    const CODED: [(&str, DataType, usize, &str); 11] = [
        ("country", DataType::Country, 2, "US"),
        ("ccy", DataType::Ccy, 3, "USD"),
        ("mic", DataType::MicCode, 4, "XPAR"),
        ("cfi", DataType::CfiCode, 6, "ESVUFR"),
        ("isin", DataType::IsinCode, 12, "US0378331005"),
        ("cusip", DataType::CusipCode, 9, "037833100"),
        ("sedol", DataType::SedolCode, 7, "B0YBKJ7"),
        ("figi", DataType::FIGICode, 12, "BBG000BLNQ16"),
        ("side", DataType::Side, 8, "BUY"),
        ("state", DataType::State, 10, "20NEW"),
        ("timeinforce", DataType::TimeInForce, 8, "0"),
    ];

    #[test]
    fn dxfeed_exchange_codes_resolve_under_the_feed_that_gives_them_meaning() {
        use DxFeedExchangeFeed::{Cboe, Cme, CtaUtp, NasdaqBasic, NyseBqt, Otc, UsOptions};

        let mappings = [
            (
                CtaUtp,
                "A:XASE B:XBOS C:XCIS D:FINR F:TXSE G:24EQ H:EPRL I:XISE J:EDGA K:EDGX L:LTSE M:XCHI N:XNYS P:ARCX Q:XNAS U:MEMX V:IEXG W:CBSX X:XPSX Y:BATY Z:BATS",
            ),
            (Cboe, "A:EDGA X:EDGX Y:BATY Z:BATS"),
            (NasdaqBasic, "B:XBOS F:FINC L:FINN Q:XNAS X:XPSX"),
            (NyseBqt, "A:XASE C:XCIS D:FINY M:XCHI N:XNYS O:GOTC P:ARCX"),
            (Otc, "U:OOTC V:OTCM"),
            (
                UsOptions,
                "A:XASE B:XBOX C:XCBO D:EMLD E:EDGO H:GMNI I:XISX J:MCRY M:XMIO N:ARCO P:MPRL Q:XNDQ S:SPHR T:XBXO U:MXOP W:C2OX X:XPHO Z:BATO",
            ),
        ];
        for (feed, mappings) in mappings {
            for mapping in mappings.split_ascii_whitespace() {
                let (exchange, mic) = mapping.split_once(':').unwrap();
                assert_eq!(
                    MicCode::from_dxfeed_exchange_code(feed, exchange)
                        .unwrap()
                        .as_str(),
                    mic,
                    "{feed:?} {exchange}"
                );
            }
        }

        for (feed, exchange) in [(Cboe, "C"), (Cboe, "U"), (Cme, "G"), (Cme, "B")] {
            let error = MicCode::from_dxfeed_exchange_code(feed, exchange)
                .unwrap_err()
                .to_string();
            assert!(error.contains(exchange), "{error}");
            assert!(error.contains("no single MIC"), "{error}");
        }
        let error = MicCode::from_dxfeed_exchange_code(CtaUtp, "R")
            .unwrap_err()
            .to_string();
        assert!(error.contains("CTA/UTP"), "{error}");
        assert!(error.contains("R"), "{error}");
    }

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
            // A code is an identity over a registry, not a string with a charset.
            assert_eq!(dtype.id().as_str(), *name, "{name}");
            assert_eq!(dtype.kind(), DataTypeKind::Code, "{name}");
            // The width bounds a value; a code stores as the text it is, so no
            // code claims a fixed layout.
            assert_eq!(dtype.code_width(), Some(*width), "{name}");
            assert_eq!(dtype.id().code_width(), Some(*width), "{name}");
            assert_eq!(dtype.fixed_byte_width(), None, "{name}");
            assert_eq!(dtype.id().fixed_byte_width(), None, "{name}");
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
            assert!(value.is_code(), "{name}");
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
            let arrow = field.clone().into_arrow_field().unwrap();
            assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field, "{name}");

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
                    &DataType::serie(DataType::Int64.nullable_field("item")),
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
        let side = DataType::Side.scalar(Scalar::from("BUY")).unwrap();
        assert!(matches!(side, Scalar::Side(_)));
        assert_eq!(side.as_str(), Some("BUY"));
        assert_eq!(DataType::Side.scalar(side.clone()).unwrap(), side);
        // A side is read by its spelling: FIX's wire code and the
        // specification's name reach the same explicit value.
        assert_eq!(DataType::Side.scalar(Scalar::from("1")).unwrap(), side);
        assert_eq!(DataType::Side.scalar(Scalar::from("Buy")).unwrap(), side);

        // Packing is the crate's fixed-ASCII packing at the code's own width:
        // NUL-padded up to it, the padding gone on the way back. The padding is
        // the packing's; the column stores no padding at all.
        assert_eq!(
            DataType::Side.ascii_packed(b"BUY").unwrap(),
            DataType::fixed_ascii(8)
                .unwrap()
                .ascii_packed(b"BUY")
                .unwrap()
        );
        for (dtype, value) in [(DataType::Side, "BUY"), (DataType::Side, "SSHORTEX")] {
            let packed = dtype.ascii_packed(value.as_bytes()).unwrap();
            let read = dtype.ascii_value(packed).unwrap();
            assert_eq!(read.as_str(), value, "{dtype} {value}");
        }

        // A spelling that names no side is refused by name: the width bounds
        // the value read, never the spelling, so a long name still reads.
        let refused = DataType::Side
            .scalar(Scalar::from("TOOLONGSIDE"))
            .unwrap_err();
        assert!(refused.to_string().contains("side"), "{refused}");
        assert_eq!(
            DataType::Side
                .scalar(Scalar::from("SellShortExempt"))
                .unwrap()
                .as_str(),
            Some("SSHORTEX")
        );
        // A time in force is the text it is, at its width, and a value longer
        // than the width is the refusal any fixed-ASCII field gives.
        let refused = DataType::TimeInForce
            .scalar(Scalar::from("TOOLONGTIF"))
            .unwrap_err();
        assert!(refused.to_string().contains("8 bytes"), "{refused}");
    }

    #[test]
    fn a_cast_into_a_code_stores_the_text_and_reading_it_back_keeps_it() {
        let venue = Field::new("venue", DataType::MicCode, false);
        let stored = Serie::from_arrow_array(
            Some(&venue),
            text(&["XPAR", "XLON"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "XPAR");
        assert_eq!(cells.value(1), "XLON");

        // A value shorter than the width stores as itself: there is no slot to
        // fill, so nothing is padded and nothing has to be trimmed back.
        let short = Serie::from_arrow_array(
            Some(&venue),
            text(&["BX"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let short = short.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(short.value(0), "BX");
        assert_eq!(short.value_length(0), 2);

        // A fixed-width column is still a spelling a cast reads, and the padding
        // its slot wrote is the slot's rather than the value's.
        let slots: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_iter([b"BX\0\0".to_vec()].into_iter()).unwrap(),
        );
        let trimmed = Serie::from_arrow_array(
            Some(&venue),
            slots,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        assert_eq!(
            trimmed
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "BX"
        );

        let row = root([venue.clone()]);
        let batch = RecordBatch::try_new(row.into_arrow_schema().unwrap(), vec![stored]).unwrap();
        let as_text = root([DataType::utf8().required_field("venue")]);
        let trimmed = Serie::from_arrow_batch(
            Some(&as_text),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        let trimmed = trimmed
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(trimmed.value(0), "XPAR");
        assert_eq!(trimmed.value(1), "XLON");

        // The refusal names the code's own width, not the next ASCII one up.
        let refused = Serie::from_arrow_array(
            Some(&venue),
            text(&["XPARIS"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
    }

    #[test]
    fn a_listing_is_a_vocabulary_and_never_a_gate_on_the_value() {
        // A time in force declares a vocabulary exactly as `MicCode` does: a value no
        // version defines is held rather than refused.
        let (dtype, outside) = (DataType::TimeInForce, "X");
        let stored = dtype.scalar(Scalar::from(outside)).unwrap();
        assert_eq!(stored.as_str(), Some(outside), "{dtype}");
        let packed = dtype.ascii_packed(outside.as_bytes()).unwrap();
        assert_eq!(dtype.ascii_value(packed).unwrap().as_str(), outside);
        // A side is the explicit values and nothing else, read by spelling as a
        // state is: a letter no version defines is refused rather than stored.
        let refused = DataType::Side.scalar(Scalar::from("Z")).unwrap_err();
        assert!(refused.to_string().contains("side"), "{refused}");
        assert_eq!(
            DataType::Side.scalar(Scalar::from("5")).unwrap().as_str(),
            Some("SSHORT")
        );

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
    fn a_code_carries_its_identity_into_equality_and_order() {
        // Two codes whose bytes agree are two values: the identity compares
        // first, then the text, so a side and a time in force never collide in
        // a set or sort beside each other.
        let side = DataType::Side.scalar(Scalar::from("BUY")).unwrap();
        let tif = DataType::TimeInForce.scalar(Scalar::from("BUY")).unwrap();
        assert_eq!(side.as_str(), tif.as_str());
        assert_ne!(side, tif);
        assert_ne!(side.cmp(&tif), std::cmp::Ordering::Equal);
        assert_eq!(side, DataType::Side.scalar(Scalar::from("BUY")).unwrap());

        // And a code is not the string of the same characters.
        assert_ne!(side, Scalar::from("BUY"));
        assert_ne!(
            DataType::Ccy.scalar(Scalar::from("USD")).unwrap(),
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
    fn every_code_stores_as_the_text_it_is_under_its_own_extension() {
        for (name, dtype, width, sample) in &CODED {
            let field = Field::new("code", dtype.clone(), false);
            let arrow = field.clone().into_arrow_field().unwrap();

            assert_eq!(arrow.data_type(), &ArrowDataType::Utf8, "{name}");
            assert_eq!(
                arrow.metadata()["ARROW:extension:name"],
                format!("yggdryl.{name}"),
                "{name}"
            );
            assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "", "{name}");
            // The identity round-trips: the same bytes come back the same code.
            assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field, "{name}");

            // A value is stored as exactly its own bytes, and text cast into the
            // column becomes the same cell.
            let value = dtype.scalar(Scalar::from(*sample)).unwrap();
            let stored = Serie::from_scalars(field.clone(), [value.clone()])
                .unwrap()
                .require_arrow_array()
                .unwrap();
            let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
            assert_eq!(cells.value(0), *sample, "{name}");
            assert_eq!(
                usize::try_from(cells.value_length(0)).unwrap(),
                sample.len(),
                "{name}"
            );
            assert!(sample.len() <= *width, "{name}");
            assert_eq!(
                Serie::from_arrow_array(
                    Some(&field),
                    Arc::clone(&stored),
                    ArrowCastOptions::default()
                )
                .unwrap()
                .scalar(0)
                .unwrap(),
                value,
                "{name}"
            );
            let cast = Serie::from_arrow_array(
                Some(&field),
                text(&[sample]),
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap()
            .require_arrow_array()
            .unwrap();
            assert_eq!(cast.as_ref(), stored.as_ref(), "{name}");

            // And the column's own storage ingests without rewriting or
            // refusing: the plan asks `is_code`, so a code added to the listing
            // is planned without being named again.
            let again = Serie::from_arrow_array(
                Some(&field),
                Arc::clone(&stored),
                ArrowCastOptions::new().with_safe(false),
            )
            .and_then(|serie| Ok(serie.require_arrow_array()?))
            .unwrap_or_else(|error| panic!("{name} did not ingest its own bytes: {error}"));
            assert_eq!(again.as_ref(), stored.as_ref(), "{name}");

            // A value past the width is refused at the code's own width, naming
            // the row it was in: the width is a bound the value rule keeps even
            // though no layout enforces it any more.
            let over = "X".repeat(*width + 1);
            let refused = Serie::from_arrow_array(
                Some(&field),
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
    fn a_code_and_the_text_that_holds_it_are_not_the_same_column() {
        let currency = Field::new("ccy", DataType::Ccy, false);
        let bounded = Field::new("ccy", DataType::from_str("ascii(3)").unwrap(), false);

        // Identical storage, different identity, so neither imports as the other:
        // the extension *name* is what separates them, never the storage.
        let currency_arrow = currency.clone().into_arrow_field().unwrap();
        let bounded_arrow = bounded.clone().into_arrow_field().unwrap();
        assert_eq!(currency_arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(bounded_arrow.data_type(), &ArrowDataType::Utf8);
        assert_ne!(currency_arrow.metadata(), bounded_arrow.metadata());
        assert_eq!(Field::from_arrow_field(&currency_arrow).unwrap(), currency);
        assert_eq!(Field::from_arrow_field(&bounded_arrow).unwrap(), bounded);

        // The same text under no extension at all stays plain text.
        let plain = arrow_schema::Field::new("ccy", ArrowDataType::Utf8, false);
        assert_eq!(
            Field::from_arrow_field(&plain).unwrap().dtype(),
            &DataType::utf8()
        );

        // A code's own name over a storage it does not lay out is not that code
        // either: it stays the storage it is.
        let mismatched = arrow_schema::Field::new("ccy", ArrowDataType::FixedSizeBinary(3), false)
            .with_metadata(
                [
                    ("ARROW:extension:name".to_owned(), "yggdryl.ccy".to_owned()),
                    ("ARROW:extension:metadata".to_owned(), String::new()),
                ]
                .into_iter()
                .collect(),
            );
        assert_eq!(
            Field::from_arrow_field(&mismatched).unwrap().dtype(),
            &DataType::fixed_binary(3).unwrap()
        );

        for storage in [
            ArrowDataType::Utf8,
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int32),
                Box::new(ArrowDataType::Utf8),
            ),
            ArrowDataType::FixedSizeBinary(3),
        ] {
            let retired = arrow_schema::Field::new("ccy", storage, false).with_metadata(
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
            let refusal = Field::from_arrow_field(&retired)
                .expect_err("the retired extension is not anonymous storage");
            let message = refusal.to_string();
            assert!(message.contains("ccy"), "names the field: {message}");
            assert!(message.contains("yggdryl.currency"), "{message}");
            assert!(message.contains("yggdryl.ccy"), "{message}");
        }
    }

    #[test]
    fn the_typed_field_and_scalar_aliases_name_their_code() {
        let ccy = CcyField::unit("ccy", false);
        let venue = MicCodeField::unit("venue", true);
        let iso = CountryField::unit("iso", true);
        let cfi = CfiCodeField::unit("classification", true);

        let ccy_field = ccy.to_field();
        let venue_field = venue.to_field();
        assert_eq!(ccy_field.dtype(), &DataType::Ccy);
        assert_eq!(venue_field.dtype(), &DataType::MicCode);
        assert_eq!(iso.to_field().dtype(), &DataType::Country);
        assert_eq!(cfi.to_field().dtype(), &DataType::CfiCode);

        // The pairing is the field's value contract, so the text becomes the code
        // leaf on the way in.
        let value = FieldScalar::new(&ccy_field, "USD").unwrap();
        assert_eq!(value.dtype(), &DataType::Ccy);
        assert_eq!(value.name(), "ccy");
        assert_eq!(value.as_str(), Some("USD"));
        assert_eq!(value.value().id(), DataTypeId::Ccy);

        // The leaf is the datatype's, so a width of the same size is not a code.
        let plain = Field::new("ccy", DataType::fixed_ascii(3).unwrap(), false);
        assert!(CcyField::try_from_field(plain).is_err());
        assert!(FieldScalar::new(&venue_field, "XPARIS").is_err());

        // A typed field's column is the text leaf its code stores as: the typed
        // read is a narrowing of the column that comes out, so a storage
        // change that the column did not follow is a narrowing failure here.
        let cells = Serie::from_arrow_array(
            Some(&ccy.clone().into_field()),
            text(&["USD", "EUR"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        let cells = cells.as_utf8().expect("a code column is UTF-8 text");
        assert_eq!(cells.value(0), Some("USD"));
        assert_eq!(cells.value(1), Some("EUR"));
        let one = Serie::from_arrow_array(
            Some(&venue.clone().into_field()),
            text(&["XPAR"]),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_scalar()
        .unwrap();
        assert_eq!(
            one.into_inner()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "XPAR"
        );
    }

    #[test]
    fn a_dictionary_encoded_code_keeps_its_identity_across_arrow() {
        // Arrow's dictionary holds a bare datatype for its values, so the field
        // is the only place the identity can ride - and a low-cardinality code
        // column is exactly the one a writer dictionary-encodes.
        for (name, dtype, _) in DataType::CODES {
            let encoded = DataType::dictionary(DataType::Int32, dtype.clone()).unwrap();
            let field = Field::new("code", encoded.clone(), false);
            let arrow = field.clone().into_arrow_field().unwrap();

            assert_eq!(
                arrow.metadata()["ARROW:extension:name"],
                format!("yggdryl.{name}"),
                "{name}"
            );
            assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field, "{name}");
        }

        // The width keeps its own identity the same way, and a dictionary of
        // anonymous bytes stays anonymous.
        let width = Field::new(
            "ccy",
            DataType::dictionary(DataType::Int32, DataType::fixed_ascii(3).unwrap()).unwrap(),
            false,
        );
        assert_eq!(
            Field::from_arrow_field(&width.clone().into_arrow_field().unwrap()).unwrap(),
            width
        );
        let plain = Field::new(
            "ccy",
            DataType::dictionary(DataType::Int32, DataType::fixed_binary(3).unwrap()).unwrap(),
            false,
        );
        assert_eq!(
            Field::from_arrow_field(&plain.clone().into_arrow_field().unwrap()).unwrap(),
            plain
        );
    }

    /// Every string and byte datatype a code column reaches, and what it reads
    /// back as.
    ///
    /// A code is ASCII text bounded by its standard's width, and that reading has
    /// to survive every layout, charset and bound the string family offers and
    /// every framing the byte family does - a code stores as text now, so both
    /// families are one cast away in each direction.
    #[test]
    fn a_code_column_reads_into_every_string_and_byte_datatype() {
        let strict = || ArrowCastOptions::new().with_safe(false);
        let ccy = Field::new("v", DataType::Ccy, false);
        let stored = Serie::from_arrow_array(Some(&ccy), text(&["USD"]), strict())
            .unwrap()
            .require_arrow_array()
            .unwrap();
        // The column carries `yggdryl.ccy`, which is what a reading reads it
        // under.
        let batch = RecordBatch::try_new(
            root([ccy.clone()]).into_arrow_schema().unwrap(),
            vec![Arc::clone(&stored)],
        )
        .unwrap();
        let into = |target: DataType| -> ArrayRef {
            Arc::clone(
                Serie::from_arrow_batch(
                    Some(&root([Field::new("v", target, false)])),
                    &batch,
                    strict(),
                )
                .unwrap()
                .into_arrow_batch()
                .unwrap()
                .column(0),
            )
        };

        for spelling in [
            "utf8",
            "large_utf8",
            "utf8_view",
            "ascii",
            "utf8(3)",
            "large_ascii",
            "fixed_ascii(3)",
            "string(windows-1252)",
            "binary",
            "large_binary",
            "binary_view",
            "fixed_binary(3)",
            "binary(3)",
        ] {
            let read = into(DataType::from_str(spelling).unwrap());
            let back = Serie::from_arrow_array(Some(&ccy), read, strict())
                .and_then(|serie| Ok(serie.require_arrow_array()?))
                .unwrap_or_else(|error| panic!("{spelling} does not read back: {error}"));
            assert_eq!(back.as_ref(), stored.as_ref(), "{spelling}");
        }

        // A width the value does not fill is a different payload either way, and
        // each refusal names both sides rather than leaving Arrow's builder to
        // complain about a slice length.
        for (spelling, expected) in [
            ("fixed_binary(8)", "exactly 8 bytes"),
            ("binary(2)", "at most 2 bytes"),
            ("utf8(2)", "at most 2"),
        ] {
            let refused = Serie::from_arrow_batch(
                Some(&root([Field::new(
                    "v",
                    DataType::from_str(spelling).unwrap(),
                    false,
                )])),
                &batch,
                strict(),
            )
            .unwrap_err()
            .to_string();
            assert!(refused.contains(expected), "{spelling}: {refused}");
            assert!(refused.contains("row 0"), "{spelling}: {refused}");
        }

        // And the same readings hold one value at a time: a code spells its text,
        // and that text's bytes are its payload.
        let value = DataType::Ccy.scalar(Scalar::from("USD")).unwrap();
        assert_eq!(
            DataType::utf8().scalar(value.clone()).unwrap().as_str(),
            Some("USD")
        );
        assert_eq!(
            DataType::binary().scalar(value.clone()).unwrap().as_bytes(),
            Some(b"USD".as_slice())
        );
        assert_eq!(
            DataType::Ccy.scalar(Scalar::from(b"USD".to_vec())).unwrap(),
            value
        );
    }

    #[test]
    fn a_code_merges_to_the_better_statement() {
        use yggdryl::{Ccy, CfiCode, CodeValue, IsinCode, MicCode, Side, State};

        // A classification fills what it left unknown from the other, and stands
        // as it is beside another instrument's.
        let partial = CfiCode::new("ESXXXR").unwrap();
        assert_eq!(
            partial
                .clone()
                .merge_with(&CfiCode::new("ESVUFX").unwrap())
                .as_str(),
            "ESVUFR"
        );
        assert_eq!(
            partial
                .merge_with(&CfiCode::new("DBFNFB").unwrap())
                .as_str(),
            "ESXXXR"
        );

        // A state that reached none takes the other, and otherwise the further
        // along stands whichever side it is on.
        let unknown = State::new("00UNKNOWN").unwrap();
        let new = State::read("New").unwrap();
        let filled = State::read("Filled").unwrap();
        assert_eq!(unknown.merge_with(&new), new);
        assert_eq!(new.clone().merge_with(&filled), filled);
        assert_eq!(filled.clone().merge_with(&new), filled);

        // A side, a currency and a market stated as none take the other, and
        // anything stated stands.
        assert_eq!(
            Side::read("UNKNOWN")
                .unwrap()
                .merge_with(&Side::read("1").unwrap())
                .as_str(),
            "BUY"
        );
        assert_eq!(
            Side::read("BUY")
                .unwrap()
                .merge_with(&Side::read("SELL").unwrap())
                .as_str(),
            "BUY"
        );
        assert_eq!(
            Ccy::new("XXX")
                .unwrap()
                .merge_with(&Ccy::new("USD").unwrap())
                .as_str(),
            "USD"
        );
        assert_eq!(
            Ccy::new("USD")
                .unwrap()
                .merge_with(&Ccy::new("EUR").unwrap())
                .as_str(),
            "USD"
        );
        assert_eq!(
            MicCode::new("XXXX")
                .unwrap()
                .merge_with(&MicCode::new("XPAR").unwrap())
                .as_str(),
            "XPAR"
        );

        // An identifier has nothing partial about it: this one stands.
        let apple = IsinCode::new("US0378331005").unwrap();
        assert_eq!(
            apple
                .clone()
                .merge_with(&IsinCode::new("US5949181045").unwrap()),
            apple
        );

        // `XXX` is the currency that states none, so the other one stands.
        let unstated = Ccy::new("XXX").unwrap();
        assert_eq!(
            unstated
                .clone()
                .merge_with(&Ccy::new("USD").unwrap())
                .as_str(),
            "USD"
        );
        // A code the other states nothing better than keeps what it had.
        let stated = Ccy::new("EUR").unwrap();
        assert_eq!(stated.clone().merge_with(&unstated), stated);
    }

    #[test]
    fn the_code_family_stands_for_every_registered_code() {
        use yggdryl::{
            BloombergCode, Ccy, CfiCode, Country, CusipCode, FIGICode, IsinCode, MicCode, SedolCode,
        };
        use yggdryl::{Side, State, TimeInForce};

        crate::scalar::assert_family_round_trip(
            vec![
                crate::family_leaf!(Country, Country::new("US").unwrap()),
                crate::family_leaf!(Ccy, Ccy::new("USD").unwrap()),
                crate::family_leaf!(MicCode, MicCode::new("XPAR").unwrap()),
                crate::family_leaf!(CfiCode, CfiCode::new("ESVUFR").unwrap()),
                crate::family_leaf!(Side, Side::new("BUY").unwrap()),
                crate::family_leaf!(State, State::new("20NEW").unwrap()),
                crate::family_leaf!(TimeInForce, TimeInForce::new("0").unwrap()),
                crate::family_leaf!(IsinCode, IsinCode::new("US0378331005").unwrap()),
                crate::family_leaf!(CusipCode, CusipCode::new("037833100").unwrap()),
                crate::family_leaf!(SedolCode, SedolCode::new("B0YBKJ7").unwrap()),
                crate::family_leaf!(BloombergCode, BloombergCode::new("BBG000B9XRY4").unwrap()),
                crate::family_leaf!(FIGICode, FIGICode::new("BBG000BLNQ16").unwrap()),
            ],
            DataTypeKind::Code,
            // The text a code is made of is not the code.
            &Scalar::from("USD"),
        );
    }
}

mod securities {
    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use std::sync::Arc;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Scalar, Serie, StructType,
        Term,
    };
    use yggdryl::{CusipCodeField, FIGICodeField, SedolCodeField};

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    fn text(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    fn cells(array: &ArrayRef) -> Vec<Option<&str>> {
        let text = array.as_any().downcast_ref::<StringArray>().unwrap();
        (0..text.len())
            .map(|index| text.is_valid(index).then(|| text.value(index)))
            .collect()
    }

    /// The two identifiers: the name, the datatype, the width, one identifier
    /// its standard closes, the same one in lower case, the same one with its
    /// check digit off by one, and one a character short.
    const IDENTIFIERS: [(&str, DataType, usize, &str, &str, &str, &str); 3] = [
        (
            "cusip",
            DataType::CusipCode,
            9,
            "38259P508",
            "38259p508",
            "38259P509",
            "38259P50",
        ),
        (
            "sedol",
            DataType::SedolCode,
            7,
            "B0YBKJ7",
            "b0ybkj7",
            "B0YBKJ8",
            "B0YBKJ",
        ),
        (
            "figi",
            DataType::FIGICode,
            12,
            "BBG000BLNQ16",
            "bbg000blnq16",
            "BBG000BLNQ17",
            "BBG000BLNQ1",
        ),
    ];

    #[test]
    fn each_identifier_is_a_registered_code_that_round_trips_everywhere() {
        for (name, dtype, width, sample, lower, typo, short) in &IDENTIFIERS {
            // The grammar, the identifier and the listing.
            assert_eq!(dtype.to_string(), *name, "{name}");
            assert_eq!(DataType::from_str(name).unwrap(), *dtype, "{name}");
            assert_eq!(DataType::from_str(&name.to_uppercase()).unwrap(), *dtype);
            assert_eq!(DataType::from_logical_name(name).unwrap(), *dtype);
            assert_eq!(DataTypeId::from_str(name).unwrap(), dtype.id(), "{name}");
            assert_eq!(dtype.id().as_str(), *name);
            assert_eq!(dtype.kind(), DataTypeKind::Code);
            assert_eq!(dtype.code_name(), Some(*name));
            assert_eq!(dtype.code_width(), Some(*width), "{name}");
            assert_eq!(dtype.fixed_byte_width(), None);
            assert!(dtype.is_code());
            assert!(!dtype.is_string());
            assert_ne!(*dtype, DataType::fixed_ascii(*width as u32).unwrap());
            assert!(
                DataType::CODES
                    .iter()
                    .any(|(held, listed, held_width)| held == name
                        && listed == dtype
                        && held_width == width),
                "{name}"
            );

            // Serde of the datatype, in both spellings.
            let json = dtype.clone().into_json().unwrap();
            assert_eq!(json, format!(r#"{{"type":"{name}"}}"#));
            assert_eq!(DataType::from_json(&json).unwrap(), *dtype);
            let rendered = serde_json::to_string(dtype).unwrap();
            assert_eq!(serde_json::from_str::<DataType>(&rendered).unwrap(), *dtype);

            // The value door: the check digit gates the space, the case folds,
            // and the stored value is the code under its own identity.
            let value = dtype.scalar(Scalar::from(*sample)).unwrap();
            assert!(value.is_code(), "{name}");
            assert_eq!(value.kind(), *name);
            assert_eq!(value.as_str(), Some(*sample));
            assert_eq!(value.dtype().unwrap(), *dtype);
            assert_eq!(dtype.scalar(Scalar::from(*lower)).unwrap(), value, "{name}");
            assert_eq!(dtype.scalar(value.clone()).unwrap(), value);
            let refused = dtype.scalar(Scalar::from(*typo)).unwrap_err().to_string();
            assert!(refused.contains("check digit"), "{refused}");
            let refused = dtype.scalar(Scalar::from(*short)).unwrap_err().to_string();
            assert!(refused.contains("characters"), "{refused}");
            // The wire shape is the identity over the text, and lower case on
            // the wire reads as the identifier it spells.
            let wire = serde_json::to_string(&value).unwrap();
            assert_eq!(wire, format!(r#"{{"type":"{name}","value":"{sample}"}}"#));
            assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);
            let folded = format!(r#"{{"type":"{name}","value":"{lower}"}}"#);
            assert_eq!(serde_json::from_str::<Scalar>(&folded).unwrap(), value);
            let broken = format!(r#"{{"type":"{name}","value":"{typo}"}}"#);
            assert!(serde_json::from_str::<Scalar>(&broken).is_err(), "{name}");

            // Arrow: the text storage under the code's own extension name, and
            // the identity back from it.
            let field = Field::new(*name, dtype.clone(), false);
            let arrow = field.clone().into_arrow_field().unwrap();
            assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
            assert_eq!(
                arrow.metadata()["ARROW:extension:name"],
                format!("yggdryl.{name}")
            );
            assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

            // The packing is at the code's own width, and there is no
            // vocabulary to prebuild for an open identifier space.
            assert_eq!(
                dtype.ascii_packed(sample.as_bytes()).unwrap(),
                DataType::fixed_ascii(*width as u32)
                    .unwrap()
                    .ascii_packed(sample.as_bytes())
                    .unwrap()
            );
            assert!(yggdryl::StringEnum::prebuilt_values(name).is_empty());
            assert!(
                yggdryl::StringEnum::from_logical_name(name)
                    .unwrap()
                    .is_empty()
            );
            // No neutral member: the empty text is not an identifier, so the
            // default is refused rather than invented.
            assert!(dtype.default_value().is_err(), "{name}");
        }
    }

    #[test]
    fn a_cast_into_an_identifier_holds_the_column_to_the_canonical_spelling() {
        for (name, dtype, _, sample, lower, typo, _) in &IDENTIFIERS {
            let field = Field::new("sid", dtype.clone(), true);
            // A column already holding the canonical spelling is the storage
            // itself, and reading it back is the identifier.
            let stored = Serie::from_arrow_array(
                Some(&field),
                text(&[sample]),
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap()
            .require_arrow_array()
            .unwrap();
            assert_eq!(cells(&stored), vec![Some(*sample)], "{name}");

            // Strict: a typo and a lower-case spelling are refused, naming the
            // row, the column and the rule. A column's bytes are what every
            // reader digests, so the cast lets in the canonical spelling only.
            for refused in [typo, lower] {
                let message = Serie::from_arrow_array(
                    Some(&field),
                    text(&[sample, refused]),
                    ArrowCastOptions::new().with_safe(false),
                )
                .unwrap_err()
                .to_string();
                assert!(message.contains("canonical spelling"), "{name}: {message}");
                assert!(message.contains("row 1 of column sid"), "{name}: {message}");
            }

            // Safe: the refused cell is null and the rest of the column stands.
            let safe = Serie::from_arrow_array(
                Some(&field),
                text(&[sample, typo, lower]),
                ArrowCastOptions::new().with_safe(true),
            )
            .unwrap()
            .require_arrow_array()
            .unwrap();
            assert_eq!(cells(&safe), vec![Some(*sample), None, None], "{name}");

            // A fixed-width slot is still a spelling the cast reads, with the
            // slot's padding trimmed.
            let width = i32::try_from(sample.len() + 2).unwrap();
            let padded = format!("{sample}\0\0");
            let slots: ArrayRef = Arc::new(
                FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                    [Some(padded.as_bytes())].into_iter(),
                    width,
                )
                .unwrap(),
            );
            let trimmed = Serie::from_arrow_array(
                Some(&field),
                slots,
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap()
            .require_arrow_array()
            .unwrap();
            assert_eq!(cells(&trimmed), vec![Some(*sample)]);

            // And back out to text, which is what the column already holds.
            let row = root([Field::new("sid", dtype.clone(), false)]);
            let batch =
                RecordBatch::try_new(row.into_arrow_schema().unwrap(), vec![stored]).unwrap();
            let as_text = root([DataType::utf8().required_field("sid")]);
            let read = Serie::from_arrow_batch(
                Some(&as_text),
                &batch,
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap()
            .into_arrow_batch()
            .unwrap();
            assert_eq!(cells(read.column(0)), vec![Some(*sample)], "{name}");
        }
    }

    #[test]
    fn an_expression_cast_answers_the_identifier_and_try_cast_answers_null() {
        let schema = root([DataType::utf8().required_field("sid")]);
        let batch = |values: Vec<&str>| {
            RecordBatch::try_new(
                schema.clone().into_arrow_schema().unwrap(),
                vec![Arc::new(StringArray::from(values))],
            )
            .unwrap()
        };
        for (name, _, _, sample, lower, typo, _) in &IDENTIFIERS {
            // The strict cast at the column tier holds to the canonical
            // spelling, exactly as the ISIN cast does: a typo refuses the
            // whole column, and so does a value the width does not fit.
            let strict = format!("cast(sid as {name}) = '{sample}'")
                .parse::<Term>()
                .unwrap()
                .bind(&schema)
                .unwrap();
            assert_eq!(strict.filter(&batch(vec![sample])).unwrap().num_rows(), 1);
            let message = strict
                .filter(&batch(vec![sample, typo]))
                .unwrap_err()
                .to_string();
            assert!(message.contains("canonical spelling"), "{name}: {message}");
            assert!(
                strict.filter(&batch(vec![sample, "HIGH_TOUCH"])).is_err(),
                "{name}"
            );
            // The row tier reads a value as the scalar does, folding the case.
            assert!(
                strict
                    .matches(&Scalar::from_sequence([Scalar::from(*lower)]))
                    .unwrap()
            );

            // The safe cast is what a derivation asks: an identifier the check
            // digit closes answers, and anything else is null rather than a
            // refusal, at both tiers.
            let safe = format!("try_cast(sid as {name}) is not null")
                .parse::<Term>()
                .unwrap()
                .bind(&schema)
                .unwrap();
            let kept = safe
                .filter(&batch(vec![sample, typo, "HIGH_TOUCH", lower]))
                .unwrap();
            assert_eq!(cells(kept.column(0)), vec![Some(*sample)], "{name}");
            assert!(
                safe.matches(&Scalar::from_sequence([Scalar::from(*lower)]))
                    .unwrap(),
                "{name}"
            );
            assert!(
                !safe
                    .matches(&Scalar::from_sequence([Scalar::from(*typo)]))
                    .unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_identifiers_sit_in_the_code_family() {
        // The discriminant is a wire contract laid out by family: every code
        // is in the code family's range, beside the codes stated before it.
        assert_eq!(DataTypeKind::Code.id(), 0x70);
        assert_eq!(DataTypeId::CusipCode.as_u8(), 0x79);
        assert_eq!(DataTypeId::SedolCode.as_u8(), 0x7a);
        assert_eq!(DataTypeId::BloombergCode.as_u8(), 0x7b);
        assert_eq!(DataTypeId::FIGICode.as_u8(), 0x7c);
        for id in [
            DataTypeId::IsinCode,
            DataTypeId::CusipCode,
            DataTypeId::SedolCode,
            DataTypeId::BloombergCode,
            DataTypeId::FIGICode,
        ] {
            assert_eq!(
                DataTypeKind::of_u8(id.as_u8()),
                Some(DataTypeKind::Code),
                "{id}"
            );
        }
        // The datatype order is total and appends too, so no earlier pair
        // moved: every code stated before them sorts before them, and the
        // last datatype before them sorts before them as well.
        assert!(DataType::IsinCode < DataType::CusipCode);
        assert!(DataType::CusipCode < DataType::SedolCode);
        assert!(DataType::MediaType < DataType::CusipCode);
        assert!(DataType::TimeInForce < DataType::CusipCode);
        let mut shuffled = [
            DataType::SedolCode,
            DataType::IsinCode,
            DataType::CusipCode,
            DataType::FIGICode,
            DataType::Country,
            DataType::MediaType,
        ];
        shuffled.sort();
        assert_eq!(
            shuffled,
            [
                DataType::Country,
                DataType::IsinCode,
                DataType::MediaType,
                DataType::CusipCode,
                DataType::SedolCode,
                DataType::FIGICode,
            ]
        );

        // A value carries its identity first: a CUSIP and a SEDOL never
        // compare equal, and neither is the string of its characters.
        let cusip = DataType::CusipCode
            .scalar(Scalar::from("037833100"))
            .unwrap();
        let sedol = DataType::SedolCode.scalar(Scalar::from("B0YBKJ7")).unwrap();
        assert_ne!(cusip, sedol);
        assert_ne!(cusip.cmp(&sedol), std::cmp::Ordering::Equal);
        assert_ne!(cusip, Scalar::from("037833100"));
        assert_eq!(cusip.id(), DataTypeId::CusipCode);
        assert_eq!(sedol.id(), DataTypeId::SedolCode);
        // Values of one identifier order by their text.
        assert!(
            DataType::CusipCode
                .scalar(Scalar::from("037833100"))
                .unwrap()
                < DataType::CusipCode
                    .scalar(Scalar::from("38259P508"))
                    .unwrap()
        );
    }

    #[test]
    fn the_typed_fields_name_their_identifier() {
        let cusip: CusipCodeField = CusipCodeField::unit("cusip", true);
        assert_eq!(cusip.to_field().dtype(), &DataType::CusipCode);
        let sedol: SedolCodeField = SedolCodeField::unit("sedol", false);
        assert_eq!(sedol.to_field().dtype(), &DataType::SedolCode);
        let figi: FIGICodeField = FIGICodeField::unit("figi", true);
        assert_eq!(figi.to_field().dtype(), &DataType::FIGICode);
        assert!(!&sedol.to_field().is_nullable());
        // A shared field is kept for each, as for every parameter-free leaf.
        for dtype in [DataType::CusipCode, DataType::SedolCode, DataType::FIGICode] {
            let shared = dtype.shared_field().unwrap();
            assert_eq!(shared.dtype(), &dtype);
            assert!(std::ptr::eq(shared, dtype.shared_field().unwrap()));
        }
    }
}
