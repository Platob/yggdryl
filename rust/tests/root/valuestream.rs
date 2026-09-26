//! `rust/src/valuestream.rs`: the value wire format, byte for byte.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::valuestream::{UNCOMPRESSED, ZSTD};
    use yggdryl::{COMPRESS_FROM, DataTypeId, Scalar, VALUE_STREAM_VERSION};

    #[test]
    fn a_number_is_its_identifier_and_its_bytes() {
        let bytes = Scalar::from(7_i32).into_value_bytes();
        assert_eq!(
            bytes,
            [VALUE_STREAM_VERSION, DataTypeId::Int32.as_u8(), 7, 0, 0, 0]
        );
        assert_eq!(
            Scalar::decode_value_bytes(&bytes).unwrap(),
            Scalar::from(7_i32)
        );
    }

    #[test]
    fn a_text_states_its_compression_and_its_size() {
        let bytes = Scalar::from("abc").into_value_bytes();
        assert_eq!(
            bytes,
            [
                VALUE_STREAM_VERSION,
                DataTypeId::Utf8String.as_u8(),
                UNCOMPRESSED,
                3,
                b'a',
                b'b',
                b'c'
            ]
        );
        let long = "x".repeat(COMPRESS_FROM + 1);
        let bytes = Scalar::from(long.as_str()).into_value_bytes();
        assert_eq!(bytes[2], ZSTD);
        assert!(bytes.len() < 64, "{}", bytes.len());
        assert_eq!(
            Scalar::decode_value_bytes(&bytes).unwrap(),
            Scalar::from(long.as_str())
        );
    }

    #[test]
    fn a_tree_streams_one_leaf_per_chunk_and_reads_back_whole() {
        let value = Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            (
                "tags",
                Scalar::from_sequence([Scalar::from("a"), Scalar::Null]),
            ),
        ])
        .unwrap();
        let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
        assert_eq!(chunks.len(), 7, "{chunks:?}");
        assert_eq!(chunks.concat(), value.into_value_bytes());
        assert_eq!(Scalar::decode_value_stream_bytes(&chunks).unwrap(), value);
    }

    #[test]
    fn the_refusals_name_the_byte() {
        let error = Scalar::decode_value_bytes(&[1, 0]).unwrap_err().to_string();
        assert!(error.contains("version 1"), "{error}");
        let error = Scalar::decode_value_bytes(&[0, 0x10])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("placeholder") && error.contains("integer"),
            "{error}"
        );
        let error = Scalar::decode_value_bytes(&[0, DataTypeId::Int32.as_u8(), 1])
            .unwrap_err()
            .to_string();
        assert!(error.contains("4 bytes announced"), "{error}");
        let error = Scalar::decode_value_bytes(&[0, 0, 0])
            .unwrap_err()
            .to_string();
        assert!(error.contains("1 bytes left"), "{error}");
    }
}

mod stream {
    use std::sync::Arc;

    use yggdryl::{
        COMPRESS_FROM, DataType, DataTypeId, DataTypeKind, DataTypeValue, Scalar, StringType,
        TimeUnit, Timezone, VALUE_STREAM_VERSION,
    };

    /// One WKB point, the payload a geospatial value is.
    fn point_wkb() -> Vec<u8> {
        let mut wkb = vec![1_u8, 1, 0, 0, 0];
        wkb.extend_from_slice(&1.0_f64.to_le_bytes());
        wkb.extend_from_slice(&2.0_f64.to_le_bytes());
        wkb
    }

