//! A typed field casts to its own array type.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, Datum, Float64Array, Int32Array, Int64Array, StringArray,
    StructArray, UInt32Array, UInt64Array,
};

use yggdryl::cast::ArrowCastOptions;

/// The reading that carries the bytes rather than the number they spell.
fn bits() -> ArrowCastOptions {
    ArrowCastOptions::new().with_representation(yggdryl::Representation::Bits)
}
use yggdryl::FieldValue as _;
use yggdryl::{DataType, EdgeAlgorithm, Field};
use yggdryl::{
    DateTimeField, DateTimeType, GeometryField, Int32Field, Int64Field, StringField,
    StructureField, UInt32Field, UInt64Field, VariantField,
};
use yggdryl::{TimeUnit, Timezone};

#[test]
fn a_typed_field_returns_its_own_array_type() {
    let field = Int64Field::new("id", yggdryl::Int64Type, false);
    let source: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));

    // The binding is an Int64Array; no downcast at the call site.
    let ids: Int64Array = field
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(ids.values(), &[1, 2, 3]);
}

#[test]
fn a_string_field_parses_and_formats_through_the_same_call() {
    let field = StringField::try_new("symbol", DataType::utf8(), false).unwrap();
    let numbers: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));

    // A string's layout and charset decide its array, so the binding is the
    // storage the field projects rather than one concrete array type.
    let cast = field
        .cast_arrow_array(numbers, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let text = cast.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(text.value(0), "1.5");
    assert_eq!(text.value(1), "2.5");
}

#[test]
fn an_unsafe_cast_fails_and_a_safe_one_defaults() {
    let field = Int64Field::new("id", yggdryl::Int64Type, false);
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
    let field = Int64Field::new("id", yggdryl::Int64Type, true);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    let ids = field
        .cast_arrow_array(text, ArrowCastOptions::new())
        .unwrap();
    assert!(ids.is_null(1));
}

