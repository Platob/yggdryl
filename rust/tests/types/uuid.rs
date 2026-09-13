//! Decision 22: UUID layouts belong to the UUID value, not a protocol.

use yggdryl::types::Uuid;
use yggdryl::{DataType, Error, Scalar};

fn assert_round_trips(value: Uuid, version: u8, text: &str) {
    assert_eq!(value.to_string(), text);
    let bytes = value.into_bytes();
    assert_eq!(bytes[6] >> 4, version);
    assert_eq!(bytes[8] >> 6, 2);
    assert_eq!(Uuid::new(value.get()), value);
    assert_eq!(Uuid::from_bytes(&bytes).unwrap(), value);
    assert_eq!(Uuid::from_bytes(text.as_bytes()).unwrap(), value);
    let scalar = Scalar::Uuid(value);
    assert_eq!(DataType::Uuid.scalar(scalar.clone()).unwrap(), scalar);
    assert_eq!(DataType::Uuid.scalar(Scalar::from(text)).unwrap(), scalar);
    assert_eq!(
        serde_json::from_str::<Uuid>(&serde_json::to_string(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn version_7_packs_exact_microsecond_fractions_and_payload_bits() {
    let cases: &[(i64, u64, &str)] = &[
        (0, 0, "00000000-0000-7000-8000-000000000000"),
        (1, 0, "00000000-0000-7004-8000-000000000000"),
        (244, 0, "00000000-0000-73e7-8000-000000000000"),
        (250, 0, "00000000-0000-7400-8000-000000000000"),
        (456, 0, "00000000-0000-774b-8000-000000000000"),
        (500, 0, "00000000-0000-7800-8000-000000000000"),
        (999, 0, "00000000-0000-7ffb-8000-000000000000"),
        (1_000, 0, "00000000-0001-7000-8000-000000000000"),
        (1_001, 0, "00000000-0001-7004-8000-000000000000"),
        (
            1_645_557_742_000_456,
            0xfedc_ba98_7654_3210,
            "017f22e2-79b0-774b-bedc-ba9876543210",
        ),
        (
            281_474_976_710_655_999,
            0,
            "ffffffff-ffff-7ffb-8000-000000000000",
        ),
        (
            281_474_976_710_655_999,
            u64::MAX,
            "ffffffff-ffff-7ffb-bfff-ffffffffffff",
        ),
    ];
    for &(micros, payload, text) in cases {
        assert_round_trips(Uuid::from_v7(micros, payload).unwrap(), 7, text);
    }
}

#[test]
fn version_7_orders_every_microsecond_and_millisecond_rollover_before_payload() {
    for base in [0, 1_645_557_742_000_000, 281_474_976_710_653_000] {
        for offset in 0..2_000 {
            let micros = base + offset;
            let earlier = Uuid::from_v7(micros, u64::MAX).unwrap();
            let later = Uuid::from_v7(micros + 1, 0).unwrap();
            assert!(earlier < later, "{micros}: UUID value order");
            assert!(
                earlier.into_bytes() < later.into_bytes(),
                "{micros}: storage order"
            );
        }
    }
}

#[test]
fn version_7_discards_exactly_the_two_high_payload_bits() {
    let instant = 1_645_557_742_000_456;
    let baseline = Uuid::from_v7(instant, 0).unwrap();
    for bit in 0..64 {
        let value = Uuid::from_v7(instant, 1_u64 << bit).unwrap();
        let difference = value.get() ^ baseline.get();
        assert_eq!(difference, if bit < 62 { 1_u128 << bit } else { 0 });
    }
    let payload = 0x0123_4567_89ab_cdef;
    for high in 0..4 {
        assert_eq!(
            Uuid::from_v7(instant, payload | (high << 62)).unwrap(),
            Uuid::from_v7(instant, payload).unwrap()
        );
    }
}

#[test]
fn version_7_refuses_negative_and_overflow_instants_at_the_value_root() {
    for micros in [i64::MIN, -1, 281_474_976_710_656_000, i64::MAX] {
        let error = Uuid::from_v7(micros, 0).unwrap_err();
        let Error::InvalidRecord { path, reason } = error else {
            panic!("expected a located UUID value refusal, got {error}");
        };
        assert_eq!(path, "$");
        assert_eq!(
            reason.as_str(),
            format!(
                "expected a UUIDv7 Unix microsecond instant in 0..=281474976710655999, got {micros}"
            )
        );
    }
}

#[test]
fn version_8_keeps_the_rfc_illustrative_vector_and_extremes() {
    // RFC 9562 Appendix B.2: the leading 128 bits of its example SHA-256.
    const RFC_VALUE: Uuid = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    assert_round_trips(RFC_VALUE, 8, "5c146b14-3c52-8afd-938a-375d0df1fbf6");
    for (payload, text) in [
        (0, "00000000-0000-8000-8000-000000000000"),
        (u128::MAX, "ffffffff-ffff-8fff-bfff-ffffffffffff"),
        (
            0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
            "01234567-89ab-8def-8123-456789abcdef",
        ),
    ] {
        assert_round_trips(Uuid::from_v8(payload), 8, text);
    }
}

#[test]
fn version_8_replaces_only_the_six_version_and_variant_bits() {
    let baseline = Uuid::from_v8(0);
    for bit in 0..128 {
        let value = Uuid::from_v8(1_u128 << bit);
        assert_eq!(
            value.get() ^ baseline.get(),
            if matches!(bit, 62 | 63 | 76..=79) {
                0
            } else {
                1_u128 << bit
            },
            "bit {bit}"
        );
        assert_eq!(Uuid::from_v8(value.get()), value);
    }
}
