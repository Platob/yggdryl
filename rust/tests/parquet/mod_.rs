//! `rust/src/parquet/mod.rs`: the logical types no caller can reach.
//!
//! `open_builder` is the file-private reader builder every Parquet read opens
//! through; the logical type a column publishes - what a reader outside this
//! crate sees - is only visible on it, and the schema descriptor carrying the
//! annotations is derived and read back by two more file-private steps.
//! Everything a caller can observe lives beside them here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Url;
    use yggdryl::holder::Buffer;

    /// A handle addressed as the Parquet file it holds.
    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_array::cast::AsArray;
    use arrow_array::{Array, ArrayRef, BinaryArray, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use parquet::basic::{EdgeInterpolationAlgorithm, LogicalType};

    use yggdryl::ArrowCastOptions;
    use yggdryl::FieldValue as _;
    use yggdryl::internals::parquet::open_builder;
    use yggdryl::internals::parquet_geospatial::{extension_schema, variant_schema};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, Field};
    use yggdryl::{IOBase, IOMedia};

    /// One little-endian ISO WKB point.
    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    /// A nullable field carrying one Arrow extension declaration.
    ///
    /// The metadata is spelled by hand so foreign and malformed documents can
    /// be written too; the well-formed spelling is proven equal to the
    /// `Field` projection's own output by the end-to-end test below.
    fn extension_field(
        name: &str,
        storage: ArrowDataType,
        extension: &str,
        document: Option<&str>,
    ) -> ArrowField {
        let mut metadata =
            HashMap::from([("ARROW:extension:name".to_owned(), extension.to_owned())]);
        if let Some(document) = document {
            metadata.insert("ARROW:extension:metadata".to_owned(), document.to_owned());
        }
        ArrowField::new(name, storage, true).with_metadata(metadata)
    }

    /// The canonical variant storage: a struct of two required binaries.
    fn variant_storage() -> ArrowDataType {
        ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::Binary, false),
            ArrowField::new("value", ArrowDataType::Binary, false),
        ]))
    }

    /// One variant column holding the given values, both children filled.
    fn variant_column(values: &[yggdryl::Scalar]) -> ArrayRef {
        let variants: Vec<yggdryl::Variant> = values
            .iter()
            .map(|value| yggdryl::Variant::encode(value).unwrap())
            .collect();
        Arc::new(arrow_array::StructArray::new(
            arrow_schema::Fields::from(vec![
                ArrowField::new("metadata", ArrowDataType::Binary, false),
                ArrowField::new("value", ArrowDataType::Binary, false),
            ]),
            vec![
                Arc::new(arrow_array::BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::metadata),
                )) as ArrayRef,
                Arc::new(arrow_array::BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::value),
                )) as ArrayRef,
            ],
            None,
        ))
    }

    /// Write one file holding the given fields and columns.
    fn written(name: &str, fields: Vec<ArrowField>, columns: Vec<ArrayRef>) -> Parquet<Buffer> {
        let schema = Arc::new(Schema::new(fields));
        let batches = if columns.is_empty() {
            Vec::new()
        } else {
            vec![RecordBatch::try_new(Arc::clone(&schema), columns).unwrap()]
        };
        let mut media = Parquet::new(handle(name));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, batches), &options)
            .unwrap();
        media
    }

    /// The logical type the footer stores for one leaf column path.
    fn leaf_logical(media: &Parquet<Buffer>, path: &str) -> Option<LogicalType> {
        let builder = open_builder(media.handle()).unwrap();
        builder
            .parquet_schema()
            .columns()
            .iter()
            .find(|column| column.path().string() == path)
            .and_then(|column| column.logical_type_ref().cloned())
    }

    #[test]
    fn a_code_column_writes_parquet_string_and_keeps_its_identity() {
        // A code stores as text, so Parquet gives it the String logical type
        // and bounds over the codes themselves. A padded column had no
        // logical type at all - a reader outside this crate saw an untyped
        // FIXED_LEN_BYTE_ARRAY where a currency is a string - and its bounds
        // carried the slot's NUL for every value short of the width.
        let media = written(
            "currency.parquet",
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                extension_field("ccy", ArrowDataType::Utf8, "yggdryl.currency", Some("")),
            ],
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("EUR"), None, Some("USD")])),
            ],
        );
        assert_eq!(leaf_logical(&media, "ccy"), Some(LogicalType::String));

        // Which means a planner gets bounds over the codes themselves.
        let statistics = media.read_statistics().unwrap();
        let ccy = statistics.row_groups[0]
            .columns
            .iter()
            .find(|column| column.path == "ccy")
            .unwrap();
        assert_eq!(ccy.min_bytes.as_deref(), Some(b"EUR".as_slice()));
        assert_eq!(ccy.max_bytes.as_deref(), Some(b"USD".as_slice()));
        assert_eq!(ccy.null_count, Some(1));

        // And the column reads back a currency rather than anonymous text.
        let options = media.record_options().unwrap();
        let batches: Vec<_> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap())
            .collect();
        let restored = Field::from_arrow_field(batches[0].schema().field(1)).unwrap();
        assert_eq!(restored.dtype(), &DataType::Currency);
        assert_eq!(restored.name(), "ccy");
        let cells = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(cells.value(0), "EUR");
        assert!(cells.is_null(1));
        assert_eq!(cells.value(2), "USD");
    }

    #[test]
    fn a_geometry_column_writes_the_logical_type_and_wkb_statistics() {
        let media = written(
            "geometry.parquet",
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                extension_field(
                    "shape",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84"}"#),
                ),
            ],
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(BinaryArray::from_opt_vec(vec![
                    Some(&wkb_point(1.0, 2.0)),
                    None,
                    Some(&wkb_point(-3.0, 7.0)),
                ])),
            ],
        );

        // The default CRS folds to Parquet's absent spelling.
        assert_eq!(
            leaf_logical(&media, "shape"),
            Some(LogicalType::geometry(None))
        );
        assert_eq!(leaf_logical(&media, "id"), None);

        let statistics = media.read_statistics().unwrap();
        let columns = &statistics.row_groups[0].columns;
        let id = columns.iter().find(|column| column.path == "id").unwrap();
        let shape = columns
            .iter()
            .find(|column| column.path == "shape")
            .unwrap();

        // The sibling still records value bounds; the geometry never does.
        assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
        assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
        assert_eq!(shape.null_count, Some(1));

        // What a geometry records instead: the WKB bounds and type codes.
        let geospatial = shape.geospatial.as_ref().unwrap();
        let bounds = geospatial.bounding_box.unwrap();
        assert_eq!(
            (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
            (-3.0, 1.0, 2.0, 7.0)
        );
        assert_eq!(geospatial.geometry_types, vec![1]);
        assert!(id.geospatial.is_none());
    }

    #[test]
    fn a_custom_crs_and_a_bare_declaration_both_survive() {
        let media = written(
            "crs.parquet",
            vec![
                extension_field(
                    "mercator",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "EPSG:3857"}"#),
                ),
                // No metadata document at all: a geometry in the default CRS.
                extension_field("bare", ArrowDataType::Binary, "geoarrow.wkb", None),
            ],
            vec![
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(0.0, 0.0)[..],
                )])),
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(0.0, 0.0)[..],
                )])),
            ],
        );

        assert_eq!(
            leaf_logical(&media, "mercator"),
            Some(LogicalType::geometry(Some("EPSG:3857".to_owned())))
        );
        assert_eq!(
            leaf_logical(&media, "bare"),
            Some(LogicalType::geometry(None))
        );
    }

    #[test]
    fn a_geography_column_carries_its_algorithm_and_writes_no_bounds() {
        let media = written(
            "geography.parquet",
            vec![
                extension_field(
                    "route",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "EPSG:4326", "edges": "vincenty"}"#),
                ),
                // The spherical default folds to Parquet's absent spelling.
                extension_field(
                    "region",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84", "edges": "spherical"}"#),
                ),
            ],
            vec![
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(4.0, 5.0)[..],
                )])),
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(6.0, 7.0)[..],
                )])),
            ],
        );

        assert_eq!(
            leaf_logical(&media, "route"),
            Some(LogicalType::geography(
                Some("EPSG:4326".to_owned()),
                Some(EdgeInterpolationAlgorithm::VINCENTY),
            ))
        );
        assert_eq!(
            leaf_logical(&media, "region"),
            Some(LogicalType::geography(None, None))
        );

        // A geography's bounds are edge-algorithm-aware, so a planar fold of
        // the vertices would under-cover them: no value bounds, and no box.
        let statistics = media.read_statistics().unwrap();
        for column in &statistics.row_groups[0].columns {
            assert!(column.min_bytes.is_none() && column.max_bytes.is_none());
            assert!(column.geospatial.is_none(), "{}", column.path);
        }
    }

    #[test]
    fn a_geometry_nested_in_a_struct_still_gets_the_logical_type() {
        let media = written(
            "nested.parquet",
            vec![ArrowField::new(
                "place",
                ArrowDataType::Struct(arrow_schema::Fields::from(vec![
                    ArrowField::new("name", ArrowDataType::Utf8, false),
                    extension_field(
                        "shape",
                        ArrowDataType::Binary,
                        "geoarrow.wkb",
                        Some(r#"{"crs": "OGC:CRS84"}"#),
                    ),
                ])),
                false,
            )],
            Vec::new(),
        );

        assert_eq!(
            leaf_logical(&media, "place.shape"),
            Some(LogicalType::geometry(None))
        );
        assert_eq!(
            leaf_logical(&media, "place.name"),
            Some(LogicalType::String)
        );
    }

    #[test]
    fn a_variant_column_writes_the_two_binaries_under_the_variant_logical_type() {
        // A variant column is the format's own group of `metadata` and `value`
        // byte arrays annotated `VARIANT(1)`, so a Parquet reader outside this
        // crate sees a variant rather than an untyped pair of binaries, and the
        // Arrow schema the file carries keeps the canonical extension name.
        let values = [
            yggdryl::Scalar::from(7_i64),
            yggdryl::Scalar::from_struct([("symbol", yggdryl::Scalar::from("AAPL"))]).unwrap(),
        ];
        let media = written(
            "variant.parquet",
            vec![extension_field(
                "payload",
                variant_storage(),
                "arrow.parquet.variant",
                Some(""),
            )],
            vec![variant_column(&values)],
        );

        let group = {
            let builder = open_builder(media.handle()).unwrap();
            let root = builder.parquet_schema().root_schema_ptr();
            Arc::clone(&root.get_fields()[0])
        };
        assert_eq!(
            group.get_basic_info().logical_type_ref(),
            Some(&LogicalType::variant(Some(1)))
        );
        let children = group.get_fields();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name(), "metadata");
        assert_eq!(children[1].name(), "value");
        for child in children {
            assert_eq!(
                child.get_physical_type(),
                parquet::basic::Type::BYTE_ARRAY,
                "{child:?}"
            );
            assert!(!child.get_basic_info().has_id(), "{child:?}");
        }
        assert_eq!(leaf_logical(&media, "payload.metadata"), None);

        let schema = media.read_arrow_schema().unwrap();
        let field = schema.field_with_name("payload").unwrap();
        assert_eq!(
            field.metadata().get("ARROW:extension:name"),
            Some(&"arrow.parquet.variant".to_owned())
        );
        assert_eq!(field.data_type(), &variant_storage());
        let imported = yggdryl::Field::from_arrow_field(field).unwrap();
        assert_eq!(imported.dtype(), &yggdryl::DataType::Variant);

        let options = media.record_options().unwrap();
        let batch = media
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let read: Vec<yggdryl::Scalar> = (0..batch.num_rows())
            .map(|row| {
                let held = yggdryl::arrow::scalar_value(
                    &DataType::Variant.nullable_field("payload"),
                    batch.column(0).slice(row, 1).as_ref(),
                )
                .unwrap();
                let yggdryl::Scalar::Variant(variant) = held else {
                    panic!("a variant value, got {held:?}");
                };
                variant.scalar().unwrap()
            })
            .collect();
        assert_eq!(read, values);
    }

    #[test]
    fn a_variant_group_keeps_only_its_outer_parquet_field_id() {
        let child = |name: &str, id: &str| {
            ArrowField::new(name, ArrowDataType::Binary, false).with_metadata(HashMap::from([(
                "PARQUET:field_id".to_owned(),
                id.to_owned(),
            )]))
        };
        let storage =
            ArrowDataType::Struct(vec![child("metadata", "12"), child("value", "13")].into());
        let metadata = HashMap::from([
            (
                "ARROW:extension:name".to_owned(),
                "arrow.parquet.variant".to_owned(),
            ),
            ("ARROW:extension:metadata".to_owned(), String::new()),
            ("PARQUET:field_id".to_owned(), "11".to_owned()),
        ]);
        let field = ArrowField::new("payload", storage, true).with_metadata(metadata);
        let schema = Schema::new(vec![field]);
        let descriptor = extension_schema(&schema)
            .unwrap()
            .expect("a variant extension needs a descriptor");
        let root = descriptor.root_schema_ptr();
        let group = &root.get_fields()[0];
        assert_eq!(
            group
                .get_basic_info()
                .has_id()
                .then(|| group.get_basic_info().id()),
            Some(11)
        );
        for child in group.get_fields() {
            assert!(!child.get_basic_info().has_id(), "{child:?}");
            assert_eq!(child.get_basic_info().logical_type_ref(), None, "{child:?}");
        }
    }

    #[test]
    fn a_variant_future_version_is_not_recovered() {
        use parquet::basic::Repetition;
        use parquet::schema::types::{SchemaDescriptor, Type};

        let children = ["metadata", "value"].map(|name| {
            Arc::new(
                Type::primitive_type_builder(name, parquet::basic::Type::BYTE_ARRAY)
                    .with_repetition(Repetition::REQUIRED)
                    .build()
                    .unwrap(),
            )
        });
        let group = Type::group_type_builder("payload")
            .with_repetition(Repetition::OPTIONAL)
            .with_logical_type(Some(LogicalType::variant(Some(2))))
            .with_fields(children.to_vec())
            .build()
            .unwrap();
        let root = Type::group_type_builder("arrow_schema")
            .with_fields(vec![Arc::new(group)])
            .build()
            .unwrap();
        let descriptor = SchemaDescriptor::new(Arc::new(root));
        let foreign = Schema::new(vec![ArrowField::new("payload", variant_storage(), true)]);
        assert!(variant_schema(&descriptor, &foreign).is_none());
    }

    #[test]
    fn a_foreign_variant_group_reads_back_as_a_variant() {
        // A file another writer produced carries an Arrow schema of its own -
        // two plain binaries - so the `VARIANT` annotation is all there is to
        // say they are one variant, and reading it is what makes the column
        // import as one.
        use parquet::basic::Repetition;
        use parquet::schema::types::{SchemaDescriptor, Type};

        let children = ["metadata", "value"].map(|name| {
            Arc::new(
                Type::primitive_type_builder(name, parquet::basic::Type::BYTE_ARRAY)
                    .with_repetition(Repetition::REQUIRED)
                    .build()
                    .unwrap(),
            )
        });
        let group = Type::group_type_builder("payload")
            .with_repetition(Repetition::OPTIONAL)
            .with_logical_type(Some(LogicalType::variant(Some(1))))
            .with_fields(children.to_vec())
            .build()
            .unwrap();
        let root = Type::group_type_builder("arrow_schema")
            .with_fields(vec![Arc::new(group)])
            .build()
            .unwrap();
        let descriptor = SchemaDescriptor::new(Arc::new(root));

        // The Arrow schema the foreign writer embeds says nothing of variants.
        let schema = Arc::new(Schema::new(vec![ArrowField::new(
            "payload",
            variant_storage(),
            true,
        )]));
        let values = [yggdryl::Scalar::from(7_i64)];
        let batch =
            RecordBatch::try_new(Arc::clone(&schema), vec![variant_column(&values)]).unwrap();
        let mut encoded = Vec::new();
        let mut writer = parquet::arrow::ArrowWriter::try_new_with_options(
            &mut encoded,
            Arc::clone(&schema),
            parquet::arrow::arrow_writer::ArrowWriterOptions::new().with_parquet_schema(descriptor),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let handle = Buffer::from_bytes(encoded).with_media_type(
            Url::from_str("file:///foreign-variant.parquet")
                .unwrap()
                .media_type(),
        );
        let read = yggdryl::parquet::read_arrow_schema(&handle).unwrap();
        let field = read.field_with_name("payload").unwrap();
        assert_eq!(
            field.metadata().get("ARROW:extension:name"),
            Some(&"arrow.parquet.variant".to_owned()),
            "the annotation names the column a variant"
        );
        assert_eq!(
            yggdryl::Field::from_arrow_field(field).unwrap().dtype(),
            &yggdryl::DataType::Variant
        );

        // And the batches carry it too, so a row reads as the value it holds.
        let batch = yggdryl::parquet::read_batch_reader(
            &handle,
            None,
            &yggdryl::parquet::ParquetOptions::new(),
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
        let held = yggdryl::arrow::scalar_value(
            &DataType::Variant.nullable_field("payload"),
            batch.column(0).slice(0, 1).as_ref(),
        )
        .unwrap();
        let yggdryl::Scalar::Variant(variant) = held else {
            panic!("a variant value, got {held:?}");
        };
        assert_eq!(variant.scalar().unwrap(), values[0]);
    }

    #[test]
    fn foreign_variant_annotations_recover_inside_lists_and_maps() {
        let variant = |name: &str, extension: bool| {
            if extension {
                extension_field(name, variant_storage(), "arrow.parquet.variant", Some(""))
            } else {
                ArrowField::new(name, variant_storage(), false)
            }
        };
        let list = |extension| ArrowDataType::List(Arc::new(variant("item", extension)));
        let map = |extension| {
            ArrowDataType::Map(
                Arc::new(ArrowField::new(
                    "entries",
                    ArrowDataType::Struct(
                        vec![
                            ArrowField::new("key", ArrowDataType::Utf8, false),
                            variant("value", extension),
                        ]
                        .into(),
                    ),
                    false,
                )),
                true,
            )
        };
        let declared = Schema::new(vec![
            ArrowField::new("items", list(true), true),
            ArrowField::new("attributes", map(true), false),
        ]);
        let descriptor = extension_schema(&declared)
            .unwrap()
            .expect("the declared nested variants need annotations");
        // A foreign Arrow footer supplies only physical storage; Parquet's
        // annotation must restore the extension beneath both wrappers.
        let foreign = Schema::new(vec![
            ArrowField::new("items", list(false), true),
            ArrowField::new("attributes", map(false), false),
        ]);
        let recovered =
            variant_schema(&descriptor, &foreign).expect("the annotations restore nested variants");

        let ArrowDataType::List(item) = recovered.field_with_name("items").unwrap().data_type()
        else {
            panic!("a list");
        };
        assert_eq!(item.name(), "item");
        assert!(!item.is_nullable());
        assert_eq!(
            item.metadata().get("ARROW:extension:name"),
            Some(&"arrow.parquet.variant".to_owned())
        );

        let ArrowDataType::Map(entries, sorted) =
            recovered.field_with_name("attributes").unwrap().data_type()
        else {
            panic!("a map");
        };
        assert!(*sorted);
        assert_eq!(entries.name(), "entries");
        assert!(!entries.is_nullable());
        let ArrowDataType::Struct(children) = entries.data_type() else {
            panic!("map entries");
        };
        assert_eq!(children[0].name(), "key");
        assert_eq!(children[1].name(), "value");
        assert!(!children[1].is_nullable());
        assert_eq!(
            children[1].metadata().get("ARROW:extension:name"),
            Some(&"arrow.parquet.variant".to_owned())
        );
    }

    #[test]
    fn reading_back_our_own_file_surfaces_the_extension_metadata() {
        let media = written(
            "roundtrip.parquet",
            vec![extension_field(
                "shape",
                ArrowDataType::Binary,
                "geoarrow.wkb",
                Some(r#"{"crs": "OGC:CRS84"}"#),
            )],
            vec![Arc::new(BinaryArray::from_opt_vec(vec![Some(
                &wkb_point(1.0, 1.0)[..],
            )]))],
        );

        // Our writer embeds the Arrow schema, so the extension identity comes
        // back; a foreign file without that embedding surfaces plain Binary,
        // which is the named read-side limit in the module docs.
        let schema = media.read_arrow_schema().unwrap();
        let field = schema.field_with_name("shape").unwrap();
        assert_eq!(field.data_type(), &ArrowDataType::Binary);
        assert_eq!(
            field.metadata().get("ARROW:extension:name"),
            Some(&"geoarrow.wkb".to_owned())
        );
        assert_eq!(
            field.metadata().get("ARROW:extension:metadata"),
            Some(&r#"{"crs": "OGC:CRS84"}"#.to_owned())
        );
    }

    #[test]
    fn an_ascii_field_round_trips_through_the_embedded_arrow_schema() {
        let declared = yggdryl::Field::new("ccy", yggdryl::DataType::fixed_ascii(4).unwrap(), true);
        let media = written(
            "ascii.parquet",
            vec![declared.clone().into_arrow_field().unwrap()],
            vec![Arc::new(
                arrow_array::FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                    [Some(b"USD\0".as_slice()), None].into_iter(),
                    4,
                )
                .unwrap(),
            )],
        );

        // Our writer embeds the Arrow schema, so the width comes back as
        // the first-class datatype rather than its fixed-binary storage.
        let schema = media.read_arrow_schema().unwrap();
        let field =
            yggdryl::Field::from_arrow_field(schema.field_with_name("ccy").unwrap()).unwrap();
        assert_eq!(field, declared);

        // The stored padding reads back as the trimmed text under a text
        // target: the identity survived the file, so the cast plan trims.
        let options = media.record_options().unwrap();
        let stored = media
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let text =
            yggdryl::StructType::from_fields([yggdryl::DataType::utf8().nullable_field("ccy")])
                .map(yggdryl::DataType::from)
                .unwrap()
                .required_field("row")
                .cast_arrow_batch(stored, ArrowCastOptions::new().with_safe(false))
                .unwrap();
        let ccy = text.column(0).as_string::<i32>();
        assert_eq!(ccy.value(0), "USD");
        assert!(ccy.is_null(1));
    }

    #[test]
    fn scanning_the_stored_wkb_recomputes_the_footer_statistics() {
        let media = written(
            "scan.parquet",
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                extension_field(
                    "shape",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84"}"#),
                ),
            ],
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(BinaryArray::from_opt_vec(vec![
                    Some(&wkb_point(10.0, -2.5)),
                    None,
                    Some(&wkb_point(-4.0, 8.0)),
                ])),
            ],
        );

        let scanned = media.read_parquet_geospatial_statistics("shape").unwrap();
        let bounds = scanned.bounding_box.unwrap();
        assert_eq!(
            (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
            (-4.0, 10.0, -2.5, 8.0)
        );
        assert_eq!(scanned.geometry_types, vec![1]);

        // The scan and the footer answer with the same statistics.
        let statistics = media.read_statistics().unwrap();
        let footer = statistics.row_groups[0]
            .columns
            .iter()
            .find(|column| column.path == "shape")
            .and_then(|column| column.geospatial.clone())
            .unwrap();
        assert_eq!(footer, scanned);
    }

    #[test]
    fn the_scan_refuses_a_column_that_is_not_wkb_by_name() {
        let media = written(
            "refuse.parquet",
            vec![ArrowField::new("id", ArrowDataType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![1]))],
        );

        let message = media
            .read_parquet_geospatial_statistics("id")
            .unwrap_err()
            .to_string();
        assert!(message.contains("expected WKB binary storage"), "{message}");
        assert!(message.contains("got Int64"), "{message}");
        assert!(message.contains("$.id"), "{message}");

        let message = media
            .read_parquet_geospatial_statistics("absent")
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("expected a stored geospatial column"),
            "{message}"
        );
        assert!(message.contains("absent"), "{message}");
    }

    #[test]
    fn a_malformed_geoarrow_document_is_refused_before_any_write() {
        let schema = Arc::new(Schema::new(vec![extension_field(
            "shape",
            ArrowDataType::Binary,
            "geoarrow.wkb",
            Some(r#"{"edges": "diagonal"}"#),
        )]));
        let mut media = Parquet::new(handle("bad-edges.parquet"));
        let options = media.record_options().unwrap();
        let message = media
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, []), &options)
            .unwrap_err()
            .to_string();
        // The native Field import owns extension metadata validation, so its
        // typed metadata error crosses the record surface without an Arrow- or
        // Parquet-specific envelope.
        assert!(message.contains("ARROW:extension:metadata"), "{message}");
        assert!(message.contains("edge algorithm"), "{message}");
        assert!(message.contains("expected one of"), "{message}");
        assert!(message.contains("\"diagonal\""), "{message}");
        // Nothing was published.
        assert!(media.handle().is_empty());
    }

    #[test]
    fn a_field_declared_schema_drives_the_logical_types_end_to_end() {
        // The schema comes from the Field layer's own projection rather than
        // hand-spelled metadata, so the two layers are proven to agree; the
        // rows the variant column carries are the test above's.
        let root = yggdryl::StructType::from_fields([
            yggdryl::DataType::Int64.required_field("id"),
            yggdryl::DataType::geometry(Some("EPSG:3857"))
                .unwrap()
                .nullable_field("shape"),
            yggdryl::DataType::geography(None, Some(yggdryl::EdgeAlgorithm::Vincenty))
                .unwrap()
                .nullable_field("route"),
            yggdryl::DataType::variant().nullable_field("payload"),
        ])
        .map(yggdryl::DataType::from)
        .unwrap()
        .required_field("row");
        let schema = root.clone().into_arrow_schema().unwrap();

        let mut media = Parquet::new(handle("field-declared.parquet"));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, []), &options)
            .unwrap();

        assert_eq!(
            leaf_logical(&media, "shape"),
            Some(LogicalType::geometry(Some("EPSG:3857".to_owned())))
        );
        // The default CRS folds to absence; a non-spherical algorithm rides.
        assert_eq!(
            leaf_logical(&media, "route"),
            Some(LogicalType::geography(
                None,
                Some(EdgeInterpolationAlgorithm::VINCENTY)
            ))
        );
        assert_eq!(leaf_logical(&media, "id"), None);
        let builder = open_builder(media.handle()).unwrap();
        let payload = builder
            .parquet_schema()
            .root_schema_ptr()
            .get_fields()
            .iter()
            .find(|field| field.name() == "payload")
            .cloned()
            .unwrap();
        // A variant column is the format's own annotated group, whether the
        // schema was hand-spelled or projected from a `Field`.
        assert_eq!(
            payload.get_basic_info().logical_type_ref(),
            Some(&LogicalType::variant(Some(1)))
        );
        assert_eq!(payload.get_fields().len(), 2);

        // And the identity survives the read: the reimported root speaks the
        // datatypes the declaration did, extension transport keys stripped.
        let read =
            Field::from_arrow_schema("row", media.read_arrow_schema().unwrap().as_ref()).unwrap();
        assert_eq!(read, root);
    }
}

