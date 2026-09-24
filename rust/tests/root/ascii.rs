//! `rust/src/ascii.rs`: the US-ASCII codec and the repertoire it refuses
//! everything outside of.

use super::typed;

mod codecs {

    use yggdryl::charset::Charset;

    use yggdryl::Error;

    #[test]
    fn ascii_refuses_every_byte_above_its_range() {
        let error = Charset::Ascii.decode(b"caf\xe9").unwrap_err();
        assert!(matches!(error, Error::Codec { position: 3, .. }));
        assert!(Charset::Ascii.encode("café").is_err());
        assert_eq!(Charset::Ascii.decode(b"cafe").unwrap(), "cafe");
    }
}

mod leaves {
    use std::sync::Arc;

    use arrow_array::{
        Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray,
    };
    use arrow_schema::DataType as ArrowDataType;
    use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
    use yggdryl::arrow::batch_reader;
    use yggdryl::expression::Literal;
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::{
        ArrowCastOptions, Charset, DataType, DataTypeId, Field, Scalar, Serie, StringEnum,
        StructType, Term, Url,
    };
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{Str, StringType};

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    /// Four-byte storage: the padded codes the writer would have stored.
    fn currencies(codes: &[&[u8; 4]]) -> ArrayRef {
        Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                codes.iter().map(|code| Some(code.as_slice())),
                4,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_filter_over_an_ascii_column_binds_and_evaluates() {
        let schema = root([
            DataType::fixed_ascii(4).unwrap().required_field("ccy"),
            DataType::Int64.required_field("qty"),
        ]);
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![
                currencies(&[b"USD\0", b"EUR\0", b"USD\0"]),
                Arc::new(Int64Array::from(vec![1, 2, 3])),
            ],
        )
        .unwrap();

        // The column meets the literal at utf8, and the cast trims the padding.
        let bound = "ccy = 'USD'"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let kept = bound.filter(&batch).unwrap();
        assert_eq!(kept.num_rows(), 2);
        let quantities = kept
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(quantities.values(), &[1, 3]);

        // The row tier reads the same trimmed text.
        let row = Scalar::from_sequence([Scalar::from("USD"), Scalar::from(1_i64)]);
        assert!(bound.matches(&row).unwrap());
        let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::from(2_i64)]);
        assert!(!bound.matches(&row).unwrap());
    }

    #[test]
    fn two_ascii_columns_compare_at_both_tiers() {
        let schema = root([
            DataType::fixed_ascii(4).unwrap().required_field("a"),
            DataType::fixed_ascii(4).unwrap().required_field("b"),
        ]);
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![
                currencies(&[b"USD\0", b"EUR\0"]),
                currencies(&[b"USD\0", b"USD\0"]),
            ],
        )
        .unwrap();

        // Two ASCII operands meet at their own width, so the row tier compares
        // the trimmed text the same way the column tier compares storage.
        let equal = "a = b".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(equal.filter(&batch).unwrap().num_rows(), 1);
        assert!(
            equal
                .matches(&Scalar::from_sequence([
                    Scalar::from("USD"),
                    Scalar::from("USD")
                ]))
                .unwrap()
        );
        assert!(
            !equal
                .matches(&Scalar::from_sequence([
                    Scalar::from("USD"),
                    Scalar::from("EUR")
                ]))
                .unwrap()
        );
        let before = "a < b".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(before.filter(&batch).unwrap().num_rows(), 1);
        assert!(
            before
                .matches(&Scalar::from_sequence([
                    Scalar::from("EUR"),
                    Scalar::from("USD")
                ]))
                .unwrap()
        );
    }

    #[test]
    fn string_functions_read_an_ascii_column_as_text() {
        let schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![currencies(&[b"USD\0", b"EUR\0"])],
        )
        .unwrap();
        let usd = Scalar::from_sequence([Scalar::from("USD")]);

        for (text, kept) in [
            ("upper(ccy) = 'USD'", 1),
            ("lower(ccy) = 'usd'", 1),
            ("length(ccy) = 3", 2),
            ("starts_with(ccy, 'U')", 1),
            ("concat(ccy, 'X') = 'USDX'", 1),
        ] {
            let bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
            assert_eq!(bound.filter(&batch).unwrap().num_rows(), kept, "{text}");
            assert!(bound.matches(&usd).unwrap(), "{text}");
        }
    }

    #[test]
    fn a_cast_to_a_bounded_ascii_obeys_the_bound_on_rows() {
        let schema = root([DataType::utf8().required_field("ccy")]);
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["USD", "EUR"]))],
        )
        .unwrap();

        let bound = "cast(ccy as ascii(4)) = 'USD'"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(bound.filter(&batch).unwrap().num_rows(), 1);
        assert!(
            bound
                .matches(&Scalar::from_sequence([Scalar::from("USD")]))
                .unwrap()
        );

        // The row tier refuses what the column tier refuses, naming the bound.
        let message = bound
            .matches(&Scalar::from_sequence([Scalar::from("EURO!")]))
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 4 bytes"), "{message}");
        let refused = "cast('EURO!' as ascii(4)) = ccy"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let message = refused.filter(&batch).unwrap_err().to_string();
        assert!(message.contains("at most 4 bytes"), "{message}");
        let message = refused
            .matches(&Scalar::from_sequence([Scalar::from("USD")]))
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 4 bytes"), "{message}");
    }

    #[test]
    fn a_cast_into_a_securities_number_holds_the_column_to_the_canonical_spelling() {
        let schema = root([DataType::utf8().required_field("sid")]);
        let batch = |values: Vec<&str>| {
            RecordBatch::try_new(
                schema.clone().into_arrow_schema().unwrap(),
                vec![Arc::new(StringArray::from(values))],
            )
            .unwrap()
        };
        let bound = "cast(sid as isin) = 'US0378331005'"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        // Two numbers closed by their check digits pass, and one matches.
        assert_eq!(
            bound
                .filter(&batch(vec!["US0378331005", "CH0012221716"]))
                .unwrap()
                .num_rows(),
            1
        );
        // The column tier refuses a number its check digit does not close, and
        // one spelled in lower case: a column's bytes are what every reader
        // digests, so a cast lets in the canonical spelling and nothing else.
        for column in [vec!["US0378331005", "US0378331006"], vec!["us0378331005"]] {
            let message = bound.filter(&batch(column)).unwrap_err().to_string();
            assert!(message.contains("canonical spelling"), "{message}");
        }
        // The row tier reads a value as the scalar does, folding the case.
        assert!(
            bound
                .matches(&Scalar::from_sequence([Scalar::from("us0378331005")]))
                .unwrap()
        );
        let message = bound
            .matches(&Scalar::from_sequence([Scalar::from("US0378331006")]))
            .unwrap_err()
            .to_string();
        assert!(message.contains("check digit"), "{message}");
    }

    #[test]
    fn an_ascii_literal_has_a_text_form() {
        let parsed = "ccy = ascii(4) 'USD'".parse::<Term>().unwrap();
        let Term::Compare(_, _, literal) = &parsed else {
            panic!("a comparison, got {parsed}");
        };
        // `ascii(4)` is the bounded string, not the padded one.
        assert_eq!(
            **literal,
            Term::Literal(Literal::new(DataType::sized_ascii(4).unwrap(), "USD").unwrap())
        );
        // The literal prints in its own datatype - the sized leaf, canonically
        // spelled - and re-parses; a registered code spells a literal of its own,
        // which is not the literal of the width that happens to hold the same
        // bytes.
        assert_eq!(parsed.to_string(), "ccy = sized_ascii(4) 'USD'");
        assert_eq!(parsed.to_string().parse::<Term>().unwrap(), parsed);
        let currency = "ccy = ccy 'USD'".parse::<Term>().unwrap();
        assert_eq!(currency.to_string(), "ccy = ccy 'USD'");
        assert_eq!(currency.to_string().parse::<Term>().unwrap(), currency);
        assert_ne!(currency, "ccy = ascii(3) 'USD'".parse::<Term>().unwrap());
        let refused = "ccy = country 'USD'"
            .parse::<Term>()
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 2 bytes"), "{refused}");

        let message = "ccy = ascii(4) 'EURO!'"
            .parse::<Term>()
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 4 bytes"), "{message}");
    }

    #[test]
    fn an_ascii_column_round_trips_through_avro_as_text() {
        let schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
        let batch = RecordBatch::try_new(
            schema.into_arrow_schema().unwrap(),
            vec![currencies(&[b"USD\0", b"EU\0\0"])],
        )
        .unwrap();
        let mut handle =
            Buffer::new().with_media_type(Url::from_str("file:///ccy.avro").unwrap().media_type());
        let options = RecordOptions::for_media_type(handle.media_type()).unwrap();
        handle
            .overwrite_arrow_reader(batch_reader(batch.schema(), [batch]), &options)
            .unwrap();

        // Avro has no fixed-width text, so the column is a string and every
        // reader sees the trimmed code rather than the padded storage.
        let stored = handle.read_arrow_field(&options).unwrap();
        assert_eq!(stored.fields()[0].dtype(), &DataType::utf8());
        let read: Vec<RecordBatch> = handle
            .read_arrow_reader(&options)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let ccy = read[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(ccy.value(0), "USD");
        assert_eq!(ccy.value(1), "EU");
    }

    #[cfg(feature = "iceberg")]
    #[test]
    fn an_ascii_column_is_an_iceberg_string() {
        let mut schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
        yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let json = yggdryl::iceberg::schema_into_json(&schema).unwrap();
        let fields = json
            .get_key_str("fields")
            .and_then(Scalar::as_sequence)
            .unwrap();
        assert_eq!(
            fields[0].get_key_str("type").and_then(Scalar::as_str),
            Some("string")
        );
    }

    #[test]
    fn ascii_is_one_charset_of_the_string_family() {
        // The three shapes are three leaves of one charset, and each reads back
        // from its own spelling.
        let plain = DataType::ascii();
        assert_eq!(plain.to_string(), "ascii");
        assert_eq!(DataType::from_str("ascii").unwrap(), plain);
        assert_eq!(DataType::from_str("string(us-ascii)").unwrap(), plain);
        assert_eq!(plain.string_parameters(), Some(StringType::AsciiString));
        assert_eq!(plain.charset(), Some(Charset::Ascii));
        assert_eq!(plain.id(), DataTypeId::AsciiString);
        assert!(plain.is_string());
        assert!(!plain.is_code());
        assert_eq!(plain.fixed_byte_width(), None);

        // One number, one reading per leaf: `ascii(4)` is a maximum, which is
        // the sized leaf, and `fixed_ascii(4)` the width.
        let bounded = DataType::from_str("ascii(4)").unwrap();
        assert_eq!(bounded.to_string(), "sized_ascii(4)");
        assert_eq!(bounded, DataType::from_str("string(us-ascii,4)").unwrap());
        assert_eq!(bounded, DataType::sized_ascii(4).unwrap());
        let parameters = bounded.string_parameters().unwrap();
        assert_eq!(parameters, StringType::SizedAsciiString(4));
        assert_eq!(parameters.max(), Some(4));
        assert_eq!(parameters.fixed(), None);
        assert_eq!(bounded.fixed_byte_width(), None);
        assert_eq!(bounded.id(), DataTypeId::SizedAsciiString);

        let fixed = DataType::fixed_ascii(4).unwrap();
        assert_eq!(fixed.to_string(), "fixed_ascii(4)");
        assert_eq!(DataType::from_str("fixed_ascii(4)").unwrap(), fixed);
        assert_eq!(
            DataType::from_str("fixed_string(us-ascii,4)").unwrap(),
            fixed
        );
        assert_eq!(
            fixed.string_parameters(),
            Some(StringType::FixedAsciiString(4))
        );
        assert_eq!(fixed.string_parameters().unwrap().fixed(), Some(4));
        assert_eq!(fixed.fixed_byte_width(), Some(4));
        assert_eq!(fixed.id(), DataTypeId::FixedAsciiString);
        assert_ne!(bounded, fixed);

        // A charset-named spelling takes only a bound; a width is what makes a
        // string fixed; a bound of no bytes is a column of one value.
        assert!(DataType::from_str("ascii(utf-8)").is_err());
        assert!(DataType::from_str("fixed_ascii").is_err());
        assert!(DataType::fixed_ascii(0).is_err());
        assert!(DataType::from_str("ascii(0)").is_err());
    }

    #[test]
    fn an_ascii_value_carries_the_leaf_of_its_column() {
        // What comes out of an `ascii` column is a string value of that leaf,
        // equal to the plain spelling of the same characters.
        let value = DataType::ascii().scalar("USD").unwrap();
        let Scalar::AsciiString(held) = &value else {
            panic!("an ascii value is an ascii string, got {value:?}");
        };
        assert_eq!(held.as_str(), "USD");
        assert_eq!(value.string_parameters(), Some(StringType::AsciiString));
        assert_eq!(
            value.string_parameters().map(StringType::charset),
            Some(Charset::Ascii)
        );
        assert_eq!(value, Scalar::from("USD"));
        assert_eq!(value.as_str(), Some("USD"));
        assert_eq!(value.id(), DataTypeId::AsciiString);
        assert_eq!(value.dtype().unwrap(), DataType::ascii());

        // A value read out of `ascii(4)` carries the maximum as its leaf, and
        // one that outgrows the column is refused naming the bound.
        let bounded = DataType::from_str("ascii(4)").unwrap();
        let sized = bounded.scalar("USD").unwrap();
        assert_eq!(sized.id(), DataTypeId::SizedAsciiString);
        assert_eq!(sized.dtype().unwrap(), bounded);
        assert_eq!(
            sized.string_parameters(),
            Some(StringType::SizedAsciiString(4))
        );
        assert_eq!(sized, value, "a value is one value in any column");
        let refused = bounded.scalar("EURO!").unwrap_err().to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");

        // A fixed width is the value's shape: it is carried, its padding is
        // trimmed on the way in, and it comes back padded on the way out.
        let fixed = DataType::fixed_ascii(4).unwrap();
        let padded = fixed.scalar("USD\0").unwrap();
        assert_eq!(padded.as_str(), Some("USD"));
        assert_eq!(padded.id(), DataTypeId::FixedAsciiString);
        assert_eq!(padded.dtype().unwrap(), fixed);
        let Scalar::FixedAsciiString(held, width) = &padded else {
            panic!("a fixed ascii value is a fixed ascii string, got {padded:?}");
        };
        assert_eq!(*width, 4);
        let leaf = padded.string_parameters().unwrap();
        assert_eq!(leaf.fixed(), Some(4));
        assert_eq!(leaf.encode(held.as_str()).unwrap().as_ref(), b"USD\0");
        assert_eq!(leaf.encoded_len(held.as_str()), 4);
        assert!(fixed.scalar("EURO!").is_err());
    }

    #[test]
    fn ascii_is_a_repertoire_and_refuses_what_it_never_holds() {
        // A byte above 0x7F and a NUL are refused by name wherever the charset
        // is US-ASCII, from text and from bytes alike.
        for dtype in [
            DataType::ascii(),
            DataType::from_str("ascii(8)").unwrap(),
            DataType::fixed_ascii(8).unwrap(),
        ] {
            let refused = dtype.scalar("caf\u{e9}").unwrap_err().to_string();
            assert!(refused.contains("0xC3"), "{dtype}: {refused}");
            assert!(
                dtype
                    .scalar(Scalar::from(vec![0x63_u8, 0x61, 0x66, 0xE9]))
                    .is_err(),
                "{dtype}"
            );
            assert!(
                dtype.scalar(Scalar::from(vec![0x80_u8])).is_err(),
                "{dtype}"
            );
            assert_eq!(
                dtype
                    .scalar(Scalar::from(b"USD".to_vec()))
                    .unwrap()
                    .as_str(),
                Some("USD"),
                "{dtype}"
            );
        }
        // Trailing NUL is padding on the fixed layout alone: a variable string
        // holds no NUL at all.
        let refused = DataType::ascii().scalar("USD\0").unwrap_err().to_string();
        assert!(refused.contains("NUL"), "{refused}");
        assert!(
            DataType::from_str("ascii(8)")
                .unwrap()
                .scalar(Scalar::from(b"USD\0".to_vec()))
                .is_err()
        );
        assert_eq!(
            DataType::fixed_ascii(8)
                .unwrap()
                .scalar(Scalar::from(b"USD\0\0\0\0\0".to_vec()))
                .unwrap()
                .as_str(),
            Some("USD")
        );
        // The same rule under the value's own door.
        let ascii = StringType::AsciiString;
        assert!(ascii.scalar(Str::new("caf\u{e9}")).is_err());
        assert!(ascii.scalar_from_bytes(&[0x80]).is_err());
    }

    #[test]
    fn ascii_rides_arrow_text_storage_under_the_string_document() {
        // ASCII bytes are UTF-8, so Arrow is told the truth about the bytes and
        // the charset rides the `yggdryl.string` document beside them.
        let cases: [(DataType, ArrowDataType, &str); 3] = [
            (
                DataType::ascii(),
                ArrowDataType::Utf8,
                r#"{"layout":"ascii","charset":"us-ascii"}"#,
            ),
            (
                DataType::from_str("ascii(4)").unwrap(),
                ArrowDataType::Utf8,
                r#"{"layout":"sized_ascii","charset":"us-ascii","max":4}"#,
            ),
            (
                DataType::fixed_ascii(4).unwrap(),
                ArrowDataType::FixedSizeBinary(4),
                r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}"#,
            ),
        ];
        for (dtype, storage, document) in cases {
            assert_eq!(
                dtype.clone().into_arrow_datatype().unwrap(),
                storage,
                "{dtype}"
            );
            let field = dtype.clone().nullable_field("ccy");
            let arrow = field.clone().into_arrow_field().unwrap();
            assert_eq!(arrow.data_type(), &storage, "{dtype}");
            assert_eq!(
                arrow
                    .metadata()
                    .get(EXTENSION_TYPE_NAME_KEY)
                    .map(String::as_str),
                Some("yggdryl.string"),
                "{dtype}"
            );
            assert_eq!(
                arrow
                    .metadata()
                    .get(EXTENSION_TYPE_METADATA_KEY)
                    .map(String::as_str),
                Some(document),
                "{dtype}"
            );
            assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field, "{dtype}");

            // A value crosses as itself in both directions.
            let value = dtype.scalar("USD").unwrap();
            let array = Serie::from_scalars(field.clone(), [value.clone()])
                .unwrap()
                .require_arrow_array()
                .unwrap();
            assert_eq!(array.data_type(), &storage, "{dtype}");
            assert_eq!(
                Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default())
                    .unwrap()
                    .scalar(0)
                    .unwrap(),
                value,
                "{dtype}"
            );
        }

        // A bare Utf8 column is plain UTF-8, and the retired `yggdryl.ascii`
        // name is nobody's: a field wearing it imports as its storage.
        let plain = arrow_schema::Field::new("ccy", ArrowDataType::Utf8, true);
        assert_eq!(
            Field::from_arrow_field(&plain).unwrap().dtype(),
            &DataType::utf8()
        );
        let retired = plain.with_metadata(
            [
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "yggdryl.ascii".to_owned(),
                ),
                (EXTENSION_TYPE_METADATA_KEY.to_owned(), String::new()),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(
            Field::from_arrow_field(&retired).unwrap().dtype(),
            &DataType::utf8()
        );
    }

    #[test]
    fn a_string_enum_needs_a_fixed_ascii_width_its_members_pack_into() {
        // The members pack into integers through `ascii_packed`, so the
        // vocabulary is accepted on a fixed US-ASCII string of at most sixteen
        // bytes or a code, and refused by name everywhere else.
        let sides = StringEnum::from_logical_name("side").unwrap();
        for accepted in [
            DataType::fixed_ascii(8).unwrap(),
            DataType::fixed_ascii(16).unwrap(),
            DataType::Side,
        ] {
            let field = Field::new("side", accepted.clone(), false)
                .try_with_string_enum(&sides)
                .unwrap_or_else(|error| panic!("{accepted}: {error}"));
            assert_eq!(field.string_enum().unwrap().as_ref(), Some(&sides));
            let recovered =
                Field::from_arrow_field(&field.clone().into_arrow_field().unwrap()).unwrap();
            assert_eq!(recovered, field, "{accepted}");
        }
        for refused in [
            DataType::ascii(),
            DataType::from_str("ascii(4)").unwrap(),
            DataType::fixed_ascii(17).unwrap(),
            DataType::fixed_utf8(4).unwrap(),
            DataType::utf8(),
        ] {
            let message = Field::new("side", refused.clone(), false)
                .try_with_string_enum(&sides)
                .unwrap_err()
                .to_string();
            assert!(
                message.contains(&refused.to_string()),
                "{refused}: {message}"
            );
        }
    }
}

