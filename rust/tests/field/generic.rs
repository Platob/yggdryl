use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{DataType, Error, Field, MediaType, Metadata, MimeType, Scheme, TimeUnit, Url};

#[test]
fn canonical_field_preserves_unicode_quotes_commas_and_metadata() {
    let field = Field::from_parts(
        "prix,€\"",
        DataType::from_str("array<struct<clé:string,valeur:decimal(18,4)>>").unwrap(),
        false,
        [("a,b", "quoted \" value"), ("unicode", "東京")],
    )
    .unwrap();
    let canonical = field.to_string();
    let restored = Field::from_str(&canonical).unwrap();
    assert_eq!(restored, field);
    assert_eq!(restored.get_metadata("a,b"), Some("quoted \" value"));
}

#[test]
fn parser_accepts_sql_hive_and_arrow_shapes() {
    let sql = Field::from_str("order_id BIGINT NOT NULL").unwrap();
    assert_eq!(sql.name(), "order_id");
    assert_eq!(sql.data_type(), &DataType::Int64);
    assert!(!sql.is_nullable());

    let hive = Field::from_str("events:array<struct<id:bigint,name:string>>").unwrap();
    assert!(hive.is_nullable());
    assert_eq!(hive.data_type().field_len(), 1);

    let arrow = Field::from_str(
        r#"Field { name: "id", data_type: Int64, nullable: false, metadata: {"source":"arrow"} }"#,
    )
    .unwrap();
    assert_eq!(
        arrow,
        Field::from_parts("id", DataType::Int64, false, [("source", "arrow")]).unwrap()
    );
}

#[test]
fn parser_accepts_flexible_sql_whitespace_and_doubled_quotes() {
    let spaced = Field::from_str("trade_id\tBIGINT \n NOT \t NULL").unwrap();
    assert_eq!(spaced.name(), "trade_id");
    assert_eq!(spaced.data_type(), &DataType::Int64);
    assert!(!spaced.is_nullable());

    let single_quoted = Field::from_str("'owner''s code'   VARCHAR(32)").unwrap();
    assert_eq!(single_quoted.name(), "owner's code");
    assert_eq!(single_quoted.data_type(), &DataType::Utf8);

    let double_quoted = Field::from_str(r#""desk""label" STRING"#).unwrap();
    assert_eq!(double_quoted.name(), "desk\"label");
    assert_eq!(double_quoted.data_type(), &DataType::Utf8);
}

#[test]
fn parser_errors_report_original_input_byte_offsets() {
    let input = r#"  ((field("id", definitely_bad, nullable=true, metadata={})))"#;
    let expected = input.find("definitely_bad").unwrap();
    let error = Field::from_str(input).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Parse {
                target: "field",
                position,
                ..
            } if position == expected
        ),
        "expected byte {expected}, got {error:?}"
    );
}

#[test]
fn parser_rejects_invalid_nullability_and_duplicate_metadata() {
    for malformed in [
        "field(\"id\",int64,nullable=maybe,metadata={})",
        "field(\"id\",int64,nullable=true,metadata={\"a\":\"1\",\"a\":\"2\"})",
    ] {
        assert!(Field::from_str(malformed).is_err(), "{malformed}");
    }
}

#[test]
fn parser_recursion_limit_has_an_exact_public_boundary() {
    let accepted = format!(
        "{}id:int64{}",
        "(".repeat(DataType::PARSE_RECURSION_LIMIT),
        ")".repeat(DataType::PARSE_RECURSION_LIMIT)
    );
    assert_eq!(Field::from_str(&accepted).unwrap().name(), "id");

    let rejected = format!("({accepted})");
    assert!(Field::from_str(&rejected).is_err());
}

