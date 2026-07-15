//! Arrow interop tests (feature `arrow`): the **zero-copy** conversions between the `fixed`
//! family and Apache Arrow — `Buffer` / `Serie` ↔ `arrow_array::PrimitiveArray`, the shared
//! `Arc` allocation (proven with `ptr_eq`), validity round-trips, and the `DataType` / `Field`
//! ↔ Arrow-schema converters.
#![cfg(feature = "arrow")]

use arrow_array::types::{Int32Type, UInt16Type};
use arrow_array::{Array, PrimitiveArray};
use arrow_schema::DataType as ArrowDataType;

use yggdryl_core::io::fixed::{Buffer, Field, PrimitiveType, Serie, TypedField};
use yggdryl_core::io::DataType;

// -------------------------------------------------------------------------------------
// Buffer <-> Arrow buffer / array (zero-copy)
// -------------------------------------------------------------------------------------

#[test]
fn buffer_to_from_arrow_array_is_zero_copy() {
    let buffer = Buffer::<i32>::from_vec(vec![1, 2, 3, 4]);
    let array = buffer.to_arrow_array();
    assert_eq!(array.len(), 4);
    assert_eq!(array.values().as_ref(), &[1, 2, 3, 4]);

    // Round-trip back, sharing the SAME allocation (an Arc bump, not a copy).
    let back = Buffer::<i32>::from_arrow_array(&array);
    assert_eq!(back.to_vec(), vec![1, 2, 3, 4]);
    assert!(back.to_arrow_buffer().ptr_eq(array.values().inner()));
}

#[test]
fn buffer_arrow_buffer_round_trip() {
    let buffer = Buffer::<u16>::from_vec(vec![10, 20, 30]);
    let arrow = buffer.to_arrow_buffer();
    let back = Buffer::<u16>::from_arrow_buffer(arrow.clone());
    assert_eq!(back.as_slice(), &[10, 20, 30]);
    assert!(back.to_arrow_buffer().ptr_eq(&arrow)); // shared allocation

    // A typed array from another native type is independent.
    let arr: PrimitiveArray<UInt16Type> = buffer.to_arrow_array();
    assert_eq!(arr.values().as_ref(), &[10, 20, 30]);
}

// -------------------------------------------------------------------------------------
// Serie <-> Arrow PrimitiveArray (values zero-copy, validity round-trips)
// -------------------------------------------------------------------------------------

#[test]
fn serie_to_arrow_preserves_nulls_and_shares_values() {
    let column = Serie::from_options(&[Some(1i32), None, Some(3), None, Some(5)]);
    let array = column.to_arrow_array();
    assert_eq!(array.len(), 5);
    assert_eq!(array.null_count(), 2);
    assert!(array.is_null(1));
    assert!(array.is_valid(2));
    assert_eq!(array.value(2), 3);

    // Back to a Serie: nulls preserved, values share the SAME allocation.
    let back = Serie::<i32>::from_arrow_array(&array);
    assert_eq!(back, column);
    assert_eq!(
        back.to_options(),
        vec![Some(1), None, Some(3), None, Some(5)]
    );
    assert!(back
        .to_arrow_array()
        .values()
        .inner()
        .ptr_eq(array.values().inner()));
}

#[test]
fn serie_dense_has_no_null_buffer() {
    let column = Serie::from_values(&[1i32, 2, 3]);
    let array = column.to_arrow_array();
    assert_eq!(array.null_count(), 0);
    assert!(array.nulls().is_none()); // a dense column pays no validity buffer
    assert_eq!(Serie::<i32>::from_arrow_array(&array), column);
}

#[test]
fn serie_from_externally_built_arrow_array() {
    // An array built by Arrow itself (with nulls) reads back into a Serie correctly.
    let array = PrimitiveArray::<Int32Type>::from(vec![Some(10), None, Some(30)]);
    let column = Serie::<i32>::from_arrow_array(&array);
    assert_eq!(column.len(), 3);
    assert_eq!(column.null_count(), 1);
    assert_eq!(column.to_options(), vec![Some(10), None, Some(30)]);
}

