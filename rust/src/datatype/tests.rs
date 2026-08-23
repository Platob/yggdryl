//! Focused regression tests for the datatype implementation modules.
use std::collections::{BTreeSet, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

use super::{
    DataType, DictionaryType, Fields, MapType, RunEndEncodedType, TimeUnit, UnionFields, UnionMode,
};
use crate::{Error, Field};

#[test]
fn nested_helper_values_have_total_order_and_hash() {
    fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
    assert_traits::<MapType>();
    assert_traits::<RunEndEncodedType>();

    let first = DataType::map_of(DataType::Utf8, DataType::Int32, false).unwrap();
    let later = DataType::map_of(DataType::Utf8, DataType::Int32, true).unwrap();
    let (DataType::Map(first), DataType::Map(later)) = (first, later) else {
        unreachable!()
    };
    assert!(first < later);

    let first = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Int32, true),
    )
    .unwrap();
    let later = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Int64, true),
    )
    .unwrap();
    let (DataType::RunEndEncoded(first), DataType::RunEndEncoded(later)) = (first, later) else {
        unreachable!()
    };
    assert!(first < later);
}

#[test]
fn canonical_display_json_and_arrow_are_lossless() {
    let item = Field::from_parts(
        "item,東京",
        DataType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::from_parts(
                "text",
                DataType::Utf8,
                true,
                [("source", "quoted \"value\"")],
            )
            .unwrap(),
        ])
        .unwrap(),
        true,
        [("doc", "nested, metadata")],
    )
    .unwrap();
    let value = DataType::list(item);

    let canonical = value.to_string();
    assert_eq!(DataType::from_str(&canonical).unwrap(), value);
    let json = value.clone().into_json().unwrap();
    let structural: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(structural.is_object());
    assert_eq!(structural["type"], "list");
    assert_eq!(structural["field"]["data_type"]["type"], "struct");
    assert_eq!(DataType::from_json(&json).unwrap(), value);
    assert_eq!(
        DataType::from_json(&value.clone().into_json().unwrap()).unwrap(),
        value
    );

    let arrow = value.clone().into_arrow().unwrap();
    assert_eq!(DataType::from_arrow(&arrow).unwrap(), value);
    assert_eq!(
        DataType::try_from(value.clone().into_arrow().unwrap()).unwrap(),
        value
    );
}

#[test]
fn structural_json_rejects_malformed_and_duplicate_values() {
    assert_eq!(DataType::Int64.into_json().unwrap(), r#"{"type":"int64"}"#);
    assert_eq!(
        DataType::decimal128(38, 6).unwrap().into_json().unwrap(),
        r#"{"type":"decimal128","precision":38,"scale":6}"#
    );

    let field = |name: &str, data_type: serde_json::Value, nullable: bool| {
        serde_json::json!({
            "name": name,
            "data_type": data_type,
            "nullable": nullable,
            "metadata": {}
        })
    };

    let malformed = [
        serde_json::json!({"type": "time32", "unit": "nanosecond"}),
        serde_json::json!({"type": "fixed_size_binary", "width": -1}),
        serde_json::json!({"type": "decimal32", "precision": 10, "scale": 0}),
        serde_json::json!({
            "type": "struct",
            "fields": [
                field("same", serde_json::json!({"type": "int32"}), false),
                field("same", serde_json::json!({"type": "utf8"}), true)
            ]
        }),
        serde_json::json!({
            "type": "union",
            "mode": "dense",
            "fields": [
                {"type_id": 1, "field": field("one", serde_json::json!({"type": "int32"}), false)},
                {"type_id": 1, "field": field("two", serde_json::json!({"type": "utf8"}), true)}
            ]
        }),
        serde_json::json!({
            "type": "dictionary",
            "key": {"type": "float64"},
            "value": {"type": "utf8"}
        }),
        serde_json::json!({
            "type": "map",
            "entries": field(
                "entries",
                serde_json::json!({
                    "type": "struct",
                    "fields": [
                        field("key", serde_json::json!({"type": "utf8"}), false),
                        field("value", serde_json::json!({"type": "int64"}), true)
                    ]
                }),
                true
            ),
            "keys_sorted": false
        }),
        serde_json::json!({
            "type": "run_end_encoded",
            "run_ends": field("run_ends", serde_json::json!({"type": "int32"}), true),
            "values": field("values", serde_json::json!({"type": "utf8"}), true)
        }),
        serde_json::json!({"type": "int64", "unexpected": true}),
    ];

    for value in malformed {
        assert!(
            serde_json::from_value::<DataType>(value.clone()).is_err(),
            "accepted malformed structural datatype: {value}"
        );
    }

    let duplicate_metadata = r#"{
            "type":"list",
            "field":{
                "name":"item",
                "data_type":{"type":"utf8"},
                "nullable":true,
                "metadata":{"source":"one","source":"two"}
            }
        }"#;
    assert!(DataType::from_json(duplicate_metadata).is_err());
}

