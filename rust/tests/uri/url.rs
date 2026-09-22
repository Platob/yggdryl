//! `rust/src/uri/url.rs`: hierarchical URLs and the components a join or a
//! filename change replaces.

mod datatype {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use arrow_array::{Array, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;

    use yggdryl::DataType;
    use yggdryl::FieldValue as _;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{
        ArrowCastOptions, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar, StructType,
        UriField, UriType, Url,
    };

    fn url(text: &str) -> Scalar {
        DataType::url().scalar(text).unwrap()
    }

    fn root(field: Field) -> Field {
        StructType::from_fields([field])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    fn text_of(value: &Scalar) -> String {
        match value {
            Scalar::Url(url) => url.to_string(),
            other => panic!("expected a url scalar, got {other:?}"),
        }
    }

    #[test]
    fn datatype_identity_naming_and_serde_are_total() {
        let dtype = DataType::url();
        assert_eq!(dtype.id(), DataTypeId::Url);
        assert_eq!(dtype.kind(), DataTypeKind::Text);
        assert_eq!(dtype.name(), "url");
        assert_eq!(dtype.to_string(), "url");
        assert_eq!("URL".parse::<DataType>().unwrap(), dtype);
        assert_eq!("URL".parse::<DataTypeId>().unwrap(), DataTypeId::Url);
        assert_eq!(DataTypeId::Url.as_str(), "url");
        // In the text family's range, because `as_u8` is a wire contract laid
        // out by family.
        assert_eq!(DataTypeId::Url.as_u8(), 0x64);
        assert_eq!(DataTypeId::Url.fixed_byte_width(), None);
        assert!(!DataTypeId::Url.is_parameterized());
        assert!(DataTypeId::Url.is_string());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();

        assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"url"}"#);
        assert_eq!(DataType::from_json(r#"{"type":"url"}"#).unwrap(), dtype);
    }

    #[test]
    fn a_value_is_canonicalized_and_refuses_what_is_not_a_location() {
        // A scheme folds to lower case and percent-encoding to upper: one spelling
        // per location, whichever spelling was written.
        assert_eq!(
            text_of(&url("HTTPS://example.com/a%2fb?x=1#f")),
            "https://example.com/a%2Fb?x=1#f"
        );
        // A bare platform path is a `file:` URL, which is what the holders address
        // themselves by.
        assert_eq!(text_of(&url("/lake/part.txt")), "file:///lake/part.txt");
        // Re-reading the canonical text answers the same value.
        let once = url("HTTPS://example.com/a");
        assert_eq!(url(&text_of(&once)), once);

        // Relative text names no location, so it is not one.
        assert!(DataType::url().scalar("./relative").is_err());
        assert!(DataType::url().scalar("example.com/x").is_err());
        // An empty text cell entering a non-text column is no value.
        assert_eq!(DataType::url().scalar("").unwrap(), Scalar::Null);
        assert_eq!(DataType::url().scalar(Scalar::Null).unwrap(), Scalar::Null);
    }

    #[test]
    fn the_scalar_carries_the_datatype_and_orders_by_canonical_text() {
        let value = url("https://example.com/b");
        assert_eq!(value.id(), DataTypeId::Url);
        assert_eq!(value.kind(), "url");

        // Lexicographic on the canonical text, which is Arrow's own string
        // ordering over the storage - there is no numeric component to sort by.
        let mut sorted = [
            url("https://example.com/b"),
            url("file:///a"),
            url("https://example.com/a"),
        ];
        sorted.sort();
        assert_eq!(
            sorted.iter().map(text_of).collect::<Vec<_>>(),
            [
                "file:///a",
                "https://example.com/a",
                "https://example.com/b"
            ]
        );

        // Equal values hash equally, which is what a key column needs.
        let hash = |value: &Scalar| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            hash(&url("HTTPS://example.com/a")),
            hash(&url("https://example.com/a"))
        );
    }

    #[test]
    fn structured_text_round_trips_the_canonical_spelling() {
        let value = url("https://example.com/a");
        // The tagged structural form is what carries the datatype back, so a
        // value written as a URL reads as one rather than as text.
        let tagged = serde_json::to_string(&value).unwrap();
        assert_eq!(tagged, r#"{"type":"url","value":"https://example.com/a"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
    }

    #[test]
    fn arrow_stores_canonical_utf8_under_an_extension_name_that_survives_a_round_trip() {
        let field = Field::new("location", DataType::url(), true);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            arrow
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some("yggdryl.url")
        );
        // The extension name is what makes a URL column come back a URL column
        // rather than prose that happens to look like one.
        assert_eq!(
            Field::from_arrow_field(&arrow).unwrap().dtype(),
            &DataType::url()
        );

        let value = url("https://example.com/a");
        let stored = scalar_array(&field, &value).unwrap();
        assert_eq!(stored.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            stored
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "https://example.com/a"
        );
        assert_eq!(scalar_value(&field, stored.as_ref()).unwrap(), value);
    }

    #[test]
    fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
        let target = root(Field::new("location", DataType::url(), true));
        let source_schema = root(Field::new("location", DataType::utf8(), true))
            .into_arrow_schema()
            .unwrap();
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![Arc::new(StringArray::from(vec![
                Some("HTTPS://example.com/a"),
                None,
            ]))],
        )
        .unwrap();
        let cast = target
            .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
            .unwrap();
        let column = cast
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        // Canonicalized on the way in, so the column holds one spelling per
        // location however each row was written.
        assert_eq!(column.value(0), "https://example.com/a");
        assert!(column.is_null(1));

