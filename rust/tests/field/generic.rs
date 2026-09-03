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
    assert_eq!(sql.dtype(), &DataType::Int64);
    assert!(!sql.is_nullable());

    let hive = Field::from_str("events:array<struct<id:bigint,name:string>>").unwrap();
    assert!(hive.is_nullable());
    assert_eq!(hive.dtype().field_len(), 1);

    let arrow = Field::from_str(
        r#"Field { name: "id", dtype: Int64, nullable: false, metadata: {"source":"arrow"} }"#,
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
    assert_eq!(spaced.dtype(), &DataType::Int64);
    assert!(!spaced.is_nullable());

    let single_quoted = Field::from_str("'owner''s code'   VARCHAR(32)").unwrap();
    assert_eq!(single_quoted.name(), "owner's code");
    assert_eq!(single_quoted.dtype(), &DataType::Utf8);

    let double_quoted = Field::from_str(r#""desk""label" STRING"#).unwrap();
    assert_eq!(double_quoted.name(), "desk\"label");
    assert_eq!(double_quoted.dtype(), &DataType::Utf8);
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

    let json = right.clone().into_json().unwrap();
    assert_eq!(Field::from_json(&json).unwrap(), right);
}

#[test]
fn structural_json_uses_tagged_dtypes_and_rejects_bad_shapes() {
    let field = Field::new(
        "payload",
        DataType::from_str("struct<id:bigint,tags:array<string>>").unwrap(),
        false,
    );
    let field_json = field.into_json().unwrap();
    let field_value: serde_json::Value = serde_json::from_str(&field_json).unwrap();
    assert!(field_value.is_object());
    assert_eq!(field_value["dtype"]["type"], "struct");

    assert!(
        Field::from_json(
            r#"{"name":"id","dtype":{"type":"int64"},"nullable":false,"unknown":true}"#,
        )
        .is_err()
    );
    assert!(
        Field::from_json(
            r#"{"name":"id","dtype":{"type":"int64"},"nullable":false,"dictionary_id":7}"#,
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
    // Metadata is reached through its view, never by subscripting the node:
    // `field["..."]` descends into children.
    assert_eq!(field.get_metadata("z"), Some("last"));
    assert_eq!(field.as_metadata().get("z"), Some("last"));
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
    let first = Arc::new(field.clone().into_arrow().unwrap());
    let field = Field::from_arrow_ref(Arc::clone(&first)).unwrap();
    let second = field.clone().into_arrow_ref().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(Field::from_arrow(first.as_ref()).unwrap(), field);

    let owned = field.clone().into_arrow().unwrap();
    assert_eq!(Field::try_from(owned).unwrap(), field);
    let shared = field.clone().into_arrow_ref().unwrap();
    let imported = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
    assert_eq!(imported, field);
    assert!(Arc::ptr_eq(&shared, &imported.into_arrow_ref().unwrap()));
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

    let json: serde_json::Value =
        serde_json::from_str(&field.clone().into_json().unwrap()).unwrap();
    assert_eq!(json["dictionary_id"], "41");
    assert_eq!(Field::from_json(&json.to_string()).unwrap(), field);

    let cached = field.clone().into_arrow_ref().unwrap();
    field.set_name("renamed_codes");
    let rebuilt = field.clone().into_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&cached, &rebuilt));
    assert_eq!(rebuilt.dict_id(), Some(41));
    assert_eq!(rebuilt.dict_is_ordered(), Some(true));
}

#[test]
fn no_op_metadata_update_retains_arrow_cache_effective_update_invalidates_it() {
    let field = Field::new("id", DataType::Int64, false)
        .try_with_metadata("source", "one")
        .unwrap();
    let original = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&original)).unwrap();
    field.insert_metadata("source", "one").unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field.insert_metadata("source", "two").unwrap();
    let changed = field.clone().into_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&original, &changed));
}

