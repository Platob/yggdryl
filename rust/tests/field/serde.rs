//! The `Scalar` conversion is the one structural model of a schema, and every
//! serialized form is expressed over it.

use yggdryl::generic::Scalar;
use yggdryl::{DataType, Field, Metadata, TimeUnit};

/// One representative field per shape the model can carry.
fn shapes() -> Vec<Field> {
    let nested = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([
            DataType::from_fields([DataType::Utf8.nullable_field("leaf")])
                .unwrap()
                .nullable_field("inner"),
        ])
        .unwrap()
        .required_field("middle"),
    ])
    .unwrap()
    .required_field("deep");

    let mut with_metadata =
        Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")]).unwrap();
    with_metadata.set_parquet_field_id(17);

    let mut dictionary = Field::new(
        "status",
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        true,
    );
    dictionary.set_dictionary_options(42, true).unwrap();

    let partitioned = DataType::from_fields([
        DataType::Utf8.required_field("venue").with_partition(true),
        DataType::Int64.required_field("id"),
    ])
    .unwrap()
    .required_field("row");

    vec![
        DataType::Int64.required_field("flat"),
        nested,
        DataType::list(
            DataType::from_fields([DataType::Int64.required_field("id")])
                .unwrap()
                .nullable_field("item"),
        )
        .nullable_field("rows"),
        DataType::map_of(DataType::Utf8, DataType::Int64, false)
            .unwrap()
            .nullable_field("counts"),
        DataType::union(
            [
                (0_i8, DataType::Int64.nullable_field("number")),
                (1_i8, DataType::Utf8.nullable_field("text")),
            ],
            yggdryl::UnionMode::Dense,
        )
        .unwrap()
        .nullable_field("either"),
        dictionary,
        with_metadata,
        partitioned,
        DataType::decimal128(38, 6)
            .unwrap()
            .nullable_field("amount"),
        DataType::Timestamp(TimeUnit::Microsecond, Some("Europe/Paris".parse().unwrap()))
            .nullable_field("at"),
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Utf8.nullable_field("values"),
        )
        .unwrap()
        .nullable_field("runs"),
        DataType::fixed_size_binary(16)
            .unwrap()
            .nullable_field("uuid"),
        DataType::Null.nullable_field("nothing"),
        DataType::variant().nullable_field("payload"),
        DataType::geometry(None).unwrap().nullable_field("shape"),
        DataType::geography(Some("EPSG:4326"), Some(yggdryl::EdgeAlgorithm::Vincenty))
            .unwrap()
            .required_field("region"),
    ]
}

#[test]
fn every_shape_round_trips_through_the_value_conversion() {
    for field in shapes() {
        let restored = Field::from_value(field.clone().into_value()).expect("a field mapping");
        assert_eq!(restored, field, "{field}");
        assert_eq!(
            restored.as_metadata(),
            field.as_metadata(),
            "metadata survives {field}"
        );

        let dtype = field.dtype().clone();
        let restored =
            DataType::from_value(dtype.clone().into_value()).expect("a datatype mapping");
        assert_eq!(restored, dtype, "{dtype}");
    }
}

#[test]
fn the_value_shape_matches_the_json_structure_exactly() {
    for field in shapes() {
        // The JSON path is expressed over this conversion, so the two must
        // produce the same bytes - this is the compatibility test that
        // pins `into_json` against drift.
        let direct = field.clone().into_json().expect("structural JSON");
        let through_value = String::from_utf8(
            yggdryl::json::into_bytes(&field.clone().into_value()).expect("a JSON dump"),
        )
        .expect("UTF-8");
        assert_eq!(direct, through_value, "{field}");

        let dtype = field.dtype();
        let direct = dtype.clone().into_json().expect("structural JSON");
        let through_value = String::from_utf8(
            yggdryl::json::into_bytes(&dtype.clone().into_value()).expect("a JSON dump"),
        )
        .expect("UTF-8");
        assert_eq!(direct, through_value, "{dtype}");
    }
}

#[test]
fn a_malformed_mapping_is_refused_by_path() {
    let missing =
        Scalar::from_mapping([(Scalar::String("name".into()), Scalar::String("id".into()))])
            .unwrap();
    let refused = Field::from_value(missing).expect_err("no datatype");
    assert!(refused.to_string().contains("dtype"), "{refused}");

    let unknown = Scalar::from_mapping([(
        Scalar::String("type".into()),
        Scalar::String("quaternion".into()),
    )])
    .unwrap();
    let refused = DataType::from_value(unknown).expect_err("no such datatype");
    assert!(refused.to_string().contains("quaternion"), "{refused}");

    let not_a_mapping = Scalar::String("int64".into());
    let refused = DataType::from_value(not_a_mapping).expect_err("not a mapping");
    assert!(
        refused.to_string().contains("datatype mapping"),
        "{refused}"
    );
}

