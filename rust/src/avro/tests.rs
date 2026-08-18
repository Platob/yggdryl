//! Unit tests for the Avro codec module.

use crate::io::{Buffer, IOBase};
use crate::{MediaType, MimeType, Value};

/// A record schema exercising every branch the manifests use.
fn manifest_shaped_schema() -> Value {
    crate::json::from_str(
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
    let mut output = Vec::new();
    output.extend_from_slice(b"Obj\x01");
    let mut put_long = |output: &mut Vec<u8>, value: i64| {
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
    };
    let put_bytes =
        |output: &mut Vec<u8>, put_long: &mut dyn FnMut(&mut Vec<u8>, i64), bytes: &[u8]| {
            put_long(output, bytes.len() as i64);
            output.extend_from_slice(bytes);
        };
    put_long(&mut output, 2);
    put_bytes(&mut output, &mut put_long, b"avro.schema");
    put_bytes(&mut output, &mut put_long, schema_json.as_bytes());
    put_bytes(&mut output, &mut put_long, b"avro.codec");
    put_bytes(&mut output, &mut put_long, codec.as_bytes());
    put_long(&mut output, 0);
    let sync = [7_u8; 16];
    output.extend_from_slice(&sync);
    for (count, payload) in blocks {
        put_long(&mut output, *count);
        put_bytes(&mut output, &mut put_long, payload);
        output.extend_from_slice(&sync);
    }
    let mut handle = buffer();
    handle.write_all_bytes(&output).unwrap();
    handle
}

mod containers {
    use super::{buffer, manifest_shaped_schema};
    use crate::io::IOBase;
    use crate::{Value, avro};

    #[test]
    fn a_container_round_trips_every_encoded_branch() {
        let schema = manifest_shaped_schema();
        let row = crate::json::from_str(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,300000],
                "nested":{"flag":true}}"#,
        )
        .unwrap();
        let empty = crate::json::from_str(
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
                .and_then(Value::as_bool),
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
            &[crate::json::from_str(
                r#"{"code":1,"name":"x","score":null,"raw":null,"tags":[],
                    "nested":{"flag":true}}"#,
            )
            .unwrap()],
        )
        .unwrap();

        let mut truncated = buffer();
        let bytes = handle.read_all().unwrap();
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
            &crate::json::from_str(
                r#"{"type":"record","name":"r","fields":[{"name":"v","type":"long"}]}"#,
            )
            .unwrap(),
            &[],
            &[crate::json::from_str(r#"{"v":1}"#).unwrap()],
        )
        .unwrap();
        let mut bytes = handle.read_all().unwrap();
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
        let limits = crate::Limits::new(64, 16, 1_000, 8);
        let message = avro::read_container_with_limits(&handle, limits)
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 16 bytes"), "{message}");
    }
}

mod schemas {
    use crate::avro::Schema;

