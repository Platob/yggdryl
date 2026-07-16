//! **Header identity-exclusion** locks (Phase 1 foundation): every concrete `Serie` and `Scalar`
//! carries its family's own field descriptor (name / declared nullability / metadata), and that
//! header is **excluded** from value identity and the byte codec. Two values equal in DATA but
//! differing in name / nullable / metadata must compare **equal**, hash **equal** (where hashable),
//! serialize **byte-identical**, and `deserialize(serialize(x)) == x` must still hold.
//!
//! One test per family locks the rule; a regression here means the header leaked into identity.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl_core::io::fixed::temporal::{TimeUnit, Ts64, Tz};
use yggdryl_core::io::fixed::{
    D128Scalar, D128Serie, FixedBinaryScalar, FixedBinarySerie, NullScalar, NullSerie, Scalar,
    Serie, Ts64Scalar, Ts64Serie, D128,
};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::{Utf8Scalar, Utf8Serie};
use yggdryl_core::io::{AnySerie, Headers};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A distinct header: a non-empty name, a flipped `nullable`, and one metadata entry.
fn meta() -> Headers {
    Headers::new().with("origin", "test")
}

// -------------------------------------------------------------------------------------
// Leaf series: eq + byte-identity + round-trip are independent of the header.
// -------------------------------------------------------------------------------------

#[test]
fn fixed_serie_header_excluded_from_identity() {
    let plain = Serie::from_options(&[Some(1i32), None, Some(3)]);
    let headed = plain
        .clone()
        .with_name("x")
        .with_nullable(true)
        .with_metadata(meta());

    assert_eq!(plain, headed, "header must not affect Serie equality");
    assert_eq!(
        plain.serialize_bytes(),
        headed.serialize_bytes(),
        "header must not affect the Serie frame"
    );
    let back = Serie::<i32>::deserialize_bytes(&headed.serialize_bytes()).unwrap();
    assert_eq!(back, headed, "deserialize(serialize(x)) == x");
    // The header itself did land on the value (just not in identity).
    assert_eq!(headed.name(), "x");
    assert!(headed.nullable());
    assert_eq!(headed.metadata().get("origin"), Some("test"));
}

#[test]
fn var_serie_header_excluded_from_identity() {
    let plain = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
    let headed = plain.clone().with_name("y").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        Utf8Serie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn fixed_size_serie_header_excluded_from_identity() {
    let plain = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None]).unwrap();
    let headed = plain.clone().with_name("z").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        FixedBinarySerie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn decimal_serie_header_excluded_from_identity() {
    let a = D128::new(12345, 2).unwrap();
    let plain = D128Serie::from_options(20, 2, &[Some(a), None]).unwrap();
    let headed = plain.clone().with_name("amt").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        D128Serie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn temporal_serie_header_excluded_from_identity_and_hash() {
    let a = Ts64::from_epoch(10, TimeUnit::Second, Tz::UTC).unwrap();
    let plain = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
    let headed = plain
        .clone()
        .with_name("t")
        .with_nullable(true)
        .with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed), "hash must ignore header");
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        Ts64Serie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn null_serie_header_excluded_from_identity() {
    let plain = NullSerie::with_len(4);
    let headed = plain.clone().with_name("n").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        NullSerie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

// -------------------------------------------------------------------------------------
// Nested series: the OWN header (not the child names) is excluded from identity + bytes.
// -------------------------------------------------------------------------------------

#[test]
fn struct_serie_own_header_excluded_from_identity() {
    let plain =
        StructSerie::from_series(vec![Serie::from_values(&[1i64, 2, 3]).named("id")]).unwrap();
    let headed = plain
        .clone()
        .with_name("row")
        .with_nullable(true)
        .with_metadata(meta());
    assert_eq!(
        plain, headed,
        "the struct's own header must not affect identity"
    );
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        StructSerie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn list_serie_own_header_excluded_from_identity() {
    let plain = ListSerie::from_values(
        Serie::from_values(&[1i32, 2, 3]).named("item"),
        &[0, 2, 3],
        None,
    )
    .unwrap();
    let headed = plain.clone().with_name("xs").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        ListSerie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn map_serie_own_header_excluded_from_identity() {
    let plain = MapSerie::from_entries(
        Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key"),
        Serie::from_values(&[1i64, 2]).named("value"),
        &[0, 2],
        None,
        false,
    )
    .unwrap();
    let headed = plain.clone().with_name("m").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        MapSerie::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

// -------------------------------------------------------------------------------------
// Scalars: eq + hash + byte-identity + round-trip are independent of the header.
// -------------------------------------------------------------------------------------

#[test]
fn fixed_scalar_header_excluded_from_identity() {
    let plain = Scalar::of(42i32);
    let headed = plain
        .clone()
        .with_name("x")
        .with_nullable(true)
        .with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        Scalar::<i32>::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn var_scalar_header_excluded_from_identity() {
    let plain = Utf8Scalar::of("héllo");
    let headed = plain.clone().with_name("y").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        Utf8Scalar::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn fixed_size_scalar_header_excluded_from_identity() {
    use yggdryl_core::io::Bytes;
    let bytes = |s: &FixedBinaryScalar| {
        let mut sink = Bytes::new();
        s.write_to(&mut sink).unwrap();
        sink.as_slice().to_vec()
    };

    let plain = FixedBinaryScalar::from_bytes(b"ab").unwrap();
    let headed = plain.clone().with_name("z").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(bytes(&plain), bytes(&headed));
    let mut source = Bytes::from_slice(&bytes(&headed));
    assert_eq!(FixedBinaryScalar::read_from(&mut source).unwrap(), headed);
}

#[test]
fn decimal_scalar_header_excluded_from_identity() {
    let plain = D128Scalar::of(D128::new(12345, 2).unwrap());
    let headed = plain.clone().with_name("amt").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        D128Scalar::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn temporal_scalar_header_excluded_from_identity() {
    let plain = Ts64Scalar::of(Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap());
    let headed = plain
        .clone()
        .with_name("t")
        .with_nullable(true)
        .with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        Ts64Scalar::deserialize_bytes(&headed.serialize_bytes()).unwrap(),
        headed
    );
}

#[test]
fn null_scalar_header_excluded_from_identity() {
    let plain = NullScalar::null();
    let headed = plain.clone().with_name("n").with_metadata(meta());
    assert_eq!(plain, headed);
    assert_eq!(hash_of(&plain), hash_of(&headed));
    assert_eq!(plain.serialize_bytes(), headed.serialize_bytes());
    assert_eq!(
        NullScalar::deserialize_bytes(&headed.serialize_bytes()),
        headed
    );
}
