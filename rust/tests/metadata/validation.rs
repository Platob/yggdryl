//! `rust/src/metadata/validation.rs`.

mod generic {

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use std::sync::Arc;

    use yggdryl::{DataType, Field, MediaType, Metadata, MimeType, Scheme, Url};

    #[test]
    fn http_metadata_is_canonical_typed_and_cache_aware() {
        let field = Field::from_parts(
            "payload",
            DataType::binary(),
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
        assert_eq!(field.get_metadata("HTTP:content-length"), Some("42"));

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field
            .as_http_mut()
            .set_content_type("text/plain; charset=utf-8")
            .unwrap();
        field.as_http_mut().set_content_length(42);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        field
            .as_http_mut()
            .set_cache_control("public, max-age=60")
            .unwrap();
        assert_eq!(field.as_http().cache_control(), Some("public, max-age=60"));
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        let unchanged = field.clone();
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        assert!(
            field
                .as_http_mut()
                .set_etag("good\r\nInjected: bad")
                .is_err()
        );
        assert_eq!(field, unchanged);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
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
        let mut field = Field::new("payload", DataType::binary(), false);
        field.as_http_mut().set_accept("application/json").unwrap();
        let snapshot = field.clone();
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        assert!(
            field
                .set_metadata([
                    ("HTTP:Accept", "application/json"),
                    ("HTTP:accept", "text/csv"),
                ])
                .is_err()
        );
        assert_eq!(field, snapshot);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        field
            .insert_metadata("HTTP:Location", "../relative/resource")
            .unwrap();
        assert!(field.as_http().location().is_err());
        assert_eq!(
            field.get_metadata("HTTP:location"),
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
        let mut field = Field::new("payload", DataType::binary(), false);
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

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        let snapshot = field.clone();
        assert!(
            field
                .set_property(&Scheme::HTTPS, "X-Trace", "safe\r\ninjected")
                .is_err()
        );
        assert_eq!(field, snapshot);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
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
                DataType::binary(),
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
            DataType::binary(),
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
        // The charset the header declared rides on the media type rather than
        // being dropped with the rest of the parameters.
        assert_eq!(media.charset(), Some(yggdryl::Charset::Utf8));
        assert_eq!(
            field.as_http().charset().unwrap(),
            Some(yggdryl::Charset::Utf8)
        );
        assert_eq!(
            Field::new("empty", DataType::binary(), true)
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
        let mut field = Field::new("payload", DataType::binary(), false);
        field.as_http_mut().set_media_type(media.clone()).unwrap();
        assert_eq!(field.as_http().content_type(), Some("text/csv"));
        assert_eq!(
            field.as_http().content_encoding(),
            Some("gzip, compress, zstd")
        );
        assert_eq!(field.as_http().media_type().unwrap(), media);

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.as_http_mut().set_media_type(media).unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        let unsupported = MediaType::from_parts(MimeType::JSON, [MimeType::BZIP2]).unwrap();
        let unchanged = field.clone();
        assert!(field.as_http_mut().set_media_type(unsupported).is_err());
        assert_eq!(field, unchanged);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
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
            DataType::binary(),
            false,
            [
                ("HTTP:content-type", "application/json; charset=utf-8"),
                ("HTTP:content-encoding", "identity"),
            ],
        )
        .unwrap();
        let snapshot = field.clone();
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        assert!(field.as_http().media_type().is_err());
        assert!(field.as_http_mut().remove_media_type().is_err());
        assert_eq!(field, snapshot);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        let duplicate = Field::from_parts(
            "duplicate",
            DataType::binary(),
            false,
            [("HTTP:content-encoding", " gzip ,\tGZIP ")],
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
                DataType::binary(),
                false,
                [("HTTP:content-encoding", coding)],
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
                std::collections::HashMap::from([(
                    "PARQUET:field_id".to_owned(),
                    "+00017".to_owned(),
                )]),
            ),
        );
        let imported = Field::from_arrow_field_ref(Arc::clone(&imported_arrow)).unwrap();
        assert_eq!(imported.parquet_field_id().unwrap(), Some(17));
        let canonical_arrow = imported.into_arrow_field_ref().unwrap();
        assert!(!Arc::ptr_eq(&imported_arrow, &canonical_arrow));
        assert_eq!(
            canonical_arrow
                .metadata()
                .get("PARQUET:field_id")
                .map(String::as_str),
            Some("17")
        );
        assert_eq!(
            Field::from_arrow_field(canonical_arrow.as_ref())
                .unwrap()
                .parquet_field_id()
                .unwrap(),
            Some(17)
        );

        let field = Field::from_parts(
            "trade",
            DataType::utf8(),
            false,
            [("PARQUET:field_id", "+00017")],
        )
        .unwrap();
        assert_eq!(field.parquet_field_id().unwrap(), Some(17));
        assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.set_parquet_field_id(17);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        field.set_parquet_field_id(i32::MIN);
        assert_eq!(field.parquet_field_id().unwrap(), Some(i32::MIN));
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        assert_eq!(
            field
                .clone()
                .into_arrow_field_ref()
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
                    DataType::utf8(),
                    false,
                    [("PARQUET:field_id", value)],
                )
                .is_err(),
                "accepted invalid field ID {value:?}"
            );
        }
        assert!(Metadata::from_json(r#"{"PARQUET:field_id":"2147483648"}"#).is_err());
        let metadata = Metadata::from_arrow_metadata(&std::collections::HashMap::from([(
            "PARQUET:field_id".to_owned(),
            "-0007".to_owned(),
        )]))
        .unwrap();
        assert_eq!(metadata.get("PARQUET:field_id"), Some("-7"));

        let mut field = Field::new("trade", DataType::utf8(), false).with_parquet_field_id(7);
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
        assert!(Field::from_arrow_field(&arrow).is_err());
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
        assert!(field.set_display("bad\nname").is_err());
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
        assert!(Field::from_arrow_field(&arrow).is_err());
    }
}

mod metadata {

