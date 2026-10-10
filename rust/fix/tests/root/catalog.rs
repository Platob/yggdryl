//! `rust/fix/src/catalog.rs`: the named definitions beside the scalar
//! indexes.
//!
//! What a caller reaches of the catalog is mostly pinned where the doors
//! are: the definition doors in `registry.rs`, the CBlock reader in `cfb.rs`.
//! This file holds the catalog's own rule on a NumInGroup counter beside the
//! group it counts, which a definition written alone is refused for and a
//! fold leaves out, and, in `internal`, what a caller cannot reach: the
//! structure two grammars declaring one component or one group agree on,
//! which a caller meets only as one definition where it declared two, so it
//! is read through `yggdryl_fix::internals`.

use yggdryl::{DataType, Error, Field, StructType};
use yggdryl_fix::FixCategory::{Components, Groups};
use yggdryl_fix::{FixField, FixFieldMut, FixRegistry};

/// A dictionary holding the three party fields and a leg symbol, the wire
/// fields every definition below reads.
fn dictionary() -> FixRegistry {
    let mut registry = FixRegistry::new();
    for (tag, name, dtype) in [
        (448, "PartyID", DataType::utf8()),
        (447, "PartyIDSource", DataType::utf8()),
        (452, "PartyRole", DataType::Int32),
        (453, "NoPartyIDs", DataType::Int32),
        (600, "LegSymbol", DataType::utf8()),
    ] {
        let mut field = dtype.nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        registry.insert(field).unwrap();
    }
    registry
}

/// One member reading the dictionary's field `name`, as a grammar's
/// constraint reads one: the field itself, under its own name, with the
/// constraint's nullability.
fn reads(registry: &FixRegistry, name: &str, nullable: bool) -> Field {
    let mut member = registry.field_by_name(name).expect(name).clone();
    member.set_nullable(nullable);
    FixFieldMut::new(&mut member).set_field_ref(name).unwrap();
    member
}

/// A component definition of `members`, named and described as a grammar
/// describes one.
fn component(name: &str, display: &str, members: Vec<Field>) -> Field {
    let mut component =
        DataType::from(StructType::from_fields(members).unwrap()).required_field(name);
    component.set_display(display).unwrap();
    component
}

/// A group definition on `counter` drawing its occurrences from the held
/// component `name`.
fn group(registry: &FixRegistry, name: &str, counter: i32, component: &str) -> Field {
    let mut item = registry.definition(Components, component).unwrap().clone();
    item.set_name(component);
    let mut group = DataType::serie(item).nullable_field(name);
    FixFieldMut::new(&mut group).set_counter(counter).unwrap();
    FixFieldMut::new(&mut group)
        .set_component(component)
        .unwrap();
    group
}

/// The group `parties` held as a member of a definition, the way a
/// catalogued message reads it.
fn reading(registry: &FixRegistry, name: &str) -> Field {
    let mut member = registry.definition(Groups, name).unwrap().clone();
    member.remove_metadata("FIX:tag");
    FixFieldMut::new(&mut member).set_group(name).unwrap();
    member
}

/// A dictionary holding the component `party` and the group `parties` on
/// 453 over it.
fn with_parties() -> FixRegistry {
    let mut registry = dictionary();
    let party = component(
        "party",
        "Party",
        vec![
            reads(&registry, "partyid", false),
            reads(&registry, "partyidsource", true),
        ],
    );
    registry.insert_definition(Components, party).unwrap();
    let parties = group(&registry, "parties", 453, "party");
    registry.insert_definition(Groups, parties).unwrap();
    registry
}

/// The wire tags the scalar members of the definition `name` read.
fn scalar_tags(registry: &FixRegistry, name: &str) -> Vec<i32> {
    registry
        .definition(Components, name)
        .unwrap()
        .fields()
        .iter()
        .filter(|member| !member.dtype().is_nested())
        .filter_map(|member| FixField::new(member).tag().unwrap())
        .collect()
}

