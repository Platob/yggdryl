//! Tests for the custom 256-bit integers [`I256`] / [`U256`].

use yggdryl_core::{Bytes, I256, U256};

#[test]
fn unsigned_arithmetic_and_ordering() {
    assert_eq!(U256::default(), U256::ZERO);
    assert_eq!(U256::from(2u8) + U256::from(3u8), U256::from(5u8));
    // Carry across a limb boundary: u64::MAX + 1.
    assert_eq!(
        U256::from(u64::MAX) + U256::ONE,
        U256::from_limbs([0, 1, 0, 0])
    );
    // Wrapping at the top: MAX + 1 == 0.
    assert_eq!(U256::MAX + U256::ONE, U256::ZERO);
    assert!(U256::MAX > U256::from(u64::MAX));
    assert!(U256::from(u128::MAX) < U256::from_limbs([0, 0, 1, 0]));
}

#[test]
fn signed_arithmetic_ordering_and_sign() {
    assert_eq!(I256::from(-1i8), -I256::ONE);
    assert_eq!(I256::from(-5i32) + I256::from(3i32), I256::from(-2i32));
    // Negatives sort below positives; MIN is the smallest.
    assert!(I256::from(-1i8) < I256::ZERO);
    assert!(I256::MIN < I256::from(i128::MIN));
    assert!(I256::MAX > I256::from(i128::MAX));
    // Two's-complement negation round-trips.
    assert_eq!(-(-I256::from(42i8)), I256::from(42i8));
}

#[test]
fn little_endian_bytes_round_trip() {
    let value = U256::from(0x0102_0304_0506_0708u64);
    let bytes = value.to_le_bytes();
    assert_eq!(bytes[0], 0x08);
    assert_eq!(bytes[7], 0x01);
    assert_eq!(U256::from_le_bytes(bytes), value);
}

#[test]
fn serialize_through_a_byte_io() {
    // I256 / U256 round-trip through the core Bytes trait as 32 little-endian bytes.
    let u = U256::from(0xdead_beefu32);
    assert_eq!(u.to_bytes().len(), 32);
    assert_eq!(U256::from_bytes(&u.to_bytes()).unwrap(), u);

    let i = I256::from(-12345i32);
    assert_eq!(I256::from_bytes(&i.to_bytes()).unwrap(), i);
}
