//! `rust/src/protocol.rs`: the protocol metadata keys no caller can name.
//!
//! `PYTHON_MODULE_KEY`, `PYTHON_QUALNAME_KEY` and `PYTHON_KIND_KEY` are
//! crate-private: they are the keys a declaration is stored under, and a
//! caller reads them back only through the borrowed view. What a caller can
//! observe lives beside them here.
//!
//! The protocol list an integration test cannot reach.
//!
//! `for_each_well_known_protocol!` is a crate-private macro: it is the one
//! place the well-known protocols are named, and this pin checks each one
//! reaches the key its own scheme spells. The rest of the suite lives beside
//! it here.

#[cfg(feature = "internals")]
mod internal {
    mod python_tests {

        use yggdryl::DataType;
        use yggdryl::internals::protocol::{
            KIND_SHAPE, PYTHON_KIND_KEY, PYTHON_MODULE_KEY, PYTHON_QUALNAME_KEY,
            validate_python_module, validate_python_qualname,
        };
        use yggdryl::{PythonKind, PythonMetadata};

        #[test]
        fn a_declaration_round_trips_through_the_field() {
            let declared =
                PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Field).unwrap();
            let mut field = DataType::Int64.required_field("price");

            field.as_python_mut().set_class(&declared).unwrap();

            assert_eq!(field.get_metadata(PYTHON_KIND_KEY), Some("field"));
            assert_eq!(field.get_metadata(PYTHON_MODULE_KEY), Some("trading.book"));
            assert_eq!(field.get_metadata(PYTHON_QUALNAME_KEY), Some("Book.Quote"));
            assert_eq!(field.as_python().class().unwrap(), Some(declared.clone()));
            assert_eq!(field.as_python().class_name(), Some("Quote"));
            assert_eq!(
                field.as_python().import_path().as_deref(),
                Some("trading.book.Book.Quote")
            );
            assert_eq!(field.as_python_mut().remove_class(), Some(declared));
            assert!(field.as_python().is_empty());
        }

        #[test]
        fn a_partial_declaration_names_no_class() {
            let mut field = DataType::Int64.required_field("price");
            field.as_python_mut().set_module("trading.book").unwrap();

            assert_eq!(field.as_python().module(), Some("trading.book"));
            assert_eq!(field.as_python().class().unwrap(), None);
            assert_eq!(field.as_python().import_path(), None);
        }

        #[test]
        fn a_class_declared_in_a_function_is_not_importable() {
            let declared =
                PythonMetadata::new("app", "build.<locals>.Row", PythonKind::Dataclass).unwrap();

            assert_eq!(declared.class_name(), "Row");
            assert!(!declared.is_importable());
            assert!(
                PythonMetadata::new("app", "Row", PythonKind::Dataclass)
                    .unwrap()
                    .is_importable()
            );
        }

        #[test]
        fn every_stored_form_round_trips_its_spelling() {
            for kind in PythonKind::ALL {
                assert_eq!(PythonKind::from_str(kind.as_str()).unwrap(), kind);
                // The refusal sentence is written out, so it is the one thing that
                // can fall behind a form added to the list.
                assert!(KIND_SHAPE.contains(kind.as_str()), "{kind}");
            }
            assert!(PythonKind::from_str("record").is_err());
        }

        #[test]
        fn a_name_python_could_not_have_written_is_refused() {
            for refused in ["", "trading.", ".book", "trading book", "1book", "class"] {
                assert!(validate_python_module(refused).is_err(), "{refused:?}");
            }
            for accepted in ["trading", "trading.book", "_private", "match", "données"] {
                assert!(validate_python_module(accepted).is_ok(), "{accepted:?}");
            }
            assert!(validate_python_qualname("build.<locals>.Row").is_ok());
            assert!(validate_python_module("build.<locals>.Row").is_err());
        }
    }

    mod tests {
        use yggdryl::internals::protocol::well_known_protocol_probes;

        /// Every named pair reaches the key its own scheme spells.
        ///
        /// The list is crate-private and a macro, which cannot cross the crate
        /// boundary, so the walk over it is behind one forwarder: it stores `x`
        /// as `1` through each mutable view and answers what each pair of views
        /// then spells. The literal key is what proves the accessor, the newtype
        /// and the scheme constant of one list entry agree; both `prefix` values
        /// would still agree if all three drifted together.
        #[test]
        fn every_named_protocol_view_spells_its_own_scheme_prefix() {
            let probes = well_known_protocol_probes();
            assert!(!probes.is_empty(), "the protocol list is empty");
            for probe in probes {
                let key = format!("{}:x", probe.scheme_prefix);

                assert_eq!(probe.stored.as_deref(), Some("1"), "{key}");
                assert_eq!(probe.view_prefix, probe.scheme_prefix);
                assert_eq!(probe.view_mut_prefix, probe.scheme_prefix);
                assert_eq!(probe.key, key);
                assert_eq!(
                    probe.entries,
                    [(String::from("x"), String::from("1"))],
                    "{key}"
                );
            }
        }
    }
}