#[test]
fn from_arrow_canonicalizes_garbage_under_nulls() {
    use arrow_buffer::{NullBuffer, ScalarBuffer};
    // A foreign array with a non-zero value UNDER a null slot — Arrow leaves those bytes
    // undefined, and real arrays (IPC/Parquet) carry garbage there.
    let values = ScalarBuffer::<i32>::from(vec![1, 999, 3]);
    let nulls = NullBuffer::from(vec![true, false, true]);
    let garbage = PrimitiveArray::<Int32Type>::new(values, Some(nulls));

    let from_garbage = Serie::<i32>::from_arrow_array(&garbage);
    assert_eq!(from_garbage.get(1), None);
    // Byte-canonical identity: the unobservable garbage under the null must NOT leak into value
    // equality — the column equals one built with a plain `None` there.
    assert_eq!(from_garbage, Serie::from_options(&[Some(1), None, Some(3)]));
}

#[test]
fn serie_from_sliced_arrow_array_respects_offset() {
    // A *sliced* arrow array carries a non-zero logical offset in both its values buffer and
    // its null buffer — the conversion must read the logical window, not the raw start.
    let full = PrimitiveArray::<Int32Type>::from(vec![Some(1), None, Some(3), Some(4), None]);
    let sliced = full.slice(1, 3); // logical: [None, Some(3), Some(4)]
    let column = Serie::<i32>::from_arrow_array(&sliced);
    assert_eq!(column.len(), 3);
    assert_eq!(column.to_options(), vec![None, Some(3), Some(4)]);
    assert_eq!(column.null_count(), 1);

    // A slice with no nulls in the window still reads its logical values.
    let dense_window = full.slice(2, 2); // [Some(3), Some(4)]
    assert_eq!(
        Serie::<i32>::from_arrow_array(&dense_window).to_options(),
        vec![Some(3), Some(4)]
    );
}

#[test]
fn element_aligned_slice_converts_to_arrow() {
    use yggdryl_core::io::IOSlice;
    // An element-aligned byte window (offset/len multiples of 4) is a valid typed slice and
    // converts to Arrow zero-copy. (A misaligned window is rejected by `slice` — see the
    // io_fixed alignment tests — so it can never reach `to_arrow_array`.)
    let buffer = Buffer::<i32>::from_vec(vec![1, 2, 3, 4]);
    let window = buffer.slice(4, 8).unwrap(); // bytes [4, 12) == elements [1, 2]
    let array = window.to_arrow_array();
    assert_eq!(array.len(), 2);
    assert_eq!(array.values().as_ref(), &[2, 3]);
}

#[test]
fn empty_and_all_null_edges() {
    // Empty buffer / column round-trip.
    let empty = Buffer::<i32>::from_vec(vec![]);
    assert_eq!(empty.to_arrow_array().len(), 0);
    assert_eq!(
        Buffer::<i32>::from_arrow_array(&empty.to_arrow_array()).count(),
        0
    );

    let empty_col = Serie::<i32>::new();
    assert_eq!(
        Serie::<i32>::from_arrow_array(&empty_col.to_arrow_array()),
        empty_col
    );

    // All-null column.
    let all_null = Serie::<i32>::from_options(&[None, None, None, None]);
    let array = all_null.to_arrow_array();
    assert_eq!(array.null_count(), 4);
    assert_eq!(Serie::<i32>::from_arrow_array(&array), all_null);
}

// -------------------------------------------------------------------------------------
// DataType <-> Arrow
// -------------------------------------------------------------------------------------

#[test]
fn data_type_to_arrow() {
    assert_eq!(PrimitiveType::<i32>::new().to_arrow(), ArrowDataType::Int32);
    assert_eq!(
        PrimitiveType::<u16>::new().to_arrow(),
        ArrowDataType::UInt16
    );
    assert_eq!(
        PrimitiveType::<f64>::new().to_arrow(),
        ArrowDataType::Float64
    );
}

// -------------------------------------------------------------------------------------
// Field <-> Arrow (typed + erased)
// -------------------------------------------------------------------------------------

#[test]
fn typed_field_arrow_round_trip() {
    let field = TypedField::<i64>::new("id", false);
    let arrow = field.to_arrow();
    assert_eq!(arrow.name(), "id");
    assert_eq!(arrow.data_type(), &ArrowDataType::Int64);
    assert!(!arrow.is_nullable());

    // Round-trips when the arrow type matches; `None` when it does not.
    assert_eq!(TypedField::<i64>::from_arrow(&arrow), Some(field));
    assert_eq!(TypedField::<i32>::from_arrow(&arrow), None);
}

