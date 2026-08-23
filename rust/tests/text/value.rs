use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use yggdryl::{I256, Scalar, TimeUnit, Timezone};

/// One value of every kind, in the order [`Scalar`]'s total ordering puts them.
///
/// Every test that has to be exhaustive over the value model reads this list,
/// so a new variant is added here once rather than forgotten in three places.
fn one_of_every_kind() -> Vec<Scalar> {
    vec![
        Scalar::Null,
        Scalar::from(true),
        Scalar::I128(i128::MIN),
        Scalar::from(-8_i8),
        Scalar::from(-4_i16),
        Scalar::from(-2_i32),
        Scalar::I64(-1),
        Scalar::from(1_u8),
        Scalar::from(2_u16),
        Scalar::from(3_u32),
        Scalar::U64(u64::MAX),
        Scalar::U128(u128::MAX),
        Scalar::from(half::f16::from_f32(1.0)),
        Scalar::from(1.25_f32),
        Scalar::from(1.5),
        Scalar::d128(-1_050, 2),
        Scalar::d256(I256::from_i128(1_050), 2),
        Scalar::from("AAPL"),
        Scalar::from(b"\x00\xff".as_slice()),
        Scalar::date32(19_723),
        Scalar::date64(1_704_067_200_000),
        Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::time64(2_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        Scalar::datetime64_in(1_700_000_000, TimeUnit::Second, "Europe/Paris").unwrap(),
        Scalar::duration32(1, TimeUnit::Second).unwrap(),
        Scalar::duration64(2, TimeUnit::Second).unwrap(),
        Scalar::from_sequence([Scalar::Null]),
        Scalar::from_mapping([(Scalar::from("k"), Scalar::Null)]).unwrap(),
        Scalar::from_record([("k", Scalar::Null)]).unwrap(),
        Scalar::Geospatial([1_u8, 1, 0, 0, 0].as_slice().into()),
    ]
}

#[test]
fn structural_serde_reads_back_every_variant() {
    // The hand-written `Deserialize` mirrors `Scalar` variant for variant, and a
    // variant missing from the mirror is not a compile error - it is data serde
    // silently refuses to read. This is the check that makes it loud.
    let naive = Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap();
    for value in one_of_every_kind().into_iter().chain([naive]) {
        let encoded = serde_json::to_vec(&value).unwrap();
        let decoded = serde_json::from_slice::<Scalar>(&encoded).unwrap();
        assert_eq!(decoded, value, "{} did not survive serde", value.kind());
        assert_eq!(decoded.kind(), value.kind());
    }
}

#[test]
fn every_kind_has_its_own_place_in_the_total_ordering() {
    // The ordering key is wire-visible: it decides the order of an Arrow
    // dictionary's values. Kinds must therefore separate, and every kind must
    // have a distinct name for an error message to be readable.
    let values = one_of_every_kind();
    for window in values.windows(2) {
        let (left, right) = (&window[0], &window[1]);
        assert!(left <= right, "{left:?} sorts after {right:?}");
    }

    // Kinds arrive in runs rather than interleaved, so a value of one kind
    // never sorts between two of another. The integer widths are the
    // deliberate exception: they are one number line spelled ten ways, and
    // the three float widths are another, spelled three ways.
    let mut kinds = values.iter().map(Scalar::kind).collect::<Vec<_>>();
    kinds.dedup();
    assert_eq!(
        kinds.len(),
        values.len(),
        "every kind spells its own name: {kinds:?}"
    );
}

#[test]
fn null_answers_absence_everywhere_a_value_is_read() {
    // Null is a value, not a trap: every accessor answers None, every
    // container helper answers emptiness, and only the documented panicking
    // Index operators are allowed to insist.
    let null = Scalar::Null;
    assert!(null.is_null());
    assert!(null.as_bool().is_none());
    assert!(null.as_i64().is_none());
    assert!(null.as_u64().is_none());
    assert!(null.as_i128().is_none());
    assert!(null.as_u128().is_none());
    assert!(null.as_f32().is_none());
    assert!(null.as_f64().is_none());
    assert!(null.as_f16().is_none());
    assert!(null.as_str().is_none());
    assert!(null.as_utf8().is_none());
    assert!(null.as_bytes().is_none());
    assert!(null.as_date32().is_none());
    assert!(null.as_date64().is_none());
    assert!(null.as_time32().is_none());
    assert!(null.as_time64().is_none());
    assert!(null.as_datetime64().is_none());
    assert!(null.as_duration32().is_none());
    assert!(null.as_duration64().is_none());
    assert!(null.as_d128().is_none());
    assert!(null.as_d256().is_none());
    assert!(null.as_sequence().is_none());
    assert!(null.as_mapping().is_none());
    assert!(null.as_record().is_none());
    assert!(!null.is_integer() && !null.is_number() && !null.is_temporal());

    assert_eq!(null.len(), 0);
    assert!(null.get(0).is_none());
    assert!(null.get_key_str("k").is_none());
    assert!(null.path("a.0.b").is_none());
    assert_eq!(null.iter().count(), 0);
    assert_eq!(null.entries().count(), 0);
    assert_eq!(null.record_iter().count(), 0);
    assert!(null.keys().is_empty());
    assert!(!null.contains_key("k"));

    // Rebuilding something that is not a mapping is an error, not a panic.
    assert!(null.with_key("k", Scalar::from(1_i64)).is_err());
    assert!(null.without_key("k").is_err());

    // A null inside a container reads back as the absence it is.
    let row = Scalar::from_mapping([(Scalar::from("gap"), Scalar::Null)]).unwrap();
    assert!(row.get_key_str("gap").is_some_and(Scalar::is_null));
    let fallback = Scalar::from(7_i64);
    assert_eq!(row.get_or("gap", &fallback), &fallback);
}

#[test]
fn every_accessor_tolerates_every_kind() {
    // No accessor is allowed to panic on a kind it does not read - the wrong
    // kind is None, never an abort. Exercising the full matrix is what keeps
    // a new variant from shipping an accessor that insists.
    for value in one_of_every_kind().into_iter().chain([Scalar::Null]) {
        let _ = value.as_bool();
        let _ = value.as_i64();
        let _ = value.as_u64();
        let _ = value.as_i128();
        let _ = value.as_u128();
        let _ = value.as_f32();
        let _ = value.as_f64();
        let _ = value.as_f16();
        let _ = value.as_str();
        let _ = value.as_utf8();
        let _ = value.as_bytes();
        let _ = value.as_date32();
        let _ = value.as_date64();
        let _ = value.as_time32();
        let _ = value.as_time64();
        let _ = value.as_datetime64();
        let _ = value.as_duration32();
        let _ = value.as_duration64();
        let _ = value.as_d128();
        let _ = value.as_d256();
        let _ = value.as_sequence();
        let _ = value.as_mapping();
        let _ = value.as_record();
        let _ = value.record_iter().count();
        let _ = value.as_json_bytes();
        let _ = value.as_json_utf8();
        let _ = value.len();
        let _ = value.get(0);
        let _ = value.get_key_str("k");
        let _ = value.path("a.b");
        let _ = value.iter().count();
        let _ = value.kind();
        let _ = value.data_type();
    }
}

#[test]
fn integer_widths_have_native_numeric_equality() {
    assert_eq!(Scalar::I64(1), Scalar::U64(1));
    assert_eq!(Scalar::I128(1), Scalar::U128(1));
    assert_eq!(Scalar::from(1_i8), Scalar::from(1_u32));
    assert_ne!(Scalar::I64(-1), Scalar::U64(1));
}

#[test]
fn float_widths_are_one_number_line() {
    // An `f32` widens to `f64` exactly, so the same reading at either width
    // is one value - as the integers are one number line across widths.
    assert_eq!(Scalar::from(1.5_f32), Scalar::from(1.5_f64));
    assert!(Scalar::from(1.25_f32) < Scalar::from(1.5_f64));
    assert!(Scalar::from(2.0_f64) < Scalar::from(2.5_f32));
    assert_ne!(Scalar::from(0.1_f32), Scalar::from(0.1_f64));
}

#[test]
fn equal_integer_representations_hash_and_order_equally() {
    fn hash(value: &Scalar) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    let values = [
        Scalar::I64(1),
        Scalar::U64(1),
        Scalar::I128(1),
        Scalar::U128(1),
    ];
    for left in &values {
        for right in &values {
            assert_eq!(left, right);
            assert_eq!(left.cmp(right), std::cmp::Ordering::Equal);
            assert_eq!(hash(left), hash(right));
        }
    }
    assert!(Scalar::I128(i128::MIN) < Scalar::I64(-1));
    assert!(Scalar::I64(-1) < Scalar::U128(u128::MAX));
}

#[test]
fn structural_serde_preserves_all_float_values() {
    for value in [0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let value = Scalar::from(value);
        let encoded = serde_json::to_vec(&value).unwrap();
        assert_eq!(serde_json::from_slice::<Scalar>(&encoded).unwrap(), value);
    }
}

#[test]
fn structural_serde_rejects_duplicate_mapping_keys() {
    let encoded = br#"{
        "type":"mapping",
        "value":[
            [{"type":"string","value":"a"},{"type":"null"}],
            [{"type":"string","value":"a"},{"type":"bool","value":true}]
        ]
    }"#;
    assert!(serde_json::from_slice::<Scalar>(encoded).is_err());
}