mod fields {
    use std::sync::Arc;

    use arrow_array::types::Int32Type;
    use arrow_array::{
        Array, ArrayRef, BinaryArray, DictionaryArray, FixedSizeBinaryArray, Int32Array,
        RecordBatch, StringArray, StringViewArray, StructArray,
    };
    use arrow_buffer::NullBuffer;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields, Schema};
    use yggdryl::string;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, Field, FieldScalar, Scalar, Serie, StringEnum,
        StructType,
    };
    use yggdryl::{CcyField, CfiCodeField, CountryField, MicCodeField, StringField};

    use super::typed::assert_typed_marker;
    use yggdryl::FieldValue as _;

    #[test]
    fn the_string_marker_covers_us_ascii_and_the_code_markers_their_codes() {
        assert_typed_marker::<string::StringType>(DataType::ascii());
        assert_typed_marker::<string::StringType>(DataType::fixed_ascii(4).unwrap());
        assert_typed_marker::<string::StringType>(DataType::fixed_ascii(16).unwrap());
        assert_typed_marker::<string::StringType>(DataType::from_str("ascii(4)").unwrap());
        assert_typed_marker::<string::StringType>(DataType::utf8());
        assert_typed_marker::<string::StringType>(DataType::large_utf8());
        assert_typed_marker::<string::StringType>(DataType::utf8_view());
        assert_typed_marker::<string::StringType>(
            DataType::from_str("string(windows-1252)").unwrap(),
        );
        assert_typed_marker::<yggdryl::CountryType>(DataType::Country);
        assert_typed_marker::<yggdryl::CcyType>(DataType::Ccy);
        assert_typed_marker::<yggdryl::MicCodeType>(DataType::MicCode);
        assert_typed_marker::<yggdryl::CfiCodeType>(DataType::CfiCode);

        // Every string is one parameterized datatype, so the field takes it
        // through `try_new`; a code is not a string and is refused by name.
        let note = StringField::try_new("note", DataType::ascii(), true).unwrap();
        assert_eq!(note.dtype(), &DataType::ascii());
        let ccy = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), false).unwrap();
        assert_eq!(ccy.dtype(), &DataType::fixed_ascii(4).unwrap());
        assert!(StringField::try_new("ccy", DataType::Ccy, false).is_err());

        // The code/width boundary is the one the markers exist for: a currency
        // and a `fixed_ascii(3)` are the same three bytes and are not each other.
        assert_eq!(CcyField::unit("ccy", false).dtype(), &DataType::Ccy);
        assert_eq!(CountryField::unit("iso", true).dtype(), &DataType::Country);
        assert_eq!(
            MicCodeField::unit("venue", true).dtype(),
            &DataType::MicCode
        );
        assert!(CcyField::try_new("ccy", DataType::fixed_ascii(3).unwrap(), false).is_err());
        // Six bytes against eight: the confusion a width/code mix-up produces.
        assert!(CfiCodeField::try_new("code", DataType::fixed_ascii(8).unwrap(), false).is_err());

        // The typed value is checked under the one US-ASCII rule for its width.
        let width = StringField::try_new("code", DataType::fixed_ascii(8).unwrap(), false).unwrap();
        let width_field = width.to_field();
        let code = FieldScalar::new(&width_field, "ABC").unwrap();
        assert_eq!(code.as_str(), Some("ABC"));
        assert_eq!(code.value().id(), DataTypeId::FixedAsciiString);
        assert!(FieldScalar::new(&width.to_field(), "ABCDEFGHI").is_err());

        // A typed code value is checked at the width its own standard fixes.
        let ccy = CcyField::unit("ccy", false);
        assert_eq!(
            FieldScalar::new(&ccy.to_field(), "USD").unwrap().as_str(),
            Some("USD")
        );
        assert!(FieldScalar::new(&ccy.to_field(), "EURO").is_err());
        let cfi = CfiCodeField::unit("classification", false);
        assert!(FieldScalar::new(&cfi.to_field(), "ESVUFR").is_ok());
    }

    #[test]
    fn the_value_door_judges_the_repertoire_and_the_bound() {
        // The variable layout trims nothing: NUL and a byte above 0x7F are
        // refused, and everything else is the same value `Scalar::from` builds.
        let note = StringField::try_new("note", DataType::ascii(), true).unwrap();
        let note_field = note.to_field();
        let held = FieldScalar::new(&note_field, "USD").unwrap();
        assert_eq!(held.value(), &Scalar::from("USD"));
        assert_eq!(held.value().id(), DataTypeId::AsciiString);
        for (text, fact) in [("U\0S", "NUL byte"), ("\u{20ac}", "non-ASCII byte")] {
            let refused = FieldScalar::new(&note.to_field(), text)
                .unwrap_err()
                .to_string();
            assert!(refused.contains(fact), "{refused}");
        }
        assert!(FieldScalar::new(&note.to_field(), "USD\0").is_err());

        // `sized_ascii(4)` is a maximum the value carries as its leaf.
        let bounded =
            StringField::try_new("ccy", DataType::from_str("ascii(4)").unwrap(), true).unwrap();
        let bounded_field = bounded.to_field();
        let held = FieldScalar::new(&bounded_field, "EURO").unwrap();
        assert_eq!(
            held.value().dtype().unwrap(),
            DataType::sized_ascii(4).unwrap()
        );
        assert_eq!(held.value(), &Scalar::from("EURO"));
        let refused = FieldScalar::new(&bounded.to_field(), "EUROS")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");

        // The fixed layout trims the padding storage writes and carries its width.
        let fixed = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
        let fixed_field = fixed.to_field();
        let held = FieldScalar::new(&fixed_field, Scalar::from(b"USD\0")).unwrap();
        assert_eq!(held.as_str(), Some("USD"));
        assert_eq!(
            held.value().dtype().unwrap(),
            DataType::fixed_ascii(4).unwrap()
        );
        assert!(FieldScalar::new(&fixed.to_field(), Scalar::from(b"US\xC3\xA9")).is_err());
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
            DataType::fixed_binary(4).unwrap(),
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
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    /// One column under the root's own Arrow schema, so it carries the
    /// extension document a stored US-ASCII column carries.
    fn batch_of(root: &Field, column: ArrayRef) -> RecordBatch {
        RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column]).unwrap()
    }

    fn fixed(width: i32, cells: &[Option<&[u8]>]) -> ArrayRef {
        Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(cells.iter().copied(), width)
                .unwrap(),
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
        Ok(Serie::from_arrow_batch(
            Some(&root([target])),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )?
        .into_arrow_batch()?
        .column(0)
        .clone())
    }

    #[test]
    fn text_entering_an_ascii_width_is_validated_and_padded() {
        let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
        let source: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EU"), None]));

        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
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
            let refused = Serie::from_arrow_array(
                Some(&field.clone().into_field()),
                source,
                ArrowCastOptions::new().with_safe(false),
            )
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
        let cast = Serie::from_arrow_array(
            Some(&field.to_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let cast = fixed_cells(&cast);
        assert_eq!(cast.value(0), b"USD\0");
        assert_eq!(cast.value(1), b"EUR\0");

        // The same storage carrying a non-ASCII byte is refused by row, naming
        // the charset that refused it.
        let broken = fixed(4, &[Some(b"USD\0"), Some(b"US\xC3\xA9")]);
        let refused = Serie::from_arrow_array(
            Some(&field.to_field()),
            broken,
            ArrowCastOptions::new().with_safe(false),
        )
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
            DataType::fixed_binary(4).unwrap().nullable_field("ccy"),
        )
        .unwrap();
        // A column holds its leaf as its own typed array, so the `Arc` around it
        // is new; the buffers under it are the caller's.
        assert!(same.to_data().ptr_eq(&stored.to_data()));
    }

    #[test]
    fn a_dictionary_of_text_enters_an_ascii_width() {
        let field = StringField::try_new("ccy", DataType::fixed_ascii(4).unwrap(), true).unwrap();
        let keys = Int32Array::from(vec![Some(0), Some(1), None, Some(0)]);
        let values: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR"]));
        let source: ArrayRef =
            Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());

        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
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

        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
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
            DataType::from(
                StructType::from_fields([DataType::fixed_ascii(4).unwrap().required_field("ccy")])
                    .unwrap(),
            ),
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

        let cast = Serie::from_arrow_batch(Some(&target), &batch, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
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
}
