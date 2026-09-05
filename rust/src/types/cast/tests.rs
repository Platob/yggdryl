//! A typed field casts to its own array type.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, Datum, Float64Array, Int32Array, Int64Array, StringArray,
    StructArray, UInt32Array, UInt64Array,
};

use super::ArrowCast;
use crate::types::{
    DateTime64Field, GeometryField, Int32Field, Int64Field, StructField, UInt32Field, UInt64Field,
    Utf8Field, VariantField,
};
use crate::{DataType, EdgeAlgorithm, Field};
use crate::{TimeUnit, Timezone};

#[test]
fn a_typed_field_returns_its_own_array_type() {
    let field = Int64Field::new("id", false);
    let source: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));

    // The binding is an Int64Array; no downcast at the call site.
    let ids: Int64Array = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(ids.values(), &[1, 2, 3]);
}

#[test]
fn a_string_field_parses_and_formats_through_the_same_call() {
    let field = Utf8Field::new("symbol", false);
    let numbers: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));

    let text: StringArray = field.cast_arrow_array(numbers, false).unwrap();
    assert_eq!(text.value(0), "1.5");
    assert_eq!(text.value(1), "2.5");
}

#[test]
fn an_unsafe_cast_fails_and_a_safe_one_defaults() {
    let field = Int64Field::new("id", false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    assert!(field.cast_arrow_array(Arc::clone(&text), false).is_err());

    // Safe casting nulls the failure, and a non-null field then defaults it.
    let ids = field.cast_arrow_array(text, true).unwrap();
    assert_eq!(ids.values(), &[1, 0]);
    assert_eq!(ids.null_count(), 0);
}

#[test]
fn a_nullable_field_keeps_the_null_a_safe_cast_produced() {
    let field = Int64Field::new("id", true);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    let ids = field.cast_arrow_array(text, true).unwrap();
    assert!(ids.is_null(1));
}

#[test]
fn a_struct_field_casts_children_by_name() {
    let field = StructField::try_from_field(Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap(),
        false,
    ))
    .unwrap();

    let source: ArrayRef = Arc::new(arrow_array::StructArray::from(vec![
        (
            Arc::new(arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int32,
                false,
            )),
            Arc::new(Int32Array::from(vec![7])) as ArrayRef,
        ),
        (
            Arc::new(arrow_schema::Field::new(
                "symbol",
                arrow_schema::DataType::Utf8,
                true,
            )),
            Arc::new(StringArray::from(vec!["ACME"])) as ArrayRef,
        ),
    ]));

    let row = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(row.num_columns(), 2);
    assert_eq!(row.column(0).data_type(), &arrow_schema::DataType::Int64);
}

#[test]
fn a_parameterized_temporal_field_casts_to_a_shared_array() {
    // A unit decides the physical width, so the result stays an ArrayRef.
    let field = DateTime64Field::try_new(
        "at",
        DataType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::NAIVE,
        },
        false,
    )
    .unwrap();
    let source: ArrayRef = Arc::new(Int64Array::from(vec![1_700_000_000_000]));

    let cast: ArrayRef = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(cast.len(), 1);
    assert_eq!(
        cast.data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None)
    );
}

#[test]
fn a_scalar_cast_requires_exactly_one_value() {
    let field = Int64Field::new("id", false);
    let one: ArrayRef = Arc::new(Int32Array::from(vec![9]));
    let two: ArrayRef = Arc::new(Int32Array::from(vec![9, 10]));

    let scalar = field.cast_arrow_scalar(one, false).unwrap();
    let (array, is_scalar) = scalar.get();
    assert!(is_scalar);
    assert_eq!(array.len(), 1);

    let message = field.cast_arrow_scalar(two, false).unwrap_err().to_string();
    assert!(message.contains("exactly 1 value"), "{message}");
}