    /// One value of every leaf the encoding spells, and of every nesting.
    fn corpus() -> Vec<Scalar> {
        let text = |dtype: &str, text: &str| {
            DataType::from_str(dtype)
                .unwrap()
                .required_field("value")
                .scalar(Scalar::from(text))
                .unwrap()
        };
        vec![
            Scalar::Null,
            Scalar::from(true),
            Scalar::from(i8::MIN),
            Scalar::from(i16::MIN),
            Scalar::from(i32::MIN),
            Scalar::from(i64::MIN),
            Scalar::from(i128::MIN),
            Scalar::from(u8::MAX),
            Scalar::from(u16::MAX),
            Scalar::from(u32::MAX),
            Scalar::from(u64::MAX),
            Scalar::from(u128::MAX),
            Scalar::from(half::f16::from_f32(-0.0)),
            Scalar::from(-0.0_f32),
            Scalar::from(f64::from_bits(0x7ff8_0000_0000_0001)),
            text("decimal32(9, 2)", "-1234567.89"),
            text("decimal64(18, 4)", "1.2345"),
            text(
                "decimal128(38, 7)",
                "-1701411834604692317316873037158.8410572",
            ),
            text(
                "decimal256(76, 18)",
                "-1234567890123456789.012345678901234567",
            ),
            Scalar::date32(19_782),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(
                1_704_190_530_000_000_000,
                TimeUnit::Nanosecond,
                Timezone::UTC,
            )
            .unwrap(),
            Scalar::datetime64(
                1,
                TimeUnit::Millisecond,
                Timezone::from_str("Europe/Paris").unwrap(),
            )
            .unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(-1, TimeUnit::Nanosecond).unwrap(),
            Scalar::Interval(yggdryl::Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap()),
            Scalar::from("naïve"),
            text("fixed_ascii(4)", "USD "),
            text("large_cp1252_view", "café"),
            text("sized_utf8(8)", "bounded"),
            text("ccy", "USD"),
            text("country", "FR"),
            text("mic", "XPAR"),
            text("cfi", "ESVUFR"),
            text("isin", "US0378331005"),
            text("cusip", "037833100"),
            text("sedol", "B0YBKJ7"),
            text("bbg", "AAPL US Equity"),
            text("ric", "AAPL.OQ"),
            text("side", "BUY"),
            text("state", "20NEW"),
            text("timeinforce", "GTC"),
            text("version", "5.0.1"),
            text("url", "https://example.com/a?b=1"),
            text("urn", "urn:isbn:0451450523"),
            text("timezone", "Europe/Paris"),
            text("mimetype", "application/json"),
            text("mediatype", "text/plain; charset=utf-8"),
            Scalar::from(vec![0_u8, 255]),
            DataType::from_str("fixed_binary(2)")
                .unwrap()
                .required_field("value")
                .scalar(Scalar::from(vec![b'a', b'b']))
                .unwrap(),
            DataType::from_str("binary_view")
                .unwrap()
                .required_field("value")
                .scalar(Scalar::from(vec![7_u8; 40]))
                .unwrap(),
            Scalar::Geometry(yggdryl::Geometry::new(point_wkb()).unwrap()),
            Scalar::Geography(yggdryl::Geography::new(point_wkb()).unwrap()),
            text("uuid", "6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
            Scalar::from_sequence([Scalar::from(1_i32), Scalar::Null, Scalar::from("a")]),
            Scalar::from_mapping([(Scalar::from("k"), Scalar::from(1_i64))]).unwrap(),
            Scalar::from_struct([
                ("id", Scalar::from(1_i64)),
                (
                    "tags",
                    Scalar::from_sequence([Scalar::from_struct([("n", Scalar::Null)]).unwrap()]),
                ),
            ])
            .unwrap(),
        ]
    }

    #[test]
    fn every_leaf_and_every_nesting_reads_back_as_itself() {
        for value in corpus() {
            let bytes = value.into_value_bytes();
            assert_eq!(bytes[0], VALUE_STREAM_VERSION, "{value:?}");
            assert_eq!(bytes[1], value.id().as_u8(), "{value:?}");
            let read = Scalar::decode_value_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{value:?} encoded as {bytes:?} refused: {error}"));
            assert_eq!(read, value);
            assert_eq!(
                read.id(),
                value.id(),
                "the leaf travels, not just the value"
            );
            assert_eq!(read.dtype().ok(), value.dtype().ok(), "{value:?}");
            // The stream is the same bytes, cut one leaf per chunk.
            let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
            assert_eq!(chunks.concat(), bytes);
            assert_eq!(Scalar::decode_value_stream_bytes(&chunks).unwrap(), value);
        }
    }

    #[test]
    fn a_leaf_keeps_its_parameters() {
        let parsed = |dtype: &str, text: &str| {
            DataType::from_str(dtype)
                .unwrap()
                .required_field("value")
                .scalar(Scalar::from(text))
                .unwrap()
        };
        let fixed = parsed("fixed_ascii(4)", "USD");
        let read = Scalar::decode_value_bytes(&fixed.into_value_bytes()).unwrap();
        assert_eq!(read.id(), DataTypeId::FixedAsciiString);
        assert_eq!(
            read.dtype().unwrap(),
            DataType::from_str("fixed_ascii(4)").unwrap()
        );

        let stamp = Scalar::datetime64(
            1,
            TimeUnit::Millisecond,
            Timezone::from_str("Asia/Tokyo").unwrap(),
        )
        .unwrap();
        let read = Scalar::decode_value_bytes(&stamp.into_value_bytes()).unwrap();
        assert_eq!(read, stamp);
        assert_eq!(read.dtype().unwrap(), stamp.dtype().unwrap());

        let decimal = parsed("decimal64(10, 3)", "1.5");
        let read = Scalar::decode_value_bytes(&decimal.into_value_bytes()).unwrap();
        assert_eq!(read, decimal);
        assert_eq!(read.as_decimal(), decimal.as_decimal());
    }

    #[test]
    fn a_number_is_two_bytes_and_its_width() {
        assert_eq!(
            Scalar::from(7_i32).into_value_bytes(),
            [VALUE_STREAM_VERSION, DataTypeId::Int32.as_u8(), 7, 0, 0, 0]
        );
        assert_eq!(
            Scalar::Null.into_value_bytes(),
            [VALUE_STREAM_VERSION, DataTypeId::Null.as_u8()]
        );
        assert_eq!(
            Scalar::from(true).into_value_bytes(),
            [VALUE_STREAM_VERSION, DataTypeId::Boolean.as_u8(), 1]
        );
        assert_eq!(Scalar::from(u64::MAX).into_value_bytes().len(), 10);
    }

    #[test]
    fn a_variable_payload_states_its_compression_and_compresses_past_four_kibibytes() {
        let bytes = Scalar::from("abc").into_value_bytes();
        assert_eq!(
            bytes,
            [
                VALUE_STREAM_VERSION,
                DataTypeId::Utf8String.as_u8(),
                0,
                3,
                b'a',
                b'b',
                b'c'
            ]
        );
        let exact = "x".repeat(COMPRESS_FROM);
        let bytes = Scalar::from(exact.as_str()).into_value_bytes();
        assert_eq!(bytes[2], 0, "four kibibytes stay as they are");
        assert_eq!(bytes.len(), COMPRESS_FROM + 5);
        let past = "x".repeat(COMPRESS_FROM + 1);
        let bytes = Scalar::from(past.as_str()).into_value_bytes();
        assert_eq!(bytes[2], 1, "one byte past is a zstd frame");
        assert!(bytes.len() < 64, "{}", bytes.len());
        assert_eq!(
            Scalar::decode_value_bytes(&bytes).unwrap(),
            Scalar::from(past.as_str())
        );
        // Bytes and a geometry compress the same way.
        let payload = Scalar::from(vec![7_u8; COMPRESS_FROM * 4]);
        let bytes = payload.into_value_bytes();
        assert_eq!(bytes[2], 1);
        assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), payload);
    }

    #[test]
    fn a_nested_value_is_a_count_and_its_children_without_the_version() {
        let value = Scalar::from_sequence([Scalar::from(1_i8), Scalar::from(2_i8)]);
        assert_eq!(
            value.into_value_bytes(),
            [
                VALUE_STREAM_VERSION,
                DataTypeId::Serie.as_u8(),
                2,
                DataTypeId::Int8.as_u8(),
                1,
                DataTypeId::Int8.as_u8(),
                2
            ]
        );
        let value = Scalar::from_struct([("b", Scalar::Null), ("a", Scalar::from(true))]).unwrap();
        // Sorted by name, as the struct holds them.
        assert_eq!(
            value.into_value_bytes(),
            [
                VALUE_STREAM_VERSION,
                DataTypeId::Struct.as_u8(),
                2,
                1,
                b'a',
                DataTypeId::Boolean.as_u8(),
                1,
                1,
                b'b',
                DataTypeId::Null.as_u8()
            ]
        );
        let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
        assert_eq!(
            chunks.len(),
            5,
            "the header, then a name and a value per child"
        );
    }

    #[test]
    fn a_datatype_casts_on_the_way_in_and_on_the_way_out() {
        let bytes = DataType::Int64
            .encode_value_bytes(&Scalar::from(7_i32))
            .unwrap();
        assert_eq!(bytes[1], DataTypeId::Int64.as_u8());
        assert_eq!(
            DataType::Int64.decode_value_bytes(&bytes).unwrap(),
            Scalar::from(7_i64)
        );
        // The cast is the datatype's own: text into a number, a number into text.
        let read = DataType::utf8().decode_value_bytes(&bytes).unwrap();
        assert_eq!(read, Scalar::from("7"));
        assert!(
            DataType::Int64
                .encode_value_bytes(&Scalar::from("seven"))
                .is_err()
        );
        let decimal = DataType::from_str("decimal128(10, 2)").unwrap();
        let held = decimal
            .clone()
            .required_field("value")
            .scalar(Scalar::from("1.5"))
            .unwrap();
        let chunks: Vec<Vec<u8>> = decimal
            .encode_value_stream_bytes(&Scalar::from(1.5_f64))
            .unwrap()
            .collect();
        assert_eq!(decimal.decode_value_stream_bytes(&chunks).unwrap(), held);
        // A leaf datatype answers the same doors through the datatype it widens to.
        let bytes = StringType::default()
            .encode_value_bytes(&Scalar::from(1_i32))
            .unwrap();
        assert_eq!(bytes[1], DataTypeId::Utf8String.as_u8());
        assert_eq!(
            StringType::default().decode_value_bytes(&bytes).unwrap(),
            Scalar::from("1")
        );
    }

    #[test]
    fn the_refusals_name_the_byte_and_the_reason() {
        let refused = |bytes: &[u8]| Scalar::decode_value_bytes(bytes).unwrap_err().to_string();
        assert!(refused(&[]).contains("ends before"), "{}", refused(&[]));
        assert!(
            refused(&[1, 0]).contains("version 1"),
            "{}",
            refused(&[1, 0])
        );
        let placeholder = refused(&[0, DataTypeKind::Integer.id()]);
        assert!(
            placeholder.contains("placeholder") && placeholder.contains("integer"),
            "{placeholder}"
        );
        assert!(
            refused(&[0, 0xf0]).contains("names no datatype"),
            "{}",
            refused(&[0, 0xf0])
        );
        let short = refused(&[0, DataTypeId::Int32.as_u8(), 1]);
        assert!(short.contains("4 bytes announced"), "{short}");
        let trailing = refused(&[0, DataTypeId::Null.as_u8(), 0]);
        assert!(trailing.contains("1 bytes left"), "{trailing}");
        let compression = refused(&[0, DataTypeId::Utf8String.as_u8(), 7, 0]);
        assert!(compression.contains("compression 7"), "{compression}");
        let twice = Scalar::from_struct([("a", Scalar::Null)])
            .unwrap()
            .into_value_bytes();
        let mut doubled = twice.clone();
        doubled[2] = 2;
        doubled.extend_from_slice(&twice[3..]);
        assert!(refused(&doubled).contains("twice"), "{}", refused(&doubled));
        // A placeholder is a valid tag for nothing, so a family gains a leaf
        // without a stream written before it moving.
        let position =
            Scalar::decode_value_bytes(&[0, DataTypeId::Serie.as_u8(), 1, 0x2f]).unwrap_err();
        assert!(position.to_string().contains("decimal"), "{position}");
    }

    #[test]
    fn the_identifiers_are_the_digest_tags_laid_out_by_family() {
        // Each family's range is pinned against an independent listing in
        // rust/tests/root/datatype_id.rs; here, the tag a value writes is the
        // byte that range places it at.
        assert_eq!(
            (
                DataTypeId::Utf8String.as_u8(),
                DataTypeId::Utf8String.kind()
            ),
            (0x51, DataTypeKind::Text)
        );
        assert_eq!(
            (DataTypeId::Int32.as_u8(), DataTypeId::Int32.kind()),
            (0x13, DataTypeKind::Integer)
        );
        assert_eq!(
            (DataTypeId::Geography.as_u8(), DataTypeId::Geography.kind()),
            (0xb2, DataTypeKind::Geospatial)
        );
        assert_eq!(DataTypeKind::Integer.id(), 0x10);
        assert_eq!(DataTypeKind::Text.id(), 0x50);
        assert_eq!(DataTypeKind::Nested.id(), 0x90);
        assert_eq!(DataTypeId::from_u8(DataTypeKind::Nested.id()), None);
        // What a value feeds a digest starts with the same byte the encoding
        // writes after the version.
        let value = Scalar::from("AAPL");
        let mut fed = yggdryl::xxhash::Xxh3::new();
        value.write_bytes(&mut fed);
        assert_eq!(value.into_value_bytes()[1], DataTypeId::Utf8String.as_u8());
        assert_eq!(Arc::new(value).id(), DataTypeId::Utf8String);
    }
}