#[test]
fn a_struct_field_casts_children_by_name() {
    let field = StructureField::try_from_field(Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
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
    let field = DateTimeField::try_new(
        "at",
        DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::NAIVE,
        }),
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
    let field = Int64Field::new("id", yggdryl::Int64Type, false);
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
    let field = Int64Field::new("id", yggdryl::Int64Type, false);
    let borrowed = &field;
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
    let signed = Int32Field::new("digest", yggdryl::Int32Type, true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(signed.values(), &[0, i32::MAX, i32::MIN, -1]);
    assert!(
        signed.values().inner().ptr_eq(source.values().inner()),
        "reading the bits shares the physical value buffer"
    );

    let restored = UInt32Field::new("digest", yggdryl::UInt32Type, true)
        .cast_arrow_array(Arc::new(signed.clone()), bits())
        .unwrap();
    assert_eq!(restored.values(), source.values());
    assert!(
        restored.values().inner().ptr_eq(source.values().inner()),
        "the reverse reading retains the same physical buffer"
    );

    assert_eq!(
        Int32Field::new("digest", yggdryl::Int32Type, true)
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
    let signed = Int64Field::new("digest", yggdryl::Int64Type, true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(signed.values(), &[0, i64::MAX, i64::MIN, -1]);
    assert!(signed.values().inner().ptr_eq(source.values().inner()));

    let restored = UInt64Field::new("digest", yggdryl::UInt64Type, true)
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
    let bytes = Field::new("digest", DataType::fixed_binary(8).unwrap(), true)
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
    let restored = UInt64Field::new("digest", yggdryl::UInt64Type, true)
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
    let nullable = Int64Field::new("digest", yggdryl::Int64Type, true)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(nullable.len(), 2);
    assert_eq!(nullable.value(0), -1);
    assert!(nullable.is_null(1));
    assert!(nullable.values().inner().ptr_eq(source.values().inner()));

    // The reading says what the bytes mean; the nullability policy still says
    // what an absent value means.
    let required = Int64Field::new("digest", yggdryl::Int64Type, false)
        .cast_arrow_array(Arc::new(source.clone()), bits())
        .unwrap();
    assert_eq!(required.values(), &[-1, 0]);
    assert_eq!(required.null_count(), 0);

    let refused = Int64Field::new("digest", yggdryl::Int64Type, false)
        .cast_arrow_array(
            Arc::new(source),
            bits().with_nullability(yggdryl::Nullability::Strict),
        )
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "required Arrow field $.digest holds 1 null values");
}

#[test]
fn a_pair_that_is_not_the_same_bytes_converts_as_it_always_did() {
    // Asking for bits is a preference, not a mode: two widths that are not one
    // buffer take the ordinary numeric conversion, and its range check with it.
    let widened = Int64Field::new("id", yggdryl::Int64Type, true)
        .cast_arrow_array(Arc::new(Int32Array::from(vec![7])), bits())
        .unwrap();
    assert_eq!(widened.values(), &[7]);

    let text = Field::new("id", DataType::utf8(), true)
        .cast_arrow_array(Arc::new(Int64Array::from(vec![7])), bits())
        .unwrap();
    assert_eq!(text.data_type(), &arrow_schema::DataType::Utf8);

    // A datatype whose values follow a rule keeps that rule: four bytes are
    // not US-ASCII text merely because they are four bytes.
    let refused = Field::new("ccy", DataType::fixed_ascii(4).unwrap(), true)
        .cast_arrow_array(
            Arc::new(
                arrow_array::FixedSizeBinaryArray::try_from_iter([[0xff_u8; 4]].into_iter())
                    .unwrap(),
            ),
            bits().with_safe(false),
        )
        .unwrap_err()
        .to_string();
    assert!(refused.contains("ccy"), "{refused}");
}

#[test]
fn ordinary_integer_casting_remains_numeric() {
    let field = Int64Field::new("digest", yggdryl::Int64Type, true);
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
    let schema = root.clone().into_arrow_schema().unwrap();
    let values: Vec<Option<&[u8]>> = cells.iter().map(|cell| cell.as_deref()).collect();
    arrow_array::RecordBatch::try_new(schema, vec![Arc::new(BinaryArray::from(values))]).unwrap()
}

fn cast_shape_to(
    batch: arrow_array::RecordBatch,
    target: Field,
) -> yggdryl::arrow::Result<arrow_array::RecordBatch> {
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
        .to_field()
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
    let cast = cast_shape_to(batch, Field::new("shape", DataType::utf8(), true)).unwrap();
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
    let cast = cast_shape_to(batch, Field::new("shape", DataType::binary(), true)).unwrap();
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
    let field = VariantField::new("payload", yggdryl::VariantType, true);
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
        .to_field()
        .cast_arrow_array(
            Arc::clone(&storage),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&identity, &storage));

    // Anything else refuses by name until the codec lands.
    let numbers: ArrayRef = Arc::new(Int64Array::from(vec![7]));
    let refused = field
        .to_field()
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
    let schema = root.clone().into_arrow_schema().unwrap();
    let batch = arrow_array::RecordBatch::try_new(schema, vec![variant_storage_array(1)]).unwrap();
    let target = Field::new(
        "row",
        DataType::from_fields([Field::new("payload", DataType::utf8(), true)]).unwrap(),
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

    use yggdryl::DataType;
    use yggdryl::DataTypeValue as _;
    use yggdryl::cast::ArrowCastOptions;

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
                    .into_arrow_field_ref()
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
        let fixed = dtype("fixed_binary(3)")
            .cast_arrow_array(Arc::clone(&text), strict())
            .unwrap();
        assert_eq!(
            fixed
                .as_any()
                .downcast_ref::<FixedSizeBinaryArray>()
                .ok_or("downcast")
                .unwrap()
                .value(0),
            b"abc"
        );

        let back = DataType::utf8().cast_arrow_array(fixed, strict()).unwrap();
        assert_eq!(
            back.as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "abc"
        );
    }
}

/// A string reads values under what the source declares and writes them
/// under what the target declares: the layout, the charset and the bound.
mod strings {

    /// Narrow an Arrow array the way any caller does, so the fixture does
    /// not borrow the crate's own internal narrowing.
    fn downcast<T: Array + 'static>(array: &dyn Array) -> Option<&T> {
        array.as_any().downcast_ref::<T>()
    }
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};

    use yggdryl::cast::ArrowCastOptions;
    use yggdryl::{DataType, Field};
    use yggdryl::{DataTypeValue as _, FieldValue as _};

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    fn strict() -> ArrowCastOptions {
        ArrowCastOptions::new().with_safe(false)
    }

    /// One column under a declared field, so its extension identity rides
    /// into the cast.
    fn batch(field: Field, column: ArrayRef) -> arrow_array::RecordBatch {
        let root = Field::new("row", DataType::from_fields([field]).unwrap(), false);
        let schema = root.clone().into_arrow_schema().unwrap();
        arrow_array::RecordBatch::try_new(schema, vec![column]).unwrap()
    }

    fn cast_column(
        source: arrow_array::RecordBatch,
        target: DataType,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<ArrayRef> {
        let root = Field::new(
            "row",
            DataType::from_fields([Field::new("text", target, true)]).unwrap(),
            false,
        );
        Ok(Arc::clone(
            root.cast_arrow_batch(source, options)?.column(0),
        ))
    }

    #[test]
    fn a_bound_is_checked_on_the_way_in_and_a_failing_cell_is_null_when_safe() {
        let text: ArrayRef = Arc::new(StringArray::from(vec![Some("abc"), Some("abcdef"), None]));
        let refused = dtype("utf8(4)")
            .cast_arrow_array(Arc::clone(&text), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("row 1"), "{refused}");
        assert!(refused.contains("at most 4 bytes"), "{refused}");

        // A nullable field keeps the null a failing cell became.
        let lenient = Field::new("text", dtype("utf8(4)"), true)
            .cast_arrow_array(text, ArrowCastOptions::new())
            .unwrap();
        let lenient = lenient
            .as_ref()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(lenient.value(0), "abc");
        assert!(lenient.is_null(1));
        assert!(lenient.is_null(2));
    }

    #[test]
    fn a_code_answers_safe_and_strict_exactly_as_a_string_does() {
        let text: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EURO"), None]));
        let refused = DataType::Currency
            .cast_arrow_array(Arc::clone(&text), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("row 1"), "{refused}");
        assert!(refused.contains("at most 3 bytes"), "{refused}");

        let lenient = Field::new("ccy", DataType::Currency, true)
            .cast_arrow_array(text, ArrowCastOptions::new())
            .unwrap();
        let lenient = lenient
            .as_ref()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(lenient.value(0), "USD");
        assert!(lenient.is_null(1));
        assert!(lenient.is_null(2));

        // A text source every cell of which passes is the code's own
        // storage, so it is shared rather than copied.
        let passing: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR"]));
        let shared = DataType::Currency
            .cast_arrow_array(Arc::clone(&passing), strict())
            .unwrap();
        assert!(Arc::ptr_eq(&shared, &passing));

        // A fixed binary source is trimmed of the padding its slot wrote and
        // stored as the text it spells.
        let stored: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                [Some(b"USD".as_slice()), Some(b"EU\xff".as_slice())].into_iter(),
                3,
            )
            .unwrap(),
        );
        assert!(
            DataType::Currency
                .cast_arrow_array(Arc::clone(&stored), strict())
                .is_err()
        );
        let lenient = Field::new("ccy", DataType::Currency, true)
            .cast_arrow_array(stored, ArrowCastOptions::new())
            .unwrap();
        let lenient = lenient
            .as_ref()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(lenient.value(0), "USD");
        assert!(lenient.is_null(1));
    }

    #[test]
    fn a_charset_is_written_from_text_and_read_back_from_its_own_bytes() {
        let text: ArrayRef = Arc::new(StringArray::from(vec!["caf\u{e9}"]));
        let latin = batch(
            Field::new("text", dtype("string(windows-1252)"), true),
            dtype("string(windows-1252)")
                .cast_arrow_array(text, strict())
                .unwrap(),
        );
        let bytes = downcast::<BinaryArray>(latin.column(0).as_ref()).unwrap();
        assert_eq!(bytes.value(0), b"caf\xe9");

        // The recognized source is read under its own charset, so the
        // characters come back rather than the bytes.
        let back = cast_column(latin, DataType::utf8(), strict()).unwrap();
        assert_eq!(
            back.as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "caf\u{e9}"
        );
    }

    #[test]
    fn a_scalar_the_target_charset_cannot_spell_fails_at_the_write() {
        // U+0101 has no windows-1252 byte, and the write seam is where that
        // is refused, naming the row.
        let text: ArrayRef = Arc::new(StringArray::from(vec!["\u{0101}"]));
        let refused = dtype("cp1252")
            .cast_arrow_array(text, strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("row 0"), "{refused}");
    }

    #[test]
    fn a_fixed_width_pads_on_the_way_in_and_trims_on_the_way_out() {
        let text: ArrayRef = Arc::new(StringArray::from(vec!["ab"]));
        let fixed = dtype("fixed_ascii(4)")
            .cast_arrow_array(text, strict())
            .unwrap();
        assert_eq!(
            downcast::<FixedSizeBinaryArray>(fixed.as_ref())
                .unwrap()
                .value(0),
            b"ab\0\0"
        );

        let source = batch(Field::new("text", dtype("fixed_ascii(4)"), true), fixed);
        let back = cast_column(source, dtype("ascii"), strict()).unwrap();
        assert_eq!(
            back.as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "ab"
        );
    }

    #[test]
    fn a_code_reads_into_a_string_and_bare_bytes_are_taken_as_the_target_charset() {
        let codes: ArrayRef = Arc::new(StringArray::from(vec!["USD"]));
        let currency = DataType::Currency
            .cast_arrow_array(codes, strict())
            .unwrap();
        let source = batch(Field::new("text", DataType::Currency, true), currency);
        let back = cast_column(source, dtype("utf8(8)"), strict()).unwrap();
        assert_eq!(
            back.as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "USD"
        );

        let bytes: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"caf\xe9"]));
        let latin = dtype("string(windows-1252)")
            .cast_arrow_array(bytes, strict())
            .unwrap();
        assert_eq!(
            latin
                .as_ref()
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap()
                .value(0),
            b"caf\xe9"
        );
    }
}

