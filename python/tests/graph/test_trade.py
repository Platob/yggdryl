"""``graph.TradeEvent``: `python/src/graph/trade.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle

import pytest

from yggdryl import graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def fill(code: str, side: str, quantity: int, clock: int = CLOCK) -> graph.ExecutionEvent:
    return graph.ExecutionEvent(
        clock, crosscode=code, side=side, lastpx=D("101.25"), lastqty=quantity, ticker="ACME"
    )


def trade() -> graph.TradeEvent:
    root = graph.ExecutionEvent(CLOCK, crosscode="T-1", ticker="ACME", lastpx=D("101.25"), lastqty=10)
    return graph.TradeEvent.from_parts(root, [fill("SELL-1", "SELL", 6), fill("BUY-1", "BUY", 4)])


def test_from_parts_of_two_executions() -> None:
    made = trade()
    assert made.crosscode == "T-1" and made.currunix == CLOCK
    assert made.ticker == "ACME"
    assert made.lastqty is not None and made.lastqty.as_py() == 10
    executions = made.executions
    assert [type(execution) for execution in executions] == [graph.ExecutionEvent] * 2
    # In canonical side order, whatever order they were handed over in.
    assert [execution.side.as_py() for execution in executions] == ["BUY", "SELL"]
    assert sorted(execution.crosscode for execution in executions) == ["BUY-1", "SELL-1"]
    assert made.is_execution


def test_any_dated_operation_or_market_data_roots_a_trade() -> None:
    root = graph.OrderEvent(CLOCK, crosscode="T-1", ticker="ACME")
    by_leaf = graph.TradeEvent.from_parts(root, [fill("B", "BUY", 1)])
    by_data = graph.TradeEvent.from_parts(graph.MarketData(root), [fill("B", "BUY", 1)])
    assert by_leaf == by_data
    assert by_leaf.crosscode == "T-1"


def test_refusals_name_what_was_wrong() -> None:
    with pytest.raises(TypeError, match="expected a dated operation as the trade's root, got order"):
        graph.TradeEvent.from_parts(graph.Order(), [])  # type: ignore[arg-type]
    with pytest.raises(ValueError, match=r"executions\[0\]\.currunix: expected the trade timestamp"):
        graph.TradeEvent.from_parts(graph.ExecutionEvent(CLOCK + 1), [fill("B", "BUY", 1)])
    with pytest.raises(ValueError, match="at least one execution"):
        graph.TradeEvent.from_parts(graph.ExecutionEvent(CLOCK), [])
    with pytest.raises(TypeError):
        graph.TradeEvent.from_parts(graph.ExecutionEvent(CLOCK), [graph.OrderEvent(CLOCK)])  # type: ignore[list-item]
    with pytest.raises(TypeError):
        graph.TradeEvent()  # type: ignore[call-arg]


def test_the_verbs_answer_new_trades() -> None:
    first = trade()
    later = graph.TradeEvent.from_parts(
        graph.ExecutionEvent(CLOCK + 1, crosscode="T-1", ticker="ACME"),
        [fill("BUY-1", "BUY", 4, CLOCK + 1)],
    )
    followed = later.with_previous(first)
    assert followed is not None and followed.prevuuid == first.curruuid
    assert later.prevuuid is None
    assert later.is_after(first) and first.is_before(later)
    # A trade folds another statement of itself into the same trade.
    merged = first.merge_with(first)
    assert isinstance(merged, graph.TradeEvent) and merged.crossuuid == first.crossuuid
    assert first.restating(first) == first


def test_equality_hash_repr_copy_pickle() -> None:
    made = trade()
    twin = pickle.loads(pickle.dumps(made))
    assert twin == made and hash(twin) == hash(made)
    assert twin.executions == made.executions
    assert copy.copy(made) == made and copy.deepcopy(made) == made
    assert repr(made) == (
        f'TradeEvent({made.curruuid.as_py()}, currunix={CLOCK}, crosscode="T-1")'
    )
    assert made != object()
