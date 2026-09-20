//! Scalar identity and container tests.

/// `POINT EMPTY` as little-endian WKB: the geometry default, whose coordinates
/// are both quiet NaN. Stated here rather than read out of the crate, so the
/// default is compared against the bytes and not against itself.
const POINT_EMPTY_WKB: [u8; 21] = [
    0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x7F, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xF8, 0x7F,
];

use std::sync::Arc;

use yggdryl::{Code, FamilyValue, FloatingValue, Geospatial, Nested, Temporal, TemporalValue};
use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit, Timezone, Value, i256};
use yggdryl::{Date32, Date64, DateTime64, Duration32, Duration64, Interval, Time32, Time64};
use yggdryl::{Float16, Float32, Float64, Floating, Scalar};
use yggdryl::{Int8, Int16, Int32, Int64, Int128, Integer, UInt8, UInt16, UInt32, UInt64, UInt128};

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

    // The variant is the width; `kind` and `id` state it.
    assert!(matches!(f16, Scalar::Float16(_)));
    assert!(matches!(f32, Scalar::Float32(_)));
    assert!(matches!(f64, Scalar::Float64(_)));
    assert_eq!([f16.kind(), f32.kind(), f64.kind()], ["f16", "f32", "f64"]);
    assert_eq!(f16.id(), DataTypeId::Float16);
    assert_eq!(f16, f32);
    assert_eq!(f32, f64);
    assert_eq!(f32.stable_hash(), f64.stable_hash());
    assert_eq!(f16.as_f16(), Some(half::f16::from_f32(1.5)));
    assert_eq!(f32.as_f32(), Some(1.5));
    assert_eq!(f64.as_f32(), None);
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
    assert!(Scalar::from(1).as_f64().is_none());
}

#[test]
fn integer_widths_preserve_width_with_logical_comparison() {
    let signed = Scalar::from(7_i32);
    let unsigned = Scalar::from(7_u8);
    let minimum = Scalar::from(i128::MIN);
    let maximum = Scalar::from(u128::MAX);
    let first_unsigned_only = Scalar::from(i128::MAX as u128 + 1);

    assert!(matches!(signed, Scalar::Int32(_)));
    assert!(matches!(unsigned, Scalar::UInt8(_)));
    assert_eq!(signed, unsigned);
    assert_eq!(signed.as_i128(), Some(7));
    assert_eq!(signed.as_u128(), Some(7));
    assert_eq!(unsigned.as_i128(), Some(7));
    assert_eq!(minimum.as_i128(), Some(i128::MIN));
    assert_eq!(minimum.as_u128(), None);
    assert_eq!(maximum.as_i128(), None);
    assert_eq!(maximum.as_u128(), Some(u128::MAX));
    assert_eq!(first_unsigned_only.as_i128(), None);
    assert_eq!(first_unsigned_only.as_u128(), Some(i128::MAX as u128 + 1));
    assert!(Scalar::from(1.5).as_i128().is_none());
    assert!(!Scalar::from(1.5).is_integer());
}

#[test]
fn the_eleven_codes_sort_by_which_code_then_by_text() {
    use std::hash::{Hash, Hasher};

    use yggdryl::{Bloomberg, Cfi, Country, Currency, Cusip, Isin, Mic, Sedol, TimeInForce};

    fn hash_of(value: &Scalar) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    // The eleven share one value rank, so nothing but the identity separates
    // them - and that identity is the one their datatypes sort by, which is
    // what makes a sorted column of fields and a sorted column of values agree.
    let ascending = [
        Scalar::Country(Country::new("FR").unwrap()),
        Scalar::Currency(Currency::new("EUR").unwrap()),
        Scalar::Mic(Mic::new("XPAR").unwrap()),
        Scalar::Cfi(Cfi::new("ESVUFR").unwrap()),
        Scalar::Side(yggdryl::Side::new("BUY").unwrap()),
        Scalar::State(yggdryl::State::read("New").unwrap()),
        Scalar::TimeInForce(TimeInForce::new("1").unwrap()),
        Scalar::Isin(Isin::new("US0378331005").unwrap()),
        Scalar::Cusip(Cusip::new("037833100").unwrap()),
        Scalar::Sedol(Sedol::new("2046251").unwrap()),
        Scalar::Bloomberg(Bloomberg::new("BBG000B9XRY4").unwrap()),
    ];
    for pair in ascending.windows(2) {
        assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
        assert_eq!(
            pair[0].cmp(&pair[1]),
            pair[0].dtype().unwrap().cmp(&pair[1].dtype().unwrap()),
            "the values disagree with their datatypes"
        );
    }

    // Two codes whose bytes agree are two values, and their hashes say so.
    let currency = Scalar::Currency(Currency::new("XXX").unwrap());
    let country = Scalar::Country(Country::new("XX").unwrap());
    assert_ne!(currency, country);
    assert_ne!(hash_of(&currency), hash_of(&country));

    // Within one code the text decides, and hash agrees with order.
    let one = Scalar::Currency(Currency::new("EUR").unwrap());
    let other = Scalar::Currency(Currency::new("USD").unwrap());
    assert!(one < other);
    assert_eq!(
        hash_of(&one),
        hash_of(&Scalar::Currency(Currency::new("EUR").unwrap()))
    );
    assert_ne!(hash_of(&one), hash_of(&other));
}

