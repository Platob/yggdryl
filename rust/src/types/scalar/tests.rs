//! Scalar identity and container tests.

use std::sync::Arc;

use super::{Float16, Float32, Float64, Scalar};
use crate::types::floating::FloatingValue;
use crate::{
    DataType, DataTypeId, DataTypeKind, I256, ScalarFamily, ScalarValue, TimeUnit, Timezone,
};

fn order() -> Scalar {
    Scalar::from_mapping([
        (Scalar::from("symbol"), Scalar::from("AAPL")),
        (
            Scalar::from("legs"),
            Scalar::from_sequence([
                Scalar::from_mapping([(Scalar::from("price"), Scalar::from(12_i64))]).unwrap(),
                Scalar::from_mapping([(Scalar::from("price"), Scalar::from(13_i64))]).unwrap(),
            ]),
        ),
        (Scalar::from("venue"), Scalar::Null),
    ])
    .unwrap()
}

#[test]
fn a_dotted_path_walks_mappings_and_sequences() {
    let order = order();

    assert_eq!(order.path("symbol").and_then(Scalar::as_str), Some("AAPL"));
    assert_eq!(
        order.path("legs.1.price").and_then(Scalar::as_i64),
        Some(13)
    );

    // A segment that does not resolve is absence, not an error.
    assert!(order.path("legs.9.price").is_none());
    assert!(order.path("symbol.price").is_none());
    assert!(order.path("missing").is_none());

    // An empty path is the value itself.
    assert_eq!(order.path(""), Some(&order));
}

#[test]
fn narrowing_an_integer_refuses_to_lose_magnitude() {
    assert_eq!(Scalar::from(7_i64).as_i64(), Some(7));
    assert_eq!(Scalar::from(7_u64).as_u64(), Some(7));

    // A 128-bit value that does not fit is None rather than a wrapped one.
    assert_eq!(Scalar::from(i128::MAX).as_i64(), None);
    assert_eq!(Scalar::from(u128::MAX).as_u64(), None);
    assert_eq!(Scalar::from(-1_i64).as_u64(), None);
}

#[test]
fn float_stable_hashes_follow_canonical_nan_and_exact_zero_bits() {
    let f16_nan = Float16::from_f16(half::f16::from_bits(0x7d01));
    let f32_nan = Float32::from_f32(f32::from_bits(0x7f80_0001));
    let f64_nan = Float64::from_f64(f64::from_bits(0x7ff0_0000_0000_0001));

    assert_eq!(
        f16_nan.stable_hash(),
        Float16::from_f16(half::f16::NAN).stable_hash()
    );
    assert_eq!(
        f32_nan.stable_hash(),
        Float32::from_f32(f32::NAN).stable_hash()
    );
    assert_eq!(
        f64_nan.stable_hash(),
        Float64::from_f64(f64::NAN).stable_hash()
    );
    assert_ne!(
        Float16::from_f16(half::f16::ZERO).stable_hash(),
        Float16::from_f16(half::f16::NEG_ZERO).stable_hash()
    );
    assert_ne!(
        Float32::from_f32(0.0).stable_hash(),
        Float32::from_f32(-0.0).stable_hash()
    );
    assert_ne!(
        Float64::from_f64(0.0).stable_hash(),
        Float64::from_f64(-0.0).stable_hash()
    );
}

#[test]
fn generic_float_selector_keeps_width_and_common_value_semantics() {
    let f16 = Scalar::from_float(1.5, 16).unwrap();
    let f32 = Scalar::from_float(1.5, 32).unwrap();
    let f64 = Scalar::from_float(1.5, 64).unwrap();

    assert_eq!(f16.as_float().unwrap().bit_width(), 16);
    assert_eq!(f32.as_float().unwrap().bit_width(), 32);
    assert_eq!(f64.as_float().unwrap().bit_width(), 64);
    assert_eq!(f16.as_float().unwrap().into_scalar(), f16);
    assert_eq!(f32, f64);
    assert_eq!(
        f32.as_float().unwrap().stable_hash(),
        f64.as_float().unwrap().stable_hash()
    );
    for invalid in [0, 15, 17, 31, 33, 63, 65, u8::MAX] {
        assert!(Scalar::from_float(1.5, invalid).is_err());
    }
    for width in [16, 32, 64] {
        assert!(
            Scalar::from_float(f64::NAN, width)
                .unwrap()
                .as_f64()
                .unwrap()
                .is_nan()
        );
        assert!(
            Scalar::from_float(-0.0, width)
                .unwrap()
                .as_f64()
                .unwrap()
                .is_sign_negative()
        );
    }
    assert!(Scalar::from(1).as_float().is_none());
}