#[test]
fn lossy_types_round_trip_exactly_via_metadata() {
    use yggdryl_core::io::fixed::{FixedBinaryField, FixedUtf8Field, I96, U96};
    use yggdryl_core::io::{FieldType, Headers};

    // `u96` → Arrow `FixedSizeBinary(12)` (lossy), but the exact type is pinned in metadata.
    let u96 = TypedField::<U96>::new("hash", false);
    let arrow = u96.to_arrow();
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(12));
    assert_eq!(
        arrow.metadata().get("yggdryl.logical_type"),
        Some(&"u96".to_string())
    );
    assert_eq!(TypedField::<U96>::from_arrow(&arrow), Some(u96));
    // A different wide type of the SAME Arrow width must NOT match.
    assert_eq!(TypedField::<I96>::from_arrow(&arrow), None);

    // `FixedBinary` and `FixedUtf8` both map to `FixedSizeBinary(N)` — metadata disambiguates.
    let fu = FixedUtf8Field::new("code", 4, true);
    let arrow = fu.to_arrow();
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(FixedUtf8Field::from_arrow(&arrow), Some(fu));
    assert_eq!(FixedBinaryField::from_arrow(&arrow), None); // it's utf8, not binary

    // `FixedBinary` is the *unambiguous* default, so it needs no tag.
    let fb = FixedBinaryField::new("blob", 8, false);
    let arrow = fb.to_arrow();
    assert!(arrow.metadata().get("yggdryl.logical_type").is_none());
    assert_eq!(FixedBinaryField::from_arrow(&arrow), Some(fb));

    // `i128` → `Decimal128`, which reverses unambiguously in our scheme — so it, too, is untagged
    // and still round-trips exactly.
    let i128 = TypedField::<i128>::new("big", false);
    let arrow = i128.to_arrow();
    assert_eq!(arrow.data_type(), &ArrowDataType::Decimal128(38, 0));
    assert!(arrow.metadata().get("yggdryl.logical_type").is_none());
    assert_eq!(TypedField::<i128>::from_arrow(&arrow), Some(i128));

    // An exact primitive adds no tag at all.
    let i32 = TypedField::<i32>::new("n", false);
    assert!(i32.to_arrow().metadata().is_empty());

    // User metadata is carried through, and the reserved key is stripped from the user-visible
    // metadata on the way back in.
    let tagged = Field::new(
        "h",
        &yggdryl_core::io::fixed::PrimitiveType::<U96>::new(),
        false,
    )
    .with_metadata(Headers::new().with("unit", "sha256"));
    let back = Field::from_arrow(&tagged.to_arrow()).unwrap();
    assert_eq!(FieldType::type_id(&back), FieldType::type_id(&tagged)); // exact type preserved
    assert_eq!(back.metadata().get("unit"), Some("sha256"));
    assert!(!back.metadata().contains("yggdryl.logical_type")); // reserved key not leaked
    assert_eq!(back, tagged);
}

#[test]
fn wide_type_from_arrow_without_metadata_falls_back_to_fixed_binary() {
    use yggdryl_core::io::FieldType;
    // A foreign `FixedSizeBinary(12)` with no yggdryl tag decodes to the safe default:
    // `fixed_binary` of that width (never a guessed wide integer).
    let foreign = arrow_schema::Field::new("x", ArrowDataType::FixedSizeBinary(12), false);
    let field = Field::from_arrow(&foreign).unwrap();
    assert_eq!(field.type_name(), "fixed_binary");
    assert_eq!(field.byte_width(), 12);
    assert!(FieldType::is_binary(&field) && FieldType::is_fixed_width(&field));
}

#[test]
fn erased_field_arrow_round_trip() {
    let field = Field::new("price", &PrimitiveType::<f64>::new(), true);
    let arrow = field.to_arrow(); // total — no Option
    assert_eq!(arrow.name(), "price");
    assert_eq!(arrow.data_type(), &ArrowDataType::Float64);
    assert!(arrow.is_nullable());
    assert_eq!(Field::from_arrow(&arrow), Some(field));

    // An Arrow type this crate does not model (e.g. Boolean) has no mapping.
    let boolean = arrow_schema::Field::new("flag", ArrowDataType::Boolean, false);
    assert_eq!(Field::from_arrow(&boolean), None);
}