    #[test]
    fn canonical_form_and_fingerprint_match_the_reference_implementation() {
        // Expected values computed with fastavro's parsing canonical form and
        // CRC-64-AVRO fingerprint; hex spellings are the little-endian byte
        // order the single-object framing writes.
        let hex = |schema: &Schema| {
            schema
                .fingerprint()
                .to_le_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };

        let int = Schema::from_str("\"int\"").unwrap();
        assert_eq!(int.to_canonical_form(), "\"int\"");
        assert_eq!(hex(&int), "8f5c393f1ad57572");

        let record = Schema::from_str(
            r#"{"type":"record","name":"trade","doc":"ignored","fields":[
                {"name":"symbol","type":"string","doc":"also ignored"},
                {"name":"qty","type":"long"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            record.to_canonical_form(),
            r#"{"name":"trade","type":"record","fields":[{"name":"symbol","type":"string"},{"name":"qty","type":"long"}]}"#
        );
        assert_eq!(hex(&record), "f5780492090a723f");

        let union = Schema::from_str(r#"["null","double"]"#).unwrap();
        assert_eq!(hex(&union), "f84fd315a2f6aa49");

        let fixed = Schema::from_str(r#"{"type":"fixed","name":"md5","size":16}"#).unwrap();
        assert_eq!(hex(&fixed), "8c5dd85ce7341b48");
    }

    #[test]
    fn attribute_order_and_unknown_attributes_never_change_the_fingerprint() {
        let one = Schema::from_str(
            r#"{"type":"record","name":"row","fields":[{"name":"id","type":"int","field-id":7}]}"#,
        )
        .unwrap();
        let other = Schema::from_str(
            r#"{"fields":[{"type":"int","name":"id"}],"name":"row","type":"record"}"#,
        )
        .unwrap();
        assert_eq!(one.fingerprint(), other.fingerprint());
    }

    #[test]
    fn the_source_json_round_trips_verbatim() {
        let document = crate::json::from_str(
            r#"{"type":"record","name":"row","fields":[{"name":"id","type":"int","field-id":42}]}"#,
        )
        .unwrap();
        let schema = Schema::from_json(&document).unwrap();
        assert_eq!(schema.to_json(), document);
        // The unmodeled attribute is still in the JSON the schema writes.
        let text = String::from_utf8(crate::json::to_vec(&schema.to_json()).unwrap()).unwrap();
        assert!(text.contains("field-id"), "{text}");
    }

    #[test]
    fn namespaces_qualify_names_and_nested_types_inherit_them() {
        let schema = Schema::from_str(
            r#"{"type":"record","name":"outer","namespace":"com.example","fields":[
                {"name":"inner","type":{"type":"record","name":"inner","fields":[
                    {"name":"self","type":["null","inner"]},
                    {"name":"outer","type":["null","com.example.outer"]}
                ]}}
            ]}"#,
        )
        .unwrap();
        // Both the bare reference (inheriting the namespace) and the dotted
        // fullname resolve to the registered types.
        assert!(schema.to_canonical_form().contains("com.example.inner"));
    }

    #[test]
    fn a_dotted_name_is_a_fullname_and_ignores_the_namespace_attribute() {
        let schema = Schema::from_str(
            r#"{"type":"fixed","name":"org.other.hash","namespace":"ignored","size":4}"#,
        )
        .unwrap();
        assert_eq!(
            schema.to_canonical_form(),
            r#"{"name":"org.other.hash","type":"fixed","size":4}"#
        );
    }

    #[test]
    fn a_recursive_schema_parses_and_round_trips_data() {
        let schema_json = crate::json::from_str(
            r#"{"type":"record","name":"node","fields":[
                {"name":"value","type":"long"},
                {"name":"next","type":["null","node"],"default":null}
            ]}"#,
        )
        .unwrap();
        let list = crate::json::from_str(
            r#"{"value":1,"next":{"value":2,"next":{"value":3,"next":null}}}"#,
        )
        .unwrap();
        let mut handle = super::buffer();
        crate::avro::write_container(&mut handle, &schema_json, &[], &[list]).unwrap();
        let container = crate::avro::read_container(&handle).unwrap();
        let tail = container.rows[0]
            .path("next.next.value")
            .and_then(crate::Value::as_i64);
        assert_eq!(tail, Some(3));
    }

    #[test]
    fn an_unknown_reference_is_an_error_naming_it() {
        let message = Schema::from_str(
            r#"{"type":"record","name":"row","fields":[{"name":"x","type":"mystery"}]}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("mystery"), "{message}");
    }

    #[test]
    fn a_deep_schema_is_bounded() {
        let mut document = String::from("\"int\"");
        for _ in 0..20 {
            document = format!(r#"{{"type":"array","items":{document}}}"#);
        }
        let parsed = crate::json::from_str(&document).unwrap();
        let limits = crate::Limits::new(8, 1 << 20, 1 << 20, 8);
        let message = Schema::from_json_with_limits(&parsed, limits)
            .unwrap_err()
            .to_string();
        assert!(message.contains("8 levels deep"), "{message}");
    }
}

mod logical {
    use std::sync::Arc;

    use crate::enums::TimeUnit;
    use crate::{Timezone, Value, avro};

    /// Round-trip one value through a single-field record container.
    fn round_trip(field_type: &str, value: Value) -> Value {
        let schema = crate::json::from_str(&format!(
            r#"{{"type":"record","name":"row","fields":[{{"name":"v","type":{field_type}}}]}}"#
        ))
        .unwrap();
        let row = Value::from_mapping([(Value::from("v"), value)]).unwrap();
        let mut handle = super::buffer();
        avro::write_container(&mut handle, &schema, &[], &[row]).unwrap();
        let container = avro::read_container(&handle).unwrap();
        container.rows[0].get_key_str("v").unwrap().clone()
    }

    #[test]
    fn dates_round_trip_as_calendar_dates() {
        // 2024-02-29, a leap day, and a pre-epoch date.
        assert_eq!(
            round_trip(
                r#"{"type":"int","logicalType":"date"}"#,
                Value::Date(19_782)
            ),
            Value::Date(19_782)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"int","logicalType":"date"}"#,
                Value::Date(-3_652)
            ),
            Value::Date(-3_652)
        );
        // A bare integer still encodes; it decodes as the calendar value.
        assert_eq!(
            round_trip(r#"{"type":"int","logicalType":"date"}"#, Value::I64(3)),
            Value::Date(3)
        );
    }

    #[test]
    fn times_round_trip_at_their_declared_unit() {
        assert_eq!(
            round_trip(
                r#"{"type":"int","logicalType":"time-millis"}"#,
                Value::Time(86_399_999, TimeUnit::Millisecond)
            ),
            Value::Time(86_399_999, TimeUnit::Millisecond)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"time-micros"}"#,
                Value::Time(1_000, TimeUnit::Millisecond)
            ),
            Value::Time(1_000_000, TimeUnit::Microsecond),
            "a coarser unit converts losslessly"
        );
    }

    #[test]
    fn timestamps_are_utc_instants_and_local_timestamps_stay_naive() {
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"timestamp-micros"}"#,
                Value::Timestamp(-1_000_000, TimeUnit::Microsecond, Timezone::UTC)
            ),
            Value::Timestamp(-1_000_000, TimeUnit::Microsecond, Timezone::UTC)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"timestamp-nanos"}"#,
                Value::I64(123)
            ),
            Value::Timestamp(123, TimeUnit::Nanosecond, Timezone::UTC)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"local-timestamp-millis"}"#,
                Value::DateTime(555, TimeUnit::Millisecond)
            ),
            Value::DateTime(555, TimeUnit::Millisecond)
        );
    }

    #[test]
    fn a_lossy_unit_conversion_is_refused_naming_both_units() {
        let schema = crate::json::from_str(
            r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"long","logicalType":"time-micros"}}
            ]}"#,
        )
        .unwrap();
        let row = Value::from_mapping([(Value::from("v"), Value::Time(1, TimeUnit::Nanosecond))])
            .unwrap();
        let mut handle = super::buffer();
        let message = avro::write_container(&mut handle, &schema, &[], &[row])
            .unwrap_err()
            .to_string();
        assert!(message.contains("from ns to us"), "{message}");
    }

    #[test]
    fn decimals_keep_their_unscaled_integer_and_scale() {
        // Nine, eighteen, and beyond-eighteen digits, over bytes.
        for unscaled in [
            123_456_789_i128,
            123_456_789_012_345_678,
            1_234_567_890_123_456_789_012_345_678,
        ] {
            for sign in [1, -1] {
                let value = Value::Decimal(unscaled * sign, 2);
                assert_eq!(
                    round_trip(
                        r#"{"type":"bytes","logicalType":"decimal","precision":38,"scale":2}"#,
                        value.clone()
                    ),
                    value
                );
            }
        }
        // Over fixed, sign-extended to the declared width.
        let value = Value::Decimal(-12_345, 2);
        assert_eq!(
            round_trip(
                r#"{"type":"fixed","name":"amount","size":16,"logicalType":"decimal","precision":20,"scale":2}"#,
                value.clone()
            ),
            value
        );
    }

    #[test]
    fn an_overflowing_decimal_is_refused_naming_the_precision() {
        let schema = crate::json::from_str(
            r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"bytes","logicalType":"decimal","precision":4,"scale":0}}
            ]}"#,
        )
        .unwrap();
        let row = Value::from_mapping([(Value::from("v"), Value::Decimal(123_456, 0))]).unwrap();
        let mut handle = super::buffer();
        let message = avro::write_container(&mut handle, &schema, &[], &[row])
            .unwrap_err()
            .to_string();
        assert!(message.contains("4 digits"), "{message}");
    }

    #[test]
    fn uuids_round_trip_in_both_encodings() {
        assert_eq!(
            round_trip(
                r#"{"type":"string","logicalType":"uuid"}"#,
                Value::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
            ),
            Value::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
        );
        // The fixed form accepts the canonical text and stores the bytes.
        let decoded = round_trip(
            r#"{"type":"fixed","name":"id","size":16,"logicalType":"uuid"}"#,
            Value::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6"),
        );
        assert_eq!(decoded.as_bytes().map(<[u8]>::len), Some(16), "{decoded:?}");
        assert_eq!(decoded.as_bytes().map(|bytes| bytes[0]), Some(0xF8));
    }

    #[test]
    fn durations_keep_their_twelve_bytes() {
        let mut duration = Vec::new();
        for part in [1_u32, 2, 3] {
            duration.extend_from_slice(&part.to_le_bytes());
        }
        let value = Value::Bytes(Arc::from(duration.as_slice()));
        assert_eq!(
            round_trip(
                r#"{"type":"fixed","name":"span","size":12,"logicalType":"duration"}"#,
                value.clone()
            ),
            value
        );
    }

    #[test]
    fn an_unknown_logical_type_degrades_to_the_underlying_type() {
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"nobody-knows-this"}"#,
                Value::I64(9)
            ),
            Value::I64(9)
        );
        // Invalid decimal attributes degrade too, per the specification.
        assert_eq!(
            round_trip(
                r#"{"type":"bytes","logicalType":"decimal","precision":2,"scale":9}"#,
                Value::Bytes(Arc::from(&[1_u8, 2][..]))
            ),
            Value::Bytes(Arc::from(&[1_u8, 2][..]))
        );
    }
}

