//! Unit tests for the Avro codec module.

use crate::IOBase;
use crate::holder::Buffer;
use crate::{MediaType, MimeType, Scalar};

/// A record schema exercising every branch the manifests use.
fn manifest_shaped_schema() -> Scalar {
    crate::text::json::from_utf8(
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
fn handmade_container_with_header(entries: &[(&str, &[u8])], blocks: &[(i64, Vec<u8>)]) -> Buffer {
    let mut output = Vec::new();
    output.extend_from_slice(b"Obj\x01");
    super::datum::put_long(&mut output, entries.len() as i64);
    for (key, value) in entries {
        super::datum::put_bytes(&mut output, key.as_bytes());
        super::datum::put_bytes(&mut output, value);
    }
    super::datum::put_long(&mut output, 0);
    let sync = [7_u8; 16];
    output.extend_from_slice(&sync);
    for (count, payload) in blocks {
        super::datum::put_long(&mut output, *count);
        super::datum::put_bytes(&mut output, payload);
        output.extend_from_slice(&sync);
    }
    let mut handle = buffer();
    handle.write_all_bytes(&output).unwrap();
    handle
}

mod containers {
    use super::{buffer, manifest_shaped_schema};
    use crate::IOBase;
    use crate::Scalar;
    use crate::media::avro;

    #[test]
    fn a_container_round_trips_every_encoded_branch() {
        let schema = manifest_shaped_schema();
        let row = crate::text::json::from_utf8(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,300000],
                "nested":{"flag":true}}"#,
        )
        .unwrap();
        let empty = crate::text::json::from_utf8(
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
            &[crate::text::json::from_utf8(
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
            &crate::text::json::from_utf8(
                r#"{"type":"record","name":"r","fields":[{"name":"v","type":"long"}]}"#,
            )
            .unwrap(),
            &[],
            &[crate::text::json::from_utf8(r#"{"v":1}"#).unwrap()],
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
        let limits = crate::Limits::new(64, 16, 1_000, 8);
        let message = avro::read_container_with_limits(&handle, limits)
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 16 bytes"), "{message}");
    }

    #[test]
    fn a_row_budget_is_applied_after_the_mandatory_header() {
        let schema = crate::text::json::from_utf8(r#""long""#).unwrap();
        let rows = [Scalar::I64(1), Scalar::I64(2)];
        let mut handle = buffer();
        avro::write_container(&mut handle, &schema, &[], &rows).unwrap();

        let limits = crate::Limits::new(128, 1 << 20, 1, 1_024);
        let message = avro::read_container_with_limits(&handle, limits)
            .unwrap_err()
            .to_string();
        assert!(message.contains("at most 1 rows"), "{message}");
    }
}

mod schemas {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use crate::media::avro::{Container, Schema};

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
        assert_eq!(int.clone().into_canonical_form(), "\"int\"");
        assert_eq!(hex(&int), "8f5c393f1ad57572");

        let record = Schema::from_str(
            r#"{"type":"record","name":"trade","doc":"ignored","fields":[
                {"name":"symbol","type":"string","doc":"also ignored"},
                {"name":"qty","type":"long"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            record.clone().into_canonical_form(),
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
    fn schema_value_traits_keep_complete_behavior_not_only_canonical_form() {
        let date = Schema::from_str(r#"{"type":"int","logicalType":"date"}"#).unwrap();
        let integer = Schema::from_str(r#"{"type":"int"}"#).unwrap();

        // Parsing Canonical Form intentionally erases logical annotations,
        // while Schema identity cannot: these decode to different Scalars.
        assert_eq!(
            date.clone().into_canonical_form(),
            integer.clone().into_canonical_form()
        );
        assert_eq!(date.fingerprint(), integer.fingerprint());
        assert_ne!(date, integer);
        assert_ne!(date.stable_hash(), integer.stable_hash());
        assert_ne!(date.cmp(&integer), std::cmp::Ordering::Equal);

        let reordered = Schema::from_str(r#"{"logicalType":"date","type":"int"}"#).unwrap();
        assert_eq!(date, reordered);
        assert_eq!(date.stable_hash(), reordered.stable_hash());

        let native_mapping = crate::Scalar::from_mapping([
            (
                crate::Scalar::from("logicalType"),
                crate::Scalar::from("date"),
            ),
            (crate::Scalar::from("type"), crate::Scalar::from("int")),
        ])
        .unwrap();
        let native = Schema::from_json(&native_mapping).unwrap();
        assert_eq!(date, native);
        assert_eq!(date.stable_hash(), native.stable_hash());

        let reparsed =
            Schema::from_str(&crate::text::json::into_utf8(&native.clone().into_json()).unwrap())
                .unwrap();
        assert_eq!(native, reparsed);

        let nested_mapping = crate::Scalar::from_mapping([
            (crate::Scalar::from("type"), crate::Scalar::from("record")),
            (crate::Scalar::from("name"), crate::Scalar::from("row")),
            (
                crate::Scalar::from("fields"),
                crate::Scalar::from_sequence([crate::Scalar::from_mapping([
                    (crate::Scalar::from("name"), crate::Scalar::from("items")),
                    (
                        crate::Scalar::from("type"),
                        crate::Scalar::from_mapping([
                            (crate::Scalar::from("type"), crate::Scalar::from("array")),
                            (
                                crate::Scalar::from("items"),
                                crate::Scalar::from_mapping([(
                                    crate::Scalar::from("type"),
                                    crate::Scalar::from("int"),
                                )])
                                .unwrap(),
                            ),
                        ])
                        .unwrap(),
                    ),
                ])
                .unwrap()]),
            ),
        ])
        .unwrap();
        let nested = Schema::from_json(&nested_mapping).unwrap();
        let nested_parsed = Schema::from_str(
            r#"{"type":"record","name":"row","fields":[{"name":"items","type":{"type":"array","items":{"type":"int"}}}]}"#,
        )
        .unwrap();
        assert_eq!(nested, nested_parsed);
        assert_eq!(nested.clone().into_json(), nested_mapping);

        let native_hash = |schema: &Schema| {
            let mut hasher = DefaultHasher::new();
            schema.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(native_hash(&date), native_hash(&reordered));
    }

    #[test]
    fn decoded_containers_have_complete_structural_value_identity() {
        fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
        assert_traits::<Container>();

        let container = Container {
            schema: Schema::from_str(r#"{"type":"long"}"#).unwrap(),
            metadata: vec![
                ("source".into(), "test".into()),
                ("zone".into(), "UTC".into()),
            ],
            rows: vec![crate::Scalar::I16(7)],
        };
        let mut equal = container.clone();
        equal.metadata.reverse();
        let mut later = container.clone();
        later.rows.push(crate::Scalar::I16(8));
        assert_eq!(container, equal);
        assert_eq!(container.stable_hash(), equal.stable_hash());
        assert!(container < later);

        // Public construction can still describe an invalid duplicate-key
        // header. Its first-value lookup remains part of exact identity even
        // though valid parsed headers reject this shape.
        let mut duplicate = container.clone();
        duplicate.metadata = vec![
            ("source".into(), "first".into()),
            ("source".into(), "last".into()),
        ];
        let mut reversed_duplicate = duplicate.clone();
        reversed_duplicate.metadata.reverse();
        assert_ne!(duplicate, reversed_duplicate);
        assert_ne!(duplicate.get("source"), reversed_duplicate.get("source"));
    }

    #[test]
    fn the_source_json_round_trips_verbatim() {
        let document = crate::text::json::from_utf8(
            r#"{"type":"record","name":"row","fields":[{"name":"id","type":"int","field-id":42}]}"#,
        )
        .unwrap();
        let schema = Schema::from_json(&document).unwrap();
        assert_eq!(schema.clone().into_json(), document);
        // The unmodeled attribute is still in the JSON the schema writes.
        let text =
            String::from_utf8(crate::text::json::into_bytes(&schema.into_json()).unwrap()).unwrap();
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
        assert!(schema.into_canonical_form().contains("com.example.inner"));
    }

    #[test]
    fn a_dotted_name_is_a_fullname_and_ignores_the_namespace_attribute() {
        let schema = Schema::from_str(
            r#"{"type":"fixed","name":"org.other.hash","namespace":"ignored","size":4}"#,
        )
        .unwrap();
        assert_eq!(
            schema.into_canonical_form(),
            r#"{"name":"org.other.hash","type":"fixed","size":4}"#
        );
    }

    #[test]
    fn a_recursive_schema_parses_and_round_trips_data() {
        let schema_json = crate::text::json::from_utf8(
            r#"{"type":"record","name":"node","fields":[
                {"name":"value","type":"long"},
                {"name":"next","type":["null","node"],"default":null}
            ]}"#,
        )
        .unwrap();
        let list = crate::text::json::from_utf8(
            r#"{"value":1,"next":{"value":2,"next":{"value":3,"next":null}}}"#,
        )
        .unwrap();
        let mut handle = super::buffer();
        crate::media::avro::write_container(&mut handle, &schema_json, &[], &[list]).unwrap();
        let container = crate::media::avro::read_container(&handle).unwrap();
        let tail = container.rows[0]
            .path("next.next.value")
            .and_then(crate::Scalar::as_i64);
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
        let parsed = crate::text::json::from_utf8(&document).unwrap();
        let limits = crate::Limits::new(8, 1 << 20, 1 << 20, 8);
        let message = Schema::from_json_with_limits(&parsed, limits)
            .unwrap_err()
            .to_string();
        assert!(message.contains("8 levels deep"), "{message}");
    }

    #[test]
    fn a_second_definition_of_a_named_type_is_refused_naming_it() {
        // One definition per fullname; the second body could disagree with
        // the first and silently shadow it in the name table.
        let bodies = [
            r#"{"type":"record","name":"dup","fields":[{"name":"x","type":"long"}]}"#,
            r#"{"type":"enum","name":"dup","symbols":["A"]}"#,
            r#"{"type":"fixed","name":"dup","size":4}"#,
        ];
        for body in bodies {
            let message = Schema::from_str(&format!(
                r#"{{"type":"record","name":"row","fields":[
                    {{"name":"p","type":{body}}},
                    {{"name":"q","type":{body}}}
                ]}}"#
            ))
            .unwrap_err()
            .to_string();
            assert!(message.contains("one definition"), "{message}");
            assert!(message.contains("dup"), "{message}");
        }
    }

    #[test]
    fn a_reference_to_an_earlier_definition_is_not_a_redefinition() {
        let schema = Schema::from_str(
            r#"{"type":"record","name":"row","fields":[
                {"name":"p","type":{"type":"record","name":"kv","fields":[
                    {"name":"x","type":"long"}]}},
                {"name":"q","type":"kv"}
            ]}"#,
        )
        .unwrap();
        assert!(schema.names.contains_key("kv"));
    }
}

mod logical {
    use std::sync::Arc;

    use crate::TimeUnit;
    use crate::media::avro;
    use crate::{Scalar, Timezone};

    /// Round-trip one value through a single-field record container.
    fn round_trip(field_type: &str, value: Scalar) -> Scalar {
        let schema = crate::text::json::from_utf8(&format!(
            r#"{{"type":"record","name":"row","fields":[{{"name":"v","type":{field_type}}}]}}"#
        ))
        .unwrap();
        let row = Scalar::from_mapping([(Scalar::from("v"), value)]).unwrap();
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
                Scalar::date32(19_782)
            ),
            Scalar::date32(19_782)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"int","logicalType":"date"}"#,
                Scalar::date32(-3_652)
            ),
            Scalar::date32(-3_652)
        );
        // A bare integer still encodes; it decodes as the calendar value.
        assert_eq!(
            round_trip(r#"{"type":"int","logicalType":"date"}"#, Scalar::I64(3)),
            Scalar::date32(3)
        );
    }

    #[test]
    fn times_round_trip_at_their_declared_unit() {
        assert_eq!(
            round_trip(
                r#"{"type":"int","logicalType":"time-millis"}"#,
                Scalar::Time32(86_399_999, TimeUnit::Millisecond, Timezone::NAIVE)
            ),
            Scalar::Time32(86_399_999, TimeUnit::Millisecond, Timezone::NAIVE)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"time-micros"}"#,
                Scalar::Time32(1_000, TimeUnit::Millisecond, Timezone::NAIVE)
            ),
            Scalar::Time64(1_000_000, TimeUnit::Microsecond, Timezone::NAIVE),
            "a coarser unit converts losslessly"
        );
    }

    #[test]
    fn timestamps_are_utc_instants_and_local_timestamps_stay_naive() {
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"timestamp-micros"}"#,
                Scalar::DateTime64(-1_000_000, TimeUnit::Microsecond, Timezone::UTC)
            ),
            Scalar::DateTime64(-1_000_000, TimeUnit::Microsecond, Timezone::UTC)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"timestamp-nanos"}"#,
                Scalar::I64(123)
            ),
            Scalar::DateTime64(123, TimeUnit::Nanosecond, Timezone::UTC)
        );
        assert_eq!(
            round_trip(
                r#"{"type":"long","logicalType":"local-timestamp-millis"}"#,
                Scalar::DateTime64(555, TimeUnit::Millisecond, Timezone::NAIVE)
            ),
            Scalar::DateTime64(555, TimeUnit::Millisecond, Timezone::NAIVE)
        );
    }

    #[test]
    fn a_lossy_unit_conversion_is_refused_naming_both_units() {
        let schema = crate::text::json::from_utf8(
            r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"long","logicalType":"time-micros"}}
            ]}"#,
        )
        .unwrap();
        let row = Scalar::from_mapping([(
            Scalar::from("v"),
            Scalar::Time64(1, TimeUnit::Nanosecond, Timezone::NAIVE),
        )])
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
                let value = Scalar::d128(unscaled * sign, 2);
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
        let value = Scalar::d128(-12_345, 2);
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
        let schema = crate::text::json::from_utf8(
            r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"bytes","logicalType":"decimal","precision":4,"scale":0}}
            ]}"#,
        )
        .unwrap();
        let row = Scalar::from_mapping([(Scalar::from("v"), Scalar::d128(123_456, 0))]).unwrap();
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
                Scalar::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
            ),
            Scalar::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
        );
        // The fixed form accepts the canonical text and stores the bytes.
        let decoded = round_trip(
            r#"{"type":"fixed","name":"id","size":16,"logicalType":"uuid"}"#,
            Scalar::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6"),
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
        let value = Scalar::Bytes(Arc::from(duration.as_slice()));
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
                Scalar::I64(9)
            ),
            Scalar::I64(9)
        );
        // Invalid decimal attributes degrade too, per the specification.
        assert_eq!(
            round_trip(
                r#"{"type":"bytes","logicalType":"decimal","precision":2,"scale":9}"#,
                Scalar::Bytes(Arc::from(&[1_u8, 2][..]))
            ),
            Scalar::Bytes(Arc::from(&[1_u8, 2][..]))
        );
    }
}