#[test]
fn a_borrowed_typed_field_casts_the_same_way() {
    let field = Int64Field::new("id", false);
    let borrowed = field.as_typed_ref();
    let source: ArrayRef = Arc::new(Int32Array::from(vec![4]));

    assert_eq!(
        borrowed.cast_arrow_array(source, false).unwrap().values(),
        &[4]
    );
}

#[test]
fn bit_casts_cover_the_full_32_bit_domain_in_both_directions_without_copying() {
    let source = UInt32Array::from(vec![0, 0x7fff_ffff, 0x8000_0000, u32::MAX]);
    let signed = Int32Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(source.clone()))
        .unwrap();
    assert_eq!(signed.values(), &[0, i32::MAX, i32::MIN, -1]);
    assert!(
        signed.values().inner().ptr_eq(source.values().inner()),
        "a bit cast shares the physical value buffer"
    );

    let restored = UInt32Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(signed.clone()))
        .unwrap();
    assert_eq!(restored.values(), source.values());
    assert!(
        restored.values().inner().ptr_eq(source.values().inner()),
        "the reverse cast retains the same physical buffer"
    );

    assert_eq!(
        Int32Field::new("digest", true)
            .cast_arrow_array_bits(Arc::new(UInt32Array::from(Vec::<u32>::new())))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn bit_casts_cover_the_full_64_bit_domain_in_both_directions_without_copying() {
    let source = UInt64Array::from(vec![
        0,
        0x7fff_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        u64::MAX,
    ]);
    let signed = Int64Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(source.clone()))
        .unwrap();
    assert_eq!(signed.values(), &[0, i64::MAX, i64::MIN, -1]);
    assert!(signed.values().inner().ptr_eq(source.values().inner()));

    let restored = UInt64Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(signed))
        .unwrap();
    assert_eq!(restored.values(), source.values());
    assert!(restored.values().inner().ptr_eq(source.values().inner()));
}

#[test]
fn bit_casts_preserve_slices_and_apply_the_target_null_contract() {
    let source = UInt64Array::from(vec![Some(3), Some(u64::MAX), None, Some(5)]).slice(1, 2);
    let nullable = Int64Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(source.clone()))
        .unwrap();
    assert_eq!(nullable.len(), 2);
    assert_eq!(nullable.value(0), -1);
    assert!(nullable.is_null(1));
    assert!(nullable.values().inner().ptr_eq(source.values().inner()));

    let required = Int64Field::new("digest", false)
        .cast_arrow_array_bits(Arc::new(source))
        .unwrap();
    assert_eq!(required.values(), &[-1, 0]);
    assert_eq!(required.null_count(), 0);
}

#[test]
fn generic_and_borrowed_fields_expose_the_same_strict_bit_cast() {
    let generic = Field::new("digest", DataType::Int32, true);
    let cast = generic
        .cast_arrow_array_bits(Arc::new(UInt32Array::from(vec![u32::MAX])))
        .unwrap();
    assert_eq!(
        cast.as_any().downcast_ref::<Int32Array>().unwrap().value(0),
        -1
    );

    let target = UInt64Field::new("digest", true);
    let restored = target
        .as_typed_ref()
        .cast_arrow_array_bits(Arc::new(Int64Array::from(vec![-1])))
        .unwrap();
    assert_eq!(restored.value(0), u64::MAX);

    let wrong_width = Int64Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(UInt32Array::from(vec![1])))
        .unwrap_err()
        .to_string();
    assert!(wrong_width.contains("digest"), "{wrong_width}");
    assert!(wrong_width.contains("uint64"), "{wrong_width}");
    assert!(wrong_width.contains("UInt32"), "{wrong_width}");

    let same_sign = Int32Field::new("digest", true)
        .cast_arrow_array_bits(Arc::new(Int32Array::from(Vec::<i32>::new())))
        .unwrap_err()
        .to_string();
    assert!(same_sign.contains("uint32"), "{same_sign}");
    assert!(same_sign.contains("Int32"), "{same_sign}");

    let unsupported = Field::new("digest", DataType::Float64, true)
        .cast_arrow_array_bits(Arc::new(UInt64Array::from(vec![1])))
        .unwrap_err()
        .to_string();
    assert!(unsupported.contains("digest"), "{unsupported}");
    assert!(unsupported.contains("int32"), "{unsupported}");
    assert!(unsupported.contains("uint64"), "{unsupported}");
    assert!(unsupported.contains("float64"), "{unsupported}");
    assert!(unsupported.contains("UInt64"), "{unsupported}");
}

