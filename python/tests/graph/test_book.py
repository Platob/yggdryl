"""Books, snapshot controls and the book walk: `python/src/graph/book.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle
from collections.abc import Iterator
from typing import Any

import pytest

from yggdryl import MarketDataKind, Scalar, Side, graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def order(
    clock: int = CLOCK,
    price: str = "101",
    code: str = "O-1",
    tradable: bool | None = None,
) -> graph.OrderEvent:
    return graph.OrderEvent(
        clock,
        crosscode=code,
        side="BUY",
        price=D(price),
        quantity=10,
        ticker="IBM",
        tradable=tradable,
    )


def quote(clock: int = CLOCK, price: str = "102", code: str = "Q-1") -> graph.QuoteEvent:
    return graph.QuoteEvent(clock, crosscode=code, side="SELL", price=D(price), quantity=5, ticker="IBM")


def decimal_of(value: Scalar | None) -> object:
    assert value is not None
    return value.as_py()


class TestBookEvent:
    def test_an_empty_book(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        assert book.currunix == CLOCK and book.crosscode == "IBM"
        assert book.marketdatakind is MarketDataKind.BOOK
        # A book states no side of its own.
        assert book.side is Side.UNKNOWN
        assert book.alive == [] and book.deltas == [] and book.executions == []
        assert book.limits(Side.BUY) == [] and book.limits("SELL") == []
        assert book.best_price(Side.BUY) is None and book.best_quantity(Side.SELL) is None
        assert book.bidpx is None and book.askpx is None
        assert not book.is_crossed
        assert book.bbo_midpoint is None and book.median_quantity is None
        assert not book.is_locked
        assert book.spread is None
        # Both sides empty: the total is zero, so there is no imbalance.
        assert book.imbalance(1) is None

    def test_limits_of_an_empty_side(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        assert decimal_of(book.depth(Side.BUY, 1)) == 0
        assert decimal_of(book.depth(Side.BUY, 0)) == 0
        # A side that is neither a bid nor an ask has no depth.
        assert book.depth(Side.CROSS, 1) is None
        assert book.limits(Side.CROSS) == []
        # A level count is a count: a negative one is not one.
        with pytest.raises(OverflowError):
            book.depth(Side.BUY, -1)
        with pytest.raises(ValueError):
            book.limits("SIDEWAYS")

    def test_one_limit_per_price_best_first(self) -> None:
        first = order(code="O-1")
        second = order(code="O-2")
        lower = order(price="100", code="O-3")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([first, second, lower])
        limits = book.limits(Side.BUY)
        assert all(isinstance(limit, Scalar) for limit in limits)
        assert [limit["price"].as_py() for limit in limits] == [D("101"), D("100")]
        # Two entries at one price are one limit holding both, in live order;
        # an entry stating nothing about trading trades.
        best = limits[0].as_py()
        assert best["quantity"] == D("20")
        assert best["uuids"] == [first.curruuid.as_py(), second.curruuid.as_py()]
        assert best["tradable"] is True
        assert limits[1].as_py() == {
            "price": D("100"),
            "quantity": D("10"),
            "uuids": [lower.curruuid.as_py()],
            "tradable": True,
        }
        # The depth walks the limits in that order.
        assert decimal_of(book.depth(Side.BUY, 1)) == D("20")
        assert decimal_of(book.depth(Side.BUY, 2)) == D("30")
        assert decimal_of(book.depth("BUY", 9)) == D("30")

    def test_the_best_is_the_first_tradable_level(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM").with_operations(
            [order(price="102", code="O-1", tradable=False), order(price="101", code="O-2")]
        )
        [top, below] = book.limits(Side.BUY)
        assert top.as_py()["tradable"] is False and below.as_py()["tradable"] is True
        assert decimal_of(book.best_price(Side.BUY)) == D("101")
        assert decimal_of(book.bidpx) == D("101")
        assert decimal_of(book.best_quantity(Side.BUY)) == D("10")
        untradable = graph.BookEvent(CLOCK, "IBM").with_operations([order(tradable=False)])
        assert untradable.best_price(Side.BUY) is None and untradable.bidpx is None

    def test_an_unpriced_entry_rests_at_the_last_limit(self) -> None:
        unpriced = graph.OrderEvent(CLOCK, crosscode="M-1", side="BUY", quantity=4, ticker="IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([unpriced, order()])
        # The market order is held rather than refused, after every priced
        # level, and states no best of its own.
        assert [entry.crosscode for entry in book.alive] == ["BUY:O-1", "BUY:M-1"]
        assert decimal_of(book.best_price(Side.BUY)) == D("101")
        last = book.limits(Side.BUY)[-1]
        assert last["price"].as_py() is None
        assert last.as_py() == {
            "price": None,
            "quantity": D("4"),
            "uuids": [unpriced.curruuid.as_py()],
            "tradable": True,
        }
        assert decimal_of(book.depth(Side.BUY, 2)) == D("14")
        alone = graph.BookEvent(CLOCK, "IBM").with_operations([unpriced])
        assert alone.best_price(Side.BUY) is None and alone.best_quantity(Side.BUY) is None
        assert len(alone.limits(Side.BUY)) == 1

    def test_spread_lock_and_imbalance(self) -> None:
        empty = graph.BookEvent(CLOCK, "IBM")
        book = empty.with_operations([order(), quote()])
        assert not book.is_locked
        assert decimal_of(book.spread) == D("1")
        # (10 - 30) / (10 + 30) over the first level of each side.
        deep = graph.QuoteEvent(CLOCK, crosscode="Q-2", side="SELL", price=D("102"), quantity=30, ticker="IBM")
        leaning = empty.with_operations([order(), deep])
        assert decimal_of(leaning.imbalance(1)) == D("-0.5")
        assert leaning.imbalance(0) is None
        locked = empty.with_operations([order(price="102"), quote()])
        assert locked.is_locked and not locked.is_crossed
        assert decimal_of(locked.spread) == 0
        # A crossed book states its spread as the negative fact it is.
        crossed = empty.with_operations([order(price="103"), quote()])
        assert crossed.is_crossed and not crossed.is_locked
        assert decimal_of(crossed.spread) == D("-1")
        # One-sided books lean all the way to their side.
        bid_only = empty.with_operations([order()])
        assert bid_only.spread is None and not bid_only.is_locked
        assert decimal_of(bid_only.imbalance(1)) == D("1")
        ask_only = empty.with_operations([quote()])
        assert decimal_of(ask_only.imbalance(1)) == D("-1")

    def test_with_operations_of_an_order_and_a_quote(self) -> None:
        empty = graph.BookEvent(CLOCK, "IBM")
        book = empty.with_operations([order(), quote()])
        assert empty.alive == []  # immutable: the verb answered a new book
        assert decimal_of(book.best_price(Side.BUY)) == D("101")
        assert decimal_of(book.best_price(Side.SELL)) == D("102")
        # The book states its best tradable levels as its own bid and ask.
        assert decimal_of(book.bidpx) == D("101")
        assert decimal_of(book.bidqty) == D("10")
        assert decimal_of(book.askpx) == D("102")
        assert decimal_of(book.askqty) == D("5")
        assert not book.is_crossed
        assert decimal_of(book.bbo_midpoint) == D("101.5")
        assert decimal_of(book.median_quantity) == D("7.5")
        alive = book.alive
        assert all(isinstance(entry, graph.MarketData) for entry in alive)
        # The bid side's entries first, then the ask side's.
        assert [entry.marketdatakind for entry in alive] == [MarketDataKind.ORDR, MarketDataKind.QUOT]
        assert alive[0].as_order_event() is not None
        assert len(book.deltas) == 2
        crossed = empty.with_operations([order(price="103"), quote()])
        assert crossed.is_crossed

    def test_executions_and_market_data_fold(self) -> None:
        execution = graph.ExecutionEvent(CLOCK, crosscode="E-1", side="BUY", lastpx=D("101"), lastqty=1)
        book = graph.BookEvent(CLOCK, "IBM").with_operations(iter([graph.MarketData(order()), execution]))
        assert [type(held) for held in book.executions] == [graph.ExecutionEvent]
        assert book.executions[0].crosscode == "BUY:E-1"

    def test_a_snapshot_control_folds(self) -> None:
        snapshot = graph.SnapshotEvent.snapshot(order(), "Symbol=IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([snapshot])
        assert book.alive == []

    def test_refusals_name_the_item(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        with pytest.raises(TypeError, match=r"operations\[1\]: .*expected MarketData or a market leaf, got int"):
            book.with_operations([order(), 1])  # type: ignore[list-item]
        with pytest.raises(ValueError, match=r"\$\.operations\[0\]\.kind: expected order_event.*got order"):
            book.with_operations([graph.Order()])

    def test_equality_hash_repr_copy_pickle(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM").with_operations([order(), quote()])
        twin = pickle.loads(pickle.dumps(book))
        assert twin == book and hash(twin) == hash(book)
        assert twin.alive == book.alive
        assert twin.limits(Side.BUY) == book.limits(Side.BUY)
        assert copy.copy(book) == book and copy.deepcopy(book) == book
        assert repr(book) == f'BookEvent({book.curruuid.as_py()}, currunix={CLOCK}, crosscode="IBM")'
        later = graph.BookEvent(CLOCK + 1, "IBM")
        assert later.is_after(book) and book.is_before(later)


class TestSnapshotEvent:
    def test_snapshot_copies_a_dated_leaf(self) -> None:
        source = order()
        snapshot = graph.SnapshotEvent.snapshot(source, "S")
        assert snapshot.currunix == source.currunix and snapshot.ticker == "IBM"
        assert snapshot.marketdatakind is MarketDataKind.BOOK
        assert snapshot.book.action == "snapshot" and snapshot.book.scope == "S"
        assert graph.SnapshotEvent.snapshot(graph.MarketData(source)).book.scope is None

    def test_refusals(self) -> None:
        with pytest.raises(TypeError, match="expected a dated leaf to snapshot, got order"):
            graph.SnapshotEvent.snapshot(graph.Order())  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            graph.SnapshotEvent()  # type: ignore[call-arg]

    def test_equality_hash_repr_copy_pickle(self) -> None:
        snapshot = graph.SnapshotEvent.snapshot(order(), "S")
        twin = pickle.loads(pickle.dumps(snapshot))
        assert twin == snapshot and hash(twin) == hash(snapshot)
        assert twin.book == snapshot.book
        assert copy.copy(snapshot) == snapshot and copy.deepcopy(snapshot) == snapshot
        assert repr(snapshot).startswith(f"SnapshotEvent({snapshot.curruuid.as_py()}, currunix={CLOCK}")


class TestBookIterator:
    def test_books_over_three_items_in_order(self) -> None:
        execution = graph.ExecutionEvent(
            CLOCK + 1, crosscode="O-1", side="BUY", lastpx=D("101"), lastqty=1, ticker="IBM"
        )
        walk = graph.BookIterator([order(), graph.MarketData(quote()), execution])
        books = list(walk)
        assert [type(book) for book in books] == [graph.BookEvent, graph.BookEvent]
        assert [book.currunix for book in books] == [CLOCK, CLOCK + 1]
        assert decimal_of(books[0].best_price(Side.BUY)) == D("101")
        assert decimal_of(books[0].best_price(Side.SELL)) == D("102")
        assert [held.crosscode for held in books[1].executions] == ["BUY:O-1"]
        assert hash(walk.__class__) is not None and walk.__hash__ is None

    def test_a_walk_takes_no_global_mode(self) -> None:
        with pytest.raises(TypeError):
            graph.BookIterator([order()], global_=True)  # type: ignore[call-arg]
        assert not hasattr(graph, "GLOBAL_SYMBOL")

    def test_a_python_failure_is_raised_as_itself(self) -> None:
        def items() -> Iterator[Any]:
            yield order()
            raise RuntimeError("the source gave up")

        with pytest.raises(RuntimeError, match="the source gave up"):
            list(graph.BookIterator(items()))
        with pytest.raises(TypeError):
            graph.BookIterator(5)  # type: ignore[arg-type]