        let bad = RecordBatch::try_new(
            source_schema,
            vec![Arc::new(StringArray::from(vec![Some("./relative")]))],
        )
        .unwrap();
        let error = target
            .cast_arrow_batch(bad, ArrowCastOptions::new().with_safe(false))
            .unwrap_err()
            .to_string();
        assert!(error.contains("row 0"), "{error}");
        assert!(error.contains("does not read as url"), "{error}");
    }

    #[test]
    fn defaults_merges_and_typed_fields_do_not_fall_through() {
        // A location has no zero, so the default is the shortest URL the
        // validator accepts.
        assert_eq!(
            text_of(&DataType::url().default_value().unwrap()),
            "file:///"
        );
        assert!(DataType::url().is_default_value(&url("file:///")).unwrap());

        // Merging into text would drop the validation that makes it a URL, so
        // only an equal type merges.
        assert_eq!(
            DataType::url().merge_with(&DataType::url(), true).unwrap(),
            DataType::url()
        );
        let refused = DataType::url()
            .merge_with(&DataType::utf8(), true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("url"), "{refused}");
        assert!(refused.contains("utf8"), "{refused}");

        let typed = UriField::new("location", UriType::Url, true);
        assert_eq!(typed.dtype(), &DataType::url());
        assert_eq!(typed.typed_dtype(), UriType::Url);
        let typed_field = typed.to_field();
        let scalar = FieldScalar::new(&typed_field, url("https://example.com/a")).unwrap();
        assert_eq!(scalar.dtype(), &DataType::url());
        assert!(FieldScalar::new(&typed.to_field(), 7_i64).is_err());
    }

    #[test]
    fn the_value_type_is_the_one_the_handles_address_themselves_by() {
        // Not a second URL type: the scalar carries `yggdryl::Url`, so a column read
        // out of a table is the value a handle can be opened from.
        let value = url("file:///lake/part.txt");
        let Scalar::Url(held) = &value else {
            panic!("expected a url scalar")
        };
        let held: &Url = held;
        assert_eq!(held.file_name(), Some("part.txt"));
        assert_eq!(held.scheme().as_str(), "file");
    }
}

mod encoding {
    use std::path::PathBuf;
    use yggdryl::Url;