#[test]
fn cross_width_numbers_agree_in_equality_order_and_hash() {
    use std::hash::{Hash, Hasher};

    fn hash_of(value: &Scalar) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    // One value spelled at every width of its kind is one value.
    let groups = [
        vec![
            Scalar::from(7_i8),
            Scalar::from(7_i16),
            Scalar::from(7_i32),
            Scalar::from(7_i64),
            Scalar::from(7_i128),
            Scalar::from(7_u8),
            Scalar::from(7_u16),
            Scalar::from(7_u32),
            Scalar::from(7_u64),
            Scalar::from(7_u128),
        ],
        vec![
            Scalar::from(-3_i8),
            Scalar::from(-3_i16),
            Scalar::from(-3_i32),
            Scalar::from(-3_i64),
            Scalar::from(-3_i128),
        ],
        vec![
            Scalar::from(half::f16::from_f32(1.5)),
            Scalar::from(1.5_f32),
            Scalar::from(1.5_f64),
        ],
        vec![
            Scalar::Decimal32(yggdryl::Decimal32::new(1_250, 2)),
            Scalar::Decimal64(yggdryl::Decimal64::new(12_500, 3)),
            Scalar::d128(125, 1),
            Scalar::d256(i256::from_i128(125), 1),
        ],
    ];
    for group in &groups {
        for value in &group[1..] {
            assert_eq!(&group[0], value, "{value:?}");
            assert_eq!(group[0].cmp(value), std::cmp::Ordering::Equal);
            assert_eq!(hash_of(&group[0]), hash_of(value), "{value:?}");
            assert_eq!(group[0].stable_hash(), value.stable_hash(), "{value:?}");
        }
    }

    // Order reads the logical number, never the width.
    assert!(Scalar::from(-1_i32) < Scalar::from(0_u8));
    assert!(Scalar::from(i128::MIN) < Scalar::from(-1_i8));
    assert!(Scalar::from(-2_i64) < Scalar::from(-1_i8));
    assert!(Scalar::from(u128::MAX) > Scalar::from(i128::MAX));
    assert!(Scalar::from(255_u8) < Scalar::from(256_i16));
    assert!(Scalar::from(half::f16::from_f32(1.0)) < Scalar::from(1.5_f64));
    assert!(Scalar::from(2.5_f32) > Scalar::from(1.5_f64));
    assert!(
        Scalar::Decimal32(yggdryl::Decimal32::new(1_249, 2))
            < Scalar::d256(i256::from_i128(125), 1)
    );
    assert!(Scalar::d128(-1, 0) < Scalar::Decimal64(yggdryl::Decimal64::new(0, 4)));

    // Different kinds stay apart even when their numbers agree.
    assert_ne!(Scalar::from(1_i32), Scalar::from(1.0_f64));
    assert_ne!(Scalar::from(1_i32), Scalar::d128(1, 0));
    assert_ne!(Scalar::from(1.0_f64), Scalar::d128(1, 0));
}

#[test]
fn shape_predicates_answer_without_matching() {
    assert!(Scalar::Null.is_null());
    assert!(Scalar::from(1_i64).is_integer());
    assert!(Scalar::from(1.5).is_number());
    assert!(Scalar::d128(15, 1).is_number());
    assert!(Scalar::d256(i256::from_i128(15), 1).is_number());
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

    let point_wkb = |x: f64, y: f64| {
        let mut bytes = vec![1, 1, 0, 0, 0];
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    };
    let wkb = point_wkb(1.0, 2.0);
    let point = Scalar::Geometry(yggdryl::Geometry::new(wkb.clone()).unwrap());
    assert_eq!(point.kind(), "geometry");

    // The same bytes under the bytes kind are a different value: the kind
    // is part of the identity, exactly as it is for string versus bytes.
    let bytes = Scalar::from(wkb.clone());
    assert_ne!(point, bytes);
    assert_ne!(hash_of(&point), hash_of(&bytes));

    // Within the kind, the bytes compare, and equal values hash equal.
    let equal = Scalar::Geometry(yggdryl::Geometry::new(wkb).unwrap());
    assert_eq!(point, equal);
    assert_eq!(hash_of(&point), hash_of(&equal));
    let later = Scalar::Geometry(yggdryl::Geometry::new(point_wkb(3.0, 4.0)).unwrap());
    assert_eq!(
        point.cmp(&later),
        point.as_bytes().unwrap().cmp(later.as_bytes().unwrap())
    );
}

#[test]
fn the_structural_wire_round_trips_a_geospatial_value() {
    let mut wkb = vec![1_u8, 1, 0, 0, 0];
    wkb.extend_from_slice(&1.0_f64.to_le_bytes());
    wkb.extend_from_slice(&2.0_f64.to_le_bytes());
    let point = Scalar::Geometry(yggdryl::Geometry::new(wkb).unwrap());
    let encoded = serde_json::to_string(&point).unwrap();
    assert!(encoded.contains("\"type\":\"geometry\""), "{encoded}");
    let decoded: Scalar = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, point);
}

