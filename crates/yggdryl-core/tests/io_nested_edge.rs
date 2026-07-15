//! Adversarial edge cases for `io::nested`: sliced Arrow arrays, the wide non-Arrow-native integers
//! (`u128`/`i128`/`u96`/`i96`/`u256`/`i256`), all-null and null-typed children, field-less structs,
//! deep nesting, decimals with nulls, and corrupt-input robustness. These probe the raw-buffer ↔
//! Arrow bridge and the self-describing schema/byte codec where they are most likely to break.

use yggdryl_core::io::fixed::{Serie, I256, I96, U256, U96};
use yggdryl_core::io::nested::{Column, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::DataTypeId;

// -------------------------------------------------------------------------------------
// Serialization robustness (no arrow)
// -------------------------------------------------------------------------------------

#[test]
fn deeply_nested_struct_serialize_round_trip() {
    // struct { a: struct { b: struct { c: i32, d: utf8 } } } — three levels deep.
    let c = Column::from(Serie::from_values(&[1i32, 2, 3]));
    let d = Column::from(Utf8Serie::from_strs(&[Some("x"), None, Some("z")]));
    let level3 = StructSerie::from_named(vec![("c", c), ("d", d)]).unwrap();
    let level2 = StructSerie::from_named(vec![("b", Column::from(level3))]).unwrap();
    let level1 = StructSerie::from_named(vec![("a", Column::from(level2))]).unwrap();
    assert_eq!(level1.len(), 3);
    let back = StructSerie::deserialize_bytes(&level1.serialize_bytes()).unwrap();
    assert_eq!(back, level1);
}

#[test]
fn corrupt_serialized_bytes_are_guided_errors_not_panics() {
    let table = StructSerie::from_named(vec![
        ("id", Column::from(Serie::from_values(&[1i64, 2, 3]))),
        (
            "name",
            Column::from(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")])),
        ),
    ])
    .unwrap();
    let bytes = table.serialize_bytes();

    // Every truncation prefix decodes to an error (never a panic, never a wrong value).
    for cut in 0..bytes.len() {
        let result = StructSerie::deserialize_bytes(&bytes[..cut]);
        // A prefix may occasionally still parse to *a* struct (e.g. an empty one), but it must
        // never equal the original and must never panic — reaching here at all proves no panic.
        if let Ok(partial) = result {
            assert_ne!(
                partial, table,
                "truncation at {cut} wrongly decoded the full value"
            );
        }
    }
    // Random garbage is a guided error, not a panic.
    assert!(StructSerie::deserialize_bytes(&[0xff; 32]).is_err());
    assert!(StructSerie::deserialize_bytes(&[]).is_err());
}

#[test]
fn wide_integer_children_serialize_round_trip() {
    // The non-Arrow-native wide integers stored as struct children, via the byte codec.
    let a = Column::from(Serie::from_values(&[1i128, -2, 3]));
    let b = Column::from(Serie::from_options(&[Some(7u128), None, None]));
    let c = Column::from(Serie::from_values(&[
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

// -------------------------------------------------------------------------------------
// Regressions for the adversarial-review findings
// -------------------------------------------------------------------------------------

#[test]
fn corrupt_var_offsets_are_a_guided_error_not_a_panic() {
    use yggdryl_core::io::fixed::Field;
    use yggdryl_core::io::nested::ColumnField;
    // A hand-crafted utf8 column frame with an offset past the (empty) data buffer.
    let field = ColumnField::leaf(Field::of("s", DataTypeId::Utf8, 4, true));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1u64.to_le_bytes()); // column len = 1
    bytes.push(0); // no validity
    bytes.extend_from_slice(&0i32.to_le_bytes()); // offsets[0] = 0
    bytes.extend_from_slice(&100i32.to_le_bytes()); // offsets[1] = 100 (past data)
    bytes.extend_from_slice(&0u64.to_le_bytes()); // data_len = 0
    let err = Column::deserialize_bytes(&field, &bytes).unwrap_err();
    assert!(
        err.to_string().contains("corrupt variable-length offsets"),
        "{err}"
    );
}

#[test]
fn hostile_column_length_errors_instead_of_allocating() {
    use yggdryl_core::io::fixed::Field;
    use yggdryl_core::io::nested::ColumnField;
    use yggdryl_core::io::IoError;
    // A tiny frame declaring len = 2^40 with the validity flag set: the bounded reader must error
    // once the (empty) source is exhausted, not pre-allocate ~2^37 bytes for the validity mask.
    let field = ColumnField::leaf(Field::of("n", DataTypeId::I64, 8, false));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(1u64 << 40).to_le_bytes()); // huge len
    bytes.push(1); // validity present -> read_validity reads len/8, bounded
    let err = Column::deserialize_bytes(&field, &bytes).unwrap_err();
    assert!(matches!(
        err,
        IoError::UnexpectedEof { .. } | IoError::CorruptLength { .. }
    ));
}

#[test]
fn null_struct_rows_are_equal_regardless_of_phantom_values() {
    // Two null struct rows (marked null by top-level validity) with different underlying child
    // values must be equal — the child values are logically absent, per the null-object convention.
    let col = Column::from(Serie::from_values(&[1i64, 2]));
    let table =
        StructSerie::from_columns(vec![col.field("id")], vec![col], Some(&[false, false])).unwrap();
    let row0 = table.row_scalar(0);
    let row1 = table.row_scalar(1);
    assert!(row0.is_null() && row1.is_null());
    assert_eq!(row0, row1);
    // And they hash equal, so they collapse to one set entry.
    use std::collections::HashSet;
    let set: HashSet<_> = [row0, row1].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn erased_values_are_hashable_map_keys() {
    use std::collections::HashSet;
    let col = Column::from(Serie::from_values(&[7i64, 7, 9]));
    let (v0, v1, v2) = (col.get(0), col.get(1), col.get(2));
    assert_eq!(v0, v1);
    let set: HashSet<_> = [v0, v1, v2].into_iter().collect();
    assert_eq!(set.len(), 2); // {7, 9}
}

// -------------------------------------------------------------------------------------
// Arrow edge cases (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use arrow_array::Array;

    /// Round-trips a single-column struct through a StructArray and asserts byte-exact identity.
    fn round_trip_array(name: &str, column: Column) {
        let table = StructSerie::from_named(vec![(name, column)]).unwrap();
        let field = table.to_field("s").to_arrow_field();
        let array = table.to_arrow_array();
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, table, "array round-trip differed for {name}");
    }

    #[test]
    fn large_offset_arrow_types_are_rejected_not_corrupted() {
        // A LargeStringArray uses i64 offsets the erased column cannot model — a guided error,
        // never silent corruption or a panic from reading i64 offsets as i32.
        let arr = arrow_array::LargeStringArray::from(vec![Some("a"), Some("bb"), Some("ccc")]);
        let field = arrow_schema::Field::new("s", arrow_schema::DataType::LargeUtf8, true);
        let err = Column::from_arrow_array(&arr, &field).unwrap_err();
        assert!(err.to_string().contains("not a yggdryl-modeled"), "{err}");

        let bin = arrow_array::LargeBinaryArray::from(vec![Some(&b"x"[..])]);
        let bfield = arrow_schema::Field::new("b", arrow_schema::DataType::LargeBinary, true);
        assert!(Column::from_arrow_array(&bin, &bfield).is_err());
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
        let from_garbage = Column::from(D128Serie::from_arrow_array(&garbage));
        let clean = Column::from(
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
        // Byte-canonical identity: the garbage under the null must not leak into equality.
        assert_eq!(from_garbage, clean);
        assert_eq!(from_garbage.serialize_bytes(), clean.serialize_bytes());
    }

    #[test]
    fn wide_integer_children_arrow_round_trip() {
        // i128 -> Decimal128, u128 -> FixedSizeBinary(16), the 96/256-bit -> FixedSizeBinary,
        // i256 -> Decimal256 — every wide integer recovers its exact logical type via metadata.
        round_trip_array(
            "i128",
            Column::from(Serie::from_values(&[1i128, -2, i128::MAX])),
        );
        round_trip_array(
            "u128",
            Column::from(Serie::from_options(&[Some(9u128), None])),
        );
        round_trip_array(
            "u96",
            Column::from(Serie::from_values(&[U96::from_le_bytes([1; 12])])),
        );
        round_trip_array(
            "i96",
            Column::from(Serie::from_values(&[I96::from_le_bytes([0xff; 12])])),
        );
        round_trip_array(
            "u256",
            Column::from(Serie::from_values(&[U256::from_le_bytes([2; 32])])),
        );
        round_trip_array(
            "i256",
            Column::from(Serie::from_values(&[I256::from_le_bytes([3; 32])])),
        );
    }

    #[test]
    fn all_null_and_null_typed_children_round_trip() {
        // An all-null (but typed) i32 column, and a genuine Null-typed column, as struct children.
        round_trip_array(
            "maybe",
            Column::from(Serie::<i32>::from_options(&[None, None, None])),
        );
        round_trip_array("nothing", Column::null(3));

        // Via RecordBatch too.
        let table = StructSerie::from_named(vec![
            (
                "n",
                Column::from(Serie::<i32>::from_options(&[None, None, None])),
            ),
            ("void", Column::null(3)),
        ])
        .unwrap();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
    }

    #[test]
    fn decimal_with_nulls_child_round_trip() {
        use yggdryl_core::io::fixed::{D256Serie, D256};
        let col = Column::from(
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
        round_trip_array("amount", col);
    }

    #[test]
    fn field_less_struct_round_trips() {
        // A struct with zero fields (degenerate) still round-trips its array and byte codec.
        let empty = StructSerie::from_named(vec![]).unwrap();
        assert_eq!(empty.num_columns(), 0);
        assert_eq!(empty.len(), 0);
        let array = empty.to_arrow_array();
        assert_eq!(array.num_columns(), 0);
        assert_eq!(
            StructSerie::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
            empty
        );
    }

    #[test]
    fn sliced_primitive_child_imports_logical_window() {
        use std::sync::Arc;
        // Build a StructArray by hand whose child is a *sliced* Int32Array.
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
        // The imported column must equal a column built directly from the logical window.
        let expected = Column::from(Serie::from_options(&[None, Some(3i32), Some(4)]));
        assert_eq!(table.column(0).unwrap(), &expected);
    }

    #[test]
    fn struct_of_struct_via_record_batch() {
        // A nested struct as a RecordBatch column (StructArray inside a batch).
        let inner = StructSerie::from_named(vec![
            ("x", Column::from(Serie::from_values(&[1i32, 2]))),
            ("y", Column::from(Serie::from_options(&[Some(3i32), None]))),
        ])
        .unwrap();
        let outer = StructSerie::from_named(vec![
            ("point", Column::from(inner)),
            (
                "tag",
                Column::from(Utf8Serie::from_strs(&[Some("a"), Some("b")])),
            ),
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
            ("u8", Column::from(Serie::from_values(&[1u8, 2]))),
            ("f64", Column::from(Serie::from_values(&[1.5f64, 2.5]))),
            (
                "utf8",
                Column::from(Utf8Serie::from_strs(&[Some("a"), Some("bb")])),
            ),
            (
                "bin",
                Column::from(
                    BinarySerie::from_byte_values(&[Some(&b"\x00"[..]), Some(&b"\xff\xfe"[..])])
                        .unwrap(),
                ),
            ),
            (
                "d64",
                Column::from(
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
                Column::from(
                    FixedUtf8Serie::from_values(2, &[Some(&b"ab"[..]), Some(&b"cd"[..])]).unwrap(),
                ),
            ),
        ])
        .unwrap();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_columns(), 6);
        assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);
    }
}