/// A byte column reads its cells only where it declares a maximum, which is
/// the one thing about bytes Arrow has nowhere to state.
mod bytes {

    /// Narrow an Arrow array the way any caller does, so the fixture does
    /// not borrow the crate's own internal narrowing.
    fn downcast<T: Array + 'static>(array: &dyn Array) -> Option<&T> {
        array.as_any().downcast_ref::<T>()
    }
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, LargeBinaryArray, StringArray};

    use yggdryl::cast::ArrowCastOptions;
    use yggdryl::{DataType, Field};
    use yggdryl::{DataTypeValue as _, FieldValue as _};

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    fn strict() -> ArrowCastOptions {
        ArrowCastOptions::new().with_safe(false)
    }

    /// One column under a declared field, so its extension identity rides
    /// into the cast.
    fn batch(field: Field, column: ArrayRef) -> arrow_array::RecordBatch {
        let root = Field::new("row", DataType::from_fields([field]).unwrap(), false);
        let schema = root.clone().into_arrow_schema().unwrap();
        arrow_array::RecordBatch::try_new(schema, vec![column]).unwrap()
    }

    #[test]
    fn a_maximum_is_checked_on_the_way_in_and_a_failing_cell_is_null_when_safe() {
        let cells: ArrayRef = Arc::new(BinaryArray::from(vec![
            Some(b"abc".as_slice()),
            Some(b"abcdef".as_slice()),
            None,
        ]));
        let refused = dtype("binary(4)")
            .cast_arrow_array(Arc::clone(&cells), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("row 1"), "{refused}");
        assert!(refused.contains("at most 4 bytes"), "{refused}");

        // A nullable field keeps the null a failing cell became.
        let lenient = Field::new("payload", dtype("binary(4)"), true)
            .cast_arrow_array(cells, ArrowCastOptions::new())
            .unwrap();
        let lenient = lenient
            .as_ref()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(lenient.value(0), b"abc");
        assert!(lenient.is_null(1));
        assert!(lenient.is_null(2));
    }

    #[test]
    fn a_bounded_target_writes_its_own_layout_from_any_byte_source() {
        let text: ArrayRef = Arc::new(StringArray::from(vec!["ab"]));
        let large = dtype("large_binary")
            .cast_arrow_array(text, strict())
            .unwrap();
        assert_eq!(
            downcast::<LargeBinaryArray>(large.as_ref())
                .unwrap()
                .value(0),
            b"ab"
        );

        let fixed = dtype("fixed_binary(3)")
            .cast_arrow_array(Arc::new(BinaryArray::from_vec(vec![b"abc"])), strict())
            .unwrap();
        let view = dtype("sized_binary(3)")
            .cast_arrow_array(fixed, strict())
            .unwrap();
        assert_eq!(view.data_type(), &arrow_schema::DataType::Binary);
    }

    #[test]
    fn an_unbounded_layout_is_its_storage_and_a_declared_source_is_exact() {
        let cells: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"abc"]));
        let plain = DataType::binary()
            .cast_arrow_array(Arc::clone(&cells), strict())
            .unwrap();
        assert!(Arc::ptr_eq(&plain, &cells));

        // A column written as `binary(4)` was measured when it was written,
        // so it comes back as the same array rather than a re-read one.
        let source = batch(Field::new("payload", dtype("binary(4)"), true), cells);
        let root = Field::new(
            "row",
            DataType::from_fields([Field::new("payload", dtype("binary(4)"), true)]).unwrap(),
            false,
        );
        let exact = root.cast_arrow_batch(source.clone(), strict()).unwrap();
        assert!(Arc::ptr_eq(exact.column(0), source.column(0)));
    }

    #[test]
    fn two_fixed_widths_are_a_value_change_and_say_so() {
        let fixed = dtype("fixed_binary(3)")
            .cast_arrow_array(Arc::new(BinaryArray::from_vec(vec![b"abc"])), strict())
            .unwrap();
        let refused = dtype("fixed_binary(4)")
            .cast_arrow_array(fixed, strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("value change"), "{refused}");
    }
}