#[test]
fn wide_mapping_constructor_rejects_duplicates() {
    let mut entries = (0_u64..128)
        .map(|index| (Scalar::from(index), Scalar::from(index)))
        .collect::<Vec<_>>();
    entries.push((Scalar::I128(64), Scalar::Null));
    assert!(Scalar::from_mapping(entries).is_err());
}

#[test]
fn collection_iteration_matches_python_sequence_and_mapping_semantics() {
    let sequence = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(
        (&sequence).into_iter().collect::<Vec<_>>(),
        vec![&sequence[0], &sequence[1]]
    );

    let mapping = Scalar::from_mapping([
        (Scalar::from("a"), Scalar::from(1_i64)),
        (Scalar::from("b"), Scalar::from(2_i64)),
    ])
    .unwrap();
    assert_eq!(mapping["a"], Scalar::I64(1));
    assert_eq!(
        mapping
            .iter()
            .filter_map(Scalar::as_str)
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn empty_collections_share_process_wide_backing() {
    let left = Scalar::from(Vec::<u8>::new());
    let encoded = serde_json::to_vec(&left).unwrap();
    let right: Scalar = serde_json::from_slice(&encoded).unwrap();
    let (Scalar::Bytes(left), Scalar::Bytes(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));

    let left = Scalar::from_sequence([]);
    let right = Scalar::from_sequence([]);
    let (Scalar::Sequence(left), Scalar::Sequence(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));

    let left = Scalar::from_mapping([]).unwrap();
    let right = Scalar::from_mapping([]).unwrap();
    let (Scalar::Mapping(left), Scalar::Mapping(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));

    let left = Scalar::from_record(std::iter::empty::<(&str, Scalar)>()).unwrap();
    let right = Scalar::from_record(std::iter::empty::<(&str, Scalar)>()).unwrap();
    let (Scalar::Record(left), Scalar::Record(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));
    assert!(!Scalar::Null.is_empty());
}