mod enums {

    use yggdryl::{Error, PythonKind};

    #[test]
    fn every_python_kind_has_one_canonical_spelling() {
        for (kind, canonical) in [
            (PythonKind::Field, "field"),
            (PythonKind::Dataclass, "dataclass"),
            (PythonKind::TypedDict, "typed_dict"),
            (PythonKind::NamedTuple, "named_tuple"),
            (PythonKind::Enum, "enum"),
            (PythonKind::NewType, "newtype"),
            (PythonKind::TypeAlias, "type_alias"),
            (PythonKind::Class, "class"),
        ] {
            assert_eq!(kind.as_str(), canonical);
            assert_eq!(kind.as_ref(), canonical);
            assert_eq!(kind.to_string(), canonical);
            assert_eq!(PythonKind::from_str(canonical).unwrap(), kind);
            // Unlike an IO mode, a stored form is exact: it is written by this
            // crate under one key, never typed by a person into a URL or a config.
            assert!(PythonKind::from_str(&canonical.to_uppercase()).is_err());
        }
        assert_eq!(PythonKind::ALL.len(), 8);
        assert_eq!(
            PythonKind::ALL
                .iter()
                .filter(|kind| kind.is_keyword_constructed())
                .count(),
            3
        );

        let error = PythonKind::from_str("record").unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "PYTHON:kind"),
            "{error}"
        );
        let rendered = error.to_string();
        for kind in PythonKind::ALL {
            assert!(rendered.contains(kind.as_str()), "{rendered}");
        }
        assert!(rendered.contains("\"record\""), "{rendered}");
    }
}

mod generic {

    use std::sync::Arc;

    use yggdryl::{
        DataType, DigestAlgorithm, Error, Field, MediaType, Metadata, MimeType, PythonKind,
        PythonMetadata, Scheme, StructType,
    };