/// An empty text cell entering a column that holds neither text nor bytes is
/// no value: null through every door, before any spelling is parsed and before
/// `safe` is asked, so `nullability` alone decides what a required column does
/// with it. A column that holds text or bytes keeps the cell as the value it is.
mod empty_text {
    use std::sync::Arc;

    use arrow_array::types::{Int8Type, Int16Type};
    use arrow_array::{
        Array, ArrayRef, BinaryArray, DictionaryArray, Int16Array, Int32Array, LargeStringArray,
        ListArray, RunArray, StringArray, StringViewArray,
    };

    use yggdryl::FieldValue as _;
    use yggdryl::arrow::scalar_value;
    use yggdryl::cast::ArrowCastOptions;
    use yggdryl::{DataType, Field, Nullability, Scalar, TimeUnit, Timezone};

    /// A failed conversion is an error rather than a null.
    fn conversion_error() -> ArrowCastOptions {
        ArrowCastOptions::new().with_safe(false)
    }

    /// A required column refuses a null rather than repairing it.
    fn strict() -> ArrowCastOptions {
        ArrowCastOptions::new().with_nullability(Nullability::Strict)
    }

    const UNITS: [TimeUnit; 4] = [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ];

    /// Every leaf that holds neither text nor bytes and reads a text column.
    fn non_text_targets() -> Vec<DataType> {
        let mut targets = vec![
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::Float16,
            DataType::Float32,
            DataType::Float64,
            DataType::decimal32(9, 2).unwrap(),
            DataType::decimal64(18, 6).unwrap(),
            DataType::decimal128(38, 10).unwrap(),
            DataType::decimal256(76, 20).unwrap(),
            DataType::Boolean,
            DataType::date32(),
            DataType::date64(),
            DataType::time32(TimeUnit::Second).unwrap(),
            DataType::time32(TimeUnit::Millisecond).unwrap(),
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            DataType::time64(TimeUnit::Nanosecond).unwrap(),
            DataType::uuid(),
            DataType::uuidv4(),
            DataType::uuidv7(),
            DataType::uuidv8(),
            DataType::Version,
            DataType::Url,
            DataType::Timezone,
            DataType::MimeType,
            DataType::MediaType,
            DataType::geometry(None).unwrap(),
            DataType::geography(None, None).unwrap(),
            // An encoding is a layout: the values node answers.
            DataType::dictionary(DataType::Int8, DataType::Int32).unwrap(),
        ];
        targets.extend(codes_by_neutral_member().1);
        let zones: [Timezone; 4] = [
            Timezone::NAIVE,
            Timezone::UTC,
            "Europe/Paris".parse().unwrap(),
            "+05:30".parse().unwrap(),
        ];
        for unit in UNITS {
            for zone in zones {
                targets.push(DataType::datetime64(unit, zone).unwrap());
            }
            targets.push(DataType::duration32(unit).unwrap());
            targets.push(DataType::duration64(unit).unwrap());
        }
        targets
    }