    /// `join_path` takes names; `joinpath` takes URI text.
    ///
    /// One door per kind of input: a platform component is data and is encoded, URI
    /// text is syntax and is validated. Mixing them is why `new URL(name, 'file:')`
    /// mangles a name holding `%` while `pathToFileURL` does not.
    #[test]
    fn join_path_spells_platform_names_and_joinpath_spells_uri_text() {
        let lake = Url::from_str("file:///lake").unwrap();

        for (name, expected) in [
            ("100%.csv", "file:///lake/100%25.csv"),
            ("a b.csv", "file:///lake/a%20b.csv"),
            ("caf\u{e9}.csv", "file:///lake/caf%C3%A9.csv"),
            ("a#b.csv", "file:///lake/a%23b.csv"),
            ("a?b.csv", "file:///lake/a%3Fb.csv"),
            ("a%2Fb.csv", "file:///lake/a%252Fb.csv"),
            ("year=2024", "file:///lake/year=2024"),
        ] {
            let joined = lake.join_path(name).unwrap();
            assert_eq!(joined.to_string(), expected, "{name:?}");
            assert_eq!(joined.file_name(), Some(&expected[13..]), "{name:?}");
            assert_eq!(
                joined.into_path().unwrap(),
                PathBuf::from(format!("/lake/{name}")),
                "{name:?} did not come back"
            );
        }

        // A separator inside the platform path is a component boundary, and a
        // separator inside one name is not.
        assert_eq!(
            lake.join_path("year=2024/a b.csv").unwrap().to_string(),
            "file:///lake/year=2024/a%20b.csv"
        );
        assert_eq!(
            lake.join_path("a/b").unwrap(),
            lake.joinpath("a").unwrap().joinpath("b").unwrap()
        );

        // The URI door takes syntax, so it refuses what a segment cannot carry and
        // keeps an escape the caller already wrote.
        assert_eq!(
            lake.joinpath("a%25b").unwrap().to_string(),
            "file:///lake/a%25b"
        );
        assert!(lake.joinpath("100%.csv").is_err());
        assert!(lake.joinpath("a b.csv").is_err());
        assert!(lake.joinpath("a#b").is_err());
    }

    /// An absolute component replaces the URL rather than extending it twice.
    ///
    /// `Path::components` yields the root and then the names under it, so the
    /// conversion of the whole path already holds them: continuing the loop past
    /// the root appends every one of them a second time.
    #[test]
    fn join_path_replaces_the_url_when_the_component_is_absolute() {
        let lake = Url::from_str("file:///lake/trades").unwrap();

        for (joined, expected) in [
            ("/a/b", "file:///a/b"),
            ("/x", "file:///x"),
            ("/a/b/100%.csv", "file:///a/b/100%25.csv"),
            ("/", "file:///"),
        ] {
            assert_eq!(
                lake.join_path(joined).unwrap().to_string(),
                expected,
                "{joined:?}"
            );
        }

        // A relative component still extends, and `.` and `..` still resolve.
        assert_eq!(
            lake.join_path("../lake2/a b.csv").unwrap().to_string(),
            "file:///lake/lake2/a%20b.csv"
        );
        assert_eq!(
            lake.join_path("./x").unwrap().to_string(),
            "file:///lake/trades/x"
        );
    }