    #[test]
    fn field_parser_validates_typed_metadata_and_round_trips_protocol_values() {
        for invalid in [
            r#"field("id",int64,nullable=false,metadata={"alias":""})"#,
            r#"field("id",int64,nullable=false,metadata={"POSTGRES:":"value"})"#,
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
            r#"field("id",int64,nullable=false,metadata={"POSTGRES:comment":"line one\nline two","POSTGRES:empty":""})"#,
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
    fn one_straight_comment_is_what_every_protocol_reads() {
        let mut field = DataType::Int64.required_field("price");
        assert_eq!(field.as_iceberg().comment(), None);

        field.set_comment("the closing price").unwrap();
        // Written once without a namespace, read by every protocol - the same
        // fallback the other reserved text key answers with.
        assert_eq!(field.as_iceberg().comment(), Some("the closing price"));
        assert_eq!(field.as_glue().comment(), Some("the closing price"));
        assert_eq!(field.as_glue_mut().comment(), Some("the closing price"));

        // A protocol naming its own wins for itself alone, and the straight key is
        // still not one of that protocol's properties.
        field.as_glue_mut().insert("comment", "close").unwrap();
        assert_eq!(field.as_glue().comment(), Some("close"));
        assert_eq!(field.as_iceberg().comment(), Some("the closing price"));
        assert!(field.as_iceberg().is_empty());
        assert_eq!(field.comment(), Some("the closing price"));

        assert_eq!(field.remove_comment().as_deref(), Some("the closing price"));
        assert_eq!(field.as_iceberg().comment(), None);
        assert_eq!(field.as_glue().comment(), Some("close"));

        let metadata = Metadata::from_entries([("comment", "the ticker")]).unwrap();
        assert_eq!(metadata.comment(), Some("the ticker"));
        assert_eq!(metadata.as_iceberg().comment(), Some("the ticker"));
    }

    #[test]
    fn one_straight_display_name_is_what_every_protocol_reads() {
        let mut field = DataType::Int64.required_field("price");
        assert_eq!(field.display(), None);
        assert_eq!(field.as_iceberg().display(), None);

        field.set_display("Closing price").unwrap();
        assert_eq!(field.get_metadata("display"), Some("Closing price"));
        // Written once without a namespace, read by every protocol.
        assert_eq!(field.as_iceberg().display(), Some("Closing price"));
        assert_eq!(field.as_glue().display(), Some("Closing price"));
        assert_eq!(field.as_glue_mut().display(), Some("Closing price"));

        // A protocol naming its own wins for itself alone, and the straight key
        // is still not one of that protocol's properties.
        field.as_glue_mut().insert("display", "Close").unwrap();
        assert_eq!(field.as_glue().display(), Some("Close"));
        assert_eq!(
            field.as_glue().iter().collect::<Vec<_>>(),
            [("display", "Close")]
        );
        assert_eq!(field.as_iceberg().display(), Some("Closing price"));
        assert!(field.as_iceberg().is_empty());
        assert_eq!(field.display(), Some("Closing price"));

        assert_eq!(field.remove_display().as_deref(), Some("Closing price"));
        assert_eq!(field.as_iceberg().display(), None);
        assert_eq!(field.as_glue().display(), Some("Close"));

        let metadata = Metadata::from_entries([("display", "Ticker symbol")]).unwrap();
        assert_eq!(metadata.display(), Some("Ticker symbol"));
        assert_eq!(metadata.as_iceberg().display(), Some("Ticker symbol"));

        let named = DataType::utf8()
            .required_field("symbol")
            .try_with_display("Ticker symbol")
            .unwrap();
        assert_eq!(named.display(), Some("Ticker symbol"));
        assert!(
            DataType::utf8()
                .required_field("symbol")
                .try_with_display("bad\nname")
                .is_err()
        );
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
        assert_eq!(field.as_iceberg().key("doc"), "ICEBERG:doc");
        assert_eq!(field.as_iceberg().prefix(), "ICEBERG");
        assert_eq!(field.as_iceberg().scheme(), &Scheme::ICEBERG);
        assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
        assert_eq!(field.get_metadata("ICEBERG:doc"), Some("closing price"));
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
    fn identity_and_partition_views_hold_independent_generic_metadata() {
        let mut field = DataType::utf8().required_field("venue");

        field
            .as_identity_mut()
            .update([("role", "primary"), ("source", "exchange")])
            .unwrap();
        field
            .as_partition_mut()
            .update([("transform", "year"), ("null", "last")])
            .unwrap();

        assert_eq!(field.as_identity().scheme(), &Scheme::IDENTITY);
        assert_eq!(field.as_partition().scheme(), &Scheme::PARTITION);
        let _: yggdryl::IdentityField<'_> = field.as_identity();
        let _: yggdryl::PartitionField<'_> = field.as_partition();
        assert_eq!(field.as_identity().get("role"), Some("primary"));
        assert_eq!(
            field.as_partition().iter().collect::<Vec<_>>(),
            [("null", "last"), ("transform", "year")]
        );
        assert_eq!(field.get_metadata("IDENTITY:source"), Some("exchange"));
        assert_eq!(field.get_metadata("PARTITION:transform"), Some("year"));
        // `IDENTITY:` stays inert text; the two typed `PARTITION:` properties do
        // not, and a dialect alias resolves to the one name the grammar owns.
        field
            .as_partition_mut()
            .insert("transform", "dayofmonth")
            .unwrap();
        assert_eq!(field.get_metadata("PARTITION:transform"), Some("day"));
        assert!(
            field
                .as_partition_mut()
                .insert("transform", "epoch")
                .is_err()
        );
        assert!(
            field
                .as_partition_mut()
                .insert("sources", r#"["a","a"]"#)
                .is_err()
        );

        // Protocol annotations do not replace the field-owned path-layout mark.
        assert!(!field.is_partition());
        assert!(!field.has_metadata("FIELD:partition"));

        field.as_partition_mut().clear();
        assert!(field.as_partition().is_empty());
        assert_eq!(field.as_identity().len(), 2);

        let restored: Field =
            serde_json::from_str(&serde_json::to_string(&field).unwrap()).unwrap();
        assert_eq!(restored.as_identity().get("role"), Some("primary"));
        assert!(restored.as_partition().is_empty());

        let metadata = Metadata::from_entries([
            ("IDENTITY:codec", "uuid"),
            ("PARTITION:sources", r#" [ "venue" ] "#),
        ])
        .unwrap();
        assert_eq!(metadata.as_identity().get("codec"), Some("uuid"));
        assert_eq!(metadata.as_partition().get("sources"), Some(r#"["venue"]"#));
    }

    #[test]
    fn digest_view_owns_one_validated_exclusive_role_and_generic_metadata() {
        let mut field = DataType::UInt64.required_field("row_digest");
        let _: yggdryl::DigestField<'_> = field.as_digest();
        assert_eq!(field.as_digest().scheme(), &Scheme::DIGEST);

        field
            .as_digest_mut()
            .insert("note", "materialized")
            .unwrap();
        field.as_digest_mut().set_holder().unwrap();
        assert!(field.as_digest().is_holder());
        assert_eq!(field.get_metadata("DIGEST:role"), Some("holder"));
        assert_eq!(field.as_digest().get("note"), Some("materialized"));

        // `holder` is the only role: a digest states what a field holds, never
        // what another field contributes, which is named on the holder instead.
        assert!(Metadata::from_entries([("DIGEST:role", "component")]).is_err());

        let snapshot = field.clone();
        let error = field.as_digest_mut().insert("role", "input").unwrap_err();
        assert!(error.to_string().contains("DIGEST:role"), "{error}");
        assert_eq!(
            field, snapshot,
            "a rejected role leaves the field unchanged"
        );
        assert!(Metadata::from_entries([("DIGEST:role", "output")]).is_err());

        let arrow = field.clone().into_arrow_field().unwrap();
        let restored = Field::from_arrow_field(&arrow).unwrap();
        assert!(restored.as_digest().is_holder());
        assert_eq!(restored.as_digest().get("note"), Some("materialized"));
        let metadata =
            Metadata::from_entries([("DIGEST:role", "holder"), ("DIGEST:note", "materialized")])
                .unwrap();
        assert_eq!(metadata.as_digest().get("role"), Some("holder"));
        assert_eq!(metadata.as_digest().get("note"), Some("materialized"));

        assert_eq!(
            field.as_digest_mut().remove_role().unwrap().as_deref(),
            Some("holder")
        );
        assert!(!field.as_digest().is_holder());
        assert_eq!(field.as_digest().get("note"), Some("materialized"));
    }

    #[test]
    fn digest_holder_sources_are_canonical_ordered_and_role_owned() {
        let metadata = Metadata::from_entries([(
            "DIGEST:sources",
            r#" [ "id", "line.price", "name,\"quoted\"", "\u6771\u4eac" ] "#,
        )])
        .unwrap();
        assert_eq!(
            metadata.get("DIGEST:sources"),
            Some(r#"["id","line.price","name,\"quoted\"","東京"]"#)
        );

        let mut holder = DataType::UInt64.required_field("row_digest");
        let unchanged = holder.clone();
        let error = holder
            .as_digest_mut()
            .set_sources(["id", "line.price"])
            .unwrap_err();
        assert!(error.to_string().contains("DIGEST:sources"), "{error}");
        assert_eq!(holder, unchanged, "a non-holder source write is atomic");

        holder.as_digest_mut().set_holder().unwrap();
        holder
            .as_digest_mut()
            .set_sources(["id", "line.price", "name,\"quoted\"", "東京"])
            .unwrap();
        assert_eq!(
            holder.as_digest().sources().unwrap(),
            Some(vec![
                "id".to_owned(),
                "line.price".to_owned(),
                "name,\"quoted\"".to_owned(),
                "東京".to_owned(),
            ])
        );

        let restored =
            Field::from_arrow_field(&holder.clone().into_arrow_field().unwrap()).unwrap();
        assert_eq!(restored, holder);
        assert_eq!(
            restored.as_digest().sources().unwrap(),
            holder.as_digest().sources().unwrap()
        );

        let unchanged = holder.clone();
        for paths in [vec!["id", ""], vec!["id", "id"]] {
            let error = holder.as_digest_mut().set_sources(paths).unwrap_err();
            assert!(error.to_string().contains("DIGEST:sources"), "{error}");
            assert_eq!(holder, unchanged, "a rejected source list is atomic");
        }

        let error = holder.as_digest_mut().remove_role().unwrap_err();
        assert!(error.to_string().contains("DIGEST:role"), "{error}");
        assert_eq!(holder, unchanged);

        assert_eq!(
            holder.as_digest_mut().remove_sources().as_deref(),
            Some(r#"["id","line.price","name,\"quoted\"","東京"]"#)
        );
        assert_eq!(holder.as_digest().sources().unwrap(), None);
        assert_eq!(
            holder.as_digest_mut().remove_role().unwrap().as_deref(),
            Some("holder")
        );
    }

    #[test]
    fn digest_sources_preserve_explicit_empty_and_reject_every_invalid_shape() {
        let mut holder = DataType::UInt64.required_field("row_digest");
        holder.as_digest_mut().set_holder().unwrap();
        holder
            .as_digest_mut()
            .set_sources(Vec::<&str>::new())
            .unwrap();
        assert_eq!(holder.get_metadata("DIGEST:sources"), Some("[]"));
        assert_eq!(holder.as_digest().sources().unwrap(), Some(Vec::new()));

        for value in [
            "null",
            "{}",
            r#""id""#,
            "[1]",
            r#"[""]"#,
            r#"["id","id"]"#,
            "[",
        ] {
            let error = Metadata::from_entries([("DIGEST:sources", value)]).unwrap_err();
            assert!(error.to_string().contains("DIGEST:sources"), "{error}");
        }

        let unchanged = holder.clone();
        let error = holder
            .as_digest_mut()
            .insert("sources", r#"["id","id"]"#)
            .unwrap_err();
        assert!(error.to_string().contains("DIGEST:sources"), "{error}");
        assert_eq!(
            holder, unchanged,
            "generic mutation uses the same validator"
        );
    }

    #[test]
    fn digest_holder_algorithm_is_canonical_typed_and_role_owned() {
        for (input, canonical, algorithm) in [
            ("xxh32", "xxh32", DigestAlgorithm::Xxh32),
            ("XXH64", "xxh64", DigestAlgorithm::Xxh64),
            (" xxh3 ", "xxh3-64", DigestAlgorithm::Xxh3),
            ("xxh128", "xxh3-128", DigestAlgorithm::Xxh128),
        ] {
            let metadata = Metadata::from_entries([("DIGEST:algorithm", input)]).unwrap();
            assert_eq!(metadata.get("DIGEST:algorithm"), Some(canonical));

            let field =
                Field::from_parts("digest", DataType::UInt64, false, metadata.iter()).unwrap();
            assert_eq!(field.as_digest().algorithm().unwrap(), Some(algorithm));
        }
        for invalid in ["", "xxh3-256", "sha256"] {
            let error = Metadata::from_entries([("DIGEST:algorithm", invalid)]).unwrap_err();
            assert!(error.to_string().contains("DIGEST:algorithm"), "{error}");
        }

        let mut holder = DataType::UInt64.required_field("row_digest");
        let unchanged = holder.clone();
        let error = holder
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh3)
            .unwrap_err();
        assert!(error.to_string().contains("DIGEST:algorithm"), "{error}");
        assert_eq!(holder, unchanged, "a non-holder algorithm write is atomic");

        holder.as_digest_mut().set_holder().unwrap();
        let unchanged = holder.clone();
        let error = holder
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh128)
            .unwrap_err();
        assert!(
            error.to_string().contains("fixed_size_binary[16]"),
            "{error}"
        );
        assert_eq!(holder, unchanged, "a mismatched algorithm write is atomic");

        holder
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh3)
            .unwrap();
        assert_eq!(holder.get_metadata("DIGEST:algorithm"), Some("xxh3-64"));
        assert_eq!(
            holder.as_digest().algorithm().unwrap(),
            Some(DigestAlgorithm::Xxh3)
        );

        let unchanged = holder.clone();
        let error = holder.as_digest_mut().remove_role().unwrap_err();
        assert!(error.to_string().contains("DIGEST:role"), "{error}");
        assert_eq!(holder, unchanged);

        assert_eq!(
            holder.as_digest_mut().remove_algorithm().as_deref(),
            Some("xxh3-64")
        );
        assert_eq!(holder.as_digest().algorithm().unwrap(), None);
        assert_eq!(
            holder.as_digest_mut().remove_role().unwrap().as_deref(),
            Some("holder")
        );

        let mut wide = DataType::fixed_binary(16)
            .unwrap()
            .required_field("wide_digest");
        wide.as_digest_mut().set_holder().unwrap();
        wide.as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh128)
            .unwrap();
        assert_eq!(
            wide.as_digest().algorithm().unwrap(),
            Some(DigestAlgorithm::Xxh128)
        );

        for (dtype, algorithm) in [
            (DataType::Int32, DigestAlgorithm::Xxh32),
            (DataType::Int64, DigestAlgorithm::Xxh64),
            (DataType::Int64, DigestAlgorithm::Xxh3),
        ] {
            let mut signed = dtype.required_field("signed_digest");
            signed.as_digest_mut().set_holder().unwrap();
            signed.as_digest_mut().set_algorithm(algorithm).unwrap();
            assert_eq!(signed.as_digest().algorithm().unwrap(), Some(algorithm));
        }
    }

    #[test]
    fn digest_field_selection_defaults_to_every_non_holder_then_honors_components() {
        let symbol = DataType::utf8().required_field("symbol");
        let quantity = DataType::Int64.required_field("quantity");
        let mut holder = DataType::UInt64.required_field("row_digest");
        holder.as_digest_mut().set_holder().unwrap();

        let mut fallback =
            StructType::from_fields([symbol.clone(), holder.clone(), quantity.clone()])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
        fallback.set_comment("trade row").unwrap();
        assert_eq!(
            fallback.digest_field_names().collect::<Vec<_>>(),
            ["symbol", "quantity"]
        );
        assert_eq!(
            fallback.digest_field_names().rev().collect::<Vec<_>>(),
            ["quantity", "symbol"]
        );
        assert_eq!(fallback.digest_field_len(), 2);
        let _: yggdryl::DigestFields<'_> = fallback.digest_fields();
        let _: yggdryl::DigestFieldNames<'_> = fallback.digest_field_names();
        let selected = fallback.only_digest_fields().unwrap();
        assert_eq!(selected.field_len(), 2);
        assert_eq!(selected.comment(), Some("trade row"));

        // Narrowing the input is the holder's business: the fields it reads stay
        // ordinary columns, and the default selection is still every non-holder.
        let mut narrowed = holder.clone();
        narrowed.as_digest_mut().set_sources(["quantity"]).unwrap();
        let explicit = StructType::from_fields([symbol, narrowed.clone(), quantity])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assert_eq!(
            narrowed.as_digest().sources().unwrap(),
            Some(vec!["quantity".to_owned()])
        );
        assert_eq!(
            explicit.digest_field_names().collect::<Vec<_>>(),
            ["symbol", "quantity"]
        );
        assert_eq!(explicit.only_digest_fields().unwrap().field_len(), 2);

        let mut other_holder = DataType::UInt32.required_field("narrow_digest");
        other_holder.as_digest_mut().set_holder().unwrap();
        let holders = StructType::from_fields([holder, other_holder])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assert_eq!(holders.digest_field_len(), 0);
        assert_eq!(holders.only_digest_fields().unwrap().field_len(), 0);

        let scalar = DataType::Int64.required_field("value");
        assert_eq!(scalar.digest_field_len(), 0);
        assert!(scalar.only_digest_fields().is_err());
    }

    #[test]
    fn a_protocol_view_shares_http_between_the_two_schemes_and_stays_case_insensitive() {
        let mut field = DataType::utf8().required_field("body");
        field
            .as_http_mut()
            .insert("Content-Type", "text/plain")
            .unwrap();

        assert_eq!(field.as_http().get("content-type"), Some("text/plain"));
        assert_eq!(field.as_http().get("CONTENT-TYPE"), Some("text/plain"));
        assert_eq!(field.as_http().key("Content-Type"), "HTTP:Content-Type");
        assert_eq!(
            field.protocol(&Scheme::HTTPS).get("Content-Type"),
            Some("text/plain")
        );
        assert_eq!(field.protocol(&Scheme::HTTPS).prefix(), "HTTP");
        assert_eq!(field.as_http().content_type(), Some("text/plain"));
        assert_eq!(field.get_metadata("HTTP:content-type"), Some("text/plain"));

        // The view is a borrow of the field's own snapshot, not a copy of it.
        let metadata = field.as_metadata().clone();
        assert_eq!(metadata.as_http(), field.as_http().as_properties());
        assert_eq!(
            metadata
                .as_http()
                .into_metadata()
                .unwrap()
                .get("HTTP:content-type"),
            Some("text/plain")
        );
    }

    #[test]
    fn a_protocol_write_invalidates_the_arrow_cache_exactly_once() {
        let field = DataType::Int64.required_field("price");
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.as_iceberg_mut().insert("doc", "close").unwrap();
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.as_iceberg_mut().insert("doc", "close").unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        // A rejected value leaves the field, and its cache, untouched.
        assert!(field.as_iceberg_mut().insert("", "no name").is_err());
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        assert_eq!(field.as_iceberg().len(), 1);
    }

    #[cfg(feature = "iceberg")]
    #[test]
    fn a_typed_read_outlives_the_view_it_was_read_through() {
        let mut field = DataType::Int64.required_field("price");
        field.as_iceberg_mut().set_doc("closing price").unwrap();
        field.set_display("Closing price").unwrap();

        // Compiling is the assertion. Every one of these reads through a view that
        // dies at its own semicolon, and each answer carries the field's lifetime
        // rather than the view's. Deref does not: `field.as_iceberg().name()` is
        // E0716, which is what `as_field` exists to spell instead.
        let name = field.as_iceberg().as_field().name();
        let doc = field.as_iceberg().doc();
        let property = field.as_iceberg().get("doc");
        let display = field.as_iceberg().display();

        assert_eq!(name, "price");
        assert_eq!(doc, Some("closing price"));
        assert_eq!(property, doc);
        assert_eq!(display, Some("Closing price"));
    }

    #[test]
    fn indexing_a_view_reads_a_property_where_indexing_a_field_reads_a_child() {
        let mut row = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        row.as_iceberg_mut().insert("doc", "one row").unwrap();
        row.as_glue_mut().insert("id", "not a child").unwrap();

        // Two operators that look alike. The view's `Output` is `str` and the
        // field's is `Field`, and it is the concrete impl on the view that stops
        // the child meaning from winning through the deref - `id` is both a child
        // name and a property name here, so nothing but that impl separates them.
        assert_eq!(&row.as_iceberg()["doc"], "one row");
        assert_eq!(&row.as_glue()["id"], "not a child");
        assert_eq!(row["id"].dtype(), &DataType::Int64);
        assert_eq!(row["venue"].name(), "venue");

        // A positional index is the field's, and the view does not forward to it:
        // the view's only `Index` impl takes `&str`, so `row.as_iceberg()[1]` is a
        // type error rather than a child lookup. The field is spelled out instead.
        assert_eq!(row[0].name(), "id");
        assert_eq!(row.as_iceberg().as_field()[1].name(), "venue");
    }

    #[test]
    fn a_typed_protocol_write_invalidates_a_populated_projection_exactly_once() {
        let media = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap();
        let field = Field::new("payload", DataType::binary(), false);
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();

        // The two-key media write is effective, so the projection it invalidated
        // is gone and the next ask rebuilds one.
        field.as_http_mut().set_media_type(media.clone()).unwrap();
        let rebuilt = field.clone().into_arrow_field_ref().unwrap();
        assert!(!Arc::ptr_eq(&cached, &rebuilt));

        // Once, though: the rebuilt projection survives writing the same value
        // again, so the invalidation belongs to the change and not to the call.
        let mut field = Field::from_arrow_field_ref(Arc::clone(&rebuilt)).unwrap();
        field.as_http_mut().set_media_type(media.clone()).unwrap();
        field.as_http_mut().set_media_type(media).unwrap();
        assert!(Arc::ptr_eq(
            &rebuilt,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        #[cfg(feature = "iceberg")]
        {
            let field = DataType::Int64.required_field("price");
            let cached = Arc::new(field.clone().into_arrow_field().unwrap());
            let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
            field.as_iceberg_mut().set_doc("closing price").unwrap();
            let rebuilt = field.clone().into_arrow_field_ref().unwrap();
            assert!(!Arc::ptr_eq(&cached, &rebuilt));

            let mut field = Field::from_arrow_field_ref(Arc::clone(&rebuilt)).unwrap();
            field.as_iceberg_mut().set_doc("closing price").unwrap();
            assert!(Arc::ptr_eq(
                &rebuilt,
                &field.clone().into_arrow_field_ref().unwrap()
            ));
            assert_eq!(field.as_iceberg().doc(), Some("closing price"));
        }
    }

    #[test]
    fn a_field_can_act_as_a_partition_column_and_a_root_reports_only_those() {
        let schema = StructType::from_fields([
            DataType::Int32.required_field("year"),
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("price"),
        ])
        .map(DataType::from)
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
                .get_metadata("FIELD:partition"),
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
        let root = StructType::from_fields([DataType::Int64.required_field("price")])
            .map(DataType::from)
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
            Field::from_parts("year", DataType::Int32, false, [("FIELD:partition", "yes")])
                .is_err()
        );
        assert!(
            !Field::from_parts(
                "year",
                DataType::Int32,
                false,
                [("FIELD:partition", "false")]
            )
            .unwrap()
            .is_partition()
        );
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
        assert_eq!(merged.get("ICEBERG:doc"), Some("held"));
        assert_eq!(merged.get("ICEBERG:id"), Some("7"));

        // Both views contribute bare names, and the result is keyed under the
        // receiver's protocol, so merging across two namespaces still answers one.
        let mut glue = DataType::Int64.required_field("price");
        glue.protocol_mut(&Scheme::GLUE)
            .insert("comment", "from glue")
            .unwrap();

        let crossed = held.as_iceberg().merge_with(&glue.as_glue()).unwrap();
        assert_eq!(crossed.get("ICEBERG:comment"), Some("from glue"));
        assert!(crossed.get("GLUE:comment").is_none());
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

    #[test]
    fn the_star_source_may_not_travel_beside_a_named_one() {
        let mut holder = DataType::UInt64.required_field("row_digest");
        holder.as_digest_mut().set_holder().unwrap();
        holder.as_digest_mut().set_sources(["*"]).unwrap();
        assert_eq!(holder.get_metadata("DIGEST:sources"), Some(r#"["*"]"#));
        assert_eq!(
            holder.as_digest().sources().unwrap(),
            Some(vec!["*".to_owned()])
        );

        let unchanged = holder.clone();
        let error = holder.as_digest_mut().set_sources(["*", "id"]).unwrap_err();
        assert!(error.to_string().contains("DIGEST:sources"), "{error}");
        assert_eq!(holder, unchanged, "a rejected source list is atomic");
        // The generic mutation path runs the same validator.
        assert!(Metadata::from_entries([("DIGEST:sources", r#"["id","*"]"#)]).is_err());
    }

    #[test]
    fn a_python_declaration_is_written_and_read_as_one_value() {
        let declared =
            PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Dataclass).unwrap();
        let mut field = DataType::from_str("struct<symbol:string,price:int64>")
            .unwrap()
            .required_field("Quote");

        field.as_python_mut().set_class(&declared).unwrap();

        // One protocol namespace holds the whole declaration, and the bare class
        // name is derived from the qualified one rather than stored beside it.
        assert_eq!(
            field.as_python().iter().collect::<Vec<_>>(),
            [
                ("kind", "dataclass"),
                ("module", "trading.book"),
                ("qualname", "Book.Quote"),
            ]
        );
        assert_eq!(field.as_python().class_name(), Some("Quote"));
        assert_eq!(field.as_python().class().unwrap(), Some(declared.clone()));
        assert_eq!(
            field.as_python().import_path().as_deref(),
            Some("trading.book.Book.Quote")
        );

        // The declaration crosses Arrow with the column and comes back identical.
        let projected =
            Field::from_arrow_field(&field.clone().into_arrow_field().unwrap()).unwrap();
        assert_eq!(projected.as_python().class().unwrap(), Some(declared));
        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
    }

    #[test]
    fn a_python_declaration_is_validated_on_every_write_path() {
        for (key, value) in [
            ("PYTHON:module", "trading."),
            ("PYTHON:module", "1trading"),
            ("PYTHON:module", "trading book"),
            ("PYTHON:module", "class"),
            ("PYTHON:qualname", ""),
            ("PYTHON:qualname", "Book..Quote"),
            ("PYTHON:kind", "record"),
        ] {
            // The snapshot constructor, the field constructor and the protocol
            // write are one validator seen from three places.
            assert!(
                Metadata::from_entries([(key, value)]).is_err(),
                "{key}={value}"
            );
            assert!(
                Field::from_parts("quote", DataType::Int64, false, [(key, value)]).is_err(),
                "{key}={value}"
            );

            let mut field = DataType::Int64.required_field("quote");
            let unchanged = field.clone();
            let name = key.strip_prefix("PYTHON:").unwrap();
            assert!(field.as_python_mut().insert(name, value).is_err(), "{key}");
            assert_eq!(field, unchanged, "a rejected property is atomic");
        }

        // A class declared inside a function keeps the one non-identifier segment
        // Python itself writes, and reports that no import reaches it.
        let local = Field::from_parts(
            "row",
            DataType::Int64,
            false,
            [
                ("PYTHON:kind", "dataclass"),
                ("PYTHON:module", "app"),
                ("PYTHON:qualname", "build.<locals>.Row"),
            ],
        )
        .unwrap();
        let declared = local.as_python().class().unwrap().unwrap();
        assert_eq!(declared.class_name(), "Row");
        assert!(!declared.is_importable());
    }

    #[test]
    fn a_half_stated_python_declaration_names_no_class() {
        let mut field = DataType::Int64.required_field("price");
        field.as_python_mut().set_module("trading.book").unwrap();
        field.as_python_mut().set_kind(PythonKind::Field).unwrap();

        // Every part is readable on its own; only the whole is a class.
        assert_eq!(field.as_python().module(), Some("trading.book"));
        assert_eq!(field.as_python().kind().unwrap(), Some(PythonKind::Field));
        assert_eq!(field.as_python().class().unwrap(), None);
        assert_eq!(field.as_python().class_name(), None);

        field.as_python_mut().set_qualname("Quote").unwrap();
        assert!(field.as_python().class().unwrap().is_some());
        assert!(field.as_python_mut().remove_class().is_some());
        assert!(field.as_python().is_empty());
    }
}

mod nested {
    use yggdryl::{DataType, StructType};

    #[test]
    fn partition_names_are_exact_top_level_names() {
        let row = StructType::from_fields([
            DataType::Int64.required_field("a.b"),
            StructType::from_fields([DataType::Float64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .required_field("line"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let partitioned = row.with_partition_fields(&["a.b"]).unwrap();
        assert_eq!(
            partitioned.partition_field_names().collect::<Vec<_>>(),
            ["a.b"]
        );
        assert!(row.with_partition_fields(&["line.price"]).is_err());
    }
}