mod records {

    use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, StringArray};
    use arrow_schema::ArrowError;
    use parquet::basic::Compression;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Field, MediaType, StructType, Url};
    use yggdryl::{IOBase, IOMedia};

    /// Two independent handles over one in-memory byte value, used to exercise
    /// opened-session freshness without involving filesystem mappings.
    #[derive(Clone, Debug)]
    struct Shared {
        handle: Arc<Mutex<Buffer>>,
        media_type: MediaType,
    }

    impl Shared {
        fn new(handle: Buffer) -> Self {
            let media_type = handle.media_type().clone();
            Self {
                handle: Arc::new(Mutex::new(handle)),
                media_type,
            }
        }
    }

    impl yggdryl::IOMedia for Shared {
        yggdryl::impl_default_iomedia!();
    }

    impl IOBase for Shared {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
            self.handle.lock().unwrap().pread(offset, buffer)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> yggdryl::Result<usize> {
            self.handle.lock().unwrap().pwrite(offset, bytes)
        }

        fn size(&self) -> u64 {
            self.handle.lock().unwrap().size()
        }

        fn capacity(&self) -> u64 {
            self.handle.lock().unwrap().capacity()
        }

        fn reserve(&mut self, capacity: u64) -> yggdryl::Result<()> {
            self.handle.lock().unwrap().reserve(capacity)
        }

        fn truncate(&mut self, size: u64) -> yggdryl::Result<()> {
            self.handle.lock().unwrap().truncate(size)
        }

        fn uri(&self) -> Option<&yggdryl::Uri> {
            None
        }

        fn url(&self) -> Option<&Url> {
            None
        }

        fn media_type(&self) -> &MediaType {
            &self.media_type
        }

        fn set_media_type(&mut self, media_type: MediaType) {
            self.handle
                .lock()
                .unwrap()
                .set_media_type(media_type.clone());
            self.media_type = media_type;
        }
    }

    /// A root carrying explicit Iceberg-style field identifiers.
    fn root() -> Field {
        StructType::from_fields([
            DataType::Int64
                .required_field("id")
                .with_parquet_field_id(1),
            DataType::utf8()
                .nullable_field("symbol")
                .with_parquet_field_id(2),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    fn batch(field: &Field, ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
        let schema = field.clone().into_arrow_schema().unwrap();
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .unwrap()
    }

    /// The batches a write takes: one reader over the batches given.
    fn reader<I>(field: &Field, batches: I) -> yggdryl::arrow::BatchReader
    where
        I: IntoIterator<Item = RecordBatch>,
        I::IntoIter: Send + 'static,
    {
        yggdryl::arrow::batch_reader(field.clone().into_arrow_schema().unwrap(), batches)
    }

    /// A one-batch reader that reports whether a write pulled its input.
    fn counted_reader(field: &Field, pulls: Arc<AtomicUsize>) -> yggdryl::arrow::BatchReader {
        let batch = batch(field, vec![1], vec![Some("AAPL")]);
        let batches = std::iter::once(batch).inspect(move |_| {
            pulls.fetch_add(1, Ordering::Relaxed);
        });
        reader(field, batches)
    }

    fn reader_then_error(field: &Field, first: RecordBatch) -> yggdryl::arrow::BatchReader {
        Box::new(RecordBatchIterator::new(
            [
                Ok(first),
                Err(ArrowError::ComputeError(
                    "later Parquet source failure".into(),
                )),
            ],
            field.clone().into_arrow_schema().unwrap(),
        ))
    }

    /// A handle whose media type comes from the name, so codings are declared.
    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    #[test]
    fn batches_round_trip_through_storage() {
        let field = root();
        let mut media = Parquet::new(handle("trades.parquet"));
        let expected = batch(
            &field,
            vec![1, 2, 3],
            vec![Some("AAPL"), None, Some("MSFT")],
        );
        let options = media.record_options().unwrap();

        media
            .overwrite_arrow_reader(reader(&field, [expected.clone()]), &options)
            .unwrap();

        let actual = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0], expected);
        assert_eq!(actual[0].num_rows(), 3);
    }

    #[test]
    fn dimensions_describe_all_batches_and_ignore_read_options() {
        let field = root();
        let mut media = Parquet::new(handle("dimensions.parquet"));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [
                        batch(&field, vec![1, 2], vec![Some("AAPL"), None]),
                        batch(
                            &field,
                            vec![3, 4, 5],
                            vec![Some("MSFT"), Some("NVDA"), None],
                        ),
                    ],
                ),
                &options,
            )
            .unwrap();
        media.options_mut().set_max_row_size(Some(1));
        media.options_mut().set_select("id".parse().unwrap());
        media
            .options_mut()
            .set_filter("id = '999'".parse().unwrap());

        assert_eq!(media.row_size().unwrap(), 5);
        assert_eq!(media.column_size().unwrap(), 2);
    }

    #[test]
    fn an_empty_open_parquet_file_has_explicit_lifecycle_and_dimensions() {
        let field = root();
        let mut media = Parquet::new(handle("empty-open.parquet")).with_field(field.clone());

        media.open().unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 0);
        assert_eq!(media.column_size().unwrap(), field.field_len());

        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                ),
                &options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 2);

        media.clear().unwrap();
        assert!(media.opened());
        assert_eq!(media.row_size().unwrap(), 0);
        assert_eq!(media.column_size().unwrap(), field.field_len());

        let narrowed = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        media.options_mut().set_field(narrowed);
        assert_eq!(
            media.column_size().unwrap(),
            1,
            "an option mutation invalidates the opened width cache"
        );

        media.remove(false).unwrap();
        assert!(!media.opened(), "removal ends the opened session");
    }

    #[test]
    fn an_open_parquet_cache_is_stable_until_close_then_reads_fresh() {
        let field = root();
        let encoded = |ids: Vec<i64>| {
            let symbols = vec![None; ids.len()];
            let mut media = Parquet::new(handle("shared.parquet"));
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(reader(&field, [batch(&field, ids, symbols)]), &options)
                .unwrap();
            media.into_handle()
        };
        let first = encoded(vec![1]);
        let replacement = encoded(vec![1, 2, 3, 4]);
        let shared = Shared::new(first);
        let mut external = shared.clone();
        let mut media = Parquet::new(shared);

        media.open().unwrap();
        assert_eq!(media.row_size().unwrap(), 1);
        external.write_all_bytes(replacement.as_slice()).unwrap();
        assert_eq!(media.row_size().unwrap(), 1, "the open footer is stable");

        media.close().unwrap();
        assert!(!media.opened());
        assert_eq!(media.row_size().unwrap(), 4, "closed reads are fresh");
    }

    #[test]
    fn the_wrapper_owns_parquet_options_over_an_unnamed_buffer() {
        let field = root();
        let media = Parquet::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();

        assert!(matches!(options, RecordOptions::Parquet(_)));
        assert_eq!(options.field(), Some(field));
    }

    #[test]
    fn mismatched_options_are_rejected_before_any_write_pulls_input() {
        let field = root();
        for operation in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
            let mut options = RecordOptions::Ipc(yggdryl::ipc::IpcOptions::new());
            if operation == "merge" {
                options.set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
            }
            let result = match operation {
                "overwrite" => yggdryl::IOMedia::overwrite_arrow_reader(
                    &mut media,
                    counted_reader(&field, Arc::clone(&pulls)),
                    &options,
                ),
                "append" => yggdryl::IOMedia::append_arrow_reader(
                    &mut media,
                    counted_reader(&field, Arc::clone(&pulls)),
                    &options,
                ),
                "merge" => yggdryl::IOMedia::merge_arrow_reader(
                    &mut media,
                    counted_reader(&field, Arc::clone(&pulls)),
                    &options,
                ),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("Parquet"), "{operation}: {message}");
            assert_eq!(pulls.load(Ordering::Relaxed), 0, "{operation}");
            assert!(media.handle().is_empty(), "{operation}");
        }
    }

    #[test]
    fn an_open_footer_tracks_selection_and_completion_on_overwrite() {
        let field = root();
        let mut media = Parquet::new(Buffer::new());
        let initial_options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]),
                &initial_options,
            )
            .unwrap();
        media.open().unwrap();

        let options = media.record_options().unwrap().with_select("id").unwrap();
        yggdryl::IOMedia::overwrite_arrow_reader(
            &mut media,
            reader(
                &field,
                [batch(&field, vec![2, 3], vec![Some("MSFT"), Some("NVDA")])],
            ),
            &options,
        )
        .unwrap();

        assert!(media.opened());
        assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
        assert_eq!(media.read_statistics().unwrap().num_rows, 2);
        let read_options = media.record_options().unwrap();
        let written = media
            .read_arrow_reader(&read_options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(written.num_columns(), 2);
        assert_eq!(written.column(1).null_count(), written.num_rows());
    }

    #[test]
    fn an_open_footer_is_refreshed_by_every_successful_write_mode() {
        let field = root();
        let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), Some("MSFT")])],
                ),
                &options,
            )
            .unwrap();
        assert!(!media.opened(), "a closed write must not start a cache");

        media.open().unwrap();
        assert!(media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 2);
        media.options_mut().set_commit_row_size(Some(1));

        let overwrite_options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(&field, [batch(&field, vec![3], vec![Some("NVDA")])]),
                &overwrite_options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 1);

        let append_options = media.record_options().unwrap();
        media
            .append_arrow_reader(
                reader(&field, [batch(&field, vec![4, 5], vec![Some("AMD"), None])]),
                &append_options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 3);

        media
            .options_mut()
            .set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
        let merge_options = media.record_options().unwrap();
        media
            .merge_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![4, 6], vec![Some("INTC"), Some("ARM")])],
                ),
                &merge_options,
            )
            .unwrap();
        assert!(media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 4);

        media.close().unwrap();
        assert!(!media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 4);
    }

    #[test]
    fn a_partial_commit_refreshes_the_open_parquet_footer() {
        let field = root();
        let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), Some("MSFT")])],
                ),
                &options,
            )
            .unwrap();
        media.open().unwrap();
        media.options_mut().set_commit_row_size(Some(1));

        let options = media.record_options().unwrap();
        let message = media
            .overwrite_arrow_reader(
                reader_then_error(&field, batch(&field, vec![7], vec![Some("NVDA")])),
                &options,
            )
            .unwrap_err()
            .to_string();

        assert!(
            message.contains("later Parquet source failure"),
            "{message}"
        );
        assert!(media.opened());
        assert_eq!(media.read_statistics().unwrap().num_rows, 1);
        assert_eq!(media.row_size().unwrap(), 1);
        assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
    }

    #[test]
    fn field_identifiers_survive_the_round_trip() {
        let field = root();
        let mut media = Parquet::new(handle("ids.parquet"));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]),
                &options,
            )
            .unwrap();

        // Ids are what an Iceberg reader resolves columns by, so they must not be
        // positional after a round trip.
        let schema = media.read_arrow_schema().unwrap();
        assert_eq!(
            schema.field(0).metadata().get("PARQUET:field_id"),
            Some(&"1".to_owned())
        );
        assert_eq!(
            schema.field(1).metadata().get("PARQUET:field_id"),
            Some(&"2".to_owned())
        );

        let recovered = media.read_arrow_field(&options).unwrap();
        let fields = recovered.dtype().as_fields().unwrap();
        assert_eq!(fields[0].parquet_field_id().unwrap(), Some(1));
        assert_eq!(fields[1].parquet_field_id().unwrap(), Some(2));
    }

    #[test]
    fn an_empty_write_still_publishes_a_readable_file() {
        let field = root();
        let mut media = Parquet::new(handle("empty.parquet"));
        let options = media.record_options().unwrap();

        media
            .overwrite_arrow_reader(reader(&field, []), &options)
            .unwrap();

        assert!(!media.handle().is_empty());
        assert!(
            media
                .read_arrow_reader(&options)
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>()
                .is_empty()
        );
        assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
        assert_eq!(media.read_statistics().unwrap().num_rows, 0);
    }

    #[test]
    fn a_coded_location_is_rejected_with_the_reason() {
        let field = root();

        for name in ["trades.parquet.gz", "trades.parquet.zst"] {
            let mut media = Parquet::new(handle(name));
            let options = media.record_options().unwrap();
            let message = media
                .overwrite_arrow_reader(reader(&field, []), &options)
                .unwrap_err()
                .to_string();
            assert!(message.contains("compresses"), "{name}: {message}");
            assert!(
                message.contains("ParquetOptions::compression"),
                "{name}: {message}"
            );
            // Nothing was published.
            assert!(media.handle().is_empty(), "{name}");
        }
    }

    #[test]
    fn a_mismatched_batch_reports_which_index_disagreed() {
        let field = root();
        let other = StructType::from_fields([DataType::utf8().required_field("unrelated")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let mut media = Parquet::new(handle("mismatch.parquet"));
        let options = media.record_options().unwrap();

        let good = batch(&field, vec![1], vec![Some("AAPL")]);
        let bad = RecordBatch::try_new(
            other.into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["x"]))],
        )
        .unwrap();

        let message = media
            .overwrite_arrow_reader(reader(&field, [good, bad]), &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("index 1"), "{message}");
        assert!(media.handle().is_empty());
    }

    #[test]
    fn every_compression_round_trips_and_changes_the_bytes() {
        let field = root();
        // A payload with structure so compression has something to remove.
        let ids: Vec<i64> = (0..4_000).collect();
        let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
        let source = batch(&field, ids, symbols);

        let mut sizes = Vec::new();
        for (name, compression) in [
            ("none.parquet", Compression::UNCOMPRESSED),
            ("snappy.parquet", Compression::SNAPPY),
            ("zstd.parquet", Compression::ZSTD(Default::default())),
        ] {
            // Read the whole file as one batch so the comparison is not split by
            // the reader's default batch size.
            let mut media = Parquet::new(handle(name)).with_options(
                ParquetOptions::new()
                    .with_compression(compression)
                    .with_batch_row_size(source.num_rows()),
            );
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(reader(&field, [source.clone()]), &options)
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            let actual = media
                .read_arrow_reader(&options)
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), 1, "{name}");
            assert_eq!(actual[0], source, "{name}");
            sizes.push(media.handle().size() as usize);
        }

        // Compression is recovered from the footer, so every file reads back the
        // same rows while the uncompressed one is the largest.
        assert!(sizes[0] > sizes[1], "{sizes:?}");
        assert!(sizes[0] > sizes[2], "{sizes:?}");
    }

    #[test]
    fn a_bounded_batch_row_size_splits_the_read() {
        let field = root();
        let mut media = Parquet::new(handle("batched.parquet"))
            .with_options(ParquetOptions::new().with_batch_row_size(256));
        let ids: Vec<i64> = (0..1_000).collect();
        let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(&field, [batch(&field, ids, symbols)]), &options)
            .unwrap();

        let batches = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert!(batches.len() >= 4, "{}", batches.len());
        assert_eq!(
            batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
            1_000
        );
    }

    /// Column pushdown: a schema naming fewer columns becomes a projection mask,
    /// which is the format's own way of not reading a column chunk.
    mod pushdown {

        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{
            Array, Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray,
        };

        use super::handle;
        use yggdryl::IOMedia;
        use yggdryl::media::IORecordOptions;
        use yggdryl::parquet::Parquet;
        use yggdryl::{DataType, Field};

        /// Four columns, so a two-column read is a genuine subset.
        fn wide() -> Field {
            StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("symbol"),
                DataType::Float64.required_field("price"),
                DataType::utf8().nullable_field("venue"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
        }

        /// The two columns a caller actually wants.
        fn narrow() -> Field {
            StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Float64.required_field("price"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
        }

        /// A file wide enough that skipping two columns is measurable.
        fn stored() -> Parquet<yggdryl::holder::Buffer> {
            let rows = 4_096;
            let ids: Vec<i64> = (0..rows).collect();
            let batch = RecordBatch::try_new(
                wide().into_arrow_schema().unwrap(),
                vec![
                    Arc::new(Int64Array::from(ids.clone())),
                    Arc::new(StringArray::from(
                        ids.iter().map(|id| format!("SYM{id}")).collect::<Vec<_>>(),
                    )),
                    #[allow(clippy::cast_precision_loss)]
                    Arc::new(Float64Array::from(
                        ids.iter().map(|id| *id as f64).collect::<Vec<_>>(),
                    )),
                    Arc::new(StringArray::from(
                        ids.iter()
                            .map(|id| format!("VENUE{id}"))
                            .collect::<Vec<_>>(),
                    )),
                ],
            )
            .unwrap();

            let mut media = Parquet::new(handle("pushdown.parquet"));
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                    &options,
                )
                .unwrap();
            media
        }

        /// Total bytes every array in a read occupies, which is the data the read
        /// actually moved into Arrow memory.
        fn materialized(reader: yggdryl::arrow::BatchReader) -> usize {
            reader
                .map(std::result::Result::unwrap)
                .map(|batch| {
                    batch
                        .columns()
                        .iter()
                        .map(|column| column.get_array_memory_size())
                        .sum::<usize>()
                })
                .sum()
        }

        #[test]
        fn a_subset_schema_is_pushed_into_the_file_rather_than_applied_after_it() {
            let media = stored();
            let options = media.record_options().unwrap();

            // The file stores four columns; nothing about it changed.
            assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 4);
            assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 4);

            let options = options.with_field(narrow());
            let reader = media.read_arrow_reader(&options).unwrap();
            // The projection is known before a single batch is decoded.
            assert_eq!(reader.schema().fields().len(), 2);
            assert_eq!(reader.schema().field(0).name(), "id");
            assert_eq!(reader.schema().field(1).name(), "price");

            let batches = reader.map(std::result::Result::unwrap).collect::<Vec<_>>();
            assert!(!batches.is_empty());
            for batch in &batches {
                assert_eq!(batch.num_columns(), 2);
            }
            assert_eq!(
                batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
                4_096
            );
        }

        #[test]
        fn the_projected_read_materializes_less_than_the_whole_file() {
            let media = stored();
            let options = media.record_options().unwrap();

            let whole = materialized(media.read_arrow_reader(&options).unwrap());
            let subset = materialized(
                media
                    .read_arrow_reader(&options.with_field(narrow()))
                    .unwrap(),
            );

            // The two string columns are the bulk of this file, and a pushed-down
            // read never builds them.
            assert!(subset * 2 < whole, "subset {subset} bytes, whole {whole}");
        }

        #[test]
        fn a_column_the_file_does_not_store_is_completed_after_pushdown() {
            let media = stored();
            let options = media.record_options().unwrap();

            // A mask can only drop columns, so the encoding reads what is present
            // and the canonical declared-Field cast supplies the absent column.
            let invented = StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("nowhere"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");

            let batches = media
                .read_arrow_reader(&options.with_field(invented))
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>();
            assert!(!batches.is_empty());
            for batch in &batches {
                assert_eq!(batch.num_columns(), 2);
                assert_eq!(batch.schema().field(1).name(), "nowhere");
            }
            assert_eq!(
                batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
                4_096
            );
            assert_eq!(
                batches
                    .iter()
                    .map(|batch| batch.column(1).null_count())
                    .sum::<usize>(),
                4_096
            );
        }
    }

    mod limits {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use arrow_array::RecordBatchReader;
        use parquet::basic::Compression;

        use super::{batch, handle, reader, root};
        use yggdryl::holder::Buffer;
        use yggdryl::media::{IORecordOptions, RecordOptions};
        use yggdryl::parquet::{Parquet, ParquetOptions};
        use yggdryl::{IOBase, IOMedia};

        /// The total rows a handle yields under `options`.
        fn rows<H: IOBase + ?Sized>(handle: &H, options: &RecordOptions) -> usize {
            handle
                .read_arrow_reader(options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum()
        }

        #[test]
        fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
            let field = root();
            let mut handle = handle("limited.parquet");
            let options = handle.record_options().unwrap().with_field(field.clone());
            handle
                .overwrite_arrow_reader(
                    reader(
                        &field,
                        [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                    ),
                    &options,
                )
                .unwrap();

            let mut limited = handle
                .read_arrow_reader(&options.with_max_row_size(0))
                .unwrap();
            // The schema is asserted, not only the emptiness: `Some(0)` is a
            // valid ask that still says what the rows would have been.
            assert_eq!(limited.schema(), field.clone().into_arrow_schema().unwrap());
            assert!(limited.next().is_none());
        }

        #[test]
        fn a_limited_write_truncates_what_the_caller_offered() {
            let field = root();
            let mut handle = handle("truncated.parquet");
            let options = handle.record_options().unwrap().with_field(field.clone());

            handle
                .overwrite_arrow_reader(
                    reader(
                        &field,
                        [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                    ),
                    &options.clone().with_max_row_size(1),
                )
                .unwrap();
            assert_eq!(rows(&handle, &options), 1);

            // An append is a write, so the same bound truncates it the same way.
            handle
                .append_arrow_reader(
                    reader(&field, [batch(&field, vec![3, 4], vec![None, None])]),
                    &options.clone().with_max_row_size(1),
                )
                .unwrap();
            assert_eq!(rows(&handle, &options), 2);
        }

        /// A handle counting the read calls and bytes that reach the one it
        /// wraps, so a test can say what a read actually fetched.
        struct Counting {
            handle: Buffer,
            reads: AtomicUsize,
            bytes: AtomicUsize,
        }

        impl Counting {
            fn new(handle: Buffer) -> Self {
                Self {
                    handle,
                    reads: AtomicUsize::new(0),
                    bytes: AtomicUsize::new(0),
                }
            }

            /// One measured run: the reads and bytes `operation` costs.
            fn cost(&self, operation: impl FnOnce()) -> (usize, usize) {
                let reads = self.reads.load(Ordering::Relaxed);
                let bytes = self.bytes.load(Ordering::Relaxed);
                operation();
                (
                    self.reads.load(Ordering::Relaxed) - reads,
                    self.bytes.load(Ordering::Relaxed) - bytes,
                )
            }
        }

        impl yggdryl::IOMedia for Counting {
            yggdryl::impl_default_iomedia!();
        }

        impl IOBase for Counting {
            yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve,
                truncate, uri, url, media_type, set_media_type, flush, parent, child_by_path,
                ls, kind, clear, remove, is_atomic, is_tabular);

            fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
                let read = self.handle.pread(offset, buffer)?;
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.bytes.fetch_add(read, Ordering::Relaxed);
                Ok(read)
            }
        }

        #[test]
        fn a_small_row_bound_over_many_row_groups_stops_reading_early() {
            // Thirty-two uncompressed row groups, so one group is a small
            // fraction of the file and the fraction shows up in bytes read.
            let field = root();
            let total = 16_384_usize;
            let mut media = Parquet::new(handle("grouped.parquet")).with_options(
                ParquetOptions::new()
                    .with_compression(Compression::UNCOMPRESSED)
                    .with_max_row_group_size(512),
            );
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    reader(
                        &field,
                        [batch(
                            &field,
                            (0..total as i64).collect(),
                            vec![None; total],
                        )],
                    ),
                    &options,
                )
                .unwrap();
            assert_eq!(media.read_statistics().unwrap().row_groups.len(), 32);

            let counting = Counting::new(media.into_handle());
            let options = counting.record_options().unwrap();

            // The logical dimension is a two-range footer read: neither column
            // pages nor row arrays are fetched.
            let (dimension_reads, dimension_bytes) = counting.cost(|| {
                assert_eq!(counting.row_size().unwrap(), total as u64);
            });
            assert_eq!(dimension_reads, 2, "tail and footer metadata");
            assert!(
                dimension_bytes < counting.size() as usize,
                "{dimension_bytes} footer bytes vs {} file bytes",
                counting.size()
            );

            // The full drain of a file this small is one whole-value read.
            let (full_reads, full_bytes) = counting.cost(|| {
                assert_eq!(rows(&counting, &options), total);
            });
            assert_eq!(full_reads, 1, "one whole-value read");
            assert_eq!(full_bytes as u64, counting.size());

            // Five rows out of 16,384: the tail, the footer, and one leading
            // row-group prefix - the other thirty-one groups are never read.
            let (limited_reads, limited_bytes) = counting.cost(|| {
                assert_eq!(rows(&counting, &options.clone().with_max_row_size(5)), 5);
            });
            assert_eq!(limited_reads, 3, "tail, footer, one-group prefix");
            assert!(
                limited_bytes * 4 < full_bytes,
                "{limited_bytes} bytes under the bound vs {full_bytes} for the drain"
            );
        }
    }
}

