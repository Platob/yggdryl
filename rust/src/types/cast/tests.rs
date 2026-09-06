//! A typed field casts to its own array type.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, Datum, Float64Array, Int32Array, Int64Array, StringArray,
    StructArray, UInt32Array, UInt64Array,
};

use super::{ArrowCast, ArrowCastOptions};

/// The reading that carries the bytes rather than the number they spell.
fn bits() -> ArrowCastOptions {
    ArrowCastOptions::new().with_representation(crate::Representation::Bits)
}
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
    let ids: Int64Array = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(ids.values(), &[1, 2, 3]);
}

#[test]
fn a_string_field_parses_and_formats_through_the_same_call() {
    let field = Utf8Field::new("symbol", false);
    let numbers: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));

    let text: StringArray = field
        .cast_arrow_array(numbers, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(text.value(0), "1.5");
    assert_eq!(text.value(1), "2.5");
}

#[test]
fn an_unsafe_cast_fails_and_a_safe_one_defaults() {
    let field = Int64Field::new("id", false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    assert!(
        field
            .cast_arrow_array(Arc::clone(&text), ArrowCastOptions::new().with_safe(false))
            .is_err()
    );

    // Safe casting nulls the failure, and a non-null field then defaults it.
    let ids = field
        .cast_arrow_array(text, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(ids.values(), &[1, 0]);
    assert_eq!(ids.null_count(), 0);
}

#[test]
fn a_nullable_field_keeps_the_null_a_safe_cast_produced() {
    let field = Int64Field::new("id", true);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    let ids = field
        .cast_arrow_array(text, ArrowCastOptions::new())
        .unwrap();
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

    let row = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
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

    let cast: ArrayRef = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
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

    let scalar = field
        .cast_arrow_scalar(one, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let (array, is_scalar) = scalar.get();
    assert!(is_scalar);
    assert_eq!(array.len(), 1);

    let message = field
        .cast_arrow_scalar(two, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(message.contains("exactly 1 value"), "{message}");
}

#[test]
fn a_borrowed_typed_field_casts_the_same_way() {
    let field = Int64Field::new("id", false);
    let borrowed = field.as_typed_ref();
    let source: ArrayRef = Arc::new(Int32Array::from(vec![4]));

    assert_eq!(
        borrowed
            .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
            .unwrap()
            .values(),
        &[4]
    );
}

#[test]
fn bits_cover_the_full_32_bit_domain_in_both_directions_without_copying() {
    let source = UInt32Array::from(vec![0, 0x7fff_ffff, 0x8000_0000, u32::MAX]);
    let signed = Int32Field::new("digest", true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(signed.values(), &[0, i32::MAX, i32::MIN, -1]);
    assert!(
        signed.values().inner().ptr_eq(source.values().inner()),
        "reading the bits shares the physical value buffer"
    );

    let restored = UInt32Field::new("digest", true)
        .cast_arrow_array(Arc::new(signed.clone()), bits())
        .unwrap();
    assert_eq!(restored.values(), source.values());
    assert!(
        restored.values().inner().ptr_eq(source.values().inner()),
        "the reverse reading retains the same physical buffer"
    );

    assert_eq!(
        Int32Field::new("digest", true)
            .cast_arrow_array(Arc::new(UInt32Array::from(Vec::<u32>::new())), bits())
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn bits_cover_the_full_64_bit_domain_in_both_directions_without_copying() {
    let source = UInt64Array::from(vec![
        0,
        0x7fff_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        u64::MAX,
    ]);
    let signed = Int64Field::new("digest", true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(signed.values(), &[0, i64::MAX, i64::MIN, -1]);
    assert!(signed.values().inner().ptr_eq(source.values().inner()));

    let restored = UInt64Field::new("digest", true)
        .cast_arrow_array(Arc::new(signed), bits())
        .unwrap();
    assert_eq!(restored.values(), source.values());
    assert!(restored.values().inner().ptr_eq(source.values().inner()));
}

#[test]
fn eight_bytes_read_as_an_integer_a_float_or_bytes_alike() {
    use arrow_array::{FixedSizeBinaryArray, Float64Array};

    let source: ArrayRef = Arc::new(UInt64Array::from(vec![0, u64::MAX]));

    // The whole point of naming a width: an integer, its opposite sign, a
    // float and raw bytes are one buffer under four readings.
    let bytes = Field::new("digest", DataType::FixedSizeBinary(8), true)
        .cast_arrow_array(Arc::clone(&source), bits())
        .unwrap();
    let stored: &FixedSizeBinaryArray = bytes.as_any().downcast_ref().unwrap();
    assert_eq!(stored.value(1), &[0xff; 8]);

    let floats = Field::new("digest", DataType::Float64, true)
        .cast_arrow_array(Arc::clone(&bytes), bits())
        .unwrap();
    let floats: &Float64Array = floats.as_any().downcast_ref().unwrap();
    assert!(floats.value(1).is_nan(), "{:?}", floats.value(1));

    // Round-tripping the whole chain restores the exact bit pattern.
    let restored = UInt64Field::new("digest", true)
        .cast_arrow_array(bytes, bits())
        .unwrap();
    assert_eq!(restored.values(), &[0, u64::MAX]);
    assert!(
        restored
            .values()
            .inner()
            .ptr_eq(&source.to_data().buffers()[0]),
        "the bytes never left the buffer they arrived in"
    );
}

#[test]
fn bits_preserve_slices_and_apply_the_target_null_contract() {
    let source = UInt64Array::from(vec![Some(3), Some(u64::MAX), None, Some(5)]).slice(1, 2);
    let nullable = Int64Field::new("digest", true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(nullable.len(), 2);
    assert_eq!(nullable.value(0), -1);
    assert!(nullable.is_null(1));
    assert!(nullable.values().inner().ptr_eq(source.values().inner()));

    // The reading says what the bytes mean; the nullability policy still says
    // what an absent value means.
    let required = Int64Field::new("digest", false)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(required.values(), &[-1, 0]);
    assert_eq!(required.null_count(), 0);

    let refused = Int64Field::new("digest", false)
        .cast_arrow_array(
            Arc::new(source),
            bits().with_nullability(crate::Nullability::Strict),
        )
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "required Arrow field $.digest holds 1 null values");
}

#[test]
fn a_pair_that_is_not_the_same_bytes_converts_as_it_always_did() {
    // Asking for bits is a preference, not a mode: two widths that are not one
    // buffer take the ordinary numeric conversion, and its range check with it.
    let widened = Int64Field::new("id", true)
        .cast_arrow_array(Arc::new(Int32Array::from(vec![7])), bits())
        .unwrap();
    assert_eq!(widened.values(), &[7]);

    let text = Field::new("id", DataType::Utf8, true)
        .cast_arrow_array(Arc::new(Int64Array::from(vec![7])), bits())
        .unwrap();
    assert_eq!(text.data_type(), &arrow_schema::DataType::Utf8);

    // A datatype whose values follow a rule keeps that rule: four bytes are
    // not an ASCII code merely because they are four bytes.
    let refused = Field::new("ccy", DataType::FixedAscii(4), true)
        .cast_arrow_array(
            Arc::new(
                arrow_array::FixedSizeBinaryArray::try_from_iter([[0xff_u8; 4]].into_iter())
                    .unwrap(),
            ),
            bits(),
        )
        .unwrap_err()
        .to_string();
    assert!(refused.contains("ccy"), "{refused}");
}

#[test]
fn ordinary_integer_casting_remains_numeric() {
    let field = Int64Field::new("digest", true);
    let source: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));
    assert!(
        field
            .cast_arrow_array(
                Arc::clone(&source),
                ArrowCastOptions::new().with_safe(false)
            )
            .is_err()
    );
    assert_eq!(field.cast_arrow_array(source, bits()).unwrap().value(0), -1);
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
    root.cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
}

#[test]
fn binary_bytes_entering_a_geometry_field_are_validated_as_wkb() {
    let field = GeometryField::try_new("shape", DataType::geometry(None).unwrap(), true).unwrap();
    let point = wkb_point(1.0, 2.0);
    let source: ArrayRef = Arc::new(BinaryArray::from(vec![Some(point.as_slice()), None]));

    // Valid WKB passes with the same bytes; the untyped cast is the identity.
    let cast = field
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert_eq!(cast.value(0), point.as_slice());
    let identity = field
        .as_field()
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&identity, &source));

    // Truncated bytes are refused naming the field and the row.
    let broken: ArrayRef = Arc::new(BinaryArray::from(vec![Some([1u8, 1, 0].as_slice())]));
    let refused = field
        .cast_arrow_array(broken, ArrowCastOptions::new().with_safe(false))
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
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("WKT parser"), "{refused}");
}

#[test]
fn a_variant_casts_only_to_itself_until_the_codec_lands() {
    let field = VariantField::new("payload", true);
    let storage = variant_storage_array(2);

    // The identity works, and the untyped cast returns the same array.
    let cast = field
        .cast_arrow_array(
            Arc::clone(&storage),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert_eq!(cast.len(), 2);
    let identity = field
        .as_field()
        .cast_arrow_array(
            Arc::clone(&storage),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&identity, &storage));

    // Anything else refuses by name until the codec lands.
    let numbers: ArrayRef = Arc::new(Int64Array::from(vec![7]));
    let refused = field
        .as_field()
        .cast_arrow_array(numbers, ArrowCastOptions::new().with_safe(false))
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
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("Iceberg v3 layer"), "{refused}");
}

/// Every wrapper reads what the value inside it reads: a list layout is a
/// layout, an encoding is a layout, and a byte framing is a framing.
mod layouts {
    use std::sync::Arc;

    use arrow_array::{
        ArrayRef, BinaryArray, FixedSizeBinaryArray, FixedSizeListArray, Int32Array,
        LargeListArray, ListArray, StringArray, StructArray,
    };
    use arrow_buffer::OffsetBuffer;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields as ArrowFields};

    use super::{ArrowCast, ArrowCastOptions};
    use crate::DataType;

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    fn strict() -> ArrowCastOptions {
        ArrowCastOptions::new().with_safe(false)
    }

    /// Two rows of two `{a: int32}` values, under every list layout in turn.
    fn struct_items() -> (Arc<ArrowField>, ArrayRef) {
        let fields: ArrowFields =
            vec![Arc::new(ArrowField::new("a", ArrowDataType::Int32, true))].into();
        let values: ArrayRef = Arc::new(StructArray::new(
            fields.clone(),
            vec![Arc::new(Int32Array::from(vec![1, 2, 3, 4])) as ArrayRef],
            None,
        ));
        let item = Arc::new(ArrowField::new("item", ArrowDataType::Struct(fields), true));
        (item, values)
    }

    #[test]
    fn every_list_layout_reads_every_other_one_through_a_struct_child() {
        let (item, values) = struct_items();
        let offsets = OffsetBuffer::new(vec![0, 2, 4].into());
        let sources: Vec<ArrayRef> = vec![
            Arc::new(
                ListArray::try_new(
                    Arc::clone(&item),
                    offsets.clone(),
                    Arc::clone(&values),
                    None,
                )
                .unwrap(),
            ),
            Arc::new(
                LargeListArray::try_new(
                    Arc::clone(&item),
                    OffsetBuffer::new(vec![0_i64, 2, 4].into()),
                    Arc::clone(&values),
                    None,
                )
                .unwrap(),
            ),
            Arc::new(
                FixedSizeListArray::try_new(Arc::clone(&item), 2, Arc::clone(&values), None)
                    .unwrap(),
            ),
        ];
        let targets = [
            "list<struct<a: int32>>",
            "large_list<struct<a: int32>>",
            "list_view<struct<a: int32>>",
            "large_list_view<struct<a: int32>>",
            "fixed_size_list<struct<a: int32>, 2>",
        ];
        for source in sources {
            for target in targets {
                let cast = dtype(target)
                    .cast_arrow_array(Arc::clone(&source), strict())
                    .unwrap_or_else(|error| {
                        panic!("{:?} -> {target}: {error}", source.data_type())
                    });
                assert_eq!(cast.len(), 2, "{target}");
            }
        }
    }

    #[test]
    fn two_fixed_sizes_are_a_row_change_and_say_so() {
        let (item, values) = struct_items();
        let source: ArrayRef =
            Arc::new(FixedSizeListArray::try_new(item, 2, values, None).unwrap());

        let refused = dtype("fixed_size_list<struct<a: int32>, 4>")
            .cast_arrow_array(source, strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("value change"), "{refused}");
    }

    #[test]
    fn an_encoded_target_runs_the_value_rule_its_leaf_carries() {
        let text: ArrayRef = Arc::new(StringArray::from(vec!["\u{e9}"]));
        for target in ["dictionary<int32, ascii>", "run_end_encoded<int32, ascii>"] {
            let refused = dtype(target)
                .cast_arrow_array(Arc::clone(&text), strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("non-ASCII byte"), "{target}: {refused}");
        }

        let wkb: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"nope"]));
        let refused = dtype("dictionary<int32, geometry>")
            .cast_arrow_array(wkb, strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("WKB"), "{refused}");
    }

    #[test]
    fn an_encoded_target_reads_what_its_bare_leaf_reads() {
        let codes: ArrayRef = Arc::new(StringArray::from(vec!["US", "US", "FR"]));
        for target in [
            "dictionary<int32, country>",
            "run_end_encoded<int32, country>",
            "dictionary<int32, uuid>",
        ] {
            let source: ArrayRef = if target.contains("uuid") {
                Arc::new(StringArray::from(vec![
                    "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
                ]))
            } else {
                Arc::clone(&codes)
            };
            let cast = dtype(target)
                .cast_arrow_array(source, strict())
                .unwrap_or_else(|error| panic!("{target}: {error}"));
            // The declared child field keeps the extension identity Arrow's
            // own encoding does not copy.
            assert_eq!(
                &cast.data_type().clone(),
                dtype(target)
                    .required_field("value")
                    .into_arrow_ref()
                    .unwrap()
                    .data_type(),
                "{target}"
            );
        }
    }

    #[test]
    fn an_encoded_struct_source_is_decoded_and_reconciled_by_name() {
        use arrow_array::{DictionaryArray, Int32Array, RunArray, types::Int32Type};

        let fields: ArrowFields =
            vec![Arc::new(ArrowField::new("key", ArrowDataType::Utf8, true))].into();
        let values: ArrayRef = Arc::new(StructArray::new(
            fields,
            vec![Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef],
            None,
        ));
        let dictionary: ArrayRef = Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                Int32Array::from(vec![0, 1, 0]),
                Arc::clone(&values),
            )
            .unwrap(),
        );
        let run: ArrayRef = Arc::new(
            RunArray::<Int32Type>::try_new(&Int32Array::from(vec![1, 2]), values.as_ref()).unwrap(),
        );

        for source in [dictionary, run] {
            // The decode happens first, so the Struct child the encoding was
            // hiding is reconciled by name rather than positionally.
            let cast = dtype("struct<KEY: utf8>")
                .cast_arrow_array(Arc::clone(&source), strict())
                .unwrap_or_else(|error| panic!("{:?}: {error}", source.data_type()));
            let ArrowDataType::Struct(cast_fields) = cast.data_type() else {
                panic!("a struct target answers a struct");
            };
            assert_eq!(cast_fields[0].name(), "KEY");
        }
    }

    #[test]
    fn a_byte_framing_reaches_every_other_one_through_binary() {
        let text: ArrayRef = Arc::new(StringArray::from(vec!["abc"]));
        let fixed = dtype("fixed_size_binary(3)")
            .cast_arrow_array(Arc::clone(&text), strict())
            .unwrap();
        assert_eq!(
            crate::types::cast::downcast::<FixedSizeBinaryArray>(fixed.as_ref())
                .unwrap()
                .value(0),
            b"abc"
        );

        let back = DataType::Utf8.cast_arrow_array(fixed, strict()).unwrap();
        assert_eq!(
            crate::types::cast::downcast::<StringArray>(back.as_ref())
                .unwrap()
                .value(0),
            "abc"
        );
    }
}
