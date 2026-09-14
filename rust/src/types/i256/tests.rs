use super::{i256, u256};

fn signed(text: &str) -> i256 {
    text.parse().unwrap()
}

fn unsigned(text: &str) -> u256 {
    text.parse().unwrap()
}

const SIGNED_MAX: &str =
    "57896044618658097711785492504343953926634992332820282019728792003956564819967";
const SIGNED_MIN: &str =
    "-57896044618658097711785492504343953926634992332820282019728792003956564819968";
const UNSIGNED_MAX: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

#[test]
fn signed_boundaries_round_trip() {
    for text in [
        "0",
        "-1",
        "170141183460469231731687303715884105727",
        "-170141183460469231731687303715884105728",
        SIGNED_MAX,
        SIGNED_MIN,
    ] {
        assert_eq!(signed(text).to_string(), text);
    }
}

#[test]
fn out_of_range_values_are_refused() {
    assert!(
        "57896044618658097711785492504343953926634992332820282019728792003956564819968"
            .parse::<i256>()
            .is_err()
    );
    assert!(
        "-57896044618658097711785492504343953926634992332820282019728792003956564819969"
            .parse::<i256>()
            .is_err()
    );
}

#[test]
fn ordering_is_signed() {
    let values = [
        "-100000000000000000000",
        "-1",
        "0",
        "1",
        "100000000000000000000",
    ]
    .map(signed);
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn stable_hash_covers_all_four_limbs_deterministically() {
    let values = [0, 8, 16, 24].map(|byte| {
        let mut bytes = [0_u8; 32];
        bytes[byte] = 7;
        i256::from_le_bytes(bytes)
    });

    assert_eq!(values[0].stable_hash(), i256::from_i128(7).stable_hash());
    for (index, value) in values.iter().enumerate() {
        assert_eq!(value.stable_hash(), value.stable_hash());
        for other in &values[index + 1..] {
            assert_ne!(value, other);
            assert_ne!(value.stable_hash(), other.stable_hash());
        }
    }
}

#[test]
fn checked_addition_and_subtraction_cover_signs_and_carries() {
    assert_eq!(signed("5").checked_add(signed("-3")), Some(signed("2")));
    assert_eq!(signed("-5").checked_add(signed("3")), Some(signed("-2")));
    assert_eq!(signed("-5").checked_add(signed("-3")), Some(signed("-8")));
    assert_eq!(signed("5").checked_sub(signed("-3")), Some(signed("8")));
    assert_eq!(signed("-5").checked_sub(signed("3")), Some(signed("-8")));
    assert_eq!(
        signed("18446744073709551615").checked_add(signed("1")),
        Some(signed("18446744073709551616"))
    );

    let maximum = signed(SIGNED_MAX);
    let minimum = signed(SIGNED_MIN);
    assert_eq!(maximum.checked_add(signed("1")), None);
    assert_eq!(minimum.checked_sub(signed("1")), None);
    assert_eq!(minimum.checked_neg(), None);
    assert_eq!(minimum.checked_abs(), None);
}

#[test]
fn multiplication_detects_the_full_signed_boundary() {
    assert_eq!(
        signed("340282366920938463463374607431768211456").checked_mul(signed("2")),
        Some(signed("680564733841876926926749214863536422912"))
    );
    assert_eq!(signed(SIGNED_MAX).checked_mul(signed("2")), None);
    assert_eq!(
        signed("-12").checked_mul(signed("-11")),
        Some(signed("132"))
    );
}

#[test]
fn division_and_remainder_recompose_every_sign_combination() {
    let magnitude = signed("12345678901234567890123456789012345678901234567890");
    for (left, right) in [
        (magnitude, signed("97")),
        (-magnitude, signed("97")),
        (magnitude, signed("-97")),
        (-magnitude, signed("-97")),
    ] {
        let quotient = left.checked_div(right).unwrap();
        let remainder = left.checked_rem(right).unwrap();
        assert_eq!(quotient.checked_mul(right).unwrap() + remainder, left);
        assert!(remainder.is_zero() || remainder.is_negative() == left.is_negative());
    }
    assert_eq!(magnitude.checked_div(i256::ZERO), None);
    assert_eq!(magnitude.checked_rem(i256::ZERO), None);

    let minimum = signed(SIGNED_MIN);
    assert_eq!(minimum.checked_div(signed("-1")), None);
    assert_eq!(minimum.checked_rem(signed("-1")), Some(i256::ZERO));
}

#[test]
fn the_signed_minimum_still_answers_an_unsigned_magnitude() {
    let minimum = signed(SIGNED_MIN);
    assert_eq!(
        minimum.unsigned_abs(),
        unsigned("57896044618658097711785492504343953926634992332820282019728792003956564819968")
    );
    assert_eq!(signed("-7").unsigned_abs(), u256::from_u128(7));
    assert_eq!(signed("7").unsigned_abs(), u256::from_u128(7));
}

#[test]
fn unsigned_boundaries_round_trip() {
    for text in [
        "0",
        "1",
        "340282366920938463463374607431768211455",
        UNSIGNED_MAX,
    ] {
        assert_eq!(unsigned(text).to_string(), text);
    }
    assert_eq!(unsigned(UNSIGNED_MAX), u256::MAX);
    assert!(
        "115792089237316195423570985008687907853269984665640564039457584007913129639936"
            .parse::<u256>()
            .is_err()
    );
    assert!("-1".parse::<u256>().is_err());
    assert!("".parse::<u256>().is_err());
}

#[test]
fn unsigned_arithmetic_saturates_at_neither_end() {
    assert_eq!(u256::MAX.checked_add(u256::from_u128(1)), None);
    assert_eq!(u256::ZERO.checked_sub(u256::from_u128(1)), None);
    assert_eq!(u256::MAX.checked_mul(u256::from_u128(2)), None);
    assert_eq!(
        u256::from_u128(u128::MAX).checked_mul(u256::from_u128(2)),
        Some(unsigned("680564733841876926926749214863536422910"))
    );
    assert_eq!(u256::from_u128(7).checked_div(u256::ZERO), None);
    assert_eq!(u256::from_u128(7).checked_rem(u256::ZERO), None);

    let dividend = unsigned("115792089237316195423570985008687907853269984665640564039457");
    let divisor = unsigned("4294967311");
    let quotient = dividend.checked_div(divisor).unwrap();
    let remainder = dividend.checked_rem(divisor).unwrap();
    assert_eq!(quotient.checked_mul(divisor).unwrap() + remainder, dividend);
    assert!(remainder < divisor);
}

#[test]
fn unsigned_ordering_and_bytes_round_trip() {
    let values = ["0", "1", "18446744073709551616", UNSIGNED_MAX].map(unsigned);
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
    for value in values {
        assert_eq!(u256::from_le_bytes(value.into_le_bytes()), value);
    }
    assert_eq!(u256::from_u128(u128::MAX).as_u128(), Some(u128::MAX));
    assert_eq!(
        unsigned("18446744073709551616").as_u128(),
        Some(1_u128 << 64)
    );
    assert_eq!(u256::MAX.as_u128(), None);
}

#[test]
fn both_widths_serialize_as_their_canonical_text() {
    assert_eq!(serde_json::to_string(&signed("-125")).unwrap(), "\"-125\"");
    assert_eq!(
        serde_json::from_str::<i256>("\"-125\"").unwrap(),
        signed("-125")
    );
    assert_eq!(
        serde_json::from_str::<i256>("-125").unwrap(),
        signed("-125")
    );
    assert_eq!(
        serde_json::to_string(&unsigned(UNSIGNED_MAX)).unwrap(),
        format!("\"{UNSIGNED_MAX}\"")
    );
    assert_eq!(
        serde_json::from_str::<u256>(&format!("\"{UNSIGNED_MAX}\"")).unwrap(),
        u256::MAX
    );
    assert!(serde_json::from_str::<u256>("-1").is_err());
}
