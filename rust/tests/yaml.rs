use std::io::{Cursor, Read};
use yggdryl::yaml;

use yggdryl::{Limits, Value};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn borrowed_text_entry_points_match_bytes_without_an_intermediate_buffer() {
    let source = "label: café\nvalue: 1\n";
    let expected = yaml::from_slice(source.as_bytes()).unwrap();
    assert_eq!(yaml::from_str(source).unwrap(), expected);
    assert_eq!(
        yaml::from_str_with_limits(source, Limits::default()).unwrap(),
        expected
    );

    let documents = "café\n---\nsecond\n";
    assert_eq!(
        yaml::from_str_all(documents).unwrap(),
        yaml::from_slice_all(documents.as_bytes()).unwrap()
    );
    assert_eq!(
        yaml::from_str_all_with_limits(documents, Limits::default()).unwrap(),
        yaml::from_slice_all(documents.as_bytes()).unwrap()
    );
}

#[test]
fn nothing_this_emitter_writes_is_a_custom_yaml_tag() {
    // A custom `!yggdryl/*` tag makes the document unreadable to other YAML
    // implementations, so every value with no native spelling goes out as the
    // ordinary `$yggdryl` flow mapping instead.
    let value = Value::from_mapping([
        (Value::from("bytes"), Value::from(vec![0_u8, 255])),
        (Value::from("wide"), Value::U128(u128::MAX)),
        (Value::from("price"), Value::from(f64::INFINITY)),
    ])
    .unwrap();
    let encoded = yaml::to_vec(&value).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();

    assert!(!text.contains('!'), "{text}");
    assert!(text.contains("\"$yggdryl\": \"bytes\""), "{text}");
    assert_eq!(yaml::from_slice(&encoded).unwrap(), value);
}

