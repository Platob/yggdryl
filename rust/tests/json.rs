use std::io::{Cursor, Read, Write};
use yggdryl::json;

use yggdryl::Value;
use yggdryl::text::{self, Format, Limits};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn borrowed_text_entry_points_match_bytes_without_an_intermediate_buffer() {
    let source = "{\"label\":\"café\",\"value\":1}";
    let expected = json::from_slice(source.as_bytes()).unwrap();
    assert_eq!(json::from_str(source).unwrap(), expected);
    assert_eq!(
        json::from_str_with_limits(source, Limits::default()).unwrap(),
        expected
    );

    let stream = "1\n{\"label\":\"café\"}";
    assert_eq!(
        json::from_str_all(stream).unwrap(),
        json::from_slice_all(stream.as_bytes()).unwrap()
    );
    assert_eq!(
        json::from_str_all_with_limits(stream, Limits::default()).unwrap(),
        json::from_slice_all(stream.as_bytes()).unwrap()
    );
}

#[test]
fn typed_values_round_trip_without_reserved_key_collisions() {
    let collision = Value::from_mapping([(
        Value::from("$yggdryl"),
        Value::from_mapping([
            (Value::from("version"), Value::from(1_u64)),
            (Value::from("type"), Value::from("bytes")),
            (Value::from("value"), Value::from("not actually bytes")),
        ])
        .unwrap(),
    )])
    .unwrap();
    let value = Value::from_sequence([
        Value::from(vec![0, 1, 255]),
        Value::I128(i128::MAX),
        Value::U128(u128::MAX),
        Value::from(f64::NAN),
        collision,
    ]);
    let encoded = json::to_vec(&value).unwrap();
    assert_eq!(json::from_slice(&encoded).unwrap(), value);
}

#[test]
fn a_tag_envelope_is_now_the_ordinary_mapping_it_is_spelled_as() {
    // The `tag` kind named a carrier this value model no longer has, so it is
    // no longer an envelope kind. A document still spelling one is user data
    // whose outer key happens to be the marker, and the mapping envelope is
    // what carries that back out unchanged.
    let source =
        br#"{"$yggdryl":{"version":1,"type":"tag","tag":"python:orders.Trade","value":null}}"#;
    let decoded = json::from_slice(source).unwrap();
    let body = Value::from_mapping([
        (Value::from("version"), Value::from(1_u64)),
        (Value::from("type"), Value::from("tag")),
        (Value::from("tag"), Value::from("python:orders.Trade")),
        (Value::from("value"), Value::Null),
    ])
    .unwrap();
    assert_eq!(
        decoded,
        Value::from_mapping([(Value::from("$yggdryl"), body)]).unwrap()
    );

    let encoded = json::to_vec(&decoded).unwrap();
    assert_eq!(json::from_slice(&encoded).unwrap(), decoded);
}

#[test]
fn non_exact_envelopes_remain_plain_mappings() {
    for encoded in [
        br#"{"$yggdryl":{"version":2,"type":"bytes","value":"AA=="}}"#.as_slice(),
        br#"{"$yggdryl":{"version":1,"type":"future","value":"AA=="}}"#.as_slice(),
        br#"{"$yggdryl":{"version":1,"type":"bytes"}}"#.as_slice(),
        br#"{"$yggdryl":{"version":1,"type":"bytes","value":"AA==","extra":true}}"#.as_slice(),
    ] {
        assert!(matches!(
            json::from_slice(encoded).unwrap(),
            Value::Mapping(_)
        ));
    }
}

