//! `rust/src/avro/container.rs`: the object container - magic, header, blocks
//! and sync markers - and what a malformed one reports.

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType, Scalar};

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

    /// A record schema exercising every branch the manifests use.
    fn manifest_shaped_schema() -> Scalar {
        yggdryl::json::from_utf8(
            r#"{"type":"record","name":"row","fields":[
            {"name":"code","type":"int","field-id":1},
            {"name":"name","type":"string","field-id":2},
            {"name":"score","type":["null","double"],"default":null,"field-id":3},
            {"name":"raw","type":["null","bytes"],"default":null,"field-id":4},
            {"name":"tags","type":{"type":"array","element-id":6,"items":"long"},
             "field-id":5},
            {"name":"nested","type":{"type":"record","name":"inner","fields":[
                {"name":"flag","type":"boolean","field-id":8}
            ]},"field-id":7}
        ]}"#,
        )
        .unwrap()
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

    mod containers {
        use super::{buffer, manifest_shaped_schema};
        use yggdryl::IOBase;
        use yggdryl::Scalar;
        use yggdryl::avro;

        #[test]
        fn a_container_round_trips_every_encoded_branch() {
            let schema = manifest_shaped_schema();
            let row = yggdryl::json::from_utf8(
                r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,300000],
                "nested":{"flag":true}}"#,
            )
            .unwrap();
            let empty = yggdryl::json::from_utf8(
                r#"{"code":0,"name":"","score":null,"raw":null,"tags":[],
                "nested":{"flag":false}}"#,
            )
            .unwrap();

            let mut handle = buffer();
            avro::write_container(
                &mut handle,
                &schema,
                &[("format-version", "2")],
                &[row.clone(), empty.clone()],
            )
            .unwrap();

            let container = avro::read_container(&handle).unwrap();
            assert_eq!(container.get("format-version"), Some("2"));
            assert_eq!(container.schema.kind(), "record");
            assert_eq!(container.rows.len(), 2);
            assert_eq!(
                container.rows[0].get_key_str("code").unwrap().as_i64(),
                Some(-7)
            );
            assert_eq!(
                container.rows[0].get_key_str("name").unwrap().as_str(),
                Some("AAPL")
            );
            assert_eq!(
                container.rows[0].get_key_str("score").unwrap().as_f64(),
                Some(1.5)
            );
            assert!(container.rows[0].get_key_str("raw").unwrap().is_null());
            assert_eq!(container.rows[0].get_key_str("tags").unwrap().len(), 3);
            assert_eq!(
                container.rows[1].get_key_str("tags").unwrap().len(),
                0,
                "an empty array is one zero-count block"
            );
            assert_eq!(
                container.rows[1]
                    .get_key_str("nested")
                    .and_then(|nested| nested.get_key_str("flag"))
                    .and_then(Scalar::as_bool),
                Some(false)
            );
        }

        #[test]
        fn an_empty_container_is_a_header_with_no_blocks() {
            let mut handle = buffer();
            avro::write_container(&mut handle, &manifest_shaped_schema(), &[], &[]).unwrap();
            let container = avro::read_container(&handle).unwrap();
            assert!(container.rows.is_empty());
        }

        #[test]
        fn bytes_that_are_not_a_container_say_what_was_expected() {
            let mut handle = buffer();
            handle.write_all_bytes(b"not avro at all").unwrap();
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("Avro object container"), "{message}");
        }

        #[test]
        fn a_truncated_container_reports_the_byte_it_ran_out_at() {
            let mut handle = buffer();
            avro::write_container(
                &mut handle,
                &manifest_shaped_schema(),
                &[],
                &[yggdryl::json::from_utf8(
                    r#"{"code":1,"name":"x","score":null,"raw":null,"tags":[],
                    "nested":{"flag":true}}"#,
                )
                .unwrap()],
            )
            .unwrap();

            let mut truncated = buffer();
            let bytes = handle.read_all_bytes().unwrap();
            truncated
                .write_all_bytes(&bytes[..bytes.len() - 8])
                .unwrap();
            let message = avro::read_container(&truncated).unwrap_err().to_string();
            assert!(message.contains("avro"), "{message}");
            assert!(message.contains("expected"), "{message}");
        }

        #[test]
        fn a_wrong_sync_marker_is_reported_after_the_block() {
            let mut handle = buffer();
            avro::write_container(
                &mut handle,
                &yggdryl::json::from_utf8(
                    r#"{"type":"record","name":"r","fields":[{"name":"v","type":"long"}]}"#,
                )
                .unwrap(),
                &[],
                &[yggdryl::json::from_utf8(r#"{"v":1}"#).unwrap()],
            )
            .unwrap();
            let mut bytes = handle.read_all_bytes().unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 0xFF;
            let mut corrupt = buffer();
            corrupt.write_all_bytes(&bytes).unwrap();
            let message = avro::read_container(&corrupt).unwrap_err().to_string();
            assert!(message.contains("synchronization marker"), "{message}");
        }

        #[test]
        fn an_unknown_codec_is_refused_by_name() {
            let handle = super::handmade_container("\"long\"", "bzip2", &[]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("bzip2"), "{message}");
            assert!(message.contains("deflate"), "{message}");
        }

        #[test]
        fn an_oversized_row_count_is_an_error_and_not_an_allocation() {
            // A block claiming ten million zero-byte rows must die on the row
            // cap, not spin decoding nothing.
            let handle = super::handmade_container("\"null\"", "null", &[(10_000_000, Vec::new())]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("rows"), "{message}");
        }

        #[test]
        fn tight_limits_bound_the_container_read() {
            let mut handle = buffer();
            avro::write_container(&mut handle, &manifest_shaped_schema(), &[], &[]).unwrap();
            let limits = yggdryl::Limits::new(64, 16, 1_000, 8);
            let message = avro::read_container_with_limits(&handle, limits)
                .unwrap_err()
                .to_string();
            assert!(message.contains("at most 16 bytes"), "{message}");
        }

        #[test]
        fn a_row_budget_is_applied_after_the_mandatory_header() {
            let schema = yggdryl::json::from_utf8(r#""long""#).unwrap();
            let rows = [Scalar::from(1), Scalar::from(2)];
            let mut handle = buffer();
            avro::write_container(&mut handle, &schema, &[], &rows).unwrap();

            let limits = yggdryl::Limits::new(128, 1 << 20, 1, 1_024);
            let message = avro::read_container_with_limits(&handle, limits)
                .unwrap_err()
                .to_string();
            assert!(message.contains("at most 1 rows"), "{message}");
        }
    }

    mod streaming {
        use yggdryl::Scalar;
        use yggdryl::avro;

        #[test]
        fn blocks_stream_and_skipping_costs_nothing() {
            // Two handmade blocks of one long each: 7 and 9.
            let encode_long = |value: i64| -> Vec<u8> {
                let mut output = Vec::new();
                let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
                loop {
                    let byte = (encoded & 0x7f) as u8;
                    encoded >>= 7;
                    if encoded == 0 {
                        output.push(byte);
                        break;
                    }
                    output.push(byte | 0x80);
                }
                output
            };
            let handle = super::handmade_container(
                "\"long\"",
                "null",
                &[(1, encode_long(7)), (1, encode_long(9))],
            );

            let mut blocks = avro::read_blocks(&handle).unwrap();
            assert_eq!(blocks.schema().kind(), "long");
            let first = blocks.next_block().unwrap().unwrap();
            assert_eq!(first.count(), 1);
            // The first block is skipped: never decompressed, never decoded.
            let second = blocks.next_block().unwrap().unwrap();
            assert_eq!(second.rows().unwrap(), [Scalar::from(9)]);
            assert!(blocks.next_block().unwrap().is_none());
        }

        #[test]
        fn a_written_container_streams_back_the_same_rows() {
            let schema = super::manifest_shaped_schema();
            let row = yggdryl::json::from_utf8(
                r#"{"code":1,"name":"x","score":null,"raw":null,"tags":[],"nested":{"flag":true}}"#,
            )
            .unwrap();
            let mut handle = super::buffer();
            avro::write_container(
                &mut handle,
                &schema,
                &[("k", "v")],
                std::slice::from_ref(&row),
            )
            .unwrap();

            let mut blocks = avro::read_blocks(&handle).unwrap();
            assert_eq!(blocks.get("k"), Some("v"));
            let block = blocks.next_block().unwrap().unwrap();
            assert_eq!(block.rows().unwrap(), [row]);
            assert!(blocks.next_block().unwrap().is_none());
        }

        #[test]
        fn owning_blocks_keep_the_stream_lazy_and_honor_limits() {
            let encode_long = |value: i64| -> Vec<u8> {
                let mut output = Vec::new();
                let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
                loop {
                    let byte = (encoded & 0x7f) as u8;
                    encoded >>= 7;
                    output.push(if encoded == 0 { byte } else { byte | 0x80 });
                    if encoded == 0 {
                        return output;
                    }
                }
            };
            let source = super::handmade_container(
                "\"long\"",
                "null",
                &[(1, encode_long(7)), (1, encode_long(9))],
            );
            let mut blocks = avro::read_blocks_owned(source).unwrap();
            assert_eq!(blocks.schema().kind(), "long");
            assert!(blocks.metadata().is_empty());
            let first = blocks.next_block().unwrap().unwrap();
            assert_eq!(first.count(), 1);
            // The first payload may be discarded without ever decoding it.
            let second = blocks.next_block().unwrap().unwrap();
            assert_eq!(second.rows().unwrap(), [Scalar::from(9)]);
            assert!(blocks.next_block().unwrap().is_none());

            let source = super::handmade_container("\"long\"", "null", &[]);
            let limits = yggdryl::Limits::new(128, 1, 1_000_000, 1_024);
            let message = avro::read_blocks_owned_with_limits(source, limits)
                .err()
                .expect("the one-byte schema limit must fail")
                .to_string();
            assert!(message.contains("at most 1 bytes"), "{message}");
        }

        #[test]
        fn a_lazy_block_applies_its_row_budget_after_opening_the_header() {
            let source = super::handmade_container("\"null\"", "null", &[(3, Vec::new())]);
            let limits = yggdryl::Limits::new(128, 1 << 20, 2, 1_024);
            let mut blocks = avro::read_blocks_with_limits(&source, limits).unwrap();

            let message = blocks.next_block().unwrap_err().to_string();
            assert!(message.contains("at most 2 rows"), "{message}");
        }
    }

    #[cfg(feature = "parquet")]
    mod snappy {
        use yggdryl::Scalar;
        use yggdryl::avro;

        /// Encode one long, snappy-compress it, and append the big-endian CRC-32.
        fn snappy_block(value: i64) -> Vec<u8> {
            let mut body = Vec::new();
            let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
            loop {
                let byte = (encoded & 0x7f) as u8;
                encoded >>= 7;
                if encoded == 0 {
                    body.push(byte);
                    break;
                }
                body.push(byte | 0x80);
            }
            let mut compressed = snap::raw::Encoder::new().compress_vec(&body).unwrap();
            let mut crc = flate2::Crc::new();
            crc.update(&body);
            compressed.extend_from_slice(&crc.sum().to_be_bytes());
            compressed
        }

        #[test]
        fn snappy_blocks_decode_and_verify_their_crc() {
            let handle = super::handmade_container("\"long\"", "snappy", &[(1, snappy_block(42))]);
            let container = avro::read_container(&handle).unwrap();
            assert_eq!(container.rows, [Scalar::from(42)]);
        }

        #[test]
        fn a_corrupt_snappy_crc_is_refused() {
            let mut block = snappy_block(42);
            let last = block.len() - 1;
            block[last] ^= 0xFF;
            let handle = super::handmade_container("\"long\"", "snappy", &[(1, block)]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("CRC-32"), "{message}");
        }
    }

    mod hardening {
        use yggdryl::avro;

        #[test]
        fn duplicate_header_keys_are_refused_before_map_projection() {
            let handle = super::handmade_container_with_header(
                &[
                    ("avro.schema", b"\"long\""),
                    ("avro.schema", b"\"null\""),
                    ("avro.codec", b"null"),
                ],
                &[],
            );
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("duplicate \"avro.schema\""), "{message}");
            let message = avro::read_blocks(&handle).err().unwrap().to_string();
            assert!(message.contains("duplicate \"avro.schema\""), "{message}");
        }

        #[test]
        fn a_block_declaring_an_absurd_row_count_is_refused_without_reserving() {
            // The count is a claim, not a measurement: a block declaring more
            // rows than the node limit must fail the cap check, never size an
            // allocation by it.
            let handle = super::handmade_container("\"long\"", "null", &[(i64::MAX, vec![0x00])]);
            let message = avro::read_container(&handle).unwrap_err().to_string();
            assert!(message.contains("expected at most"), "{message}");
        }
    }

    mod snapshots {
        use yggdryl::IOBase;

        use yggdryl::avro;

        /// The byte snapshot of one fixed schema and data pair.
        ///
        /// The writer is a pure function of its input, so any encoding change -
        /// varints, field order, header layout, the derived sync marker, the
        /// deflate stream - surfaces here immediately. Update the expectation
        /// only for a deliberate format change, never to quiet the test.
        #[test]
        fn a_fixed_container_encodes_to_exactly_these_bytes() {
            let schema = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"snap","fields":[
                {"name":"id","type":"long"},
                {"name":"tag","type":"string"}
            ]}"#,
            )
            .unwrap();
            let rows = [
                yggdryl::json::from_utf8(r#"{"id":1,"tag":"a"}"#).unwrap(),
                yggdryl::json::from_utf8(r#"{"id":-2,"tag":"bc"}"#).unwrap(),
            ];
            let mut handle = super::buffer();
            avro::write_container(&mut handle, &schema, &[("k", "v")], &rows).unwrap();
            let bytes = handle.read_all_bytes().unwrap();
            let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            let expected = concat!(
                "4f626a0106166176726f2e736368656d61ca017b226669656c6473223a5b7b226e",
                "616d65223a226964222c2274797065223a226c6f6e67227d2c7b226e616d65223a",
                "22746167222c2274797065223a22737472696e67227d5d2c226e616d65223a2273",
                "6e6170222c2274797065223a227265636f7264227d146176726f2e636f6465630e",
                "6465666c617465026b027600c98ecdf13366dd2eb73d0abf308c2d40041263624a",
                "6466494a0600c98ecdf13366dd2eb73d0abf308c2d40",
            );
            assert_eq!(
                hex, expected,
                "the container's bytes changed; was that deliberate?"
            );

            // The same input encodes to the same bytes, every time.
            let mut again = super::buffer();
            avro::write_container(&mut again, &schema, &[("k", "v")], &rows).unwrap();
            assert_eq!(bytes, again.read_all_bytes().unwrap());

            // And they still decode to the rows that produced them.
            let decoded = avro::read_container(&handle).unwrap();
            assert_eq!(decoded.rows.to_vec(), rows);
            assert_eq!(decoded.get("k"), Some("v"));
        }
    }
}
