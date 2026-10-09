//! `rust/market/src/graph/facts.rs`: the four crate-private holders - market,
//! market event, operation, operation event - each leaf holds the one of
//! its role. A caller reads their facts through the leaves, so what a
//! holder answers on its own is pinned through `yggdryl_market::internals`.

use yggdryl_market::IdKey;

use yggdryl::Decimal;
use yggdryl::graph::Element;
use yggdryl_market::graph::{BookEvent, Market, Order, OrderEvent, Quote};
use yggdryl_market::{IdType, Identifier, Identifiers, Side};

/// One security identifier of `kind` from `base`, validated by its type.
fn id(kind: IdType, code: &str) -> Identifier {
    Identifier::new(IdKey::base(kind), code).unwrap()
}

/// Every identifier an element holds, as `src:type=code`, in key order.
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
    crate::install::installed();
    let mut order = Order::new();
    order.set_crosscode("ORDER".to_owned());
    order.finalize();
    let mut quote = Quote::new();
    quote.set_crosscode("ORDER".to_owned());
    quote.finalize();
    assert_ne!(order.get_hashcode(), quote.get_hashcode());
}

/// What an ISIN implies hangs on it: removing the ISIN takes back the
/// national number it carried, so a replacement derives its own rather
/// than sitting beside another instrument's.
#[test]
fn replacing_the_isin_takes_back_what_the_old_one_implied() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order
        .insert_securityid(id(IdType::Isin, "US0378331005"))
        .unwrap();
    order.finalize();
    assert_eq!(
        ids(&order),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );

    assert!(order.remove_securityid(&IdKey::base(IdType::Isin)).unwrap());
    assert!(ids(&order).is_empty());
    order
        .insert_securityid(id(IdType::Isin, "GB0002634946"))
        .unwrap();
    order.finalize();
    assert_eq!(
        ids(&order),
        [
            "derived:sedol=0263494",
            "isin=GB0002634946",
            "sedol=0263494"
        ]
    );

    // Replacing the whole set holds each identifier under the source it
    // carries: what it carries as derived stays derived, so the ISIN still
    // takes it back, and what it carries as stated outlives the ISIN.
    let mut replaced = OrderEvent::at(1);
    replaced
        .insert_securityid(id(IdType::Isin, "US0378331005"))
        .unwrap();
    replaced.finalize();
    replaced
        .set_securityids(order.get_securityids().clone(), true)
        .unwrap();
    replaced.finalize();
    assert_eq!(
        ids(&replaced),
        [
            "derived:sedol=0263494",
            "isin=GB0002634946",
            "sedol=0263494"
        ]
    );
    assert!(
        replaced
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert!(ids(&replaced).is_empty());

    let stated: Identifiers = [
        id(IdType::Isin, "GB0002634946"),
        id(IdType::Sedol, "0263494"),
    ]
    .into_iter()
    .collect();
    replaced.set_securityids(stated, true).unwrap();
    replaced.finalize();
    assert_eq!(ids(&replaced), ["isin=GB0002634946", "sedol=0263494"]);
    assert!(
        replaced
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert_eq!(ids(&replaced), ["sedol=0263494"]);
}

/// A stated identifier replaces one the element only derived, and nothing
/// replaces a stated one: neither another statement nor a derivation. What
/// was stated outlives the ISIN.
#[test]
fn a_stated_identifier_replaces_a_derived_one_and_outlives_the_isin() {
    crate::install::installed();
    let mut order = Order::new();
    order
        .insert_securityid(id(IdType::Isin, "US0378331005"))
        .unwrap();
    order.finalize();
    assert!(
        order
            .insert_securityid(id(IdType::Cusip, "594918104"))
            .unwrap()
    );
    order.finalize();
    assert_eq!(
        order.get_securityids().get(&IdType::Cusip),
        Some("594918104")
    );
    assert!(
        !order
            .insert_securityid(id(IdType::Cusip, "037833100"))
            .unwrap()
    );
    assert!(!order.derive_securityid(&IdType::Cusip, "037833100"));

    assert!(order.remove_securityid(&IdKey::base(IdType::Isin)).unwrap());
    assert_eq!(ids(&order), ["cusip=594918104"]);
}

/// A RIC is kept as it is written, case and all, under the type FIX's source
/// `5` reads as; one a lifecycle learned is derived like any other source, so
/// a stated RIC replaces it and the ISIN it was learned under takes it back.
#[test]
fn a_ric_is_held_as_written_and_derived_like_any_source() {
    crate::install::installed();
    let mut quote = Quote::new();
    assert!(quote.insert_securityid(id(IdType::Ric, "ESc1")).unwrap());
    quote.finalize();
    assert_eq!(quote.get_securityids().get(&IdType::Ric), Some("ESc1"));
    assert_eq!(IdType::from_security_source("5").unwrap(), IdType::Ric);
    assert!(Identifier::new(IdKey::base(IdType::Ric), "ESc 1").is_err());

    let mut order = Order::new();
    order
        .insert_securityid(id(IdType::Isin, "GB0002634946"))
        .unwrap();
    assert!(order.derive_securityid(&IdType::Ric, "BAES.L"));
    order.finalize();
    assert_eq!(
        ids(&order),
        [
            "derived:ric=BAES.L",
            "derived:sedol=0263494",
            "isin=GB0002634946",
            "ric=BAES.L",
            "sedol=0263494"
        ]
    );
    assert!(order.insert_securityid(id(IdType::Ric, "BAES.L")).unwrap());
    assert!(order.remove_securityid(&IdKey::base(IdType::Isin)).unwrap());
    assert_eq!(ids(&order), ["ric=BAES.L"]);
}

/// A book's legs are its sides' best levels, which the book settles: its
/// side, `BOTH`, moves no leg and quotes no price, so a one-sided book
/// whose price is its bid keeps both when it settles to `BOTH`. A tag set
/// on a book-kind fact quotes as any tag does - a book message's entry
/// passes through this kind before it is refiled as the quote it is, and
/// the side it tags must quote its price onto that leg - and the book
/// settles to `BOTH` whatever it was set to.
#[test]
fn a_book_settles_to_both_sides_and_that_side_moves_no_leg() {
    crate::install::installed();
    let price = Some(Decimal::from_int(99));
    let mut book = BookEvent::new(1, "ACME");
    book.set_bidpx(price, true);
    book.set_bidqty(Some(Decimal::from_int(10)), true);
    book.set_side(Side::Buy, true);
    assert_eq!(book.get_side(), Side::Buy);
    assert_eq!(book.get_price(), price, "a tag quotes the leg it names");
    assert_eq!(book.get_crosscode(), "3:0:ACME");

    book.set_side(Side::Both, true);
    assert_eq!(book.get_price(), price, "the leg it read stays");
    assert_eq!(book.get_bidpx(), price);
    assert_eq!(book.get_bidqty(), Some(Decimal::from_int(10)));
    book.set_side(Side::Sell, true);
    assert_eq!(book.get_bidpx(), price, "a moved tag withdraws no leg");
    book.finalize();
    assert_eq!(book.get_side(), Side::Both, "a book holds both sides");
    assert_eq!((book.get_price(), book.get_bidpx()), (price, price));
    assert_eq!(book.get_crosscode(), "3:0:ACME");
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Uuid;
    use yggdryl::graph::Element;
    use yggdryl_market::graph::{Order, OrderEvent};
    use yggdryl_market::internals::graph_facts;

    /// The four holders' sizes, first pinned when the slim `Market` trait
    /// landed: the nineteen market facts, then the clocks, then the boxed
    /// lanes and the identifier maps each operation adds. It moved
    /// when price and quantity became what the element states:
    /// `Option<Decimal>` has no niche, so each costs sixteen bytes more
    /// than the zero that used to stand in. It moved again, by sixteen
    /// bytes each, when the market facts began to know which identifiers
    /// they only derived: one `u64` mask, padded to sixteen bytes. The two
    /// dated holders moved back by sixteen bytes when their `State` became
    /// an `i32` member rather than a twenty-four-byte code string: twenty
    /// bytes fewer, padded to the sixteen-byte alignment. They moved again
    /// when the market facts gained their FX rates and the operation facts
    /// lost their category: the two market holders by exactly sixteen - an
    /// `Option<Box<[FxRate]>>` - and the operation's own facts from 224
    /// bytes to 216 - an `Option<i32>` `marketoperationid`, eight - so the
    /// two operation holders are 672 + 216 = 888 and 832 + 216 = 1048, each
    /// padded to sixteen. They moved once more when the rates became a
    /// target-to-rate map and the accounts and users left the operation:
    /// the two market holders by sixteen - an `Option<Box<FxRates>>` is
    /// eight, and the eight left over fall inside the padding the holders
    /// already had, so 656 and 816 - and the operation's own facts from 216
    /// bytes to 104, two fifty-six-byte `IdMap`s fewer, so the two operation
    /// holders are 656 + 104 = 760 and 816 + 104 = 920, each padded to
    /// sixteen. They moved again when the bid and ask lanes left the
    /// operation: the operation's own facts from 104 bytes to 88, two boxed
    /// lanes of eight fewer, so the two operation holders are 656 + 88 = 744
    /// and 816 + 88 = 904, each padded to sixteen. They moved again when the
    /// market facts gained the stated bid and ask (A20): one
    /// `Option<Box<BidAsk>>` of eight, padded to sixteen, so 672 and 832, and
    /// the operation holders 672 + 88 = 760 and 832 + 88 = 920, each padded
    /// to sixteen. They moved again when the operation gained the accounts
    /// its parties name: the operation's own facts from 88 bytes to 136, one
    /// forty-eight-byte `IdMap` more, so the two operation holders are
    /// 672 + 136 = 808 and 832 + 136 = 968, each padded to sixteen; the
    /// market facts' kind stamp fell inside their padding. They moved again
    /// when the execution clock left the event for the market: one
    /// `Option<i64>` of sixteen moved from the dated holder into the market
    /// facts, so those grew to 688 and the undated operation holder to
    /// 688 + 136 = 824, padded to 832, while the dated holders kept 832 and
    /// 976 - the same facts, one of them held one level down. They moved
    /// again when the time in force became an enum: a one-byte member where
    /// a twenty-four-byte optional code stood, so the operation's own facts
    /// fell from 136 bytes to 128 and the two operation holders are
    /// 688 + 128 = 816 and 832 + 128 = 960. They moved again when the
    /// identifiers became `Identifiers`, one sorted vector of 24 bytes each.
    /// Each market holder gave up a 56-byte `SecurityIds` and its eight-byte
    /// derived mask - the source is the identifier's own now - for one set,
    /// 64 - 24 = 40 fewer, 32 after padding to sixteen, so 656 and 800; the
    /// operation's own facts gave up two 56-byte `IdMap`s for two sets,
    /// exactly 2 * (56 - 24) = 64 fewer, from 128 to 64, so the two
    /// operation holders are 656 + 64 = 720 and 800 + 64 = 864. A moved
    /// number is a design answer, never a number to re-pin from a whole run.
    #[test]
    fn the_holders_are_the_sizes_the_build_reported_when_first_pinned() {
        crate::install::installed();
        assert_eq!(graph_facts::sizes(), [656, 800, 720, 864]);
    }

    /// An undated holder's identity is RFC 9562 UUIDv8 over the code it
    /// digests to; a dated one's is ordered by its instant instead.
    #[test]
    fn an_undated_holder_is_identified_by_its_code_and_a_dated_one_by_its_instant() {
        crate::install::installed();
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
        crate::install::installed();
        let unix = 1_700_000_000_000_000_000;
        let [_, _, operation, operation_event] = graph_facts::finalized("ORDER", unix);
        let mut order = Order::new();
        order.set_crosscode("ORDER".to_owned());
        order.finalize();
        assert_ne!(order.get_hashcode(), operation.1);
        let mut event = OrderEvent::at(unix);
        event.set_crosscode("ORDER".to_owned());
        event.finalize();
        assert_ne!(event.get_hashcode(), operation_event.1);
    }

    /// An undated holder states no order; a dated one is after another by
    /// its instant.
    #[test]
    fn only_a_dated_holder_stands_after_another() {
        crate::install::installed();
        assert_eq!(graph_facts::is_after(1, 2), [false, true, false, true]);
        assert_eq!(graph_facts::is_after(2, 1), [false; 4]);
    }

    /// Every holder merges another statement of itself - its sources taken
    /// - and never a stranger's.
    #[test]
    fn a_holder_merges_only_another_statement_of_itself() {
        crate::install::installed();
        let source = Uuid::from_v8(9);
        for (merged, stranger) in graph_facts::merged(source) {
            assert_eq!(merged, Some(vec![source]));
            assert!(!stranger);
        }
    }
}