#[test]
fn the_structural_wire_validates_interval_layouts() {
    let value =
        Scalar::Interval(yggdryl::Interval::new(0, 1, 2_000_000, TimeUnit::DayTime).unwrap());
    let encoded = serde_json::to_string(&value).unwrap();
    let decoded: Scalar = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, value);

    let malformed =
        r#"{"type":"interval","value":{"months":0,"days":0,"nanoseconds":1,"unit":"day_time"}}"#;
    let message = serde_json::from_str::<Scalar>(malformed)
        .unwrap_err()
        .to_string();
    assert!(message.contains("selected layout"), "{message}");
}

#[test]
fn structural_record_deserialization_canonicalizes_and_rejects_duplicates() {
    let unordered = r#"{"type":"struct","value":{
            "z":{"type":"i8","value":2},
            "a":{"type":"i8","value":1}
        }}"#;
    let record: Scalar = serde_json::from_str(unordered).unwrap();
    assert_eq!(record.keys(), ["a", "z"]);

    let duplicate = r#"{"type":"struct","value":{
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
        vec![Scalar::from(1), Scalar::from(1), Scalar::from(1)],
        vec![
            Scalar::from(Float16::from_f16(half::f16::from_f32(1.0))),
            Scalar::from(Float32::from_f32(1.0)),
            Scalar::from(Float64::from_f64(1.0)),
        ],
        vec![Scalar::d128(100, 2), Scalar::d256(i256::from_i128(10), 1)],
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
    let record = Scalar::from_struct([
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
    let geometry = Scalar::Geometry(yggdryl::Geometry::new(POINT_EMPTY_WKB.as_slice()).unwrap());
    assert_eq!(text.as_str(), Some("AAPL"));
    assert_eq!(text.as_bytes(), None);
    assert_eq!(bytes.as_bytes(), Some(b"AAPL".as_slice()));
    assert_eq!(bytes.as_str(), None);
    assert_eq!(geometry.as_bytes(), Some(POINT_EMPTY_WKB.as_slice()));

    let record = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("active", Scalar::from(true)),
    ])
    .unwrap();
    let json_bytes = record.into_json_bytes().unwrap();
    let json_utf8 = record.into_json().unwrap();
    assert_eq!(json_bytes, json_utf8.as_bytes());
    assert_eq!(yggdryl::json::from_bytes(&json_bytes).unwrap(), record);
}

#[test]
fn time_construction_refuses_zones_its_datatype_cannot_preserve() {
    assert!(Scalar::time64(1, TimeUnit::Microsecond, Timezone::UTC).is_err());
}

#[test]
fn scalar_traits_narrow_an_existing_leaf_without_revalidation() {
    let leaf = Float32::from_f32(1.25);
    let scalar = Value::into_scalar(leaf);

    assert_eq!(Value::dtype(&leaf).unwrap().id(), DataTypeId::Float32);
    assert_eq!(Value::dtype(&leaf).unwrap(), DataType::Float32);
    assert_eq!(<Float32 as Value>::from_scalar(&scalar), Some(&leaf));
    assert_eq!(Value::into_scalar(leaf), scalar);
    assert_eq!(FloatingValue::as_f64(&leaf), 1.25);
    assert_eq!(<Float32 as FloatingValue>::BIT_WIDTH, 32);
}

/// One leaf beside the family variant that shares its name, and what the
/// family owes it: the widening `From<Leaf>`, the scalar variant of the same
/// name, the leaf's own datatype and its rendering.
#[macro_export]
macro_rules! family_leaf {
    ($family:ident :: $variant:ident, $leaf:expr) => {{
        let leaf = $leaf;
        (
            $family::$variant(leaf.clone()),
            $family::from(leaf.clone()),
            yggdryl::Scalar::$variant(leaf.clone()),
            yggdryl::Value::dtype(&leaf).unwrap(),
            leaf.to_string(),
        )
    }};
}

/// One family leaf as [`family_leaf!`] states it: the family variant, the
/// widened leaf, the scalar, the leaf's datatype and its rendering.
pub(crate) type FamilyLeaf<F> = (F, F, Scalar, DataType, String);

/// What a family enum answers for each of its leaves: `from_scalar` narrows
/// the leaf's scalar to that variant, `dtype` and `Display` are the leaf's,
/// `into_scalar` and `From<Family> for Scalar` widen back to the scalar the
/// leaf widens to, `KIND` is the family's, and a scalar of another kind
/// narrows to nothing.
pub(crate) fn assert_family_round_trip<F>(
    leaves: Vec<FamilyLeaf<F>>,
    kind: DataTypeKind,
    other: &Scalar,
) where
    F: FamilyValue,
    Scalar: From<F>,
{
    assert_eq!(F::KIND, kind);
    for (held, widened, scalar, dtype, display) in leaves {
        assert_eq!(widened, held, "{scalar:?}");
        assert_eq!(F::from_scalar(&scalar), Some(held.clone()), "{scalar:?}");
        assert_eq!(scalar.family(), kind, "{scalar:?}");
        assert_eq!(held.dtype().unwrap(), dtype, "{scalar:?}");
        assert_eq!(held.to_string(), display, "{scalar:?}");
        assert_eq!(held.clone().into_scalar(), scalar, "{scalar:?}");
        assert_eq!(Scalar::from(held), scalar, "{scalar:?}");
    }
    assert_ne!(other.family(), kind, "{other:?}");
    assert_eq!(F::from_scalar(other), None, "{other:?}");
    assert_eq!(F::from_scalar(&Scalar::Null), None);
}