#[test]
fn invalid_dtype_replacement_is_transactional() {
    let mut field = Field::new("value", DataType::Utf8, true);
    assert!(
        field
            .set_dtype(DataType::Time32(TimeUnit::Nanosecond))
            .is_err()
    );
    assert_eq!(field.dtype(), &DataType::Utf8);
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

    let json = metadata.clone().into_json().unwrap();
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
    field.set_comment("the latest trade").unwrap();
    // Catalog coordinates belong to whichever protocol names them, not to
    // straight metadata.
    field
        .protocol_mut(&Scheme::ICEBERG)
        .insert("table_name", "trades")
        .unwrap();
    field.set_location(location.clone());
    assert_eq!(field.alias(), Some("latest_trade"));
    assert_eq!(field.comment(), Some("the latest trade"));
    assert_eq!(
        field.get_property(&Scheme::ICEBERG, "table_name"),
        Some("trades")
    );
    assert_eq!(field.get_metadata("table_name"), None);
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

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.set_alias("latest_trade").unwrap();
    field
        .set_property(&Scheme::POSTGRES, "ddl", "CREATE TABLE trades")
        .unwrap();
    field.set_location(location);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field.clear_properties(&Scheme::POSTGRES);
    assert!(!field.has_property(&Scheme::POSTGRES, "ddl"));
    assert_eq!(
        field.get_property(&Scheme::ICEBERG, "format-version"),
        Some("2")
    );
    assert!(!Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    assert_eq!(field.remove_alias().as_deref(), Some("latest_trade"));
    assert_eq!(field.remove_comment().as_deref(), Some("the latest trade"));
    assert!(field.remove_location().unwrap().is_some());
}

#[test]
fn http_metadata_is_canonical_typed_and_cache_aware() {
    let field = Field::from_parts(
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

    assert_eq!(
        field.as_http().content_type(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(field.as_http().content_length().unwrap(), Some(42));
    assert_eq!(field.as_http().etag(), Some("\"revision-1\""));
    assert_eq!(field.get_metadata("HTTP:CONTENT-LENGTH"), Some("42"));
    assert_eq!(field.get_metadata("http:content-length"), Some("42"));

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field
        .as_http_mut()
        .set_content_type("text/plain; charset=utf-8")
        .unwrap();
    field.as_http_mut().set_content_length(42);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field
        .as_http_mut()
        .set_cache_control("public, max-age=60")
        .unwrap();
    assert_eq!(field.as_http().cache_control(), Some("public, max-age=60"));
    assert!(!Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    let unchanged = field.clone();
    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    assert!(
        field
            .as_http_mut()
            .set_etag("good\r\nInjected: bad")
            .is_err()
    );
    assert_eq!(field, unchanged);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    assert_eq!(
        field.as_http_mut().remove_content_length().unwrap(),
        Some(42)
    );
    assert_eq!(field.as_http_mut().remove_content_length().unwrap(), None);
    assert_eq!(
        field.remove_metadata("HTTP:ETAG").as_deref(),
        Some("\"revision-1\"")
    );
}

#[test]
fn http_case_collisions_and_typed_location_are_transactional() {
    let mut field = Field::new("payload", DataType::Binary, false);
    field.as_http_mut().set_accept("application/json").unwrap();
    let snapshot = field.clone();
    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    assert!(
        field
            .set_metadata([
                ("HTTP:Accept", "application/json"),
                ("http:accept", "text/csv"),
            ])
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field
        .insert_metadata("HTTP:Location", "../relative/resource")
        .unwrap();
    assert!(field.as_http().location().is_err());
    assert_eq!(
        field.get_metadata("http:location"),
        Some("../relative/resource")
    );
    let before_remove = field.clone();
    assert!(field.as_http_mut().remove_location().is_err());
    assert_eq!(field, before_remove);

    let absolute = Url::from_str("HTTPS://example.test/data").unwrap();
    field.as_http_mut().set_location(absolute.clone());
    assert_eq!(field.as_http().location().unwrap(), Some(absolute));
    assert_eq!(
        field
            .as_http_mut()
            .remove_location()
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
    assert_eq!(field.as_http().content_type(), Some("application/json"));
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

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    let snapshot = field.clone();
    assert!(
        field
            .set_property(&Scheme::HTTPS, "X-Trace", "safe\r\ninjected")
            .is_err()
    );
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

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
        field.as_http().content_type(),
        Some("Application/JSON; Charset=utf-8")
    );
    assert_eq!(field.as_http().mime_type().unwrap(), MimeType::JSON);
    let media = field.as_http().media_type().unwrap();
    assert_eq!(media.base(), &MimeType::JSON);
    assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::BROTLI]);
    assert_eq!(
        Field::new("empty", DataType::Binary, true)
            .as_http()
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
    field.as_http_mut().set_media_type(media.clone()).unwrap();
    assert_eq!(field.as_http().content_type(), Some("text/csv"));
    assert_eq!(
        field.as_http().content_encoding(),
        Some("gzip, compress, zstd")
    );
    assert_eq!(field.as_http().media_type().unwrap(), media);

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.as_http_mut().set_media_type(media).unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    let unsupported = MediaType::from_parts(MimeType::JSON, [MimeType::BZIP2]).unwrap();
    let unchanged = field.clone();
    assert!(field.as_http_mut().set_media_type(unsupported).is_err());
    assert_eq!(field, unchanged);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field.as_http_mut().set_mime_type(MimeType::JSON);
    assert_eq!(field.as_http().content_type(), Some("application/json"));
    assert_eq!(
        field.as_http().content_encoding(),
        Some("gzip, compress, zstd")
    );
    assert_eq!(
        field.as_http_mut().remove_mime_type().unwrap(),
        Some(MimeType::JSON)
    );
    assert_eq!(field.as_http().content_type(), None);
    assert_eq!(
        field.as_http().content_encoding(),
        Some("gzip, compress, zstd")
    );
    assert_eq!(
        field.as_http().media_type().unwrap().base(),
        &MimeType::OCTET_STREAM
    );
    assert!(field.as_http_mut().remove_media_type().unwrap().is_some());
    assert_eq!(field.as_http().content_encoding(), None);
}

#[test]
fn malformed_typed_http_media_removal_is_transactional() {
    let field = Field::from_parts(
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
    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    assert!(field.as_http().media_type().is_err());
    assert!(field.as_http_mut().remove_media_type().is_err());
    assert_eq!(field, snapshot);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    let duplicate = Field::from_parts(
        "duplicate",
        DataType::Binary,
        false,
        [("http:content-encoding", " gzip ,\tGZIP ")],
    )
    .unwrap();
    assert_eq!(
        duplicate.as_http().media_type().unwrap().encodings(),
        &[MimeType::GZIP, MimeType::GZIP]
    );
    assert_eq!(
        duplicate.as_http().content_encoding(),
        Some(" gzip ,\tGZIP ")
    );

    for coding in ["", "identity", "gzip,", "unknown-coding"] {
        let raw = Field::from_parts(
            "invalid",
            DataType::Binary,
            false,
            [("http:content-encoding", coding)],
        )
        .unwrap();
        assert_eq!(raw.as_http().content_encoding(), Some(coding));
        assert!(
            raw.as_http().media_type().is_err(),
            "accepted coding {coding:?}"
        );
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
    let canonical_arrow = imported.into_arrow_ref().unwrap();
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

    let field = Field::from_parts(
        "trade",
        DataType::Utf8,
        false,
        [("PARQUET:field_id", "+00017")],
    )
    .unwrap();
    assert_eq!(field.parquet_field_id().unwrap(), Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.set_parquet_field_id(17);
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    field.set_parquet_field_id(i32::MIN);
    assert_eq!(field.parquet_field_id().unwrap(), Some(i32::MIN));
    assert!(!Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    assert_eq!(
        field
            .clone()
            .into_arrow_ref()
            .unwrap()
            .metadata()
            .get("PARQUET:field_id")
            .map(String::as_str),
        Some("-2147483648")
    );

    field.set_parquet_field_id(i32::MAX);
    let json = field.clone().into_json().unwrap();
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
    assert!(field.set_comment("bad\nname").is_err());
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

    let projected = field.into_arrow_ref().unwrap();
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
    let projected = field.into_arrow().unwrap();

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
    let projected = field.into_arrow_ref().unwrap();

    assert!(!Arc::ptr_eq(&arrow, &projected));
    assert_eq!(
        nested_location(&projected),
        Some("https://example.com/table")
    );

    let canonical = Field::from_arrow_ref(Arc::clone(&projected)).unwrap();
    assert!(Arc::ptr_eq(
        &projected,
        &canonical.into_arrow_ref().unwrap()
    ));
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
    let id = DataType::Int64.named_field("id", false);
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
    assert!(tags.dtype().is_nested());
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
    assert_eq!(root.get_field_by_path("symbol").unwrap().name(), "symbol");
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
    field
        .as_iceberg_mut()
        .insert("doc", "closing price")
        .unwrap();
    field
        .as_iceberg_mut()
        .update([("schema-id", "3"), ("field-id", "7")])
        .unwrap();
    field.as_postgres_mut().insert("comment", "trades").unwrap();

    // The view spells the key once, so a caller never assembles one.
    assert_eq!(field.as_iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.as_iceberg().prefix(), "iceberg");
    assert_eq!(field.as_iceberg().scheme(), &Scheme::ICEBERG);
    assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(&field.as_iceberg()["schema-id"], "3");
    assert!(field.as_iceberg().contains_key("field-id"));
    assert!(!field.as_iceberg().contains_key("comment"));
    assert_eq!(field.as_iceberg().len(), 3);
    assert_eq!(field.as_postgres().len(), 1);
    assert!(field.as_mysql().is_empty());

    // It reads out of the same map every other metadata accessor reads.
    assert_eq!(
        field.as_iceberg().iter().collect::<Vec<_>>(),
        [
            ("doc", "closing price"),
            ("field-id", "7"),
            ("schema-id", "3")
        ]
    );
    assert_eq!(
        field.as_iceberg().next_entry(Some("doc")),
        Some(("field-id", "7"))
    );
    assert_eq!(
        field.as_iceberg().to_string(),
        r#"{"doc":"closing price","field-id":"7","schema-id":"3"}"#
    );
    assert_eq!(field.metadata_len(), 4);

    // A protocol-scoped replacement leaves every other protocol alone.
    field
        .as_iceberg_mut()
        .set([("doc", "close"), ("sort-order-id", "1")])
        .unwrap();
    assert_eq!(
        field.as_iceberg().iter().collect::<Vec<_>>(),
        [("doc", "close"), ("sort-order-id", "1")]
    );
    assert_eq!(field.as_postgres().get("comment"), Some("trades"));

    assert_eq!(
        field.as_iceberg_mut().remove("doc").as_deref(),
        Some("close")
    );
    field.as_iceberg_mut().clear();
    assert!(field.as_iceberg().is_empty());
    assert_eq!(field.as_postgres().len(), 1);
}

#[test]
fn a_protocol_view_shares_http_between_the_two_schemes_and_stays_case_insensitive() {
    let mut field = DataType::Utf8.required_field("body");
    field
        .as_http_mut()
        .insert("Content-Type", "text/plain")
        .unwrap();

    assert_eq!(field.as_http().get("content-type"), Some("text/plain"));
    assert_eq!(field.as_http().get("CONTENT-TYPE"), Some("text/plain"));
    assert_eq!(field.as_http().key("Content-Type"), "http:Content-Type");
    assert_eq!(
        field.protocol(&Scheme::HTTPS).get("Content-Type"),
        Some("text/plain")
    );
    assert_eq!(field.protocol(&Scheme::HTTPS).prefix(), "http");
    assert_eq!(field.as_http().content_type(), Some("text/plain"));
    assert_eq!(field.get_metadata("http:content-type"), Some("text/plain"));

    // The view is a borrow of the field's own snapshot, not a copy of it.
    let metadata = field.as_metadata().clone();
    assert_eq!(metadata.as_http(), field.as_http().as_properties());
    assert_eq!(
        metadata
            .as_http()
            .into_metadata()
            .unwrap()
            .get("http:content-type"),
        Some("text/plain")
    );
}

#[test]
fn a_protocol_write_invalidates_the_arrow_cache_exactly_once() {
    let field = DataType::Int64.required_field("price");
    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.as_iceberg_mut().insert("doc", "close").unwrap();
    assert!(!Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.as_iceberg_mut().insert("doc", "close").unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));

    // A rejected value leaves the field, and its cache, untouched.
    assert!(field.as_iceberg_mut().insert("", "no name").is_err());
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    assert_eq!(field.as_iceberg().len(), 1);
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
    assert!(schema.get_field_by_path("year").unwrap().is_partition());
    assert!(!schema.get_field_by_path("price").unwrap().is_partition());

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
            .get_field_by_path("year")
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
        .try_with_dtype(
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

#[test]
fn subscripting_a_schema_node_reaches_a_nested_child() {
    let line = DataType::from_fields([
        DataType::Float64.required_field("price"),
        DataType::Int64.required_field("qty"),
    ])
    .unwrap()
    .required_field("line");
    let mut order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        line.clone(),
        DataType::list(DataType::Utf8.nullable_field("tag")).nullable_field("tags"),
        DataType::map_of(DataType::Utf8, DataType::Int64, false)
            .unwrap()
            .nullable_field("counts"),
    ])
    .unwrap()
    .required_field("order");
    order.insert_metadata("owner", "trading").unwrap();

    // By name and by position, on the Field and on the DataType alike.
    assert_eq!(order["id"].dtype(), &DataType::Int64);
    assert_eq!(order[0].name(), "id");
    assert_eq!(order.dtype()["id"].dtype(), &DataType::Int64);
    assert_eq!(order.dtype()[1].name(), "line");

    // Chained descent, two levels and through a List and a Map.
    assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);
    assert_eq!(order["tags"][0].name(), "tag");
    assert_eq!(order["counts"]["entries"]["key"].dtype(), &DataType::Utf8);
    assert_eq!(order["counts"][0]["value"].dtype(), &DataType::Int64);

    // Metadata is not reachable by subscript any more, and is still reachable
    // through its own view and the named accessor.
    assert_eq!(order.get_metadata("owner"), Some("trading"));
    assert_eq!(order.as_metadata().get("owner"), Some("trading"));
    assert!(order.get_field_by_path("owner").is_none());
}

#[test]
#[should_panic(expected = "is not a child of the field")]
fn subscripting_an_absent_child_panics_by_name() {
    let row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let _ = &row["absent"];
}

#[test]
#[should_panic(expected = "so position 3 is out of range")]
fn subscripting_an_absent_child_panics_by_position() {
    let row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let _ = &row[3];
}

#[test]
#[should_panic(expected = "is not a child of the datatype")]
fn subscripting_a_scalar_datatype_panics() {
    let _ = &DataType::Int64["anything"];
}

#[test]
fn child_mutation_replaces_by_position_and_appends_by_unknown_name() {
    let mut row = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("venue"),
    ])
    .unwrap()
    .required_field("row");

    // A known name replaces in place, keeping its position.
    row.set_field_by_path("id", DataType::Utf8.required_field("id"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[0].name(), "id");
    assert_eq!(row["id"].dtype(), &DataType::Utf8);

    // An unknown name appends.
    row.set_field_by_path("price", DataType::Float64.nullable_field("price"))
        .unwrap();
    assert_eq!(row.field_len(), 3);
    assert_eq!(row[2].name(), "price");

    // A position replaces only, and never grows the node.
    row.set_field(1, DataType::Int32.required_field("venue"))
        .unwrap();
    assert_eq!(row.field_len(), 3);
    assert_eq!(row["venue"].dtype(), &DataType::Int32);
    let refused = row
        .set_field(9, DataType::Int64.required_field("late"))
        .unwrap_err();
    assert!(refused.to_string().contains("below 3"), "{refused}");
    assert_eq!(row.field_len(), 3, "a refusal leaves the field unchanged");

    // Removal closes the gap, by either key form.
    let dropped = row.remove_field_by_path("id").unwrap();
    assert_eq!(dropped.name(), "id");
    assert_eq!(row[0].name(), "venue");
    row.remove_field(0).unwrap();
    assert_eq!(row.field_len(), 1);
    assert_eq!(row[0].name(), "price");

    // A non-struct has no children to replace, and says so.
    let mut scalar = DataType::Int64.required_field("id");
    let refused = scalar
        .set_field_by_path("child", DataType::Int64.required_field("child"))
        .unwrap_err();
    assert!(refused.to_string().contains("struct field"), "{refused}");
}

#[test]
fn child_mutation_invalidates_the_arrow_cache_exactly_once() {
    let mut row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let before = row.clone().into_arrow().unwrap();
    assert!(before.data_type().to_string().contains("id"));
    assert!(!before.data_type().to_string().contains("venue"));

    row.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))
        .unwrap();

    // The projection is rebuilt from the mutated field, never served stale.
    let after = row.into_arrow().unwrap();
    assert!(
        after.data_type().to_string().contains("venue"),
        "{}",
        after.data_type()
    );
}

#[test]
fn metadata_merges_as_a_union_the_receiver_wins() {
    let held = Metadata::from_entries([("owner", "held"), ("only_held", "1")]).unwrap();
    let other = Metadata::from_entries([("owner", "other"), ("only_other", "2")]).unwrap();

    let merged = held.merge_with(&other).unwrap();

    // Every key arrives, and the receiver wins the one they disagree on.
    assert_eq!(merged.get("owner"), Some("held"));
    assert_eq!(merged.get("only_held"), Some("1"));
    assert_eq!(merged.get("only_other"), Some("2"));

    // The direction is the whole rule, so the other way round differs.
    assert_eq!(other.merge_with(&held).unwrap().get("owner"), Some("other"));

    // Merging with itself changes nothing.
    assert_eq!(held.merge_with(&held).unwrap(), held);
}

#[test]
fn a_protocol_view_merges_under_its_own_namespace() {
    let mut held = DataType::Int64.required_field("price");
    held.protocol_mut(&Scheme::ICEBERG)
        .insert("doc", "held")
        .unwrap();

    let mut other = DataType::Int64.required_field("price");
    other
        .protocol_mut(&Scheme::ICEBERG)
        .insert("doc", "other")
        .unwrap();
    other
        .protocol_mut(&Scheme::ICEBERG)
        .insert("id", "7")
        .unwrap();

    let merged = held.as_iceberg().merge_with(&other.as_iceberg()).unwrap();
    assert_eq!(merged.get("iceberg:doc"), Some("held"));
    assert_eq!(merged.get("iceberg:id"), Some("7"));

    // Both views contribute bare names, and the result is keyed under the
    // receiver's protocol, so merging across two namespaces still answers one.
    let mut glue = DataType::Int64.required_field("price");
    glue.protocol_mut(&Scheme::GLUE)
        .insert("comment", "from glue")
        .unwrap();

    let crossed = held.as_iceberg().merge_with(&glue.as_glue()).unwrap();
    assert_eq!(crossed.get("iceberg:comment"), Some("from glue"));
    assert!(crossed.get("glue:comment").is_none());
}

#[test]
fn a_mutable_protocol_view_merges_in_place_and_only_adds() {
    let mut source = DataType::Int64.required_field("price");
    source
        .protocol_mut(&Scheme::ICEBERG)
        .insert("doc", "source")
        .unwrap();
    source
        .protocol_mut(&Scheme::ICEBERG)
        .insert("id", "7")
        .unwrap();

    let mut target = DataType::Int64.required_field("price");
    target
        .protocol_mut(&Scheme::ICEBERG)
        .insert("doc", "target")
        .unwrap();
    target
        .protocol_mut(&Scheme::GLUE)
        .insert("comment", "glue")
        .unwrap();

    target
        .protocol_mut(&Scheme::ICEBERG)
        .merge_with(&source.as_iceberg())
        .unwrap();

    // A name already held keeps its value; a new one arrives.
    assert_eq!(target.get_property(&Scheme::ICEBERG, "doc"), Some("target"));
    assert_eq!(target.get_property(&Scheme::ICEBERG, "id"), Some("7"));

    // A scoped merge leaves every other protocol alone.
    assert_eq!(target.get_property(&Scheme::GLUE, "comment"), Some("glue"));
}
