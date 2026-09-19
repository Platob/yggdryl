//! Focused regression tests for the datatype implementation modules.
use std::collections::{BTreeSet, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

use yggdryl::types::{
    BytesType, DataType, DateTimeType, DateType, DecimalType, DictionaryType, DurationType, Fields,
    FloatingType, GeospatialType, IntegerType, IntervalType, MapType, RunEndEncodedType,
    StringType, TimeType, TimeUnit, UnionFields, UnionMode,
};
use yggdryl::{Charset, Error, Field, Timezone};

#[test]
fn datatype_family_enums_round_trip_the_root_without_losing_parameters() {
    let integer = IntegerType::try_from(&DataType::UInt32).unwrap();
    assert_eq!(integer.id(), yggdryl::DataTypeId::UInt32);
    assert_eq!(DataType::from(integer), DataType::UInt32);

    let floating = FloatingType::try_from(&DataType::Float16).unwrap();
    assert_eq!(DataType::from(floating), DataType::Float16);

    let decimal = DataType::decimal128(20, 4).unwrap();
    let decimal_family = DecimalType::try_from(&decimal).unwrap();
    assert_eq!(DataType::from(decimal_family), decimal);

    // The five temporal families each hold their own payload; a unit or a
    // zone is a parameter of the leaf and survives the round trip.
    let datetime = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap();
    let datetime_family = DateTimeType::try_from(&datetime).unwrap();
    assert_eq!(datetime_family.unit(), TimeUnit::Microsecond);
    assert_eq!(DataType::from(datetime_family), datetime);

    let date = DateType::try_from(&DataType::date64()).unwrap();
    assert_eq!(date, DateType::Date64);
    assert_eq!(DataType::from(date), DataType::date64());

    let time = DataType::time64(TimeUnit::Nanosecond).unwrap();
    let time_family = TimeType::try_from(&time).unwrap();
    assert_eq!(time_family, TimeType::Time64(TimeUnit::Nanosecond));
    assert_eq!(DataType::from(time_family), time);

    let duration = DataType::duration32(TimeUnit::Second).unwrap();
    let duration_family = DurationType::try_from(&duration).unwrap();
    assert_eq!(duration_family, DurationType::Duration32(TimeUnit::Second));
    assert_eq!(DataType::from(duration_family), duration);

    let interval = DataType::interval(TimeUnit::DayTime).unwrap();
    let interval_family = IntervalType::try_from(&interval).unwrap();
    assert_eq!(interval_family.unit(), TimeUnit::DayTime);
    assert_eq!(DataType::from(interval_family), interval);

    let text = DataType::large_utf8().string_parameters().unwrap();
    assert_eq!(text, StringType::LargeUtf8String);
    assert_eq!(DataType::string(text).unwrap(), DataType::large_utf8());

    let encoded = StringType::Utf8StringView
        .with_charset(Charset::Cp1252)
        .unwrap();
    assert_eq!(encoded, StringType::Cp1252StringView);
    assert_eq!(
        DataType::string(encoded).unwrap().string_parameters(),
        Some(encoded)
    );

    // The string family has no family enum of its own: `DataTypeId` names
    // the exact leaf and `string_parameters` answers it, charset and number
    // included, so a third listing would only be one more thing to disagree.
    let ascii = DataType::fixed_ascii(7).unwrap();
    assert_eq!(ascii.id(), yggdryl::DataTypeId::FixedAsciiString);
    assert_eq!(ascii.fixed_byte_width(), Some(7));
    assert!(ascii.is_string());
    let parameters = ascii.string_parameters().unwrap();
    assert_eq!(parameters, StringType::FixedAsciiString(7));
    assert_eq!(parameters.charset(), Charset::Ascii);
    assert_eq!(parameters.fixed(), Some(7));
    assert_eq!(DataType::string(parameters).unwrap(), ascii);

    // The byte family reads back the same way: the layout and the bound.
    let bytes = DataType::fixed_binary(16).unwrap();
    assert_eq!(bytes.id(), yggdryl::DataTypeId::FixedBinary);
    assert_eq!(bytes.fixed_byte_width(), Some(16));
    let parameters = bytes.bytes_parameters().unwrap();
    assert_eq!(parameters, BytesType::FixedBinary(16));
    assert_eq!(parameters.fixed(), Some(16));
    assert_eq!(DataType::bytes(parameters).unwrap(), bytes);
    assert_eq!(DataType::utf8().bytes_parameters(), None);

    // A wrapper datatype answers for itself now that no intermediate enum
    // stands between the root and the families.
    let nested = DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap();
    assert!(nested.id().is_wrapper());
    assert_eq!(nested.id(), yggdryl::DataTypeId::Dictionary);

    let geospatial = DataType::geography(None, None).unwrap();
    let geospatial_family = GeospatialType::try_from(&geospatial).unwrap();
    assert_eq!(DataType::from(geospatial_family), geospatial);

    assert!(IntegerType::try_from(&DataType::utf8()).is_err());
}

#[test]
fn nested_helper_values_have_total_order_and_hash() {
    fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
    assert_traits::<MapType>();
    assert_traits::<RunEndEncodedType>();

    let first = DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap();
    let later = DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap();
    let (DataType::Mapping(first), DataType::Mapping(later)) = (first, later) else {
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
                DataType::utf8(),
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

    let arrow = value.clone().into_arrow_datatype().unwrap();
    assert_eq!(DataType::from_arrow_datatype(&arrow).unwrap(), value);
    assert_eq!(
        DataType::try_from(value.clone().into_arrow_datatype().unwrap()).unwrap(),
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
        serde_json::json!({"type": "binary", "layout": "fixed_size_binary", "fixed": 0}),
        serde_json::json!({"type": "decimal32", "precision": 10, "scale": 0}),
        serde_json::json!({
            "type": "struct",
            "fields": [
                field("same", serde_json::json!({"type": "int32"}), false),
                field("same", serde_json::json!({"type": "string"}), true)
            ]
        }),
        serde_json::json!({
            "type": "union",
            "mode": "dense",
            "fields": [
                {"type_id": 1, "field": field("one", serde_json::json!({"type": "int32"}), false)},
                {"type_id": 1, "field": field("two", serde_json::json!({"type": "string"}), true)}
            ]
        }),
        serde_json::json!({
            "type": "dictionary",
            "key": {"type": "float64"},
            "value": {"type": "string"}
        }),
        serde_json::json!({
            "type": "map",
            "entries": field(
                "entries",
                serde_json::json!({
                    "type": "struct",
                    "fields": [
                        field("key", serde_json::json!({"type": "string"}), false),
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
            "values": field("values", serde_json::json!({"type": "string"}), true)
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
                "dtype":{"type":"string"},
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
        "value": {"type": "string"}
    });
    assert_eq!(
        serde_json::from_value::<DictionaryType>(dictionary)
            .unwrap_err()
            .to_string(),
        "invalid Dictionary datatype: expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got float64"
    );
    assert_eq!(
        DataType::dictionary(DataType::Float64, DataType::utf8())
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
        "values": field("values", serde_json::json!({"type": "string"}), true)
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
            Field::new("values", DataType::utf8(), true),
        )
        .unwrap_err()
        .to_string(),
        "invalid RunEndEncoded datatype: expected a non-null run_ends field, got nullable field \"run_ends\""
    );

    // The two run_ends rules are independent, so each reports the half that
    // actually failed rather than one fused sentence.
    assert_eq!(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::utf8(), false),
            Field::new("values", DataType::utf8(), true),
        )
        .unwrap_err()
        .to_string(),
        "invalid RunEndEncoded datatype: expected a run_ends datatype of int16, int32, or int64, got utf8"
    );
}

#[test]
fn structural_serialization_rejects_public_enum_invalid_states() {
    let invalid = [
        DataType::Time(TimeType::Time32(TimeUnit::Nanosecond)),
        DataType::Bytes(BytesType::FixedBinary(0)),
        DataType::String(StringType::FixedUtf8String(0)),
        DataType::Decimal(DecimalType::Decimal128 {
            precision: 0,
            scale: 0,
        }),
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
        DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: yggdryl::Timezone::from_str("Europe/Paris").unwrap()
        })
    );
    assert_eq!(
        DataType::from_str("TIMESTAMP WITH TIME ZONE").unwrap(),
        DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: yggdryl::Timezone::UTC
        })
    );
    assert_eq!(
        DataType::from_str("interval year to month").unwrap(),
        DataType::interval(TimeUnit::YearMonth).unwrap()
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
    let item = || Field::new("item", DataType::utf8(), true);
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
        DataType::datetime64(
            TimeUnit::Nanosecond,
            yggdryl::Timezone::from_str("Europe/Paris").unwrap(),
        )
        .unwrap(),
        DataType::date32(),
        DataType::date64(),
        DataType::time32(TimeUnit::Millisecond).unwrap(),
        DataType::time64(TimeUnit::Microsecond).unwrap(),
        DataType::duration64(TimeUnit::Nanosecond).unwrap(),
        DataType::interval(TimeUnit::YearMonth).unwrap(),
        DataType::interval(TimeUnit::DayTime).unwrap(),
        DataType::interval(TimeUnit::MonthDayNano).unwrap(),
        DataType::binary(),
        DataType::fixed_binary(16).unwrap(),
        DataType::large_binary(),
        DataType::binary_view(),
        DataType::utf8(),
        DataType::large_utf8(),
        DataType::utf8_view(),
        DataType::list(item()),
        DataType::list_view(item()),
        DataType::fixed_size_list(item(), 4).unwrap(),
        DataType::large_list(item()),
        DataType::large_list_view(item()),
        DataType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
        DataType::union(
            [
                (0, Field::new("number", DataType::Int64, false)),
                (1, Field::new("text", DataType::utf8(), true)),
            ],
            UnionMode::Sparse,
        )
        .unwrap(),
        DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, -2).unwrap(),
        DataType::decimal128(38, 18).unwrap(),
        DataType::decimal256(76, 20).unwrap(),
        DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )
        .unwrap(),
    ];

    for source in values {
        let arrow: ArrowDataType = source.clone().into_arrow_datatype().unwrap();
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

    let borrowed = DataType::from_arrow_datatype(&arrow).unwrap();
    let borrowed_outer = borrowed.get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &borrowed_outer.clone().into_arrow_field_ref().unwrap(),
        &outer
    ));
    let borrowed_inner = borrowed_outer.dtype().get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &borrowed_inner.clone().into_arrow_field_ref().unwrap(),
        &inner
    ));

    let owned = DataType::try_from(arrow).unwrap();
    let owned_outer = owned.get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &owned_outer.clone().into_arrow_field_ref().unwrap(),
        &outer
    ));
    let owned_inner = owned_outer.dtype().get_field(0).unwrap();
    assert!(Arc::ptr_eq(
        &owned_inner.clone().into_arrow_field_ref().unwrap(),
        &inner
    ));
}

