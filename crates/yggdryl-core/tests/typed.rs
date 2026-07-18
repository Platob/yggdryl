//! Functional tests for the [`typed`](yggdryl_core::typed) serialization layer — the
//! `Encoder`/`Decoder` round-trip into an `IOBase`, the `FixedScalar` / `FixedSerie` value carriers
//! (nullable and non-nullable), the vectorized `Reduce` aggregations, the `Bit` boolean column,
//! filtering, and the `HeaderField` metadata — plus the edges (empty, all-null, out-of-range, NaN).

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{Float64, Int128, Int32, Int64, Int8, UInt128, UInt8};
use yggdryl_core::typed::{
    Decoder, Encoder, Field, FixedScalar, FixedSerie, HeaderField, Scalar, Serie,
};

// -------------------------------------------------------------------------------------
// Encoder / Decoder — the byte round-trip over any IOBase
// -------------------------------------------------------------------------------------

#[test]
fn encoder_decoder_round_trip_scalar_and_bulk() {
    let mut h = Heap::new();
    Int32::encode(&mut h, 0, -42).unwrap();
    Int32::encode(&mut h, 1, 7).unwrap();
    assert_eq!(Int32::decode(&h, 0).unwrap(), -42);
    assert_eq!(Int32::decode(&h, 1).unwrap(), 7);

    // Bulk: encode a slice at an element offset, decode it back.
    let mut b = Heap::new();
    Int64::encode_slice(&mut b, 0, &[1, -2, 3, -4]).unwrap();
    let mut out = [0i64; 4];
    Int64::decode_slice(&b, 0, &mut out).unwrap();
    assert_eq!(out, [1, -2, 3, -4]);

    // The widest + the byte type both round-trip.
    let mut w = Heap::new();
    UInt128::encode(&mut w, 3, u128::MAX).unwrap();
    assert_eq!(UInt128::decode(&w, 3).unwrap(), u128::MAX);
    let mut bytes = Heap::new();
    UInt8::encode_slice(&mut bytes, 0, &[0xAB, 0xCD, 0xEF]).unwrap();
    assert_eq!(UInt8::decode(&bytes, 2).unwrap(), 0xEF);
}

// -------------------------------------------------------------------------------------
// FixedScalar — the single-element Scalar
// -------------------------------------------------------------------------------------

#[test]
fn fixed_scalar_value_null_and_option() {
    let some = FixedScalar::<Int32>::of(42);
    assert_eq!(some.value(), Some(42));
    assert_eq!(some.len(), 1);
    assert!(some.is_valid(0));
    assert_eq!(some.get(0), Some(42));
    assert_eq!(some.data_type_id(), DataTypeId::I32);

    let null = FixedScalar::<Int32>::null();
    assert_eq!(null.value(), None);
    assert!(null.is_null(0));
    assert_eq!(null.null_count(), 1);

    assert_eq!(FixedScalar::<Int64>::from_option(Some(7)).value(), Some(7));
    assert_eq!(FixedScalar::<Int64>::from_option(None).value(), None);
}

// -------------------------------------------------------------------------------------
// FixedSerie — the typed column
// -------------------------------------------------------------------------------------

#[test]
fn serie_from_values_reads_and_reduces() {
    let col = FixedSerie::<Int64>::from_values(&[4, 8, 15, 16, 23, 42]);
    assert_eq!(col.len(), 6);
    assert!(!col.is_empty());
    assert_eq!(col.null_count(), 0);
    assert_eq!(col.get(0), Some(4));
    assert_eq!(col.get(5), Some(42));
    assert_eq!(col.get(6), None); // out of range
    assert_eq!(col.values(), vec![4, 8, 15, 16, 23, 42]);
    assert_eq!(
        col.to_options(),
        (0..6).map(|i| col.get(i)).collect::<Vec<_>>()
    );

    // Reductions route to the data buffer's vectorized Aggregate kernels.
    assert_eq!(col.sum().unwrap(), 108i128);
    assert_eq!(col.min().unwrap(), Some(4));
    assert_eq!(col.max().unwrap(), Some(42));
    assert_eq!(col.mean().unwrap(), Some(18.0));
}

#[test]
fn serie_nullable_from_options_and_push() {
    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    assert_eq!(col.len(), 5);
    assert_eq!(col.null_count(), 2);
    assert!(col.is_valid(0) && col.is_null(1) && col.is_valid(2) && col.is_null(3));
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(2), Some(3));
    assert_eq!(
        col.to_options(),
        vec![Some(1), None, Some(3), None, Some(5)]
    );

    // Building by push — a null after non-nulls back-fills the validity buffer.
    let mut built = FixedSerie::<Int8>::new();
    built.push(1);
    built.push(-2);
    built.push_null();
    built.push_option(Some(4));
    built.push_option(None);
    assert_eq!(built.len(), 5);
    assert_eq!(built.null_count(), 2);
    assert_eq!(
        built.to_options(),
        vec![Some(1), Some(-2), None, Some(4), None]
    );
}