    use yggdryl::Metadata;
    use yggdryl::Scheme;

    #[test]
    fn http_keys_are_canonical_case_insensitive_and_collision_safe() {
        let metadata = Metadata::from_entries([
            ("HTTP:Content-Type", "text/plain; charset=utf-8"),
            ("HtTpS:X-Custom", "preserved"),
            ("http:Content-Length", "00042"),
        ])
        .unwrap();

        assert_eq!(
            metadata.iter().collect::<Vec<_>>(),
            [
                ("HTTP:content-length", "42"),
                ("HTTP:content-type", "text/plain; charset=utf-8"),
                ("HTTP:x-custom", "preserved"),
            ]
        );
        assert_eq!(
            metadata.get("HTTP:CONTENT-TYPE"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            metadata.get("HTTPS:CONTENT-TYPE"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            metadata.get_property(&Scheme::HTTPS, "X-CUSTOM"),
            Some("preserved")
        );
        assert_eq!(
            metadata.property_iter(&Scheme::HTTPS).collect::<Vec<_>>(),
            [
                ("content-length", "42"),
                ("content-type", "text/plain; charset=utf-8"),
                ("x-custom", "preserved"),
            ]
        );
        assert_eq!(metadata.get("HTTP:content-length"), Some("42"));
        // A protocol key folds on the way in, so every spelling of it reads the
        // one stored entry.
        assert_eq!(
            metadata.get("HTTPS:CONTENT-TYPE"),
            Some("text/plain; charset=utf-8")
        );
        assert!(metadata.contains_key("HTTP:content-type"));

        assert!(
            Metadata::from_entries([
                ("HTTPS:Content-Type", "text/plain"),
                ("HTTP:content-type", "application/json"),
            ])
            .is_err()
        );
    }

    #[test]
    fn http_values_reject_injection_but_allow_horizontal_tab() {
        let metadata = Metadata::from_entries([("HTTPS:X-Trace", "one\ttwo")]).unwrap();
        assert_eq!(metadata.get("HTTP:x-trace"), Some("one\ttwo"));

        for value in ["a\0b", "a\nb", "a\rb", "a\u{1f}b", "a\u{7f}b"] {
            assert!(
                Metadata::from_entries([("https:x-trace", value)]).is_err(),
                "accepted HTTP control value {value:?}"
            );
        }
        for key in ["HTTP:", "HTTP:bad name", "HTTP:bad:name", "HTTP:café"] {
            assert!(
                Metadata::from_entries([(key, "value")]).is_err(),
                "accepted invalid HTTP field name {key:?}"
            );
        }
    }

    #[test]
    fn content_length_requires_ascii_digits_and_u64_range() {
        assert_eq!(
            Metadata::from_entries([("HTTP:content-length", u64::MAX.to_string())])
                .unwrap()
                .get("HTTP:content-length"),
            Some("18446744073709551615")
        );
        for value in ["", "+1", "-1", " 1", "1 ", "١", "18446744073709551616"] {
            assert!(
                Metadata::from_entries([("HTTP:content-length", value)]).is_err(),
                "accepted invalid Content-Length {value:?}"
            );
        }
    }
}
