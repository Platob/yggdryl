//! The Parquet logical types an integration test cannot reach.
//!
//! `open_builder` is the file-private reader builder every Parquet read opens
//! through; the logical type a column publishes is only visible on it.
//! Everything a caller can observe lives in `tests/media/parquet.rs`.

use crate::Url;
use crate::holder::Buffer;

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

use super::Parquet;
use crate::ArrowCastOptions;
use crate::FieldValue as _;
use crate::{DataType, Field};
use crate::{IOBase, IOMedia};

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
    let mut metadata = HashMap::from([("ARROW:extension:name".to_owned(), extension.to_owned())]);
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
fn variant_column(values: &[crate::Scalar]) -> ArrayRef {
    let variants: Vec<crate::Variant> = values
        .iter()
        .map(|value| crate::Variant::encode(value).unwrap())
        .collect();
    Arc::new(arrow_array::StructArray::new(
        arrow_schema::Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::Binary, false),
            ArrowField::new("value", ArrowDataType::Binary, false),
        ]),
        vec![
            Arc::new(arrow_array::BinaryArray::from_iter_values(
                variants.iter().map(crate::Variant::metadata),
            )) as ArrayRef,
            Arc::new(arrow_array::BinaryArray::from_iter_values(
                variants.iter().map(crate::Variant::value),
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
        .overwrite_arrow_reader(crate::arrow::batch_reader(schema, batches), &options)
        .unwrap();
    media
}

/// The logical type the footer stores for one leaf column path.
fn leaf_logical(media: &Parquet<Buffer>, path: &str) -> Option<LogicalType> {
    let builder = super::open_builder(media.handle()).unwrap();
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
        crate::Scalar::from(7_i64),
        crate::Scalar::from_struct([("symbol", crate::Scalar::from("AAPL"))]).unwrap(),
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
        let builder = super::open_builder(media.handle()).unwrap();
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
    let imported = crate::Field::from_arrow_field(field).unwrap();
    assert_eq!(imported.dtype(), &crate::DataType::Variant);

    let options = media.record_options().unwrap();
    let batch = media
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let read: Vec<crate::Scalar> = (0..batch.num_rows())
        .map(|row| {
            let held = crate::arrow::value::value_from_array(
                &crate::DataType::Variant,
                batch.column(0).as_ref(),
                row,
            )
            .unwrap();
            let crate::Scalar::Variant(variant) = held else {
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
    let storage = ArrowDataType::Struct(vec![child("metadata", "12"), child("value", "13")].into());
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
    let descriptor = super::geospatial::extension_schema(&schema)
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
    assert!(super::geospatial::variant_schema(&descriptor, &foreign).is_none());
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
    let values = [crate::Scalar::from(7_i64)];
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![variant_column(&values)]).unwrap();
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
    let read = super::read_arrow_schema(&handle).unwrap();
    let field = read.field_with_name("payload").unwrap();
    assert_eq!(
        field.metadata().get("ARROW:extension:name"),
        Some(&"arrow.parquet.variant".to_owned()),
        "the annotation names the column a variant"
    );
    assert_eq!(
        crate::Field::from_arrow_field(field).unwrap().dtype(),
        &crate::DataType::Variant
    );

    // And the batches carry it too, so a row reads as the value it holds.
    let batch = super::read_batch_reader(&handle, None, &super::ParquetOptions::new())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let held = crate::arrow::value::value_from_array(
        &crate::DataType::Variant,
        batch.column(0).as_ref(),
        0,
    )
    .unwrap();
    let crate::Scalar::Variant(variant) = held else {
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
    let descriptor = super::geospatial::extension_schema(&declared)
        .unwrap()
        .expect("the declared nested variants need annotations");
    // A foreign Arrow footer supplies only physical storage; Parquet's
    // annotation must restore the extension beneath both wrappers.
    let foreign = Schema::new(vec![
        ArrowField::new("items", list(false), true),
        ArrowField::new("attributes", map(false), false),
    ]);
    let recovered = super::geospatial::variant_schema(&descriptor, &foreign)
        .expect("the annotations restore nested variants");

    let ArrowDataType::List(item) = recovered.field_with_name("items").unwrap().data_type() else {
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
    let declared = crate::Field::new("ccy", crate::DataType::fixed_ascii(4).unwrap(), true);
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
    let field = crate::Field::from_arrow_field(schema.field_with_name("ccy").unwrap()).unwrap();
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
    let text = crate::StructType::from_fields([crate::DataType::utf8().nullable_field("ccy")])
        .map(crate::DataType::from)
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
        .overwrite_arrow_reader(crate::arrow::batch_reader(schema, []), &options)
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
    let root = crate::StructType::from_fields([
        crate::DataType::Int64.required_field("id"),
        crate::DataType::geometry(Some("EPSG:3857"))
            .unwrap()
            .nullable_field("shape"),
        crate::DataType::geography(None, Some(crate::EdgeAlgorithm::Vincenty))
            .unwrap()
            .nullable_field("route"),
        crate::DataType::variant().nullable_field("payload"),
    ])
    .map(crate::DataType::from)
    .unwrap()
    .required_field("row");
    let schema = root.clone().into_arrow_schema().unwrap();

    let mut media = Parquet::new(handle("field-declared.parquet"));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(crate::arrow::batch_reader(schema, []), &options)
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
    let builder = super::open_builder(media.handle()).unwrap();
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
        crate::arrow::field_from_arrow_schema("row", media.read_arrow_schema().unwrap().as_ref())
            .unwrap();
    assert_eq!(read, root);
}