#[test]
#[allow(clippy::mutable_key_type)]
fn native_order_hash_json_and_stable_hash_are_value_based() {
    let left =
        Field::from_str(r#"field("a",struct<x:int64>,nullable=false,metadata={"z":"1"})"#).unwrap();
    let right =
        Field::from_str(r#"field("b",struct<x:int64>,nullable=false,metadata={"z":"1"})"#).unwrap();

    let mut ordered = BTreeSet::from([right.clone(), left.clone()]);
    assert_eq!(ordered.pop_first(), Some(left.clone()));
    let hashed = HashSet::from([right.clone(), left.clone()]);
    assert!(hashed.contains(&left));
    assert_ne!(left.stable_hash(), right.stable_hash());

    let json = right.to_json().unwrap();
    assert_eq!(Field::from_json(&json).unwrap(), right);
}

#[test]
fn structural_json_uses_tagged_data_types_and_rejects_bad_shapes() {
    let field = Field::new(
        "payload",
        DataType::from_str("struct<id:bigint,tags:array<string>>").unwrap(),
        false,
    );
    let field_json = field.to_json().unwrap();
    let field_value: serde_json::Value = serde_json::from_str(&field_json).unwrap();
    assert!(field_value.is_object());
    assert_eq!(field_value["data_type"]["type"], "struct");

    assert!(
        Field::from_json(
            r#"{"name":"id","data_type":{"type":"int64"},"nullable":false,"unknown":true}"#,
        )
        .is_err()
    );
    assert!(
        Field::from_json(
            r#"{"name":"id","data_type":{"type":"int64"},"nullable":false,"dictionary_id":7}"#,
        )
        .is_err()
    );
}

#[test]
fn metadata_behaves_as_a_sorted_transactional_mapping() {
    let mut field = Field::new("id", DataType::Int64, false);
    field
        .update_metadata([("z", "last"), ("a", "first")])
        .unwrap();
    assert_eq!(field.metadata_len(), 2);
    assert!(!field.is_metadata_empty());
    assert!(field.has_metadata("a"));
    assert_eq!(&field["z"], "last");
    assert_eq!(
        field.metadata_iter().collect::<Vec<_>>(),
        [("a", "first"), ("z", "last")]
    );

    let snapshot = field.clone();
    assert!(
        field
            .set_metadata([("dup", "one"), ("dup", "two")])
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(field.insert_metadata("", "bad").is_err());
    assert_eq!(field.remove_metadata("a").as_deref(), Some("first"));
    field.clear_metadata();
    assert!(field.is_metadata_empty());
}

#[test]
fn borrowed_and_consuming_arrow_paths_are_lossless() {
    let field = Field::from_str(
        r#"field("items",array<struct<id:bigint,name:string>>,nullable=false,metadata={"source":"test"})"#,
    )
    .unwrap();
    let first = field.to_arrow_ref().unwrap();
    let second = field.to_arrow_ref().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(Field::from_arrow(first.as_ref()).unwrap(), field);

    let owned = field.clone().into_arrow().unwrap();
    assert_eq!(Field::try_from(owned).unwrap(), field);
    let shared = field.clone().into_arrow_ref().unwrap();
    let imported = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
    assert_eq!(imported, field);
    assert!(Arc::ptr_eq(&shared, &imported.to_arrow_ref().unwrap()));
}

#[test]
#[allow(deprecated)]
fn arrow_dictionary_options_survive_parsing_and_cache_invalidation() {
    let arrow = ArrowField::new_dict(
        "codes",
        ArrowDataType::Dictionary(
            Box::new(ArrowDataType::Int16),
            Box::new(ArrowDataType::Utf8),
        ),
        true,
        41,
        true,
    );
    let mut field = Field::from_arrow(&arrow).unwrap();

    assert_eq!(field.dictionary_id(), Some(41));
    assert_eq!(field.dictionary_is_ordered(), Some(true));
    assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
    assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);

    let nested = DataType::from_fields([field.clone()]).unwrap();
    assert_eq!(DataType::from_str(&nested.to_string()).unwrap(), nested);
    let mut different_field = field.clone();
    different_field.set_dictionary_options(42, false).unwrap();
    let different = DataType::from_fields([different_field]).unwrap();
    assert_ne!(nested, different);
    assert_ne!(nested.cmp(&different), std::cmp::Ordering::Equal);

    let json: serde_json::Value = serde_json::from_str(&field.to_json().unwrap()).unwrap();
    assert_eq!(json["dictionary_id"], "41");
    assert_eq!(Field::from_json(&json.to_string()).unwrap(), field);

    let cached = field.to_arrow_ref().unwrap();
    field.set_name("renamed_codes");
    let rebuilt = field.to_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&cached, &rebuilt));
    assert_eq!(rebuilt.dict_id(), Some(41));
    assert_eq!(rebuilt.dict_is_ordered(), Some(true));
}

#[test]
fn no_op_metadata_update_retains_arrow_cache_effective_update_invalidates_it() {
    let mut field = Field::new("id", DataType::Int64, false)
        .try_with_metadata("source", "one")
        .unwrap();
    let original = field.to_arrow_ref().unwrap();
    field.insert_metadata("source", "one").unwrap();
    assert!(Arc::ptr_eq(&original, &field.to_arrow_ref().unwrap()));

    field.insert_metadata("source", "two").unwrap();
    let changed = field.to_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&original, &changed));
}

#[test]
fn invalid_data_type_replacement_is_transactional() {
    let mut field = Field::new("value", DataType::Utf8, true);
    assert!(
        field
            .set_data_type(DataType::Time32(TimeUnit::Nanosecond))
            .is_err()
    );
    assert_eq!(field.data_type(), &DataType::Utf8);
}