#[test]
fn nested_serde_and_core_validators_keep_distinct_error_contracts() {
    let dictionary = serde_json::json!({
        "key": {"type": "float64"},
        "value": {"type": "utf8"}
    });
    assert_eq!(
        serde_json::from_value::<DictionaryType>(dictionary)
            .unwrap_err()
            .to_string(),
        "invalid Dictionary datatype: expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got float64"
    );
    assert_eq!(
        DataType::dictionary(DataType::Float64, DataType::Utf8)
            .unwrap_err()
            .to_string(),
        "invalid Dictionary datatype: expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got float64"
    );

    let field = |name: &str, data_type: serde_json::Value, nullable: bool| {
        serde_json::json!({
            "name": name,
            "data_type": data_type,
            "nullable": nullable,
            "metadata": {}
        })
    };
    let run_end = serde_json::json!({
        "run_ends": field("run_ends", serde_json::json!({"type": "int32"}), true),
        "values": field("values", serde_json::json!({"type": "utf8"}), true)
    });
    assert_eq!(
        serde_json::from_value::<RunEndEncodedType>(run_end)
            .unwrap_err()
            .to_string(),
        "invalid RunEndEncoded datatype: expected a non-null run_ends field, got nullable field \"run_ends\""
    );
    assert_eq!(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, true),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap_err()
        .to_string(),
        "invalid RunEndEncoded datatype: expected a non-null run_ends field, got nullable field \"run_ends\""
    );

    // The two run_ends rules are independent, so each reports the half that
    // actually failed rather than one fused sentence.
    assert_eq!(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Utf8, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap_err()
        .to_string(),
        "invalid RunEndEncoded datatype: expected a run_ends datatype of int16, int32, or int64, got utf8"
    );
}

#[test]
fn structural_serialization_rejects_public_enum_invalid_states() {
    let invalid = [
        DataType::Time32(TimeUnit::Nanosecond),
        DataType::FixedSizeBinary(-1),
        DataType::Decimal128 {
            precision: 0,
            scale: 0,
        },
    ];

    for value in invalid {
        assert!(
            serde_json::to_string(&value).is_err(),
            "serialized {value:?}"
        );
        assert!(value.clone().into_json().is_err(), "serialized {value:?}");
    }
}