#[test]
fn ordinary_integer_casting_remains_numeric() {
    let field = Int64Field::new("digest", true);
    let source: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));
    assert!(field.cast_arrow_array(Arc::clone(&source), false).is_err());
    assert_eq!(field.cast_arrow_array_bits(source).unwrap().value(0), -1);
}

/// One little-endian ISO WKB point.
fn wkb_point(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = vec![1u8, 1, 0, 0, 0];
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

fn variant_storage_array(rows: usize) -> ArrayRef {
    let fields = arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("metadata", arrow_schema::DataType::Binary, false),
        arrow_schema::Field::new("value", arrow_schema::DataType::Binary, false),
    ]);
    let empty: Vec<&[u8]> = vec![b""; rows];
    let columns: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(empty.clone())),
        Arc::new(BinaryArray::from(empty)),
    ];
    Arc::new(StructArray::new(fields, columns, None))
}

fn geospatial_batch(dtype: DataType, cells: Vec<Option<Vec<u8>>>) -> arrow_array::RecordBatch {
    let root = Field::new(
        "row",
        DataType::from_fields([Field::new("shape", dtype, true)]).unwrap(),
        false,
    );
    let schema = crate::arrow::arrow_schema_from_field(&root).unwrap();
    let values: Vec<Option<&[u8]>> = cells.iter().map(|cell| cell.as_deref()).collect();
    arrow_array::RecordBatch::try_new(schema, vec![Arc::new(BinaryArray::from(values))]).unwrap()
}

fn cast_shape_to(
    batch: arrow_array::RecordBatch,
    target: Field,
) -> crate::arrow::Result<arrow_array::RecordBatch> {
    let root = Field::new("row", DataType::from_fields([target]).unwrap(), false);
    root.cast_arrow_batch(batch, false)
}

#[test]
fn binary_bytes_entering_a_geometry_field_are_validated_as_wkb() {
    let field = GeometryField::try_new("shape", DataType::geometry(None).unwrap(), true).unwrap();
    let point = wkb_point(1.0, 2.0);
    let source: ArrayRef = Arc::new(BinaryArray::from(vec![Some(point.as_slice()), None]));

    // Valid WKB passes with the same bytes; the untyped cast is the identity.
    let cast = field.cast_arrow_array(Arc::clone(&source), false).unwrap();
    assert_eq!(cast.value(0), point.as_slice());
    let identity = field
        .as_field()
        .cast_arrow_array(Arc::clone(&source), false)
        .unwrap();
    assert!(Arc::ptr_eq(&identity, &source));

    // Truncated bytes are refused naming the field and the row.
    let broken: ArrayRef = Arc::new(BinaryArray::from(vec![Some([1u8, 1, 0].as_slice())]));
    let refused = field
        .cast_arrow_array(broken, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("shape"), "{refused}");
    assert!(refused.contains("row 0"), "{refused}");
    assert!(refused.contains("WKB"), "{refused}");
}

#[test]
fn a_geometry_column_renders_wkt_into_a_utf8_target() {
    let batch = geospatial_batch(
        DataType::geometry(None).unwrap(),
        vec![Some(wkb_point(1.0, 2.0)), None],
    );
    let cast = cast_shape_to(batch, Field::new("shape", DataType::Utf8, true)).unwrap();
    let text = cast
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(text.value(0), "POINT (1 2)");
    assert!(text.is_null(1));
}