    /// The eleven registered codes.
    fn codes() -> Vec<DataType> {
        vec![
            DataType::Country,
            DataType::Currency,
            DataType::Mic,
            DataType::Cfi,
            DataType::Isin,
            DataType::Cusip,
            DataType::Sedol,
            DataType::Bloomberg,
            DataType::Side,
            DataType::State,
            DataType::TimeInForce,
        ]
    }

    /// The codes holding the empty text as their neutral member - which is
    /// their canonical default - and the identifiers holding none, whose
    /// default is refused rather than invented.
    fn codes_by_neutral_member() -> (Vec<DataType>, Vec<DataType>) {
        codes()
            .into_iter()
            .partition(|code| code.default_value().is_ok())
    }

    /// The eighteen string leaves.
    fn string_leaves() -> Vec<DataType> {
        vec![
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::large_utf8_view(),
            DataType::fixed_utf8(3).unwrap(),
            DataType::sized_utf8(4).unwrap(),
            DataType::ascii(),
            DataType::large_ascii(),
            DataType::ascii_view(),
            DataType::large_ascii_view(),
            DataType::fixed_ascii(3).unwrap(),
            DataType::sized_ascii(4).unwrap(),
            DataType::cp1252(),
            DataType::large_cp1252(),
            DataType::cp1252_view(),
            DataType::large_cp1252_view(),
            DataType::fixed_cp1252(3).unwrap(),
            DataType::sized_cp1252(4).unwrap(),
        ]
    }

