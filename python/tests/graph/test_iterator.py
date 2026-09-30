"""``graph.EventIterator``: `python/src/graph/iterator.rs`."""

from __future__ import annotations

import decimal
from collections.abc import Iterator
from typing import Any

import pytest

from yggdryl import Identifier, Identifiers, State, graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def order(clock: int, state: str = "NEW", **facts: Any) -> graph.OrderEvent:
    return graph.OrderEvent(clock, crosscode="O-1", side="BUYS", price=D("101"), quantity=10, state=state, **facts)


def test_an_order_chains_to_the_live_order_it_follows() -> None:
    first, second = order(CLOCK), order(CLOCK + 1, state="REPLACED", leavesqty=...)
    walked = list(graph.EventIterator([first, second]))
    assert [type(data) for data in walked] == [graph.MarketData, graph.MarketData]
    head = walked[0].as_order_event()
    tail = walked[1].as_order_event()
    assert head is not None and tail is not None
    # The walk dates a chain's creation where no event states it: the first
    # event its own instant, every later one the chain's.
    assert head.seqnum == 0 and head.prevuuid is None
    assert first.creaunix is None and head.creaunix == CLOCK and tail.creaunix == CLOCK
    assert tail.prevuuid == first.curruuid
    # A later instant keeps its own place, even one nanosecond on.
    assert tail.prevunix == CLOCK and tail.seqnum == 0


def test_an_execution_and_every_other_leaf_walk_through() -> None:
    first = order(CLOCK)
    # A millisecond later: two instants in one millisecond share an identity.
    execution = graph.ExecutionEvent(CLOCK + 1_000_000, crosscode="O-1", side="BUYS", lastpx=D("101"), lastqty=10)
    book = graph.BookEvent(CLOCK + 2_000_000, "ACME")
    walked = list(graph.EventIterator([first, execution, book, graph.Order(crosscode="O-1")]))
    assert [data.kind for data in walked] == ["order_event", "execution_event", "book_event", "order"]
    # A book is yielded unchanged, in place.
    assert walked[2].as_book_event() == book
    assert walked[3].as_order() == graph.Order(crosscode="O-1")
    # A chain holds one market data kind: the stored cross code leads with
    # the kind, so the execution is a chain - and a cross identity - of its
    # own, and never follows the order it fills.
    fill = walked[1].as_execution_event()
    assert fill is not None and fill.crosscode == "8:1:O-1" and first.crosscode == "10:1:O-1"
    assert fill.crossuuid != first.crossuuid
    assert fill.prevuuid is None and fill.seqnum == 0


def test_unsorted_items_are_sorted_first() -> None:
    first, second = order(CLOCK), order(CLOCK + 1, state="REPLACED")
    walked = [data.as_order_event() for data in graph.EventIterator([second, first], sorted=False)]
    assert [event.currunix for event in walked if event is not None] == [CLOCK, CLOCK + 1]


def test_alive_and_the_snapshot_grid() -> None:
    walk = graph.EventIterator([order(CLOCK, exprunix=CLOCK + 10)], snapshot_ns=5)
    assert walk.snapshot_ns == 5
    walked = [data.as_order_event() for data in walk]
    # Each view is dated at its tick and keeps the instant the order was
    # stated at.
    assert [
        (event.currunix, event.snapunix) for event in walked if event is not None
    ] == [(CLOCK, None), (CLOCK, CLOCK), (CLOCK + 5, CLOCK), (CLOCK + 10, None)]
    assert walked[-1] is not None and walked[-1].state is State.EXPIRED
    assert walk.alive() == []
    live = graph.EventIterator([order(CLOCK)])
    assert live.snapshot_ns is None
    list(live)
    # A sided order's stored cross code states its kind and its side.
    assert [data.crosscode for data in live.alive()] == ["10:1:O-1"]
    assert all(isinstance(data, graph.MarketData) for data in live.alive())


