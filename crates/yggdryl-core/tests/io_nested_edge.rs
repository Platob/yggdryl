//! Adversarial edge cases for `io::nested` + the root erased primitives: sliced Arrow arrays, the
//! wide non-Arrow-native integers, all-null and null-typed children, field-less structs, deep
//! nesting, decimals with nulls, corrupt-input robustness, safe downcast, and hashable erased
//! values.

use yggdryl_core::io::fixed::{Field, Serie, U96};
use yggdryl_core::io::nested::StructSerie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, read_any_leaf, AnyField, Bytes, DataTypeId};

// -------------------------------------------------------------------------------------
// Serialization robustness (no arrow)
// -------------------------------------------------------------------------------------

#[test]
fn deeply_nested_struct_serialize_round_trip() {
    // struct { a: struct { b: struct { c: i32, d: utf8 } } } — three levels deep.
    let c = boxed(Serie::from_values(&[1i32, 2, 3]));
    let d = boxed(Utf8Serie::from_strs(&[Some("x"), None, Some("z")]));
    let level3 = StructSerie::from_named(vec![("c", c), ("d", d)]).unwrap();
    let level2 = StructSerie::from_named(vec![("b", boxed(level3))]).unwrap();
    let level1 = StructSerie::from_named(vec![("a", boxed(level2))]).unwrap();
    assert_eq!(level1.len(), 3);
    let back = StructSerie::deserialize_bytes(&level1.serialize_bytes()).unwrap();
    assert_eq!(back, level1);
}

#[test]
fn corrupt_serialized_bytes_are_guided_errors_not_panics() {
    let table = StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
        (
            "name",
            boxed(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")])),
        ),
    ])
    .unwrap();
    let bytes = table.serialize_bytes();

    for cut in 0..bytes.len() {
        if let Ok(partial) = StructSerie::deserialize_bytes(&bytes[..cut]) {
            assert_ne!(
                partial, table,
                "truncation at {cut} wrongly decoded the full value"
            );
        }
    }
    assert!(StructSerie::deserialize_bytes(&[0xff; 32]).is_err());
    assert!(StructSerie::deserialize_bytes(&[]).is_err());
}

#[test]
fn wide_integer_children_serialize_round_trip() {
    let a = boxed(Serie::from_values(&[1i128, -2, 3]));
    let b = boxed(Serie::from_options(&[Some(7u128), None, None]));
    let c = boxed(Serie::from_values(&[
        U96::from_le_bytes([1; 12]),
        U96::from_le_bytes([2; 12]),
        U96::from_le_bytes([3; 12]),
    ]));
    let table = StructSerie::from_named(vec![("i128", a), ("u128", b), ("u96", c)]).unwrap();
    assert_eq!(table.column(0).unwrap().type_id(), DataTypeId::I128);
    assert_eq!(table.column(2).unwrap().type_id(), DataTypeId::U96);
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back, table);
}

#[test]
fn corrupt_var_offsets_are_a_guided_error_not_a_panic() {
    // A hand-crafted utf8 leaf frame with an offset past the (empty) data buffer.
    let field = AnyField::leaf(Field::of("s", DataTypeId::Utf8, 4, true));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1u64.to_le_bytes()); // len = 1
    bytes.push(0); // no validity
    bytes.extend_from_slice(&0i32.to_le_bytes()); // offsets[0] = 0
    bytes.extend_from_slice(&100i32.to_le_bytes()); // offsets[1] = 100 (past data)
    bytes.extend_from_slice(&0u64.to_le_bytes()); // data_len = 0
    let err = read_any_leaf(&field, &mut Bytes::from_slice(&bytes)).unwrap_err();
    assert!(
        err.to_string().contains("corrupt variable-length offsets"),
        "{err}"
    );
}

#[test]
fn hostile_column_length_errors_instead_of_allocating() {
    use yggdryl_core::io::IoError;
    let field = AnyField::leaf(Field::of("n", DataTypeId::I64, 8, false));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(1u64 << 40).to_le_bytes()); // huge len
    bytes.push(1); // validity present -> the bounded reader errors on the short source
    let err = read_any_leaf(&field, &mut Bytes::from_slice(&bytes)).unwrap_err();
    assert!(matches!(
        err,
        IoError::UnexpectedEof { .. } | IoError::CorruptLength { .. }
    ));
}

