//! PART A — the arithmetic ops coerce a convertible right operand of **any** type into the left's
//! element type (utf8/binary numeric strings, decimals, temporals, wide integers), not just the 12
//! numeric leaves; and PART B — targeted SIMD-correctness cases proving the vectorized `*_unchecked`
//! loops (add / div with nulls + a zero divisor, integer overflow wrap) stay byte-identical to a
//! per-element reference. The result type always **follows the LEFT** operand; nulls propagate.

use yggdryl_core::io::fixed::temporal::{Date32, TimeUnit, Tz};
use yggdryl_core::io::fixed::{D128Serie, Date32Serie, Field, Serie, D128};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId, NumericSerie};

/// An erased leaf scalar of the given native type + bytes.
fn leaf(id: DataTypeId, width: usize, bytes: Vec<u8>) -> AnyScalar {
    AnyScalar::leaf(Field::of("", id, width, false), bytes)
}

// -------------------------------------------------------------------------------------
// PART A — serie right operand of a non-numeric family is coerced into the left's type.
// -------------------------------------------------------------------------------------

#[test]
fn utf8_right_operand_is_parsed_into_the_left_type() {
    // i64.add(utf8["5","6"]) -> the strings parse into i64, result follows the left (i64).
    let left = boxed(Serie::from_values(&[1i64, 2]));
    let right = boxed(Utf8Serie::from_strs(&[Some("5"), Some("6")]));
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::I64);
    assert_eq!(
        sum.as_serie::<i64>().unwrap().to_options(),
        [Some(6), Some(8)]
    );
}

#[test]
fn utf8_fractional_string_reaches_a_float_target() {
    // f64.add(utf8["2.5"]) -> parsed as f64 (an integer target would reject the fraction).
    let left = boxed(Serie::from_values(&[1.0f64]));
    let right = boxed(Utf8Serie::from_strs(&[Some("2.5")]));
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(sum.as_serie::<f64>().unwrap().to_options(), [Some(3.5)]);
}

#[test]
fn non_numeric_string_is_a_guided_error() {
    // i64.add(utf8["x"]) -> a guided parse error naming the value + target.
    let left = boxed(Serie::from_values(&[1i64]));
    let right = boxed(Utf8Serie::from_strs(&[Some("x")]));
    let err = left.add(right.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("cannot parse") && err.contains("i64") && err.contains('x'),
        "got: {err}"
    );
}

#[test]
fn decimal_right_operand_is_coerced() {
    // i64.add(d128[5.00, 3.00]) -> the decimal value (through f64) truncates into i64.
    let left = boxed(Serie::from_values(&[10i64, 20]));
    let right = boxed(
        D128Serie::from_values(
            10,
            2,
            &[D128::new(500, 2).unwrap(), D128::new(300, 2).unwrap()],
        )
        .unwrap(),
    );
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::I64);
    assert_eq!(
        sum.as_serie::<i64>().unwrap().to_options(),
        [Some(15), Some(23)]
    );

    // f64.add(d128[2.25, 0.50]) -> the decimal value is kept for a float target.
    let fleft = boxed(Serie::from_values(&[1.5f64, 2.5]));
    let fright = boxed(
        D128Serie::from_values(
            10,
            2,
            &[D128::new(225, 2).unwrap(), D128::new(50, 2).unwrap()],
        )
        .unwrap(),
    );
    let fsum = fleft.add(fright.as_ref()).unwrap();
    assert_eq!(
        fsum.as_serie::<f64>().unwrap().to_options(),
        [Some(3.75), Some(3.0)]
    );
}

#[test]
fn temporal_right_operand_coerces_its_backing_count() {
    // i64.add(date32[day 5, day 6]) -> the day counts coerce into i64.
    let left = boxed(Serie::from_values(&[100i64, 200]));
    let right = boxed(
        Date32Serie::from_values(
            TimeUnit::Day,
            Tz::NAIVE,
            &[Date32::from_days(5), Date32::from_days(6)],
        )
        .unwrap(),
    );
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::I64);
    assert_eq!(
        sum.as_serie::<i64>().unwrap().to_options(),
        [Some(105), Some(206)]
    );
}