#[test]
fn recognized_envelope_errors_keep_json_identity() {
    let error =
        json::from_slice(br#"{"$yggdryl":{"version":1,"type":"bytes","value":"not-base64"}}"#)
            .unwrap_err();
    match error {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "json");
            assert_eq!(position, 0);
            assert_eq!(reason, "invalid base64 bytes envelope");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[test]
fn all_integer_storage_variants_round_trip_by_value() {
    for value in [
        Value::I64(1),
        Value::I64(-1),
        Value::U64(1),
        Value::I128(1),
        Value::I128(i128::MIN),
        Value::U128(1),
        Value::U128(u128::MAX),
    ] {
        assert_eq!(
            json::from_slice(&json::to_vec(&value).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn plain_json_numbers_preserve_exact_128_bit_boundaries() {
    for (encoded, expected) in [
        (u64::MAX.to_string(), Value::U64(u64::MAX)),
        (
            (u128::from(u64::MAX) + 1).to_string(),
            Value::U128(u128::from(u64::MAX) + 1),
        ),
        (u128::MAX.to_string(), Value::U128(u128::MAX)),
        (i64::MIN.to_string(), Value::I64(i64::MIN)),
        (
            (i128::from(i64::MIN) - 1).to_string(),
            Value::I128(i128::from(i64::MIN) - 1),
        ),
        (i128::MIN.to_string(), Value::I128(i128::MIN)),
    ] {
        assert_eq!(json::from_slice(encoded.as_bytes()).unwrap(), expected);
        assert_eq!(
            json::from_reader(Cursor::new(encoded.as_bytes())).unwrap(),
            expected
        );
        assert_eq!(
            json::from_lines_slice(encoded.as_bytes()).unwrap(),
            vec![expected]
        );
    }

    let values = json::Reader::new(Cursor::new(b"18446744073709551617 -9223372036854775809"))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        values,
        vec![
            Value::U128(18_446_744_073_709_551_617),
            Value::I128(-9_223_372_036_854_775_809),
        ]
    );
}

#[test]
fn plain_json_number_overflow_and_float_semantics_are_deterministic() {
    for (encoded, reason) in [
        (
            b"340282366920938463463374607431768211456".as_slice(),
            "unsigned 128-bit",
        ),
        (
            b"-170141183460469231731687303715884105729".as_slice(),
            "signed 128-bit",
        ),
    ] {
        for error in [
            json::from_slice(encoded).unwrap_err(),
            json::from_reader(Cursor::new(encoded)).unwrap_err(),
            json::from_lines_slice(encoded).unwrap_err(),
        ] {
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    for encoded in [b"1.25".as_slice(), b"1e3", b"18446744073709551617.0"] {
        assert!(matches!(
            json::from_slice(encoded).unwrap(),
            Value::Float(_)
        ));
    }
    for encoded in [b"-0".as_slice(), b"-0.0", b"-0e3"] {
        let value = json::from_slice(encoded).unwrap();
        let float = value.as_f64().expect("negative zero is a float");
        assert_eq!(float.to_bits(), (-0.0_f64).to_bits());
    }
    assert!(
        json::from_slice(b"1e400")
            .unwrap_err()
            .to_string()
            .contains("finite f64")
    );

    let second = b"0\n340282366920938463463374607431768211456";
    match json::from_slice_all(second).unwrap_err() {
        yggdryl::Error::Codec { position, .. } => assert_eq!(position, 2),
        other => panic!("unexpected error: {other}"),
    }

    let limits = Limits::new(1, 16, 3, 2);
    match json::from_slice_all_with_limits(b"0 1 2", limits).unwrap_err() {
        yggdryl::Error::Codec { position, .. } => assert_eq!(position, 4),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn serde_json_private_number_key_never_collides_with_plain_objects() {
    let encoded = br#"{"$serde_json::private::Number":"18446744073709551617"}"#;
    let value = json::from_slice(encoded).unwrap();
    assert_eq!(
        value.get_key_str("$serde_json::private::Number"),
        Some(&Value::from("18446744073709551617"))
    );
    assert_eq!(
        json::from_slice(b"18446744073709551617").unwrap(),
        Value::U128(18_446_744_073_709_551_617)
    );
}

#[test]
fn duplicate_object_keys_report_the_exact_second_decoded_key() {
    let input = br#"{"ok":0,"nested":{"a":1,"\u0061":2}}"#;
    let expected = br#"{"ok":0,"nested":{"a":1,"#.len();
    for error in [
        json::from_slice(input).unwrap_err(),
        json::from_str(std::str::from_utf8(input).unwrap()).unwrap_err(),
        json::from_reader(Cursor::new(input)).unwrap_err(),
    ] {
        match error {
            yggdryl::Error::Codec {
                format,
                position,
                reason,
            } => {
                assert_eq!(format, "json");
                assert_eq!(position, expected);
                assert!(reason.contains("duplicate key"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    let mut wide = String::from("{");
    for index in 0..17 {
        if index != 0 {
            wide.push(',');
        }
        wide.push_str(&format!("\"key{index}\":{index}"));
    }
    wide.push_str(",\"\\u006bey0\":99}");
    let expected = wide.find("\"\\u006bey0\"").unwrap();
    match json::from_str(&wide).unwrap_err() {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "json");
            assert_eq!(position, expected);
            assert!(reason.contains("duplicate key"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn parser_limits_reject_depth_and_nodes_during_deserialization() {
    let depth_limits = Limits::new(2, 1024, 100, 10);
    assert!(json::from_slice_with_limits(b"[[0]]", depth_limits).is_ok());
    let error = json::from_slice_with_limits(b"[[[0]]]", depth_limits).unwrap_err();
    assert!(error.to_string().contains("nesting depth limit"));

    let node_limits = Limits::new(16, 1024, 3, 10);
    let error = json::from_slice_with_limits(b"[0,1,2]", node_limits).unwrap_err();
    assert!(error.to_string().contains("node limit"));
}

#[test]
fn zero_and_exact_resource_boundaries_are_deterministic() {
    assert!(json::from_slice_with_limits(b"0", Limits::new(0, 1, 1, 1)).is_ok());
    assert!(json::from_slice_with_limits(b"[]", Limits::new(0, 2, 1, 1)).is_err());
    assert!(json::from_slice_with_limits(b"0", Limits::new(0, 1, 0, 1)).is_err());
    assert!(json::from_slice_with_limits(b"0", Limits::new(0, 1, 1, 0)).is_err());
    assert!(json::from_slice_with_limits(b"00", Limits::new(0, 1, 1, 1)).is_err());

    let limits = Limits::new(1, 16, 1, 1);
    let mut reader = json::Reader::with_limits(Cursor::new(b"0 1"), limits);
    assert!(reader.next().unwrap().is_ok());
    assert!(reader.next().unwrap().is_err());
}

#[test]
fn reader_byte_limit_checks_the_sentinel_after_valid_data() {
    let input = b"null     ";
    let limits = Limits::new(16, 4, 100, 10);
    let error = json::from_reader_with_limits(Cursor::new(input), limits).unwrap_err();
    assert!(error.to_string().contains("input byte limit"));
}

#[test]
fn json_lines_are_strict_and_report_cumulative_offsets() {
    let decoded = text::from_slice(b"1\r\n\n{\"x\":2}\n", Format::JsonLines).unwrap();
    assert_eq!(decoded.len(), 2);
    assert!(text::from_slice(b"1 2\n", Format::JsonLines).is_err());
    assert!(text::from_slice(b"{\n\"x\": 1\n}\n", Format::JsonLines).is_err());

    let error = text::from_slice(b"1\n{bad}\n", Format::JsonLines).unwrap_err();
    let position = match error {
        yggdryl::Error::Codec { position, .. } => position,
        other => panic!("unexpected error: {other}"),
    };
    assert!(position >= 2);
}

#[test]
fn borrowed_json_lines_preserve_empty_crlf_unicode_and_final_rows() {
    assert!(json::from_lines_str("").unwrap().is_empty());
    assert!(json::from_lines_str("\r\n \t\r\n").unwrap().is_empty());

    let input = "\r\n{\"label\":\"café\"}\r\n42";
    let expected = vec![
        Value::from_mapping([(Value::from("label"), Value::from("café"))]).unwrap(),
        Value::from(42_u64),
    ];
    assert_eq!(json::from_lines_str(input).unwrap(), expected);
    assert_eq!(json::from_lines_slice(input.as_bytes()).unwrap(), expected);
    assert_eq!(
        json::LinesReader::new(Cursor::new(input.as_bytes()))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap(),
        expected
    );

    let exact = Limits::new(8, input.len(), 8, 2);
    assert_eq!(
        json::from_lines_str_with_limits(input, exact).unwrap(),
        expected
    );
    assert!(
        json::from_lines_str_with_limits(
            input,
            Limits::new(8, input.len().saturating_sub(1), 8, 2),
        )
        .is_err()
    );

    let malformed = "\n{\"label\":\"café\"}\r\n{bad}";
    let failure = "\n{\"label\":\"café\"}\r\n".len() + 1;
    match json::from_lines_str(malformed).unwrap_err() {
        yggdryl::Error::Codec { position, .. } => assert_eq!(position, failure),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn json_lines_skip_only_json_whitespace_rows() {
    for control in *b"\x0b\x0c" {
        let input = [control, b'\n', b'1', b'\n'];

        for error in [
            json::from_lines_slice(&input).unwrap_err(),
            json::from_lines_str(std::str::from_utf8(&input).unwrap()).unwrap_err(),
            json::from_lines_reader(Cursor::new(input)).unwrap_err(),
        ] {
            match error {
                yggdryl::Error::Codec { position, .. } => assert_eq!(position, 0),
                other => panic!("unexpected error: {other}"),
            }
        }
    }
}

#[test]
fn borrowed_and_owning_readers_stream_values() {
    let mut input = Cursor::new(b"1\n2\n3\n");
    let values = json::from_lines_reader_iter(&mut input)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values, vec![Value::U64(1), Value::U64(2), Value::U64(3)]);

    let values = json::Reader::new(Cursor::new(b"true false null"))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        values,
        vec![Value::Bool(true), Value::Bool(false), Value::Null]
    );
}

#[test]
fn malformed_reader_item_reports_the_original_cumulative_byte() {
    let input = "{\"é\": 0}\n[1, }\n".as_bytes();
    let failure = "{\"é\": 0}\n[1, ".len();
    let mut reader = json::Reader::new(Cursor::new(input));
    assert!(reader.next().unwrap().is_ok());
    let error = reader.next().unwrap().unwrap_err();
    match error {
        yggdryl::Error::Codec {
            format, position, ..
        } => {
            assert_eq!(format, "json");
            assert_eq!(position, failure);
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(reader.next().is_none());

    let newline_count = 128 * 1024;
    let mut newline_heavy = vec![b'\n'; newline_count];
    newline_heavy.push(b'x');
    let error = json::Reader::new(Cursor::new(newline_heavy))
        .next()
        .unwrap()
        .unwrap_err();
    match error {
        yggdryl::Error::Codec { position, .. } => assert_eq!(position, newline_count),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn single_document_paths_report_the_start_of_trailing_json() {
    let input = "\"café\"\r\n\t {\"next\":true}";
    let expected = "\"café\"\r\n\t ".len();
    for error in [
        json::from_slice(input.as_bytes()).unwrap_err(),
        json::from_str(input).unwrap_err(),
        json::from_reader(Cursor::new(input.as_bytes())).unwrap_err(),
        json::from_reader(OneByte(Cursor::new(input.as_bytes()))).unwrap_err(),
    ] {
        match error {
            yggdryl::Error::Codec {
                format, position, ..
            } => {
                assert_eq!(format, "json");
                assert_eq!(position, expected);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

#[test]
fn chunked_readers_and_short_writers_preserve_bytes() {
    struct Chunks<R> {
        inner: R,
        size: usize,
    }

    impl<R: Read> Read for Chunks<R> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let length = output.len().min(self.size);
            self.inner.read(&mut output[..length])
        }
    }

    #[derive(Default)]
    struct ShortWriter(Vec<u8>);

    impl Write for ShortWriter {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let length = input.len().min(3);
            self.0.extend_from_slice(&input[..length]);
            Ok(length)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let value = Value::from_mapping([(
        Value::from("items"),
        Value::from_sequence([Value::from(1_i64), Value::from(2_i64)]),
    )])
    .unwrap();
    let encoded = json::to_vec(&value).unwrap();
    let reader = Chunks {
        inner: Cursor::new(&encoded),
        size: 2,
    };
    assert_eq!(json::from_reader(reader).unwrap(), value);

    let mut writer = ShortWriter::default();
    json::to_writer(&mut writer, &value).unwrap();
    assert_eq!(writer.0, encoded);
}

#[test]
fn iterator_writer_does_not_require_a_materialized_value_slice() {
    let mut output = Vec::new();
    json::to_writer_all(&mut output, (0_u64..3).map(Value::from)).unwrap();
    assert_eq!(output, b"0\n1\n2\n");
}

#[test]
fn encoder_rejects_one_over_the_default_depth_limit() {
    let maximum = Limits::default().max_depth();
    let at_limit = (0..maximum).fold(Value::Null, |value, _| Value::from_sequence([value]));
    let over_limit = Value::from_sequence([at_limit.clone()]);

    assert!(json::to_vec(&at_limit).is_ok());
    let error = json::to_vec(&over_limit).unwrap_err();
    assert!(error.to_string().contains("nesting depth limit"));
}

#[test]
fn reader_accepts_exact_document_limit_and_rejects_only_the_next_value() {
    let limits = Limits::new(16, 64, 16, 2);
    let values = json::Reader::with_limits(Cursor::new(b"1 2"), limits)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values, vec![Value::U64(1), Value::U64(2)]);

    let mut values = json::Reader::with_limits(Cursor::new(b"1 2 3"), limits);
    assert!(values.next().unwrap().is_ok());
    assert!(values.next().unwrap().is_ok());
    assert!(values.next().unwrap().is_err());
    assert!(values.next().is_none());
}

#[test]
fn custom_depth_limits_replace_serde_jsons_default_recursion_cap() {
    let depth = 300;
    let input = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    let limits = Limits::new(depth, input.len(), depth + 1, 1);
    assert!(json::from_slice_with_limits(input.as_bytes(), limits).is_ok());

    let over = format!("{}0{}", "[".repeat(depth + 1), "]".repeat(depth + 1));
    let error = json::from_slice_with_limits(
        over.as_bytes(),
        Limits::new(depth, over.len(), depth + 2, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("nesting depth limit"));

    let hard_limit = json::MAX_PARSER_DEPTH;
    let at_hard_limit = format!("{}0{}", "[".repeat(hard_limit), "]".repeat(hard_limit));
    assert!(
        json::from_slice_with_limits(
            at_hard_limit.as_bytes(),
            Limits::new(hard_limit + 1, at_hard_limit.len(), hard_limit + 1, 1,),
        )
        .is_ok()
    );

    let over_hard_limit = format!(
        "{}0{}",
        "[".repeat(hard_limit + 1),
        "]".repeat(hard_limit + 1)
    );
    let error = json::from_slice_with_limits(
        over_hard_limit.as_bytes(),
        Limits::new(hard_limit + 100, over_hard_limit.len(), hard_limit + 2, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("parser hard limit of 384"));
}