/// The threads a read decodes on and a write encodes on, seen from outside:
/// the same rows, the same bounds, the same bytes as one thread produces.
mod parallel {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
    use parquet::arrow::ArrowWriter;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use parquet::basic::{Compression, ZstdLevel};
    use parquet::file::properties::WriterProperties;
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, StructType};

    /// Three columns, one of them incompressible, so a few hundred thousand
    /// rows clear the size a read or a write splits across threads at.
    fn field() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    fn rows(count: usize) -> RecordBatch {
        let ids: Vec<i64> = (0..i64::try_from(count).unwrap()).collect();
        // A deterministic scramble: every price distinct, none predictable.
        let prices: Vec<f64> = ids
            .iter()
            .map(|id| (id.wrapping_mul(6_364_136_223_846_793_005) >> 11) as f64)
            .collect();
        let symbols: Vec<Option<&str>> = ids
            .iter()
            .map(|id| {
                ["AAPL", "MSFT", "GOOG"]
                    .get(usize::try_from(id % 4).unwrap())
                    .copied()
            })
            .collect();
        RecordBatch::try_new(
            field().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(ids)) as ArrayRef,
                Arc::new(Float64Array::from(prices)) as ArrayRef,
                Arc::new(StringArray::from(symbols)) as ArrayRef,
            ],
        )
        .unwrap()
    }

    /// Options that decode and encode on four threads whatever the host
    /// offers, where the `internals` hook can pin them.
    fn threaded(options: ParquetOptions) -> ParquetOptions {
        #[cfg(feature = "internals")]
        let options = yggdryl::internals::parquet::with_threads(options, 4);
        options
    }

    fn written(batch: &RecordBatch, options: ParquetOptions) -> Parquet<Buffer> {
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(threaded(options));
        let record = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &record,
            )
            .unwrap();
        media
    }

    /// What the `parquet` crate's own reader, on one thread, reads back.
    fn reference(media: &Parquet<Buffer>, columns: Option<&[usize]>) -> RecordBatch {
        let bytes = bytes::Bytes::from(media.handle().read_all_bytes().unwrap());
        let builder = ParquetRecordBatchReaderBuilder::try_new(bytes).unwrap();
        let builder = match columns {
            Some(columns) => {
                let mask = parquet::arrow::ProjectionMask::roots(
                    builder.parquet_schema(),
                    columns.iter().copied(),
                );
                builder.with_projection(mask)
            }
            None => builder,
        };
        let reader = builder.build().unwrap();
        let schema = arrow_array::RecordBatchReader::schema(&reader);
        let batches: Vec<RecordBatch> = reader.map(Result::unwrap).collect();
        arrow_select::concat::concat_batches(&schema, &batches).unwrap()
    }

    // Only a read split across threads cuts its batches at every row group;
    // one reader fills a batch across them.
    #[cfg(feature = "internals")]
    #[test]
    fn row_groups_decoded_side_by_side_come_back_in_file_order() {
        let batch = rows(300_000);
        let media = written(
            &batch,
            ParquetOptions::new().with_max_row_group_size(100_000),
        );
        let options = media.record_options().unwrap();

        let batches: Vec<RecordBatch> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(Result::unwrap)
            .collect();

        // No batch exceeds the default size, and none spans two row groups:
        // every group boundary is a batch boundary.
        let mut ends = Vec::new();
        let mut end = 0;
        for read in &batches {
            assert!(read.num_rows() <= yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE);
            end += read.num_rows();
            ends.push(end);
        }
        for boundary in [100_000, 200_000, 300_000] {
            assert!(ends.contains(&boundary), "{boundary} in {ends:?}");
        }
        let joined = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap();
        assert_eq!(joined, reference(&media, None));
        assert_eq!(joined.num_rows(), 300_000);
    }

    #[test]
    fn one_row_group_splits_its_projected_columns_and_joins_them_back() {
        let batch = rows(300_000);
        let media = written(&batch, ParquetOptions::new());
        let projected = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let options = media.record_options().unwrap().with_field(projected);

        let batches: Vec<RecordBatch> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(Result::unwrap)
            .collect();

        assert_eq!(
            batches[0].num_rows(),
            yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE,
            "an unbounded read takes the crate's batch size"
        );
        let joined = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap();
        let expected = reference(&media, Some(&[0, 1]));
        assert_eq!(joined.num_columns(), 2);
        assert_eq!(joined.column(0), expected.column(0));
        assert_eq!(joined.column(1), expected.column(1));
    }

    #[test]
    fn columns_encoded_side_by_side_write_the_sequential_writers_bytes() {
        // Two row groups, each well past the size that encodes on threads.
        let batch = rows(300_000);
        let media = written(
            &batch,
            ParquetOptions::new().with_max_row_group_size(200_000),
        );

        let mut expected = Vec::new();
        let properties = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .set_max_row_group_row_count(Some(200_000))
            .build();
        let mut writer =
            ArrowWriter::try_new(&mut expected, batch.schema(), Some(properties)).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        assert_eq!(media.handle().read_all_bytes().unwrap(), expected);
    }

    #[cfg(feature = "internals")]
    #[test]
    fn one_thread_reads_the_same_rows_as_four() {
        let batch = rows(300_000);
        let media = written(
            &batch,
            ParquetOptions::new().with_max_row_group_size(100_000),
        );
        let read = |threads: usize| {
            let options = yggdryl::media::RecordOptions::from(
                yggdryl::internals::parquet::with_threads(media.options().clone(), threads),
            );
            let batches: Vec<RecordBatch> = media
                .read_arrow_reader(&options)
                .unwrap()
                .map(Result::unwrap)
                .collect();
            arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap()
        };
        assert_eq!(read(1), read(4));
        assert_eq!(read(1), reference(&media, None));
    }

    #[test]
    fn a_failing_row_group_fails_the_read_rather_than_ending_it() {
        let batch = rows(300_000);
        let mut media = written(
            &batch,
            ParquetOptions::new().with_max_row_group_size(100_000),
        );
        let bytes = media.handle().read_all_bytes().unwrap();
        let footer = parquet::file::metadata::ParquetMetaDataReader::new()
            .parse_and_finish(&bytes::Bytes::from(bytes))
            .unwrap();
        // Break the last row group's pages, footer untouched.
        let last = footer.row_group(2);
        let start = last
            .columns()
            .iter()
            .map(|column| column.byte_range().0)
            .min()
            .unwrap();
        let end = last
            .columns()
            .iter()
            .map(|column| column.byte_range().0 + column.byte_range().1)
            .max()
            .unwrap();
        media
            .pwrite(start, &vec![0xA5_u8; usize::try_from(end - start).unwrap()])
            .unwrap();
        let options = media.record_options().unwrap();
        let mut rows = 0;
        let mut failed = false;
        for read in media.read_arrow_reader(&options).unwrap() {
            match read {
                Ok(read) => rows += read.num_rows(),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(
            failed,
            "the broken row group is an error, after {rows} rows"
        );
        assert!(rows <= 200_000, "{rows}");
    }

    #[test]
    fn nested_columns_split_across_threads_join_back() {
        use arrow_array::builder::{Int64Builder, ListBuilder};
        use arrow_array::{Array, StructArray};

        let count = 200_000_usize;
        let mut legs = ListBuilder::new(Int64Builder::new());
        for row in 0..count {
            for leg in 0..(row % 4) {
                legs.values()
                    .append_value(i64::try_from(row * 10 + leg).unwrap());
            }
            legs.append(row % 7 != 0);
        }
        let legs = legs.finish();
        let ids = Int64Array::from_iter_values(0..i64::try_from(count).unwrap());
        let prices = Float64Array::from_iter_values((0..count).map(|row| row as f64 * 0.5));
        let quote = StructArray::from(vec![
            (
                Arc::new(arrow_schema::Field::new(
                    "bid",
                    arrow_schema::DataType::Float64,
                    false,
                )),
                Arc::new(prices.clone()) as ArrayRef,
            ),
            (
                Arc::new(arrow_schema::Field::new(
                    "size",
                    arrow_schema::DataType::Int64,
                    false,
                )),
                Arc::new(ids.clone()) as ArrayRef,
            ),
        ]);
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
            arrow_schema::Field::new("legs", legs.data_type().clone(), true),
            arrow_schema::Field::new("quote", quote.data_type().clone(), false),
            arrow_schema::Field::new("price", arrow_schema::DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(ids) as ArrayRef,
                Arc::new(legs) as ArrayRef,
                Arc::new(quote) as ArrayRef,
                Arc::new(prices) as ArrayRef,
            ],
        )
        .unwrap();
        // One row group, so the read deals its columns across threads.
        let media = written(&batch, ParquetOptions::new());
        let options = media.record_options().unwrap();
        let batches: Vec<RecordBatch> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let joined = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap();
        assert_eq!(joined, reference(&media, None));
    }

    #[test]
    fn a_limited_read_under_a_filter_keeps_the_full_batch_size() {
        let batch = rows(300_000);
        let media = written(&batch, ParquetOptions::new());
        let mut options = media.options().clone();
        options.set_max_row_size(Some(1));
        let options = options.with_filter("id >= 0").unwrap();
        // The limit counts rows the filter keeps, so it does not size the
        // batches the file is decoded in.
        let first = yggdryl::parquet::read_batch_reader(media.handle(), None, &options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            first.num_rows(),
            yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE
        );
    }

    // Pinned to four threads, so the failing column is one of several being
    // encoded side by side.
    #[cfg(feature = "internals")]
    #[test]
    fn a_column_the_writer_cannot_encode_reports_its_own_failure() {
        use arrow_array::Array as _;
        use arrow_array::IntervalMonthDayNanoArray;
        use arrow_array::types::IntervalMonthDayNano;

        let count = 200_000_usize;
        // Twelve light columns ahead of the failing one, so more columns than
        // threads remain when it fails.
        let mut fields = Vec::new();
        let mut columns: Vec<ArrayRef> = Vec::new();
        for index in 0..12 {
            fields.push(arrow_schema::Field::new(
                format!("c{index}"),
                arrow_schema::DataType::Int64,
                false,
            ));
            columns.push(Arc::new(Int64Array::from_iter_values(
                0..i64::try_from(count).unwrap(),
            )));
        }
        let spans = IntervalMonthDayNanoArray::from_iter_values(
            (0..count).map(|row| IntervalMonthDayNano::new(0, i32::try_from(row % 30).unwrap(), 0)),
        );
        fields.push(arrow_schema::Field::new(
            "span",
            spans.data_type().clone(),
            false,
        ));
        columns.push(Arc::new(spans));
        let schema = Arc::new(arrow_schema::Schema::new(fields));
        let batch = RecordBatch::try_new(Arc::clone(&schema), columns).unwrap();
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(threaded(ParquetOptions::new()));
        let record = media.record_options().unwrap();
        match media.overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &record) {
            Err(error) => {
                let message = error.to_string();
                assert!(
                    message.contains("MonthDayNano") || message.contains("not yet implemented"),
                    "{message}"
                );
            }
            // A writer that learns to encode the interval wrote every row.
            Ok(()) => assert_eq!(media.row_size().unwrap(), count as u64),
        }
    }

    #[cfg(feature = "internals")]
    #[test]
    fn a_row_group_fed_in_several_feeds_writes_the_sequential_writers_bytes() {
        let batch = rows(300_000);
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
        // A 64 KiB feed, so one row group is encoded over dozens of feeds.
        yggdryl::internals::parquet::overwrite_with_write_buffer(
            media.handle_mut(),
            yggdryl::arrow::batch_reader(
                batch.schema(),
                (0..300_000)
                    .step_by(20_000)
                    .map(|start| batch.slice(start, 20_000))
                    .collect::<Vec<_>>(),
            ),
            &threaded(ParquetOptions::new().with_max_row_group_size(200_000)),
            64 * 1024,
        )
        .unwrap();

        let mut expected = Vec::new();
        let properties = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .set_max_row_group_row_count(Some(200_000))
            .build();
        // The same slices, one write each: page cuts follow the calls.
        let mut writer =
            ArrowWriter::try_new(&mut expected, batch.schema(), Some(properties)).unwrap();
        for start in (0..300_000).step_by(20_000) {
            writer.write(&batch.slice(start, 20_000)).unwrap();
        }
        writer.close().unwrap();

        assert!(media.handle().read_all_bytes().unwrap() == expected);
    }

    // A limited read decodes on one reader, which fills a batch across a row
    // group boundary where threads would cut it there.
    #[cfg(feature = "internals")]
    #[test]
    fn a_limited_read_stays_on_one_reader() {
        let batch = rows(300_000);
        let media = written(
            &batch,
            ParquetOptions::new().with_max_row_group_size(100_000),
        );
        let options = yggdryl::media::RecordOptions::from(
            yggdryl::internals::parquet::with_threads(media.options().clone(), 4),
        )
        .with_max_row_size(250_000);
        let sizes: Vec<usize> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(|read| read.unwrap().num_rows())
            .collect();
        assert_eq!(sizes[..2], [65_536, 65_536], "{sizes:?}");
        assert_eq!(sizes.iter().sum::<usize>(), 250_000);
    }

    /// A limited read of a folder decodes each leaf on one reader too: the
    /// limit is applied over the whole folder, and a leaf decoding ahead of
    /// it on every thread would decode rows no one reads.
    #[test]
    fn a_limited_folder_read_decodes_its_leaves_on_one_reader() {
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-parquet-limited-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let batch = rows(300_000);
        let mut leaf = Parquet::new(LocalFile::create(root.join("part-0.parquet")).unwrap())
            .with_options(ParquetOptions::new().with_max_row_group_size(100_000));
        let options = leaf.record_options().unwrap();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        leaf.flush().unwrap();
        drop(leaf);

        let folder = LocalFolder::new(&root).unwrap();
        let options = folder.record_options().unwrap().with_max_row_size(250_000);
        let sizes: Vec<usize> = folder
            .read_arrow_reader(&options)
            .unwrap()
            .map(|read| read.unwrap().num_rows())
            .collect();
        // One reader fills a batch across the row group boundary at 100,000.
        assert_eq!(sizes[..2], [65_536, 65_536], "{sizes:?}");
        assert_eq!(sizes.iter().sum::<usize>(), 250_000);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Batches a read returned stay whole after the file they came from is
    /// rewritten shorter - including view columns whose values, decoded from
    /// uncompressed pages, sit in the page buffers themselves.
    #[test]
    fn view_columns_outlive_a_shorter_rewrite_of_their_file() {
        use arrow_array::{Array as _, StringViewArray};
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!(
            "yggdryl-parquet-views-{}.parquet",
            std::process::id()
        ));
        let _ = LocalFile::new(&path).unwrap().remove(false);
        let count = 200_000_usize;
        let notes = StringViewArray::from_iter_values(
            (0..count).map(|row| format!("a note long enough to live out of line {row:08}")),
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "note",
            notes.data_type().clone(),
            false,
        )]));
        let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(notes)]).unwrap();
        let mut bytes = Vec::new();
        let properties = WriterProperties::builder()
            .set_compression(Compression::UNCOMPRESSED)
            .build();
        let mut writer =
            ArrowWriter::try_new(&mut bytes, Arc::clone(&schema), Some(properties)).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let mut file = LocalFile::create(&path).unwrap();
        file.write_all_bytes(&bytes).unwrap();
        file.flush().unwrap();

        let mut media = Parquet::new(file);
        let options = media.record_options().unwrap();
        let read: Vec<RecordBatch> = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(read[0].schema(), [read[0].slice(0, 3)]),
                &options,
            )
            .unwrap();
        media.flush().unwrap();

        let joined = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
        let expected = arrow_cast::cast(batch.column(0), joined.column(0).data_type()).unwrap();
        assert_eq!(joined.column(0).as_ref(), expected.as_ref());
        drop(media);
        let _ = LocalFile::new(&path).unwrap().remove(false);
    }

    /// A file rewritten from a streamed read of itself - in commits, on one
    /// lazy reader that is still decoding when the first commit lands.
    #[cfg(feature = "internals")]
    #[test]
    fn a_file_rewritten_from_a_streamed_read_of_itself_keeps_every_row() {
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!(
            "yggdryl-parquet-self-{}.parquet",
            std::process::id()
        ));
        let _ = LocalFile::new(&path).unwrap().remove(false);
        let batch = rows(600_000);
        let mut media = Parquet::new(LocalFile::create(&path).unwrap()).with_options(
            yggdryl::internals::parquet::with_threads(
                ParquetOptions::new().with_max_row_group_size(100_000),
                1,
            ),
        );
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();
        media.flush().unwrap();

        // Every row group holds nulls, so the filter prunes none of them.
        let reader = media
            .read_arrow_reader(&options.clone().with_filter("symbol is not null").unwrap())
            .unwrap();
        let mut streamed = options.clone();
        streamed.set_commit_row_size(Some(50_000));
        media.overwrite_arrow_reader(reader, &streamed).unwrap();
        media.flush().unwrap();
        assert_eq!(media.row_size().unwrap(), 450_000);
        drop(media);
        let _ = LocalFile::new(&path).unwrap().remove(false);
    }
}

