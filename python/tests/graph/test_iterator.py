"""``graph.EventIterator``: `python/src/graph/iterator.rs`."""

from __future__ import annotations

import decimal
from collections.abc import Iterator
from typing import Any

import pytest

from yggdryl import graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def order(clock: int, state: str = "NEW", **facts: Any) -> graph.OrderEvent:
    return graph.OrderEvent(clock, crosscode="O-1", side="BUY", price=D("101"), quantity=10, state=state, **facts)


def test_an_order_chains_to_the_live_order_it_follows() -> None:
    first, second = order(CLOCK), order(CLOCK + 1, state="REPLACED", leavesqty=...)
    walked = list(graph.EventIterator([first, second]))
    assert [type(data) for data in walked] == [graph.MarketData, graph.MarketData]
    head = walked[0].as_order_event()
    tail = walked[1].as_order_event()
    assert head is not None and tail is not None
    assert head == first and head.seqnum == 0 and head.prevuuid is None
    assert tail.prevuuid == first.curruuid
    assert tail.prevunix == CLOCK and tail.seqnum == 1


def test_an_execution_and_every_other_leaf_walk_through() -> None:
    first = order(CLOCK)
    # A millisecond later: two instants in one millisecond share an identity.
    execution = graph.ExecutionEvent(CLOCK + 1_000_000, crosscode="O-1", side="BUY", lastpx=D("101"), lastqty=10)
    side = graph.BookSide("BUY")
    walked = list(graph.EventIterator([first, execution, side, graph.Order(crosscode="O-1")]))
    assert [data.kind for data in walked] == ["order_event", "execution_event", "book_side", "order"]
    # A book side is yielded unchanged, in place.
    assert walked[2].as_book_side() == side
    assert walked[3].as_order() == graph.Order(crosscode="O-1")
    # The execution follows the order it fills across kinds, keeping its own.
    fill = walked[1].as_execution_event()
    assert fill is not None and fill.crossuuid == first.crossuuid
    assert fill.prevuuid == first.curruuid and fill.seqnum == 1


def test_unsorted_items_are_sorted_first() -> None:
    first, second = order(CLOCK), order(CLOCK + 1, state="REPLACED")
    walked = [data.as_order_event() for data in graph.EventIterator([second, first], sorted=False)]
    assert [event.currunix for event in walked if event is not None] == [CLOCK, CLOCK + 1]


def test_alive_and_the_snapshot_grid() -> None:
    walk = graph.EventIterator([order(CLOCK, exprtime=CLOCK + 10)], snapshot_ns=5)
    assert walk.snapshot_ns == 5
    walked = [data.as_order_event() for data in walk]
    assert [event.snapunix for event in walked if event is not None] == [None, CLOCK, CLOCK + 5, None]
    assert walked[-1] is not None and walked[-1].state.as_py() == "95EXPIRED"
    assert walk.alive() == []
    live = graph.EventIterator([order(CLOCK)])
    assert live.snapshot_ns is None
    list(live)
    assert [data.crosscode for data in live.alive()] == ["O-1"]
    assert all(isinstance(data, graph.MarketData) for data in live.alive())


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
    with pytest.raises(TypeError, match="expected MarketData or a market leaf, got int"):
        list(graph.EventIterator([1]))  # type: ignore[list-item]


def test_the_walk_is_unhashable() -> None:
    with pytest.raises(TypeError):
        hash(graph.EventIterator([]))