#[test]
fn hostile_var_column_length_errors_instead_of_allocating() {
    // A variable-length (utf8) leaf frame declaring a giant length must error on the *bounded* read,
    // not pre-allocate `len + 1` i32 offsets (~terabytes) straight from the untrusted length and
    // abort the process. `validity = 0` so the reader skips the (already bounded) validity read and
    // reaches the offsets buffer — the reachable-through-any-nested-frame path.
    use yggdryl_core::io::IoError;
    let field = AnyField::leaf(Field::of("s", DataTypeId::Utf8, 4, true));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(1u64 << 40).to_le_bytes()); // huge len
    bytes.push(0); // no validity -> the offsets buffer is reached next
    let err = read_any_leaf(&field, &mut Bytes::from_slice(&bytes)).unwrap_err();
    assert!(matches!(
        err,
        IoError::UnexpectedEof { .. } | IoError::CorruptLength { .. }
    ));
}

#[test]
fn null_struct_rows_are_equal_regardless_of_phantom_values() {
    let col = boxed(Serie::from_values(&[1i64, 2]));
    let table =
        StructSerie::from_columns(vec![col.field("id")], vec![col], Some(&[false, false])).unwrap();
    let row0 = table.row_scalar(0);
    let row1 = table.row_scalar(1);
    assert!(row0.is_null() && row1.is_null());
    assert_eq!(row0, row1);
    use std::collections::HashSet;
    let set: HashSet<_> = [row0, row1].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn erased_values_are_hashable_map_keys() {
    use std::collections::HashSet;
    let col = boxed(Serie::from_values(&[7i64, 7, 9]));
    let (v0, v1, v2) = (col.value(0), col.value(1), col.value(2));
    assert_eq!(v0, v1);
    let set: HashSet<_> = [v0, v1, v2].into_iter().collect();
    assert_eq!(set.len(), 2); // {7, 9}
}

#[test]
fn typed_downcasts_are_keyed_on_the_element_type() {
    use yggdryl_core::io::fixed::{D128Serie, Dec128, Dec64, D128};
    use yggdryl_core::io::var::{Binary, Utf8};
    // A decimal column downcasts only to its exact backing, never a different width or a raw int.
    let dec = boxed(D128Serie::from_values(20, 2, &[D128::new(1, 2).unwrap()]).unwrap());
    assert!(dec.as_decimal::<Dec128>().is_some());
    assert!(dec.as_decimal::<Dec64>().is_none());
    assert!(dec.as_serie::<i128>().is_none());
    // A utf8 column and a binary column are the same struct with a different marker — the
    // downcast must still tell them apart.
    let text = boxed(Utf8Serie::from_strs(&[Some("a")]));
    assert!(text.as_bytes_serie::<Utf8>().is_some());
    assert!(text.as_bytes_serie::<Binary>().is_none());
}

#[test]
fn schema_that_is_not_a_struct_is_a_guided_error() {
    // A frame whose schema decodes to a *leaf* field (not a struct) is rejected, not misread.
    let leaf = AnyField::leaf(Field::of("x", DataTypeId::I32, 4, false));
    let schema = leaf.serialize_bytes();
    let mut frame = Vec::new();
    frame.extend_from_slice(&(schema.len() as u64).to_le_bytes());
    frame.extend_from_slice(&schema);
    let err = StructSerie::deserialize_bytes(&frame).unwrap_err();
    assert!(
        err.to_string().contains("did not decode to a struct"),
        "{err}"
    );
}

#[test]
fn child_length_disagreeing_with_the_frame_is_a_guided_error() {
    // Tamper a valid 2-row frame's row count to 3; the child self-reports 2 -> a length mismatch.
    let table =
        StructSerie::from_named(vec![("id", boxed(Serie::from_values(&[1i64, 2])))]).unwrap();
    let mut bytes = table.serialize_bytes();
    let schema_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let len_off = 8 + schema_len; // the frame's row-count u64 sits just after the schema
    bytes[len_off..len_off + 8].copy_from_slice(&3u64.to_le_bytes());
    let err = StructSerie::deserialize_bytes(&bytes).unwrap_err();
    assert!(err.to_string().contains("must be the same length"), "{err}");
}

#[test]
fn from_columns_rejects_a_field_column_count_mismatch() {
    let col = boxed(Serie::from_values(&[1i64, 2]));
    let err = StructSerie::from_columns(vec![], vec![col], None).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("fields but") && text.contains("child columns"),
        "{text}"
    );
}