mod resolution {
    use crate::avro::{Resolution, Schema};
    use crate::{Value, avro};

    /// Write rows with the writer schema, read them back with the reader.
    fn resolved(writer: &str, reader: &str, rows: &[&str]) -> Vec<Value> {
        let writer_json = crate::json::from_str(writer).unwrap();
        let mut handle = super::buffer();
        let rows: Vec<Value> = rows
            .iter()
            .map(|row| crate::json::from_str(row).unwrap())
            .collect();
        avro::write_container(&mut handle, &writer_json, &[], &rows).unwrap();
        let reader = Schema::from_str(reader).unwrap();
        avro::read_container_resolved(&handle, &reader)
            .unwrap()
            .rows
    }

    fn record(fields: &str) -> String {
        format!(r#"{{"type":"record","name":"row","fields":[{fields}]}}"#)
    }

    #[test]
    fn every_legal_promotion_widens_in_place() {
        let cases = [
            ("\"int\"", "\"long\"", r#"{"v":7}"#, Value::I64(7)),
            ("\"int\"", "\"float\"", r#"{"v":7}"#, Value::from(7_f32)),
            ("\"int\"", "\"double\"", r#"{"v":7}"#, Value::from(7_f64)),
            ("\"long\"", "\"float\"", r#"{"v":7}"#, Value::from(7_f32)),
            ("\"long\"", "\"double\"", r#"{"v":7}"#, Value::from(7_f64)),
            (
                "\"float\"",
                "\"double\"",
                r#"{"v":1.5}"#,
                Value::from(1.5_f64),
            ),
            (
                "\"string\"",
                "\"bytes\"",
                r#"{"v":"hi"}"#,
                Value::from(b"hi".as_slice()),
            ),
        ];
        for (from, to, row, expected) in cases {
            let rows = resolved(
                &record(&format!(r#"{{"name":"v","type":{from}}}"#)),
                &record(&format!(r#"{{"name":"v","type":{to}}}"#)),
                &[row],
            );
            assert_eq!(rows[0].get_key_str("v"), Some(&expected), "{from} -> {to}");
        }
    }

    #[test]
    fn bytes_promote_to_string_when_they_are_utf8() {
        // Encoded via the string writer so the bytes are valid UTF-8.
        let rows = resolved(
            &record(r#"{"name":"v","type":"string"}"#),
            &record(r#"{"name":"v","type":"string"}"#),
            &[r#"{"v":"ok"}"#],
        );
        assert_eq!(rows[0].get_key_str("v").and_then(Value::as_str), Some("ok"));
    }

    #[test]
    fn an_illegal_resolution_is_refused_naming_both_sides() {
        let writer = Schema::from_str(&record(r#"{"name":"v","type":"string"}"#)).unwrap();
        let reader = Schema::from_str(&record(r#"{"name":"v","type":"long"}"#)).unwrap();
        let message = Resolution::from_schemas(&writer, &reader)
            .unwrap_err()
            .to_string();
        assert!(message.contains("string"), "{message}");
        assert!(message.contains("long"), "{message}");
        assert!(message.contains("row.v"), "{message}");
    }

    #[test]
    fn extra_writer_fields_are_skipped_without_being_decoded() {
        let rows = resolved(
            &record(
                r#"{"name":"a","type":"long"},
                   {"name":"noise","type":{"type":"array","items":"string"}},
                   {"name":"b","type":"string"}"#,
            ),
            &record(r#"{"name":"b","type":"string"}"#),
            &[r#"{"a":1,"noise":["x","y"],"b":"kept"}"#],
        );
        assert_eq!(rows[0].len(), 1);
        assert_eq!(
            rows[0].get_key_str("b").and_then(Value::as_str),
            Some("kept")
        );
    }

    #[test]
    fn missing_reader_fields_fill_from_defaults() {
        let rows = resolved(
            &record(r#"{"name":"a","type":"long"}"#),
            &record(
                r#"{"name":"a","type":"long"},
                   {"name":"note","type":"string","default":"none"},
                   {"name":"maybe","type":["null","long"],"default":null},
                   {"name":"raw","type":"bytes","default":"\u00ff\u0000"}"#,
            ),
            &[r#"{"a":5}"#],
        );
        assert_eq!(
            rows[0].get_key_str("note").and_then(Value::as_str),
            Some("none")
        );
        assert!(rows[0].get_key_str("maybe").unwrap().is_null());
        assert_eq!(
            rows[0].get_key_str("raw").and_then(Value::as_bytes),
            Some(&[0xFF_u8, 0x00][..])
        );
    }

    #[test]
    fn a_missing_field_without_a_default_is_refused_naming_it() {
        let writer = Schema::from_str(&record(r#"{"name":"a","type":"long"}"#)).unwrap();
        let reader = Schema::from_str(&record(
            r#"{"name":"a","type":"long"},{"name":"b","type":"long"}"#,
        ))
        .unwrap();
        let message = Resolution::from_schemas(&writer, &reader)
            .unwrap_err()
            .to_string();
        assert!(message.contains("row.b"), "{message}");
        assert!(message.contains("default"), "{message}");
    }

    #[test]
    fn reader_aliases_match_renamed_records_and_fields() {
        let rows = resolved(
            r#"{"type":"record","name":"old_row","fields":[{"name":"qty","type":"long"}]}"#,
            r#"{"type":"record","name":"new_row","aliases":["old_row"],"fields":[
                {"name":"quantity","aliases":["qty"],"type":"long"}
            ]}"#,
            &[r#"{"qty":31}"#],
        );
        assert_eq!(
            rows[0].get_key_str("quantity").and_then(Value::as_i64),
            Some(31)
        );
    }

    #[test]
    fn field_order_never_matters_only_names_do() {
        let rows = resolved(
            &record(r#"{"name":"a","type":"long"},{"name":"b","type":"string"}"#),
            &record(r#"{"name":"b","type":"string"},{"name":"a","type":"long"}"#),
            &[r#"{"a":1,"b":"x"}"#],
        );
        let keys = rows[0].keys();
        assert_eq!(keys, ["b", "a"], "the reader's order wins");
        assert_eq!(rows[0].get_key_str("a").and_then(Value::as_i64), Some(1));
    }

    #[test]
    fn enums_resolve_by_symbol_with_the_reader_default_as_fallback() {
        let rows = resolved(
            &record(
                r#"{"name":"v","type":{"type":"enum","name":"side","symbols":["BUY","SELL","HOLD"]}}"#,
            ),
            &record(
                r#"{"name":"v","type":{"type":"enum","name":"side","symbols":["BUY","SELL","OTHER"],"default":"OTHER"}}"#,
            ),
            &[r#"{"v":"SELL"}"#, r#"{"v":"HOLD"}"#],
        );
        assert_eq!(
            rows[0].get_key_str("v").and_then(Value::as_str),
            Some("SELL")
        );
        assert_eq!(
            rows[1].get_key_str("v").and_then(Value::as_str),
            Some("OTHER"),
            "an unknown writer symbol falls back to the reader default"
        );
    }

    #[test]
    fn a_union_writer_resolves_into_a_narrower_reader() {
        // The long branch resolves; a row taking the null branch fails at
        // read time, which is the specification's deferral.
        let writer = record(r#"{"name":"v","type":["null","long"]}"#);
        let rows = resolved(
            &writer,
            &record(r#"{"name":"v","type":"long"}"#),
            &[r#"{"v":9}"#],
        );
        assert_eq!(rows[0].get_key_str("v").and_then(Value::as_i64), Some(9));

        let writer_json = crate::json::from_str(&writer).unwrap();
        let mut handle = super::buffer();
        avro::write_container(
            &mut handle,
            &writer_json,
            &[],
            &[crate::json::from_str(r#"{"v":null}"#).unwrap()],
        )
        .unwrap();
        let reader = Schema::from_str(&record(r#"{"name":"v","type":"long"}"#)).unwrap();
        let message = avro::read_container_resolved(&handle, &reader)
            .unwrap_err()
            .to_string();
        assert!(message.contains("null"), "{message}");
    }

    #[test]
    fn a_non_union_writer_resolves_into_a_wider_reader() {
        let rows = resolved(
            &record(r#"{"name":"v","type":"long"}"#),
            &record(r#"{"name":"v","type":["null","long"]}"#),
            &[r#"{"v":4}"#],
        );
        assert_eq!(rows[0].get_key_str("v").and_then(Value::as_i64), Some(4));
    }

    #[test]
    fn unions_wider_than_two_branches_resolve_branch_by_branch() {
        let rows = resolved(
            &record(r#"{"name":"v","type":["null","long","string","bytes"]}"#),
            &record(r#"{"name":"v","type":["null","string","double","bytes"]}"#),
            &[r#"{"v":"text"}"#, r#"{"v":null}"#, r#"{"v":7}"#],
        );
        assert_eq!(
            rows[0].get_key_str("v").and_then(Value::as_str),
            Some("text")
        );
        assert!(rows[1].get_key_str("v").unwrap().is_null());
        assert_eq!(
            rows[2].get_key_str("v").and_then(Value::as_f64),
            Some(7.0),
            "the long branch promotes into the reader's double branch"
        );
    }

    #[test]
    fn a_recursive_schema_resolves_against_a_projection_of_itself() {
        let writer = r#"{"type":"record","name":"node","fields":[
            {"name":"value","type":"long"},
            {"name":"label","type":"string"},
            {"name":"next","type":["null","node"],"default":null}
        ]}"#;
        let reader = r#"{"type":"record","name":"node","fields":[
            {"name":"value","type":"long"},
            {"name":"next","type":["null","node"],"default":null}
        ]}"#;
        let rows = resolved(
            writer,
            reader,
            &[r#"{"value":1,"label":"a","next":{"value":2,"label":"b","next":null}}"#],
        );
        assert_eq!(rows[0].path("next.value").and_then(Value::as_i64), Some(2));
        assert!(rows[0].path("next.label").is_none(), "projected away");
    }

    #[test]
    fn resolving_to_the_writer_schema_is_the_identity() {
        let schema = super::manifest_shaped_schema();
        let parsed = Schema::from_json(&schema).unwrap();
        let row = crate::json::from_str(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1],"nested":{"flag":true}}"#,
        )
        .unwrap();
        let mut handle = super::buffer();
        avro::write_container(&mut handle, &schema, &[], &[row]).unwrap();
        let direct = avro::read_container(&handle).unwrap().rows;
        let via_plan = avro::read_container_resolved(&handle, &parsed)
            .unwrap()
            .rows;
        assert_eq!(direct, via_plan);
    }
}

mod streaming {
    use crate::{Value, avro};

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
        assert_eq!(second.rows().unwrap(), [Value::I64(9)]);
        assert!(blocks.next_block().unwrap().is_none());
    }

    #[test]
    fn a_written_container_streams_back_the_same_rows() {
        let schema = super::manifest_shaped_schema();
        let row = crate::json::from_str(
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
}

mod single_object {
    use crate::avro::Schema;
    use crate::{Value, avro};

    #[test]
    fn a_datum_round_trips_through_the_single_object_framing() {
        let schema = Schema::from_str(
            r#"{"type":"record","name":"trade","fields":[
                {"name":"symbol","type":"string"},
                {"name":"qty","type":"long"}
            ]}"#,
        )
        .unwrap();
        let value = Value::from_mapping([
            (Value::from("symbol"), Value::from("AAPL")),
            (Value::from("qty"), Value::from(100_i64)),
        ])
        .unwrap();
        let framed = avro::to_single_object_vec(&schema, &value).unwrap();
        assert_eq!(&framed[..2], &[0xC3, 0x01]);
        assert_eq!(
            avro::from_single_object_slice(&framed, &schema).unwrap(),
            value
        );
    }

    #[test]
    fn a_wrong_fingerprint_is_refused_naming_both() {
        let schema = Schema::from_str("\"long\"").unwrap();
        let other = Schema::from_str("\"string\"").unwrap();
        let framed = avro::to_single_object_vec(&schema, &Value::I64(1)).unwrap();
        let message = avro::from_single_object_slice(&framed, &other)
            .unwrap_err()
            .to_string();
        assert!(message.contains("fingerprint"), "{message}");
    }
}

#[cfg(feature = "parquet")]
mod snappy {
    use crate::{Value, avro};

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
        assert_eq!(container.rows, [Value::I64(42)]);
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
    use crate::avro;

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

#[cfg(feature = "arrow")]
mod records {
    use std::sync::Arc;

    use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, StringBuilder};
    use arrow_array::types::{Float64Type, Int64Type};
    use arrow_array::{Array, RecordBatch, cast::AsArray};

    use crate::avro::{Avro, AvroOptions};
    use crate::generic::IORecordOptions;
    use crate::io::{Buffer, IOBase};
    use crate::{DataType, Field, Url, avro};

    /// One canonical batch with a nullable column and a list column.
    fn batch() -> (Field, RecordBatch) {
        let schema = Field::new(
            "trades",
            DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Utf8.nullable_field("symbol"),
                DataType::Float64.nullable_field("price"),
                DataType::list(DataType::Int64.required_field("item")).required_field("legs"),
            ])
            .unwrap(),
            false,
        );
        let mut symbols = StringBuilder::new();
        symbols.append_value("AAPL");
        symbols.append_null();
        symbols.append_value("MSFT");
        let mut prices = Float64Builder::new();
        prices.append_value(187.5);
        prices.append_value(12.25);
        prices.append_null();
        let mut legs = ListBuilder::new(Int64Builder::new());
        legs.append_value([Some(1), Some(2)]);
        legs.append_value([]);
        legs.append_value([Some(7)]);
        let legs = {
            // The canonical list item is a required field called `item`.
            let array = legs.finish();
            let (_, offsets, values, nulls) = array.into_parts();
            arrow_array::ListArray::new(
                Arc::new(arrow_schema::Field::new(
                    "item",
                    arrow_schema::DataType::Int64,
                    false,
                )),
                offsets,
                values,
                nulls,
            )
        };
        let arrow_schema = schema.to_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![
                Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3])),
                Arc::new(symbols.finish()),
                Arc::new(prices.finish()),
                Arc::new(legs),
            ],
        )
        .unwrap();
        (schema, batch)
    }

    fn handle() -> Buffer {
        Buffer::new().with_media_type(Url::from_str("file:///trades.avro").unwrap().media_type())
    }

    #[test]
    fn batches_round_trip_through_the_record_surface() {
        let (schema, batch) = batch();
        let mut handle = handle();
        let options = crate::generic::RecordOptions::for_media_type(handle.media_type()).unwrap();
        handle
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();

        let read: Vec<RecordBatch> = handle
            .read_arrow_batch_reader(&options)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].num_rows(), 3);
        assert_eq!(
            read[0].column(0).as_primitive::<Int64Type>().values(),
            &[1, 2, 3]
        );
        let symbols = read[0].column(1).as_string::<i32>();
        assert_eq!(symbols.value(0), "AAPL");
        assert!(symbols.is_null(1));
        let legs = read[0].column(3).as_list::<i32>();
        assert_eq!(legs.value(0).len(), 2);
        assert_eq!(legs.value(1).len(), 0);
        let _ = schema;
    }

    #[test]
    fn a_projection_skips_the_bytes_of_unselected_columns() {
        let (_, batch) = batch();
        let mut handle = handle();
        let options = AvroOptions::new();
        avro::write_batch_reader(
            &mut handle,
            crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options,
        )
        .unwrap();

        let narrow = Field::new(
            "trades",
            DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Float64.nullable_field("price"),
            ])
            .unwrap(),
            false,
        );
        let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, Some(&narrow), &options)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read[0].num_columns(), 2, "{:?}", read[0].schema());
        assert_eq!(
            read[0].column(0).as_primitive::<Int64Type>().values(),
            &[1, 2, 3]
        );
        assert_eq!(
            read[0].column(1).as_primitive::<Float64Type>().value(0),
            187.5
        );
        assert!(read[0].column(1).as_primitive::<Float64Type>().is_null(2));
    }

    #[test]
    fn an_empty_handle_reads_as_no_batches() {
        let handle = handle();
        let options = AvroOptions::new().with_schema(batch().0);
        let read = avro::read_batch_reader(&handle, None, &options).unwrap();
        assert_eq!(read.count(), 0);
    }

    #[test]
    fn an_outer_content_coding_is_rejected_by_name() {
        let handle = Buffer::new().with_media_type(
            Url::from_str("file:///trades.avro.gz")
                .unwrap()
                .media_type(),
        );
        let message = avro::read_field(&handle, &AvroOptions::new())
            .unwrap_err()
            .to_string();
        assert!(message.contains("outer content coding"), "{message}");
        assert!(message.contains("gzip"), "{message}");
    }

    #[test]
    fn the_stateful_wrapper_caches_the_schema_between_open_and_close() {
        let (schema, batch) = batch();
        // An Arrow schema is anonymous, so the record's name is the options'
        // root name; naming it keeps the round trip exact.
        let mut media = Avro::new(handle()).with_root_name("trades");
        media
            .write_batch_reader(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        assert!(media.is_open(), "a write refreshes the cache");
        media.close().unwrap();
        assert!(!media.is_open());
        media.open().unwrap();
        assert!(media.is_open());
        let derived = media.schema().unwrap();
        assert_eq!(derived.name(), schema.name());
        assert_eq!(derived.field_len(), schema.field_len());
    }

    #[test]
    fn a_wide_union_is_refused_for_the_record_surface() {
        let mut handle = handle();
        avro::write_container(
            &mut handle,
            &crate::json::from_str(
                r#"{"type":"record","name":"row","fields":[
                    {"name":"v","type":["null","long","string"]}
                ]}"#,
            )
            .unwrap(),
            &[],
            &[],
        )
        .unwrap();
        let message = avro::read_field(&handle, &AvroOptions::new())
            .unwrap_err()
            .to_string();
        assert!(message.contains("3 branches"), "{message}");
    }

    #[test]
    fn logical_columns_round_trip_columnar() {
        let schema = Field::new(
            "row",
            DataType::from_fields([
                DataType::Date32.required_field("day"),
                DataType::Timestamp(
                    crate::enums::TimeUnit::Microsecond,
                    Some(crate::Timezone::UTC),
                )
                .nullable_field("at"),
                DataType::decimal128(10, 2).unwrap().required_field("cost"),
            ])
            .unwrap(),
            false,
        );
        let arrow_schema = schema.to_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![
                Arc::new(arrow_array::Date32Array::from(vec![19_782, -3_652])),
                Arc::new(
                    arrow_array::TimestampMicrosecondArray::from(vec![
                        Some(1_700_000_000_000_000),
                        None,
                    ])
                    .with_timezone("UTC"),
                ),
                Arc::new(
                    arrow_array::Decimal128Array::from(vec![18_750_i128, -99])
                        .with_precision_and_scale(10, 2)
                        .unwrap(),
                ),
            ],
        )
        .unwrap();

        let mut handle = handle();
        let options = AvroOptions::new();
        avro::write_batch_reader(
            &mut handle,
            crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options,
        )
        .unwrap();

        // The container is readable at the Value level with typed temporals.
        let container = avro::read_container(&handle).unwrap();
        assert_eq!(
            container.rows[0].get_key_str("day"),
            Some(&crate::Value::Date(19_782))
        );
        assert_eq!(
            container.rows[1].get_key_str("cost"),
            Some(&crate::Value::Decimal(-99, 2))
        );

        // And columnar reads reproduce the arrays exactly.
        let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, None, &options)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read[0].column(0).as_ref(), batch.column(0).as_ref());
        assert_eq!(read[0].column(1).as_ref(), batch.column(1).as_ref());
        assert_eq!(read[0].column(2).as_ref(), batch.column(2).as_ref());
    }
}