def test_a_view_is_the_live_event_as_of_its_tick() -> None:
    ms = 1_000_000
    first = order(CLOCK)
    second = order(CLOCK + ms + ms // 2, state="REPLACED", leavesqty=..., exprunix=CLOCK + 3 * ms)
    walked = [data.as_order_event() for data in graph.EventIterator([first, second], snapshot_ns=ms)]
    events = [event for event in walked if event is not None]
    sources = [event for event in events if event.snapunix is None]
    views = [event for event in events if event.snapunix is not None]
    assert [view.currunix for view in views] == [CLOCK, CLOCK + ms, CLOCK + 2 * ms]
    for view in views:
        tick = view.currunix
        source = [held for held in sources if held.currunix <= tick][-1]
        # Dated at its tick, so its identity is the one that tick derives -
        # a row of its own - while its content, its place and its cross
        # element are the live event's, and its snapshot instant the one
        # the live event was stated at.
        assert view.snapunix == source.currunix
        assert view.currhashcode == source.currhashcode
        assert (view.seqnum, view.prevuuid, view.crossuuid) == (source.seqnum, source.prevuuid, source.crossuuid)
        assert (view.curruuid == source.curruuid) == (tick == source.currunix)
    # The tick past the replacement views it: a later instant keeps its own
    # place, the chain it follows kept.
    assert views[-1].prevuuid == first.curruuid and views[-1].seqnum == 0
    assert views[1].curruuid != first.curruuid


def test_a_python_failure_is_raised_as_itself() -> None:
    def items() -> Iterator[Any]:
        yield order(CLOCK)
        raise RuntimeError("the source gave up")

    walk = graph.EventIterator(items())
    assert next(walk).kind == "order_event"
    with pytest.raises(RuntimeError, match="the source gave up"):
        next(walk)
    # Unsorted, the whole source is read at once, so the failure is too.
    with pytest.raises(RuntimeError, match="the source gave up"):
        graph.EventIterator(items(), sorted=False)
    with pytest.raises(TypeError, match="expected MarketData, a market leaf or a FixMsg, got int"):
        list(graph.EventIterator([1]))  # type: ignore[list-item]


def test_the_walk_is_unhashable() -> None:
    with pytest.raises(TypeError):
        hash(graph.EventIterator([]))


def named(clock: int, kind: str, value: str, *parents: tuple[str, str], state: str = "NEW") -> graph.OrderEvent:
    """An order stating one `fix` identifier of `kind` and its stated `parents`, under one cross code."""
    ids = Identifiers([Identifier("fix", kind, value), *(Identifier("fix", p, v) for p, v in parents)])
    return graph.OrderEvent(clock, crosscode="O-100", side="BUYS", state=state, identifiers=ids)


def held(event: graph.OrderEvent | None, *kinds: str) -> list[str | None]:
    assert event is not None
    return [event.identifiers.get_from("fix", kind) for kind in kinds]


def test_a_walk_carries_the_parents_of_each_identifier_along_its_chain() -> None:
    # `orderid` A, B, C, D ends with `parentorderid` C and `origorderid` A:
    # the first parent is the previous value, the last the chain's first.
    states = ["NEW", "REPLACED", "REPLACED", "REPLACED"]
    orders = [named(CLOCK + at, "orderid", value, state=state) for at, (value, state) in enumerate(zip("ABCD", states))]
    walked = [data.as_order_event() for data in graph.EventIterator(orders)]
    assert [held(event, "orderid", "parentorderid", "origorderid") for event in walked] == [
        ["A", None, None],
        ["B", "A", "A"],
        ["C", "B", "A"],
        ["D", "C", "A"],
    ]
    # One chain: each event follows the one before.
    assert all(
        later is not None and earlier is not None and later.prevuuid == earlier.curruuid
        for earlier, later in zip(walked, walked[1:])
    )

    # A restatement of the live value moves neither parent.
    restated = [
        data.as_order_event()
        for data in graph.EventIterator(
            [named(CLOCK, "orderid", "A"), named(CLOCK + 1, "orderid", "B"), named(CLOCK + 2, "orderid", "B")]
        )
    ]
    assert held(restated[2], "parentorderid", "origorderid") == ["A", "A"]

    # A client order identifier has one parent, the previous value.
    replaced = [
        data.as_order_event()
        for data in graph.EventIterator([named(CLOCK + at, "clordid", value) for at, value in enumerate("ABC")])
    ]
    assert [held(event, "origclordid") for event in replaced] == [[None], ["A"], ["B"]]
    # `parentclordid` is that one parent's other spelling, never a second.
    assert [held(event, "parentclordid") for event in replaced] == [[None], ["A"], ["B"]]


def test_an_element_joins_a_live_chain_through_a_parent_identifiers_value() -> None:
    first = named(CLOCK, "orderid", "A")
    # Replaced under a new `orderid` that says what it replaced: the chain of
    # A, whose code - and cross identity - it keeps.
    replacement = graph.OrderEvent(
        CLOCK + 1,
        identifiers=Identifiers([Identifier("fix", "orderid", "B"), Identifier("fix", "parentorderid", "A")]),
    )
    # An element stating only the parent is what it came from: its `orderid`
    # is filled from it, and it joins the chain going by A.
    only = graph.OrderEvent(CLOCK + 1, identifiers=Identifiers([Identifier("fix", "parentorderid", "A")]))
    assert only.identifiers.get_from("fix", "orderid") == "A"
    [head, joined] = [data.as_order_event() for data in graph.EventIterator([first, replacement])]
    assert head is not None and joined is not None
    assert joined.prevuuid == head.curruuid and joined.crossuuid == head.crossuuid
    assert joined.crosscode == "10:1:O-100", "the chain's stored code"
    assert held(joined, "orderid", "parentorderid", "origorderid") == ["B", "A", "A"]
    [_, filled] = [data.as_order_event() for data in graph.EventIterator([first, only])]
    assert filled is not None and filled.prevuuid == head.curruuid

    # A parent no live chain goes by joins nothing: a chain of its own.
    stranger = graph.OrderEvent(
        CLOCK + 1, identifiers=Identifiers([Identifier("fix", "orderid", "Y"), Identifier("fix", "parentorderid", "Z")])
    )
    [_, alone] = [data.as_order_event() for data in graph.EventIterator([first, stranger])]
    assert alone is not None and alone.prevuuid is None