#[test]
fn metadata_is_a_deterministic_shared_native_value() {
    let metadata = Metadata::from_entries([("z", "last"), ("a", "first")]).unwrap();
    assert_eq!(
        metadata.iter().collect::<Vec<_>>(),
        [("a", "first"), ("z", "last")]
    );
    assert_eq!(metadata.next_entry(None), Some(("a", "first")));
    assert_eq!(metadata.next_entry(Some("a")), Some(("z", "last")));

    let json = metadata.to_json().unwrap();
    assert_eq!(json, r#"{"a":"first","z":"last"}"#);
    assert_eq!(Metadata::from_json(&json).unwrap(), metadata);
    assert_eq!(metadata.to_string().parse::<Metadata>().unwrap(), metadata);
    assert_eq!(metadata.clone().into_json().unwrap(), json);
    assert_eq!(metadata.clone().into_iter().count(), 2);
    assert_eq!(
        metadata.as_ref().get("a").map(String::as_str),
        Some("first")
    );
    assert_eq!(metadata.stable_hash(), metadata.clone().stable_hash());

    let arrow = metadata.clone().into_arrow();
    assert_eq!(Metadata::from_arrow(&arrow).unwrap(), metadata);
}

#[test]
fn typed_names_location_and_protocol_properties_share_one_metadata_map() {
    let location = Url::from_str("HTTPS://example.com/warehouse/table").unwrap();
    let mut field = Field::new("trade", DataType::Utf8, false);
    field.set_alias("latest_trade").unwrap();
    field.set_catalog_name("analytics").unwrap();
    field.set_schema_name("public").unwrap();
    field.set_table_name("trades").unwrap();
    field.set_location(location.clone());
    assert_eq!(field.alias(), Some("latest_trade"));
    assert_eq!(field.catalog_name(), Some("analytics"));
    assert_eq!(field.schema_name(), Some("public"));
    assert_eq!(field.table_name(), Some("trades"));
    assert_eq!(field.location().unwrap(), Some(location.clone()));
    assert_eq!(
        field.get_metadata("location"),
        Some(location.to_string().as_str())
    );

    assert_eq!(
        field
            .set_property(&Scheme::POSTGRES, "ddl", "CREATE TABLE trades")
            .unwrap(),
        None
    );
    field
        .set_property(&Scheme::POSTGRES, "comment", "line one\nline two")
        .unwrap();
    field.set_property(&Scheme::POSTGRES, "empty", "").unwrap();
    field
        .set_property(&Scheme::ICEBERG, "format-version", "2")
        .unwrap();
    assert!(field.has_property(&Scheme::POSTGRES, "ddl"));
    assert_eq!(
        field.property_iter(&Scheme::POSTGRES).collect::<Vec<_>>(),
        [
            ("comment", "line one\nline two"),
            ("ddl", "CREATE TABLE trades"),
            ("empty", "")
        ]
    );
    assert_eq!(
        field.next_property_entry(&Scheme::POSTGRES, Some("comment")),
        Some(("ddl", "CREATE TABLE trades"))
    );

    let cached = field.to_arrow_ref().unwrap();
    field.set_alias("latest_trade").unwrap();
    field
        .set_property(&Scheme::POSTGRES, "ddl", "CREATE TABLE trades")
        .unwrap();
    field.set_location(location);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    field.clear_properties(&Scheme::POSTGRES);
    assert!(!field.has_property(&Scheme::POSTGRES, "ddl"));
    assert_eq!(
        field.get_property(&Scheme::ICEBERG, "format-version"),
        Some("2")
    );
    assert!(!Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    assert_eq!(field.remove_alias().as_deref(), Some("latest_trade"));
    assert_eq!(field.remove_catalog_name().as_deref(), Some("analytics"));
    assert_eq!(field.remove_schema_name().as_deref(), Some("public"));
    assert_eq!(field.remove_table_name().as_deref(), Some("trades"));
    assert!(field.remove_location().unwrap().is_some());
}

#[test]
fn http_metadata_is_canonical_typed_and_cache_aware() {
    let mut field = Field::from_parts(
        "payload",
        DataType::Binary,
        false,
        [
            ("HTTP:Content-Type", "text/plain; charset=utf-8"),
            ("HtTp:Content-Length", "00042"),
            ("http:ETag", "\"revision-1\""),
        ],
    )
    .unwrap();

    assert_eq!(field.content_type(), Some("text/plain; charset=utf-8"));
    assert_eq!(field.content_length().unwrap(), Some(42));
    assert_eq!(field.etag(), Some("\"revision-1\""));
    assert_eq!(field.get_metadata("HTTP:CONTENT-LENGTH"), Some("42"));
    assert_eq!(field.get_metadata("http:content-length"), Some("42"));

    let cached = field.to_arrow_ref().unwrap();
    field.set_content_type("text/plain; charset=utf-8").unwrap();
    field.set_content_length(42);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    field.set_cache_control("public, max-age=60").unwrap();
    assert_eq!(field.cache_control(), Some("public, max-age=60"));
    assert!(!Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    let unchanged = field.clone();
    let cached = field.to_arrow_ref().unwrap();
    assert!(field.set_etag("good\r\nInjected: bad").is_err());
    assert_eq!(field, unchanged);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    assert_eq!(field.remove_content_length().unwrap(), Some(42));
    assert_eq!(field.remove_content_length().unwrap(), None);
    assert_eq!(
        field.remove_metadata("HTTP:ETAG").as_deref(),
        Some("\"revision-1\"")
    );
}

#[test]
fn http_case_collisions_and_typed_location_are_transactional() {
    let mut field = Field::new("payload", DataType::Binary, false);
    field.set_accept("application/json").unwrap();
    let snapshot = field.clone();
    let cached = field.to_arrow_ref().unwrap();
    assert!(
        field
            .set_metadata([
                ("HTTP:Accept", "application/json"),
                ("http:accept", "text/csv"),
            ])
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    field
        .insert_metadata("HTTP:Location", "../relative/resource")
        .unwrap();
    assert!(field.http_location().is_err());
    assert_eq!(
        field.get_metadata("http:location"),
        Some("../relative/resource")
    );
    let before_remove = field.clone();
    assert!(field.remove_http_location().is_err());
    assert_eq!(field, before_remove);

    let absolute = Url::from_str("HTTPS://example.test/data").unwrap();
    field.set_http_location(absolute.clone());
    assert_eq!(field.http_location().unwrap(), Some(absolute));
    assert_eq!(
        field
            .remove_http_location()
            .unwrap()
            .map(|value| value.to_string()),
        Some("https://example.test/data".to_owned())
    );
}

#[test]
fn https_properties_share_the_canonical_http_namespace() {
    let mut field = Field::new("payload", DataType::Binary, false);
    assert_eq!(
        field
            .set_property(&Scheme::HTTPS, "Content-Type", "application/json")
            .unwrap(),
        None
    );
    assert_eq!(field.content_type(), Some("application/json"));
    assert_eq!(
        field.get_property(&Scheme::HTTP, "CONTENT-TYPE"),
        Some("application/json")
    );
    assert_eq!(
        field.get_property(&Scheme::HTTPS, "content-type"),
        Some("application/json")
    );
    assert_eq!(
        field.property_iter(&Scheme::HTTPS).collect::<Vec<_>>(),
        [("content-type", "application/json")]
    );

    let cached = field.to_arrow_ref().unwrap();
    let snapshot = field.clone();
    assert!(
        field
            .set_property(&Scheme::HTTPS, "X-Trace", "safe\r\ninjected")
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    assert_eq!(
        field
            .set_property(&Scheme::HTTP, "CONTENT-TYPE", "text/csv")
            .unwrap()
            .as_deref(),
        Some("application/json")
    );
    assert_eq!(field.metadata_len(), 1);
    assert_eq!(
        field
            .remove_property(&Scheme::HTTPS, "Content-Type")
            .as_deref(),
        Some("text/csv")
    );

    field
        .set_property(&Scheme::HTTP, "accept", "application/json")
        .unwrap();
    field
        .set_property(&Scheme::HTTPS, "vary", "accept")
        .unwrap();
    field.clear_properties(&Scheme::HTTPS);
    assert!(field.property_iter(&Scheme::HTTP).next().is_none());

    assert!(
        Field::from_parts(
            "collision",
            DataType::Binary,
            false,
            [
                ("HTTPS:Content-Type", "application/json"),
                ("HTTP:content-type", "text/csv"),
            ],
        )
        .is_err()
    );
}

#[test]
fn typed_http_media_preserves_raw_parameters_and_encoding_order() {
    let field = Field::from_parts(
        "payload",
        DataType::Binary,
        false,
        [
            ("HTTP:Content-Type", "Application/JSON; Charset=utf-8"),
            ("HTTP:Content-Encoding", "gzip, br"),
        ],
    )
    .unwrap();

    assert_eq!(
        field.content_type(),
        Some("Application/JSON; Charset=utf-8")
    );
    assert_eq!(field.mime_type().unwrap(), MimeType::JSON);
    let media = field.media_type().unwrap();
    assert_eq!(media.base(), &MimeType::JSON);
    assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::BROTLI]);
    assert_eq!(
        Field::new("empty", DataType::Binary, true)
            .mime_type()
            .unwrap(),
        MimeType::OCTET_STREAM
    );
}

#[test]
fn typed_http_media_pair_updates_once_and_rejects_unmappable_encodings() {
    let media = MediaType::from_parts(
        MimeType::CSV,
        [MimeType::GZIP, MimeType::COMPRESS, MimeType::ZSTD],
    )
    .unwrap();
    let mut field = Field::new("payload", DataType::Binary, false);
    field.set_media_type(media.clone()).unwrap();
    assert_eq!(field.content_type(), Some("text/csv"));
    assert_eq!(field.content_encoding(), Some("gzip, compress, zstd"));
    assert_eq!(field.media_type().unwrap(), media);

    let cached = field.to_arrow_ref().unwrap();
    field.set_media_type(media).unwrap();
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    let unsupported = MediaType::from_parts(MimeType::JSON, [MimeType::BZIP2]).unwrap();
    let unchanged = field.clone();
    assert!(field.set_media_type(unsupported).is_err());
    assert_eq!(field, unchanged);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    field.set_mime_type(MimeType::JSON);
    assert_eq!(field.content_type(), Some("application/json"));
    assert_eq!(field.content_encoding(), Some("gzip, compress, zstd"));
    assert_eq!(field.remove_mime_type().unwrap(), Some(MimeType::JSON));
    assert_eq!(field.content_type(), None);
    assert_eq!(field.content_encoding(), Some("gzip, compress, zstd"));
    assert_eq!(field.media_type().unwrap().base(), &MimeType::OCTET_STREAM);
    assert!(field.remove_media_type().unwrap().is_some());
    assert_eq!(field.content_encoding(), None);
}

#[test]
fn malformed_typed_http_media_removal_is_transactional() {
    let mut field = Field::from_parts(
        "payload",
        DataType::Binary,
        false,
        [
            ("http:content-type", "application/json; charset=utf-8"),
            ("http:content-encoding", "identity"),
        ],
    )
    .unwrap();
    let snapshot = field.clone();
    let cached = field.to_arrow_ref().unwrap();
    assert!(field.media_type().is_err());
    assert!(field.remove_media_type().is_err());
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    let duplicate = Field::from_parts(
        "duplicate",
        DataType::Binary,
        false,
        [("http:content-encoding", " gzip ,\tGZIP ")],
    )
    .unwrap();
    assert_eq!(
        duplicate.media_type().unwrap().encodings(),
        &[MimeType::GZIP, MimeType::GZIP]
    );
    assert_eq!(duplicate.content_encoding(), Some(" gzip ,\tGZIP "));

    for coding in ["", "identity", "gzip,", "unknown-coding"] {
        let raw = Field::from_parts(
            "invalid",
            DataType::Binary,
            false,
            [("http:content-encoding", coding)],
        )
        .unwrap();
        assert_eq!(raw.content_encoding(), Some(coding));
        assert!(raw.media_type().is_err(), "accepted coding {coding:?}");
    }
}

#[test]
fn typed_field_id_uses_canonical_arrow_parquet_metadata() {
    let imported_arrow = Arc::new(
        ArrowField::new("trade", ArrowDataType::Utf8, false).with_metadata(
            std::collections::HashMap::from([("PARQUET:field_id".to_owned(), "+00017".to_owned())]),
        ),
    );
    let imported = Field::from_arrow_ref(Arc::clone(&imported_arrow)).unwrap();
    assert_eq!(imported.parquet_field_id().unwrap(), Some(17));
    let canonical_arrow = imported.to_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&imported_arrow, &canonical_arrow));
    assert_eq!(
        canonical_arrow
            .metadata()
            .get("PARQUET:field_id")
            .map(String::as_str),
        Some("17")
    );
    assert_eq!(
        Field::from_arrow(canonical_arrow.as_ref())
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(17)
    );

    let mut field = Field::from_parts(
        "trade",
        DataType::Utf8,
        false,
        [("PARQUET:field_id", "+00017")],
    )
    .unwrap();
    assert_eq!(field.parquet_field_id().unwrap(), Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));

    let cached = field.to_arrow_ref().unwrap();
    field.set_parquet_field_id(17);
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    field.set_parquet_field_id(i32::MIN);
    assert_eq!(field.parquet_field_id().unwrap(), Some(i32::MIN));
    assert!(!Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));
    assert_eq!(
        field
            .to_arrow_ref()
            .unwrap()
            .metadata()
            .get("PARQUET:field_id")
            .map(String::as_str),
        Some("-2147483648")
    );

    field.set_parquet_field_id(i32::MAX);
    let json = field.to_json().unwrap();
    let restored = Field::from_json(&json).unwrap();
    assert_eq!(restored.parquet_field_id().unwrap(), Some(i32::MAX));
    assert_eq!(field.remove_parquet_field_id().unwrap(), Some(i32::MAX));
    assert_eq!(field.remove_parquet_field_id().unwrap(), None);
}