#[test]
fn the_integer_family_stands_for_every_width() {
    assert_family_round_trip(
        vec![
            family_leaf!(Integer::Int8, Int8::new(-8)),
            family_leaf!(Integer::Int16, Int16::new(-16)),
            family_leaf!(Integer::Int32, Int32::new(-32)),
            family_leaf!(Integer::Int64, Int64::new(-64)),
            family_leaf!(Integer::UInt8, UInt8::new(8)),
            family_leaf!(Integer::UInt16, UInt16::new(16)),
            family_leaf!(Integer::UInt32, UInt32::new(32)),
            family_leaf!(Integer::UInt64, UInt64::new(64)),
            family_leaf!(Integer::Int128, Int128::new(i128::MIN)),
            family_leaf!(Integer::UInt128, UInt128::new(u128::MAX)),
        ],
        DataTypeKind::Integer,
        &Scalar::from(1.5_f64),
    );
}

#[test]
fn the_floating_family_stands_for_every_width() {
    assert_family_round_trip(
        vec![
            family_leaf!(
                Floating::Float16,
                Float16::from_f16(half::f16::from_f32(1.5))
            ),
            family_leaf!(Floating::Float32, Float32::from_f32(1.25)),
            family_leaf!(Floating::Float64, Float64::from_f64(0.1)),
        ],
        DataTypeKind::Floating,
        &Scalar::from(1_i64),
    );
}

#[test]
fn every_family_accessor_answers_its_own_kind_and_no_other() {
    let mut point = vec![1, 1, 0, 0, 0];
    point.extend_from_slice(&1.5_f64.to_le_bytes());
    point.extend_from_slice(&2.5_f64.to_le_bytes());
    // One value of every kind, so each accessor meets every other kind.
    let values = [
        Scalar::Null,
        Scalar::from(true),
        Scalar::from(7_i32),
        Scalar::from(u128::MAX),
        Scalar::from(1.5_f32),
        Scalar::d128(125, 1),
        Scalar::date32(1),
        Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
        Scalar::from("text"),
        Scalar::Currency(yggdryl::Currency::new("EUR").unwrap()),
        Scalar::from(vec![1_u8, 2]),
        Scalar::Uuid(yggdryl::Uuid::from_bytes(b"550e8400-e29b-41d4-a716-446655440000").unwrap()),
        Scalar::Timezone(Timezone::UTC),
        Scalar::Geometry(yggdryl::Geometry::new(point.clone()).unwrap()),
        Scalar::Geography(yggdryl::Geography::new(point).unwrap()),
        Scalar::from_sequence([Scalar::from(1_i64)]),
        Scalar::from_mapping([(Scalar::from("k"), Scalar::from(1_i64))]).unwrap(),
        Scalar::from_struct([("id", Scalar::from(1_i64))]).unwrap(),
    ];

    // An accessor is the family's own narrowing: it answers exactly where
    // the scalar's kind is the family's, and what it answers widens back to
    // the scalar it read.
    macro_rules! answers_its_own_kind {
        ($accessor:ident, $family:ident) => {
            for value in &values {
                let held = value.$accessor();
                assert_eq!(held, $family::from_scalar(value), "{value:?}");
                assert_eq!(
                    held.is_some(),
                    value.family() == DataTypeKind::$family,
                    "{value:?}"
                );
                if let Some(held) = held {
                    assert_eq!(Scalar::from(held), *value, "{value:?}");
                }
            }
        };
    }
    answers_its_own_kind!(as_integer, Integer);
    answers_its_own_kind!(as_floating, Floating);
    answers_its_own_kind!(as_temporal, Temporal);
    answers_its_own_kind!(as_code, Code);
    answers_its_own_kind!(as_geospatial, Geospatial);
    answers_its_own_kind!(as_nested, Nested);

    // The decimal family narrows through its own `from_scalar`; `as_decimal`
    // is the coefficient-and-scale reader.
    let price = Scalar::d128(125, 1);
    assert_eq!(price.as_decimal(), Some((i256::from_i128(125), 1)));
    assert!(matches!(
        yggdryl::Decimal::from_scalar(&price),
        Some(yggdryl::Decimal::Decimal128(_))
    ));
    assert_eq!(Scalar::from(7_i32).as_decimal(), None);
}

