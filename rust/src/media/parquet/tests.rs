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
use crate::types::FieldValue as _;
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
fn the_variant_storage_struct_publishes_the_variant_logical_type() {
    // Schema level only: a variant *value* cannot cross an Arrow array
    // boundary yet - the binary encoding lands with the Iceberg v3 layer.
    let media = written(
        "variant.parquet",
        vec![extension_field(
            "payload",
            variant_storage(),
            "arrow.parquet.variant",
            Some(""),
        )],
        Vec::new(),
    );

    let builder = super::open_builder(media.handle()).unwrap();
    let root = builder.parquet_schema().root_schema_ptr();
    let payload = root
        .get_fields()
        .iter()
        .find(|field| field.name() == "payload")
        .unwrap();
    assert_eq!(
        payload.get_basic_info().logical_type_ref(),
        Some(&LogicalType::variant(None))
    );

    // The storage struct itself round-trips as a plain struct.
    let schema = media.read_arrow_schema().unwrap();
    let field = schema.field_with_name("payload").unwrap();
    assert_eq!(
        field.metadata().get("ARROW:extension:name"),
        Some(&"arrow.parquet.variant".to_owned())
    );
    let ArrowDataType::Struct(children) = field.data_type() else {
        panic!("expected struct storage, got {}", field.data_type());
    };
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name(), "metadata");
    assert_eq!(children[1].name(), "value");
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
    let text = crate::DataType::from_fields([crate::DataType::utf8().nullable_field("ccy")])
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
    // hand-spelled metadata, so the two layers are proven to agree; rows
    // stay out because a variant value cannot cross an Arrow array yet.
    let root = crate::DataType::from_fields([
        crate::DataType::Int64.required_field("id"),
        crate::DataType::geometry(Some("EPSG:3857"))
            .unwrap()
            .nullable_field("shape"),
        crate::DataType::geography(None, Some(crate::EdgeAlgorithm::Vincenty))
            .unwrap()
            .nullable_field("route"),
        crate::DataType::variant().nullable_field("payload"),
    ])
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
    assert_eq!(
        payload.get_basic_info().logical_type_ref(),
        Some(&LogicalType::variant(None))
    );

    // And the identity survives the read: the reimported root speaks the
    // datatypes the declaration did, extension transport keys stripped.
    let read =
        crate::arrow::field_from_arrow_schema("row", media.read_arrow_schema().unwrap().as_ref())
            .unwrap();
    assert_eq!(read, root);
}