#[test]
fn typed_field_id_rejects_non_i32_metadata_transactionally() {
    for value in ["", "1.0", " 1", "2147483648", "-2147483649"] {
        assert!(
            Field::from_parts(
                "trade",
                DataType::Utf8,
                false,
                [("PARQUET:field_id", value)],
            )
            .is_err(),
            "accepted invalid field ID {value:?}"
        );
    }
    assert!(Metadata::from_json(r#"{"PARQUET:field_id":"2147483648"}"#).is_err());
    let metadata = Metadata::from_arrow(&std::collections::HashMap::from([(
        "PARQUET:field_id".to_owned(),
        "-0007".to_owned(),
    )]))
    .unwrap();
    assert_eq!(metadata.get("PARQUET:field_id"), Some("-7"));

    let mut field = Field::new("trade", DataType::Utf8, false).with_parquet_field_id(7);
    let snapshot = field.clone();
    assert!(
        field
            .insert_metadata("PARQUET:field_id", "not-an-integer")
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(
        Field::from_str(
            r#"field("trade",utf8,nullable=false,metadata={"PARQUET:field_id":"2147483648"})"#,
        )
        .is_err()
    );

    let arrow = ArrowField::new("trade", ArrowDataType::Utf8, false).with_metadata(
        std::collections::HashMap::from([(
            "PARQUET:field_id".to_owned(),
            "not-an-integer".to_owned(),
        )]),
    );
    assert!(Field::from_arrow(&arrow).is_err());
}

#[test]
fn reserved_metadata_is_transactional_and_arbitrary_arrow_keys_are_preserved() {
    let mut field = Field::from_parts(
        "id",
        DataType::Int64,
        false,
        [("ARROW:extension:name", ""), ("note", "line one\nline two")],
    )
    .unwrap();
    let snapshot = field.clone();
    assert!(field.set_alias("").is_err());
    assert!(field.set_table_name("bad\nname").is_err());
    assert!(field.set_property(&Scheme::POSTGRES, "", "value").is_err());
    assert!(
        field
            .set_property(&Scheme::POSTGRES, "bad\nname", "value")
            .is_err()
    );
    assert!(field.insert_metadata("location", "not a URL").is_err());
    assert_eq!(field, snapshot);
    assert_eq!(field.get_metadata("ARROW:extension:name"), Some(""));
    assert_eq!(field.get_metadata("note"), Some("line one\nline two"));

    let arrow = ArrowField::new("id", ArrowDataType::Int64, false).with_metadata(
        std::collections::HashMap::from([("location".to_owned(), "invalid".to_owned())]),
    );
    assert!(Field::from_arrow(&arrow).is_err());
}

#[test]
fn arrow_cache_is_rebuilt_when_typed_metadata_is_canonicalized() {
    let arrow = Arc::new(
        ArrowField::new("id", ArrowDataType::Int64, false).with_metadata(
            std::collections::HashMap::from([(
                "location".to_owned(),
                "HTTPS://example.com/table".to_owned(),
            )]),
        ),
    );
    let field = Field::from_arrow_ref(Arc::clone(&arrow)).unwrap();
    assert_eq!(
        field.location().unwrap().map(|url| url.to_string()),
        Some("https://example.com/table".to_owned())
    );

    let projected = field.to_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&arrow, &projected));
    assert_eq!(
        projected.metadata().get("location").map(String::as_str),
        Some("https://example.com/table")
    );
}

fn arrow_field_with_nested_noncanonical_location() -> ArrowField {
    let leaf = Arc::new(
        ArrowField::new("item", ArrowDataType::Int64, false).with_metadata(
            std::collections::HashMap::from([(
                "location".to_owned(),
                "HTTPS://example.com/table".to_owned(),
            )]),
        ),
    );
    let items = Arc::new(ArrowField::new("items", ArrowDataType::List(leaf), false));
    ArrowField::new("root", ArrowDataType::Struct(vec![items].into()), false)
}

fn nested_location(field: &ArrowField) -> Option<&str> {
    let ArrowDataType::Struct(fields) = field.data_type() else {
        return None;
    };
    let ArrowDataType::List(item) = fields.first()?.data_type() else {
        return None;
    };
    item.metadata().get("location").map(String::as_str)
}

#[test]
fn borrowed_arrow_import_rebuilds_parent_for_nested_canonicalization() {
    let arrow = arrow_field_with_nested_noncanonical_location();
    let field = Field::from_arrow(&arrow).unwrap();
    let projected = field.to_arrow().unwrap();

    assert_eq!(nested_location(&arrow), Some("HTTPS://example.com/table"));
    assert_eq!(
        nested_location(&projected),
        Some("https://example.com/table")
    );
}

#[test]
fn shared_arrow_import_rebuilds_parent_for_nested_canonicalization() {
    let arrow = Arc::new(arrow_field_with_nested_noncanonical_location());
    let field = Field::from_arrow_ref(Arc::clone(&arrow)).unwrap();
    let projected = field.to_arrow_ref().unwrap();

    assert!(!Arc::ptr_eq(&arrow, &projected));
    assert_eq!(
        nested_location(&projected),
        Some("https://example.com/table")
    );

    let canonical = Field::from_arrow_ref(Arc::clone(&projected)).unwrap();
    assert!(Arc::ptr_eq(&projected, &canonical.to_arrow_ref().unwrap()));
}

#[test]
fn owned_arrow_import_rebuilds_parent_for_nested_canonicalization() {
    let field = Field::try_from(arrow_field_with_nested_noncanonical_location()).unwrap();
    let projected = field.into_arrow().unwrap();

    assert_eq!(
        nested_location(&projected),
        Some("https://example.com/table")
    );
}

#[test]
fn field_parser_validates_typed_metadata_and_round_trips_protocol_values() {
    for invalid in [
        r#"field("id",int64,nullable=false,metadata={"alias":""})"#,
        r#"field("id",int64,nullable=false,metadata={"postgres:":"value"})"#,
        r#"field("id",int64,nullable=false,metadata={"location":"invalid"})"#,
    ] {
        assert!(
            matches!(
                Field::from_str(invalid),
                Err(Error::Parse {
                    target: "field",
                    ..
                })
            ),
            "{invalid}"
        );
    }

    let field = Field::from_str(
        r#"field("id",int64,nullable=false,metadata={"postgres:comment":"line one\nline two","postgres:empty":""})"#,
    )
    .unwrap();
    assert_eq!(
        field.get_property(&Scheme::POSTGRES, "comment"),
        Some("line one\nline two")
    );
    assert_eq!(field.get_property(&Scheme::POSTGRES, "empty"), Some(""));
    assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
}

#[test]
fn the_init_flag_defaults_to_true_and_stores_only_when_false() {
    let mut field = Field::new("total", DataType::Int64, true);

    // An ordinary field participates in initialization and carries no key.
    assert!(field.is_init().unwrap());
    assert!(!field.has_metadata("field:init"));

    // Marking it derived stores exactly one canonical value.
    field.set_init(false);
    assert!(!field.is_init().unwrap());
    assert_eq!(field.get_metadata("field:init"), Some("false"));

    // Restoring the default removes the key rather than storing `true`.
    field.set_init(true);
    assert!(field.is_init().unwrap());
    assert!(!field.has_metadata("field:init"));

    // The consuming form mirrors the setter.
    let derived = Field::new("total", DataType::Int64, true).with_init(false);
    assert!(!derived.is_init().unwrap());
}

#[test]
fn the_init_flag_rejects_a_non_boolean_spelling() {
    let error = Field::from_parts("total", DataType::Int64, true, [("field:init", "yes")])
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected true or false"), "{error}");
    assert!(error.contains("\"yes\""), "{error}");

    // The canonical spellings are accepted and round-trip.
    for (text, expected) in [("true", true), ("false", false)] {
        let field =
            Field::from_parts("total", DataType::Int64, true, [("field:init", text)]).unwrap();
        assert_eq!(field.is_init().unwrap(), expected, "{text}");
    }
}

#[test]
fn datatype_builds_fields_in_schema_reading_order() {
    let id = DataType::Int64.field("id", false);
    assert_eq!(id, Field::new("id", DataType::Int64, false));

    assert_eq!(
        DataType::Utf8.nullable_field("note"),
        Field::new("note", DataType::Utf8, true)
    );
    assert_eq!(
        DataType::Utf8.required_field("symbol"),
        Field::new("symbol", DataType::Utf8, false)
    );

    // A nested type composes without naming the inner type twice.
    let tags = DataType::list(DataType::Utf8.nullable_field("item")).nullable_field("tags");
    assert!(tags.data_type().is_nested());
    assert_eq!(tags.name(), "tags");
}

#[test]
fn a_struct_field_is_usable_as_a_schema_root() {
    let root = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row");

    // A struct field carries everything a schema needs: the column list, the
    // per-column lookup, and validation of its own root-ness.
    assert!(root.is_struct());
    root.validate_struct_root().unwrap();
    assert_eq!(root.field_len(), 2);
    assert_eq!(root.fields().len(), 2);
    assert_eq!(root.index_of("symbol"), Some(1));
    assert_eq!(root.index_of("absent"), None);
    assert_eq!(root.get_field(0).unwrap().name(), "id");
    assert_eq!(root.get_field_by_name("symbol").unwrap().name(), "symbol");
}

#[test]
fn a_root_must_be_a_non_null_struct_and_says_why() {
    let scalar = DataType::Int64.required_field("value");
    assert!(!scalar.is_struct());
    assert!(scalar.fields().is_empty());
    let message = scalar.validate_struct_root().unwrap_err().to_string();
    assert!(message.contains("expected a struct root"), "{message}");
    assert!(message.contains("int64"), "{message}");

    let nullable = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .nullable_field("row");
    let message = nullable.validate_struct_root().unwrap_err().to_string();
    assert!(message.contains("non-null struct root"), "{message}");
    assert!(message.contains("\"row\""), "{message}");
}

#[test]
fn a_protocol_view_reads_and_writes_by_bare_name_over_one_shared_map() {
    let mut field = DataType::Int64.required_field("price");
    field.iceberg_mut().insert("doc", "closing price").unwrap();
    field
        .iceberg_mut()
        .update([("schema-id", "3"), ("field-id", "7")])
        .unwrap();
    field.postgres_mut().insert("comment", "trades").unwrap();

    // The view spells the key once, so a caller never assembles one.
    assert_eq!(field.iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.iceberg().prefix(), "iceberg");
    assert_eq!(field.iceberg().scheme(), &Scheme::ICEBERG);
    assert_eq!(field.iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(&field.iceberg()["schema-id"], "3");
    assert!(field.iceberg().contains_key("field-id"));
    assert!(!field.iceberg().contains_key("comment"));
    assert_eq!(field.iceberg().len(), 3);
    assert_eq!(field.postgres().len(), 1);
    assert!(field.mysql().is_empty());

    // It reads out of the same map every other metadata accessor reads.
    assert_eq!(
        field.iceberg().iter().collect::<Vec<_>>(),
        [
            ("doc", "closing price"),
            ("field-id", "7"),
            ("schema-id", "3")
        ]
    );
    assert_eq!(
        field.iceberg().next_entry(Some("doc")),
        Some(("field-id", "7"))
    );
    assert_eq!(
        field.iceberg().to_string(),
        r#"{"doc":"closing price","field-id":"7","schema-id":"3"}"#
    );
    assert_eq!(field.metadata_len(), 4);

    // A protocol-scoped replacement leaves every other protocol alone.
    field
        .iceberg_mut()
        .set([("doc", "close"), ("sort-order-id", "1")])
        .unwrap();
    assert_eq!(
        field.iceberg().iter().collect::<Vec<_>>(),
        [("doc", "close"), ("sort-order-id", "1")]
    );
    assert_eq!(field.postgres().get("comment"), Some("trades"));

    assert_eq!(field.iceberg_mut().remove("doc").as_deref(), Some("close"));
    field.iceberg_mut().clear();
    assert!(field.iceberg().is_empty());
    assert_eq!(field.postgres().len(), 1);
}

#[test]
fn a_protocol_view_shares_http_between_the_two_schemes_and_stays_case_insensitive() {
    let mut field = DataType::Utf8.required_field("body");
    field
        .http_mut()
        .insert("Content-Type", "text/plain")
        .unwrap();

    assert_eq!(field.http().get("content-type"), Some("text/plain"));
    assert_eq!(field.http().get("CONTENT-TYPE"), Some("text/plain"));
    assert_eq!(field.http().key("Content-Type"), "http:Content-Type");
    assert_eq!(
        field.protocol(&Scheme::HTTPS).get("Content-Type"),
        Some("text/plain")
    );
    assert_eq!(field.protocol(&Scheme::HTTPS).prefix(), "http");
    assert_eq!(field.content_type(), Some("text/plain"));
    assert_eq!(field.get_metadata("http:content-type"), Some("text/plain"));

    // The view is a borrow of the field's own snapshot, not a copy of it.
    let metadata = field.as_metadata().clone();
    assert_eq!(metadata.http(), field.http());
    assert_eq!(
        metadata
            .http()
            .to_metadata()
            .unwrap()
            .get("http:content-type"),
        Some("text/plain")
    );
}

#[test]
fn a_protocol_write_invalidates_the_arrow_cache_exactly_once() {
    let mut field = DataType::Int64.required_field("price");
    let cached = field.to_arrow_ref().unwrap();
    field.iceberg_mut().insert("doc", "close").unwrap();
    assert!(!Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    let cached = field.to_arrow_ref().unwrap();
    field.iceberg_mut().insert("doc", "close").unwrap();
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));

    // A rejected value leaves the field, and its cache, untouched.
    assert!(field.iceberg_mut().insert("", "no name").is_err());
    assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));
    assert_eq!(field.iceberg().len(), 1);
}