#[test]
fn metadata_entries_must_be_strings() {
    let field = Field::new("id", DataType::Int64, false);
    let mut value = field.into_value().as_mapping().unwrap().to_vec();
    value.pop();
    value.push((
        Scalar::String("metadata".into()),
        Scalar::from_mapping([(Scalar::String("count".into()), Scalar::I64(7))]).unwrap(),
    ));
    let refused = Field::from_value(Scalar::from_mapping(value).unwrap())
        .expect_err("a non-string metadata value");
    assert!(refused.to_string().contains("count"), "{refused}");
}

#[test]
fn the_trait_forms_sit_beside_the_inherent_ones() {
    let field = DataType::Int64.required_field("id");
    let value: Scalar = (&field).into();
    assert_eq!(Field::try_from(value).unwrap(), field);

    let dtype = DataType::Utf8;
    let value: Scalar = (&dtype).into();
    assert_eq!(DataType::try_from(value).unwrap(), dtype);

    // Metadata keeps its own entries type; this is only the schema route.
    assert!(Metadata::new().is_empty());
}

/// One representative small field, for literal-text assertions.
fn small() -> Field {
    DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row")
}

#[test]
fn every_shape_round_trips_through_every_format() {
    for field in shapes() {
        assert_eq!(
            Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
            field
        );
        assert_eq!(
            Field::from_yaml(&field.clone().into_yaml().unwrap()).unwrap(),
            field
        );
        assert_eq!(
            Field::from_toml(&field.clone().into_toml().unwrap()).unwrap(),
            field
        );

        let dtype = field.dtype().clone();
        assert_eq!(
            DataType::from_json(&dtype.clone().into_json().unwrap()).unwrap(),
            dtype
        );
        assert_eq!(
            DataType::from_yaml(&dtype.clone().into_yaml().unwrap()).unwrap(),
            dtype
        );
        assert_eq!(
            DataType::from_toml(&dtype.clone().into_toml().unwrap()).unwrap(),
            dtype
        );
    }
}

#[test]
fn natural_text_objects_feed_record_aware_structural_readers() {
    let field = small();
    let documents = [
        yggdryl::json::from_utf8(&field.clone().into_json().unwrap()).unwrap(),
        yggdryl::yaml::from_utf8(&field.clone().into_yaml().unwrap()).unwrap(),
        yggdryl::toml::from_utf8(&field.clone().into_toml().unwrap()).unwrap(),
    ];

    for document in documents {
        assert!(document.as_record().is_some(), "{document:?}");
        let dtype = document.get_key_str("dtype").unwrap().clone();
        assert!(dtype.as_record().is_some(), "{dtype:?}");
        assert_eq!(DataType::from_value(dtype).unwrap(), field.dtype().clone());
        assert_eq!(Field::from_value(document).unwrap(), field);
    }
}

#[test]
fn the_three_formats_parse_back_to_equal_values() {
    // Cross-format agreement is by construction - one `Scalar` mapping feeds
    // all three - and this is the test that pins it.
    for field in shapes() {
        let from_json = Field::from_json(&field.clone().into_json().unwrap()).unwrap();
        let from_yaml = Field::from_yaml(&field.clone().into_yaml().unwrap()).unwrap();
        let from_toml = Field::from_toml(&field.clone().into_toml().unwrap()).unwrap();
        assert_eq!(from_json, from_yaml, "{field}");
        assert_eq!(from_yaml, from_toml, "{field}");
    }
}

#[test]
fn a_small_dump_reads_the_way_the_docs_say() {
    // Literal text, so a formatting regression fails a test rather than
    // passing unnoticed.
    let field = small();

    assert_eq!(
        field.clone().into_json().unwrap(),
        r#"{"name":"row","dtype":{"type":"struct","fields":[{"name":"id","dtype":{"type":"int64"},"nullable":false,"metadata":{}}]},"nullable":false,"metadata":{}}"#
    );

    assert_eq!(
        field.clone().into_yaml().unwrap(),
        "\
name: row
dtype:
  type: struct
  fields:
    - name: id
      dtype:
        type: int64
      nullable: false
      metadata: {}
nullable: false
metadata: {}
"
    );

    assert_eq!(
        field.into_toml().unwrap(),
        "\
\"name\" = \"row\"
\"dtype\" = {\"type\" = \"struct\", \"fields\" = [{\"name\" = \"id\", \"dtype\" = {\"type\" = \"int64\"}, \"nullable\" = false, \"metadata\" = {}}]}
\"nullable\" = false
\"metadata\" = {}
"
    );
}

