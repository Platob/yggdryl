//! `rust/src/text/loading.rs`: what a structured-text read may do beyond
//! parsing.
//!
//! `Loading` is a caller's value - limits, placeholders, a field - so each of
//! these builds one and reads what a load did with it through `yggdryl::text`.
//! Its map semantics are pinned by the crate's own stable hash, which reaches
//! no caller and is read through `yggdryl::internals`.

use std::hash::Hash;

use yggdryl::internals::hashing_stable::stable_hash_of;
use yggdryl::text::{Format, Loading, Placeholders};
use yggdryl::{DataType, Field, Scalar};

fn amount_field() -> Field {
    Field::new(
        "amount",
        DataType::decimal128(8, 2).expect("valid decimal"),
        false,
    )
}

#[test]
fn field_recovers_an_exact_natural_value() {
    let loading = Loading::new().with_field(amount_field());
    let value =
        yggdryl::text::from_utf8_with("\"12.50\"", Format::Json, &loading).expect("typed JSON");
    assert_eq!(value, Scalar::d128(1_250, 2));
    assert_eq!(loading.field().map(Field::name), Some("amount"));

    let (_, inferred_utf8) =
        yggdryl::text::from_utf8_inferred_with_field("\"12.50\"", &amount_field())
            .expect("inferred typed UTF-8");
    let (_, inferred_bytes) =
        yggdryl::text::from_bytes_inferred_with_field(b"\"12.50\"", &amount_field())
            .expect("inferred typed bytes");
    assert_eq!(inferred_utf8, value);
    assert_eq!(inferred_bytes, value);
}

#[test]
fn placeholders_are_resolved_before_field_interpretation() {
    let loading = Loading::new()
        .with_placeholders(Placeholders::new().with_variable("AMOUNT", Scalar::from("12.50")))
        .with_field(amount_field());
    let value = yggdryl::text::from_utf8_with("\"{{ AMOUNT }}\"\n", Format::Yaml, &loading)
        .expect("filled typed YAML");
    assert_eq!(value, Scalar::d128(1_250, 2));
}

#[test]
fn loading_and_placeholders_have_map_semantic_value_traits() {
    fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
    assert_traits::<Placeholders>();
    assert_traits::<Loading>();

    let first = Placeholders::new()
        .with_variable("A", Scalar::from(1))
        .with_variable("B", Scalar::from(2));
    let equal = Placeholders::new()
        .with_variable("B", Scalar::from(2))
        .with_variable("A", Scalar::from(1));
    assert_eq!(first, equal);
    assert_eq!(stable_hash_of(&first), stable_hash_of(&equal));

    let first = Loading::new().with_placeholders(first);
    let equal = Loading::new().with_placeholders(equal);
    assert_eq!(first, equal);
    assert_eq!(stable_hash_of(&first), stable_hash_of(&equal));
}