#[test]
fn a_field_can_act_as_a_partition_column_and_a_root_reports_only_those() {
    let schema = DataType::from_fields([
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("venue"),
        DataType::Int64.required_field("price"),
    ])
    .unwrap()
    .required_field("row")
    .with_partition_fields(&["year", "venue"])
    .unwrap();

    assert!(schema.has_partition_fields());
    assert_eq!(schema.partition_field_len(), 2);
    assert_eq!(
        schema.partition_field_names().collect::<Vec<_>>(),
        ["year", "venue"]
    );
    assert_eq!(
        schema
            .partition_fields()
            .rev()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["venue", "year"]
    );
    assert!(schema.get_field_by_name("year").unwrap().is_partition());
    assert!(!schema.get_field_by_name("price").unwrap().is_partition());

    // The two halves of the layout: what a path spells, and what a leaf stores.
    let stored = schema.without_partition_fields().unwrap();
    assert_eq!(stored.field_len(), 1);
    assert_eq!(stored.get_field(0).unwrap().name(), "price");
    let partitions = schema.only_partition_fields().unwrap();
    assert_eq!(
        partitions
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["year", "venue"]
    );

    // The marker is reserved metadata, so it round-trips like any other.
    let restored = Field::from_str(&schema.to_string()).unwrap();
    assert_eq!(restored, schema);
    assert_eq!(restored.partition_field_len(), 2);
    assert_eq!(
        schema
            .get_field_by_name("year")
            .unwrap()
            .get_metadata("field:partition"),
        Some("true")
    );
}