#[test]
fn sql_hive_spark_and_arrow_spellings_parse_recursively() {
    let values = [
        "ROW(id INTEGER NOT NULL, payload VARBINARY, score DOUBLE PRECISION)",
        "struct<`quoted,name`:string,nested:map<string,array<decimal(38,18)>>>",
        "string[][]",
        "Dictionary(UInt16, List(Field { name: 'item', data_type: Utf8, nullable: true, metadata: {} }))",
        "Union([(0, Field { name: 'id', data_type: Int64, nullable: false, metadata: {} }), (7, Field { name: 'name', data_type: Utf8, nullable: true, metadata: {} })], Dense)",
        "run_end_encoded(int32,array<string>)",
        "fixed_size_list(string,16)",
    ];

    for source in values {
        let value = DataType::from_str(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(DataType::from_str(&value.to_string()).unwrap(), value);
    }
}

#[test]
fn temporal_decimal_and_wrapper_forms_are_validated() {
    assert_eq!(
        DataType::from_str("timestamp(9,'Europe/Paris')").unwrap(),
        DataType::Timestamp(
            TimeUnit::Nanosecond,
            Some(crate::Timezone::from_str("Europe/Paris").unwrap())
        )
    );
    assert_eq!(
        DataType::from_str("TIMESTAMP WITH TIME ZONE").unwrap(),
        DataType::Timestamp(TimeUnit::Microsecond, Some(crate::Timezone::UTC))
    );
    assert_eq!(
        DataType::from_str("interval year to month").unwrap(),
        DataType::Interval(TimeUnit::YearMonth)
    );
    assert_eq!(
        DataType::from_str("[{(decimal256(76,-20))}]").unwrap(),
        DataType::decimal256(76, -20).unwrap()
    );
    assert!(DataType::from_str("time32(ns)").is_err());
    assert!(DataType::from_str("decimal32(10,0)").is_err());
    assert!(DataType::from_str("timestamp(10)").is_err());
}

#[test]
fn parser_reports_positions_and_rejects_adversarial_input() {
    for source in [
        "struct<a:int,a:string>",
        "map<string>",
        "array<struct<a:int>",
        "int64 trailing",
        "union(dense,0=a:int,0=b:string)",
        "'unterminated",
        "decimal(18,wat)",
    ] {
        assert!(
            matches!(
                DataType::from_str(source),
                Err(Error::Parse { .. }) | Err(Error::InvalidDataType { .. })
            ),
            "{source}"
        );
    }

    let mut overdeep = "int64".to_owned();
    for _ in 0..=DataType::PARSE_RECURSION_LIMIT {
        overdeep = format!("array<{overdeep}>");
    }
    assert!(matches!(
        DataType::from_str(&overdeep),
        Err(Error::Parse { position, .. }) if position > 0
    ));
}

#[test]
#[allow(clippy::mutable_key_type)] // Field caches are excluded from all value traits.
fn native_order_hash_and_child_access_are_value_based() {
    let left = DataType::from_str("struct<a:int32,b:string>").unwrap();
    let right = DataType::from_str("struct<a:int64,b:string>").unwrap();
    let ordered = BTreeSet::from([right.clone(), left.clone()]);
    let hashed = HashSet::from([right.clone(), left.clone()]);

    assert_eq!(ordered.len(), 2);
    assert_eq!(hashed.len(), 2);
    assert_ne!(left.stable_hash(), right.stable_hash());
    assert_eq!(left.field_len(), 2);
    assert_eq!(left.get_field(0).map(Field::name), Some("a"));
    assert_eq!(left.get_field_by_name("b").map(Field::name), Some("b"));
    assert_eq!(left.as_fields().map(<[Field]>::len), Some(2));
}

#[test]
fn every_arrow_variant_has_a_lossless_owned_equivalent() {
    let item = || Field::new("item", DataType::Utf8, true);
    let values = vec![
        DataType::Null,
        DataType::Boolean,
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
        DataType::Float16,
        DataType::Float32,
        DataType::Float64,
        DataType::Timestamp(
            TimeUnit::Nanosecond,
            Some(crate::Timezone::from_str("Europe/Paris").unwrap()),
        ),
        DataType::Date32,
        DataType::Date64,
        DataType::time32(TimeUnit::Millisecond).unwrap(),
        DataType::time64(TimeUnit::Microsecond).unwrap(),
        DataType::Duration64(TimeUnit::Nanosecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::Binary,
        DataType::fixed_size_binary(16).unwrap(),
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::list(item()),
        DataType::list_view(item()),
        DataType::fixed_size_list(item(), 4).unwrap(),
        DataType::large_list(item()),
        DataType::large_list_view(item()),
        DataType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
        DataType::union(
            [
                (0, Field::new("number", DataType::Int64, false)),
                (1, Field::new("text", DataType::Utf8, true)),
            ],
            UnionMode::Sparse,
        )
        .unwrap(),
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, -2).unwrap(),
        DataType::decimal128(38, 18).unwrap(),
        DataType::decimal256(76, 20).unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
    ];

    for source in values {
        let arrow: ArrowDataType = source.clone().into_arrow().unwrap();
        let arrow_debug = format!("{arrow:?}");
        let parsed_debug = DataType::from_str(&arrow_debug)
            .unwrap_or_else(|error| panic!("failed to parse {arrow_debug}: {error}"));
        assert_eq!(
            parsed_debug, source,
            "failed Arrow Debug parse: {arrow_debug}"
        );
        let restored = DataType::try_from(arrow).unwrap();
        assert_eq!(restored, source, "failed round-trip for {source}");
    }
}

#[test]
fn arrow_import_preserves_nested_field_projection_arcs() {
    let inner = Arc::new(ArrowField::new("item", ArrowDataType::Utf8, true));
    let outer = Arc::new(ArrowField::new(
        "item",
        ArrowDataType::List(Arc::clone(&inner)),
        true,
    ));
    let arrow = ArrowDataType::List(Arc::clone(&outer));

    let borrowed = DataType::from_arrow(&arrow).unwrap();
    let borrowed_outer = borrowed.get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &borrowed_outer.clone().into_arrow_ref().unwrap(),
        &outer
    ));
    let borrowed_inner = borrowed_outer.data_type().get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &borrowed_inner.clone().into_arrow_ref().unwrap(),
        &inner
    ));

    let owned = DataType::try_from(arrow).unwrap();
    let owned_outer = owned.get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &owned_outer.clone().into_arrow_ref().unwrap(),
        &outer
    ));
    let owned_inner = owned_outer.data_type().get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &owned_inner.clone().into_arrow_ref().unwrap(),
        &inner
    ));
}