#[test]
fn long_timezones_reuse_process_interned_storage_across_arrow_conversions() {
    let timezone: Arc<str> = Arc::from("America/Argentina/Buenos_Aires");
    let arrow = ArrowDataType::Timestamp(
        arrow_schema::TimeUnit::Nanosecond,
        Some(Arc::clone(&timezone)),
    );

    let borrowed = DataType::from_arrow_datatype(&arrow).unwrap();
    let DataType::DateTime(DateTimeType::DateTime64 {
        unit: _,
        timezone: borrowed_timezone,
    }) = &borrowed
    else {
        panic!("timestamp import changed variant");
    };
    let borrowed_timezone = *borrowed_timezone;

    let borrowed_arrow = borrowed.into_arrow_datatype().unwrap();
    let ArrowDataType::Timestamp(_, Some(borrowed_arrow_timezone)) = borrowed_arrow else {
        panic!("timestamp projection changed variant");
    };
    assert_eq!(borrowed_arrow_timezone.as_ref(), timezone.as_ref());

    let owned = DataType::try_from(arrow).unwrap();
    let DataType::DateTime(DateTimeType::DateTime64 {
        unit: _,
        timezone: owned_timezone,
    }) = &owned
    else {
        panic!("timestamp import changed variant");
    };
    assert!(std::ptr::eq(
        borrowed_timezone.as_smol_str(),
        owned_timezone.as_smol_str()
    ));

    let owned_arrow = owned.into_arrow_datatype().unwrap();
    let ArrowDataType::Timestamp(_, Some(owned_arrow_timezone)) = owned_arrow else {
        panic!("timestamp projection changed variant");
    };
    assert_eq!(owned_arrow_timezone.as_ref(), timezone.as_ref());
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
    assert!(DataType::from_arrow_datatype(&maximum).is_ok());
    assert!(DataType::try_from(maximum).is_ok());

    let over_limit = nested_list(DataType::PARSE_RECURSION_LIMIT);
    assert!(DataType::from_arrow_datatype(&over_limit).is_err());
    assert!(DataType::try_from(over_limit).is_err());
}

#[test]
fn public_field_collections_validate_children_without_clone_helpers() {
    let invalid = Field::new(
        "invalid",
        DataType::Time(TimeType::Time32(TimeUnit::Nanosecond)),
        false,
    );
    assert!(Fields::from_fields([invalid.clone()]).is_err());
    assert!(UnionFields::from_fields([(0, invalid)]).is_err());
}