#[test]
fn unmarking_a_partition_column_removes_the_marker_rather_than_storing_a_default() {
    let plain = DataType::Int32.required_field("year");
    let marked = plain.clone().with_partition(true);
    assert!(marked.is_partition());
    assert_eq!(marked.clone().with_partition(false), plain);
    assert!(!plain.is_partition());
    assert!(plain.without_partition_fields().is_err());

    // A field that never partitions anything answers the accessors anyway.
    let root = DataType::from_fields([DataType::Int64.required_field("price")])
        .unwrap()
        .required_field("row");
    assert!(!root.has_partition_fields());
    assert_eq!(root.without_partition_fields().unwrap(), root);
    assert_eq!(root.only_partition_fields().unwrap().field_len(), 0);

    // A name the root does not carry is refused by name.
    let message = root
        .with_partition_fields(&["year"])
        .unwrap_err()
        .to_string();
    assert!(message.contains("\"year\""), "{message}");
    assert!(message.contains("partition on"), "{message}");

    // Only the canonical booleans are accepted for the reserved marker.
    assert!(
        Field::from_parts("year", DataType::Int32, false, [("field:partition", "yes")]).is_err()
    );
    assert!(
        !Field::from_parts(
            "year",
            DataType::Int32,
            false,
            [("field:partition", "false")]
        )
        .unwrap()
        .is_partition()
    );
}