#[test]
fn formatting_changes_bytes_never_meaning() {
    use yggdryl::text::{Formatting, Indent};

    let field = small();
    let settings = [
        Formatting::default(),
        Formatting::compact(),
        Formatting::indented(2),
        Formatting::indented(4),
        Formatting::default().with_indent(Indent::Tabs),
    ];

    for formatting in settings {
        for (dump, parse) in [
            (
                Box::new(move |f: &Field| f.clone().into_json_with_formatting(formatting))
                    as Box<dyn Fn(&Field) -> yggdryl::Result<String>>,
                Box::new(Field::from_json) as Box<dyn Fn(&str) -> yggdryl::Result<Field>>,
            ),
            (
                Box::new(move |f: &Field| f.clone().into_yaml_with_formatting(formatting)),
                Box::new(Field::from_yaml),
            ),
            (
                Box::new(move |f: &Field| f.clone().into_toml_with_formatting(formatting)),
                Box::new(Field::from_toml),
            ),
        ] {
            let text = dump(&field).unwrap();
            // Round-trip: parsing any formatting yields an equal value.
            let restored = parse(&text).unwrap();
            assert_eq!(restored, field, "{text}");
            // Idempotence: dumping again is byte-identical.
            assert_eq!(dump(&restored).unwrap(), text);
        }
    }
}

#[test]
fn indentation_reads_literally_in_every_format() {
    use yggdryl::text::Formatting;

    let value = yggdryl::generic::Scalar::from_mapping([
        (
            yggdryl::generic::Scalar::String("id".into()),
            yggdryl::generic::Scalar::I64(1),
        ),
        (
            yggdryl::generic::Scalar::String("tags".into()),
            yggdryl::generic::Scalar::from_sequence([yggdryl::generic::Scalar::String("a".into())]),
        ),
    ])
    .unwrap();

    assert_eq!(
        yggdryl::json::into_bytes(&value).unwrap(),
        br#"{"id":1,"tags":["a"]}"#
    );
    assert_eq!(
        yggdryl::json::into_bytes_with_formatting(&value, Formatting::indented(2)).unwrap(),
        b"{\n  \"id\": 1,\n  \"tags\": [\n    \"a\"\n  ]\n}"
    );
    assert_eq!(
        yggdryl::json::into_bytes_with_formatting(&value, Formatting::indented(4)).unwrap(),
        b"{\n    \"id\": 1,\n    \"tags\": [\n        \"a\"\n    ]\n}"
    );

    assert_eq!(
        yggdryl::yaml::into_bytes(&value).unwrap(),
        b"id: 1\ntags:\n  - a\n"
    );
    assert_eq!(
        yggdryl::yaml::into_bytes_with_formatting(&value, Formatting::indented(4)).unwrap(),
        b"id: 1\ntags:\n    - a\n"
    );
    assert_eq!(
        yggdryl::yaml::into_bytes_with_formatting(&value, Formatting::compact()).unwrap(),
        b"{id: 1, tags: [a]}\n"
    );

    assert_eq!(
        yggdryl::toml::into_bytes(&value).unwrap(),
        b"\"id\" = 1\n\"tags\" = [\"a\"]\n"
    );
    assert_eq!(
        yggdryl::toml::into_bytes_with_formatting(&value, Formatting::indented(2)).unwrap(),
        b"\"id\" = 1\n\"tags\" = [\n  \"a\",\n]\n"
    );
}

