//! `rust/src/graph/facts.rs`: the four crate-private holders - market,
//! market event, operation, operation event - each leaf holds the one of
//! its role. A caller reads their facts through the leaves, so what a
//! holder answers on its own is pinned through `yggdryl::internals`.

use yggdryl::graph::{Element, Order, Quote};

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
    /// than the zero that used to stand in. A moved number is a design
    /// answer, never a number to re-pin from a whole run.
    #[test]
    fn the_holders_are_the_sizes_the_build_reported_when_first_pinned() {
        assert_eq!(graph_facts::sizes(), [640, 816, 864, 1040]);
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