#[test]
fn integer_family_view_is_logical_and_canonical() {
    let signed = Scalar::I8(7).as_integer().unwrap();
    let unsigned = Scalar::U64(7).as_integer().unwrap();
    let minimum = Scalar::I128(i128::MIN).as_integer().unwrap();
    let maximum = Scalar::U128(u128::MAX).as_integer().unwrap();
    let first_unsigned_only = Scalar::U128(i128::MAX as u128 + 1).as_integer().unwrap();

    assert_eq!(signed, unsigned);
    assert_eq!(signed.as_i128(), Some(7));
    assert_eq!(signed.as_u128(), Some(7));
    assert_eq!(signed.into_scalar(), Scalar::I64(7));
    assert_eq!(minimum.into_scalar(), Scalar::I128(i128::MIN));
    assert_eq!(maximum.as_i128(), None);
    assert_eq!(maximum.into_scalar(), Scalar::U128(u128::MAX));
    assert_eq!(first_unsigned_only.as_i128(), None);
    assert_eq!(
        first_unsigned_only.into_scalar(),
        Scalar::U128(i128::MAX as u128 + 1)
    );
    assert!(Scalar::from(1.5).as_integer().is_none());
}

#[test]
fn shape_predicates_answer_without_matching() {
    assert!(Scalar::Null.is_null());
    assert!(Scalar::from(1_i64).is_integer());
    assert!(Scalar::from(1.5).is_number());
    assert!(Scalar::d128(15, 1).is_number());
    assert!(Scalar::d256(I256::from_i128(15), 1).is_number());
    assert!(!Scalar::from(1.5).is_integer());
    assert!(order().is_container());
    assert!(!Scalar::from("AAPL").is_container());
}

#[test]
fn mapping_helpers_read_and_rebuild_in_order() {
    let order = order();

    assert_eq!(order.keys(), vec!["symbol", "legs", "venue"]);
    assert!(order.contains_key("venue"));
    assert_eq!(order.entries().count(), 3);

    // A null value counts as absent for a default.
    let fallback = Scalar::from("XPAR");
    assert_eq!(order.get_or("venue", &fallback), &fallback);
    assert_eq!(order.get_or("symbol", &fallback), &Scalar::from("AAPL"));

    // Replacing keeps position; adding appends.
    let updated = order.with_key("venue", "XPAR").unwrap();
    assert_eq!(updated.keys(), vec!["symbol", "legs", "venue"]);
    assert_eq!(updated.path("venue").and_then(Scalar::as_str), Some("XPAR"));

    let added = order.with_key("currency", "EUR").unwrap();
    assert_eq!(added.keys(), vec!["symbol", "legs", "venue", "currency"]);

    let removed = order.without_key("venue").unwrap();
    assert_eq!(removed.keys(), vec!["symbol", "legs"]);
    // Removing something absent changes nothing.
    assert_eq!(removed.without_key("absent").unwrap(), removed);
}

#[test]
fn a_geospatial_value_is_its_own_kind_over_its_bytes() {
    use std::hash::{Hash, Hasher};

    fn hash_of(value: &Scalar) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    let wkb: &[u8] = &[1, 1, 0, 0, 0];
    let point = Scalar::Geospatial(wkb.into());
    assert_eq!(point.kind(), "geospatial");

    // The same bytes under the bytes kind are a different value: the kind
    // is part of the identity, exactly as it is for string versus bytes.
    let bytes = Scalar::from(wkb);
    assert_ne!(point, bytes);
    assert_ne!(hash_of(&point), hash_of(&bytes));

    // Within the kind, the bytes compare, and equal values hash equal.
    assert_eq!(point, Scalar::Geospatial(wkb.into()));
    assert_eq!(hash_of(&point), hash_of(&Scalar::Geospatial(wkb.into())));
    assert!(point < Scalar::Geospatial([1u8, 2].as_slice().into()));
}

#[test]
fn the_structural_wire_round_trips_a_geospatial_value() {
    let point = Scalar::Geospatial([1u8, 1, 0, 0, 0].as_slice().into());
    let encoded = serde_json::to_string(&point).unwrap();
    assert!(encoded.contains("\"type\":\"geospatial\""), "{encoded}");
    let decoded: Scalar = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, point);
}

#[test]
fn structural_record_deserialization_canonicalizes_and_rejects_duplicates() {
    let unordered = r#"{"type":"record","value":{
            "z":{"type":"i8","value":2},
            "a":{"type":"i8","value":1}
        }}"#;
    let record: Scalar = serde_json::from_str(unordered).unwrap();
    assert_eq!(record.keys(), ["a", "z"]);

    let duplicate = r#"{"type":"record","value":{
            "a":{"type":"i8","value":1},
            "a":{"type":"i8","value":2}
        }}"#;
    let message = serde_json::from_str::<Scalar>(duplicate)
        .unwrap_err()
        .to_string();
    assert!(message.contains("duplicate field name"), "{message}");
}

