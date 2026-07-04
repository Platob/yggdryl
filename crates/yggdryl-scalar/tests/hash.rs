//! Every atomic scalar and every concrete serie is hashable: equal values hash
//! equally (so they de-duplicate in a set / key a map) and unequal values stay
//! distinct. Floats hash by bit pattern with `-0.0` canonicalized to `+0.0`; the
//! `NaN != NaN` value rule is the usual float-in-a-set caveat.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use yggdryl_scalar::half::f16;
use yggdryl_scalar::{
    BinaryScalar, Float16Scalar, Float32Scalar, Float64Scalar, Int64Scalar, Int64Serie, NullScalar,
    UInt8Scalar, Utf8Scalar,
};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// Equal values hash equally; a different value hashes differently (almost surely);
// and the type de-duplicates in a `HashSet`.
fn assert_hashes<T: Hash + Eq + Clone + std::fmt::Debug>(a: T, same: T, different: T) {
    assert_eq!(a, same, "the two values should be equal");
    assert_eq!(hash_of(&a), hash_of(&same), "equal values must hash equally");
    assert_ne!(a, different, "the third value should differ");

    let set: HashSet<T> = [a.clone(), same, different.clone()].into_iter().collect();
    assert_eq!(set.len(), 2, "equal values collapse, the different one stays");
    assert!(set.contains(&a));
    assert!(set.contains(&different));
}

#[test]
fn integer_scalars_are_hashable() {
    assert_hashes(Int64Scalar::new(1), Int64Scalar::new(1), Int64Scalar::new(2));
    assert_hashes(UInt8Scalar::new(7), UInt8Scalar::new(7), UInt8Scalar::new(8));
    // Null is a distinct, hashable value.
    assert_hashes(Int64Scalar::null(), Int64Scalar::null(), Int64Scalar::new(0));
}

#[test]
fn null_scalar_is_hashable() {
    assert_eq!(hash_of(&NullScalar::default()), hash_of(&NullScalar::default()));
    let set: HashSet<NullScalar> = [NullScalar::default(), NullScalar::default()]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn float_scalars_are_hashable() {
    assert_hashes(
        Float64Scalar::new(1.5),
        Float64Scalar::new(1.5),
        Float64Scalar::new(2.5),
    );
    assert_hashes(
        Float32Scalar::new(1.5),
        Float32Scalar::new(1.5),
        Float32Scalar::new(2.5),
    );
    assert_hashes(
        Float16Scalar::new(f16::from_f32(1.5)),
        Float16Scalar::new(f16::from_f32(1.5)),
        Float16Scalar::new(f16::from_f32(2.5)),
    );
    // Null floats are hashable and distinct from a value.
    assert_hashes(
        Float64Scalar::null(),
        Float64Scalar::null(),
        Float64Scalar::new(0.0),
    );
}

#[test]
fn float_zero_is_canonical_and_nan_is_the_documented_caveat() {
    // -0.0 == +0.0 by value, and they hash equally (so they collapse in a set).
    let plus = Float64Scalar::new(0.0);
    let minus = Float64Scalar::new(-0.0);
    assert_eq!(plus, minus);
    assert_eq!(hash_of(&plus), hash_of(&minus));
    let zeros: HashSet<Float64Scalar> = [plus, minus].into_iter().collect();
    assert_eq!(zeros.len(), 1);

    // A NaN scalar hashes fine, but is unequal to itself by value — so it can be
    // stored yet never looked up. This does not panic; it is the float-in-a-set rule.
    let nan = Float64Scalar::new(f64::NAN);
    assert_ne!(nan, Float64Scalar::new(f64::NAN));
    let _ = hash_of(&nan); // hashing a NaN scalar is well-defined
    let mut set = HashSet::new();
    set.insert(nan);
    assert!(!set.contains(&Float64Scalar::new(f64::NAN))); // never found
}

#[test]
fn binary_and_utf8_scalars_are_hashable() {
    assert_hashes(
        BinaryScalar::new(b"hi".to_vec()),
        BinaryScalar::new(b"hi".to_vec()),
        BinaryScalar::new(b"bye".to_vec()),
    );
    assert_hashes(
        Utf8Scalar::new("hé".to_string()),
        Utf8Scalar::new("hé".to_string()),
        Utf8Scalar::new("ho".to_string()),
    );
    assert_hashes(
        BinaryScalar::null(),
        BinaryScalar::null(),
        BinaryScalar::new(Vec::new()), // null is distinct from the empty value
    );
}

#[test]
fn concrete_series_are_hashable() {
    assert_hashes(
        Int64Serie::from(vec![1, 2, 3]),
        Int64Serie::from(vec![1, 2, 3]),
        Int64Serie::from(vec![1, 2, 4]),
    );
    // Null and empty are distinct, both hashable.
    assert_hashes(
        Int64Serie::null(),
        Int64Serie::null(),
        Int64Serie::from(Vec::<i64>::new()),
    );
    // A per-element null is respected: [1, null] differs from [1, 2], and the null
    // slot's arbitrary buffer byte does not leak into the hash.
    let with_null = Int64Serie::from(vec![Some(1), None]);
    assert_eq!(hash_of(&with_null), hash_of(&Int64Serie::from(vec![Some(1), None])));
    assert_ne!(with_null, Int64Serie::from(vec![Some(1), Some(2)]));
}
