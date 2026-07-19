//! Functional tests for the [`typed`](yggdryl_core::typed) serialization layer — the
//! `Encoder`/`Decoder` round-trip into an `IOBase`, the `FixedScalar` / `FixedSerie` value carriers
//! (nullable and non-nullable), the vectorized `Reduce` aggregations, the `Bit` boolean column,
//! filtering, and the `HeaderField` metadata — plus the edges (empty, all-null, out-of-range, NaN).

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{
    Decimal128, Decimal256, Decimal32, Decimal64, Float64, Int128, Int32, Int64, Int8, UInt128,
    UInt8, I256,
};
use yggdryl_core::typed::{
    Binary, Decimal, Decoder, Encoder, Field, FixedBinary, FixedScalar, FixedSerie, FixedSizeSerie,
    FixedUtf8, HeaderField, Scalar, Serie, Utf8, VarScalar, VarSerie,
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

// -------------------------------------------------------------------------------------
// Wrapping any IOBase as a typed column (zero-copy view)
// -------------------------------------------------------------------------------------

#[test]
fn serie_from_data_views_an_existing_buffer() {
    // A caller writes i32s into some IOBase (a Heap here; a mapped file / device buffer works the
    // same via the generic `D`), then views it as a typed column without copying.
    let mut buffer = Heap::new();
    buffer.pwrite_i32_array(0, &[100, 200, 300, 400]).unwrap();
    let column: FixedSerie<Int32, Heap> = FixedSerie::from_data(buffer, None, 4);
    assert_eq!(column.len(), 4);
    assert_eq!(column.get(0), Some(100));
    assert_eq!(column.get(3), Some(400));
    assert_eq!(column.values(), vec![100, 200, 300, 400]);
    assert_eq!(column.sum().unwrap(), 1000i64);

    // With a validity buffer the view is null-aware too.
    let mut data = Heap::new();
    data.pwrite_i32_array(0, &[1, 0, 3]).unwrap();
    let mut validity = Heap::new();
    validity.pwrite_bit(0, true).unwrap();
    validity.pwrite_bit(1, false).unwrap();
    validity.pwrite_bit(2, true).unwrap();
    let nullable: FixedSerie<Int32, Heap> = FixedSerie::from_data(data, Some(validity), 3);
    assert_eq!(nullable.to_options(), vec![Some(1), None, Some(3)]);
    assert_eq!(nullable.null_count(), 1);
}

// -------------------------------------------------------------------------------------
// Fixed-point decimals — Decimal32/64/128/256 + the shared Decimal trait + precision/scale
// -------------------------------------------------------------------------------------

#[test]
fn decimal_serie_precision_scale_and_format() {
    // Money as Decimal128 scale 2: the stored value is the unscaled integer; the string is scaled.
    let col = FixedSerie::<Decimal128>::from_values(&[12345, 5, -5, 100000])
        .with_name("price")
        .with_precision_scale(10, 2);
    assert_eq!(col.len(), 4);
    assert_eq!(col.get(0), Some(12345i128)); // the raw unscaled value
    assert_eq!(col.to_decimal_string(0).as_deref(), Some("123.45"));
    assert_eq!(col.to_decimal_string(1).as_deref(), Some("0.05"));
    assert_eq!(col.to_decimal_string(2).as_deref(), Some("-0.05"));
    assert_eq!(col.to_decimal_string(3).as_deref(), Some("1000.00"));
    assert_eq!(col.decimal_scale(), 2);
    assert_eq!(col.decimal_precision(), 10);

    // The field carries the decimal metadata (in its Headers).
    let field = col.field();
    assert_eq!(field.name(), Some("price"));
    assert_eq!(field.data_type_id(), DataTypeId::Decimal128);
    assert_eq!(field.precision(), Some(10));
    assert_eq!(field.scale(), Some(2));
    assert!(field.data_type_id().is_decimal());
}

#[test]
fn decimal_widths_and_trait() {
    assert_eq!(
        FixedSerie::<Decimal32>::from_values(&[999, -1]).get(1),
        Some(-1i32)
    );
    assert_eq!(
        FixedSerie::<Decimal64>::from_values(&[i64::MAX]).get(0),
        Some(i64::MAX)
    );
    // The shared Decimal trait: max precision per width + scale-aware format.
    assert_eq!(Decimal32::MAX_PRECISION, 9);
    assert_eq!(Decimal64::MAX_PRECISION, 18);
    assert_eq!(Decimal128::MAX_PRECISION, 38);
    assert_eq!(Decimal256::MAX_PRECISION, 76);
    assert_eq!(Decimal64::format(-12345, 3), "-12.345");
    assert_eq!(Decimal32::format(7, 0), "7");
}

#[test]
fn decimal256_i256_round_trip_and_format() {
    let col = FixedSerie::<Decimal256>::from_values(&[
        I256::from_i128(12345678901234567890i128),
        I256::from_i128(-1),
        I256::ZERO,
    ])
    .with_precision_scale(40, 4);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(1), Some(I256::from_i128(-1)));
    assert_eq!(
        col.to_decimal_string(0).as_deref(),
        Some("1234567890123456.7890")
    );
    assert_eq!(col.to_decimal_string(2).as_deref(), Some("0.0000"));

    // I256 native basics: i128 interop, ordering, byte round-trip.
    assert_eq!(I256::from_i128(42).to_i128(), Some(42));
    assert!(I256::from_i128(-5) < I256::from_i128(5));
    assert!(I256::ZERO < I256::from_i128(1));
    let bytes = I256::from_i128(-1).to_le_bytes();
    assert_eq!(I256::from_le_bytes(bytes), I256::from_i128(-1));
    assert_eq!(I256::from_i128(-1).to_string(), "-1");
}