    /// The byte leaves that hold a payload of any length.
    fn byte_leaves() -> Vec<DataType> {
        vec![
            DataType::binary(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::large_binary_view(),
            DataType::sized_binary(4).unwrap(),
        ]
    }

    /// One `""` cell under each plain text layout, and a dictionary and a
    /// run-end pair over one.
    fn empty_sources() -> Vec<ArrayRef> {
        vec![
            Arc::new(StringArray::from(vec![""])),
            Arc::new(LargeStringArray::from(vec![""])),
            Arc::new(StringViewArray::from(vec![""])),
            Arc::new(DictionaryArray::<Int8Type>::from_iter([Some("")])),
            Arc::new(
                RunArray::<Int16Type>::try_new(
                    &Int16Array::from(vec![1]),
                    &StringArray::from(vec![""]),
                )
                .unwrap(),
            ),
        ]
    }

    fn cell(field: &Field, array: &ArrayRef) -> Scalar {
        scalar_value(field, array.as_ref()).unwrap()
    }

    /// Whether the one row is null as a reader sees it: a dictionary pair
    /// whose key points at a null value is null through its key.
    fn is_null(array: &ArrayRef) -> bool {
        array.logical_nulls().is_some_and(|nulls| nulls.is_null(0))
    }

    #[test]
    fn an_empty_cell_is_null_in_a_nullable_column_whatever_safe_says() {
        for target in non_text_targets() {
            let field = Field::new("x", target, true);
            for source in empty_sources() {
                for options in [ArrowCastOptions::new(), conversion_error()] {
                    let cast = field
                        .cast_arrow_array(Arc::clone(&source), options)
                        .unwrap_or_else(|error| {
                            panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                        });
                    assert_eq!(cast.len(), 1, "{}", field.dtype());
                    assert!(
                        is_null(&cast),
                        "{:?} -> {} kept the empty cell",
                        source.data_type(),
                        field.dtype()
                    );
                }
            }
        }
    }

    #[test]
    fn a_required_column_repairs_or_refuses_an_empty_cell_by_its_nullability() {
        for target in non_text_targets() {
            let field = Field::new("x", target, false);
            for source in empty_sources() {
                let repaired = field.cast_arrow_array(Arc::clone(&source), ArrowCastOptions::new());
                match field.default_value() {
                    Ok(default) => {
                        let repaired = repaired.unwrap_or_else(|error| {
                            panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                        });
                        assert_eq!(cell(&field, &repaired), default, "{}", field.dtype());
                    }
                    // A code with no neutral member has nothing to repair
                    // with, so the null the empty cell became is refused.
                    Err(_) => assert!(repaired.is_err(), "{}", field.dtype()),
                }

                let refused = field
                    .cast_arrow_array(Arc::clone(&source), strict())
                    .err()
                    .unwrap_or_else(|| {
                        panic!(
                            "{:?} -> {} took the empty cell",
                            source.data_type(),
                            field.dtype()
                        )
                    })
                    .to_string();
                assert_eq!(
                    refused,
                    "required Arrow field $.x holds 1 null values",
                    "{:?} -> {}",
                    source.data_type(),
                    field.dtype()
                );
            }
        }
    }

    #[test]
    fn a_spelling_no_reader_takes_keeps_its_own_answer_beside_an_empty_cell() {
        let field = Field::new("x", DataType::Int32, true);
        let mixed: ArrayRef = Arc::new(StringArray::from(vec!["7", "", "not a number"]));

        let lenient = field
            .cast_arrow_array(Arc::clone(&mixed), ArrowCastOptions::new())
            .unwrap();
        let lenient = lenient.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(lenient.value(0), 7);
        assert!(lenient.is_null(1));
        assert!(lenient.is_null(2));

        // The refusal is the misspelt cell's, and the empty one is never named.
        let refused = field
            .cast_arrow_array(mixed, conversion_error())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("not a number"), "{refused}");
        assert!(!refused.contains("''"), "{refused}");

        // Through one of the crate's own readers, whose wording names the
        // row: the misspelt cell is row 2, and row 1 - the empty one - is
        // never named.
        let dates = Field::new("x", DataType::date32(), true);
        let mixed: ArrayRef = Arc::new(StringArray::from(vec!["2024-01-01", "", "not a date"]));
        let refused = dates
            .cast_arrow_array(mixed, conversion_error())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("row 2"), "{refused}");
        assert!(!refused.contains("row 1"), "{refused}");