#[test]
fn serie_empty_edges() {
    let empty = FixedSerie::<Int32>::new();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.values(), Vec::<i32>::new());
    assert_eq!(empty.to_options(), Vec::<Option<i32>>::new());
    assert_eq!(empty.sum().unwrap(), 0);
    assert_eq!(empty.min().unwrap(), None);
    assert_eq!(empty.max().unwrap(), None);
    assert_eq!(empty.mean().unwrap(), None);

    // An all-null column.
    let all_null = FixedSerie::<Int64>::from_options(&[None, None, None]);
    assert_eq!(all_null.len(), 3);
    assert_eq!(all_null.null_count(), 3);
    assert!(all_null.get(0).is_none() && all_null.get(2).is_none());
}

#[test]
fn serie_wide_and_unsigned_reductions() {
    assert_eq!(
        FixedSerie::<UInt128>::from_values(&[1, 2, 3])
            .sum()
            .unwrap(),
        6u128
    );
    assert_eq!(
        FixedSerie::<Int128>::from_values(&[-5, 10, -20])
            .min()
            .unwrap(),
        Some(-20)
    );
    assert_eq!(
        FixedSerie::<UInt8>::from_values(&[10, 250, 3])
            .max()
            .unwrap(),
        Some(250)
    );
}

#[test]
fn float_serie_min_max_ignore_nan() {
    let col = FixedSerie::<Float64>::from_values(&[1.5, f64::NAN, 2.5, 0.5]);
    assert_eq!(col.min().unwrap(), Some(0.5)); // NaN never poisons min/max
    assert_eq!(col.max().unwrap(), Some(2.5));
    // A sum that includes a NaN is NaN (checked via is_nan, not equality).
    assert!(col.sum().unwrap().is_nan());
    // Over the non-NaN part the sum + mean are exact.
    let clean = FixedSerie::<Float64>::from_values(&[1.5, 2.5, 0.5]);
    assert_eq!(clean.sum().unwrap(), 4.5);
    assert_eq!(clean.mean().unwrap(), Some(1.5));
}

// -------------------------------------------------------------------------------------
// Bit — the boolean column (bit-granular)
// -------------------------------------------------------------------------------------

#[test]
fn bit_serie_packs_and_reads() {
    let col = FixedSerie::<Bit>::from_values(&[true, false, true, true, false]);
    assert_eq!(col.len(), 5);
    assert_eq!(col.get(0), Some(true));
    assert_eq!(col.get(1), Some(false));
    assert_eq!(col.get(4), Some(false));
    assert_eq!(col.values(), vec![true, false, true, true, false]);
    assert_eq!(col.data_type_id(), DataTypeId::Bool);

    // Nullable booleans.
    let nullable = FixedSerie::<Bit>::from_options(&[Some(true), None, Some(false)]);
    assert_eq!(nullable.null_count(), 1);
    assert_eq!(nullable.to_options(), vec![Some(true), None, Some(false)]);
}

// -------------------------------------------------------------------------------------
// Filter
// -------------------------------------------------------------------------------------

#[test]
fn serie_filter_by_bit_mask() {
    let col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40, 50]);
    let mut mask = Heap::new();
    for (index, keep) in [true, false, true, false, true].iter().enumerate() {
        mask.pwrite_bit(index as u64, *keep).unwrap();
    }
    let filtered = col.filter(&mask);
    assert_eq!(filtered.len(), 3);
    assert_eq!(filtered.values(), vec![10, 30, 50]);

    // Filtering a nullable column preserves the surviving nulls.
    let nullable = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None]);
    let mut keep_all = Heap::new();
    for i in 0..4 {
        keep_all.pwrite_bit(i, true).unwrap();
    }
    let kept = nullable.filter(&keep_all);
    assert_eq!(kept.to_options(), vec![Some(1), None, Some(3), None]);
}

// -------------------------------------------------------------------------------------
// Field — the column metadata (Headers-backed)
// -------------------------------------------------------------------------------------

#[test]
fn header_field_metadata_and_serie_field() {
    let field = HeaderField::new(Some("price"), DataTypeId::I64, true);
    assert_eq!(field.name(), Some("price"));
    assert_eq!(field.data_type_id(), DataTypeId::I64);
    assert!(field.nullable());
    // The metadata really lives in the Headers map.
    assert_eq!(field.headers().type_id(), DataTypeId::I64);
    assert_eq!(field.headers().name(), Some("price"));
    assert!(field.headers().nullable());

    // A serie reports its own field: name from `with_name`, nullable from validity presence.
    let non_null = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("id");
    assert_eq!(non_null.field().name(), Some("id"));
    assert_eq!(non_null.field().data_type_id(), DataTypeId::I64);
    assert!(!non_null.field().nullable());

    let nullable = FixedSerie::<Int32>::from_options(&[Some(1), None]);
    assert!(nullable.field().nullable());
    assert_eq!(nullable.field().data_type_id(), DataTypeId::I32);
}
