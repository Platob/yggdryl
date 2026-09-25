"""Books, their sides, snapshot controls and the book walk: `python/src/graph/book.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle
from collections.abc import Iterator
from typing import Any

import pytest

from yggdryl import graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def order(clock: int = CLOCK, price: str = "101", code: str = "O-1") -> graph.OrderEvent:
    return graph.OrderEvent(clock, crosscode=code, side="BUY", price=D(price), quantity=10, ticker="IBM")


def quote(clock: int = CLOCK, price: str = "102", code: str = "Q-1") -> graph.QuoteEvent:
    return graph.QuoteEvent(clock, crosscode=code, side="SELL", price=D(price), quantity=5, ticker="IBM")


class TestSnapshotPartition:
    def test_slots_order_and_value_protocols(self) -> None:
        partition = graph.SnapshotPartition("S", "IBM")
        assert (partition.scope, partition.symbol) == ("S", "IBM")
        assert graph.SnapshotPartition("S").symbol is None
        assert graph.SnapshotPartition("A") < graph.SnapshotPartition("B")
        assert pickle.loads(pickle.dumps(partition)) == partition
        assert hash(pickle.loads(pickle.dumps(partition))) == hash(partition)
        assert copy.copy(partition) == partition and copy.deepcopy(partition) == partition
        assert repr(partition) == 'SnapshotPartition(scope="S", symbol="IBM")'
        assert repr(graph.SnapshotPartition("S")) == 'SnapshotPartition(scope="S", symbol=None)'


class TestBookSide:
    def test_an_empty_side(self) -> None:
        side = graph.BookSide("BUY")
        assert side.side.as_py() == "BUY"
        assert graph.BookSide("sell").side.as_py() == "SELL"
        assert len(side) == 0 and side.is_empty
        assert side.best_price is None and side.best_quantity is None
        assert side.live == [] and side.deltas == []

    def test_with_operation_answers_a_new_side(self) -> None:
        empty = graph.BookSide("BUY")
        side = empty.with_operation(order()).with_operation(graph.MarketData(order(price="100", code="O-2")))
        assert len(empty) == 0
        assert len(side) == 2 and not side.is_empty
        assert side.best_price is not None and side.best_price.as_py() == D("101")
        assert side.best_quantity is not None and side.best_quantity.as_py() == 10
        assert all(isinstance(entry, graph.MarketData) for entry in side.live)
        assert [entry.kind for entry in side.live] == ["order_event", "order_event"]
        assert [entry.crosscode for entry in side.live] == ["O-1", "O-2"]
        assert len(side.deltas) == 2

    def test_refusals(self) -> None:
        with pytest.raises(ValueError, match="side"):
            graph.BookSide("SIDEWAYS")
        with pytest.raises(ValueError, match="expected an order or quote on a book side"):
            graph.BookSide("BUY").with_operation(graph.ExecutionEvent(CLOCK))  # type: ignore[arg-type]
        with pytest.raises(TypeError, match="expected MarketData or a market leaf"):
            graph.BookSide("BUY").with_operation(1)  # type: ignore[arg-type]

    def test_equality_hash_repr_copy_pickle(self) -> None:
        side = graph.BookSide("BUY").with_operation(order())
        twin = pickle.loads(pickle.dumps(side))
        assert twin == side and hash(twin) == hash(side)
        assert twin.live == side.live
        assert copy.copy(side) == side and copy.deepcopy(side) == side
        assert repr(side).startswith(f"BookSide({side.curruuid.as_py()}")
        assert side.with_previous(side) is None or isinstance(side.with_previous(side), graph.BookSide)


class TestBookEvent:
    def test_an_empty_book(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        assert book.currunix == CLOCK and book.crosscode == "IBM"
        assert isinstance(book.bid, graph.BookSide) and isinstance(book.ask, graph.BookSide)
        assert book.bid.side.as_py() == "BUY" and book.ask.side.as_py() == "SELL"
        assert book.executions == [] and book.snapshot_partitions == []
        assert not book.is_crossed
        assert book.bbo_midpoint is None and book.median_quantity is None

    def test_with_operations_of_an_order_and_a_quote(self) -> None:
        empty = graph.BookEvent(CLOCK, "IBM")
        book = empty.with_operations([order(), quote()])
        assert empty.bid.is_empty  # immutable: the verb answered a new book
        assert book.bid.best_price is not None and book.bid.best_price.as_py() == D("101")
        assert book.ask.best_price is not None and book.ask.best_price.as_py() == D("102")
        assert not book.is_crossed
        assert book.bbo_midpoint is not None and book.bbo_midpoint.as_py() == D("101.5")
        assert book.median_quantity is not None and book.median_quantity.as_py() == D("7.5")
        live = book.bid.live
        assert all(isinstance(entry, graph.MarketData) for entry in live)
        assert live[0].as_order_event() is not None
        crossed = empty.with_operations([order(price="103"), quote()])
        assert crossed.is_crossed

    def test_executions_and_market_data_fold(self) -> None:
        execution = graph.ExecutionEvent(CLOCK, crosscode="E-1", side="BUY", lastpx=D("101"), lastqty=1)
        book = graph.BookEvent(CLOCK, "IBM").with_operations(iter([graph.MarketData(order()), execution]))
        assert [type(held) for held in book.executions] == [graph.ExecutionEvent]
        assert book.executions[0].crosscode == "E-1"

    def test_a_snapshot_control_records_its_partition(self) -> None:
        snapshot = graph.SnapshotEvent.snapshot(order(), "Symbol=IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([snapshot])
        assert [partition.scope for partition in book.snapshot_partitions] == ["Symbol=IBM"]

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
        assert twin.bid == book.bid and twin.ask == book.ask
        assert copy.copy(book) == book and copy.deepcopy(book) == book
        assert repr(book) == f'BookEvent({book.curruuid.as_py()}, currunix={CLOCK}, crosscode="IBM")'
        later = graph.BookEvent(CLOCK + 1, "IBM")
        assert later.is_after(book) and book.is_before(later)


class TestSnapshotEvent:
    def test_snapshot_copies_a_dated_leaf(self) -> None:
        source = order()
        snapshot = graph.SnapshotEvent.snapshot(source, "S")
        assert snapshot.currunix == source.currunix and snapshot.ticker == "IBM"
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
        assert walk.global_ is False
        books = list(walk)
        assert [type(book) for book in books] == [graph.BookEvent, graph.BookEvent]
        assert [book.currunix for book in books] == [CLOCK, CLOCK + 1]
        assert books[0].bid.best_price is not None and books[0].bid.best_price.as_py() == D("101")
        assert books[0].ask.best_price is not None and books[0].ask.best_price.as_py() == D("102")
        assert [held.crosscode for held in books[1].executions] == ["O-1"]
        assert hash(walk.__class__) is not None and walk.__hash__ is None

    def test_a_global_walk_consolidates_symbols(self) -> None:
        walk = graph.BookIterator([order()], global_=True)
        assert walk.global_
        [book] = walk
        assert book.crosscode == graph.GLOBAL_SYMBOL

    def test_a_book_side_item_is_refused_by_name(self) -> None:
        side = graph.BookSide("BUY")
        with pytest.raises(ValueError, match=r"\$\.operation\.kind: expected order_event.*got book_side"):
            list(graph.BookIterator([side]))

    def test_a_python_failure_is_raised_as_itself(self) -> None:
        def items() -> Iterator[Any]:
            yield order()
            raise RuntimeError("the source gave up")

        with pytest.raises(RuntimeError, match="the source gave up"):
            list(graph.BookIterator(items()))
        with pytest.raises(TypeError):
            graph.BookIterator(5)  # type: ignore[arg-type]
