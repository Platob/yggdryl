//! `rust/src/graph/facts.rs`: the four crate-private holders - market,
//! market event, operation, operation event - each leaf holds the one of
//! its role. A caller reads their facts through the leaves, so what a
//! holder answers on its own is pinned through `yggdryl::internals`.

use yggdryl::graph::{Element, Market, Order, OrderEvent, Quote};
use yggdryl::securityid::{SecType, SecurityId};

/// One security identifier under `key`, validated by its source.
fn id(key: &str, code: &str) -> SecurityId {
    SecurityId::new(SecType::read(key).unwrap(), code).unwrap()
}

fn isin() -> SecType {
    SecType::read("ISIN").unwrap()
}

/// Every identifier an element holds, as `KEY:code`, in source order.
fn ids(element: &impl Market) -> Vec<String> {
    element
        .get_securityids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// An undated operation continues its holder's digest with its kind, so
/// the leaf's code is its own and not the holder's.
#[test]
fn an_undated_leaf_digests_its_kind_behind_its_holder() {
    let mut order = Order::new();
    order.set_crosscode("ORDER".to_owned());
    order.finalize();
    let mut quote = Quote::new();
    quote.set_crosscode("ORDER".to_owned());
    quote.finalize();
    assert_ne!(order.get_currhashcode(), quote.get_currhashcode());
}

/// What an ISIN implies hangs on it: removing the ISIN takes back the
/// national number it carried, so a replacement derives its own rather
/// than sitting beside another instrument's.
#[test]
fn replacing_the_isin_takes_back_what_the_old_one_implied() {
    let mut order = OrderEvent::at(1);
    order.insert_securityid(id("ISIN", "US0378331005")).unwrap();
    order.finalize();
    assert_eq!(ids(&order), ["CUSIP:037833100", "ISIN:US0378331005"]);

    assert!(order.remove_securityid(&isin()).unwrap());
    assert!(ids(&order).is_empty());
    order.insert_securityid(id("ISIN", "GB0002634946")).unwrap();
    order.finalize();
    assert_eq!(ids(&order), ["ISIN:GB0002634946", "SEDOL:0263494"]);

    // Replacing the whole set states every identifier in it.
    let mut replaced = OrderEvent::at(1);
    replaced
        .insert_securityid(id("ISIN", "US0378331005"))
        .unwrap();
    replaced.finalize();
    replaced
        .set_securityids(order.get_securityids().clone())
        .unwrap();
    replaced.finalize();
    assert_eq!(ids(&replaced), ["ISIN:GB0002634946", "SEDOL:0263494"]);
    assert!(replaced.remove_securityid(&isin()).unwrap());
    assert_eq!(ids(&replaced), ["SEDOL:0263494"]);
}

/// A stated identifier replaces one the element only derived, and nothing
/// replaces a stated one: neither another statement nor a derivation. What
/// was stated outlives the ISIN.
#[test]
fn a_stated_identifier_replaces_a_derived_one_and_outlives_the_isin() {
    let mut order = Order::new();
    order.insert_securityid(id("ISIN", "US0378331005")).unwrap();
    order.finalize();
    assert!(order.insert_securityid(id("CUSIP", "594918104")).unwrap());
    order.finalize();
    assert_eq!(order.get_securityids().get("CUSIP"), Some("594918104"));
    assert!(!order.insert_securityid(id("CUSIP", "037833100")).unwrap());
    assert!(!order.derive_securityid(id("CUSIP", "037833100")));

    assert!(order.remove_securityid(&isin()).unwrap());
    assert_eq!(ids(&order), ["CUSIP:594918104"]);
}

/// A RIC is kept as it is written, case and all, under FIX's source `5`; one
/// a lifecycle learned is derived like any other source, so a stated RIC
/// replaces it and the ISIN it was learned under takes it back.
#[test]
fn a_ric_is_held_as_written_and_derived_like_any_source() {
    let mut quote = Quote::new();
    assert!(quote.insert_securityid(id("RIC", "ESc1")).unwrap());
    quote.finalize();
    assert_eq!(quote.get_securityids().get("5"), Some("ESc1"));
    assert!(SecurityId::new(SecType::read("RIC").unwrap(), "ESc 1").is_err());

    let mut order = Order::new();
    order.insert_securityid(id("ISIN", "GB0002634946")).unwrap();
    assert!(order.derive_securityid(id("RIC", "BAES.L")));
    order.finalize();
    assert_eq!(
        ids(&order),
        ["ISIN:GB0002634946", "RIC:BAES.L", "SEDOL:0263494"]
    );
    assert!(order.insert_securityid(id("RIC", "BAES.L")).unwrap());
    assert!(order.remove_securityid(&isin()).unwrap());
    assert_eq!(ids(&order), ["RIC:BAES.L"]);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Uuid;
    use yggdryl::graph::{Element, Order, OrderEvent};
    use yggdryl::internals::graph_facts;

    /// The four holders' sizes, first pinned when the slim `Market` trait
    /// landed: the nineteen market facts, then the clocks, then the boxed
    /// lanes and the three identifier maps each operation adds. It moved
    /// when price and quantity became what the element states:
    /// `Option<Decimal>` has no niche, so each costs sixteen bytes more
    /// than the zero that used to stand in. It moved again, by sixteen
    /// bytes each, when the market facts began to know which identifiers
    /// they only derived: one `u64` mask, padded to sixteen bytes. A moved
    /// number is a design answer, never a number to re-pin from a whole run.
    #[test]
    fn the_holders_are_the_sizes_the_build_reported_when_first_pinned() {
        assert_eq!(graph_facts::sizes(), [656, 832, 880, 1056]);
    }

    /// An undated holder's identity is RFC 9562 UUIDv8 over the code it
    /// digests to; a dated one's is ordered by its instant instead.
    #[test]
    fn an_undated_holder_is_identified_by_its_code_and_a_dated_one_by_its_instant() {
        let [market, event, operation, operation_event] =
            graph_facts::finalized("ORDER", 1_700_000_000_000_000_000);
        for (uuid, code) in [market, operation] {
            assert_eq!(uuid, Uuid::from_v8(u128::from(code)));
        }
        for (uuid, code) in [event, operation_event] {
            assert_ne!(uuid, Uuid::from_v8(u128::from(code)));
        }
        // An operation holder feeds only the operation facts it states, so
        // one stating none digests to its market's code.
        assert_eq!(market.1, operation.1);
        assert_eq!(event.1, operation_event.1);
    }

    /// A leaf continues its holder's digest with its kind: neither the
    /// undated nor the dated order digests to its holder's code.
    #[test]
    fn a_leaf_is_not_identified_as_its_holder() {
        let unix = 1_700_000_000_000_000_000;
        let [_, _, operation, operation_event] = graph_facts::finalized("ORDER", unix);
        let mut order = Order::new();
        order.set_crosscode("ORDER".to_owned());
        order.finalize();
        assert_ne!(order.get_currhashcode(), operation.1);
        let mut event = OrderEvent::at(unix);
        event.set_crosscode("ORDER".to_owned());
        event.finalize();
        assert_ne!(event.get_currhashcode(), operation_event.1);
    }

    /// An undated holder states no order; a dated one is after another by
    /// its instant.
    #[test]
    fn only_a_dated_holder_stands_after_another() {
        assert_eq!(graph_facts::is_after(1, 2), [false, true, false, true]);
        assert_eq!(graph_facts::is_after(2, 1), [false; 4]);
    }

    /// Every holder merges another statement of itself - its sources taken
    /// - and never a stranger's.
    #[test]
    fn a_holder_merges_only_another_statement_of_itself() {
        let source = Uuid::from_v8(9);
        for (merged, stranger) in graph_facts::merged(source) {
            assert_eq!(merged, Some(vec![source]));
            assert!(!stranger);
        }
    }
}