        // Whitespace is not empty: it is a spelling no reader takes.
        let blank: ArrayRef = Arc::new(StringArray::from(vec![" "]));
        let lenient = field
            .cast_arrow_array(Arc::clone(&blank), ArrowCastOptions::new())
            .unwrap();
        assert!(lenient.is_null(0));
        assert!(field.cast_arrow_array(blank, conversion_error()).is_err());
    }

    #[test]
    fn a_text_or_byte_column_keeps_an_empty_cell_as_the_value_it_is() {
        let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
        for leaf in string_leaves().into_iter().chain(byte_leaves()) {
            let field = Field::new("x", leaf, true);
            let cast = field
                .cast_arrow_array(Arc::clone(&empty), conversion_error())
                .unwrap_or_else(|error| panic!("{}: {error}", field.dtype()));
            assert!(!cast.is_null(0), "{}", field.dtype());
            assert_eq!(
                cell(&field, &cast),
                field.scalar("").unwrap(),
                "{}",
                field.dtype()
            );
        }

        // A fixed width is a rule about the payload, and an empty one is the
        // wrong width.
        assert!(
            Field::new("x", DataType::fixed_binary(4).unwrap(), true)
                .cast_arrow_array(empty, conversion_error())
                .is_err()
        );

        // The rule reads one direction: an empty payload renders as `""`.
        let payload: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b""]));
        let field = Field::new("x", DataType::utf8(), true);
        let text = field.cast_arrow_array(payload, conversion_error()).unwrap();
        assert!(!text.is_null(0));
        assert_eq!(cell(&field, &text), Scalar::from(""));
    }

    #[test]
    fn the_scalar_door_reads_an_empty_text_as_null_before_any_parser() {
        let empty = Scalar::from("");
        for target in [
            DataType::Int32,
            DataType::Float16,
            DataType::Float64,
            DataType::decimal128(10, 2).unwrap(),
            DataType::Boolean,
            DataType::date32(),
            DataType::date64(),
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
            DataType::uuid(),
            DataType::Cusip,
            DataType::Version,
            DataType::Url,
            DataType::Timezone,
            DataType::MimeType,
            DataType::MediaType,
            DataType::geometry(None).unwrap(),
        ] {
            assert_eq!(target.scalar("").unwrap(), Scalar::Null, "{target}");
            assert_eq!(
                target.cast_scalar(&empty).unwrap(),
                Scalar::Null,
                "{target}"
            );
            assert_eq!(target.try_cast_scalar(&empty), Scalar::Null, "{target}");

            let refused = Field::new("x", target.clone(), false)
                .scalar("")
                .unwrap_err()
                .to_string();
            assert!(refused.contains("$.x"), "{target}: {refused}");
            assert!(refused.contains("null"), "{target}: {refused}");
        }

        assert_eq!(DataType::utf8().scalar("").unwrap(), Scalar::from(""));
        assert_eq!(
            DataType::binary().scalar("").unwrap().as_bytes(),
            Some(&[][..])
        );
    }

    /// A code with a neutral member holds the empty text as that member - it
    /// is the code's own canonical default - so a cast keeps it through every
    /// door and reads its own repair back; an identifier holds none, so the
    /// empty text is absence, as it is for a UUID.
    #[test]
    fn a_code_with_a_neutral_member_keeps_an_empty_cell_as_that_member() {
        let (neutral, identifiers) = codes_by_neutral_member();
        assert!(!neutral.is_empty());
        assert!(!identifiers.is_empty());
        for code in neutral {
            let member = code.default_value().unwrap();
            assert_eq!(code.scalar("").unwrap(), member, "{code}");
            assert_eq!(
                code.cast_scalar(&Scalar::from("")).unwrap(),
                member,
                "{code}"
            );
            for nullable in [true, false] {
                let field = Field::new("x", code.clone(), nullable);
                for source in empty_sources() {
                    for options in [ArrowCastOptions::new(), conversion_error(), strict()] {
                        let cast = field
                            .cast_arrow_array(Arc::clone(&source), options)
                            .unwrap_or_else(|error| {
                                panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                            });
                        assert!(!cast.is_null(0), "{:?} -> {code}", source.data_type());
                        assert_eq!(cell(&field, &cast), member, "{code}");
                    }
                }
            }

            // Idempotence: what a required column holds after its own
            // repair, and its own default array, read back under Strict.
            let required = Field::new("x", code.clone(), false);
            for column in [
                required
                    .cast_arrow_array(
                        Arc::new(StringArray::from(vec![""])),
                        ArrowCastOptions::new(),
                    )
                    .unwrap(),
                required.default_arrow_array().unwrap(),
            ] {
                let again = required
                    .cast_arrow_array(column, strict())
                    .unwrap_or_else(|error| panic!("{code}: {error}"));
                assert_eq!(cell(&required, &again), member, "{code}");
            }
        }
        for code in identifiers {
            assert_eq!(code.scalar("").unwrap(), Scalar::Null, "{code}");
        }
    }

    /// An interval has no text spelling, so an empty one is a spelling it
    /// refuses rather than absence: today's answer, through both doors.
    #[test]
    fn an_interval_refuses_an_empty_cell_as_the_spelling_it_is_not() {
        let interval = DataType::interval(TimeUnit::DayTime).unwrap();
        assert!(interval.scalar("").is_err());
        assert!(interval.cast_scalar(&Scalar::from("")).is_err());
        assert_eq!(interval.try_cast_scalar(&Scalar::from("")), Scalar::Null);

        let field = Field::new("x", interval, true);
        let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
        let lenient = field
            .cast_arrow_array(Arc::clone(&empty), ArrowCastOptions::new())
            .unwrap();
        assert!(lenient.is_null(0));
        assert!(field.cast_arrow_array(empty, conversion_error()).is_err());
    }

    /// A list target reads a scalar source into its item, so the item is
    /// what answers: a text item keeps the empty cell, a numeric one does not.
    #[test]
    fn a_list_target_answers_for_its_item() {
        let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));

        let texts = Field::new(
            "x",
            DataType::list(DataType::utf8().nullable_field("item")),
            true,
        );
        let cast = texts
            .cast_arrow_array(Arc::clone(&empty), conversion_error())
            .unwrap();
        let list = cast.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(list.value_length(0), 1);
        assert!(!list.values().is_null(0));
        assert_eq!(
            list.values()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            ""
        );

        let counts = Field::new(
            "x",
            DataType::list(DataType::Int32.nullable_field("item")),
            true,
        );
        let cast = counts.cast_arrow_array(empty, conversion_error()).unwrap();
        let list = cast.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(list.value_length(0), 1);
        assert!(list.values().is_null(0));
    }
}