#[test]
fn a_geometry_column_stays_lossless_into_a_binary_target() {
    let point = wkb_point(3.0, 4.0);
    let batch = geospatial_batch(DataType::geometry(None).unwrap(), vec![Some(point.clone())]);
    let cast = cast_shape_to(batch, Field::new("shape", DataType::Binary, true)).unwrap();
    let bytes = cast
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(bytes.value(0), point.as_slice());
}

#[test]
fn a_crs_change_between_geospatial_columns_is_refused_naming_both() {
    let batch = geospatial_batch(
        DataType::geometry(None).unwrap(),
        vec![Some(wkb_point(1.0, 2.0))],
    );
    let refused = cast_shape_to(
        batch,
        Field::new(
            "shape",
            DataType::geometry(Some("EPSG:3857")).unwrap(),
            true,
        ),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("OGC:CRS84"), "{refused}");
    assert!(refused.contains("EPSG:3857"), "{refused}");
}

#[test]
fn geometry_and_geography_refuse_each_other_naming_the_edge_change() {
    let batch = geospatial_batch(
        DataType::geometry(None).unwrap(),
        vec![Some(wkb_point(1.0, 2.0))],
    );
    let refused = cast_shape_to(
        batch,
        Field::new("shape", DataType::geography(None, None).unwrap(), true),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("edge"), "{refused}");

    let batch = geospatial_batch(
        DataType::geography(None, Some(EdgeAlgorithm::Spherical)).unwrap(),
        vec![Some(wkb_point(1.0, 2.0))],
    );
    let refused = cast_shape_to(
        batch,
        Field::new("shape", DataType::geometry(None).unwrap(), true),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("edge"), "{refused}");
}

#[test]
fn a_matching_geospatial_pair_casts_as_the_identity() {
    let point = wkb_point(5.0, 6.0);
    let batch = geospatial_batch(DataType::geometry(None).unwrap(), vec![Some(point.clone())]);
    let cast = cast_shape_to(
        batch,
        Field::new("shape", DataType::geometry(None).unwrap(), true),
    )
    .unwrap();
    let bytes = cast
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(bytes.value(0), point.as_slice());
}

#[test]
fn text_into_a_geospatial_target_names_the_absent_wkt_parser() {
    let field = Field::new("shape", DataType::geometry(None).unwrap(), true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["POINT (1 2)"]));
    let refused = field
        .cast_arrow_array(source, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("WKT parser"), "{refused}");
}

#[test]
fn a_variant_casts_only_to_itself_until_the_codec_lands() {
    let field = VariantField::new("payload", true);
    let storage = variant_storage_array(2);

    // The identity works, and the untyped cast returns the same array.
    let cast = field.cast_arrow_array(Arc::clone(&storage), false).unwrap();
    assert_eq!(cast.len(), 2);
    let identity = field
        .as_field()
        .cast_arrow_array(Arc::clone(&storage), false)
        .unwrap();
    assert!(Arc::ptr_eq(&identity, &storage));

    // Anything else refuses by name until the codec lands.
    let numbers: ArrayRef = Arc::new(Int64Array::from(vec![7]));
    let refused = field
        .as_field()
        .cast_arrow_array(numbers, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("Iceberg v3 layer"), "{refused}");
}

#[test]
fn a_variant_column_refuses_to_leave_the_type_until_the_codec_lands() {
    let root = Field::new(
        "row",
        DataType::from_fields([Field::new("payload", DataType::variant(), true)]).unwrap(),
        false,
    );
    let schema = crate::arrow::arrow_schema_from_field(&root).unwrap();
    let batch = arrow_array::RecordBatch::try_new(schema, vec![variant_storage_array(1)]).unwrap();
    let target = Field::new(
        "row",
        DataType::from_fields([Field::new("payload", DataType::Utf8, true)]).unwrap(),
        false,
    );
    let refused = target
        .cast_arrow_batch(batch, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("Iceberg v3 layer"), "{refused}");
}
