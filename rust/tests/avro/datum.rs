//! `rust/src/avro/datum.rs`: the binary encoding of one datum, its budget, and
//! the typed errors a malformed one raises.

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType};

    /// Append a zig-zag variable-length integer, as Avro's `long` is encoded.
    ///
    /// Spelled here rather than borrowed from the codec: a fixture that builds its
    /// bytes with the reader under test proves only that the two agree.
    fn put_long(target: &mut Vec<u8>, value: i64) {
        let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
        loop {
            let byte = u8::try_from(encoded & 0x7f).unwrap_or_default();
            encoded >>= 7;
            if encoded == 0 {
                target.push(byte);
                return;
            }
            target.push(byte | 0x80);
        }
    }

    /// Append a length-prefixed byte run, as Avro's `bytes` is encoded.
    fn put_bytes(target: &mut Vec<u8>, bytes: &[u8]) {
        put_long(target, bytes.len() as i64);
        target.extend_from_slice(bytes);
    }

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    /// Write a container by hand: magic, header, one block per payload.
    fn handmade_container(schema_json: &str, codec: &str, blocks: &[(i64, Vec<u8>)]) -> Buffer {
        handmade_container_with_header(
            &[
                ("avro.schema", schema_json.as_bytes()),
                ("avro.codec", codec.as_bytes()),
            ],
            blocks,
        )
    }

    /// Write a container with caller-controlled header entries for hardening tests.
    fn handmade_container_with_header(
        entries: &[(&str, &[u8])],
        blocks: &[(i64, Vec<u8>)],
    ) -> Buffer {
        let mut output = Vec::new();
        output.extend_from_slice(b"Obj\x01");
        put_long(&mut output, entries.len() as i64);
        for (key, value) in entries {
            put_bytes(&mut output, key.as_bytes());
            put_bytes(&mut output, value);
        }
        put_long(&mut output, 0);
        let sync = [7_u8; 16];
        output.extend_from_slice(&sync);
        for (count, payload) in blocks {
            put_long(&mut output, *count);
            put_bytes(&mut output, payload);
            output.extend_from_slice(&sync);
        }
        let mut handle = buffer();
        handle.write_all_bytes(&output).unwrap();
        handle
    }

    mod hardening {
        use yggdryl::avro;

        #[test]
        fn recursive_data_deeper_than_the_limit_is_an_error_not_a_crash() {
            // A linked list driven 300 levels deep by data alone: each level is
            // the union index for the "node" branch plus a zero value.
            let mut payload = Vec::new();
            for _ in 0..300 {
                payload.push(0x00); // value: long 0
                payload.push(0x02); // next: branch 1, the node itself
            }
            payload.push(0x00); // final value
            payload.push(0x00); // final next: branch 0, null
            let schema = r#"{"type":"record","name":"node","fields":[
            {"name":"value","type":"long"},
            {"name":"next","type":["null","node"]}
        ]}"#;
            let handle = super::handmade_container(schema, "null", &[(1, payload)]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("levels deep"), "{message}");
        }

        #[test]
        fn a_declared_length_beyond_the_container_is_a_typed_error() {
            // A string claiming a gigabyte that is not there.
            let mut payload = Vec::new();
            let mut encoded = (1_000_000_000_i64 << 1) as u64;
            loop {
                let byte = (encoded & 0x7f) as u8;
                encoded >>= 7;
                if encoded == 0 {
                    payload.push(byte);
                    break;
                }
                payload.push(byte | 0x80);
            }
            let handle = super::handmade_container("\"string\"", "null", &[(1, payload)]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("expected"), "{message}");
        }

        #[test]
        fn a_truncated_varint_is_a_typed_error() {
            let handle = super::handmade_container("\"long\"", "null", &[(1, vec![0x80, 0x80])]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("expected"), "{message}");
        }
    }

    mod matrix {
        use yggdryl::Scalar;
        use yggdryl::TimeUnit;
        use yggdryl::avro;

        /// Round-trip rows through a container and hand them back.
        fn round_trip(schema: &str, rows: &[Scalar]) -> Vec<Scalar> {
            let schema = yggdryl::json::from_utf8(schema).unwrap();
            let mut handle = super::buffer();
            avro::write_container(&mut handle, &schema, &[], rows).unwrap();
            avro::read_container(&handle).unwrap().rows
        }

        #[test]
        fn maps_round_trip_including_the_empty_one() {
            let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"counts","type":{"type":"map","values":"long"}}
        ]}"#;
            let full = Scalar::from_struct([(
                "counts",
                Scalar::from_mapping([
                    (Scalar::from("a"), Scalar::from(1_i64)),
                    (Scalar::from("b"), Scalar::from(-2_i64)),
                ])
                .unwrap(),
            )])
            .unwrap();
            let empty =
                Scalar::from_struct([("counts", Scalar::from_mapping([]).unwrap())]).unwrap();
            let rows = round_trip(schema, &[full.clone(), empty.clone()]);
            assert_eq!(rows, [full, empty]);
        }

        #[test]
        fn four_levels_of_nesting_round_trip() {
            // array<record<map<string, array<record<flag>>>>>
            let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"outer","type":{"type":"array","items":
                {"type":"record","name":"middle","fields":[
                    {"name":"by_name","type":{"type":"map","values":
                        {"type":"array","items":
                            {"type":"record","name":"leaf","fields":[
                                {"name":"flag","type":"boolean"}
                            ]}}}}
                ]}}}
        ]}"#;
            let row = yggdryl::json::from_utf8(
                r#"{"outer":[{"by_name":{"legs":[{"flag":true},{"flag":false}],"none":[]}}]}"#,
            )
            .unwrap();
            let rows = round_trip(schema, std::slice::from_ref(&row));
            assert!(rows[0].as_struct().is_some());
            assert!(
                rows[0]
                    .path("outer.0.by_name")
                    .unwrap()
                    .as_mapping()
                    .is_some()
            );
            assert_eq!(
                rows[0]
                    .path("outer.0.by_name.legs.1.flag")
                    .and_then(Scalar::as_bool),
                Some(false)
            );
        }

        #[test]
        fn unions_of_records_choose_the_branch_by_shape_order() {
            let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"v","type":["null",
                {"type":"record","name":"point","fields":[{"name":"x","type":"long"}]}
            ]}
        ]}"#;
            let some = yggdryl::json::from_utf8(r#"{"v":{"x":9}}"#).unwrap();
            let none = yggdryl::json::from_utf8(r#"{"v":null}"#).unwrap();
            let rows = round_trip(schema, &[some.clone(), none.clone()]);
            assert_eq!(rows, [some, none]);
        }

        #[test]
        fn time_boundaries_round_trip_exactly() {
            let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"ms","type":{"type":"int","logicalType":"time-millis"}},
            {"name":"us","type":{"type":"long","logicalType":"time-micros"}}
        ]}"#;
            let row = Scalar::from_struct([
                (
                    "ms",
                    Scalar::time32(0, TimeUnit::Millisecond, yggdryl::Timezone::NAIVE).unwrap(),
                ),
                (
                    "us",
                    Scalar::time64(
                        86_399_999_999,
                        TimeUnit::Microsecond,
                        yggdryl::Timezone::NAIVE,
                    )
                    .unwrap(),
                ),
            ])
            .unwrap();
            assert_eq!(round_trip(schema, std::slice::from_ref(&row))[0], row);
        }

        #[test]
        fn a_leap_day_survives_as_the_date_it_is() {
            // 2024-02-29 is day 19_782 since the epoch.
            let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"day","type":{"type":"int","logicalType":"date"}}
        ]}"#;
            let row = Scalar::from_struct([("day", Scalar::date32(19_782))]).unwrap();
            assert_eq!(round_trip(schema, std::slice::from_ref(&row))[0], row);
        }

        #[test]
        fn trailing_bytes_after_the_declared_rows_are_an_error() {
            // A block declaring one null row but carrying a stray byte.
            let handle = super::handmade_container("\"null\"", "null", &[(1, vec![0x2A])]);
            let message = yggdryl::avro::read_container(&handle)
                .unwrap_err()
                .to_string();
            assert!(message.contains("end after 1 declared rows"), "{message}");
        }
    }
}