#[test]
fn a_counter_beside_the_group_it_counts_is_refused_by_hand() {
    crate::install::installed();
    // A group is its list and its length the count, so a definition written
    // alone that lists the counter beside the group states one fact twice,
    // and the catalog refuses it whole, leaving the dictionary as it was.
    let mut registry = with_parties();
    let order = component(
        "order",
        "Order",
        vec![
            reads(&registry, "nopartyids", true),
            reading(&registry, "parties"),
        ],
    );
    let before = registry.clone();
    let error = registry.insert_definition(Components, order).unwrap_err();
    let Error::InvalidRecord { path, reason } = &error else {
        panic!("an invalid record, got {error}");
    };
    assert_eq!(path.as_str(), "order.NoPartyIDs");
    assert!(
        reason.contains(
            "expected no NumInGroup counter beside the group it counts, whose length is its count, got NoPartyIDs (453)"
        ),
        "{reason}"
    );
    assert_eq!(registry, before);
}

#[test]
fn a_fold_leaves_out_a_counter_beside_the_group_the_other_dictionary_holds() {
    crate::install::installed();
    // One dictionary's `order` reads the group, the other's lists the
    // counter alone and requires it. Folded either way, `order` reads the
    // group and no member on 453, and nothing is named as dropped.
    let mut grouped = with_parties();
    let order = component(
        "order",
        "Order",
        vec![
            reads(&grouped, "legsymbol", true),
            reading(&grouped, "parties"),
        ],
    );
    grouped.insert_definition(Components, order).unwrap();
    let mut bare = with_parties();
    let order = component(
        "order",
        "Order",
        vec![
            reads(&bare, "legsymbol", true),
            reads(&bare, "nopartyids", false),
        ],
    );
    bare.insert_definition(Components, order).unwrap();
    // The counter left out required the group, so the group is required in
    // either order: where the group arrives onto the counter's side, it
    // states the structure the fold left `order` with, and the relaxation a
    // definition stating the held structure takes keeps what the counter
    // required.
    for (held, incoming) in [(&grouped, &bare), (&bare, &grouped)] {
        let mut folded = held.clone();
        let merge = folded.merge_with(incoming).expect("the two fold");
        assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
        assert_eq!(scalar_tags(&folded, "order"), [600]);
        let parties = folded
            .definition(Components, "order")
            .unwrap()
            .get_field("parties")
            .expect("the group");
        assert_eq!(FixField::new(parties).group(), Some("parties"));
        assert!(!parties.is_nullable(), "the counter required the group");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use super::{Components, DataType, FixFieldMut, Groups, component, dictionary, group, reads};
    use yggdryl_fix::internals::catalog::structural_key;

    #[test]
    fn the_structure_is_the_members_tags_and_fields_and_nothing_a_declaration_adds_to_them() {
        crate::install::installed();
        let mut registry = dictionary();
        // One structure declared twice: another name, another display, the
        // one member required where the first has it nullable, and sources
        // only one side states.
        let mut party = component(
            "party",
            "Party",
            vec![
                reads(&registry, "partyid", false),
                reads(&registry, "partyidsource", true),
            ],
        );
        FixFieldMut::new(&mut party).set_sources(["venue"]).unwrap();
        let mut dealer = component(
            "dealer",
            "Dealer",
            vec![
                reads(&registry, "partyid", true),
                reads(&registry, "partyidsource", false),
            ],
        );
        FixFieldMut::new(&mut dealer)
            .set_description("A dealer.")
            .unwrap();
        registry.insert_definition(Components, party).unwrap();
        registry.insert_definition(Components, dealer).unwrap();
        let party = registry.definition(Components, "party").unwrap();
        let dealer = registry.definition(Components, "dealer").unwrap();
        let key = structural_key(&registry, party).unwrap();
        assert_eq!(key, "(448:partyid,447:partyidsource)");
        assert_eq!(structural_key(&registry, dealer).unwrap(), key);
        // The derived tag the catalog stamps a definition with is the catalog's
        // identity for its name, not the structure's: the key is read off a
        // definition and off a bare clone of its members alike.
        let mut bare = party.clone();
        bare.remove_metadata("FIX:tag");
        assert_eq!(structural_key(&registry, &bare).unwrap(), key);

        // Another member, another order, another tag: another structure.
        for (name, members) in [
            (
                "attributed",
                vec![
                    reads(&registry, "partyid", true),
                    reads(&registry, "partyidsource", true),
                    reads(&registry, "partyrole", true),
                ],
            ),
            (
                "reversed",
                vec![
                    reads(&registry, "partyidsource", true),
                    reads(&registry, "partyid", true),
                ],
            ),
            (
                "role",
                vec![
                    reads(&registry, "partyid", true),
                    reads(&registry, "partyrole", true),
                ],
            ),
        ] {
            let other = component(name, name, members);
            assert_ne!(structural_key(&registry, &other).unwrap(), key, "{name}");
        }
    }

    #[test]
    fn a_group_is_its_counter_over_the_structure_of_the_component_it_draws_on() {
        crate::install::installed();
        let mut registry = dictionary();
        let party = component(
            "party",
            "Party",
            vec![
                reads(&registry, "partyid", false),
                reads(&registry, "partyidsource", true),
            ],
        );
        registry
            .insert_definition(Components, party.clone())
            .unwrap();
        let mut dealer = party.clone();
        dealer.set_name("dealer");
        registry.insert_definition(Components, dealer).unwrap();
        let parties = group(&registry, "parties", 453, "party");
        registry.insert_definition(Groups, parties.clone()).unwrap();
        let key =
            structural_key(&registry, registry.definition(Groups, "parties").unwrap()).unwrap();
        assert_eq!(key, "453*(448:partyid,447:partyidsource)");
        // Two groups on one counter over one structure are one structure
        // whatever their components are called, and a group on another counter
        // is another.
        let dealers = group(&registry, "dealers", 453, "dealer");
        assert_eq!(structural_key(&registry, &dealers).unwrap(), key);
        let legs = group(&registry, "legs", 555, "party");
        assert_eq!(
            structural_key(&registry, &legs).unwrap(),
            "555*(448:partyid,447:partyidsource)"
        );
        // A message holding the group keys it inline where it holds it, and
        // nesting is structure: a component holding the group is not one holding
        // the two fields beside each other.
        let mut member = registry.definition(Groups, "parties").unwrap().clone();
        member.remove_metadata("FIX:tag");
        FixFieldMut::new(&mut member).set_group("parties").unwrap();
        let report = component(
            "report",
            "Report",
            vec![reads(&registry, "legsymbol", true), member],
        );
        assert_eq!(
            structural_key(&registry, &report).unwrap(),
            "(600:legsymbol,453*(448:partyid,447:partyidsource))"
        );
        let flat = component(
            "flat",
            "Flat",
            vec![
                reads(&registry, "legsymbol", true),
                reads(&registry, "partyid", true),
                reads(&registry, "partyidsource", true),
            ],
        );
        assert_eq!(
            structural_key(&registry, &flat).unwrap(),
            "(600:legsymbol,448:partyid,447:partyidsource)"
        );
        // A reference the dictionary does not resolve is its absence, not a
        // structure: the occurrence names the component, the root only says
        // which one the group draws on.
        let mut dangling = parties.clone();
        let DataType::Serie(item) = dangling.dtype().clone() else {
            panic!("a serie group");
        };
        let mut item = item.as_ref().clone();
        FixFieldMut::new(&mut item).set_component("nobody").unwrap();
        dangling.set_dtype(DataType::serie(item)).unwrap();
        let error = structural_key(&registry, &dangling).unwrap_err();
        assert!(error.to_string().contains("nobody"), "{error}");
    }
}