#[test]
fn yaml_preserves_arbitrary_mapping_keys_and_bytes() {
    let value = Value::from_mapping([
        (
            Value::from_sequence([Value::from(1_i64)]),
            Value::from(vec![0, 255]),
        ),
        (Value::from(true), Value::from(f64::INFINITY)),
    ])
    .unwrap();
    assert_eq!(
        yaml::from_slice(&yaml::to_vec(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn comments_are_readability_only_and_never_instantiate_types() {
    let plain = yaml::from_slice(b"# python class orders.Trade\n{\"quantity\": 2}\n").unwrap();
    assert!(matches!(plain, Value::Mapping(_)));

    // A comment beside a value is display text: rewriting it cannot change what
    // the document decodes to, because no comment is ever read.
    let value = Value::from_mapping([(Value::from("quantity"), Value::from(2_u64))]).unwrap();
    let encoded = yaml::to_vec(&value).unwrap();
    let commented = format!(
        "# python class orders.Trade\n{}",
        String::from_utf8(encoded).unwrap()
    );
    let changed = commented.replace("python class orders.Trade", "untrusted display text");
    assert_eq!(yaml::from_slice(commented.as_bytes()).unwrap(), value);
    assert_eq!(yaml::from_slice(changed.as_bytes()).unwrap(), value);
}

#[test]
fn yaml_envelope_collisions_round_trip_as_plain_mappings() {
    let collision = Value::from_mapping([
        (Value::from("$yggdryl"), Value::from("bytes")),
        (Value::from("value"), Value::from("AP8=")),
    ])
    .unwrap();
    let encoded = yaml::to_vec(&collision).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(!text.contains("!yggdryl"), "{text}");
    assert!(text.contains("\"$yggdryl\": \"mapping\""), "{text}");
    assert_eq!(yaml::from_slice(&encoded).unwrap(), collision);
}

#[test]
fn yaml_decodes_a_plain_envelope_without_a_custom_tag() {
    // YAML no longer needs a `!yggdryl/*` tag to recognize an envelope. Note the
    // YAML envelope is flat (`{"$yggdryl": "bytes", "value": ...}`) while the
    // JSON one nests under the marker, so the two texts are not interchangeable.
    let cases: [(&[u8], Value); 4] = [
        (
            br#"{"$yggdryl": "bytes", "value": "AP8="}"#,
            Value::from(vec![0_u8, 255]),
        ),
        (
            br#"{"$yggdryl": "i128", "value": "-170141183460469231731687303715884105728"}"#,
            Value::I128(i128::MIN),
        ),
        (
            br#"{"$yggdryl": "u128", "value": "340282366920938463463374607431768211455"}"#,
            Value::U128(u128::MAX),
        ),
        (
            br#"{"$yggdryl": "mapping", "value": [["key", 1]]}"#,
            Value::from_mapping([(Value::from("key"), Value::from(1_u64))]).unwrap(),
        ),
    ];
    for (encoded, expected) in cases {
        let decoded = yaml::from_slice(encoded).unwrap();
        assert_eq!(decoded, expected, "{}", String::from_utf8_lossy(encoded));
    }

    // The `tag` kind named a carrier this value model no longer has, so it is
    // no longer an envelope kind and its mapping stays the mapping it is.
    let former_tag =
        yaml::from_slice(br#"{"$yggdryl": "tag", "tag": "python:pkg.C", "value": null}"#).unwrap();
    assert_eq!(
        former_tag,
        Value::from_mapping([
            (Value::from("$yggdryl"), Value::from("tag")),
            (Value::from("tag"), Value::from("python:pkg.C")),
            (Value::from("value"), Value::Null),
        ])
        .unwrap()
    );
    assert_eq!(
        yaml::from_slice(&yaml::to_vec(&former_tag).unwrap()).unwrap(),
        former_tag
    );
}

#[test]
fn a_plain_mapping_that_looks_like_an_envelope_still_round_trips() {
    // Emission double-wraps a colliding mapping, so user data holding
    // `$yggdryl` keys is never silently reinterpreted on the way back.
    for collision in [
        Value::from_mapping([
            (Value::from("$yggdryl"), Value::from("bytes")),
            (Value::from("value"), Value::from("AP8=")),
        ])
        .unwrap(),
        Value::from_mapping([
            (Value::from("$yggdryl"), Value::from("float")),
            (Value::from("value"), Value::from("nan")),
        ])
        .unwrap(),
    ] {
        let encoded = yaml::to_vec(&collision).unwrap();
        let text = std::str::from_utf8(&encoded).unwrap();
        assert!(!text.contains("!yggdryl"), "{text}");
        assert!(text.contains("\"$yggdryl\": \"mapping\""), "{text}");
        assert_eq!(yaml::from_slice(&encoded).unwrap(), collision);
    }
}

#[test]
fn quoted_merge_spelling_is_data_while_plain_merge_syntax_is_rejected() {
    let value = Value::from_mapping([(Value::from("<<"), Value::from(1_u64))]).unwrap();
    let encoded = yaml::to_vec(&value).unwrap();
    assert_eq!(yaml::from_slice(&encoded).unwrap(), value);
    assert_eq!(yaml::from_slice(br#"{"<<": 1}"#).unwrap(), value);
    assert!(yaml::from_slice(b"<<: {a: 1}\n").is_err());
    assert_eq!(
        yaml::from_slice(b"value: <<\n")
            .unwrap()
            .get_key_str("value"),
        Some(&Value::from("<<"))
    );
}

#[test]
fn yaml_limits_reject_deep_and_alias_heavy_documents() {
    let depth = Limits::new(2, 4096, 100, 10);
    assert!(yaml::from_slice_with_limits(b"[[0]]", depth).is_ok());
    assert!(yaml::from_slice_with_limits(b"[[[0]]]", depth).is_err());

    let aliases = b"base: &base [1, 2, 3]\nexpanded: [*base, *base, *base, *base]\n";
    let strict = Limits::new(16, 4096, 3, 10);
    assert!(yaml::from_slice_with_limits(aliases, strict).is_err());
}

#[test]
fn yaml_limits_apply_per_document_at_exact_boundaries() {
    let limits = Limits::new(0, 64, 1, 2);
    let values = yaml::from_slice_all_with_limits(b"0\n---\n1\n", limits).unwrap();
    assert_eq!(values.len(), 2);
    assert!(yaml::from_slice_with_limits(b"[]", limits).is_err());
    assert!(yaml::from_slice_with_limits(b"0", Limits::new(0, 64, 0, 1)).is_err());
    assert!(yaml::from_slice_all_with_limits(b"0\n---\n1\n---\n2\n", limits).is_err());
}

#[test]
fn yaml_document_reader_is_lazy_and_multi_document() {
    let mut reader = Cursor::new(b"{\"id\": 1}\n---\n{\"id\": 2}\n");
    let values = yaml::from_reader_iter(&mut reader)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values.len(), 2);
}

#[test]
fn encoder_rejects_one_over_the_default_depth_limit() {
    let maximum = Limits::default().max_depth();
    let at_limit = (0..maximum).fold(Value::Null, |value, _| Value::from_sequence([value]));
    let over_limit = Value::from_sequence([at_limit.clone()]);

    assert!(yaml::to_vec(&at_limit).is_ok());
    let error = yaml::to_vec(&over_limit).unwrap_err();
    assert!(error.to_string().contains("nesting depth limit"));
}

#[test]
fn wide_integer_envelopes_preserve_every_128_bit_boundary() {
    for value in [
        Value::I128(i128::MIN),
        Value::I128(i128::MAX),
        Value::U128(u128::from(u64::MAX) + 1),
        Value::U128(u128::MAX),
    ] {
        let encoded = yaml::to_vec(&value).unwrap();
        let text = std::str::from_utf8(&encoded).unwrap();
        assert!(!text.contains("!yggdryl"), "{text}");
        assert!(text.contains("\"$yggdryl\""), "{text}");
        assert_eq!(yaml::from_slice(&encoded).unwrap(), value);
    }

    assert_eq!(
        yaml::from_slice(i128::MIN.to_string().as_bytes()).unwrap(),
        Value::I128(i128::MIN)
    );
}

#[test]
fn numeric_separators_do_not_reclassify_malformed_spellings() {
    for (encoded, expected) in [
        ("1_000", Value::from(1_000_u64)),
        ("0xFF_FF", Value::from(65_535_u64)),
        ("1_2.5_0e+1_0", Value::from(12.50e10_f64)),
    ] {
        assert_eq!(yaml::from_str(encoded).unwrap(), expected);
    }
    for encoded in ["1__2", "1_.0", "1._0", "1e_2"] {
        assert_eq!(yaml::from_str(encoded).unwrap(), Value::from(encoded));
    }
    assert!(yaml::from_str("0x_FF").is_err());
}

#[test]
fn null_documents_are_never_dropped_from_any_stream_position() {
    let expected = vec![Value::Null, Value::Bool(false), Value::Null];
    let encoded = yaml::to_vec_all(&expected).unwrap();
    assert_eq!(yaml::from_slice_all(&encoded).unwrap(), expected);
    assert_eq!(
        yaml::Reader::new(Cursor::new(&encoded))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap(),
        expected
    );

    let all_null = b"null\n---\n~\n---\nnull\n";
    assert_eq!(
        yaml::from_slice_all(all_null).unwrap(),
        vec![Value::Null, Value::Null, Value::Null]
    );
    assert_eq!(
        yaml::from_slice_all(b"---\n---\n").unwrap(),
        vec![Value::Null, Value::Null]
    );
}

#[test]
fn a_machine_tag_is_semantic_and_every_other_tag_is_an_annotation() {
    assert_eq!(
        yaml::from_slice(b"!yggdryl/bytes AP8=\n").unwrap(),
        Value::from(vec![0, 255])
    );

    // No value can hold a free-form name any more, and every runtime that
    // consumed one already kept only the payload, so an application tag is read
    // as the YAML annotation it is and its node decodes as the plain value it
    // annotates rather than failing a document this codec can otherwise read.
    let annotated = yaml::from_slice(b"!python:pkg.C {value: 1}\n").unwrap();
    assert_eq!(
        annotated,
        Value::from_mapping([(Value::from("value"), Value::U64(1))]).unwrap()
    );
    assert_eq!(
        yaml::from_slice(b"!vendor:ratio 1.5\n").unwrap(),
        Value::from(1.5)
    );
}

#[test]
fn recognized_envelope_errors_keep_yaml_identity() {
    let error =
        yaml::from_slice(br#"!yggdryl/bytes {"$yggdryl": "bytes", "value": "not-base64"}\n"#)
            .unwrap_err();
    match error {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "yaml");
            assert_eq!(position, 0);
            assert_eq!(reason, "invalid base64 bytes envelope");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[test]
fn semantic_errors_keep_tag_and_duplicate_key_positions() {
    let tagged = b"ok: 1\nbad: !yggdryl/bytes ????\n";
    let error = yaml::from_slice(tagged).unwrap_err();
    match error {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "yaml");
            assert_eq!(position, b"ok: 1\nbad: ".len());
            assert!(reason.contains("base64"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }

    let duplicate = b"ok: 0\nnested: {a: 1, a: 2}\n";
    let error = yaml::from_slice(duplicate).unwrap_err();
    match error {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "yaml");
            assert_eq!(position, b"ok: 0\nnested: {a: 1, ".len());
            assert!(reason.contains("duplicate key"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn malformed_second_document_has_one_cumulative_byte_error_then_fuses() {
    let input = "é: ok\n---\n[1,\n".as_bytes();
    let second_start = "é: ok\n---\n".len();

    let mut reader = yaml::Reader::new(Cursor::new(input));
    assert!(reader.next().unwrap().is_ok());
    let error = reader.next().unwrap().unwrap_err();
    let position = match error {
        yggdryl::Error::Codec { position, .. } => position,
        other => panic!("unexpected error: {other}"),
    };
    assert!(position >= second_start, "{position} < {second_start}");
    assert!(reader.next().is_none());

    for error in [
        yaml::from_slice_all(input).unwrap_err(),
        yaml::from_reader_all(Cursor::new(input)).unwrap_err(),
    ] {
        let position = match error {
            yggdryl::Error::Codec { position, .. } => position,
            other => panic!("unexpected error: {other}"),
        };
        assert!(position >= second_start, "{position} < {second_start}");
    }
}

#[test]
fn single_document_paths_report_the_start_of_trailing_yaml() {
    let input = "---\r\nfirst: café\r\n---\r\nsecond: true\r\n";
    let expected = "---\r\nfirst: café\r\n".len();
    for error in [
        yaml::from_slice(input.as_bytes()).unwrap_err(),
        yaml::from_str(input).unwrap_err(),
        yaml::from_reader(Cursor::new(input.as_bytes())).unwrap_err(),
        yaml::from_reader(OneByte(Cursor::new(input.as_bytes()))).unwrap_err(),
    ] {
        match error {
            yggdryl::Error::Codec {
                format, position, ..
            } => {
                assert_eq!(format, "yaml");
                assert_eq!(position, expected);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

#[test]
fn yaml_custom_depth_and_integer_error_positions_are_effective() {
    let depth = 200;
    let input = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    assert!(
        yaml::from_slice_with_limits(
            input.as_bytes(),
            Limits::new(depth, input.len(), depth + 1, 1),
        )
        .is_ok()
    );

    let flow_limit = usize::from(u8::MAX) + 1;
    let over_flow_limit = format!("{}0{}", "[".repeat(flow_limit), "]".repeat(flow_limit));
    let error = yaml::from_slice_with_limits(
        over_flow_limit.as_bytes(),
        Limits::new(500, over_flow_limit.len(), 1_000, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("parser hard limit of 255"));

    let overflow = b"ok: 1\nbad: 340282366920938463463374607431768211456\n";
    let error = yaml::from_slice(overflow).unwrap_err();
    let position = match error {
        yggdryl::Error::Codec { position, .. } => position,
        other => panic!("unexpected error: {other}"),
    };
    assert_eq!(position, b"ok: 1\nbad: ".len());
}

#[test]
fn yaml_block_nesting_has_a_safe_conversion_ceiling() {
    fn block_sequence(depth: usize) -> Vec<u8> {
        let mut input = Vec::new();
        for level in 0..depth {
            input.extend(std::iter::repeat_n(b' ', level * 2));
            input.extend_from_slice(b"-\n");
        }
        input.extend(std::iter::repeat_n(b' ', depth * 2));
        input.extend_from_slice(b"0\n");
        input
    }

    let hard_limit = yaml::MAX_PARSER_DEPTH;
    let at_limit = block_sequence(hard_limit);
    assert!(
        yaml::from_slice_with_limits(
            &at_limit,
            Limits::new(hard_limit + 1, at_limit.len(), hard_limit + 1, 1),
        )
        .is_ok()
    );

    let over_limit = block_sequence(hard_limit + 1);
    let error = yaml::from_slice_with_limits(
        &over_limit,
        Limits::new(hard_limit + 100, over_limit.len(), hard_limit + 2, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("parser hard limit of 384"));
}

#[test]
fn documents_are_written_in_block_style_with_indented_nesting() {
    let value = Value::from_mapping([
        (Value::from("site_name"), Value::from("Yggdryl")),
        (
            Value::from("theme"),
            Value::from_mapping([
                (Value::from("name"), Value::from("material")),
                (
                    Value::from("features"),
                    Value::from_sequence([
                        Value::from("content.code.copy"),
                        Value::from("navigation.top"),
                    ]),
                ),
            ])
            .unwrap(),
        ),
        (
            Value::from("nav"),
            Value::from_sequence([Value::from_mapping([(
                Value::from("Home"),
                Value::from("index.md"),
            )])
            .unwrap()]),
        ),
    ])
    .unwrap();

    let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        text,
        "site_name: Yggdryl\n\
         theme:\n  \
           name: material\n  \
           features:\n    \
             - content.code.copy\n    \
             - navigation.top\n\
         nav:\n  \
           - Home: index.md\n"
    );
    assert_eq!(yaml::from_slice(text.as_bytes()).unwrap(), value);
}

#[test]
fn a_scalar_that_would_read_back_as_another_type_stays_quoted() {
    for spelling in [
        "yes", "no", "true", "null", "42", "1.5", "~", "on", "off", "<<",
    ] {
        let value = Value::from_mapping([(Value::from("key"), Value::from(spelling))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert!(
            text.contains(&format!("\"{spelling}\"")),
            "{spelling}: {text}"
        );
        assert_eq!(
            yaml::from_slice(text.as_bytes()).unwrap(),
            value,
            "{spelling}"
        );
    }

    // An ordinary string needs no quoting.
    let plain = Value::from_mapping([(Value::from("key"), Value::from("material"))]).unwrap();
    assert_eq!(
        String::from_utf8(yaml::to_vec(&plain).unwrap()).unwrap(),
        "key: material\n"
    );
}

#[test]
fn a_string_the_scanner_resolves_as_a_number_is_quoted_and_stays_a_string() {
    // Rust's own `FromStr` reads none of these spellings, so an emitter that
    // asked it instead of the scanner wrote them plain and read them back as
    // numbers. Only the scanner knows which spellings it resolves.
    for spelling in [
        ".inf", ".nan", ".NaN", "+.inf", "-.inf", "1_000.5", "1_0", "0x1F", "0X1f", "0o17", "0b101",
    ] {
        let value = Value::from_mapping([(Value::from("key"), Value::from(spelling))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(text, format!("key: \"{spelling}\"\n"), "{spelling}");
        assert_eq!(
            yaml::from_slice(text.as_bytes()).unwrap(),
            value,
            "{spelling}"
        );
    }

    // A key is written by the same guard, so it is quoted for the same reason.
    let key = Value::from_mapping([(Value::from("0x1F"), Value::from(1_u64))]).unwrap();
    let text = String::from_utf8(yaml::to_vec(&key).unwrap()).unwrap();
    assert_eq!(text, "\"0x1F\": 1\n");
    assert_eq!(yaml::from_slice(text.as_bytes()).unwrap(), key);

    // A spelling the scanner rejects outright is quoted too, or the document
    // it produced would not load at all.
    for spelling in ["0x_FF", "0b102"] {
        let value = Value::from_mapping([(Value::from("key"), Value::from(spelling))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(text, format!("key: \"{spelling}\"\n"), "{spelling}");
        assert_eq!(
            yaml::from_slice(text.as_bytes()).unwrap(),
            value,
            "{spelling}"
        );
    }
}

#[test]
fn a_string_the_scanner_reads_back_unchanged_is_still_written_plain() {
    // Quoting everything that merely looks wordy would make every document
    // noisier, so the scanner decides here as well.
    for spelling in [
        "material",
        "content.code.copy",
        "index.md",
        "1.5.2",
        "v1_0",
        "1__2",
        "release candidate",
    ] {
        let value = Value::from_mapping([(Value::from("key"), Value::from(spelling))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(text, format!("key: {spelling}\n"), "{spelling}");
        assert_eq!(
            yaml::from_slice(text.as_bytes()).unwrap(),
            value,
            "{spelling}"
        );
    }
}

#[test]
fn non_finite_floats_use_the_native_yaml_spellings_instead_of_an_envelope() {
    for (float, spelling) in [(f64::INFINITY, ".inf"), (f64::NEG_INFINITY, "-.inf")] {
        let value = Value::from_mapping([(Value::from("key"), Value::from(float))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(text, format!("key: {spelling}\n"));
        assert_eq!(yaml::from_slice(text.as_bytes()).unwrap(), value, "{text}");
    }

    // Not a number is never equal to itself, so identity is the only assertion
    // available on the way back.
    let value = Value::from_mapping([(Value::from("key"), Value::from(f64::NAN))]).unwrap();
    let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
    assert_eq!(text, "key: .nan\n");
    let decoded = yaml::from_slice(text.as_bytes()).unwrap();
    assert!(
        decoded
            .get_key_str("key")
            .and_then(Value::as_f64)
            .is_some_and(f64::is_nan),
        "{text}"
    );

    // The flow style inside an envelope carries the same native spelling. A
    // mapping that collides with the marker is escaped through the mapping
    // envelope, which is the one envelope carrying arbitrary payload values.
    let escaped = Value::from_mapping([
        (Value::from("$yggdryl"), Value::from("float")),
        (Value::from("value"), Value::from(f64::NEG_INFINITY)),
    ])
    .unwrap();
    let encoded = yaml::to_vec(&escaped).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(text.contains("\"$yggdryl\": \"mapping\""), "{text}");
    assert!(text.contains("[\"value\", -.inf]"), "{text}");
    assert_eq!(yaml::from_slice(&encoded).unwrap(), escaped);
}

#[test]
fn empty_collections_and_deep_nesting_stay_readable() {
    let value = Value::from_mapping([
        (Value::from("empty_list"), Value::from_sequence([])),
        (
            Value::from("empty_map"),
            Value::from_mapping(Vec::<(Value, Value)>::new()).unwrap(),
        ),
        (
            Value::from("deep"),
            Value::from_sequence([Value::from_sequence([Value::from(1_u64)])]),
        ),
    ])
    .unwrap();

    let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
    assert!(text.contains("empty_list: []"), "{text}");
    assert!(text.contains("empty_map: {}"), "{text}");
    assert!(text.contains("deep:\n  - - 1"), "{text}");
    assert_eq!(yaml::from_slice(text.as_bytes()).unwrap(), value);
}

mod numbers {
    use super::{Value, yaml};

    fn float(source: &str) -> f64 {
        yaml::from_str(source)
            .unwrap_or_else(|error| panic!("{source}: {error}"))
            .as_f64()
            .unwrap_or_else(|| panic!("{source} is not a float"))
    }

    fn reason(source: &str) -> String {
        match yaml::from_str(source).unwrap_err() {
            yggdryl::Error::Codec { format, reason, .. } => {
                assert_eq!(format, "yaml", "{source}");
                reason.to_string()
            }
            other => panic!("{source}: unexpected error: {other}"),
        }
    }

    #[test]
    fn an_explicit_float_tag_resolves_an_integer_spelling_as_a_float() {
        // The YAML 1.2 core float regex matches a spelling with neither a
        // fraction nor an exponent, so a tag that names the float type alone
        // decides the answer.
        for source in ["!!float 1", "!!float '1'", "!!float \"1\"", "!!float +1"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(1.0),
                "{source}"
            );
        }
        assert_eq!(yaml::from_str("!!float 0").unwrap(), Value::from(0.0));
        assert_eq!(yaml::from_str("!!float -2").unwrap(), Value::from(-2.0));
        assert_eq!(
            yaml::from_str("!!float 1_000").unwrap(),
            Value::from(1000.0)
        );
    }

    #[test]
    fn an_untagged_integer_spelling_is_still_an_integer() {
        assert_eq!(yaml::from_str("1").unwrap(), Value::U64(1));
        assert_eq!(yaml::from_str("0").unwrap(), Value::U64(0));
        assert_eq!(yaml::from_str("-2").unwrap(), Value::I64(-2));
        assert_eq!(
            yaml::from_str("value: 1\n").unwrap().get_key_str("value"),
            Some(&Value::U64(1))
        );

        // Grammar alone decides, so a fraction or an exponent is what makes an
        // untagged scalar a float.
        assert_eq!(yaml::from_str("1.0").unwrap(), Value::from(1.0));
        assert_eq!(yaml::from_str("1e3").unwrap(), Value::from(1000.0));
    }

    #[test]
    fn an_explicit_int_tag_still_rejects_a_float_spelling() {
        for source in ["!!int 1e3", "!!int 1.0", "!!int .inf", "!!int .nan"] {
            assert_eq!(reason(source), "invalid YAML integer", "{source}");
        }
    }

    #[test]
    fn a_finite_spelling_beyond_the_f64_range_is_an_error() {
        for source in [
            "1e400",
            "-1e400",
            "1.7976931348623159e308",
            "!!float 1e400",
            "!!float -1e400",
            "value: 1e400\n",
        ] {
            assert_eq!(
                reason(source),
                "YAML float is outside the finite f64 range",
                "{source}"
            );
        }
    }

    #[test]
    fn a_spelling_that_rounds_to_zero_keeps_its_sign() {
        // Underflow is ordinary IEEE-754 rounding that any conforming producer
        // may emit, so it is accepted rather than rejected.
        for source in ["1e-400", "0.0", "!!float 1e-400", "!!float 0"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(0.0),
                "{source}"
            );
            assert!(!float(source).is_sign_negative(), "{source}");
        }
        for source in ["-1e-400", "-0.0", "!!float -1e-400", "!!float -0.0"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(-0.0),
                "{source}"
            );
            assert!(float(source).is_sign_negative(), "{source}");
        }
    }

    #[test]
    fn the_yaml_spellings_of_infinity_and_not_a_number_still_resolve() {
        // Only a finite spelling that overflows is an error; these spellings
        // name a non-finite value outright and stay legitimate.
        for source in [".inf", "+.inf", ".INF", ".Inf", "!!float .inf"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(f64::INFINITY),
                "{source}"
            );
        }
        for source in ["-.inf", "-.INF", "!!float -.inf"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(f64::NEG_INFINITY),
                "{source}"
            );
        }
        for source in [".nan", ".NaN", ".NAN", "!!float .nan", "!!float -.nan"] {
            assert!(float(source).is_nan(), "{source}");
        }
    }

    #[test]
    fn the_rust_spellings_of_infinity_and_not_a_number_are_not_yaml_floats() {
        for source in ["inf", "-inf", "infinity", "nan", "NaN"] {
            assert_eq!(
                yaml::from_str(source).unwrap(),
                Value::from(source),
                "{source}"
            );
        }
        for source in ["!!float inf", "!!float nan", "!!float 0x10"] {
            assert_eq!(reason(source), "invalid YAML float", "{source}");
        }
    }

    #[test]
    fn a_string_holding_an_overflowing_spelling_still_survives_a_round_trip() {
        // The scanner rejects `1e400` rather than resolving it, so the emitter
        // has to quote it: written plain, the document would no longer load.
        let value = Value::from_mapping([(Value::from("key"), Value::from("1e400"))]).unwrap();
        let text = String::from_utf8(yaml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(yaml::from_slice(text.as_bytes()).unwrap(), value, "{text}");
    }
}