    /// A file name is a name, so no mutation may turn one into a dot segment.
    ///
    /// `..a` has the extension `a` and the stem `.`, so dropping the extension used
    /// to leave the path addressing the directory that holds the file instead of
    /// the file. The suffix removers keep the name; the setters refuse the value.
    #[test]
    fn a_filename_mutation_never_produces_a_dot_segment() {
        // Removing the one extension these names have would leave `.` or `..`.
        for name in ["..a", "...a"] {
            let source = Url::from_str(&format!("file:///lake/{name}")).unwrap();

            let mut removed = source.clone();
            assert!(!removed.remove_extension(), "{name:?}");
            assert_eq!(removed, source, "{name:?}");

            let mut cleared = source.clone();
            assert!(!cleared.clear_extensions(), "{name:?}");
            assert_eq!(cleared, source, "{name:?}");

            assert_eq!(source.parts(), ["lake", name], "{name:?}");
        }

        // A compound name keeps a usable stem, so one suffix comes off; clearing
        // every suffix would leave `.` again, so that one does not.
        let mut compound = Url::from_str("file:///lake/..a.b.c").unwrap();
        assert!(compound.remove_extension());
        assert_eq!(compound.to_string(), "file:///lake/..a.b");
        assert!(!compound.clear_extensions());
        assert_eq!(compound.to_string(), "file:///lake/..a.b");

        // The setters refuse a dot segment outright, and leave the path alone.
        let mut url = Url::from_str("file:///lake/report.csv").unwrap();
        for value in [".", ".."] {
            assert!(url.set_file_name(value).is_err(), "{value:?}");
            assert!(url.set_stem(value).is_err(), "{value:?}");
            assert_eq!(url.to_string(), "file:///lake/report.csv", "{value:?}");
        }

        // An ordinary dotfile is untouched by the rule.
        let mut hidden = Url::from_str("file:///lake/.env.local").unwrap();
        assert!(hidden.remove_extension());
        assert_eq!(hidden.to_string(), "file:///lake/.env");
    }
}

mod location {

    use yggdryl::{Arn, Uri, Url, Urn};

    /// The location door reads every spelling of *where*: a URL, a bare path,
    /// and a name - while the strict door keeps refusing a name.
    #[test]
    fn the_location_door_resolves_a_name_and_the_strict_door_refuses_one() {
        assert_eq!(
            Url::from_location("arn:aws:s3:::trades/2026/part.parquet").unwrap(),
            Url::from_str("s3://trades/2026/part.parquet").unwrap()
        );
        assert_eq!(
            Url::from_location("urn:lake:trades:part.parquet").unwrap(),
            Urn::from_str("urn:lake:trades:part.parquet")
                .unwrap()
                .locator()
                .unwrap()
        );

        // A location still reads as itself, and a bare path still roots.
        assert_eq!(
            Url::from_location("s3://trades/part.parquet")
                .unwrap()
                .to_string(),
            "s3://trades/part.parquet"
        );
        let rooted = Url::from_location("data/part.parquet").unwrap();
        assert!(rooted.is_local());
        assert!(rooted.to_string().ends_with("/data/part.parquet"));

        // The strict door is the other reading, and names its refusal.
        for (name, reason) in [
            ("urn:lake:trades:part.parquet", "URN values are not URLs"),
            ("arn:aws:s3:::trades", "ARN values are not URLs"),
        ] {
            let error = Url::from_uri(Uri::from_str(name).unwrap())
                .map(|url| url.to_string())
                .expect_err(name);
            assert!(error.to_string().contains(reason), "{name}: {error}");
        }

        // A name that locates nothing is refused by the service, not rooted.
        assert!(Url::from_location("arn:aws:iam::123456789012:user/David").is_err());
    }
}

/// The canonical rendering a located reader shares across its rows.
///
/// `shared_text` is crate-internal - a text reader projects one `Arc<Url>`
/// into every row through it - so the rule that a mutation clears the cache
/// is pinned through `yggdryl::internals`.
#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Result;
    use yggdryl::Url;
    use yggdryl::internals::uri_url::shared_text;

    #[test]
    fn shared_text_follows_mutations() -> Result<()> {
        let mut url = Url::from_str("https://example.com/trades.csv?day=1")?;
        assert_eq!(shared_text(&url).as_str(), url.to_string());

        url.set_query(Some("day=2"))?;
        assert_eq!(shared_text(&url).as_str(), url.to_string());

        assert!(url.remove_extension());
        assert_eq!(shared_text(&url).as_str(), url.to_string());
        Ok(())
    }
}
