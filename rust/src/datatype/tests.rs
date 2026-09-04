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
    assert_eq!(structural["field"]["dtype"]["type"], "struct");
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

    let field = |name: &str, dtype: serde_json::Value, nullable: bool| {
        serde_json::json!({
            "name": name,
            "dtype": dtype,
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
                "dtype":{"type":"utf8"},
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

    let field = |name: &str, dtype: serde_json::Value, nullable: bool| {
        serde_json::json!({
            "name": name,
            "dtype": dtype,
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
    assert_eq!(left.get_field_by_path("b").map(Field::name), Some("b"));
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
    let borrowed_inner = borrowed_outer.dtype().get_field(0).unwrap();
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
    let owned_inner = owned_outer.dtype().get_field(0).unwrap();
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
    use crate::generic::{DataTypeId, DataTypeKind, EdgeAlgorithm};
    use crate::{Field, Scalar};

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
        for dtype in [
            DataType::Variant,
            DataType::geometry(Some("EPSG:4326")).unwrap(),
            DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Karney)).unwrap(),
            DataType::geography(None, None).unwrap(),
        ] {
            let json = dtype.clone().into_json().unwrap();
            assert_eq!(DataType::from_json(&json).unwrap(), dtype, "{json}");

            let value = dtype.clone().into_value();
            assert_eq!(DataType::from_value(value).unwrap(), dtype);
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
        assert_eq!(variant.default_value().unwrap(), Scalar::Null);

        // The geospatial default is the conventional empty geometry, in the
        // canonical value spelling.
        let geometry = DataType::geometry(None).unwrap().required_field("shape");
        let default = geometry.default_value().unwrap();
        assert!(matches!(default, Scalar::Geospatial(_)), "{default:?}");
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
        let row = |value: Scalar| Scalar::from_sequence([value]);

        // A variant column validates any value, null included: the variant
        // null is a value the encoding can spell, so a required variant
        // holding it is present, not absent.
        let variant = root(DataType::Variant.required_field("payload"));
        variant.validate_value(&row(Scalar::Null)).unwrap();
        variant.validate_value(&row(Scalar::from(12_i64))).unwrap();
        variant
            .validate_value(&row(Scalar::from_mapping([(
                Scalar::from("a"),
                Scalar::from(1_i64),
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
        geometry.validate_value(&row(Scalar::from(point))).unwrap();

        let refusal = geometry
            .validate_value(&row(Scalar::from(vec![0x01_u8, 0xFF])))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("geometry"), "{refusal}");
        let not_bytes = geometry
            .validate_value(&row(Scalar::from("POINT (1 2)")))
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

/// The ASCII widths and the currency registration.
mod ascii {
    use std::cmp::Ordering;

    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;

    use super::super::DataType;
    use crate::generic::{DataTypeId, DataTypeKind};
    use crate::{Error, Field, Scalar, Scheme};

    fn hash_of(value: &DataType) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn stored(array: &dyn Array) -> &FixedSizeBinaryArray {
        array
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("fixed-width ASCII storage")
    }

    #[test]
    fn every_spelling_parses_and_displays_as_its_width() {
        for (spelling, dtype) in [
            ("ascii16", DataType::Ascii16),
            ("ascii24", DataType::Ascii24),
            ("ascii32", DataType::Ascii32),
            ("ascii64", DataType::Ascii64),
            ("ascii96", DataType::Ascii96),
            ("ascii128", DataType::Ascii128),
            ("ASCII64", DataType::Ascii64),
            ("ascii(1)", DataType::Ascii16),
            ("ascii(2)", DataType::Ascii16),
            ("ascii(3)", DataType::Ascii24),
            ("ascii(4)", DataType::Ascii32),
            ("ascii(5)", DataType::Ascii64),
            ("ascii(6)", DataType::Ascii64),
            ("ascii(8)", DataType::Ascii64),
            ("ascii(9)", DataType::Ascii96),
            ("ascii(12)", DataType::Ascii96),
            ("ascii(13)", DataType::Ascii128),
            ("ascii(16)", DataType::Ascii128),
            ("country", DataType::Ascii16),
            ("currency", DataType::Ascii24),
            ("Currency", DataType::Ascii24),
            ("cfi", DataType::Ascii64),
            ("CFI", DataType::Ascii64),
        ] {
            let parsed: DataType = spelling
                .parse()
                .unwrap_or_else(|error| panic!("{spelling} must parse: {error}"));
            assert_eq!(parsed, dtype, "{spelling}");
            // One canonical spelling: a registration displays as its width.
            assert_eq!(parsed.to_string(), dtype.name(), "{spelling}");
            assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
        }
        let row: DataType = "struct<ccy: currency, isin: ascii(12), code: cfi, iso: country>"
            .parse()
            .unwrap();
        assert_eq!(
            row.get_field_by_path("ccy").map(Field::dtype),
            Some(&DataType::Ascii24)
        );
        assert_eq!(
            row.get_field_by_path("isin").map(Field::dtype),
            Some(&DataType::Ascii96)
        );
        assert_eq!(
            row.get_field_by_path("code").map(Field::dtype),
            Some(&DataType::Ascii64)
        );
        assert_eq!(
            row.get_field_by_path("iso").map(Field::dtype),
            Some(&DataType::Ascii16)
        );
    }

    #[test]
    fn widths_outside_the_family_and_a_bare_ascii_are_refused_by_name() {
        let error = "ascii(17)".parse::<DataType>().unwrap_err().to_string();
        assert!(
            error.contains("expected an ASCII width from 1 to 16 bytes, got 17"),
            "{error}"
        );
        let error = "ascii(0)".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("got 0"), "{error}");
        let error = "ascii".parse::<DataType>().unwrap_err().to_string();
        assert!(
            error.contains("expected an ASCII width from 1 to 16 bytes"),
            "{error}"
        );
        assert!(matches!(
            "ascii()".parse::<DataType>(),
            Err(Error::Parse { .. })
        ));

        let error = DataType::ascii(17).unwrap_err().to_string();
        assert!(error.contains("got 17"), "{error}");
        assert!(DataType::ascii(-1).is_err());
    }

    #[test]
    fn a_registered_logical_name_resolves_case_insensitively_and_names_its_vocabulary() {
        assert_eq!(
            DataType::LOGICAL_NAMES,
            &[
                ("country", DataType::Ascii16),
                ("currency", DataType::Ascii24),
                ("cfi", DataType::Ascii64),
            ]
        );
        assert_eq!(
            DataType::from_logical_name("currency").unwrap(),
            DataType::Ascii24
        );
        assert_eq!(
            DataType::from_logical_name(" CURRENCY ").unwrap(),
            DataType::Ascii24
        );
        // ISO 10962 is six characters, so it registers over the narrowest
        // width that holds six bytes.
        assert_eq!(
            DataType::from_logical_name("cfi").unwrap(),
            DataType::Ascii64
        );
        // ISO 3166-1 alpha-2 is two letters, which is exactly `ascii16`.
        assert_eq!(
            DataType::from_logical_name("country").unwrap(),
            DataType::Ascii16
        );

        let error = DataType::from_logical_name("isin").unwrap_err().to_string();
        assert!(error.contains("currency"), "{error}");
        assert!(error.contains("cfi"), "{error}");
        assert!(error.contains("\"isin\""), "{error}");
        // The grammar reports an unregistered word as unknown.
        let error = "isin".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("unknown datatype \"isin\""), "{error}");
    }

    #[test]
    fn serde_and_the_structural_value_round_trip() {
        for (dtype, tag) in [
            (DataType::Ascii32, "ascii32"),
            (DataType::Ascii64, "ascii64"),
            (DataType::Ascii128, "ascii128"),
        ] {
            let json = dtype.clone().into_json().unwrap();
            assert_eq!(json, format!(r#"{{"type":"{tag}"}}"#));
            assert_eq!(DataType::from_json(&json).unwrap(), dtype);

            let value = dtype.clone().into_value();
            assert_eq!(
                value.get_key_str("type").and_then(Scalar::as_str),
                Some(tag)
            );
            assert_eq!(DataType::from_value(value).unwrap(), dtype);
        }
    }

    #[test]
    fn identity_kind_and_widths_answer_for_every_width() {
        for (dtype, id, width) in [
            (DataType::Ascii16, DataTypeId::Ascii16, 2),
            (DataType::Ascii24, DataTypeId::Ascii24, 3),
            (DataType::Ascii32, DataTypeId::Ascii32, 4),
            (DataType::Ascii64, DataTypeId::Ascii64, 8),
            (DataType::Ascii96, DataTypeId::Ascii96, 12),
            (DataType::Ascii128, DataTypeId::Ascii128, 16),
        ] {
            assert_eq!(dtype.id(), id);
            assert_eq!(dtype.kind(), DataTypeKind::String);
            assert_eq!(dtype.name(), id.as_str());
            assert_eq!(dtype.ascii_width(), Some(width));
            assert_eq!(id.fixed_byte_width(), usize::try_from(width).ok());
            assert!(!dtype.is_nested());
            dtype.validate().unwrap();
        }
        assert_eq!(DataType::Utf8.ascii_width(), None);
        assert_eq!(DataType::FixedSizeBinary(4).ascii_width(), None);
    }

    #[test]
    fn ordering_and_hashing_are_consistent_for_every_width() {
        // The widths sit after the variable text layouts, in width order.
        assert!(DataType::Utf8View < DataType::Ascii16);
        assert!(DataType::Ascii16 < DataType::Ascii24);
        assert!(DataType::Ascii24 < DataType::Ascii32);
        assert!(DataType::Ascii32 < DataType::Ascii64);
        assert!(DataType::Ascii64 < DataType::Ascii96);
        assert!(DataType::Ascii96 < DataType::Ascii128);
        assert!(DataType::Ascii128 < DataType::list(DataType::Utf8.nullable_field("item")));
        assert_eq!(DataType::Ascii64.cmp(&DataType::Ascii64), Ordering::Equal);
        assert_eq!(hash_of(&DataType::Ascii64), hash_of(&DataType::Ascii64));
        assert_ne!(
            DataType::Ascii32.stable_hash(),
            DataType::Ascii64.stable_hash()
        );
    }

    #[test]
    fn the_default_is_the_empty_string_stored_as_all_nul() {
        for dtype in [
            DataType::Ascii16,
            DataType::Ascii24,
            DataType::Ascii32,
            DataType::Ascii64,
            DataType::Ascii96,
            DataType::Ascii128,
        ] {
            assert_eq!(dtype.default_value().unwrap(), Scalar::from(""));
            assert!(dtype.is_default_value(&Scalar::from("")).unwrap());
            assert!(!dtype.is_default_value(&Scalar::from("USD")).unwrap());

            let width = dtype.ascii_width().unwrap();
            let field = dtype.required_field("ccy");
            assert_eq!(field.default_value().unwrap(), Scalar::from(""));
            let array = field.default_arrow_array().unwrap();
            assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(width));
            let stored = stored(array.as_ref());
            assert_eq!(stored.len(), 1);
            assert!(stored.value(0).iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn values_validate_and_canonicalize_under_the_one_ascii_rule() {
        let root = DataType::from_fields([DataType::Ascii32.required_field("ccy")])
            .unwrap()
            .required_field("row");
        let row = |value: Scalar| Scalar::from_sequence([value]);
        let canonical = |value: Scalar| root.canonicalize_value(row(value)).unwrap();

        // The trimmed string is the canonical spelling and passes untouched.
        assert_eq!(canonical(Scalar::from("USD")), row(Scalar::from("USD")));
        assert_eq!(canonical(Scalar::from("ABCD")), row(Scalar::from("ABCD")));
        assert_eq!(canonical(Scalar::from("")), row(Scalar::from("")));
        // Trailing NULs are trimmed, and bytes are rewritten to the string.
        assert_eq!(canonical(Scalar::from("USD\0")), row(Scalar::from("USD")));
        assert_eq!(
            canonical(Scalar::from(b"USD\0".to_vec())),
            row(Scalar::from("USD"))
        );
        assert_eq!(
            canonical(Scalar::from(b"EUR".to_vec())),
            row(Scalar::from("EUR"))
        );

        // Every refusal names the width and the offending fact.
        for (value, fact) in [
            (Scalar::from("EURO!"), "5 bytes"),
            (Scalar::from(b"ABCDE".to_vec()), "5 bytes"),
            (Scalar::from("\u{20AC}"), "non-ASCII byte 0xE2 at 0"),
            (Scalar::from("U\0D"), "NUL byte at 1"),
        ] {
            let refused = root
                .validate_value(&row(value.clone()))
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("ASCII text of at most 4 bytes"),
                "{refused}"
            );
            assert!(refused.contains(fact), "{refused}");
            assert!(refused.contains("ccy"), "{refused}");
            assert!(root.canonicalize_value(row(value)).is_err());
        }
        // Anything that is neither text nor bytes is refused by kind.
        let refused = root
            .validate_value(&row(Scalar::I64(7)))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("expected ascii32"), "{refused}");
    }

    #[test]
    fn arrow_storage_is_padded_and_reads_back_trimmed() {
        let field = DataType::Ascii64.nullable_field("code");
        let array = crate::arrow::scalar_array(&field, &Scalar::from("ABC")).unwrap();
        assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(8));
        assert_eq!(stored(array.as_ref()).value(0), b"ABC\0\0\0\0\0");
        assert_eq!(
            crate::arrow::scalar_value(&field, array.as_ref()).unwrap(),
            Scalar::from("ABC")
        );

        // Padded bytes, a null, and the empty string, through the array boundary.
        let values = Scalar::from_sequence([
            Scalar::from(b"XY\0\0\0\0\0\0".to_vec()),
            Scalar::Null,
            Scalar::from(""),
        ]);
        let array = crate::arrow::array_from_value(&field, &values).unwrap();
        let fixed = stored(array.as_ref());
        assert_eq!(fixed.len(), 3);
        assert_eq!(fixed.value(0), b"XY\0\0\0\0\0\0");
        assert!(fixed.is_null(1));
        assert_eq!(fixed.value(2), &[0; 8]);
        let read = |index: usize| {
            crate::arrow::value::value_from_array(field.dtype(), array.as_ref(), index).unwrap()
        };
        assert_eq!(read(0), Scalar::from("XY"));
        assert_eq!(read(2), Scalar::from(""));

        // What does not fit is refused at this boundary too.
        assert!(crate::arrow::scalar_array(&field, &Scalar::from("ABCDEFGHI")).is_err());
    }

    #[test]
    fn compatibility_reads_every_width_as_utf8() {
        let schema = DataType::from_fields([
            DataType::Ascii32.nullable_field("ccy"),
            DataType::Ascii128.required_field("code"),
        ])
        .unwrap()
        .required_field("row");
        for scheme in [
            Scheme::SPARK,
            Scheme::POLARS,
            Scheme::PANDAS,
            Scheme::ICEBERG,
        ] {
            let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
            assert_eq!(compat["ccy"].dtype(), &DataType::Utf8);
            assert_eq!(compat["code"].dtype(), &DataType::Utf8);
            assert!(!compat["code"].is_nullable());
        }
        assert_eq!(
            schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
            schema
        );
        assert_eq!(
            DataType::Ascii64
                .into_scheme_compat(&Scheme::ICEBERG)
                .unwrap(),
            DataType::Utf8
        );
    }
}

/// The per-column ASCII vocabulary and the enum members it names.
mod ascii_dictionary {
    use super::super::{AsciiDictionary, AsciiEnum, DataType};

    #[test]
    fn registration_is_first_appearance_and_a_repeat_keeps_its_code() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        assert!(currencies.is_empty());
        assert_eq!(currencies.push("USD").unwrap(), 0);
        assert_eq!(currencies.push("EUR").unwrap(), 1);
        assert_eq!(currencies.push("USD").unwrap(), 0);
        assert_eq!(currencies.push("JPY").unwrap(), 2);
        assert_eq!(currencies.push("EUR").unwrap(), 1);
        assert_eq!(currencies.len(), 3);
        assert!(!currencies.is_empty());
        assert_eq!(currencies.as_values(), ["USD", "EUR", "JPY"]);

        // Both directions, and the padded spelling resolves to its trimmed form.
        assert_eq!(currencies.get(0), Some("USD"));
        assert_eq!(currencies.get(2), Some("JPY"));
        assert_eq!(currencies.get(3), None);
        assert_eq!(currencies.get(-1), None);
        assert_eq!(currencies.get_code("EUR"), Some(1));
        assert_eq!(currencies.get_code("EUR\0"), Some(1));
        assert_eq!(currencies.push("EUR\0\0").unwrap(), 1);
        assert_eq!(currencies.get_code("GBP"), None);

        // The width and key the codes are read under.
        assert_eq!(currencies.values_dtype(), &DataType::Ascii32);
        assert_eq!(currencies.key(), &DataType::Int32);
    }

    #[test]
    fn the_bytes_spelling_registers_as_its_trimmed_text() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        assert_eq!(currencies.push_bytes(b"USD\0").unwrap(), 0);
        assert_eq!(currencies.push("USD").unwrap(), 0);
        assert_eq!(currencies.push_bytes(b"EUR").unwrap(), 1);
        assert_eq!(currencies.as_values(), ["USD", "EUR"]);

        // Bytes meet the width's own rule, never a decoding error of their own.
        let refused = currencies.push_bytes(b"\xFF\xFE").unwrap_err().to_string();
        assert!(
            refused.contains("at most 4 bytes, got a non-ASCII byte 0xFF at 0"),
            "{refused}"
        );
        assert_eq!(currencies.len(), 2);
    }

    #[test]
    fn push_refuses_what_the_width_refuses_naming_the_width() {
        let mut codes = AsciiDictionary::new(DataType::Ascii32).unwrap();
        for value in ["EUR\u{20ac}", "US\0D", "EURO!"] {
            let refused = codes.push(value).unwrap_err().to_string();
            assert!(
                refused.contains("ASCII text of at most 4 bytes"),
                "{value}: {refused}"
            );
        }
        // A refusal registers nothing.
        assert!(codes.is_empty());

        // The wider member refuses at its own width.
        let mut wide = AsciiDictionary::new(DataType::Ascii64).unwrap();
        assert_eq!(wide.push("EURO!").unwrap(), 0);
        let refused = wide.push("NINE-CHAR").unwrap_err().to_string();
        assert!(
            refused.contains("ASCII text of at most 8 bytes"),
            "{refused}"
        );
    }

    #[test]
    fn the_values_and_key_datatypes_are_refused_by_name() {
        let refused = AsciiDictionary::new(DataType::Utf8)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("an ASCII width"), "{refused}");
        assert!(refused.contains("got utf8"), "{refused}");

        let dictionary = AsciiDictionary::new(DataType::Ascii32).unwrap();
        assert_eq!(
            dictionary.clone().with_key(DataType::Int64).unwrap().key(),
            &DataType::Int64
        );
        for key in [DataType::Int16, DataType::UInt32, DataType::Utf8] {
            let refused = dictionary
                .clone()
                .with_key(key.clone())
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("an int32 or int64 key datatype"),
                "{refused}"
            );
            assert!(refused.contains(&format!("got {key}")), "{refused}");
        }
    }

    #[test]
    fn from_values_dedups_and_preserves_first_appearance_order() {
        let seeded =
            AsciiDictionary::from_values(DataType::Ascii32, ["EUR", "USD", "EUR", "JPY", "USD"])
                .unwrap();
        assert_eq!(seeded.as_values(), ["EUR", "USD", "JPY"]);
        assert_eq!(seeded.get_code("EUR"), Some(0));
        assert_eq!(seeded.get_code("JPY"), Some(2));

        // The same values pushed one at a time are the same vocabulary.
        let mut pushed = AsciiDictionary::new(DataType::Ascii32).unwrap();
        for value in ["EUR", "USD", "EUR", "JPY", "USD"] {
            pushed.push(value).unwrap();
        }
        assert_eq!(pushed, seeded);

        let refused = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "EURO!"])
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
    }

    #[test]
    fn the_datatype_is_the_dictionary_of_the_key_and_the_width() {
        let currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        let dtype = currencies.dtype().unwrap();
        assert_eq!(
            dtype,
            DataType::dictionary(DataType::Int32, DataType::Ascii32).unwrap()
        );
        assert_eq!(dtype.to_string(), "dictionary(int32,ascii32)");
        assert_eq!(dtype.to_string().parse::<DataType>().unwrap(), dtype);

        let wide = AsciiDictionary::new(DataType::Ascii64)
            .unwrap()
            .with_key(DataType::Int64)
            .unwrap();
        assert_eq!(
            wide.dtype().unwrap().to_string(),
            "dictionary(int64,ascii64)"
        );
    }

    #[test]
    fn equality_is_the_width_the_key_and_the_values_in_order() {
        let usd_first = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "EUR"]).unwrap();
        let eur_first = AsciiDictionary::from_values(DataType::Ascii32, ["EUR", "USD"]).unwrap();
        assert_ne!(usd_first, eur_first);
        assert_eq!(
            usd_first,
            AsciiDictionary::from_values(DataType::Ascii32, ["USD", "EUR"]).unwrap()
        );
        assert_ne!(
            usd_first,
            AsciiDictionary::from_values(DataType::Ascii64, ["USD", "EUR"]).unwrap()
        );
        assert_ne!(
            usd_first,
            usd_first.clone().with_key(DataType::Int64).unwrap()
        );
        assert_ne!(
            usd_first,
            AsciiDictionary::from_values(DataType::Ascii32, ["USD"]).unwrap()
        );
    }

    #[test]
    fn members_apply_the_name_rule_once() {
        let members =
            AsciiDictionary::from_values(DataType::Ascii32, ["USD", "n/a", "1st", "", "a-b"])
                .unwrap()
                .into_members()
                .unwrap();
        assert_eq!(
            members,
            [
                ("USD".into(), 0x5553_4400),
                ("N_A".into(), 0x6E2F_6100),
                ("_1ST".into(), 0x3173_7400),
                ("_".into(), 0),
                ("A_B".into(), 0x612D_6200),
            ]
        );

        // A name that opens and closes with `_` is the shape Python reserves,
        // so the trailing run goes; a name of nothing but `_` keeps it.
        let members = AsciiDictionary::from_values(DataType::Ascii64, ["-a-", "--b--", "-", "--"])
            .unwrap()
            .into_members()
            .unwrap();
        assert_eq!(
            members,
            [
                ("_A".into(), 0x2D61_2D00_0000_0000),
                ("__B".into(), 0x2D2D_622D_2D00_0000),
                ("_".into(), 0x2D00_0000_0000_0000),
                ("__".into(), 0x2D2D_0000_0000_0000),
            ]
        );
        assert!(
            AsciiDictionary::new(DataType::Ascii32)
                .unwrap()
                .into_members()
                .unwrap()
                .is_empty()
        );

        // The same rule names one value at a time, for a vocabulary declared
        // member by member rather than generated from a whole listing.
        assert_eq!(AsciiDictionary::member_name("--b--").as_str(), "__B");
        assert_eq!(AsciiDictionary::member_name("1st").as_str(), "_1ST");
        assert_eq!(AsciiDictionary::member_name("USD\0").as_str(), "USD_");
    }

    #[test]
    fn members_refuse_a_collision_naming_both_values() {
        let refused = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "usd"])
            .unwrap()
            .into_members()
            .unwrap_err()
            .to_string();
        assert!(refused.contains("\"USD\""), "{refused}");
        assert!(refused.contains("\"usd\""), "{refused}");
        assert!(refused.contains("USD"), "{refused}");

        let refused = AsciiDictionary::from_values(DataType::Ascii32, ["a-b", "a/b"])
            .unwrap()
            .into_members()
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("\"a-b\"") && refused.contains("\"a/b\""),
            "{refused}"
        );

        // Sixteen bytes name members too: the code is the whole `i128`, and
        // a width names one value with one integer whatever the vocabulary.
        assert_eq!(
            AsciiDictionary::from_values(DataType::Ascii128, ["US0378331005"])
                .unwrap()
                .into_members()
                .unwrap(),
            [(
                "US0378331005".into(),
                0x5553_3033_3738_3333_3130_3035_0000_0000
            )]
        );
        assert_eq!(
            AsciiDictionary::from_values(DataType::Ascii32, ["EUR", "USD"])
                .unwrap()
                .into_members()
                .unwrap(),
            AsciiDictionary::from_values(DataType::Ascii32, ["USD", "EUR"])
                .unwrap()
                .into_members()
                .unwrap()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_enum_names_its_values_and_renders_one_document() {
        let mut side =
            AsciiEnum::from_members("Side", [("SELL", "S"), ("BUY", "B"), ("BID", "B")]).unwrap();
        assert_eq!(side.name(), "Side");
        assert_eq!(side.len(), 3);
        assert_eq!(side.get("BID"), Some("B"));

        // Two members may name one value; the first by name reads it back, so
        // an alias never changes what a stored code decodes as.
        assert_eq!(side.get_member("B"), Some("BID"));
        assert_eq!(side.get_member("X"), None);
        assert_eq!(
            side.into_members(&DataType::Ascii32).unwrap(),
            [
                ("BID".into(), 0x4200_0000),
                ("BUY".into(), 0x4200_0000),
                ("SELL".into(), 0x5300_0000),
            ]
        );

        // The vocabulary behind the names is an ordinary dictionary, whose
        // codes stay the local positions every vocabulary's codes are.
        let dictionary = side.into_dictionary(DataType::Ascii32).unwrap();
        assert_eq!(dictionary.as_values(), ["B", "S"]);

        // One enum is one document however it was built, and the document is
        // what comes back.
        let document = side.into_json();
        assert_eq!(
            document,
            r#"{"members":{"BID":"B","BUY":"B","SELL":"S"},"name":"Side"}"#
        );
        assert_eq!(AsciiEnum::from_json(&document).unwrap(), side);
        assert_eq!(
            AsciiEnum::from_json(r#" {"name":"Side","members":{"SELL":"S","BUY":"B","BID":"B"}} "#)
                .unwrap(),
            side
        );
        assert_eq!(
            AsciiEnum::from_json(r#"{"name":"Empty"}"#).unwrap(),
            AsciiEnum::new("Empty").unwrap()
        );
        assert!(AsciiEnum::new("Empty").unwrap().is_empty());

        assert_eq!(side.remove("BID"), Some("B".into()));
        assert_eq!(side.remove("BID"), None);
        assert_eq!(side.get_member("B"), Some("BUY"));
        assert_eq!(
            side.iter().collect::<Vec<_>>(),
            [("BUY", "B"), ("SELL", "S")]
        );

        // A member the width cannot store is refused when the codes are asked
        // for, which is where the width is known.
        let refused = side.into_members(&DataType::Utf8).unwrap_err().to_string();
        assert!(refused.contains("an ASCII width"), "{refused}");
    }

    #[test]
    fn an_enum_refuses_what_a_document_could_not_carry_back() {
        for (name, member) in [("", "BUY"), ("Side", ""), ("Si\u{7}de", "BUY")] {
            assert!(AsciiEnum::from_members(name, [(member, "B")]).is_err());
        }
        for document in [
            "[]",
            "not json",
            r#"{"members":{"BUY":"B"}}"#,
            r#"{"name":7}"#,
            r#"{"name":"Side","members":[]}"#,
            r#"{"name":"Side","members":{"BUY":7}}"#,
            r#"{"name":"Side","members":{"":"B"}}"#,
        ] {
            assert!(AsciiEnum::from_json(document).is_err(), "{document}");
        }
    }

    #[test]
    fn an_ascii_value_packs_into_the_integer_its_storage_reads_as() {
        for (dtype, value, packed) in [
            (DataType::Ascii32, "USD", 0x5553_4400_i128),
            (DataType::Ascii32, "", 0),
            (DataType::Ascii64, "EUREX", 0x4555_5245_5800_0000),
            (
                DataType::Ascii128,
                "US0378331005",
                0x5553_3033_3738_3333_3130_3035_0000_0000,
            ),
        ] {
            assert_eq!(dtype.ascii_packed(value.as_bytes()).unwrap(), packed);
            // The storage padding is the same value, and reads back trimmed.
            let padded = format!("{value}\0");
            assert_eq!(dtype.ascii_packed(padded.as_bytes()).unwrap(), packed);
            assert_eq!(dtype.ascii_value(packed).unwrap(), value);
        }

        // An ASCII byte never sets the sign bit, so the order of the packed
        // integers is the order of the text.
        assert!(
            DataType::Ascii32.ascii_packed(b"EUR").unwrap()
                < DataType::Ascii32.ascii_packed(b"USD").unwrap()
        );
        assert!(
            DataType::Ascii64
                .ascii_packed(b"\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f")
                .unwrap()
                > 0
        );

        // What the width refuses, the packing refuses, in both directions.
        let refused = DataType::Ascii32
            .ascii_packed(b"EURO!")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        let refused = DataType::Ascii32.ascii_value(-1).unwrap_err().to_string();
        assert!(refused.contains("wider than the width"), "{refused}");
        let refused = DataType::Ascii32
            .ascii_value(0x1_5553_4400)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("wider than the width"), "{refused}");
        let refused = DataType::Ascii32
            .ascii_value(0x0055_4400)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("a NUL byte at 0"), "{refused}");
        let refused = DataType::Utf8.ascii_packed(b"USD").unwrap_err().to_string();
        assert!(refused.contains("an ASCII width"), "{refused}");
        assert!(DataType::Utf8.ascii_value(0).is_err());
    }
}