#[test]
fn wide_int_right_operand_is_coerced_and_range_checked() {
    // i64.add(u128[10, 20]) -> the wide magnitudes coerce into i64 (range-checked).
    let left = boxed(Serie::from_values(&[1i64, 2]));
    let right = boxed(Serie::from_values(&[10u128, 20]));
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(
        sum.as_serie::<i64>().unwrap().to_options(),
        [Some(11), Some(22)]
    );

    // A wide magnitude beyond the i128 bridge -> a guided out-of-range error.
    let small = boxed(Serie::from_values(&[0i8]));
    let huge = boxed(Serie::from_values(&[u128::MAX]));
    assert!(small.add(huge.as_ref()).is_err());

    // A wide value that fits i128 but not the (narrower) left type -> a guided range error.
    let thousand = boxed(Serie::from_values(&[1000u128]));
    let err = small.add(thousand.as_ref()).unwrap_err().to_string();
    assert!(err.contains("range") && err.contains("i8"), "got: {err}");
}

#[test]
fn nulls_are_preserved_through_the_coercion() {
    // A null in either the numeric left or the coerced (utf8) right propagates to a null.
    let left = boxed(Serie::from_options(&[Some(1i64), None, Some(3)]));
    let right = boxed(Utf8Serie::from_strs(&[Some("10"), Some("20"), None]));
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(
        sum.as_serie::<i64>().unwrap().to_options(),
        [Some(11), None, None]
    );
}

// -------------------------------------------------------------------------------------
// PART A — scalar broadcast of a non-numeric convertible scalar.
// -------------------------------------------------------------------------------------

#[test]
fn scalar_broadcast_coerces_utf8_decimal_temporal_and_wide() {
    let col = boxed(Serie::from_values(&[10i64, 20, 30]));

    // A utf8 numeric-string scalar.
    let utf8_scalar = leaf(DataTypeId::Utf8, 4, b"5".to_vec());
    assert_eq!(
        col.add_scalar(&utf8_scalar)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(15), Some(25), Some(35)]
    );

    // A decimal scalar (built through the column so it carries its scale metadata): 5.00 -> 5.
    let d128_scalar = D128Serie::from_values(10, 2, &[D128::new(500, 2).unwrap()])
        .unwrap()
        .value(0);
    assert_eq!(
        col.add_scalar(&d128_scalar)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(15), Some(25), Some(35)]
    );

    // A temporal (date32) scalar -> its backing day count (5).
    let date_scalar = Date32Serie::from_values(TimeUnit::Day, Tz::NAIVE, &[Date32::from_days(5)])
        .unwrap()
        .value(0);
    assert_eq!(
        col.add_scalar(&date_scalar)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(15), Some(25), Some(35)]
    );

    // A wide-integer (u128) scalar -> its magnitude (7), range-checked.
    let wide_scalar = leaf(DataTypeId::U128, 16, 7u128.to_le_bytes().to_vec());
    assert_eq!(
        col.add_scalar(&wide_scalar)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(17), Some(27), Some(37)]
    );

    // A non-numeric utf8 scalar -> a guided parse error.
    let bad = leaf(DataTypeId::Utf8, 4, b"nope".to_vec());
    assert!(col.add_scalar(&bad).is_err());
}

// -------------------------------------------------------------------------------------
// PART B — SIMD-correctness: the vectorized `*_unchecked` loops equal a per-element reference,
// byte-for-byte (including the canonical placeholder under null slots).
// -------------------------------------------------------------------------------------

#[test]
fn simd_add_matches_reference_over_large_n_with_nulls() {
    let n = 1000usize;
    let a: Vec<Option<i32>> = (0..n as i32).map(|i| (i % 7 != 0).then_some(i)).collect();
    let b: Vec<Option<i32>> = (0..n as i32)
        .map(|i| (i % 11 != 0).then_some(i.wrapping_mul(2)))
        .collect();
    let sum = Serie::from_options(&a).add_unchecked(&Serie::from_options(&b));

    let expected: Vec<Option<i32>> = (0..n)
        .map(|i| match (a[i], b[i]) {
            (Some(x), Some(y)) => Some(x.wrapping_add(y)),
            _ => None,
        })
        .collect();
    assert_eq!(sum.to_options(), expected);
    // Byte-identical to the per-element path: same values (incl. the default placeholder under
    // every null slot), so it compares equal to a freshly built column.
    assert_eq!(sum, Serie::from_options(&expected));
    for (i, e) in expected.iter().enumerate() {
        if e.is_none() {
            assert_eq!(
                sum.values()[i],
                0,
                "null slot must hold the canonical placeholder"
            );
        }
    }
}

