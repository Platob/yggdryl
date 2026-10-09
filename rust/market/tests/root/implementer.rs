//! `rust/market/src/implementer.rs`: the door `yggdryl-fix` reaches this
//! crate's crate-private items through. Every route it exports resolves
//! and answers what the item behind it answers - a forwarder, a
//! `<type>_<item>` free function and an item raised to `pub`; the exported
//! `delegate_*` macros compiling in a foreign crate is `yggdryl-fix`'s
//! build.

use yggdryl_market::graph::{Market, OrderEvent};
use yggdryl_market::implementer;
use yggdryl_market::{IdKey, IdType, Identifier, IsinRegistry, MarketDataKind, Side};

/// A forwarder answers what its item answers: the one fold an identifier
/// word reads by - the one a public spelling of a type folds through - and
/// the base a stored cross code keeps.
#[test]
fn a_forwarder_answers_what_its_item_answers() {
    crate::install::installed();
    let mut buffer = [0_u8; implementer::WORD_PAIR_WIDTH];
    let folded = implementer::fold_into("Executing Trader", &mut buffer).unwrap();
    assert_eq!(folded, "executingtrader");
    assert_eq!(
        implementer::folded_len("Executing Trader"),
        Some(folded.len())
    );
    assert!(implementer::is_word("Executing Trader"));
    assert!(!implementer::is_word("ex/ec"));
    assert_eq!(
        implementer::fold_into("ex/ec", &mut buffer),
        Err("ASCII letters, digits and '.'")
    );
    let folded = implementer::fold_into("Order_ID", &mut buffer).unwrap();
    assert_eq!("Order_ID".parse::<IdType>().unwrap(), folded);
    for (stored, base) in [
        ("10:1:ORD-1", "ORD-1"),
        ("14:0:Q-1", "Q-1"),
        ("ORD-1", "ORD-1"),
    ] {
        assert_eq!(implementer::base_crosscode(stored), base, "{stored}");
    }
}

/// A `<type>_<item>` function answers what the associated item answers:
/// the side a kind's stored cross code carries is the stated one for a
/// sided kind and `UKNW` for every other, and the registry's learn stating
/// nothing beyond the event learns what the public `learn` does.
#[test]
fn a_type_item_function_answers_what_its_associated_item_answers() {
    crate::install::installed();
    for kind in MarketDataKind::ALL {
        for side in [Side::Unknown, Side::Buy, Side::Sell, Side::Both] {
            let expected = if kind.is_sided() { side } else { Side::Unknown };
            assert_eq!(
                implementer::market_data_kind_stored_side(*kind, side),
                expected,
                "{kind} {side}"
            );
        }
    }

    let mut event = OrderEvent::at(1);
    event
        .insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap())
        .unwrap();
    let mut public = IsinRegistry::new();
    let mut routed = IsinRegistry::new();
    assert!(public.learn(&event));
    let learned = implementer::isin_registry_learn_stating(
        &mut routed,
        &event,
        Some(event.get_origccy()),
        None,
        None,
        None,
    );
    assert!(learned.moved);
    assert_eq!(learned.full, None);
    assert_eq!(routed.get("US0378331005"), public.get("US0378331005"));
    assert_eq!(routed.len(), 1);
}

/// An item raised to `pub` is the item itself: the one rule a key naming
/// another instrument's fact is read by.
#[test]
fn a_raised_item_is_the_item_itself() {
    crate::install::installed();
    for (folded, fact, another) in [
        ("omsunderlyingisin", "isin", true),
        ("fix.legisin", "isin", true),
        ("underlyingeusipa", "eusipa", true),
        ("omsisin", "isin", false),
        ("isin", "isin", false),
    ] {
        let at = folded.len() - fact.len();
        assert_eq!(
            implementer::names_another_instrument(folded, at),
            another,
            "{folded}"
        );
    }
}