mod resolution {
    use crate::Scalar;
    use crate::media::avro;
    use crate::media::avro::{Resolution, Schema};

    /// Write rows with the writer schema, read them back with the reader.
    fn resolved(writer: &str, reader: &str, rows: &[&str]) -> Vec<Scalar> {
        let writer_json = crate::text::json::from_utf8(writer).unwrap();
        let mut handle = super::buffer();
        let rows: Vec<Scalar> = rows
            .iter()
            .map(|row| crate::text::json::from_utf8(row).unwrap())
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
            ("\"int\"", "\"long\"", r#"{"v":7}"#, Scalar::I64(7)),
            ("\"int\"", "\"float\"", r#"{"v":7}"#, Scalar::from(7_f32)),
            ("\"int\"", "\"double\"", r#"{"v":7}"#, Scalar::from(7_f64)),
            ("\"long\"", "\"float\"", r#"{"v":7}"#, Scalar::from(7_f32)),
            ("\"long\"", "\"double\"", r#"{"v":7}"#, Scalar::from(7_f64)),
            (
                "\"float\"",
                "\"double\"",
                r#"{"v":1.5}"#,
                Scalar::from(1.5_f64),
            ),
            (
                "\"string\"",
                "\"bytes\"",
                r#"{"v":"hi"}"#,
                Scalar::from(b"hi".as_slice()),
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
        assert_eq!(
            rows[0].get_key_str("v").and_then(Scalar::as_str),
            Some("ok")
        );
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
            rows[0].get_key_str("b").and_then(Scalar::as_str),
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
            rows[0].get_key_str("note").and_then(Scalar::as_str),
            Some("none")
        );
        assert!(rows[0].get_key_str("maybe").unwrap().is_null());
        assert_eq!(
            rows[0].get_key_str("raw").and_then(Scalar::as_bytes),
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
            rows[0].get_key_str("quantity").and_then(Scalar::as_i64),
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
        assert_eq!(keys, ["a", "b"], "records are name-sorted values");
        assert_eq!(rows[0].get_key_str("a").and_then(Scalar::as_i64), Some(1));
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
            rows[0].get_key_str("v").and_then(Scalar::as_str),
            Some("SELL")
        );
        assert_eq!(
            rows[1].get_key_str("v").and_then(Scalar::as_str),
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
        assert_eq!(rows[0].get_key_str("v").and_then(Scalar::as_i64), Some(9));

        let writer_json = crate::text::json::from_utf8(&writer).unwrap();
        let mut handle = super::buffer();
        avro::write_container(
            &mut handle,
            &writer_json,
            &[],
            &[crate::text::json::from_utf8(r#"{"v":null}"#).unwrap()],
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
        assert_eq!(rows[0].get_key_str("v").and_then(Scalar::as_i64), Some(4));
    }

    #[test]
    fn unions_wider_than_two_branches_resolve_branch_by_branch() {
        let rows = resolved(
            &record(r#"{"name":"v","type":["null","long","string","bytes"]}"#),
            &record(r#"{"name":"v","type":["null","string","double","bytes"]}"#),
            &[r#"{"v":"text"}"#, r#"{"v":null}"#, r#"{"v":7}"#],
        );
        assert_eq!(
            rows[0].get_key_str("v").and_then(Scalar::as_str),
            Some("text")
        );
        assert!(rows[1].get_key_str("v").unwrap().is_null());
        assert_eq!(
            rows[2].get_key_str("v").and_then(Scalar::as_f64),
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
        assert_eq!(rows[0].path("next.value").and_then(Scalar::as_i64), Some(2));
        assert!(rows[0].path("next.label").is_none(), "projected away");
    }

    #[test]
    fn resolving_to_the_writer_schema_is_the_identity() {
        let schema = super::manifest_shaped_schema();
        let parsed = Schema::from_json(&schema).unwrap();
        let row = crate::text::json::from_utf8(
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

    #[test]
    fn a_failed_writer_union_branch_does_not_poison_later_plans() {
        // Resolving f1 tries writer "r" against reader "r" and fails on the
        // field types; that attempt registers a partial record plan for the
        // ("r", "r") pair. Resolving f2 then needs the same pair for real -
        // a plan registry that kept the poisoned entry would refuse a legal
        // decode.
        let writer = r#"{"type":"record","name":"top","fields":[
            {"name":"f1","type":[
                {"type":"record","name":"a","fields":[{"name":"x","type":"long"}]},
                {"type":"record","name":"r","fields":[{"name":"x","type":"string"}]}
            ]},
            {"name":"f2","type":"r"}
        ]}"#;
        let reader = r#"{"type":"record","name":"top","fields":[
            {"name":"f1","type":{"type":"record","name":"r","aliases":["a"],"fields":[
                {"name":"x","type":"long"}]}},
            {"name":"f2","type":[
                "r",
                {"type":"record","name":"rr","aliases":["r"],"fields":[
                    {"name":"x","type":"string"}]}
            ]}
        ]}"#;
        let rows = resolved(writer, reader, &[r#"{"f1":{"x":7},"f2":{"x":"hi"}}"#]);
        assert_eq!(rows[0].path("f1.x").and_then(Scalar::as_i64), Some(7));
        assert_eq!(rows[0].path("f2.x").and_then(Scalar::as_str), Some("hi"));
    }

    #[test]
    fn a_reader_union_prefers_the_branch_naming_the_writer_exactly() {
        // Both reader branches can hold the row; the branch whose fullname
        // is the writer's must win over one that merely shares a bare name
        // across namespaces, whatever the union order says.
        let rows = resolved(
            r#"{"type":"record","name":"r","fields":[{"name":"v","type":"long"}]}"#,
            r#"[
                {"type":"record","name":"ns1.r","fields":[
                    {"name":"v","type":"long"},
                    {"name":"via","type":"string","default":"lenient"}]},
                {"type":"record","name":"r","fields":[
                    {"name":"v","type":"long"},
                    {"name":"via","type":"string","default":"exact"}]}
            ]"#,
            &[r#"{"v":3}"#],
        );
        assert_eq!(
            rows[0].get_key_str("via").and_then(Scalar::as_str),
            Some("exact")
        );
    }

    #[test]
    fn a_reader_field_matches_a_writer_field_by_name_before_alias() {
        let rows = resolved(
            &record(r#"{"name":"a","type":"long"},{"name":"b","type":"long"}"#),
            &record(r#"{"name":"b","aliases":["a"],"type":"long"}"#),
            &[r#"{"a":1,"b":2}"#],
        );
        assert_eq!(rows[0].get_key_str("b").and_then(Scalar::as_i64), Some(2));
    }

    #[test]
    fn a_fixed_default_of_the_wrong_length_is_refused() {
        let writer = Schema::from_str(&record(r#"{"name":"a","type":"long"}"#)).unwrap();
        let reader = Schema::from_str(&record(
            r#"{"name":"a","type":"long"},
               {"name":"pad","type":{"type":"fixed","name":"four","size":4},
                "default":"ab"}"#,
        ))
        .unwrap();
        let message = Resolution::from_schemas(&writer, &reader)
            .unwrap_err()
            .to_string();
        assert!(message.contains("4 bytes, got 2"), "{message}");
    }

    #[test]
    fn a_default_nested_past_the_default_depth_bound_is_refused() {
        // A recursive type lets a default nest as deep as the schema
        // document allows, which is deeper than the schema's own structure
        // may go; the default walk carries its own bound.
        let deep = format!("{}null{}", r#"{"next":"#.repeat(100), "}".repeat(100));
        let reader = Schema::from_str(&format!(
            r#"{{"type":"record","name":"row","fields":[
                {{"name":"a","type":"long"}},
                {{"name":"chain","type":{{"type":"record","name":"n","fields":[
                    {{"name":"next","type":["n","null"]}}
                ]}},"default":{deep}}}
            ]}}"#
        ))
        .unwrap();
        let writer = Schema::from_str(&record(r#"{"name":"a","type":"long"}"#)).unwrap();
        let message = Resolution::from_schemas(&writer, &reader)
            .unwrap_err()
            .to_string();
        assert!(message.contains("64 levels deep"), "{message}");
    }
}

mod streaming {
    use crate::Scalar;
    use crate::media::avro;

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
        assert_eq!(second.rows().unwrap(), [Scalar::I64(9)]);
        assert!(blocks.next_block().unwrap().is_none());
    }

    #[test]
    fn a_written_container_streams_back_the_same_rows() {
        let schema = super::manifest_shaped_schema();
        let row = crate::text::json::from_utf8(
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
        assert_eq!(second.rows().unwrap(), [Scalar::I64(9)]);
        assert!(blocks.next_block().unwrap().is_none());

        let source = super::handmade_container("\"long\"", "null", &[]);
        let limits = crate::Limits::new(128, 1, 1_000_000, 1_024);
        let message = avro::read_blocks_owned_with_limits(source, limits)
            .err()
            .expect("the one-byte schema limit must fail")
            .to_string();
        assert!(message.contains("at most 1 bytes"), "{message}");
    }

    #[test]
    fn a_lazy_block_applies_its_row_budget_after_opening_the_header() {
        let source = super::handmade_container("\"null\"", "null", &[(3, Vec::new())]);
        let limits = crate::Limits::new(128, 1 << 20, 2, 1_024);
        let mut blocks = avro::read_blocks_with_limits(&source, limits).unwrap();

        let message = blocks.next_block().unwrap_err().to_string();
        assert!(message.contains("at most 2 rows"), "{message}");
    }
}

mod single_object {
    use crate::Scalar;
    use crate::media::avro;
    use crate::media::avro::Schema;

    #[test]
    fn a_datum_round_trips_through_the_single_object_framing() {
        let schema = Schema::from_str(
            r#"{"type":"record","name":"trade","fields":[
                {"name":"symbol","type":"string"},
                {"name":"qty","type":"long"}
            ]}"#,
        )
        .unwrap();
        let value = Scalar::from_record([
            ("symbol", Scalar::from("AAPL")),
            ("qty", Scalar::from(100_i64)),
        ])
        .unwrap();
        let framed = avro::into_single_object_vec(&schema, &value).unwrap();
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
        let framed = avro::into_single_object_vec(&schema, &Scalar::I64(1)).unwrap();
        let message = avro::from_single_object_slice(&framed, &other)
            .unwrap_err()
            .to_string();
        assert!(message.contains("fingerprint"), "{message}");
    }

    #[test]
    fn bytes_after_the_datum_are_refused() {
        let schema = Schema::from_str("\"long\"").unwrap();
        let mut framed = avro::into_single_object_vec(&schema, &Scalar::I64(1)).unwrap();
        framed.push(0x00);
        let message = avro::from_single_object_slice(&framed, &schema)
            .unwrap_err()
            .to_string();
        assert!(message.contains("end after its datum"), "{message}");
    }
}

#[cfg(feature = "parquet")]
mod snappy {
    use crate::Scalar;
    use crate::media::avro;

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
        assert_eq!(container.rows, [Scalar::I64(42)]);
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
    use crate::media::avro;

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

#[cfg(feature = "arrow")]
mod records {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, StringBuilder};
    use arrow_array::types::{Float64Type, Int64Type};
    use arrow_array::{Array, RecordBatch, RecordBatchIterator, cast::AsArray};
    use arrow_schema::ArrowError;

    use crate::holder::Buffer;
    use crate::media::avro;
    use crate::media::avro::{Avro, AvroOptions};
    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{DataType, Field, MediaType, Url};
    use crate::{IOBase, IOMedia};

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
        let arrow_schema = schema.clone().into_arrow_schema().unwrap();
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

    /// Two independent handles over one in-memory byte value.
    #[derive(Clone, Debug)]
    struct Shared {
        handle: Arc<Mutex<Buffer>>,
        media_type: MediaType,
    }

    impl Shared {
        fn new(handle: Buffer) -> Self {
            let media_type = handle.media_type().clone();
            Self {
                handle: Arc::new(Mutex::new(handle)),
                media_type,
            }
        }
    }

    impl crate::IOMedia for Shared {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Shared {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
            self.handle.lock().unwrap().pread(offset, buffer)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            self.handle.lock().unwrap().pwrite(offset, bytes)
        }

        fn size(&self) -> u64 {
            self.handle.lock().unwrap().size()
        }

        fn capacity(&self) -> u64 {
            self.handle.lock().unwrap().capacity()
        }

        fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
            self.handle.lock().unwrap().reserve(capacity)
        }

        fn truncate(&mut self, size: u64) -> crate::Result<()> {
            self.handle.lock().unwrap().truncate(size)
        }

        fn url(&self) -> Option<&Url> {
            None
        }

        fn media_type(&self) -> &MediaType {
            &self.media_type
        }

        fn set_media_type(&mut self, media_type: MediaType) {
            self.handle
                .lock()
                .unwrap()
                .set_media_type(media_type.clone());
            self.media_type = media_type;
        }
    }

    /// A positional handle measuring how much row-size metadata traversal
    /// fetches from a large container.
    struct Counting {
        handle: Buffer,
        reads: AtomicUsize,
        bytes: AtomicUsize,
    }

    impl Counting {
        fn new(handle: Buffer) -> Self {
            Self {
                handle,
                reads: AtomicUsize::new(0),
                bytes: AtomicUsize::new(0),
            }
        }

        fn cost(&self, operation: impl FnOnce()) -> (usize, usize) {
            let reads = self.reads.load(Ordering::Relaxed);
            let bytes = self.bytes.load(Ordering::Relaxed);
            operation();
            (
                self.reads.load(Ordering::Relaxed) - reads,
                self.bytes.load(Ordering::Relaxed) - bytes,
            )
        }
    }

    impl crate::IOMedia for Counting {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Counting {
        crate::delegate_iobase!(handle: pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular);

        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
            let read = self.handle.pread(offset, buffer)?;
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.bytes.fetch_add(read, Ordering::Relaxed);
            Ok(read)
        }
    }

    /// A one-batch reader that reports whether a write pulled its input.
    fn counted_reader(pulls: Arc<AtomicUsize>) -> crate::arrow::BatchReader {
        let (_, batch) = batch();
        let schema = batch.schema();
        let batches = std::iter::once(batch).inspect(move |_| {
            pulls.fetch_add(1, Ordering::Relaxed);
        });
        crate::arrow::batch_reader(schema, batches)
    }

    fn reader_then_error(first: RecordBatch) -> crate::arrow::BatchReader {
        let schema = first.schema();
        Box::new(RecordBatchIterator::new(
            [
                Ok(first),
                Err(ArrowError::ComputeError("later Avro source failure".into())),
            ],
            schema,
        ))
    }

    #[test]
    fn batches_round_trip_through_the_record_surface() {
        let (schema, batch) = batch();
        let mut handle = handle();
        let options = crate::media::RecordOptions::for_media_type(handle.media_type()).unwrap();
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();

        let read: Vec<RecordBatch> = handle
            .read_arrow_reader(&options)
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
        avro::overwrite_arrow_reader(
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
        let options = AvroOptions::new().with_field(batch().0);
        let read = avro::read_batch_reader(&handle, None, &options).unwrap();
        assert_eq!(read.count(), 0);
    }

    #[test]
    fn dimensions_describe_all_blocks_and_ignore_read_options() {
        let (field, first) = batch();
        let (_, second) = batch();
        let mut media = Avro::new(handle());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(first.schema(), [first, second]),
                &options,
            )
            .unwrap();
        media.options_mut().set_max_row_size(Some(1));
        media.options_mut().set_select_by_names(vec!["id".into()]);
        media
            .options_mut()
            .set_filter_partitions(vec![("id".into(), "999".into())]);

        assert_eq!(media.row_size().unwrap(), 6);
        assert_eq!(media.column_size().unwrap(), field.field_len());
    }

    #[test]
    fn an_empty_open_avro_container_has_explicit_lifecycle_and_dimensions() {
        let (field, batch) = batch();
        let mut media = Avro::new(handle()).with_field(field.clone());

        media.open().unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 0);
        assert_eq!(media.column_size().unwrap(), field.field_len());

        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 3);

        media.clear().unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 0);
        assert_eq!(media.column_size().unwrap(), field.field_len());

        media.remove(false).unwrap();
        assert!(!media.opened(), "removal ends the opened session");
    }

    #[test]
    fn an_open_avro_cache_is_stable_until_close_then_reads_fresh() {
        let encoded = |copies: usize| {
            let (_, template) = batch();
            let schema = template.schema();
            let batches = std::iter::repeat_n(template, copies);
            let mut media = Avro::new(handle());
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(crate::arrow::batch_reader(schema, batches), &options)
                .unwrap();
            media.into_handle()
        };
        let first = encoded(1);
        let replacement = encoded(2);
        let shared = Shared::new(first);
        let mut external = shared.clone();
        let mut media = Avro::new(shared);

        media.open().unwrap();
        assert_eq!(media.row_size().unwrap(), 3);
        external.write_all_bytes(replacement.as_slice()).unwrap();
        assert_eq!(media.row_size().unwrap(), 3, "the open metadata is stable");

        media.close().unwrap();
        assert!(!media.opened());
        assert_eq!(media.row_size().unwrap(), 6, "closed reads are fresh");
    }

    #[test]
    fn dimensions_skip_a_large_avro_payload_without_decoding_it() {
        let field = DataType::from_fields([DataType::Utf8.required_field("payload")])
            .unwrap()
            .required_field("rows");
        let payload = "0123456789abcdef".repeat(65_536);
        let batch = RecordBatch::try_new(
            field.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(arrow_array::StringArray::from(vec![payload]))],
        )
        .unwrap();
        let options = AvroOptions::new()
            .with_codec("null")
            .with_field(field.clone());
        let mut media = Avro::new(handle()).with_options(options);
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        let counting = Counting::new(media.into_handle());
        let total = counting.size() as usize;

        let (schema_reads, schema_fetched) = counting.cost(|| {
            assert_eq!(counting.column_size().unwrap(), 1);
        });
        assert!(schema_reads > 0);
        assert!(
            schema_fetched * 4 < total,
            "schema header fetched {schema_fetched} of {total} encoded bytes"
        );

        let (reads, fetched) = counting.cost(|| {
            assert_eq!(counting.row_size().unwrap(), 1);
        });
        assert!(reads > 0);
        assert!(
            fetched * 4 < total,
            "metadata traversal fetched {fetched} of {total} encoded bytes"
        );
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
        let mut media = Avro::new(handle()).with_name("trades");
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert!(!media.opened(), "a closed write must not start a cache");
        media.open().unwrap();
        assert!(media.opened());
        let derived = media.read_arrow_field(&options).unwrap();
        assert_eq!(derived.name(), schema.name());
        assert_eq!(derived.field_len(), schema.field_len());
        media.close().unwrap();
        assert!(!media.opened());
    }

    #[test]
    fn the_wrapper_owns_avro_options_over_an_unnamed_buffer() {
        let (field, _) = batch();
        let media = Avro::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();

        assert!(matches!(options, RecordOptions::Avro(_)));
        assert_eq!(options.field(), Some(field));
    }

    #[test]
    fn mismatched_options_are_rejected_before_any_write_pulls_input() {
        let (field, _) = batch();
        for operation in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut media = Avro::new(Buffer::new()).with_field(field.clone());
            let mut options = RecordOptions::Ipc(crate::media::ipc::IpcOptions::new());
            if operation == "merge" {
                options.set_merge_by_names(vec!["id".into()]);
            }
            let result = match operation {
                "overwrite" => crate::IOMedia::overwrite_arrow_reader(
                    &mut media,
                    counted_reader(Arc::clone(&pulls)),
                    &options,
                ),
                "append" => crate::IOMedia::append_arrow_reader(
                    &mut media,
                    counted_reader(Arc::clone(&pulls)),
                    &options,
                ),
                "merge" => crate::IOMedia::merge_arrow_reader(
                    &mut media,
                    counted_reader(Arc::clone(&pulls)),
                    &options,
                ),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("Avro"), "{operation}: {message}");
            assert_eq!(pulls.load(Ordering::Relaxed), 0, "{operation}");
            assert!(media.handle().is_empty(), "{operation}");
        }
    }

    #[test]
    fn an_ascii_column_is_an_avro_string() {
        let root = DataType::from_fields([
            DataType::FixedAscii(4).required_field("ccy"),
            DataType::FixedAscii(16).nullable_field("code"),
        ])
        .unwrap()
        .required_field("row");
        let schema = crate::media::avro::arrow::schema_json_from_field(&root).unwrap();
        let fields = schema
            .get_key_str("fields")
            .and_then(crate::Scalar::as_sequence)
            .unwrap();
        assert_eq!(
            fields[0]
                .get_key_str("type")
                .and_then(crate::Scalar::as_str),
            Some("string")
        );
        let optional = fields[1]
            .get_key_str("type")
            .and_then(crate::Scalar::as_sequence)
            .unwrap();
        assert!(
            optional
                .iter()
                .any(|branch| branch.as_str() == Some("string")),
            "{optional:?}"
        );
    }

    #[test]
    fn a_code_column_is_an_avro_string_and_a_code_key_is_spellable() {
        // Avro has no fixed-width text, so a code spells `string` with no
        // logical type - the contrast with a GUID, which annotates `uuid`.
        let root = DataType::from_fields([
            DataType::Country.required_field("iso"),
            DataType::Currency.required_field("ccy"),
            DataType::Mic.required_field("venue"),
            DataType::Cfi.required_field("classification"),
            // A map key gate that nothing else in the tree exercises for a
            // non-Utf8 key.
            DataType::map_of(DataType::Mic, DataType::Int64, true)
                .unwrap()
                .required_field("by_venue"),
        ])
        .unwrap()
        .required_field("row");
        let schema = crate::media::avro::arrow::schema_json_from_field(&root).unwrap();
        let fields = schema
            .get_key_str("fields")
            .and_then(crate::Scalar::as_sequence)
            .unwrap();

        for (index, field) in fields.iter().take(4).enumerate() {
            assert_eq!(
                field.get_key_str("type").and_then(crate::Scalar::as_str),
                Some("string"),
                "{index}"
            );
        }
        assert_eq!(
            fields[4]
                .get_key_str("type")
                .and_then(|value| value.get_key_str("type"))
                .and_then(|value| value.as_str().map(str::to_owned)),
            Some("map".to_owned())
        );
    }

    #[test]
    fn an_open_cache_tracks_the_final_avro_field_after_casting() {
        let stored = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap()
        .required_field("trades");
        let first = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&stored).unwrap(),
            vec![
                Arc::new(arrow_array::Int64Array::from(vec![1])),
                Arc::new(arrow_array::StringArray::from(vec![Some("AAPL")])),
            ],
        )
        .unwrap();
        let mut media = Avro::new(handle()).with_name("trades");
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(first.schema(), [first]),
                &options,
            )
            .unwrap();
        media.open().unwrap();

        let loose = DataType::from_fields([
            DataType::Utf8.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap()
        .required_field("trades");
        let incoming = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&loose).unwrap(),
            vec![
                Arc::new(arrow_array::StringArray::from(vec!["7"])),
                Arc::new(arrow_array::StringArray::from(vec![Some("MSFT")])),
            ],
        )
        .unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        assert!(media.opened());
        assert_eq!(media.read_arrow_field(&options).unwrap(), stored);
    }

    #[test]
    fn append_and_merge_keep_an_open_avro_cache_coherent_until_close() {
        let (field, first) = batch();
        let mut media = Avro::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(first.schema(), [first]),
                &options,
            )
            .unwrap();
        assert!(!media.opened());
        media.open().unwrap();
        media.options_mut().set_commit_row_size(Some(1));

        let (_, appended) = batch();
        let append_options = media.record_options().unwrap();
        media
            .append_arrow_reader(
                crate::arrow::batch_reader(appended.schema(), [appended]),
                &append_options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.read_arrow_field(&append_options).unwrap(), field);
        assert_eq!(
            media
                .read_arrow_reader(&append_options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            6
        );

        media.options_mut().set_merge_by_names(vec!["id".into()]);
        let (_, merged) = batch();
        let merge_options = media.record_options().unwrap();
        media
            .merge_arrow_reader(
                crate::arrow::batch_reader(merged.schema(), [merged]),
                &merge_options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.read_arrow_field(&merge_options).unwrap(), field);
        assert_eq!(
            media
                .read_arrow_reader(&merge_options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            6
        );

        media.close().unwrap();
        assert!(!media.opened());
        assert_eq!(media.read_arrow_field(&merge_options).unwrap(), field);
    }

    #[test]
    fn a_partial_commit_keeps_an_open_avro_cache_coherent() {
        let (field, first) = batch();
        let mut media = Avro::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(first.schema(), [first]),
                &options,
            )
            .unwrap();
        media.open().unwrap();
        media.options_mut().set_commit_row_size(Some(1));
        let (_, incoming) = batch();

        let options = media.record_options().unwrap();
        let message = media
            .overwrite_arrow_reader(reader_then_error(incoming.slice(0, 1)), &options)
            .unwrap_err()
            .to_string();

        assert!(message.contains("later Avro source failure"), "{message}");
        assert!(media.opened());
        assert_eq!(media.read_arrow_field(&options).unwrap(), field);
        assert_eq!(media.row_size().unwrap(), 1);
        assert_eq!(
            media
                .read_arrow_reader(&options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn a_wide_union_is_refused_for_the_record_surface() {
        let mut handle = handle();
        avro::write_container(
            &mut handle,
            &crate::text::json::from_utf8(
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
    fn single_branch_unions_read_through_the_record_surface() {
        // ["null"] alone and ["long"] alone are legal unions, and each still
        // spends a branch index on the wire.
        let mut handle = handle();
        avro::write_container(
            &mut handle,
            &crate::text::json::from_utf8(
                r#"{"type":"record","name":"row","fields":[
                    {"name":"gap","type":["null"]},
                    {"name":"v","type":["long"]}
                ]}"#,
            )
            .unwrap(),
            &[],
            &[
                crate::text::json::from_utf8(r#"{"gap":null,"v":5}"#).unwrap(),
                crate::text::json::from_utf8(r#"{"gap":null,"v":6}"#).unwrap(),
            ],
        )
        .unwrap();
        let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, None, &AvroOptions::new())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read[0].num_rows(), 2);
        // A NullArray's nulls are logical, so the count is asked logically.
        assert_eq!(read[0].column(0).logical_null_count(), 2);
        assert_eq!(
            read[0].column(1).as_primitive::<Int64Type>().values(),
            &[5, 6]
        );
    }

    #[test]
    fn a_null_typed_column_round_trips_without_double_wrapping() {
        // A nullable Null column must not be spelled ["null","null"] - the
        // declared type is already the null the wrap would add.
        let schema = Field::new(
            "row",
            DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Null.nullable_field("gap"),
            ])
            .unwrap(),
            false,
        );
        let arrow_schema = schema.into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![
                Arc::new(arrow_array::Int64Array::from(vec![1, 2])),
                Arc::new(arrow_array::NullArray::new(2)),
            ],
        )
        .unwrap();
        let mut handle = handle();
        avro::overwrite_arrow_reader(
            &mut handle,
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &AvroOptions::new(),
        )
        .unwrap();
        let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, None, &AvroOptions::new())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read[0].num_rows(), 2);
        assert_eq!(read[0].column(1).logical_null_count(), 2);
    }

    #[test]
    fn an_out_of_range_duration_is_refused_in_both_directions() {
        // Encode: the wire counts are unsigned, so a negative arrow interval
        // cannot be spelled.
        let schema = Field::new(
            "row",
            DataType::from_fields([
                DataType::Interval(crate::TimeUnit::MonthDayNano).required_field("span")
            ])
            .unwrap(),
            false,
        );
        let arrow_schema = schema.into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![Arc::new(arrow_array::IntervalMonthDayNanoArray::from(
                vec![arrow_buffer::IntervalMonthDayNano::new(-1, 0, 0)],
            ))],
        )
        .unwrap();
        let mut handle = handle();
        let message = avro::overwrite_arrow_reader(
            &mut handle,
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &AvroOptions::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("non-negative duration months"),
            "{message}"
        );

        // Decode: a wire count above 31 bits cannot fit the arrow interval
        // and is refused, never clamped.
        let mut payload = vec![0xFF, 0xFF, 0xFF, 0xFF];
        payload.extend_from_slice(&[0; 8]);
        let handle = super::handmade_container(
            r#"{"type":"record","name":"row","fields":[
                {"name":"span","type":
                    {"type":"fixed","name":"dur","size":12,"logicalType":"duration"}}
            ]}"#,
            "null",
            &[(1, payload)],
        );
        let message = avro::read_batch_reader(&handle, None, &AvroOptions::new())
            .unwrap()
            .collect::<Result<Vec<RecordBatch>, _>>()
            .unwrap_err()
            .to_string();
        assert!(message.contains("within 31 bits"), "{message}");
    }

    #[test]
    fn logical_columns_round_trip_columnar() {
        let schema = Field::new(
            "row",
            DataType::from_fields([
                DataType::Date32.required_field("day"),
                DataType::Timestamp(crate::TimeUnit::Microsecond, Some(crate::Timezone::UTC))
                    .nullable_field("at"),
                DataType::decimal128(10, 2).unwrap().required_field("cost"),
            ])
            .unwrap(),
            false,
        );
        let arrow_schema = schema.into_arrow_schema().unwrap();
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
        avro::overwrite_arrow_reader(
            &mut handle,
            crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options,
        )
        .unwrap();

        // The container is readable at the Scalar level with typed temporals.
        let container = avro::read_container(&handle).unwrap();
        assert_eq!(
            container.rows[0].get_key_str("day"),
            Some(&crate::Scalar::date32(19_782))
        );
        assert_eq!(
            container.rows[1].get_key_str("cost"),
            Some(&crate::Scalar::d128(-99, 2))
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

mod matrix {
    use crate::Scalar;
    use crate::TimeUnit;
    use crate::media::avro;

    /// Round-trip rows through a container and hand them back.
    fn round_trip(schema: &str, rows: &[Scalar]) -> Vec<Scalar> {
        let schema = crate::text::json::from_utf8(schema).unwrap();
        let mut handle = super::buffer();
        avro::write_container(&mut handle, &schema, &[], rows).unwrap();
        avro::read_container(&handle).unwrap().rows
    }

    #[test]
    fn maps_round_trip_including_the_empty_one() {
        let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"counts","type":{"type":"map","values":"long"}}
        ]}"#;
        let full = Scalar::from_record([(
            "counts",
            Scalar::from_mapping([
                (Scalar::from("a"), Scalar::from(1_i64)),
                (Scalar::from("b"), Scalar::from(-2_i64)),
            ])
            .unwrap(),
        )])
        .unwrap();
        let empty = Scalar::from_record([("counts", Scalar::from_mapping([]).unwrap())]).unwrap();
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
        let row = crate::text::json::from_utf8(
            r#"{"outer":[{"by_name":{"legs":[{"flag":true},{"flag":false}],"none":[]}}]}"#,
        )
        .unwrap();
        let rows = round_trip(schema, std::slice::from_ref(&row));
        assert!(rows[0].as_record().is_some());
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
        let some = crate::text::json::from_utf8(r#"{"v":{"x":9}}"#).unwrap();
        let none = crate::text::json::from_utf8(r#"{"v":null}"#).unwrap();
        let rows = round_trip(schema, &[some.clone(), none.clone()]);
        assert_eq!(rows, [some, none]);
    }

    #[test]
    fn time_boundaries_round_trip_exactly() {
        let schema = r#"{"type":"record","name":"row","fields":[
            {"name":"ms","type":{"type":"int","logicalType":"time-millis"}},
            {"name":"us","type":{"type":"long","logicalType":"time-micros"}}
        ]}"#;
        let row = Scalar::from_record([
            (
                "ms",
                Scalar::Time32(0, TimeUnit::Millisecond, crate::Timezone::NAIVE),
            ),
            (
                "us",
                Scalar::Time64(
                    86_399_999_999,
                    TimeUnit::Microsecond,
                    crate::Timezone::NAIVE,
                ),
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
        let row = Scalar::from_record([("day", Scalar::date32(19_782))]).unwrap();
        assert_eq!(round_trip(schema, std::slice::from_ref(&row))[0], row);
    }

    #[test]
    fn trailing_bytes_after_the_declared_rows_are_an_error() {
        // A block declaring one null row but carrying a stray byte.
        let handle = super::handmade_container("\"null\"", "null", &[(1, vec![0x2A])]);
        let message = crate::media::avro::read_container(&handle)
            .unwrap_err()
            .to_string();
        assert!(message.contains("end after 1 declared rows"), "{message}");
    }
}

mod snapshots {
    use crate::IOBase;
    use crate::Scalar;
    use crate::media::avro;

    /// The byte snapshot of one fixed schema and data pair.
    ///
    /// The writer is a pure function of its input, so any encoding change -
    /// varints, field order, header layout, the derived sync marker, the
    /// deflate stream - surfaces here immediately. Update the expectation
    /// only for a deliberate format change, never to quiet the test.
    #[test]
    fn a_fixed_container_encodes_to_exactly_these_bytes() {
        let schema = crate::text::json::from_utf8(
            r#"{"type":"record","name":"snap","fields":[
                {"name":"id","type":"long"},
                {"name":"tag","type":"string"}
            ]}"#,
        )
        .unwrap();
        let rows = [
            crate::text::json::from_utf8(r#"{"id":1,"tag":"a"}"#).unwrap(),
            crate::text::json::from_utf8(r#"{"id":-2,"tag":"bc"}"#).unwrap(),
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

    /// The single-object framing is fixed by the specification.
    #[test]
    fn a_single_object_datum_encodes_to_exactly_these_bytes() {
        let schema = avro::Schema::from_str("\"long\"").unwrap();
        let framed = avro::into_single_object_vec(&schema, &Scalar::I64(3)).unwrap();
        let hex: String = framed.iter().map(|byte| format!("{byte:02x}")).collect();
        // C3 01, the little-endian Rabin fingerprint of "long" (the value
        // fastavro computes for the same schema), then zig-zag 3.
        assert_eq!(hex, "c301b71df49344e154d006");
    }
}

mod fuzz_lite {
    //! Seeded mutation sweeps: every outcome must be a value or a typed
    //! error - no panic, no runaway allocation. A longer sweep scales with
    //! `AVRO_FUZZ_ITERATIONS`, matching how the repository gates its slow
    //! legs outside the default test run.

    use crate::IOBase;
    use crate::Limits;
    use crate::media::avro;

    /// A deterministic pseudo-random byte source.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }

        fn byte(&mut self) -> u8 {
            (self.next() >> 33) as u8
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() >> 16) as usize % bound.max(1)
        }
    }

    fn iterations() -> usize {
        std::env::var("AVRO_FUZZ_ITERATIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(400)
    }

    #[test]
    fn mutated_containers_never_panic() {
        let schema = super::manifest_shaped_schema();
        let rows = [crate::text::json::from_utf8(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,3],
                "nested":{"flag":true}}"#,
        )
        .unwrap()];
        let mut handle = super::buffer();
        avro::write_container(&mut handle, &schema, &[("k", "v")], &rows).unwrap();
        let valid = handle.read_all_bytes().unwrap();

        let limits = Limits::new(32, 1 << 16, 1 << 12, 64);
        let mut random = Lcg(0x5EED);
        for _ in 0..iterations() {
            let mut mutated = valid.clone();
            for _ in 0..1 + random.below(4) {
                let index = random.below(mutated.len());
                mutated[index] = random.byte();
            }
            let mut corrupt = super::buffer();
            corrupt.write_all_bytes(&mutated).unwrap();
            // A value or a typed error; anything else fails the test by panic.
            let _ = avro::read_container_with_limits(&corrupt, limits);
        }
    }

    #[test]
    fn random_bytes_never_panic_any_entry_point() {
        let limits = Limits::new(32, 1 << 14, 1 << 10, 64);
        let mut random = Lcg(0xACE5);
        for _ in 0..iterations() {
            let length = random.below(512);
            let bytes: Vec<u8> = (0..length).map(|_| random.byte()).collect();
            let mut handle = super::buffer();
            handle.write_all_bytes(&bytes).unwrap();
            let _ = avro::read_container_with_limits(&handle, limits);
            if let Ok(mut blocks) = avro::read_blocks_with_limits(&handle, limits) {
                while let Ok(Some(block)) = blocks.next_block() {
                    let _ = block.rows();
                }
            }
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let _ = avro::Schema::from_str(text);
            }
        }
    }

    #[test]
    fn mutated_schema_documents_never_panic() {
        let source = r#"{"type":"record","name":"row","fields":[
            {"name":"a","type":["null","long"],"default":null},
            {"name":"b","type":{"type":"array","items":"string"}},
            {"name":"c","type":{"type":"fixed","name":"f","size":4,"logicalType":"decimal",
             "precision":9,"scale":2}}
        ]}"#;
        let mut random = Lcg(0xF00D);
        let bytes = source.as_bytes();
        for _ in 0..iterations() {
            let mut mutated = bytes.to_vec();
            for _ in 0..1 + random.below(3) {
                let index = random.below(mutated.len());
                mutated[index] = random.byte();
            }
            if let Ok(text) = std::str::from_utf8(&mutated) {
                let _ = avro::Schema::from_str(text);
            }
        }
    }
}

mod limits {
    use std::sync::Arc;

    use arrow_array::RecordBatchReader;

    use crate::IOMedia;
    use crate::holder::Buffer;
    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{DataType, Field, Url};

    /// A struct field is the schema of the batches it describes.
    fn schema() -> Field {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row")
    }

    /// A reader over one batch of `ids`.
    fn reader(ids: Vec<i64>) -> crate::arrow::BatchReader {
        let batch = arrow_array::RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![Arc::new(arrow_array::Int64Array::from(ids))],
        )
        .unwrap();
        crate::arrow::batch_reader(batch.schema(), [batch])
    }

    fn handle() -> Buffer {
        Buffer::new().with_media_type(Url::from_str("file:///limited.avro").unwrap().media_type())
    }

    /// The total rows a handle yields under `options`.
    fn rows(handle: &Buffer, options: &RecordOptions) -> usize {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    #[test]
    fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
        let mut handle = handle();
        let options = handle.record_options().unwrap().with_field(schema());
        handle
            .overwrite_arrow_reader(reader(vec![1, 2]), &options)
            .unwrap();

        let mut limited = handle
            .read_arrow_reader(&options.with_max_row_size(0))
            .unwrap();
        // The schema is asserted, not only the emptiness: `Some(0)` is a
        // valid ask that still says what the rows would have been.
        assert_eq!(
            limited.schema(),
            crate::arrow::arrow_schema_from_field(&schema()).unwrap()
        );
        assert!(limited.next().is_none());
    }

    #[test]
    fn a_limited_write_truncates_what_the_caller_offered() {
        let mut handle = handle();
        let options = handle.record_options().unwrap().with_field(schema());

        handle
            .overwrite_arrow_reader(reader(vec![1, 2]), &options.clone().with_max_row_size(1))
            .unwrap();
        assert_eq!(rows(&handle, &options), 1);

        // An append is a write, so the same bound truncates it the same way.
        handle
            .append_arrow_reader(reader(vec![3, 4]), &options.clone().with_max_row_size(1))
            .unwrap();
        assert_eq!(rows(&handle, &options), 2);
    }
}