#[test]
fn simd_div_matches_reference_with_zero_divisor_and_nulls() {
    let n = 600usize;
    let a: Vec<Option<i32>> = (0..n as i32)
        .map(|i| (i % 5 != 0).then_some(i + 1))
        .collect();
    // Divisor: null at multiples of 8, else `i % 3` (a 0 divisor at multiples of 3 -> a null cell).
    let b: Vec<Option<i32>> = (0..n as i32)
        .map(|i| (i % 8 != 0).then_some(i % 3))
        .collect();
    let sa = Serie::from_options(&a);
    let sb = Serie::from_options(&b);
    let div = sa.div_unchecked(&sb); // must not panic on the zero divisors
    let rem = sa.rem_unchecked(&sb);

    let expected_div: Vec<Option<i32>> = (0..n)
        .map(|i| match (a[i], b[i]) {
            (Some(x), Some(y)) if y != 0 => Some(x.wrapping_div(y)),
            _ => None, // null input OR an integer zero divisor
        })
        .collect();
    let expected_rem: Vec<Option<i32>> = (0..n)
        .map(|i| match (a[i], b[i]) {
            (Some(x), Some(y)) if y != 0 => Some(x.wrapping_rem(y)),
            _ => None,
        })
        .collect();
    assert_eq!(div, Serie::from_options(&expected_div));
    assert_eq!(rem, Serie::from_options(&expected_rem));
}

#[test]
fn simd_i128_min_div_neg_one_stays_defined() {
    // i128::MIN / -1 overflows in checked arithmetic; the vectorized kernel wraps (defined, no panic).
    let a = Serie::from_values(&[i128::MIN, 10, 7]);
    let b = Serie::from_values(&[-1i128, 0, 2]); // the 0 divisor -> a null cell
    assert_eq!(
        a.div_unchecked(&b).to_options(),
        [Some(i128::MIN), None, Some(3)]
    );
}

#[test]
fn simd_i8_add_wraps_like_reference() {
    let n = 260usize;
    let a: Vec<i8> = (0..n).map(|i| i as i8).collect();
    let b: Vec<i8> = (0..n).map(|i| i.wrapping_mul(3) as i8).collect();
    let sum = Serie::from_values(&a).add_unchecked(&Serie::from_values(&b));
    let expected: Vec<Option<i8>> = (0..n).map(|i| Some(a[i].wrapping_add(b[i]))).collect();
    assert_eq!(sum.to_options(), expected);
    // The classic boundary.
    assert_eq!(
        Serie::from_values(&[127i8])
            .add_unchecked(&Serie::from_values(&[1i8]))
            .get(0),
        Some(-128)
    );
}

#[test]
fn simd_scalar_zero_divisor_nulls_every_cell() {
    let n = 300usize;
    let col = Serie::from_values(&(0..n as i64).collect::<Vec<_>>());
    let out = col.div_scalar_unchecked(0); // integer zero divisor -> all null, no panic
    assert_eq!(out.null_count(), n);
    assert!(out.to_options().iter().all(Option::is_none));

    // A float column divides by zero to IEEE ±∞ / NaN (never a null).
    let f = Serie::from_values(&[1.0f64, -2.0]);
    let d = f.div_scalar_unchecked(0.0);
    assert_eq!(d.null_count(), 0);
    assert!(d.get(0).unwrap().is_infinite() && d.get(1).unwrap().is_infinite());
}

#[test]
fn simd_reductions_match_reference_over_large_n() {
    let n = 4096usize;
    // No-null column -> the slice-fold fast path.
    let dense: Vec<i32> = (0..n as i32).map(|i| i - 2000).collect();
    let col = Serie::from_values(&dense);
    let want_sum: f64 = dense.iter().map(|&v| v as f64).sum();
    assert_eq!(col.sum_f64(), want_sum);
    assert_eq!(col.min_f64(), Some(-2000.0));
    assert_eq!(col.max_f64(), Some((n as i32 - 1 - 2000) as f64));

    // With nulls -> the null-aware fold; results identical to skipping the nulls.
    let opt: Vec<Option<i32>> = (0..n as i32).map(|i| (i % 3 != 0).then_some(i)).collect();
    let scol = Serie::from_options(&opt);
    let want: f64 = opt.iter().flatten().map(|&v| v as f64).sum();
    assert_eq!(scol.sum_f64(), want);
    assert_eq!(scol.min_f64(), Some(1.0));
}