#[test]
fn one_walk_numbers_finds_and_bounds_every_identifier_in_a_tree() {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::list(DataType::Utf8.nullable_field("item")).nullable_field("tags"),
        DataType::from_fields([DataType::Int32.required_field("depth")])
            .unwrap()
            .nullable_field("book"),
    ])
    .unwrap()
    .required_field("row");

    // Numbering is depth first in declaration order, over every child a
    // layout has - a list's item and a map's entries included.
    assert_eq!(schema.assign_parquet_field_ids(1).unwrap(), 6);
    assert_eq!(schema.max_parquet_field_id().unwrap(), Some(5));
    assert_eq!(
        schema.field_by_parquet_field_id(1).map(Field::name),
        Some("id")
    );
    assert_eq!(
        schema.field_by_parquet_field_id(3).map(Field::name),
        Some("item")
    );
    assert_eq!(
        schema.field_by_parquet_field_id(5).map(Field::name),
        Some("depth")
    );
    assert_eq!(schema.field_by_parquet_field_id(9), None);

    // A field that already carries an identifier keeps it, so evolving a
    // schema never renumbers the columns that were already there.
    let mut evolved = schema
        .clone()
        .try_with_data_type(
            DataType::from_fields(
                schema
                    .fields()
                    .iter()
                    .cloned()
                    .chain([DataType::Utf8.nullable_field("venue")]),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        evolved
            .assign_parquet_field_ids(schema.max_parquet_field_id().unwrap().unwrap() + 1)
            .unwrap(),
        7
    );
    assert_eq!(
        evolved.field_by_parquet_field_id(1).map(Field::name),
        Some("id")
    );
    assert_eq!(
        evolved.field_by_parquet_field_id(6).map(Field::name),
        Some("venue")
    );
}