#[test]
fn decimal_encoder_decoder_direct() {
    let mut h = Heap::new();
    Decimal128::encode(&mut h, 0, 999i128).unwrap();
    Decimal128::encode(&mut h, 1, -1i128).unwrap();
    assert_eq!(Decimal128::decode(&h, 0).unwrap(), 999);
    assert_eq!(Decimal128::decode(&h, 1).unwrap(), -1);

    // Decimal256 encodes its 32 LE bytes per element.
    let mut d256 = Heap::new();
    Decimal256::encode_slice(&mut d256, 0, &[I256::from_i128(7), I256::from_i128(-9)]).unwrap();
    assert_eq!(Decimal256::decode(&d256, 0).unwrap(), I256::from_i128(7));
    assert_eq!(Decimal256::decode(&d256, 1).unwrap(), I256::from_i128(-9));

    // A nullable decimal column via from_options.
    let nullable = FixedSerie::<Decimal64>::from_options(&[Some(100), None, Some(250)])
        .with_precision_scale(6, 2);
    assert_eq!(nullable.null_count(), 1);
    assert_eq!(nullable.to_decimal_string(0).as_deref(), Some("1.00"));
    assert_eq!(nullable.to_decimal_string(1), None); // null
    assert_eq!(nullable.to_decimal_string(2).as_deref(), Some("2.50"));
}

// -------------------------------------------------------------------------------------
// Variable-length: Binary / Utf8 (offsets + data layout)
// -------------------------------------------------------------------------------------

#[test]
fn var_binary_serie() {
    let col =
        VarSerie::<Binary>::from_values(&[b"hello".to_vec(), b"".to_vec(), b"world!".to_vec()]);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(0), Some(b"hello".to_vec()));
    assert_eq!(col.get(1), Some(b"".to_vec())); // an empty (zero-length) element
    assert_eq!(col.get(2), Some(b"world!".to_vec()));
    assert_eq!(col.get(3), None); // out of range
    assert_eq!(col.data_type_id(), DataTypeId::Binary);
    assert_eq!(col.bytes_at(2), Some(b"world!".to_vec()));

    // push + nullable + raw bytes front door.
    let mut built = VarSerie::<Binary>::new();
    built.push(&b"a".to_vec());
    built.push_null();
    built.push_bytes(b"bc");
    assert_eq!(built.len(), 3);
    assert_eq!(built.null_count(), 1);
    assert_eq!(built.get(1), None);
    assert_eq!(
        built.to_options(),
        vec![Some(b"a".to_vec()), None, Some(b"bc".to_vec())]
    );
}

#[test]
fn var_utf8_serie() {
    let col = VarSerie::<Utf8>::from_options(&[
        Some("héllo".to_string()),
        None,
        Some(String::new()),
        Some("世界".to_string()),
    ]);
    assert_eq!(col.len(), 4);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(0).as_deref(), Some("héllo")); // multibyte
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(3).as_deref(), Some("世界"));
    assert_eq!(col.data_type_id(), DataTypeId::Utf8);
    assert_eq!(col.field().data_type_id(), DataTypeId::Utf8);
    assert!(col.field().nullable());
    // values() ignores validity: the null slot (index 1) is a zero-length span -> "".
    assert_eq!(col.values(), vec!["héllo", "", "", "世界"]);
}

#[test]
fn var_scalar() {
    let some = VarScalar::<Utf8>::of("hi".to_string());
    assert_eq!(some.value(), Some(&"hi".to_string()));
    assert_eq!(some.len(), 1);
    assert!(some.is_valid(0));
    assert_eq!(some.get(0).as_deref(), Some("hi"));

    let null = VarScalar::<Binary>::null();
    assert_eq!(null.value(), None);
    assert!(null.is_null(0));
    assert_eq!(
        VarScalar::<Binary>::from_option(Some(vec![1, 2])).get(0),
        Some(vec![1, 2])
    );
}

// -------------------------------------------------------------------------------------
// Fixed-size (parameterized width): FixedBinary / FixedUtf8
// -------------------------------------------------------------------------------------

#[test]
fn fixed_binary_serie_padded_and_truncated() {
    let col = FixedSizeSerie::<FixedBinary>::from_values(
        4,
        &[b"ab".to_vec(), b"cdef".to_vec(), b"ghijk".to_vec()],
    );
    assert_eq!(col.width(), 4);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(0), Some(b"ab\0\0".to_vec())); // zero-padded to 4
    assert_eq!(col.get(1), Some(b"cdef".to_vec())); // exact width
    assert_eq!(col.get(2), Some(b"ghij".to_vec())); // truncated to 4
    assert_eq!(col.data_type_id(), DataTypeId::FixedBinary);

    let field = col.field();
    assert_eq!(field.byte_width(), Some(4)); // the parameterized length rides the field metadata
    assert!(field.data_type_id().is_binary());
}

#[test]
fn fixed_utf8_serie_nullable() {
    let col = FixedSizeSerie::<FixedUtf8>::from_options(
        3,
        &[Some("ab".to_string()), None, Some("xyz".to_string())],
    )
    .with_name("code");
    assert_eq!(col.width(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(0).as_deref(), Some("ab\0")); // zero-padded to 3
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(2).as_deref(), Some("xyz"));
    assert_eq!(col.field().name(), Some("code"));
    assert_eq!(col.field().byte_width(), Some(3));
    assert!(col.field().data_type_id().is_utf8());
}