#[test]
fn a_temporal_names_its_family_as_a_datatype_spells_it() {
    // The five words are the datatypes' own, so the family read off a value
    // is the word a column of it declares.
    let cases = [
        (
            Temporal::from(Date32::new(1, TimeUnit::Day, Timezone::NAIVE).unwrap()),
            "date",
        ),
        (
            Temporal::from(
                Date64::new(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            ),
            "date",
        ),
        (
            Temporal::from(Time32::new(1, TimeUnit::Second, Timezone::NAIVE).unwrap()),
            "time",
        ),
        (
            Temporal::from(Time64::new(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()),
            "time",
        ),
        (
            Temporal::from(DateTime64::new(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap()),
            "datetime",
        ),
        (
            Temporal::from(Duration32::new(-1, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()),
            "duration",
        ),
        (
            Temporal::from(Duration64::new(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()),
            "duration",
        ),
        (
            Temporal::from(Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap()),
            "interval",
        ),
    ];
    for (held, family) in &cases {
        assert_eq!(held.family(), *family, "{held:?}");
        assert_eq!(
            Scalar::from(held.clone())
                .as_temporal()
                .map(|held| held.family()),
            Some(*family),
            "{held:?}"
        );
    }

    // Each leaf type states the same word as a constant.
    assert_eq!(Date32::FAMILY, "date");
    assert_eq!(Date64::FAMILY, "date");
    assert_eq!(Time32::FAMILY, "time");
    assert_eq!(Time64::FAMILY, "time");
    assert_eq!(DateTime64::FAMILY, "datetime");
    assert_eq!(Duration32::FAMILY, "duration");
    assert_eq!(Duration64::FAMILY, "duration");
    assert_eq!(Interval::FAMILY, "interval");
}

#[test]
fn every_scalar_family_exposes_its_leaf_contract() {
    use yggdryl::{
        CodeValue, DecimalValue, GeospatialValue, IntegerValue, NestedValue, TemporalValue,
    };
    use yggdryl::{bytes, decimal, geospatial, integer, sequence, string, time, uuid};

    let integer = integer::UInt128::new(u128::MAX);
    assert_eq!(IntegerValue::as_i128(&integer), None);
    assert_eq!(IntegerValue::as_u128(&integer), Some(u128::MAX));
    assert_eq!(Value::dtype(&integer).unwrap().id(), DataTypeId::Decimal256);

    let decimal = decimal::Decimal32::new(1_251, 2);
    assert_eq!(DecimalValue::coefficient(&decimal), i256::from_i128(1_251));
    assert_eq!(
        DecimalValue::rescale(decimal, 3).unwrap().coefficient(),
        12_510
    );
    assert!(DecimalValue::rescale(decimal, 1).is_err());

    let time = time::Time32::new(2, TimeUnit::Second, Timezone::NAIVE).unwrap();
    let milliseconds = TemporalValue::with_unit(time, TimeUnit::Millisecond).unwrap();
    assert_eq!(milliseconds.count(), 2_000);
    assert_eq!(milliseconds.unit(), TimeUnit::Millisecond);

    let text = string::Str::new("AAPL")
        .try_with_parameters(string::StringType::LargeUtf8String)
        .unwrap();
    assert_eq!(text.as_str(), "AAPL");
    assert_eq!(Value::dtype(&text).unwrap(), DataType::large_utf8());

    let bytes = bytes::Bytes::new([1, 2, 3])
        .try_with_parameters(bytes::BytesType::BinaryView)
        .unwrap();
    assert_eq!(bytes.as_bytes(), [1, 2, 3]);
    assert_eq!(Value::dtype(&bytes).unwrap(), DataType::binary_view());

    let currency = yggdryl::Currency::new("USD").unwrap();
    assert_eq!(<yggdryl::Currency as CodeValue>::WIDTH, 3);
    assert_eq!(CodeValue::as_str(&currency), "USD");
    assert_eq!(Value::dtype(&currency).unwrap(), DataType::Currency);

    let geometry = geospatial::Geometry::new(POINT_EMPTY_WKB.as_slice()).unwrap();
    assert_eq!(
        GeospatialValue::as_bytes(&geometry),
        POINT_EMPTY_WKB.as_slice()
    );
    assert_eq!(Value::dtype(&geometry).unwrap().id(), DataTypeId::Geometry);

    let sequence = sequence::Sequence::new(Arc::from([Scalar::from(1_i32), Scalar::from(2_i32)]));
    assert_eq!(NestedValue::len(&sequence), 2);
    assert_eq!(NestedValue::children(&sequence).count(), 2);
    assert_eq!(Value::dtype(&sequence).unwrap().id(), DataTypeId::List);

    let uuid = uuid::Uuid::from_bytes(b"550e8400-e29b-41d4-a716-446655440000").unwrap();
    assert_eq!(Value::dtype(&uuid).unwrap(), DataType::Uuid);

    let scalar = Value::into_scalar(text);
    assert_eq!(scalar.id(), DataTypeId::LargeUtf8String);
    assert_eq!(scalar.family(), DataTypeKind::Text);
}

#[test]
fn concrete_leaves_preserve_their_physical_identity() {
    use yggdryl::{
        bytes, date, datetime, decimal, geospatial, integer, mapping, sequence, string, structure,
        uuid,
    };

    let integer = integer::Int32::new(-7);
    assert_eq!(integer.get(), -7);
    assert_eq!(integer.to_string(), "-7");

    let decimal = decimal::Decimal32::new(1_250, 2);
    assert_eq!(decimal.coefficient(), 1_250);
    assert_eq!(decimal.scale(), 2);
    assert_eq!(decimal.to_string(), "12.50");

    let datetime = datetime::DateTime64::new(7, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
    assert_eq!(datetime.count(), 7);
    assert_eq!(datetime.unit(), TimeUnit::Nanosecond);
    assert_eq!(datetime.timezone(), Timezone::UTC);
    assert!(date::Date32::new(0, TimeUnit::Second, Timezone::NAIVE).is_err());

    let utf8 = string::Str::new("東京");
    let view = utf8
        .clone()
        .try_with_parameters(string::StringType::Utf8StringView)
        .unwrap();
    assert_eq!(utf8.as_str(), view.as_str());
    assert_eq!(utf8.parameters(), string::StringType::Utf8String);
    assert_eq!(view.parameters(), string::StringType::Utf8StringView);
    assert_eq!(
        serde_json::from_str::<string::Str>(&serde_json::to_string(&utf8).unwrap()).unwrap(),
        utf8
    );

    let ascii = string::Str::new("FIX")
        .try_with_parameters(string::StringType::AsciiString)
        .unwrap();
    let currency = yggdryl::Currency::new("USD").unwrap();
    assert_eq!(ascii.as_str(), "FIX");
    assert_eq!(ascii.charset(), yggdryl::Charset::Ascii);
    assert_eq!(currency.as_str(), "USD");
    assert!(
        string::Str::new("café")
            .try_with_parameters(string::StringType::AsciiString)
            .is_err()
    );
    assert!(
        string::Str::new("")
            .try_with_parameters(string::StringType::FixedAsciiString(0))
            .is_err()
    );
    assert!(yggdryl::Cfi::new("TOO-LONG").is_err());

    let binary = bytes::Bytes::from(vec![0, 1, 0xff]);
    let binary_view = binary
        .clone()
        .try_with_parameters(bytes::BytesType::BinaryView)
        .unwrap();
    assert_eq!(binary.as_bytes(), binary_view.as_bytes());
    assert_eq!(binary.to_string(), "0001ff");

    let uuid = uuid::Uuid::from_bytes(b"550e8400-e29b-41d4-a716-446655440000").unwrap();
    assert_eq!(uuid.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    assert_eq!(uuid::Uuid::from_bytes(&uuid.into_bytes()).unwrap(), uuid);

    let mut point = vec![1, 1, 0, 0, 0];
    point.extend_from_slice(&1.5_f64.to_le_bytes());
    point.extend_from_slice(&2.5_f64.to_le_bytes());
    let geometry = geospatial::Geometry::new(point.clone()).unwrap();
    assert_eq!(geometry.as_bytes(), point);
    assert!(geospatial::Geography::new(vec![0xff]).is_err());

    let values = sequence::Sequence::new(Arc::from([Scalar::from(1_i32), Scalar::from("one")]));
    assert_eq!(values.as_slice().len(), 2);
    let mapping = mapping::Mapping::Map(mapping::Map::new(Arc::from([(
        Scalar::from("one"),
        Scalar::from(1_i32),
    )])));
    assert_eq!(mapping.as_slice().len(), 1);
    let record = structure::Struct::new(Arc::new(std::collections::BTreeMap::from([(
        "one".into(),
        Scalar::from(1_i32),
    )])));
    assert_eq!(record.as_map().get("one"), Some(&Scalar::from(1_i32)));
}

#[test]
fn width_variants_keep_exact_members_and_logical_identity() {
    use yggdryl::{bytes, datetime, decimal, geospatial, integer, sequence, string};

    let signed = Scalar::Int32(integer::Int32::new(7));
    let unsigned = Scalar::UInt8(integer::UInt8::new(7));
    assert_eq!(signed, unsigned);
    assert_eq!(signed, Scalar::from(7_i32));
    assert_eq!(signed.kind(), "i32");
    assert_eq!(unsigned.kind(), "u8");

    let narrow = Scalar::Float32(Float32::from_f32(1.25));
    let wide = Scalar::Float64(Float64::from_f64(1.25));
    assert_eq!(narrow, wide);

    let narrow = Scalar::Decimal32(decimal::Decimal32::new(1_250, 2));
    let wide = Scalar::Decimal256(decimal::Decimal256::new(i256::from_i128(125), 1));
    assert_eq!(narrow, wide);
    assert_eq!(narrow.as_decimal(), Some((i256::from_i128(1_250), 2)));
    assert_eq!(wide.as_decimal(), Some((i256::from_i128(125), 1)));

    let utf8 = string::Str::new("same");
    let large = utf8
        .clone()
        .try_with_parameters(string::StringType::LargeUtf8String)
        .unwrap();
    assert_eq!(utf8, large);

    let binary = bytes::Bytes::from(vec![1, 2]);
    let view = binary
        .clone()
        .try_with_parameters(bytes::BytesType::BinaryView)
        .unwrap();
    assert_eq!(binary, view);

    // A code carries its identity: two codes whose bytes agree are two
    // values, and neither is the string spelling the same bytes.
    let side = Scalar::Side(yggdryl::Side::new("BUY").unwrap());
    let time_in_force = Scalar::TimeInForce(yggdryl::TimeInForce::new("BUY").unwrap());
    assert_ne!(side, time_in_force);
    assert_eq!(side.as_str(), time_in_force.as_str());
    assert_ne!(side, Scalar::from("BUY"));

    let mut point = vec![1, 1, 0, 0, 0];
    point.extend_from_slice(&1.5_f64.to_le_bytes());
    point.extend_from_slice(&2.5_f64.to_le_bytes());
    let geometry = Scalar::Geometry(geospatial::Geometry::new(point.clone()).unwrap());
    let geography = Scalar::Geography(geospatial::Geography::new(point).unwrap());
    assert_eq!(geometry, geography);

    let sequence = Scalar::Sequence(sequence::Sequence::new(Arc::from([
        Scalar::from(1_i32),
        Scalar::from(2_i32),
    ])));
    assert_eq!(sequence.len(), 2);
    assert!(!sequence.is_empty());
    assert!(sequence.is_container());

    let datetime = Scalar::DateTime64(
        datetime::DateTime64::new(7, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
    );
    assert_eq!(
        datetime.as_temporal().map(|held| held.family()),
        Some("datetime")
    );
    assert!(matches!(
        datetime.as_temporal(),
        Some(Temporal::DateTime64(_))
    ));
    assert_eq!(datetime.kind(), "datetime64");
    assert_eq!(datetime.temporal_timezone(), Some(Timezone::UTC));

    let encoded = serde_json::to_string(&sequence).unwrap();
    assert_eq!(serde_json::from_str::<Scalar>(&encoded).unwrap(), sequence);
}

#[test]
fn every_width_leaf_round_trips_under_its_unchanged_tag() {
    let cases = [
        (Scalar::from(-8_i8), "i8"),
        (Scalar::from(-16_i16), "i16"),
        (Scalar::from(-32_i32), "i32"),
        (Scalar::from(-64_i64), "i64"),
        (Scalar::from(8_u8), "u8"),
        (Scalar::from(16_u16), "u16"),
        (Scalar::from(32_u32), "u32"),
        (Scalar::from(64_u64), "u64"),
        (Scalar::from(i128::MIN), "i128"),
        (Scalar::from(u128::MAX), "u128"),
        (Scalar::from(half::f16::from_f32(1.5)), "f16"),
        (Scalar::from(1.25_f32), "f32"),
        (Scalar::from(0.1_f64), "f64"),
        (
            Scalar::Decimal32(yggdryl::decimal::Decimal32::new(1_250, 2)),
            "d32",
        ),
        (
            Scalar::Decimal64(yggdryl::decimal::Decimal64::new(-7, 1)),
            "d64",
        ),
        (Scalar::d128(125, 1), "d128"),
        (Scalar::d256(i256::from_i128(-125), 3), "d256"),
        (Scalar::date32(19_000), "date32"),
        (Scalar::date64(86_400_000), "date64"),
        (
            Scalar::time32(5, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            "time32",
        ),
        (
            Scalar::time64(6, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            "time64",
        ),
        (
            Scalar::datetime64(7, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            "datetime64",
        ),
        (
            Scalar::duration32(8, TimeUnit::Second).unwrap(),
            "duration32",
        ),
        (
            Scalar::duration64(9, TimeUnit::Nanosecond).unwrap(),
            "duration64",
        ),
        (
            Scalar::Interval(yggdryl::Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap()),
            "interval",
        ),
        (Scalar::from_sequence([Scalar::from(1_i32)]), "sequence"),
        (
            Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i32))]).unwrap(),
            "mapping",
        ),
        (
            Scalar::from_struct([("a", Scalar::from(1_i32))]).unwrap(),
            "struct",
        ),
    ];
    assert_eq!(cases.len(), 28);
    for (value, tag) in &cases {
        assert_eq!(value.kind(), *tag);
        // Text, not `serde_json::Value`: a 128-bit integer does not fit the
        // latter's number.
        let encoded = serde_json::to_string(value).unwrap();
        assert!(
            encoded.starts_with(&format!(r#"{{"type":"{tag}","value":"#)),
            "{encoded}"
        );
        let decoded: Scalar = serde_json::from_str(&encoded).unwrap();
        assert_eq!(&decoded, value, "{encoded}");
        // Equality normalizes scale and unit, so the exact payload is pinned too.
        assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
        assert_eq!(format!("{decoded:?}"), format!("{value:?}"));
        assert_eq!(decoded.kind(), *tag, "{encoded}");
    }
}

/// An iterator that reports no children and yields two.
///
/// `Iterator::size_hint` is a hint, and a nested value is built from whatever
/// a caller's iterator hands over, so the emptiness a bound claims is never
/// what decides the children a value keeps.
struct UnderReporting(std::vec::IntoIter<Scalar>);

impl Iterator for UnderReporting {
    type Item = Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}

#[test]
fn nested_children_come_from_the_iterator_and_not_from_its_bound() {
    let children = || UnderReporting(vec![Scalar::from("AAPL"), Scalar::from(12_i64)].into_iter());
    assert_eq!(
        Scalar::from_sequence(children()),
        Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(12_i64)])
    );

    let entries = Scalar::from_mapping(children().map(|value| (value, Scalar::Null))).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries.get_key_str("AAPL"), Some(&Scalar::Null));

    // An empty run still answers with the one shared value.
    assert!(Scalar::from_sequence([]).is_empty());
    assert!(Scalar::from_mapping([]).unwrap().is_empty());
}

#[test]
fn truthiness_reads_absence_zero_and_emptiness_as_false() {
    // Falsy is absence, zero at every width, empty text or bytes, and a
    // container with nothing truthy in it.
    for falsy in [
        Scalar::Null,
        Scalar::from(false),
        Scalar::from(0_i32),
        Scalar::from(0_u64),
        Scalar::from(0.0_f64),
        Scalar::from(-0.0_f64),
        Scalar::d128(0, 4),
        Scalar::from(""),
        Scalar::from("   "),
        Scalar::from(Arc::from(b"".as_slice())),
        Scalar::from_sequence([]),
        Scalar::from_mapping([]).unwrap(),
        Scalar::from_struct(Vec::<(&str, Scalar)>::new()).unwrap(),
    ] {
        assert!(!falsy.is_truthy(), "{falsy:?} should read false");
    }

    for truthy in [
        Scalar::from(true),
        Scalar::from(1_i32),
        Scalar::from(-1_i64),
        Scalar::from(f64::NAN),
        Scalar::d128(1, 4),
        Scalar::from("0.0"),
        Scalar::from("anything"),
        Scalar::from(Arc::from(b"\0".as_slice())),
        Scalar::from_sequence([Scalar::from(1)]),
    ] {
        assert!(truthy.is_truthy(), "{truthy:?} should read true");
    }
}

#[test]
fn truthiness_reads_the_text_a_column_spells_false_with() {
    // Wider than Python on purpose: text arrives from CSV, FIX and query
    // strings, where a column that spells false is not asking to be read true.
    for spelling in ["false", "FALSE", "False", " no ", "OFF", "f", "N", "0"] {
        assert!(
            !Scalar::from(spelling).is_truthy(),
            "{spelling:?} should read false"
        );
    }
    for spelling in ["true", "yes", "on", "1", "00", "falsey", "n/a"] {
        assert!(
            Scalar::from(spelling).is_truthy(),
            "{spelling:?} should read true"
        );
    }

    // The cast reader stays strict - this coercion does not widen it.
    assert!(
        yggdryl::DataType::Boolean
            .scalar(Scalar::from("off"))
            .is_err()
    );
    assert_eq!(
        yggdryl::DataType::Boolean
            .scalar(Scalar::from("false"))
            .unwrap(),
        Scalar::from(false)
    );
}

#[test]
fn a_container_of_empty_values_is_itself_empty() {
    // The "struct all empty values" case: `is_empty` counts entries, so a
    // record of three nulls is not empty - but nothing in it is set.
    let all_null = Scalar::from_struct(vec![
        ("a", Scalar::Null),
        ("b", Scalar::from("")),
        ("c", Scalar::from_sequence([Scalar::Null])),
    ])
    .unwrap();
    assert!(!all_null.is_empty(), "it has three fields");
    assert!(!all_null.is_truthy(), "none of them is set");

    let one_set = Scalar::from_struct(vec![("a", Scalar::Null), ("b", Scalar::from(1))]).unwrap();
    assert!(one_set.is_truthy());
}

#[test]
fn addition_joins_a_repertoire_that_has_no_sum() {
    // Text, bytes and sequences have no sum, so `+` joins them.
    assert_eq!(
        (Scalar::from("AA") + Scalar::from("PL")).unwrap(),
        Scalar::from("AAPL")
    );
    assert_eq!(
        (Scalar::from(Arc::from(b"\x01".as_slice())) + Scalar::from(Arc::from(b"\x02".as_slice())))
            .unwrap(),
        Scalar::from(Arc::from(b"\x01\x02".as_slice()))
    );
    assert_eq!(
        (Scalar::from_sequence([Scalar::from(1)]) + Scalar::from_sequence([Scalar::from(2)]))
            .unwrap(),
        Scalar::from_sequence([Scalar::from(1), Scalar::from(2)])
    );

    // A code joins as the text it is, and stops being a code: `FR` and `X`
    // concatenated are not a country.
    let country = yggdryl::DataType::Country
        .scalar(Scalar::from("FR"))
        .unwrap();
    let joined = (country + Scalar::from("X")).unwrap();
    assert_eq!(joined, Scalar::from("FRX"));
    assert_eq!(joined.id(), yggdryl::DataTypeId::Utf8String);

    // Only `+`. Nothing else names anything a reader would agree on.
    for refused in [
        Scalar::from("a") - Scalar::from("b"),
        Scalar::from("a") * Scalar::from("b"),
        Scalar::from("a") / Scalar::from("b"),
    ] {
        assert!(refused.is_err());
    }

    // Two repertoires do not join, and a null still answers null.
    assert!((Scalar::from("a") + Scalar::from(1)).is_err());
    assert_eq!((Scalar::from("a") + Scalar::Null).unwrap(), Scalar::Null);
}

#[test]
fn two_wkb_payloads_are_not_a_geometry() {
    // Geospatial values read as bytes, so an untyped join would have accepted
    // them. Laying two WKB payloads end to end does not make a geometry.
    let point = yggdryl::DataType::geometry(None)
        .unwrap()
        .default_value()
        .unwrap();
    assert!(point.as_bytes().is_some(), "it does read as bytes");
    assert!((point.clone() + point).is_err());
}