#[test]
fn long_timezones_share_heap_storage_across_arrow_conversions() {
    let timezone: Arc<str> = Arc::from("America/Argentina/Buenos_Aires");
    let expected = timezone.as_ptr();
    let arrow = ArrowDataType::Timestamp(
        arrow_schema::TimeUnit::Nanosecond,
        Some(Arc::clone(&timezone)),
    );

    let borrowed = DataType::from_arrow(&arrow).unwrap();
    let DataType::Timestamp(_, Some(borrowed_timezone)) = &borrowed else {
        panic!("timestamp import changed variant");
    };
    assert_eq!(borrowed_timezone.as_str().as_ptr(), expected);

    let borrowed_arrow = borrowed.into_arrow().unwrap();
    let ArrowDataType::Timestamp(_, Some(borrowed_arrow_timezone)) = borrowed_arrow else {
        panic!("timestamp projection changed variant");
    };
    assert_eq!(borrowed_arrow_timezone.as_ptr(), expected);

    let owned = DataType::try_from(arrow).unwrap();
    let DataType::Timestamp(_, Some(owned_timezone)) = &owned else {
        panic!("timestamp import changed variant");
    };
    assert_eq!(owned_timezone.as_str().as_ptr(), expected);

    let owned_arrow = owned.into_arrow().unwrap();
    let ArrowDataType::Timestamp(_, Some(owned_arrow_timezone)) = owned_arrow else {
        panic!("timestamp projection changed variant");
    };
    assert_eq!(owned_arrow_timezone.as_ptr(), expected);
}

#[test]
fn arrow_import_enforces_one_shared_recursion_budget() {
    fn nested_list(levels: usize) -> ArrowDataType {
        let mut value = ArrowDataType::Int64;
        for _ in 0..levels {
            value = ArrowDataType::List(Arc::new(ArrowField::new("item", value, true)));
        }
        value
    }

    let maximum = nested_list(DataType::PARSE_RECURSION_LIMIT - 1);
    assert!(DataType::from_arrow(&maximum).is_ok());
    assert!(DataType::try_from(maximum).is_ok());

    let over_limit = nested_list(DataType::PARSE_RECURSION_LIMIT);
    assert!(DataType::from_arrow(&over_limit).is_err());
    assert!(DataType::try_from(over_limit).is_err());
}

#[test]
fn public_field_collections_validate_children_without_clone_helpers() {
    let invalid = Field::new("invalid", DataType::Time32(TimeUnit::Nanosecond), false);
    assert!(Fields::from_fields([invalid.clone()]).is_err());
    assert!(UnionFields::from_fields([(0, invalid)]).is_err());
}

/// The three v3-era datatypes: variant, geometry, and geography.
mod semi_structured_and_geospatial {
    use super::super::{DataType, GeospatialType};
    use crate::enums::{DataTypeId, DataTypeKind, EdgeAlgorithm};
    use crate::{Field, Value};

    #[test]
    fn bare_variant_and_the_member_sugar_parse_to_different_types() {
        // The parenthesis disambiguates, deterministically, in one branch:
        // bare `variant` is the self-describing datatype, and the member form
        // stays the dense-union input sugar. Side by side, so the
        // disambiguation can never be broken silently.
        let bare: DataType = "variant".parse().unwrap();
        assert_eq!(bare, DataType::Variant);

        let sugar: DataType = "variant(a: int32, b: utf8)".parse().unwrap();
        assert_eq!(sugar.id(), DataTypeId::Union);
        assert!(sugar.to_string().starts_with("union(dense,"), "{sugar}");

        // The union spelling never round-trips as `variant`, so nothing that
        // reads canonical text is affected by the new bare meaning.
        let redisplayed: DataType = sugar.to_string().parse().unwrap();
        assert_eq!(redisplayed, sugar);
    }

