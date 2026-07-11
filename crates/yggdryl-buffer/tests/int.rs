//! Tests for the wide integers `i96` / `i128` / `i256`: arithmetic, overflow
//! behaviour, byte round-trips, value semantics, and typed-cursor IO.

use std::collections::HashSet;

use yggdryl_buffer::{i256, i96, TypedCursor, TypedIOBase, Whence};

#[test]
fn i96_range_and_constants() {
    assert_eq!(i96::BITS, 96);
    assert_eq!(i96::MAX.to_i128(), (1i128 << 95) - 1);
    assert_eq!(i96::MIN.to_i128(), -(1i128 << 95));
    assert_eq!(i96::ZERO.to_i128(), 0);
    assert_eq!(i96::ONE.to_i128(), 1);
    assert!(i96::MIN < i96::ZERO && i96::ZERO < i96::MAX);
}

#[test]
fn i96_checked_arithmetic_detects_overflow() {
    assert_eq!(i96::MAX.checked_add(i96::ONE), None);
    assert_eq!(i96::MIN.checked_sub(i96::ONE), None);
    assert_eq!(i96::MIN.checked_neg(), None);
    assert_eq!(
        i96::from_i64(2).checked_add(i96::from_i64(3)),
        Some(i96::from_i64(5))
    );
    // A product that overflows i128 (and so i96) is caught.
    assert_eq!(i96::MAX.checked_mul(i96::MAX), None);
    assert_eq!(
        i96::from_i64(1_000_000).checked_mul(i96::from_i64(1_000_000)),
        Some(i96::from_i64(1_000_000_000_000))
    );
}

#[test]
fn i96_wrapping_and_operators() {
    // Wrapping past MAX comes back around to MIN.
    assert_eq!(i96::MAX.wrapping_add(i96::ONE), i96::MIN);
    assert_eq!(i96::MIN.wrapping_sub(i96::ONE), i96::MAX);
    assert_eq!(i96::MIN.wrapping_neg(), i96::MIN); // -MIN wraps to MIN

    // Operators compute the true value when it fits.
    let a = i96::from_i64(7);
    let b = i96::from_i64(3);
    assert_eq!((a + b).to_i128(), 10);
    assert_eq!((a - b).to_i128(), 4);
    assert_eq!((a * b).to_i128(), 21);
    assert_eq!((a / b).to_i128(), 2);
    assert_eq!((a % b).to_i128(), 1);
    assert_eq!((-a).to_i128(), -7);
    assert_eq!(i96::from_i64(-7).abs(), a);
}

#[test]
#[should_panic(expected = "overflow")]
fn i96_operator_panics_on_overflow() {
    let _ = i96::MAX + i96::ONE;
}

#[test]
fn i96_saturating() {
    assert_eq!(i96::MAX.saturating_add(i96::ONE), i96::MAX);
    assert_eq!(i96::MIN.saturating_sub(i96::ONE), i96::MIN);
    assert_eq!(i96::MAX.saturating_mul(i96::from_i64(2)), i96::MAX);
    assert_eq!(i96::MAX.saturating_mul(i96::from_i64(-2)), i96::MIN);
}

#[test]
fn i96_byte_round_trip_and_value_semantics() {
    for value in [
        i96::ZERO,
        i96::ONE,
        i96::MAX,
        i96::MIN,
        i96::from_i64(-1),
        i96::from_i64(1_234_567_890),
    ] {
        let bytes = value.to_le_bytes();
        assert_eq!(bytes.len(), 12);
        assert_eq!(i96::from_le_bytes(bytes), value);
    }
    // Equal iff bytes equal; hashes agree.
    let set: HashSet<i96> = [i96::ONE, i96::from_i64(1), i96::from_i64(2)]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
    assert_eq!(i96::from_i64(-5).to_string(), "-5");
}

#[test]
fn i96_division_and_remainder_signs() {
    // Division truncates toward zero; the remainder takes the dividend's sign.
    assert_eq!((i96::from_i64(-7) / i96::from_i64(3)).to_i128(), -2);
    assert_eq!((i96::from_i64(-7) % i96::from_i64(3)).to_i128(), -1);
    assert_eq!((i96::from_i64(7) / i96::from_i64(-3)).to_i128(), -2);
    assert_eq!((i96::from_i64(7) % i96::from_i64(-3)).to_i128(), 1);

    // Checked division catches divide-by-zero and the `MIN / -1` overflow.
    assert_eq!(i96::ONE.checked_div(i96::ZERO), None);
    assert_eq!(i96::ONE.checked_rem(i96::ZERO), None);
    assert_eq!(i96::MIN.checked_div(i96::from_i64(-1)), None);
    assert_eq!(
        i96::MIN.checked_rem(i96::from_i64(-1)),
        Some(i96::ZERO),
        "MIN % -1 is 0, no overflow"
    );
}

#[test]
#[should_panic(expected = "divide by zero")]
fn i96_division_by_zero_panics() {
    let _ = i96::ONE / i96::ZERO;
}

#[test]
#[should_panic(expected = "overflow")]
fn i96_abs_of_min_panics() {
    let _ = i96::MIN.abs();
}

