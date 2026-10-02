"""The operation leaves and ``BookRef``: `python/src/graph/operation.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle
from typing import Any

import pytest

from yggdryl import Identifier, Identifiers, MarketDataKind, Scalar, Side, State, TimeInForce, graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def order_event(**facts: Any) -> graph.OrderEvent:
    """A buy order stating a fact from every one of the three column sets."""
    stated: dict[str, Any] = {
        "crosscode": "O-100",
        "seqnum": 3,
        "creaunix": CLOCK - 100_000_000_000,
        "recdunix": CLOCK - 50_000_000_000,
        "side": "BUYS",
        "price": D("101"),
        "currency": "USD",
        "quantity": 5,
        "ticker": "ACME",
        "timeinforce": "0",
        "identifiers": Identifiers([Identifier("orderid", "O-100")]),
        "securityids": [Identifier("isin", "US0378331005")],
        "fxrates": {"EUR": D("1.25")},
        "bidpx": D("100.5"),
        "bidqty": 7,
        "bidccy": "USD",
    }
    stated.update(facts)
    return graph.OrderEvent(CLOCK, **stated)


def test_an_order_event_reads_every_fact_back_typed() -> None:
    event = order_event()
    assert isinstance(event.curruuid, Scalar) and event.curruuid.kind == "uuid"
    assert isinstance(event.crossuuid, Scalar) and event.crossuuid.kind == "uuid"
    # A market element's cross code is stored as `{kind}:{side}:{base}`.
    assert event.crosscode == "10:1:O-100"
    assert isinstance(event.currhashcode, int) and isinstance(event.crosshashcode, int)
    assert event.srcuuids == []
    assert event.currunix == CLOCK
    assert event.state is State.UNKNOWN
    assert event.seqnum == 3
    assert event.creaunix == CLOCK - 100_000_000_000
    assert event.execunix is None
    assert event.recdunix == CLOCK - 50_000_000_000
    assert event.exprunix is None
    assert event.prevunix is None and event.prevuuid is None
    assert event.snapunix is None
    assert event.price is not None and event.price.as_py() == D("101")
    assert event.currency.as_py() == "USD"
    assert event.quantity is not None and event.quantity.as_py() == 5
    assert event.unit == ""
    assert event.side is Side.BUYS
    # A US ISIN embeds its CUSIP, which the core derives beside it.
    assert [str(id) for id in event.securityids] == [
        "cusip=037833100",
        "derived:cusip=037833100",
        "isin=US0378331005",
    ]
    assert event.securityids.get("isin") == "US0378331005"
    assert event.isincode == "US0378331005"
    assert {target: rate.as_py() for target, rate in event.fxrates.items()} == {"EUR": D("1.25")}
    assert event.bidpx is not None and event.bidpx.as_py() == D("100.5")
    assert event.bidqty is not None and event.bidqty.as_py() == 7
    assert event.bidccy is not None and event.bidccy.as_py() == "USD"
    assert event.askpx is None and event.askqty is None and event.askccy is None
    assert event.cficode is None and event.miccode is None
    assert event.lastpx is None and event.lastqty is None
    assert event.avgpx is None and event.cumqty is None and event.leavesqty is None
    assert event.prevpx is None and event.prevqty is None
    assert event.spotrate is None and event.forwardpoints is None
    assert event.ticker == "ACME"
    assert event.metadata == {}
    assert event.timeinforce is TimeInForce.DAY
    assert event.tradable is None
    assert event.identifiers == Identifiers([Identifier("orderid", "O-100")])
    # The party ids an operation names, by role and source: none stated here.
    assert len(event.partyids) == 0 and not event.partyids
    assert event.kind == "order"
    assert event.marketdatakind is MarketDataKind.ORDR
    for retired in ("marketoperationid", "userids", "accountids", "secaltids", "altids", "parties", "bid", "ask"):
        assert not hasattr(event, retired), retired
    assert not event.is_execution
    assert event.book is None and event.action is None
    assert event.scope == "" and not event.is_full_snapshot


def test_a_side_stated_as_none_is_unknown_never_none() -> None:
    event = graph.OrderEvent(CLOCK, crosscode="O-1")
    assert event.side is Side.UNKN
    # An element stating no side states side 0 in its stored cross code.
    assert event.crosscode == "10:0:O-1"
    assert event.isincode is None and event.fxrates == {}
    assert graph.OrderEvent(CLOCK, side=Side.SELL).side is Side.SELL
    assert graph.QuoteEvent(CLOCK).marketdatakind is MarketDataKind.QUOT
    assert graph.ExecutionEvent(CLOCK).marketdatakind is MarketDataKind.EXEC
    assert graph.Execution().marketdatakind is MarketDataKind.EXEC


def test_an_unknown_fact_is_refused_by_name() -> None:
    with pytest.raises(ValueError, match='OrderEvent states no fact "bogus"'):
        graph.OrderEvent(CLOCK, bogus=1)
    with pytest.raises(ValueError, match='Quote states no fact "bogus"'):
        graph.Quote(bogus=1)
    # A name folds as the column enums read it.
    assert graph.OrderEvent(CLOCK, PRICE=1).price == graph.OrderEvent(CLOCK, price=1).price


def test_the_three_identifier_sets_are_one_name_each_on_every_leaf() -> None:
    event = graph.OrderEvent(
        CLOCK,
        crosscode="O-1",
        securityids=[Identifier("isin", "us0378331005")],
        identifiers=[Identifier("orderid", "O-9"), Identifier("ullink:instrumentid", "dbi;X")],
        partyids=[Identifier("proprietary:executingtrader", "T-1")],
    )
    assert event.securityids.get("isin") == "US0378331005"
    assert [str(id) for id in event.identifiers] == ["instrumentid=dbi;X", "orderid=O-9", "ullink:instrumentid=dbi;X"]
    assert [str(id) for id in event.partyids] == ["executingtrader=T-1", "proprietary:executingtrader=T-1"]
    # A source and a type are keys: a set holds one value per `src:type`.
    assert event.identifiers.get_from("ullink:instrumentid") == "dbi;X"
    assert event.identifiers.get_from("oms:instrumentid") is None
    assert event.identifiers.get_from("instrumentid") == "dbi;X", "a named source fills the base key"
    # The retired spellings are no fact of any leaf.
    for retired in ("secaltids", "altids", "parties"):
        with pytest.raises(ValueError, match=f'OrderEvent states no fact "{retired}"'):
            graph.OrderEvent(CLOCK, **{retired: []})


def test_a_fact_is_checked_by_its_columns_field() -> None:
    with pytest.raises(ValueError, match=r"\$\.price"):
        graph.OrderEvent(CLOCK, price="not a number")
    with pytest.raises(ValueError, match="side"):
        graph.Order(side="SIDEWAYS")


def test_ellipsis_is_skipped_and_none_clears() -> None:
    # `...` is a fact not given: the event states nothing of it.
    assert graph.OrderEvent(CLOCK, crosscode="X", ticker=...) == graph.OrderEvent(CLOCK, crosscode="X")
    assert order_event(ticker=...).ticker is None
    stated = {
        "crosscode": "X",
        "price": 1,
        "ticker": "T",
        "timeinforce": "0",
        "identifiers": [Identifier("orderid", "X")],
    }
    cleared = graph.OrderEvent(CLOCK, **{**stated, "ticker": None, "price": None, "timeinforce": None, "identifiers": None})
    assert cleared.ticker is None and cleared.price is None and cleared.timeinforce is None
    assert cleared.identifiers == Identifiers()
    assert cleared == graph.OrderEvent(CLOCK, crosscode="X")
    assert cleared != graph.OrderEvent(CLOCK, **stated)
    assert cleared != order_event()
    assert graph.OrderEvent(CLOCK, book=...) == graph.OrderEvent(CLOCK, book=None)


def test_an_undated_element_states_no_clock_state_or_chain() -> None:
    element = graph.Order(crosscode="O-1", price=D("10"), side="SELL", curruuid=...)
    assert element.crosscode == "10:2:O-1" and element.kind == "order"
    assert element.side is Side.SELL
    for name in ("currunix", "state", "seqnum", "prevuuid"):
        with pytest.raises(ValueError, match="an undated element has no clock, state or chain"):
            graph.Order(**{name: 1})
    assert not hasattr(element, "currunix")


def test_a_derived_identity_is_refused_by_name() -> None:
    # `finalize` derives the four identities, so a stated one would be
    # overwritten: it is refused on the dated and the undated leaf alike.
    uuid = "00000000-0000-8000-8000-000000000001"
    for name, value in (("curruuid", uuid), ("crossuuid", uuid), ("currhashcode", 7), ("crosshashcode", 7)):
        with pytest.raises(ValueError, match=f'Order states no fact "{name}": an identity is derived'):
            graph.Order(**{name: value})
        with pytest.raises(ValueError, match=f'OrderEvent states no fact "{name}": an identity is derived'):
            graph.OrderEvent(CLOCK, **{name: value})
    # The two element facts `finalize` keeps are stated.
    assert graph.Order(crosscode="O-1", srcuuids=[]).crosscode == "10:0:O-1"


def test_currunix_is_stated_once() -> None:
    with pytest.raises(TypeError, match="multiple values for argument 'currunix'"):
        graph.OrderEvent(1, currunix=5)
    # A folded spelling reaches the facts, and is refused there by name.
    with pytest.raises(ValueError, match="OrderEvent states currunix once, as its first argument"):
        graph.OrderEvent(1, CURRUNIX=5)


def test_at_dates_an_element_and_into_element_undates_it() -> None:
    element = graph.Quote(crosscode="Q-1", side="SELL", price=D("102"))
    event = element.at(CLOCK)
    assert isinstance(event, graph.QuoteEvent)
    assert (event.currunix, event.crosscode, event.price) == (CLOCK, "14:2:Q-1", element.price)
    back = event.into_element()
    assert isinstance(back, graph.Quote)
    assert back == element
    assert graph.Execution().at(CLOCK).kind == "execution"
    assert graph.ExecutionEvent(CLOCK).into_element().kind == "execution"


def test_the_kind_is_the_type() -> None:
    same = {"crosscode": "X", "price": 1}
    assert graph.OrderEvent(CLOCK, **same).curruuid != graph.QuoteEvent(CLOCK, **same).curruuid
    assert graph.ExecutionEvent(CLOCK).is_execution
    assert not graph.QuoteEvent(CLOCK).is_execution
    assert graph.Order(**same) != graph.Quote(**same)  # type: ignore[comparison-overlap]


def test_book_states_the_control_facts() -> None:
    control = graph.BookRef(action="0", scope="S", position=1, entry_px=D("1"), entry_size=2)
    event = graph.QuoteEvent(CLOCK, book=control, crosscode="Q")
    assert event.book == control
    assert (event.action, event.scope, event.is_full_snapshot) == ("0", "S", False)
    assert event.with_book(control) == event
    plain = graph.QuoteEvent(CLOCK, crosscode="Q")
    assert plain.with_book(control) == event
    assert plain.book is None
    assert graph.OrderEvent(CLOCK, book=graph.BookRef(action="snapshot")).is_full_snapshot
    with pytest.raises(TypeError, match="expected a BookRef for OrderEvent.book"):
        graph.OrderEvent(CLOCK, book=1)


def test_an_order_follows_the_order_it_replaces() -> None:
    first = order_event(seqnum=...)
    later = graph.OrderEvent(CLOCK + 1, crosscode="O-100", side="BUYS", price=D("100"), quantity=4)
    followed = later.with_previous(first)
    assert followed is not None
    assert followed.prevuuid == first.curruuid
    assert followed.prevunix == first.currunix
    # A later instant keeps its own place.
    assert followed.seqnum == 0
    assert later.prevuuid is None  # immutable: the verb answered a new event
    assert first.is_before(later) and later.is_after(first)
    assert not first.is_after(first)
    # Following refuses what it cannot follow, and a fold that changes
    # nothing answers `None`.
    assert first.with_previous(later) is None
    assert first.merge_with(first) is None
    assert followed.restating(followed) == followed


def test_following_crosses_no_kind() -> None:
    order = order_event()
    execution = graph.ExecutionEvent(CLOCK + 1, crosscode="O-100")
    with pytest.raises(TypeError):
        execution.with_previous(order)  # type: ignore[arg-type]


@pytest.mark.parametrize(
    "leaf",
    [
        order_event(),
        graph.QuoteEvent(CLOCK, crosscode="Q", askpx=1, askccy="EUR", fxrates={"USD": D("0.8")}),
        graph.ExecutionEvent(CLOCK, crosscode="E", lastpx=D("1.5"), lastqty=3, state="FILLED"),
        # A row states the book scope alone: the other controls are walk-time
        # facts no column carries (A1), so a pickle keeps the scope.
        graph.QuoteEvent(CLOCK, book=graph.BookRef(scope="S")),
        graph.Order(crosscode="O", metadata={"k": "v"}),
        graph.Quote(),
        graph.Execution(securityids=[Identifier("isin", "US0378331005")]),
    ],
    ids=lambda leaf: type(leaf).__name__,
)
def test_equality_hash_repr_copy_pickle(leaf: Any) -> None:
    twin = pickle.loads(pickle.dumps(leaf))
    assert type(twin) is type(leaf)
    assert twin == leaf and hash(twin) == hash(leaf)
    assert twin.curruuid == leaf.curruuid and twin.currhashcode == leaf.currhashcode
    assert copy.copy(leaf) == leaf and copy.deepcopy(leaf) == leaf
    assert repr(leaf).startswith(f"{type(leaf).__name__}({leaf.curruuid.as_py()}")
    assert leaf != object()
    assert len({leaf, twin}) == 1


def test_the_leaves_are_immutable() -> None:
    event = order_event()
    with pytest.raises(AttributeError):
        event.price = None  # type: ignore[misc]


class TestBookRef:
    def test_every_slot_reads_back(self) -> None:
        control = graph.BookRef(action="1", scope="S", position=3, entry_px=D("2"), entry_size=5)
        assert control.action == "1" and control.scope == "S" and control.position == 3
        assert control.entry_px is not None and control.entry_px.as_py() == D("2")
        assert control.entry_size is not None and control.entry_size.as_py() == 5
        assert control.is_stated() and control.is_partial()
        assert not control.is_range_delete()
        assert not graph.BookRef().is_stated()

    def test_an_unknown_action_is_refused_listing_every_spelling(self) -> None:
        with pytest.raises(ValueError, match="unknown MdUpdateAction"):
            graph.BookRef(action="9")

    def test_equality_hash_repr_copy_pickle(self) -> None:
        control = graph.BookRef(action="snapshot", scope="S")
        assert pickle.loads(pickle.dumps(control)) == control
        assert hash(pickle.loads(pickle.dumps(control))) == hash(control)
        assert copy.copy(control) == control and copy.deepcopy(control) == control
        assert repr(control) == (
            'BookRef(action="snapshot", scope="S", position=None, entry_px=None, entry_size=None)'
        )


def test_a_market_elements_cross_code_is_stored_as_its_kind_its_side_and_its_base() -> None:
    # The kind is `MarketDataKind.code()` - order 10, quotation 14, execution 8,
    # trade 21, book 3 - and the side `Side.code()` for a sided kind alone.
    assert graph.OrderEvent(CLOCK, crosscode="ORD-1", side="BUYS").crosscode == "10:1:ORD-1"
    assert graph.OrderEvent(CLOCK, crosscode="ORD-1", side="SELL").crosscode == "10:2:ORD-1"
    assert graph.OrderEvent(CLOCK, crosscode="ORD-1").crosscode == "10:0:ORD-1", "no side stated, side 0"
    assert graph.QuoteEvent(CLOCK, crosscode="Q-1", side="BUYS").crosscode == "14:1:Q-1"
    assert graph.ExecutionEvent(CLOCK, crosscode="E-1", side="SELL").crosscode == "8:2:E-1"
    # A book is no sided kind: it states side 0 whatever side it takes, and its
    # base may hold colons.
    assert graph.BookEvent(CLOCK, "AAPL").crosscode == "3:0:AAPL"
    assert graph.BookEvent(CLOCK, "XNAS:ESVUFR").crosscode == "3:0:XNAS:ESVUFR"
    assert graph.OrderEvent(CLOCK, crosscode="ORD-1", side="BUYS").crosscode.split(":", 2) == ["10", "1", "ORD-1"]
    # A code already prefixed with another kind or side has the prefix replaced,
    # and an empty code stays empty.
    assert graph.OrderEvent(CLOCK, crosscode="14:2:ORD-1", side="BUYS").crosscode == "10:1:ORD-1"
    assert graph.OrderEvent(CLOCK, crosscode="10:1:ORD-1", side="BUYS").crosscode == "10:1:ORD-1"
    assert graph.OrderEvent(CLOCK, crosscode="", side="BUYS").crosscode == ""
    # The identities derive from the stored code: one base, two sides or two
    # kinds, are different chains.
    buy = graph.OrderEvent(CLOCK, crosscode="ORD-1", side="BUYS")
    sell = graph.OrderEvent(CLOCK, crosscode="ORD-1", side="SELL")
    fill = graph.ExecutionEvent(CLOCK, crosscode="ORD-1", side="BUYS")
    assert len({buy.crossuuid, sell.crossuuid, fill.crossuuid}) == 3
    assert len({buy.crosshashcode, sell.crosshashcode, fill.crosshashcode}) == 3
    assert buy.crossuuid == graph.OrderEvent(CLOCK + 1, crosscode="10:1:ORD-1", side="BUYS").crossuuid


def test_a_sequence_of_identifiers_states_the_map_it_makes() -> None:
    ids = [Identifier("orderid", "O-9"), Identifier("ullink:instrumentid", "dbi;X"), Identifier("orderid", "O-9")]
    event = graph.OrderEvent(CLOCK, identifiers=ids)
    assert event.identifiers == Identifiers(ids), "one identifier stated twice is one"
    assert [id.key for id in event.identifiers] == ["instrumentid", "orderid", "ullink:instrumentid"]
    assert event.identifiers.get_from("orderid") == "O-9"
    # Two values under one key are two readings, refused naming the place.
    with pytest.raises(ValueError, match=r"\$\[1\].*orderid"):
        graph.OrderEvent(CLOCK, identifiers=[Identifier("orderid", "O-9"), Identifier("orderid", "O-10")])
    assert graph.OrderEvent(CLOCK, identifiers=tuple(ids)).identifiers == event.identifiers
    assert graph.OrderEvent(CLOCK, identifiers=Identifiers(ids)).identifiers == event.identifiers
    assert graph.OrderEvent(CLOCK, identifiers=[]).identifiers == Identifiers()
    # The three sets state alike, and a sequence of anything else is no map.
    quote = graph.QuoteEvent(CLOCK, partyids=[Identifier("proprietary:executingtrader", "T-1")])
    assert quote.partyids.get_from("proprietary:executingtrader") == "T-1"
    with pytest.raises(ValueError):
        graph.OrderEvent(CLOCK, identifiers=[ids[0], 1])
