use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use yggdryl::{TimeUnit, Value};

/// One value of every kind, in the order [`Value`]'s total ordering puts them.
///
/// Every test that has to be exhaustive over the value model reads this list,
/// so a new variant is added here once rather than forgotten in three places.
fn one_of_every_kind() -> Vec<Value> {
    vec![
        Value::Null,
        Value::from(true),
        Value::I128(i128::MIN),
        Value::from(-8_i8),
        Value::from(-4_i16),
        Value::from(-2_i32),
        Value::I64(-1),
        Value::from(1_u8),
        Value::from(2_u16),
        Value::from(3_u32),
        Value::U64(u64::MAX),
        Value::U128(u128::MAX),
        Value::from(1.25_f32),
        Value::from(1.5),
        Value::decimal(-1_050, 2),
        Value::from("AAPL"),
        Value::from(b"\x00\xff".as_slice()),
        Value::date(19_723),
        Value::time(45_296_000_000, TimeUnit::Microsecond),
        Value::timestamp(1_700_000_000, TimeUnit::Second, Some("Europe/Paris")).unwrap(),
        Value::duration(90, TimeUnit::Second),
        Value::from_sequence([Value::Null]),
        Value::from_mapping([(Value::from("k"), Value::Null)]).unwrap(),
        // The naive reading arrived after the containers, and the ordering
        // numbering is kept, so its place is at the end rather than beside
        // its zoned sibling.
        Value::timestamp(1_700_000_000, TimeUnit::Second, None).unwrap(),
    ]
}

#[test]
fn structural_serde_reads_back_every_variant() {
    // The hand-written `Deserialize` mirrors `Value` variant for variant, and a
    // variant missing from the mirror is not a compile error - it is data serde
    // silently refuses to read. This is the check that makes it loud.
    for value in one_of_every_kind() {
        let encoded = serde_json::to_vec(&value).unwrap();
        let decoded = serde_json::from_slice::<Value>(&encoded).unwrap();
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
    // the two float widths are another, spelled two ways.
    let mut kinds = values.iter().map(Value::kind).collect::<Vec<_>>();
    kinds.dedup();
    assert_eq!(
        kinds.len(),
        values.len(),
        "every kind spells its own name: {kinds:?}"
    );
}

#[test]
fn integer_widths_have_native_numeric_equality() {
    assert_eq!(Value::I64(1), Value::U64(1));
    assert_eq!(Value::I128(1), Value::U128(1));
    assert_eq!(Value::from(1_i8), Value::from(1_u32));
    assert_ne!(Value::I64(-1), Value::U64(1));
}

#[test]
fn float_widths_are_one_number_line() {
    // An `f32` widens to `f64` exactly, so the same reading at either width
    // is one value - as the integers are one number line across widths.
    assert_eq!(Value::from(1.5_f32), Value::from(1.5_f64));
    assert!(Value::from(1.25_f32) < Value::from(1.5_f64));
    assert!(Value::from(2.0_f64) < Value::from(2.5_f32));
    assert_ne!(Value::from(0.1_f32), Value::from(0.1_f64));
}

#[test]
fn equal_integer_representations_hash_and_order_equally() {
    fn hash(value: &Value) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    let values = [Value::I64(1), Value::U64(1), Value::I128(1), Value::U128(1)];
    for left in &values {
        for right in &values {
            assert_eq!(left, right);
            assert_eq!(left.cmp(right), std::cmp::Ordering::Equal);
            assert_eq!(hash(left), hash(right));
        }
    }
    assert!(Value::I128(i128::MIN) < Value::I64(-1));
    assert!(Value::I64(-1) < Value::U128(u128::MAX));
}

#[test]
fn structural_serde_preserves_all_float_values() {
    for value in [0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let value = Value::from(value);
        let encoded = serde_json::to_vec(&value).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&encoded).unwrap(), value);
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
    assert!(serde_json::from_slice::<Value>(encoded).is_err());
}

#[test]
fn wide_mapping_constructor_rejects_duplicates() {
    let mut entries = (0_u64..128)
        .map(|index| (Value::from(index), Value::from(index)))
        .collect::<Vec<_>>();
    entries.push((Value::I128(64), Value::Null));
    assert!(Value::from_mapping(entries).is_err());
}

#[test]
fn collection_iteration_matches_python_sequence_and_mapping_semantics() {
    let sequence = Value::from_sequence([Value::from(1_i64), Value::from(2_i64)]);
    assert_eq!(
        (&sequence).into_iter().collect::<Vec<_>>(),
        vec![&sequence[0], &sequence[1]]
    );

    let mapping = Value::from_mapping([
        (Value::from("a"), Value::from(1_i64)),
        (Value::from("b"), Value::from(2_i64)),
    ])
    .unwrap();
    assert_eq!(mapping["a"], Value::I64(1));
    assert_eq!(
        mapping.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn empty_collections_share_process_wide_backing() {
    let left = Value::from(Vec::<u8>::new());
    let encoded = serde_json::to_vec(&left).unwrap();
    let right: Value = serde_json::from_slice(&encoded).unwrap();
    let (Value::Bytes(left), Value::Bytes(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));

    let left = Value::from_sequence([]);
    let right = Value::from_sequence([]);
    let (Value::Sequence(left), Value::Sequence(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));

    let left = Value::from_mapping([]).unwrap();
    let right = Value::from_mapping([]).unwrap();
    let (Value::Mapping(left), Value::Mapping(right)) = (&left, &right) else {
        unreachable!();
    };
    assert!(std::sync::Arc::ptr_eq(left, right));
    assert!(!Value::Null.is_empty());
}