#[test]
fn i96_overflowing_and_wrapping_edges() {
    assert_eq!(i96::MAX.overflowing_add(i96::ONE), (i96::MIN, true));
    assert_eq!(
        i96::from_i64(2).overflowing_add(i96::from_i64(3)),
        (i96::from_i64(5), false)
    );
    assert!(i96::MAX.overflowing_mul(i96::from_i64(2)).1, "overflows");

    // wrapping_mul on the exact boundary: 2^95 mod 2^96 wraps to MIN (-2^95).
    let half = i96::from_i128(1i128 << 94);
    assert_eq!(half.wrapping_mul(i96::from_i64(2)), i96::MIN);
    // wrapping div/rem never panic on overflow (MIN / -1 wraps to MIN).
    assert_eq!(i96::MIN.wrapping_div(i96::from_i64(-1)), i96::MIN);
    assert_eq!(i96::MIN.wrapping_rem(i96::from_i64(-1)), i96::ZERO);
}

#[test]
fn i96_conversions_ordering_and_display() {
    assert_eq!(i96::from(-3i32).to_i128(), -3);
    assert_eq!(i96::from(7i64).to_i128(), 7);
    assert_eq!(i128::from(i96::from_i64(-9)), -9);
    assert_eq!(i96::try_from_i128(1i128 << 95), None, "just past MAX");
    assert_eq!(i96::try_from_i128(i96::MAX.to_i128()), Some(i96::MAX));

    // from_i128 wraps; try_from_i128 rejects.
    assert_eq!(i96::from_i128(1i128 << 95), i96::MIN);

    assert!(i96::MIN.is_negative() && i96::MAX.is_positive() && !i96::ZERO.is_negative());

    let mut values = [i96::MAX, i96::MIN, i96::ZERO, i96::ONE, i96::from_i64(-1)];
    values.sort();
    assert_eq!(
        values,
        [i96::MIN, i96::from_i64(-1), i96::ZERO, i96::ONE, i96::MAX]
    );

    assert_eq!(i96::MIN.to_string(), (-(1i128 << 95)).to_string());
    assert_eq!(i96::MAX.to_string(), ((1i128 << 95) - 1).to_string());
    assert_eq!(format!("{:?}", i96::from_i64(5)), "i96(5)");
}

#[test]
fn i256_negative_round_trip_and_ordering() {
    let big = i256::from_i128(i128::MIN) * i256::from_i128(4); // very negative, beyond i128
    assert_eq!(i256::from_le_bytes(big.to_le_bytes()), big);
    assert!(i256::MIN < big && big < i256::ZERO);
    assert!(i256::MIN < i256::MAX);
    // A round trip through the wide byte codec preserves the sign of MIN/MAX.
    for value in [i256::MIN, i256::MAX, i256::from_i128(-1)] {
        assert_eq!(i256::from_le_bytes(value.to_le_bytes()), value);
    }
}

#[test]
fn i256_arithmetic_and_bytes() {
    let big = i256::from_i128(i128::MAX);
    let doubled = big * i256::from_i128(2);
    assert_eq!(doubled.to_i128(), None, "exceeds i128");
    assert_eq!(doubled, big + big);
    assert_eq!(i256::from_le_bytes(doubled.to_le_bytes()), doubled);
    assert_eq!(<i256 as yggdryl_buffer::IoPrimitive>::ZERO, i256::ZERO);
}

#[test]
fn typed_cursor_reads_and_writes_i96() {
    let values = [
        i96::MIN,
        i96::from_i64(-1),
        i96::ZERO,
        i96::from_i64(42),
        i96::MAX,
    ];
    let mut cursor = <TypedCursor<i96> as TypedIOBase<i96>>::with_capacity(values.len());
    assert_eq!(cursor.pwrite_array(&values, Whence::Start).unwrap(), 5);
    // 12 bytes per i96.
    assert_eq!(cursor.as_bytes().len(), 60);

    cursor.seek(0, Whence::Start).unwrap();
    assert_eq!(cursor.tell().unwrap(), 0);
    assert_eq!(cursor.pread_array(5, Whence::Current).unwrap(), values);
    assert_eq!(cursor.tell().unwrap(), 5);
    assert_eq!(cursor.size().unwrap(), 0, "remaining i96 count");
}

#[test]
fn typed_cursor_reads_and_writes_i128_and_i256() {
    let mut c128 = <TypedCursor<i128> as TypedIOBase<i128>>::with_capacity(3);
    c128.pwrite_array(&[i128::MIN, 0, i128::MAX], Whence::Start)
        .unwrap();
    c128.seek(0, Whence::Start).unwrap();
    assert_eq!(
        c128.pread_array(3, Whence::Current).unwrap(),
        vec![i128::MIN, 0, i128::MAX]
    );
    assert_eq!(c128.as_bytes().len(), 48); // 16 bytes each

    let vals256 = [i256::MIN, i256::ZERO, i256::MAX];
    let mut c256 = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(3);
    c256.pwrite_array(&vals256, Whence::Start).unwrap();
    c256.seek(0, Whence::Start).unwrap();
    assert_eq!(c256.pread_array(3, Whence::Current).unwrap(), vals256);
    assert_eq!(c256.as_bytes().len(), 96); // 32 bytes each
}

#[test]
fn typed_cursor_i256_single_and_default_fill() {
    let mut cursor = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(4);
    cursor
        .pwrite_one(i256::from_i128(7), Whence::Start)
        .unwrap();
    // Skip two i256 values, write at index 3; the gap is default (zero) filled.
    cursor.seek(3, Whence::Start).unwrap();
    cursor
        .pwrite_one(i256::from_i128(9), Whence::Current)
        .unwrap();
    cursor.seek(0, Whence::Start).unwrap();
    assert_eq!(
        cursor.pread_array(4, Whence::Current).unwrap(),
        vec![
            i256::from_i128(7),
            i256::ZERO,
            i256::ZERO,
            i256::from_i128(9)
        ]
    );
}