#[test]
fn a_datatype_rebuilds_any_layout_from_replacement_children() {
    let list = DataType::list(DataType::Int32.nullable_field("item"));
    assert_eq!(
        list.with_fields([DataType::Int64.nullable_field("item")])
            .unwrap(),
        DataType::list(DataType::Int64.nullable_field("item"))
    );

    // A union keeps its type ids and its mode; only the members change.
    let union = DataType::union(
        [
            (7_i8, DataType::Int64.nullable_field("number")),
            (9_i8, DataType::Utf8.nullable_field("text")),
        ],
        yggdryl::UnionMode::Dense,
    )
    .unwrap();
    let rebuilt = union
        .with_fields([
            DataType::Int32.nullable_field("number"),
            DataType::Utf8.nullable_field("text"),
        ])
        .unwrap();
    assert_eq!(
        rebuilt.to_string(),
        union.to_string().replace("int64", "int32")
    );

    // The arity is the layout's, and a mismatch says which was expected.
    let message = list
        .with_fields([
            DataType::Int64.nullable_field("item"),
            DataType::Int64.nullable_field("extra"),
        ])
        .unwrap_err()
        .to_string();
    assert!(message.contains("1 children"), "{message}");
    assert!(message.contains("got 2"), "{message}");

    // A scalar has no children, so replacing none of them is itself.
    assert_eq!(DataType::Int64.with_fields([]).unwrap(), DataType::Int64);
}