/// The read's filter answered from the footer before a page is decoded.
mod pruning {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, StructType};

    /// Two row groups of sorted ids: 0..1000 and 1000..2000.
    fn written() -> Parquet<Buffer> {
        let field = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("note"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let ids: Vec<i64> = (0..2_000).collect();
        let notes: Vec<String> = ids.iter().map(|id| format!("note {id}")).collect();
        let batch = RecordBatch::try_new(
            field.into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(ids)) as ArrayRef,
                Arc::new(StringArray::from(notes)) as ArrayRef,
            ],
        )
        .unwrap();
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(ParquetOptions::new().with_max_row_group_size(1_000));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        media
    }

    fn rows(media: &Parquet<Buffer>, filter: &str) -> yggdryl::Result<usize> {
        let options = media.record_options()?.with_filter(filter)?;
        let mut rows = 0;
        for batch in media.read_arrow_reader(&options)? {
            rows += batch.map_err(yggdryl::Error::Arrow)?.num_rows();
        }
        Ok(rows)
    }

    #[test]
    fn a_row_group_the_statistics_rule_out_is_never_decoded() {
        let mut media = written();
        // Break every page of the second row group, footer untouched: a read
        // that decodes it fails, one that skips it does not.
        let bytes = media.handle().read_all_bytes().unwrap();
        let footer = parquet::file::metadata::ParquetMetaDataReader::new()
            .parse_and_finish(&bytes::Bytes::from(bytes))
            .unwrap();
        let second = footer.row_group(1);
        let start = second
            .columns()
            .iter()
            .map(|column| column.byte_range().0)
            .min()
            .unwrap();
        let end = second
            .columns()
            .iter()
            .map(|column| column.byte_range().0 + column.byte_range().1)
            .max()
            .unwrap();
        let garbage = vec![0xA5_u8; usize::try_from(end - start).unwrap()];
        media.pwrite(start, &garbage).unwrap();

        assert_eq!(rows(&media, "id < 10").unwrap(), 10);
        assert_eq!(
            rows(&media, "id between 100 and 199 and note is not null").unwrap(),
            100
        );
        assert!(
            rows(&media, "id >= 0").is_err(),
            "the broken group is decoded"
        );
    }

    #[test]
    fn a_column_the_declared_root_casts_prunes_nothing() {
        // Stored as text, read as a number: "10" < "9" as text, so the stored
        // bounds would rule out the one row the cast value keeps.
        let stored = StructType::from_fields([DataType::utf8().required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let batch = RecordBatch::try_new(
            stored.into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["10", "2", "3"])) as ArrayRef],
        )
        .unwrap();
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        let read = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let options = media
            .record_options()
            .unwrap()
            .with_field(read)
            .with_filter("id > 5")
            .unwrap();
        let kept: usize = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(kept, 1);
    }

    /// One Parquet file of one row group, written from `batch`.
    fn file_of(batch: &RecordBatch) -> Parquet<Buffer> {
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();
        media
    }

    /// The `i` column of the rows a filter keeps, under an optional root.
    fn kept(media: &Parquet<Buffer>, root: Option<yggdryl::Field>, filter: &str) -> Vec<i64> {
        use arrow_array::cast::AsArray;
        let mut options = media.record_options().unwrap();
        if let Some(root) = root {
            options = options.with_field(root);
        }
        let options = options.with_filter(filter).unwrap();
        media
            .read_arrow_reader(&options)
            .unwrap()
            .flat_map(|batch| {
                let batch = batch.unwrap();
                let index = batch.schema().index_of("i").unwrap();
                batch
                    .column(index)
                    .as_primitive::<arrow_array::types::Int64Type>()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    #[test]
    fn a_nan_row_is_never_pruned_by_the_extremes_that_leave_it_out() {
        // Parquet writers leave NaN out of a float column's minimum and
        // maximum; this crate orders NaN past every number, by its sign.
        let positive = f64::NAN;
        let negative = f64::from_bits(0xFFF8_0000_0000_0000);
        for (values, filters) in [
            (
                [1.0, positive, 2.0],
                &[
                    "x > 2",
                    "x >= 3",
                    "not (x between 1 and 2)",
                    "x is distinct from 1",
                ][..],
            ),
            ([1.0, negative, 2.0], &["x < 0", "x <= 0.5"][..]),
            ([1.0, positive, 1.0], &["x != 1.0", "not (x = 1.0)"][..]),
        ] {
            let batch = RecordBatch::try_new(
                Arc::new(arrow_schema::Schema::new(vec![
                    arrow_schema::Field::new("x", arrow_schema::DataType::Float64, false),
                    arrow_schema::Field::new("i", arrow_schema::DataType::Int64, false),
                ])),
                vec![
                    Arc::new(arrow_array::Float64Array::from(values.to_vec())) as ArrayRef,
                    Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
                ],
            )
            .unwrap();
            let media = file_of(&batch);
            for filter in filters {
                // The same filter over the rows in memory, with no footer to
                // answer from, is the rows' own answer.
                let expected: Vec<i64> = filter
                    .parse::<yggdryl::Filter>()
                    .unwrap()
                    .apply_arrow_batch(&batch)
                    .unwrap()
                    .column(1)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec();
                assert!(!expected.is_empty(), "{filter} over {values:?}");
                assert_eq!(
                    kept(&media, None, filter),
                    expected,
                    "{filter} over {values:?}"
                );
            }
        }
    }

    #[test]
    fn a_null_row_is_distinct_from_the_one_value_the_extremes_hold() {
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("x", arrow_schema::DataType::Int64, true),
                arrow_schema::Field::new("i", arrow_schema::DataType::Int64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![Some(1), None, Some(1)])) as ArrayRef,
                Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            ],
        )
        .unwrap();
        let media = file_of(&batch);
        assert_eq!(kept(&media, None, "x is distinct from 1"), vec![2]);
        assert_eq!(
            kept(&media, None, "not (x is not distinct from 1)"),
            vec![2]
        );
        assert_eq!(kept(&media, None, "x is not distinct from 1"), vec![1, 3]);
        assert!(kept(&media, None, "x = 2").is_empty());
    }

    #[test]
    fn a_stored_null_read_as_a_default_is_not_pruned_by_the_statistics() {
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("x", arrow_schema::DataType::Int64, true),
                arrow_schema::Field::new("i", arrow_schema::DataType::Int64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![Some(5), None, Some(10)])) as ArrayRef,
                Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            ],
        )
        .unwrap();
        let media = file_of(&batch);
        // Declared non-null, the stored null reads as the default zero.
        let root = StructType::from_fields([
            DataType::Int64.required_field("x"),
            DataType::Int64.required_field("i"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        for filter in ["x = 0", "x < 5", "x is not null"] {
            let unpruned = filter.replacen('x', "(x + 0)", 1);
            assert_eq!(
                kept(&media, Some(root.clone()), filter),
                kept(&media, Some(root.clone()), &unpruned),
                "{filter}"
            );
        }
        assert_eq!(kept(&media, Some(root), "x = 0"), vec![2]);
    }

    #[test]
    fn legacy_extremes_in_an_unsigned_order_prune_nothing() {
        use parquet::data_type::ByteArray;
        use parquet::file::metadata::{ParquetMetaDataReader, ParquetMetaDataWriter};
        use parquet::file::statistics::{Statistics, ValueStatistics};

        // Written by a current writer, then given the footer a writer before
        // parquet-mr 1.10 left: only the deprecated extremes, taken in signed
        // byte order, so 'é' (0xC3...) sorts below 'a'.
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("s", arrow_schema::DataType::Utf8, false),
                arrow_schema::Field::new("i", arrow_schema::DataType::Int64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["a", "é"])) as ArrayRef,
                Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            ],
        )
        .unwrap();
        let bytes = file_of(&batch).handle().read_all_bytes().unwrap();
        let footer_length =
            u32::from_le_bytes(bytes[bytes.len() - 8..bytes.len() - 4].try_into().unwrap());
        let body = bytes.len() - 8 - usize::try_from(footer_length).unwrap();
        let metadata = ParquetMetaDataReader::new()
            .parse_and_finish(&bytes::Bytes::from(bytes.clone()))
            .unwrap();
        let mut builder = metadata.into_builder();
        let groups = builder
            .take_row_groups()
            .into_iter()
            .map(|group| {
                let columns = group
                    .columns()
                    .iter()
                    .map(|column| {
                        let legacy = if column.column_path().string() == "s" {
                            Statistics::ByteArray(ValueStatistics::new(
                                Some(ByteArray::from("é")),
                                Some(ByteArray::from("a")),
                                None,
                                Some(0),
                                true,
                            ))
                        } else {
                            column.statistics().unwrap().clone()
                        };
                        column
                            .clone()
                            .into_builder()
                            .set_statistics(legacy)
                            .build()
                            .unwrap()
                    })
                    .collect();
                group
                    .into_builder()
                    .set_column_metadata(columns)
                    .build()
                    .unwrap()
            })
            .collect();
        let metadata = builder.set_row_groups(groups).build();
        let mut rewritten = bytes[..body].to_vec();
        ParquetMetaDataWriter::new(&mut rewritten, &metadata)
            .finish()
            .unwrap();
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
        media.write_all_bytes(&rewritten).unwrap();

        assert_eq!(kept(&media, None, "s = 'é'"), vec![2]);
        assert_eq!(kept(&media, None, "s > 'b'"), vec![2]);
        assert_eq!(kept(&media, None, "i = 1"), vec![1]);
    }

    #[test]
    fn a_derived_column_prunes_nothing_by_what_is_stored_under_its_name() {
        // Stored all null under `y`; the declared root computes `y` from `x`.
        let batch = RecordBatch::try_new(
            Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
                arrow_schema::Field::new("y", arrow_schema::DataType::Int64, true),
                arrow_schema::Field::new("i", arrow_schema::DataType::Int64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![5, 7])) as ArrayRef,
                Arc::new(Int64Array::from(vec![None, None])) as ArrayRef,
                Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            ],
        )
        .unwrap();
        let media = file_of(&batch);
        let mut derived = DataType::Int64.nullable_field("y");
        derived
            .insert_metadata("TRANSFORM:expression", "x * 2")
            .unwrap();
        let root = StructType::from_fields([
            DataType::Int64.required_field("x"),
            derived,
            DataType::Int64.required_field("i"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        // The unprunable spelling is the rows' own answer.
        assert_eq!(kept(&media, Some(root.clone()), "(y + 0) = 10"), vec![1]);
        assert_eq!(kept(&media, Some(root), "y = 10"), vec![1]);
    }
}
