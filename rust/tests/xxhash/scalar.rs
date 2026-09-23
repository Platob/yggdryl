//! `rust/src/xxhash/scalar.rs`: the canonical byte feed one `Scalar` hashes as.

mod xxhash {

    mod values {
        use std::hash::Hasher as _;
        use std::sync::Arc;

        use yggdryl::xxhash::{Xxh3, xxh3};
        use yggdryl::{
            Bytes, BytesType, Currency, Decimal32, Decimal64, Geography, Interval, Side, Str,
            StringType, TimeInForce,
        };
        use yggdryl::{
            Codec, DataTypeId, DigestAlgorithm, Float16, Float32, Float64, Scalar, TimeUnit,
            Timezone, i256,
        };

        const POINT_WKB: [u8; 21] = [
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];

        fn geometry() -> Scalar {
            Scalar::Geometry(yggdryl::Geometry::new(POINT_WKB).unwrap())
        }

        fn geography() -> Scalar {
            Scalar::Geography(Geography::new(POINT_WKB).unwrap())
        }

        /// A sink that keeps the feed so two values can be compared byte for byte.
        #[derive(Default)]
        struct Collected(Vec<u8>);

        impl std::hash::Hasher for Collected {
            fn finish(&self) -> u64 {
                xxh3(&self.0)
            }

            fn write(&mut self, bytes: &[u8]) {
                self.0.extend_from_slice(bytes);
            }
        }

        /// The text, restated under the parameters a column stores it in.
        fn stored(text: &str, parameters: StringType) -> Str {
            Str::new(text).try_with_parameters(parameters).unwrap()
        }

        /// `AAPL` as a byte value stored under one leaf.
        fn stored_bytes(leaf: BytesType) -> Scalar {
            Scalar::Bytes(Bytes::new(b"AAPL").try_with_parameters(leaf).unwrap())
        }

        /// Return one value's canonical feed.
        fn feed(value: &Scalar) -> Vec<u8> {
            let mut sink = Collected::default();
            value.write_bytes(&mut sink);
            sink.0
        }

        /// Every variant, plus the pairs that must agree and the pairs that must not.
        fn corpus() -> Vec<Scalar> {
            vec![
                Scalar::Null,
                Scalar::from(false),
                Scalar::from(true),
                Scalar::from(-1_i8),
                Scalar::from(0_i8),
                Scalar::from(1_i8),
                Scalar::from(-300_i16),
                Scalar::from(0x31_i32),
                Scalar::from(i64::MIN),
                Scalar::from(i128::MIN),
                Scalar::from(0x31_u8),
                Scalar::from(u16::MAX),
                Scalar::from(u32::MAX),
                Scalar::from(u64::MAX),
                Scalar::from(u128::MAX),
                Scalar::from(Float16::from_f16(half::f16::from_f32(1.5))),
                Scalar::from(Float32::from_f32(1.5)),
                Scalar::from(Float64::from_f64(1.5)),
                Scalar::from(Float64::from_f64(-0.0)),
                Scalar::from(Float64::from_f64(0.0)),
                Scalar::from(Float64::from_f64(f64::NAN)),
                Scalar::d128(100, 2),
                Scalar::Decimal32(Decimal32::new(100, 2)),
                Scalar::Decimal64(Decimal64::new(100, 2)),
                Scalar::d128(-1, 0),
                Scalar::d256(i256::from_i128(1), 0),
                Scalar::from(""),
                Scalar::from("1"),
                Scalar::from("AAPL"),
                Scalar::String(stored("AAPL", StringType::LargeUtf8String)),
                Scalar::String(stored("AAPL", StringType::Utf8StringView)),
                Scalar::String(stored("USD", StringType::AsciiString)),
                Scalar::String(stored("USD", StringType::FixedAsciiString(4))),
                Scalar::Currency(Currency::new("USD").unwrap()),
                Scalar::Side(Side::new("BUY").unwrap()),
                Scalar::TimeInForce(TimeInForce::new("1").unwrap()),
                Scalar::from(Codec::Gzip),
                Scalar::from(Codec::Zstd),
                Scalar::from(DataTypeId::Int128),
                Scalar::from(Arc::from(b"".as_slice())),
                Scalar::from(Arc::from(b"1".as_slice())),
                Scalar::from(Arc::from(b"AAPL".as_slice())),
                stored_bytes(BytesType::FixedBinary(4)),
                stored_bytes(BytesType::LargeBinary),
                stored_bytes(BytesType::BinaryView),
                geometry(),
                geography(),
                Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                Scalar::date64_in(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
                Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::time64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
                Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                Scalar::duration32_in(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration64_in(1_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
                Scalar::Interval(Interval::new(1, 0, 0, TimeUnit::YearMonth).unwrap()),
                Scalar::Interval(Interval::new(0, 1, 2_000_000, TimeUnit::DayTime).unwrap()),
                Scalar::Interval(Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap()),
                Scalar::from_sequence([]),
                Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")]),
                Scalar::from_sequence([Scalar::from("ab")]),
                Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i64))]).unwrap(),
                Scalar::from_struct([("a", Scalar::from(1_i64))]).unwrap(),
                Scalar::from_struct([("a", Scalar::from(1_i64)), ("b", Scalar::Null)]).unwrap(),
            ]
        }

