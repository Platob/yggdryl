//! `rust/src/avro/mod.rs`: the named-type registry no caller can reach.
//!
//! A record names itself, and a later field may name that record instead of
//! restating it. `Schema::names` is the crate-private table that resolves the
//! second to the first. Everything a caller can observe lives in
//! `tests/media/avro.rs`.

use yggdryl::avro::Schema;
use yggdryl::internals::avro_schema::names;

#[test]
fn a_reference_to_an_earlier_definition_is_not_a_redefinition() {
    let schema = Schema::from_str(
        r#"{"type":"record","name":"row","fields":[
            {"name":"p","type":{"type":"record","name":"kv","fields":[
                {"name":"x","type":"long"}]}},
            {"name":"q","type":"kv"}
        ]}"#,
    )
    .unwrap();
    assert!(names(&schema).contains(&"kv"));
}