#[test]
fn rebuilding_a_value_that_is_not_a_mapping_says_what_it_is() {
    let message = Scalar::from("AAPL")
        .with_key("symbol", "AAPL")
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected a mapping"), "{message}");
    assert!(message.contains("string"), "{message}");
}

#[test]
fn equal_cross_width_values_have_one_stable_hash() {
    let groups = [
        vec![Scalar::I8(1), Scalar::U64(1), Scalar::I128(1)],
        vec![
            Scalar::F16(Float16::from_f16(half::f16::from_f32(1.0))),
            Scalar::F32(Float32::from_f32(1.0)),
            Scalar::F64(Float64::from_f64(1.0)),
        ],
        vec![Scalar::d128(100, 2), Scalar::d256(I256::from_i128(10), 1)],
        vec![Scalar::date32(1), Scalar::date64(86_400_000)],
        vec![
            Scalar::duration32(1, TimeUnit::Second).unwrap(),
            Scalar::duration64(1_000, TimeUnit::Millisecond).unwrap(),
        ],
    ];
    for group in groups {
        for value in &group[1..] {
            assert_eq!(&group[0], value);
            assert_eq!(group[0].stable_hash(), value.stable_hash());
        }
    }
}

#[test]
fn records_are_sorted_and_rebuilt_by_field_name() {
    let record = Scalar::from_record([
        ("z", Scalar::from(3)),
        ("a", Scalar::from(1)),
        ("m", Scalar::from(2)),
    ])
    .unwrap();
    assert_eq!(record.keys(), vec!["a", "m", "z"]);
    assert_eq!(
        record
            .record_iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "m", "z"]
    );
    assert_eq!(
        record.iter().filter_map(Scalar::as_i64).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    let updated = record
        .with_field("b", 4)
        .unwrap()
        .without_field("m")
        .unwrap();
    assert_eq!(updated.keys(), vec!["a", "b", "z"]);
    assert_eq!(updated.get_key_str("b").and_then(Scalar::as_i64), Some(4));
    assert!(updated.without_field("absent").unwrap() == updated);
    assert!(
        Scalar::from_mapping([])
            .unwrap()
            .with_field("x", 1)
            .is_err()
    );
}

#[test]
fn native_and_json_accessors_have_explicit_borrowing_semantics() {
    let text = Scalar::from("AAPL");
    let bytes = Scalar::from(b"AAPL".as_slice());
    let geometry = Scalar::Geospatial(Arc::from(b"WKB".as_slice()));
    assert_eq!(text.as_utf8(), Some("AAPL"));
    assert_eq!(text.as_bytes(), None);
    assert_eq!(bytes.as_bytes(), Some(b"AAPL".as_slice()));
    assert_eq!(bytes.as_utf8(), None);
    assert_eq!(geometry.as_bytes(), Some(b"WKB".as_slice()));

    let record = Scalar::from_record([
        ("symbol", Scalar::from("AAPL")),
        ("active", Scalar::from(true)),
    ])
    .unwrap();
    let json_bytes = record.as_json_bytes().unwrap();
    let json_utf8 = record.as_json_utf8().unwrap();
    assert_eq!(json_bytes, json_utf8.as_bytes());
    assert_eq!(crate::text::json::from_bytes(&json_bytes).unwrap(), record);
}

#[test]
fn time_datatype_inference_refuses_zones_it_cannot_preserve() {
    let zoned = Scalar::Time64(1, TimeUnit::Microsecond, Timezone::UTC);
    assert!(zoned.dtype().is_err());
}

#[test]
fn scalar_traits_narrow_an_existing_leaf_without_revalidation() {
    let leaf = Float32::from_f32(1.25);
    let scalar = ScalarValue::into_scalar(leaf);

    assert_eq!(<Float32 as ScalarValue>::ID, DataTypeId::Float32);
    assert_eq!(<Float32 as ScalarValue>::KIND, DataTypeKind::Floating);
    assert_eq!(ScalarValue::dtype(&leaf), DataType::Float32);
    assert_eq!(<Float32 as ScalarValue>::from_scalar(&scalar), Some(&leaf));
    assert_eq!(ScalarFamily::id(&leaf), DataTypeId::Float32);
    assert_eq!(ScalarFamily::dtype(&leaf), DataType::Float32);
    assert_eq!(<Float32 as ScalarFamily>::from_scalar(&scalar), Some(&leaf));
    assert_eq!(FloatingValue::as_f64(&leaf), 1.25);
    assert_eq!(<Float32 as FloatingValue>::BIT_WIDTH, 32);
}