        #[test]
        fn equal_values_feed_identical_bytes() {
            // The pairs that make this non-trivial: `Scalar` compares across
            // widths, so a feed keyed on the storage width would break here.
            let equal = [
                (Scalar::from(1), Scalar::from(1)),
                (Scalar::from(1), Scalar::from(1)),
                (Scalar::from(-1), Scalar::from(-1)),
                (
                    Scalar::from(Float32::from_f32(1.5)),
                    Scalar::from(Float64::from_f64(1.5)),
                ),
                (
                    Scalar::from(Float16::from_f16(half::f16::from_f32(1.5))),
                    Scalar::from(Float64::from_f64(1.5)),
                ),
                (Scalar::d128(100, 2), Scalar::d256(i256::from_i128(1), 0)),
                (
                    Scalar::Decimal32(Decimal32::new(100, 2)),
                    Scalar::d128(1, 0),
                ),
                (
                    Scalar::Decimal64(Decimal64::new(100, 2)),
                    Scalar::d256(i256::from_i128(1), 0),
                ),
                (
                    Scalar::String(stored("AAPL", StringType::LargeUtf8String)),
                    Scalar::from("AAPL"),
                ),
                (
                    Scalar::String(stored("AAPL", StringType::Utf8StringView)),
                    Scalar::from("AAPL"),
                ),
                (
                    stored_bytes(BytesType::FixedBinary(4)),
                    Scalar::from(Arc::<[u8]>::from(b"AAPL".as_slice())),
                ),
                (
                    stored_bytes(BytesType::LargeBinary),
                    Scalar::from(Arc::<[u8]>::from(b"AAPL".as_slice())),
                ),
                (
                    stored_bytes(BytesType::BinaryView),
                    Scalar::from(Arc::<[u8]>::from(b"AAPL".as_slice())),
                ),
                (
                    Scalar::String(stored("USD", StringType::FixedAsciiString(4))),
                    Scalar::String(stored("USD", StringType::AsciiString)),
                ),
                (geometry(), geography()),
                (
                    Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                    Scalar::date64_in(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
                ),
                (
                    Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                    Scalar::time64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                ),
            ];
            for (left, right) in equal {
                assert_eq!(left, right, "the corpus pair is not equal to begin with");
                assert_eq!(feed(&left), feed(&right), "{left:?} vs {right:?}");
                assert_eq!(
                    left.digest(DigestAlgorithm::Xxh3),
                    right.digest(DigestAlgorithm::Xxh3)
                );
            }

            // And over the whole corpus: equality and an identical feed agree in
            // both directions.
            let values = corpus();
            for left in &values {
                for right in &values {
                    assert_eq!(
                        left == right,
                        feed(left) == feed(right),
                        "{left:?} vs {right:?}"
                    );
                }
            }
        }

        #[test]
        fn values_that_differ_feed_different_bytes() {
            let values = corpus();
            for (index, left) in values.iter().enumerate() {
                for right in &values[index + 1..] {
                    if left == right {
                        continue;
                    }
                    assert_ne!(feed(left), feed(right), "{left:?} vs {right:?}");
                }
            }

            // The specific boundaries a tagless feed would collapse.
            assert_ne!(feed(&Scalar::from("1")), feed(&Scalar::from(0x31)));
            assert_ne!(
                feed(&Scalar::from("1")),
                feed(&Scalar::from(Arc::from(b"1".as_slice())))
            );
            assert_ne!(
                feed(&Scalar::from(Arc::from(b"AAPL".as_slice()))),
                feed(&geometry())
            );
            assert_ne!(
                feed(&Scalar::from_sequence([
                    Scalar::from("a"),
                    Scalar::from("b")
                ])),
                feed(&Scalar::from_sequence([Scalar::from("ab")]))
            );
            // A null and an empty string are not the same absence.
            assert_ne!(feed(&Scalar::Null), feed(&Scalar::from("")));
            assert_ne!(
                feed(&Scalar::Null),
                feed(&Scalar::from(Arc::from(b"".as_slice())))
            );
        }

        #[test]
        fn the_feed_starts_with_the_pinned_datatype_id_byte() {
            // The wire contract: a variant inserted into `DataTypeId` anywhere but
            // the end moves these numbers and changes every stored digest.
            let cases: [(Scalar, DataTypeId); 15] = [
                (Scalar::Null, DataTypeId::Null),
                (Scalar::from(true), DataTypeId::Boolean),
                (Scalar::from(1), DataTypeId::UInt128),
                (Scalar::from(-1), DataTypeId::Int128),
                (Scalar::from(Float32::from_f32(1.5)), DataTypeId::Float64),
                (Scalar::d128(1, 0), DataTypeId::Decimal256),
                (Scalar::from("AAPL"), DataTypeId::Utf8String),
                (
                    Scalar::from(Arc::from(b"AAPL".as_slice())),
                    DataTypeId::Binary,
                ),
                (geometry(), DataTypeId::Geometry),
                (
                    Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                    DataTypeId::Date64,
                ),
                (
                    Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                    DataTypeId::Time64,
                ),
                (
                    Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                    DataTypeId::DateTime64,
                ),
                (
                    Scalar::duration32_in(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                    DataTypeId::Duration64,
                ),
                (Scalar::from_sequence([]), DataTypeId::Serie),
                (
                    Scalar::from_struct([] as [(&str, Scalar); 0]).unwrap(),
                    DataTypeId::Struct,
                ),
            ];
            for (value, id) in cases {
                assert_eq!(feed(&value)[0], id.as_u8(), "{value:?}");
            }
            assert_eq!(
                feed(&Scalar::from_mapping([]).unwrap())[0],
                DataTypeId::Map.as_u8()
            );

            // The exact bytes of a small value, so the layout itself is pinned and
            // not only its first byte.
            assert_eq!(feed(&Scalar::Null), vec![DataTypeId::Null.as_u8()]);
            assert_eq!(
                feed(&Scalar::from(true)),
                vec![DataTypeId::Boolean.as_u8(), 1]
            );
            let mut expected = vec![DataTypeId::Utf8String.as_u8()];
            expected.extend_from_slice(&4_u64.to_le_bytes());
            expected.extend_from_slice(b"AAPL");
            assert_eq!(feed(&Scalar::from("AAPL")), expected);

            // A string feeds one tag whatever leaf stores it, and a code feeds
            // its own: the identity is part of the value.
            assert_eq!(
                feed(&Scalar::String(stored(
                    "AAPL",
                    StringType::FixedAsciiString(8)
                ))),
                expected
            );
            let mut expected = vec![DataTypeId::Currency.as_u8()];
            expected.extend_from_slice(&3_u64.to_le_bytes());
            expected.extend_from_slice(b"USD");
            assert_eq!(
                feed(&Scalar::Currency(Currency::new("USD").unwrap())),
                expected
            );
        }

        #[test]
        fn the_feed_does_not_depend_on_how_the_sink_batches_it() {
            for value in corpus() {
                let bytes = feed(&value);
                let mut state = Xxh3::new();
                state.write_scalar(&value);
                assert_eq!(state.as_u64(), xxh3(&bytes), "{value:?}");

                for split in [1_usize, 3, 7, 64] {
                    let mut chunked = Xxh3::new();
                    for chunk in bytes.chunks(split) {
                        chunked.write_bytes(chunk);
                    }
                    assert_eq!(chunked.as_u64(), state.as_u64(), "{value:?} split {split}");
                }
            }
        }

        #[test]
        fn every_algorithm_digests_a_value_through_the_same_feed() {
            for value in corpus() {
                let bytes = feed(&value);
                for algorithm in DigestAlgorithm::ALL {
                    assert_eq!(
                        value.digest(algorithm),
                        algorithm.digest(&bytes),
                        "{value:?} under {algorithm}"
                    );
                }
            }
        }

        #[test]
        fn a_field_scalar_digests_as_the_value_inside_it() {
            let field = yggdryl::Field::new("symbol", yggdryl::DataType::utf8(), false);
            let typed = yggdryl::FieldScalar::new(&field, "AAPL").unwrap();
            for algorithm in DigestAlgorithm::ALL {
                assert_eq!(
                    typed.digest(algorithm),
                    Scalar::from("AAPL").digest(algorithm)
                );
            }
            assert_eq!(typed.stable_hash(), Scalar::from("AAPL").stable_hash());
        }

        #[test]
        fn a_field_record_digests_as_the_sequence_it_canonicalizes_to() {
            let row = yggdryl::StructType::from_fields([
                yggdryl::Field::new("id", yggdryl::DataType::Int64, false),
                yggdryl::Field::new("symbol", yggdryl::DataType::utf8(), true),
                yggdryl::Field::new(
                    "legs",
                    yggdryl::DataType::serie(yggdryl::Field::new(
                        "item",
                        yggdryl::DataType::Int32,
                        true,
                    )),
                    true,
                ),
            ])
            .map(yggdryl::DataType::from)
            .unwrap()
            .required_field("row");
            let nested = Scalar::from_sequence([Scalar::from(1_i32), Scalar::Null]);
            let cells = [Scalar::from(7_i64), Scalar::from("AAPL"), nested];
            let record =
                yggdryl::FieldRecord::new(&row, Scalar::from_sequence(cells.clone())).unwrap();
            let sequence = Scalar::from_sequence(cells);
            assert_eq!(record.stable_hash(), sequence.stable_hash());
            for algorithm in DigestAlgorithm::ALL {
                assert_eq!(record.digest(algorithm), sequence.digest(algorithm));
            }
            assert_eq!(
                record.clone().into_scalar().stable_hash(),
                record.stable_hash()
            );
            // The empty row frames as the empty sequence, exactly as a row digest does.
            let empty = yggdryl::StructType::from_fields([])
                .map(yggdryl::DataType::from)
                .unwrap()
                .required_field("row");
            let record = yggdryl::FieldRecord::new(&empty, Scalar::from_sequence([])).unwrap();
            assert_eq!(
                record.stable_hash(),
                Scalar::from_sequence([]).stable_hash()
            );
        }

        #[test]
        fn nesting_past_the_shared_limit_is_bounded_rather_than_a_panic() {
            // Caller input can nest as deeply as whoever built it chose, so the
            // walk is bounded the way every other recursive descent here is.
            let mut deep = Scalar::from("leaf");
            for _ in 0..yggdryl::DataType::PARSE_RECURSION_LIMIT * 4 {
                deep = Scalar::from_sequence([deep]);
            }
            let bytes = feed(&deep);
            assert_eq!(*bytes.last().unwrap(), 0xff, "the subtree was cut");
            assert_eq!(
                deep.digest(DigestAlgorithm::Xxh3),
                DigestAlgorithm::Xxh3.digest(&bytes)
            );
            // Values differing only below the cut are indistinguishable, exactly
            // as `dtype` refuses to name them.
            let mut other = Scalar::from("other");
            for _ in 0..yggdryl::DataType::PARSE_RECURSION_LIMIT * 4 {
                other = Scalar::from_sequence([other]);
            }
            assert_eq!(feed(&other), bytes);
            assert!(deep.dtype().is_err());
        }

        #[test]
        fn value_bytes_are_the_payload_alone() {
            assert_eq!(&*Scalar::from("AAPL").as_value_bytes().unwrap(), b"AAPL");
            assert_eq!(
                &*Scalar::from(Arc::from(b"\x00\xff".as_slice()))
                    .as_value_bytes()
                    .unwrap(),
                &[0x00, 0xff]
            );
            assert_eq!(&*geometry().as_value_bytes().unwrap(), &POINT_WKB);
            assert_eq!(&*Scalar::from(true).as_value_bytes().unwrap(), &[1]);
            assert_eq!(&*Scalar::from(false).as_value_bytes().unwrap(), &[0]);
            assert_eq!(
                &*Scalar::from(1_i32).as_value_bytes().unwrap(),
                &[1, 0, 0, 0]
            );
            assert_eq!(&*Scalar::from(0x31_u8).as_value_bytes().unwrap(), b"1");
            assert_eq!(
                &*Scalar::from(Float64::from_f64(1.5))
                    .as_value_bytes()
                    .unwrap(),
                &1.5_f64.to_bits().to_le_bytes()
            );
            assert_eq!(
                Scalar::d256(i256::from_i128(1), 3)
                    .as_value_bytes()
                    .unwrap()
                    .len(),
                32
            );
            assert_eq!(
                &*Scalar::from(Codec::Gzip).as_value_bytes().unwrap(),
                b"gzip"
            );
            assert_eq!(
                &*Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE)
                    .unwrap()
                    .as_value_bytes()
                    .unwrap(),
                &[1, 0, 0, 0]
            );

            // The four variants with no payload of their own.
            assert!(Scalar::Null.as_value_bytes().is_none());
            assert!(Scalar::from_sequence([]).as_value_bytes().is_none());
            assert!(Scalar::from_mapping([]).unwrap().as_value_bytes().is_none());
            assert!(
                Scalar::from_struct([] as [(&str, Scalar); 0])
                    .unwrap()
                    .as_value_bytes()
                    .is_none()
            );

            // The widths a payload view keeps, which the canonical feed collapses.
            assert_eq!(Scalar::from(1_i8).as_value_bytes().unwrap().len(), 1);
            assert_eq!(Scalar::from(1_i64).as_value_bytes().unwrap().len(), 8);
            assert_ne!(
                Scalar::from(1_i8).as_value_bytes().unwrap(),
                Scalar::from(1_i64).as_value_bytes().unwrap()
            );
            assert_eq!(feed(&Scalar::from(1_i8)), feed(&Scalar::from(1_i64)));
        }

        #[test]
        fn a_value_byte_view_compares_and_hashes_as_its_bytes() {
            let text = Scalar::from("1");
            let number = Scalar::from(0x31_u8);
            let borrowed = text.as_value_bytes().unwrap();
            let inline = number.as_value_bytes().unwrap();
            assert_eq!(borrowed, inline);
            assert_eq!(format!("{borrowed:?}"), format!("{inline:?}"));
            assert_eq!(borrowed.as_ref(), b"1");

            let mut left = Xxh3::new();
            std::hash::Hash::hash(&borrowed, &mut left);
            let mut right = Xxh3::new();
            std::hash::Hash::hash(&inline, &mut right);
            assert_eq!(left.finish(), right.finish());
        }

        #[test]
        fn a_record_feeds_its_fields_in_sorted_name_order() {
            // The stored map is sorted, so two records built in different orders
            // are one value and feed one way.
            let left =
                Scalar::from_struct([("b", Scalar::from(2_i64)), ("a", Scalar::from(1_i64))])
                    .unwrap();
            let right =
                Scalar::from_struct([("a", Scalar::from(1_i64)), ("b", Scalar::from(2_i64))])
                    .unwrap();
            assert_eq!(feed(&left), feed(&right));

            // A mapping is insertion-ordered, so its order is part of the value.
            let left = Scalar::from_mapping([
                (Scalar::from("b"), Scalar::from(2)),
                (Scalar::from("a"), Scalar::from(1)),
            ])
            .unwrap();
            let right = Scalar::from_mapping([
                (Scalar::from("a"), Scalar::from(1)),
                (Scalar::from("b"), Scalar::from(2)),
            ])
            .unwrap();
            assert_ne!(left, right);
            assert_ne!(feed(&left), feed(&right));
        }

        #[test]
        fn a_record_name_cannot_be_confused_with_its_value() {
            // Names are length-prefixed, so a field named "ab" with value "" and a
            // field named "a" with value "b" are different feeds.
            let left = Scalar::from_struct([("ab", Scalar::from(""))]).unwrap();
            let right = Scalar::from_struct([("a", Scalar::from("b"))]).unwrap();
            assert_ne!(feed(&left), feed(&right));
        }
    }
}