    #[test]
    fn every_spelling_of_the_geospatial_pair_parses_and_round_trips() {
        for (spelling, canonical) in [
            ("variant", "variant"),
            ("geometry", "geometry"),
            ("geometry()", "geometry"),
            ("geometry('OGC:CRS84')", "geometry"),
            ("geometry('EPSG:4326')", "geometry(\"EPSG:4326\")"),
            ("geography", "geography"),
            ("geography('OGC:CRS84')", "geography"),
            ("geography('OGC:CRS84', 'spherical')", "geography"),
            (
                "geography('OGC:CRS84', 'vincenty')",
                "geography(\"OGC:CRS84\",\"vincenty\")",
            ),
            ("GEOMETRY('EPSG:4326')", "geometry(\"EPSG:4326\")"),
            (
                "geography('EPSG:4326', 'KARNEY')",
                "geography(\"EPSG:4326\",\"karney\")",
            ),
        ] {
            let parsed: DataType = spelling.parse().unwrap_or_else(|error| {
                panic!("{spelling} must parse: {error}");
            });
            assert_eq!(parsed.to_string(), canonical, "{spelling}");
            let reparsed: DataType = parsed.to_string().parse().unwrap();
            assert_eq!(reparsed, parsed, "{spelling}");
        }
    }

    #[test]
    fn adversarial_geospatial_spellings_are_refused_by_name() {
        // A geometry given an edge algorithm: straight planar lines need none.
        let error = "geometry('EPSG:4326', 'vincenty')"
            .parse::<DataType>()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "expected no edge algorithm for geometry, got \"vincenty\"; \
                 geography is the type whose edges take one"
            ),
            "{error}"
        );

        // An unknown algorithm reports the accepted vocabulary.
        let error = "geography('OGC:CRS84', 'euclidean')"
            .parse::<DataType>()
            .unwrap_err()
            .to_string();
        assert!(error.contains("spherical"), "{error}");
        assert!(error.contains("\"euclidean\""), "{error}");

        // An unterminated CRS string fails at a byte position.
        assert!("geometry('EPSG:4326".parse::<DataType>().is_err());

        // An empty CRS names nothing; the absent spelling fills the default.
        let error = "geometry('')".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn the_constructors_fill_and_refuse_what_the_grammar_does() {
        assert_eq!(DataType::variant(), DataType::Variant);

        let bare = DataType::geometry(None).unwrap();
        let explicit = DataType::geometry(Some("OGC:CRS84")).unwrap();
        // Filling the default makes the two spellings one type.
        assert_eq!(bare, explicit);

        let geography = DataType::geography(None, None).unwrap();
        let DataType::Geography(geospatial) = &geography else {
            panic!("expected a geography, got {geography}");
        };
        assert_eq!(geospatial.algorithm(), Some(EdgeAlgorithm::Spherical));
        assert!(geospatial.has_default_crs());

        // The shared value refuses what the grammar refuses.
        assert!(GeospatialType::geometry(Some("")).is_err());
        assert_eq!(
            GeospatialType::geometry(Some("EPSG:3857")).unwrap().crs(),
            "EPSG:3857"
        );
    }

    #[test]
    fn identity_kind_and_nesting_answer_for_the_new_variants() {
        assert_eq!(DataType::Variant.id(), DataTypeId::Variant);
        assert_eq!(DataType::Variant.kind(), DataTypeKind::Variant);
        // A variant holds a tree, so it is nested; a geometry is one value.
        assert!(DataType::Variant.is_nested());

        let geometry = DataType::geometry(None).unwrap();
        assert_eq!(geometry.id(), DataTypeId::Geometry);
        assert_eq!(geometry.kind(), DataTypeKind::Geospatial);
        assert!(!geometry.is_nested());

        let geography = DataType::geography(None, None).unwrap();
        assert_eq!(geography.id(), DataTypeId::Geography);
        assert_eq!(geography.kind(), DataTypeKind::Geospatial);
    }

    #[test]
    fn serde_and_the_structural_value_round_trip() {
        for data_type in [
            DataType::Variant,
            DataType::geometry(Some("EPSG:4326")).unwrap(),
            DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Karney)).unwrap(),
            DataType::geography(None, None).unwrap(),
        ] {
            let json = data_type.clone().into_json().unwrap();
            assert_eq!(DataType::from_json(&json).unwrap(), data_type, "{json}");

            let value = data_type.clone().into_value();
            assert_eq!(DataType::from_value(value).unwrap(), data_type);
        }
    }

    #[test]
    fn ordering_and_hashing_are_consistent_for_the_new_variants() {
        let one = DataType::geometry(Some("EPSG:4326")).unwrap();
        let two = DataType::geometry(Some("EPSG:4326")).unwrap();
        assert_eq!(one, two);
        assert_eq!(one.cmp(&two), std::cmp::Ordering::Equal);

        // Hash agrees with Eq, checked pairwise: a datatype carries interior
        // caches elsewhere, so the set spelling trips the mutable-key lint
        // where the direct hash comparison says the same thing.
        fn hash_of(value: &DataType) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }
        assert_eq!(hash_of(&one), hash_of(&two));

        // Distinct parameters are distinct types, ordered deterministically.
        let other = DataType::geometry(Some("EPSG:3857")).unwrap();
        assert_ne!(two, other);
        assert_eq!(two.cmp(&other), other.cmp(&two).reverse());
    }

    #[test]
    fn defaults_are_a_present_variant_null_and_a_point_empty() {
        // The variant's present zero value is the variant null - a value the
        // encoding can spell, not an absence - so a required variant column
        // has a default.
        let variant = DataType::Variant.required_field("payload");
        assert_eq!(variant.default_value().unwrap(), Value::Null);

        // The geospatial default is the conventional empty geometry, in the
        // canonical value spelling.
        let geometry = DataType::geometry(None).unwrap().required_field("shape");
        let default = geometry.default_value().unwrap();
        assert!(matches!(default, Value::Geospatial(_)), "{default:?}");
        let bytes = default.as_wkb().expect("a WKB payload");
        assert_eq!(
            crate::generic::wkb::into_wkt(bytes).unwrap(),
            "POINT EMPTY",
            "the default is POINT EMPTY"
        );
    }

    #[test]
    fn rows_validate_through_the_new_columns() {
        let root = |field: Field| {
            DataType::from_fields([field])
                .unwrap()
                .required_field("row")
        };
        let row = |value: Value| Value::from_sequence([value]);

        // A variant column validates any value, null included: the variant
        // null is a value the encoding can spell, so a required variant
        // holding it is present, not absent.
        let variant = root(DataType::Variant.required_field("payload"));
        variant.validate_value(&row(Value::Null)).unwrap();
        variant.validate_value(&row(Value::from(12_i64))).unwrap();
        variant
            .validate_value(&row(Value::from_mapping([(
                Value::from("a"),
                Value::from(1_i64),
            )])
            .unwrap()))
            .unwrap();

        // A geospatial column validates the WKB payload's own framing.
        let geometry = root(DataType::geometry(None).unwrap().required_field("shape"));
        let point: Vec<u8> = {
            // POINT (1 2), little-endian ISO WKB.
            let mut bytes = vec![0x01, 0x01, 0x00, 0x00, 0x00];
            bytes.extend_from_slice(&1.0_f64.to_le_bytes());
            bytes.extend_from_slice(&2.0_f64.to_le_bytes());
            bytes
        };
        geometry.validate_value(&row(Value::from(point))).unwrap();

        let refusal = geometry
            .validate_value(&row(Value::from(vec![0x01_u8, 0xFF])))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("geometry"), "{refusal}");
        let not_bytes = geometry
            .validate_value(&row(Value::from("POINT (1 2)")))
            .unwrap_err()
            .to_string();
        assert!(not_bytes.contains("geometry"), "{not_bytes}");
    }

    #[test]
    fn compatibility_rows_answer_for_every_target() {
        use crate::Scheme;

        let schema = DataType::from_fields([
            DataType::Variant.nullable_field("payload"),
            DataType::geometry(None).unwrap().nullable_field("shape"),
            DataType::geography(None, None)
                .unwrap()
                .nullable_field("region"),
        ])
        .unwrap()
        .required_field("row");

        // Iceberg v3 owns all three, parameters included: pass unchanged.
        let iceberg = schema.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap();
        assert_eq!(iceberg, schema);

        // The engines without the types refuse by name, with a path.
        for scheme in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
            let error = schema
                .clone()
                .into_scheme_compat(&scheme)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("payload") || error.contains("variant"),
                "{error}"
            );
        }
    }
}
