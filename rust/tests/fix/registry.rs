//! `rust/src/fix/registry.rs`: the five indexes behind one namespace.
//!
//! A caller asks a registry for a tag, an identity or a name; what it is made
//! of - the seeded digests, the position indexes, the compiled derivations and
//! the lifted-name cache - is reached through `yggdryl::internals`, because a
//! digest collision and a warm cache cannot be staged from outside. What the
//! lookups answer is pinned through `yggdryl::` in the suites beside this one.

use std::sync::Arc;

use yggdryl::internals::fix_registry::{
    ALIAS_SEED, NAME_SEED, canonical_id, derivations as compiled, fields, force_alias_index,
    force_id_index, force_name_index, lifted_names_cached, name_digest, warm_lifted_names,
};
use yggdryl::internals::metadata::shares_storage_with;
use yggdryl::{DataType, Error, Field, FixRegistry};

fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

#[test]
fn a_forced_name_digest_collision_is_a_miss_then_a_conflict() {
    let held = tagged("Held", 1);
    let incoming = tagged("Incoming", 2);
    let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
    // The crate's own fields sit in front of it, so its position is
    // found rather than assumed to be the first.
    let at = fields(&registry)
        .iter()
        .position(|field| field.name() == held.name())
        .expect("the held field");
    let collided = name_digest(incoming.name(), NAME_SEED);
    force_name_index(&mut registry, collided, at);

    assert!(
        registry.get_field_by_name(incoming.name()).is_none(),
        "a digest hit is rechecked against the canonical name"
    );
    let before = fields(&registry).to_vec();
    let error = registry.insert(incoming).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Conflict { path, .. }
                if path.contains("Incoming") && path.contains("Held")
        ),
        "{error}"
    );
    assert_eq!(fields(&registry), before);
}

#[test]
fn a_forced_identity_collision_is_a_conflict_and_a_hit_is_rechecked() {
    let held = tagged("Held", 1);
    let incoming = tagged("Incoming", 2);
    let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
    let at = fields(&registry)
        .iter()
        .position(|field| field.name() == held.name())
        .expect("the held field");
    // The incoming identity is forced onto the held field's position, as
    // a 32-bit digest collision would land it.
    let collided = canonical_id(&incoming).unwrap();
    force_id_index(&mut registry, collided, at);

    let hit = registry
        .get_field_by_id(collided)
        .expect("the position answers");
    assert_eq!(
        hit.name(),
        "Held",
        "an identity hit answers the field indexed under it"
    );
    let before = fields(&registry).to_vec();
    let error = registry.insert(incoming).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Conflict { path, .. }
                if path.contains("Incoming") && path.contains("Held")
        ),
        "{error}"
    );
    assert_eq!(fields(&registry), before);
}

#[test]
fn default_aliases_refuse_a_digest_collision_without_mutating_the_source() {
    let offer = tagged("offerpx", 1);
    let unrelated = tagged("Unrelated", 2);
    let mut registry = FixRegistry::from_fields([offer, unrelated]).unwrap();
    let unrelated_at = fields(&registry)
        .iter()
        .position(|field| field.name() == "Unrelated")
        .expect("the unrelated holder");
    force_alias_index(
        &mut registry,
        name_digest("askpx", ALIAS_SEED),
        unrelated_at,
    );

    assert!(
        registry.get_field_by_name("askpx").is_none(),
        "a colliding alias digest is rechecked before lookup answers"
    );
    let before = registry.clone();
    let error = registry.clone().with_default_aliases().unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path.contains("askpx")),
        "{error}"
    );
    assert_eq!(
        registry, before,
        "the consumed attempt leaves its source intact"
    );
}

#[test]
fn default_aliases_do_not_reindex_an_unchanged_second_pass() {
    let registry = FixRegistry::from_fields([tagged("offerpx", 1)])
        .unwrap()
        .with_default_aliases()
        .unwrap();
    let before = registry
        .get_field_by_name("offerpx")
        .expect("the lender")
        .as_metadata()
        .clone();
    let derivations = compiled(&registry).unwrap();
    warm_lifted_names(&registry);
    let registry = registry.with_default_aliases().unwrap();
    let after = registry
        .get_field_by_name("offerpx")
        .expect("the lender")
        .as_metadata();
    assert!(
        shares_storage_with(after, &before),
        "an unchanged field keeps its metadata storage"
    );
    assert!(
        Arc::ptr_eq(&derivations, &compiled(&registry).unwrap()),
        "an unchanged pass keeps the compiled derivations"
    );
    assert!(
        lifted_names_cached(&registry),
        "an unchanged pass keeps the lifted-name cache"
    );
}
