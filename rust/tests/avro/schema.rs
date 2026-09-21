//! `rust/src/avro/schema.rs`: the schema model - named types, namespaces,
//! canonical form and fingerprints.

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType};

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    mod schemas {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        use yggdryl::avro::Container;
        use yggdryl::avro::Schema;

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

            let native_mapping = yggdryl::Scalar::from_mapping([
                (
                    yggdryl::Scalar::from("logicalType"),
                    yggdryl::Scalar::from("date"),
                ),
                (yggdryl::Scalar::from("type"), yggdryl::Scalar::from("int")),
            ])
            .unwrap();
            let native = Schema::from_json(&native_mapping).unwrap();
            assert_eq!(date, native);
            assert_eq!(date.stable_hash(), native.stable_hash());

            let reparsed =
                Schema::from_str(&yggdryl::json::into_utf8(&native.clone().into_json()).unwrap())
                    .unwrap();
            assert_eq!(native, reparsed);

            let nested_mapping = yggdryl::Scalar::from_mapping([
                (
                    yggdryl::Scalar::from("type"),
                    yggdryl::Scalar::from("record"),
                ),
                (yggdryl::Scalar::from("name"), yggdryl::Scalar::from("row")),
                (
                    yggdryl::Scalar::from("fields"),
                    yggdryl::Scalar::from_sequence([yggdryl::Scalar::from_mapping([
                        (
                            yggdryl::Scalar::from("name"),
                            yggdryl::Scalar::from("items"),
                        ),
                        (
                            yggdryl::Scalar::from("type"),
                            yggdryl::Scalar::from_mapping([
                                (
                                    yggdryl::Scalar::from("type"),
                                    yggdryl::Scalar::from("array"),
                                ),
                                (
                                    yggdryl::Scalar::from("items"),
                                    yggdryl::Scalar::from_mapping([(
                                        yggdryl::Scalar::from("type"),
                                        yggdryl::Scalar::from("int"),
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
                rows: vec![yggdryl::Scalar::from(7)],
            };
            let mut equal = container.clone();
            equal.metadata.reverse();
            let mut later = container.clone();
            later.rows.push(yggdryl::Scalar::from(8));
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
            let document = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"row","fields":[{"name":"id","type":"int","field-id":42}]}"#,
            )
            .unwrap();
            let schema = Schema::from_json(&document).unwrap();
            assert_eq!(schema.clone().into_json(), document);
            // The unmodeled attribute is still in the JSON the schema writes.
            let text =
                String::from_utf8(yggdryl::json::into_bytes(&schema.into_json()).unwrap()).unwrap();
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
            let schema_json = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"node","fields":[
                {"name":"value","type":"long"},
                {"name":"next","type":["null","node"],"default":null}
            ]}"#,
            )
            .unwrap();
            let list = yggdryl::json::from_utf8(
                r#"{"value":1,"next":{"value":2,"next":{"value":3,"next":null}}}"#,
            )
            .unwrap();
            let mut handle = super::buffer();
            yggdryl::avro::write_container(&mut handle, &schema_json, &[], &[list]).unwrap();
            let container = yggdryl::avro::read_container(&handle).unwrap();
            let tail = container.rows[0]
                .path("next.next.value")
                .and_then(yggdryl::Scalar::as_i64);
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
            let parsed = yggdryl::json::from_utf8(&document).unwrap();
            let limits = yggdryl::Limits::new(8, 1 << 20, 1 << 20, 8);
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
    }
}