#[test]
fn from_columns_present_mask_shorter_than_len_marks_only_given_rows() {
    let col = boxed(Serie::from_values(&[1i64, 2, 3]));
    let field = col.field("id");
    // The mask covers only row 0; rows 1..3 stay present (the mask read is `.take(len)`-bounded).
    let table = StructSerie::from_columns(vec![field], vec![col], Some(&[false])).unwrap();
    assert_eq!(table.null_count(), 1);
    assert!(table.row(0).is_null());
    assert!(!table.row(1).is_null());
}

// -------------------------------------------------------------------------------------
// Arrow edge cases (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use arrow_array::Array;
    use yggdryl_core::io::fixed::{I256, I96, U256};
    use yggdryl_core::io::{from_arrow_any_leaf, AnySerie};

    /// Round-trips a single-column struct through a StructArray and asserts byte-exact identity.
    fn round_trip(name: &str, column: Box<dyn AnySerie>) {
        let table = StructSerie::from_named(vec![(name, column)]).unwrap();
        let field = table.to_field("s").to_arrow_field();
        let array = table.to_arrow_array().unwrap();
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, table, "array round-trip differed for {name}");
    }

    #[test]
    fn large_offset_arrow_types_are_rejected_not_corrupted() {
        let arr = arrow_array::LargeStringArray::from(vec![Some("a"), Some("bb"), Some("ccc")]);
        let field = arrow_schema::Field::new("s", arrow_schema::DataType::LargeUtf8, true);
        let err = from_arrow_any_leaf(&arr, &field).unwrap_err();
        assert!(err.to_string().contains("not a yggdryl-modeled"), "{err}");

        let bin = arrow_array::LargeBinaryArray::from(vec![Some(&b"x"[..])]);
        let bfield = arrow_schema::Field::new("b", arrow_schema::DataType::LargeBinary, true);
        assert!(from_arrow_any_leaf(&bin, &bfield).is_err());
    }

    #[test]
    fn wide_integer_children_arrow_round_trip() {
        round_trip("i128", boxed(Serie::from_values(&[1i128, -2, i128::MAX])));
        round_trip("u128", boxed(Serie::from_options(&[Some(9u128), None])));
        round_trip(
            "u96",
            boxed(Serie::from_values(&[U96::from_le_bytes([1; 12])])),
        );
        round_trip(
            "i96",
            boxed(Serie::from_values(&[I96::from_le_bytes([0xff; 12])])),
        );
        round_trip(
            "u256",
            boxed(Serie::from_values(&[U256::from_le_bytes([2; 32])])),
        );
        round_trip(
            "i256",
            boxed(Serie::from_values(&[I256::from_le_bytes([3; 32])])),
        );
    }

    #[test]
    fn all_null_and_null_typed_children_round_trip() {
        use yggdryl_core::io::fixed::NullSerie;
        round_trip(
            "maybe",
            boxed(Serie::<i32>::from_options(&[None, None, None])),
        );
        round_trip("nothing", boxed(NullSerie::with_len(3)));

        let table = StructSerie::from_named(vec![
            ("n", boxed(Serie::<i32>::from_options(&[None, None, None]))),
            ("void", boxed(NullSerie::with_len(3))),
        ])
        .unwrap();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
    }

    #[test]
    fn decimal_with_nulls_child_round_trip() {
        use yggdryl_core::io::fixed::{D256Serie, D256};
        let col = boxed(
            D256Serie::from_options(
                40,
                6,
                &[
                    Some(D256::new(123_456, 6).unwrap()),
                    None,
                    Some(D256::new(1, 6).unwrap()),
                ],
            )
            .unwrap(),
        );
        round_trip("amount", col);
    }

    #[test]
    fn decimal_erasure_canonicalizes_garbage_under_nulls() {
        use arrow_buffer::{NullBuffer, ScalarBuffer};
        use yggdryl_core::io::fixed::{D128Serie, D128};
        // A foreign Arrow decimal array with a nonzero coefficient UNDER a null slot.
        let values = ScalarBuffer::<i128>::from(vec![100i128, 999_999, 300]);
        let nulls = NullBuffer::from(vec![true, false, true]);
        let garbage = arrow_array::Decimal128Array::new(values, Some(nulls))
            .with_precision_and_scale(20, 2)
            .unwrap();
        let from_garbage = boxed(D128Serie::from_arrow_array(&garbage));
        let clean = boxed(
            D128Serie::from_options(
                20,
                2,
                &[
                    Some(D128::new(100, 2).unwrap()),
                    None,
                    Some(D128::new(300, 2).unwrap()),
                ],
            )
            .unwrap(),
        );
        // Byte-canonical identity: garbage under the null does not leak into equality or bytes.
        assert_eq!(from_garbage.as_ref(), clean.as_ref());
        assert_eq!(from_garbage.serialize_bytes(), clean.serialize_bytes());
    }

    #[test]
    fn field_less_struct_round_trips() {
        let empty = StructSerie::from_named(vec![]).unwrap();
        assert_eq!(empty.num_columns(), 0);
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.to_arrow_array().unwrap().num_columns(), 0);
        assert_eq!(
            StructSerie::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
            empty
        );
    }

    #[test]
    fn sliced_primitive_child_imports_logical_window() {
        use std::sync::Arc;
        let full = arrow_array::Int32Array::from(vec![Some(1), None, Some(3), Some(4), None]);
        let sliced = full.slice(1, 3); // logical: [None, 3, 4]
        let fields = arrow_schema::Fields::from(vec![arrow_schema::Field::new(
            "n",
            arrow_schema::DataType::Int32,
            true,
        )]);
        let struct_array = arrow_array::StructArray::new(fields, vec![Arc::new(sliced)], None);
        let field = arrow_schema::Field::new("s", struct_array.data_type().clone(), false);
        let table = StructSerie::from_arrow_array(&struct_array, &field).unwrap();
        assert_eq!(table.len(), 3);
        // The imported column equals one built directly from the logical window [None, 3, 4].
        let expected = StructSerie::from_named(vec![(
            "n",
            boxed(Serie::from_options(&[None, Some(3i32), Some(4)])),
        )])
        .unwrap();
        assert_eq!(table, expected);
    }

    #[test]
    fn struct_of_struct_via_record_batch() {
        let inner = StructSerie::from_named(vec![
            ("x", boxed(Serie::from_values(&[1i32, 2]))),
            ("y", boxed(Serie::from_options(&[Some(3i32), None]))),
        ])
        .unwrap();
        let outer = StructSerie::from_named(vec![
            ("point", boxed(inner)),
            ("tag", boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]))),
        ])
        .unwrap();
        let batch = outer.to_record_batch().unwrap();
        assert!(matches!(
            batch.schema().field(0).data_type(),
            arrow_schema::DataType::Struct(_)
        ));
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), outer);
    }

    #[test]
    fn every_leaf_family_as_a_child_via_record_batch() {
        use yggdryl_core::io::fixed::{D64Serie, FixedUtf8Serie, D64};
        use yggdryl_core::io::var::BinarySerie;
        let table = StructSerie::from_named(vec![
            ("u8", boxed(Serie::from_values(&[1u8, 2]))),
            ("f64", boxed(Serie::from_values(&[1.5f64, 2.5]))),
            (
                "utf8",
                boxed(Utf8Serie::from_strs(&[Some("a"), Some("bb")])),
            ),
            (
                "bin",
                boxed(
                    BinarySerie::from_byte_values(&[Some(&b"\x00"[..]), Some(&b"\xff\xfe"[..])])
                        .unwrap(),
                ),
            ),
            (
                "d64",
                boxed(
                    D64Serie::from_values(
                        10,
                        2,
                        &[D64::new(1, 2).unwrap(), D64::new(2, 2).unwrap()],
                    )
                    .unwrap(),
                ),
            ),
            (
                "fu8",
                boxed(
                    FixedUtf8Serie::from_values(2, &[Some(&b"ab"[..]), Some(&b"cd"[..])]).unwrap(),
                ),
            ),
        ])
        .unwrap();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_columns(), 6);
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
    }

    #[test]
    fn more_primitive_families_as_children_arrow_round_trip() {
        use yggdryl_core::io::fixed::f16;
        round_trip("u16", boxed(Serie::from_values(&[1u16, 2, u16::MAX])));
        round_trip("u32", boxed(Serie::from_options(&[Some(5u32), None])));
        round_trip("u64", boxed(Serie::from_values(&[10u64, 20])));
        round_trip("i8", boxed(Serie::from_values(&[-1i8, 2])));
        round_trip("i16", boxed(Serie::from_values(&[-3i16, 4])));
        round_trip("i64", boxed(Serie::from_options(&[None, Some(9i64)])));
        round_trip("f32", boxed(Serie::from_values(&[1.5f32, -2.5])));
        round_trip(
            "f16",
            boxed(Serie::from_values(&[
                f16::from_f32(1.5),
                f16::from_f32(-2.0),
            ])),
        );
    }

    #[test]
    fn struct_child_export_shares_the_primitive_arc_buffer() {
        // The "recomposition is zero-copy" claim, on the nested export path: a primitive child's
        // Arrow value buffer inside the exported StructArray is the *same allocation* as the
        // Serie's own — an Arc bump, not a payload copy.
        let values: Vec<i32> = (0..1024).collect();
        let serie = Serie::from_values(&values);
        let direct_ptr = serie.to_arrow_array().to_data().buffers()[0].as_ptr();
        let table = StructSerie::from_named(vec![("n", boxed(serie))]).unwrap();
        let struct_array = table.to_arrow_array().unwrap();
        let child_ptr = struct_array.column(0).to_data().buffers()[0].as_ptr();
        assert_eq!(
            direct_ptr, child_ptr,
            "struct child export copied the value buffer"
        );
    }

    #[test]
    fn nested_struct_child_with_null_rows_arrow_round_trip() {
        // An inner struct that itself has null rows, inside an outer struct that does not —
        // exercises the child NullBuffer export and offset-aware `struct_validity_from_arrow`.
        let id = boxed(Serie::from_values(&[1i64, 2, 3]));
        let inner_fields = vec![id.field("id")];
        let inner =
            StructSerie::from_columns(inner_fields, vec![id], Some(&[true, false, true])).unwrap();
        assert_eq!(inner.null_count(), 1);
        let outer = StructSerie::from_named(vec![("inner", boxed(inner))]).unwrap();
        let field = outer.to_field("s").to_arrow_field();
        let array = outer.to_arrow_array().unwrap();
        assert_eq!(
            StructSerie::from_arrow_array(&array, &field).unwrap(),
            outer
        );
    }

    #[test]
    fn sliced_wide_integer_child_imports_logical_window() {
        // A wide (non-Arrow-native) u128 column maps to FixedSizeBinary(16); slice it and import,
        // exercising `wide_from_arrow`'s `offset * width` byte arithmetic.
        let wide = boxed(Serie::from_values(&[10u128, 20, 30, 40, 50]));
        let child_field = wide.field("w").to_arrow(); // tagged FixedSizeBinary(16)
        let sliced = arrow_array::Array::slice(&wide.to_arrow_array().unwrap(), 1, 3); // logical [20,30,40]
        let fields = arrow_schema::Fields::from(vec![child_field]);
        let struct_array = arrow_array::StructArray::new(fields, vec![sliced], None);
        let field = arrow_schema::Field::new("s", struct_array.data_type().clone(), false);
        let back = StructSerie::from_arrow_array(&struct_array, &field).unwrap();
        let expected =
            StructSerie::from_named(vec![("w", boxed(Serie::from_values(&[20u128, 30, 40])))])
                .unwrap();
        assert_eq!(back, expected);
    }

    #[test]
    fn wide_integer_erasure_canonicalizes_garbage_under_nulls() {
        // A foreign FixedSizeBinary(16) (a u128 column) with nonzero bytes UNDER a null slot must
        // canonicalize to zeroes on import, so byte-identity holds against a cleanly-built column.
        let mut raw = vec![0u8; 3 * 16];
        raw[0..16].copy_from_slice(&10u128.to_le_bytes());
        raw[16..32].copy_from_slice(&999u128.to_le_bytes()); // garbage under the null
        raw[32..48].copy_from_slice(&30u128.to_le_bytes());
        let values = arrow_buffer::Buffer::from_vec(raw);
        let nulls = arrow_buffer::NullBuffer::from(vec![true, false, true]);
        let garbage = arrow_array::FixedSizeBinaryArray::new(16, values, Some(nulls));
        let field = AnyField::leaf(Field::of("w", DataTypeId::U128, 16, true)).to_arrow();
        let imported = from_arrow_any_leaf(&garbage, &field).unwrap();
        let clean = boxed(Serie::from_options(&[Some(10u128), None, Some(30u128)]));
        assert_eq!(imported.as_ref(), clean.as_ref());
        assert_eq!(imported.serialize_bytes(), clean.serialize_bytes());
    }

    #[test]
    fn empty_fields_struct_with_rows_round_trips() {
        // A field-less struct with N > 0 rows -> the RecordBatchOptions row-count path.
        let array = arrow_array::StructArray::new_empty_fields(4, None);
        let field = arrow_schema::Field::new("s", array.data_type().clone(), false);
        let table = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(table.len(), 4);
        assert_eq!(table.num_columns(), 0);
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_rows(), 4);
        assert_eq!(batch.num_columns(), 0);
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
        assert_eq!(table.to_arrow_array().unwrap().len(), 4);
    }

    #[test]
    fn zero_row_struct_with_fields_round_trips_via_arrow() {
        use yggdryl_core::io::nested::StructField;
        let schema = StructField::new(
            "person",
            vec![
                AnyField::leaf(Field::of("id", DataTypeId::I64, 8, false)),
                AnyField::leaf(Field::of("name", DataTypeId::Utf8, 4, true)),
            ],
            false,
        );
        let table = StructSerie::empty(&schema);
        assert_eq!((table.len(), table.num_columns()), (0, 2));
        let batch = table.to_record_batch().unwrap();
        assert_eq!((batch.num_rows(), batch.num_columns()), (0, 2));
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
        let field = table.to_field("s").to_arrow_field();
        assert_eq!(
            StructSerie::from_arrow_array(&table.to_arrow_array().unwrap(), &field).unwrap(),
            table
        );
    }

    #[test]
    fn temporal_arrow_child_imports_as_a_column() {
        // Temporal columns are now wired into the erased layer, so importing a `Date32` Arrow array
        // yields a real column (downcast back with `as_temporal`) — it used to be a guided
        // "not modeled" error, when temporal was value-types-only.
        use yggdryl_core::io::fixed::Date32Kind;
        let dates = arrow_array::Date32Array::from(vec![19_000, 19_001]);
        let field = arrow_schema::Field::new("d", arrow_schema::DataType::Date32, false);
        let column = from_arrow_any_leaf(&dates, &field).unwrap();
        assert_eq!(column.type_id(), DataTypeId::Date32);
        let dates = column.as_temporal::<Date32Kind>().expect("Date32 column");
        assert_eq!(dates.len(), 2);
        assert_eq!(dates.get(0).unwrap().days(), 19_000);
    }
}
