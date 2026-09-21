//! `rust/src/avro/resolve.rs`: writer/reader resolution - every promotion it
//! allows, every mismatch it names, and the Variant buffers it keeps.

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType, Scalar};

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

    mod resolution {
        use yggdryl::Scalar;
        use yggdryl::avro;
        use yggdryl::avro::Resolution;
        use yggdryl::avro::Schema;

        /// Write rows with the writer schema, read them back with the reader.
        fn resolved(writer: &str, reader: &str, rows: &[&str]) -> Vec<Scalar> {
            let writer_json = yggdryl::json::from_utf8(writer).unwrap();
            let mut handle = super::buffer();
            let rows: Vec<Scalar> = rows
                .iter()
                .map(|row| yggdryl::json::from_utf8(row).unwrap())
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
                ("\"int\"", "\"long\"", r#"{"v":7}"#, Scalar::from(7)),
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

            let writer_json = yggdryl::json::from_utf8(&writer).unwrap();
            let mut handle = super::buffer();
            avro::write_container(
                &mut handle,
                &writer_json,
                &[],
                &[yggdryl::json::from_utf8(r#"{"v":null}"#).unwrap()],
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
            let row = yggdryl::json::from_utf8(
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
}

mod variants {
    use yggdryl::IOBase;
    use yggdryl::avro::{self, Schema};
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType, Scalar, Variant};

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    fn variant() -> Variant {
        Variant::encode(
            &Scalar::from_struct([
                ("name", Scalar::from("Ada")),
                ("active", Scalar::from(true)),
            ])
            .unwrap(),
        )
        .unwrap()
    }

    fn variant_record(annotation: bool) -> String {
        let annotation = if annotation {
            ",\"logicalType\":\"variant\""
        } else {
            ""
        };
        format!(
            r#"{{"type":"record","name":"variant_value"{annotation},"fields":[{{"name":"metadata","type":"bytes"}},{{"name":"value","type":"bytes"}}]}}"#
        )
    }

    fn schema_with_one_variant(annotation: bool) -> String {
        format!(
            r#"{{"type":"record","name":"row","fields":[{{"name":"v","type":{}}}]}}"#,
            variant_record(annotation),
        )
    }

    fn resolved(writer: &str, reader: &str, rows: &[Scalar]) -> Vec<Scalar> {
        let writer_json = yggdryl::json::from_utf8(writer).unwrap();
        let reader = reader.parse::<Schema>().unwrap();
        let mut handle = buffer();
        avro::write_container(&mut handle, &writer_json, &[], rows).unwrap();
        avro::read_container_resolved(&handle, &reader)
            .unwrap()
            .rows
    }

    #[test]
    fn an_annotated_reader_resolves_a_variant_with_its_exact_buffers() {
        let held = variant();
        let rows = resolved(
            &schema_with_one_variant(true),
            &schema_with_one_variant(true),
            &[Scalar::from_struct([("v", Scalar::Variant(held.clone()))]).unwrap()],
        );
        assert_eq!(
            rows[0].get_key_str("v"),
            Some(&Scalar::Variant(held)),
            "the reader annotation owns Variant interpretation"
        );
    }

    #[test]
    fn an_unannotated_reader_keeps_a_variant_record_as_an_ordinary_struct() {
        let held = variant();
        let rows = resolved(
            &schema_with_one_variant(true),
            &schema_with_one_variant(false),
            &[Scalar::from_struct([("v", Scalar::Variant(held.clone()))]).unwrap()],
        );
        let record = rows[0]
            .get_key_str("v")
            .and_then(Scalar::as_struct)
            .unwrap();
        assert_eq!(
            record.get("metadata").and_then(Scalar::as_bytes),
            Some(held.metadata())
        );
        assert_eq!(
            record.get("value").and_then(Scalar::as_bytes),
            Some(held.value())
        );
    }

    #[test]
    fn a_variant_through_a_named_reference_and_union_resolves_as_a_variant() {
        let held = variant();
        let schema = format!(
            r#"{{"type":"record","name":"row","fields":[
            {{"name":"first","type":{}}},
            {{"name":"by_ref","type":"variant_value"}},
            {{"name":"by_union","type":["null","variant_value"],"default":null}}
        ]}}"#,
            variant_record(true),
        );
        let row = Scalar::from_struct([
            ("first", Scalar::Variant(held.clone())),
            ("by_ref", Scalar::Variant(held.clone())),
            ("by_union", Scalar::Variant(held.clone())),
        ])
        .unwrap();
        let rows = resolved(&schema, &schema, &[row]);
        for name in ["first", "by_ref", "by_union"] {
            assert_eq!(
                rows[0].get_key_str(name),
                Some(&Scalar::Variant(held.clone())),
                "{name}"
            );
        }
    }

    #[test]
    fn a_missing_annotated_variant_field_uses_its_variant_default() {
        let held = variant();
        let byte_string = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("\\u{byte:04x}"))
                .collect::<String>()
        };
        let writer = r#"{"type":"record","name":"row","fields":[{"name":"id","type":"long"}]}"#;
        let reader = format!(
            r#"{{"type":"record","name":"row","fields":[
            {{"name":"id","type":"long"}},
            {{"name":"v","type":{},"default":{{"metadata":"{}","value":"{}"}}}}
        ]}}"#,
            variant_record(true),
            byte_string(held.metadata()),
            byte_string(held.value()),
        );
        let rows = resolved(
            writer,
            &reader,
            &[Scalar::from_struct([("id", Scalar::from(7_i64))]).unwrap()],
        );
        assert_eq!(rows[0].get_key_str("v"), Some(&Scalar::Variant(held)));
    }
}