#[test]
fn depth_three_indents_one_level_per_level() {
    use yggdryl::text::Formatting;

    let deep = DataType::from_fields([DataType::from_fields([DataType::from_fields([
        DataType::Int64.required_field("leaf"),
    ])
    .unwrap()
    .required_field("inner")])
    .unwrap()
    .required_field("middle")])
    .unwrap()
    .required_field("outer");

    let yaml = deep
        .clone()
        .into_yaml_with_formatting(Formatting::indented(2))
        .unwrap();
    // One unit deeper per level, with no drift at depth.
    let columns: Vec<usize> = yaml
        .lines()
        .filter(|line| line.trim() == "type: struct")
        .map(|line| line.len() - line.trim_start().len())
        .collect();
    assert_eq!(columns.len(), 3, "{yaml}");
    assert_eq!(columns[1] - columns[0], columns[2] - columns[1], "{yaml}");
    assert!(columns[0] < columns[1], "{yaml}");

    let wide = deep
        .clone()
        .into_yaml_with_formatting(Formatting::indented(4))
        .unwrap();
    let wide_columns: Vec<usize> = wide
        .lines()
        .filter(|line| line.trim() == "type: struct")
        .map(|line| line.len() - line.trim_start().len())
        .collect();
    // A wider unit is strictly deeper and still drift-free. It is not simply
    // double: a level of this schema is one key indent plus one `- ` marker,
    // and the marker is always two columns because that is what YAML requires
    // for a sequence entry's continuation lines whatever the unit is.
    assert_eq!(wide_columns.len(), columns.len(), "{wide}");
    assert_eq!(
        wide_columns[1] - wide_columns[0],
        wide_columns[2] - wide_columns[1],
        "{wide}"
    );
    assert!(
        wide_columns[1] - wide_columns[0] > columns[1] - columns[0],
        "{wide}"
    );

    // JSON indents strictly by level, so the `fields` key at each depth sits
    // exactly one unit deeper than the one above it - no drift at depth.
    let json = deep
        .clone()
        .into_json_with_formatting(Formatting::indented(2))
        .unwrap();
    let json_columns: Vec<usize> = json
        .lines()
        .filter(|line| line.trim_start().starts_with("\"fields\""))
        .map(|line| line.len() - line.trim_start().len())
        .collect();
    assert_eq!(json_columns.len(), 3, "{json}");
    assert_eq!(
        json_columns[1] - json_columns[0],
        json_columns[2] - json_columns[1]
    );

    let wide_json = deep
        .into_json_with_formatting(Formatting::indented(4))
        .unwrap();
    let wide_json_columns: Vec<usize> = wide_json
        .lines()
        .filter(|line| line.trim_start().starts_with("\"fields\""))
        .map(|line| line.len() - line.trim_start().len())
        .collect();
    assert_eq!(
        wide_json_columns[1] - wide_json_columns[0],
        2 * (json_columns[1] - json_columns[0]),
        "{wide_json}"
    );
}

#[test]
fn the_compact_form_is_unchanged_and_still_parses_back() {
    // The readable form is an addition; `Display` is untouched, because it
    // round-trips through the parsers and `__repr__` depends on it.
    for field in shapes() {
        let compact = field.to_string();
        assert!(!compact.contains('\n'), "{compact}");
        assert_eq!(Field::from_str(&compact).unwrap(), field);

        let dtype = field.dtype();
        let compact = dtype.to_string();
        assert!(!compact.contains('\n'), "{compact}");
        assert_eq!(DataType::from_str(&compact).unwrap(), *dtype);
    }
}

#[test]
fn the_readable_form_indents_by_depth_and_omits_unset_attributes() {
    let mut field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])
            .unwrap()
            .nullable_field("line"),
        DataType::list(DataType::Utf8.nullable_field("tag")).nullable_field("tags"),
    ])
    .unwrap()
    .required_field("order");
    field.insert_metadata("owner", "trading").unwrap();

    assert_eq!(
        format!("{field:#}"),
        "\
order: struct[3], required
  @owner = trading
  id: int64, required
  line: struct[1], nullable
    price: float64, required
  tags: list, nullable
    tag: utf8, nullable"
    );

    // `{:#}` and the named adapter are one implementation.
    assert_eq!(format!("{field:#}"), field.pretty().to_string());

    // Unset attributes are absent: no dictionary_id=0, no empty metadata blob.
    let plain = DataType::Int64.required_field("id");
    assert_eq!(format!("{plain:#}"), "id: int64, required");
    assert!(!format!("{plain:#}").contains("dictionary"));
    assert!(!format!("{plain:#}").contains("metadata"));

    // A set one is shown.
    let mut dictionary = DataType::dictionary(DataType::Int16, DataType::Utf8)
        .unwrap()
        .nullable_field("status");
    dictionary.set_dictionary_options(42, true).unwrap();
    let readable = format!("{dictionary:#}");
    assert!(readable.contains("dictionary_id=42"), "{readable}");
    assert!(readable.contains("dictionary_is_ordered"), "{readable}");
}

#[test]
fn the_readable_form_is_stable_across_runs() {
    // Nothing here iterates a hash map, so two renderings of one value - and
    // of two equal values built independently - agree exactly.
    let build = || {
        let mut field = DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row");
        for (key, value) in [("z", "last"), ("a", "first"), ("m", "middle")] {
            field.insert_metadata(key, value).unwrap();
        }
        field
    };
    let first = build();
    let second = build();
    assert_eq!(format!("{first:#}"), format!("{second:#}"));
    assert_eq!(format!("{first:#}"), format!("{first:#}"));
    // Metadata renders as indented lines in key order, not one braced blob.
    assert!(format!("{first:#}").contains("\n  @a = first\n  @m = middle\n  @z = last\n"));
}
