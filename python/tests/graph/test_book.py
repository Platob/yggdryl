"""Books, snapshot controls and the book walk: `python/src/graph/book.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle
from collections.abc import Iterator
from typing import Any

import pytest

from yggdryl import Filter, Identifier, MarketDataKind, Scalar, Side, State, Term, graph

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
        side="BUYS",
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
        # A book states side 0 whatever side it takes: kind 3, side 0, its ticker.
        assert book.currunix == CLOCK and book.crosscode == "3:0:IBM"
        assert book.marketdatakind is MarketDataKind.BOOK
        # A book holds both sides.
        assert book.side is Side.BOTH
        assert book.alive_on(Side.BOTH) == []
        assert book.alive == [] and book.deltas == []
        assert book.ordlive == [] and book.orddelta == [] and book.quotes == [] and book.executions == []
        assert book.events == []
        assert book.alive_on(Side.BUYS) == [] and book.alive_on("SELL") == []
        # A book a caller builds holds its sides, empty or not.
        assert book.is_complete
        assert book.limits(Side.BUYS) == [] and book.limits("SELL") == []
        assert book.best_price(Side.BUYS) is None and book.best_quantity(Side.SELL) is None
        assert book.bidpx is None and book.askpx is None
        assert not book.is_crossed
        assert book.bbo_midpoint is None and book.median_quantity is None
        assert not book.is_locked
        assert book.spread is None
        # Both sides empty: the total is zero, so there is no imbalance.
        assert book.imbalance(1) is None

    def test_limits_of_an_empty_side(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        assert decimal_of(book.depth(Side.BUYS, 1)) == 0
        assert decimal_of(book.depth(Side.BUYS, 0)) == 0
        # A side that is neither a bid nor an ask has no depth.
        assert book.depth(Side.CROS, 1) is None
        assert book.limits(Side.CROS) == []
        # A level count is a count: a negative one is not one.
        with pytest.raises(OverflowError):
            book.depth(Side.BUYS, -1)
        with pytest.raises(ValueError):
            book.limits("SIDEWAYS")

    def test_one_limit_per_price_best_first(self) -> None:
        first = order(code="O-1")
        second = order(code="O-2")
        lower = order(price="100", code="O-3")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([first, second, lower])
        limits = book.limits(Side.BUYS)
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
        assert decimal_of(book.depth(Side.BUYS, 1)) == D("20")
        assert decimal_of(book.depth(Side.BUYS, 2)) == D("30")
        assert decimal_of(book.depth("BUYS", 9)) == D("30")

    def test_the_best_is_the_first_tradable_level(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM").with_operations(
            [order(price="102", code="O-1", tradable=False), order(price="101", code="O-2")]
        )
        [top, below] = book.limits(Side.BUYS)
        assert top.as_py()["tradable"] is False and below.as_py()["tradable"] is True
        assert decimal_of(book.best_price(Side.BUYS)) == D("101")
        assert decimal_of(book.bidpx) == D("101")
        assert decimal_of(book.best_quantity(Side.BUYS)) == D("10")
        untradable = graph.BookEvent(CLOCK, "IBM").with_operations([order(tradable=False)])
        assert untradable.best_price(Side.BUYS) is None and untradable.bidpx is None

    def test_an_unpriced_entry_rests_at_the_last_limit(self) -> None:
        unpriced = graph.OrderEvent(CLOCK, crosscode="M-1", side="BUYS", quantity=4, ticker="IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([unpriced, order()])
        # The market order is held rather than refused, after every priced
        # level, and states no best of its own.
        assert [entry.crosscode for entry in book.alive] == ["10:1:O-1", "10:1:M-1"]
        assert decimal_of(book.best_price(Side.BUYS)) == D("101")
        last = book.limits(Side.BUYS)[-1]
        assert last["price"].as_py() is None
        assert last.as_py() == {
            "price": None,
            "quantity": D("4"),
            "uuids": [unpriced.curruuid.as_py()],
            "tradable": True,
        }
        assert decimal_of(book.depth(Side.BUYS, 2)) == D("14")
        alone = graph.BookEvent(CLOCK, "IBM").with_operations([unpriced])
        assert alone.best_price(Side.BUYS) is None and alone.best_quantity(Side.BUYS) is None
        assert len(alone.limits(Side.BUYS)) == 1

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
        assert decimal_of(book.best_price(Side.BUYS)) == D("101")
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

    def test_market_data_folds_and_an_execution_is_recorded(self) -> None:
        # A book places no execution: a fill moves it through its order's
        # report, and the execution stands among the deltas of its instant.
        execution = graph.ExecutionEvent(CLOCK, crosscode="E-1", side="BUYS", lastpx=D("101"), lastqty=1)
        book = graph.BookEvent(CLOCK, "IBM").with_operations(iter([graph.MarketData(order()), execution]))
        assert [entry.crosscode for entry in book.alive] == ["10:1:O-1"]
        assert [delta.crosscode for delta in book.deltas] == ["10:1:O-1", "8:1:E-1"]
        # Read by kind, the execution is an `ExecutionEvent` among the deltas.
        assert [type(held) for held in book.executions] == [graph.ExecutionEvent]
        assert [held.crosscode for held in book.executions] == ["8:1:E-1"]
        assert [held.crosscode for held in book.orddelta] == ["10:1:O-1"]
        assert book.executions[0] == book.deltas[1].as_execution_event()

    def test_resting_orders_and_deltas_by_kind(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM").with_operations(
            [order(code="B-1"), order(price="100", code="B-2"), quote()]
        )
        cancel = graph.OrderEvent(
            CLOCK + 1, crosscode="B-1", side="BUYS", price=D("101"), quantity=10, ticker="IBM", state="CANCELED"
        )
        fill = graph.ExecutionEvent(CLOCK + 1, crosscode="E-1", side="BUYS", lastpx=D("100"), lastqty=1, ticker="IBM")
        later = book.with_operations([cancel, fill, quote(CLOCK + 1, price="103", code="Q-2")])
        assert all(isinstance(entry, graph.OrderEvent) for entry in later.ordlive + later.orddelta)
        # Resting: the orders alive now. Changed: the orders the instant applied.
        assert [entry.crosscode for entry in later.ordlive] == ["10:1:B-2"]
        assert [(entry.crosscode, entry.state) for entry in later.orddelta] == [("10:1:B-1", State.CANCELED)]
        assert [type(entry) for entry in later.quotes] == [graph.QuoteEvent]
        assert [entry.crosscode for entry in later.quotes] == ["14:0:Q-2"]
        assert [entry.crosscode for entry in later.executions] == ["8:1:E-1"]
        # Q-1 still rests: it is alive, not a delta of this instant.
        assert "14:0:Q-1" in [entry.crosscode for entry in later.alive]
        # The four partition the deltas; `events`, every other delta, is empty
        # by construction: a fold records no other kind among them.
        assert later.events == []
        assert len(later.orddelta) + len(later.quotes) + len(later.executions) + len(later.events) == len(later.deltas)
        # A delta book states its changed orders and no resting one.
        books = list(graph.BookIterator([order(), order(CLOCK + 1, "100", "B-2")]))
        assert books[1].ordlive == [] and [entry.crosscode for entry in books[1].orddelta] == ["10:1:B-2"]
        first = books[0].with_previous(graph.BookEvent.keyed(CLOCK, "IBM"))
        assert first is not None
        rebuilt = books[1].with_previous(first)
        assert rebuilt is not None and [entry.crosscode for entry in rebuilt.ordlive] == ["10:1:O-1", "10:1:B-2"]
        for held in (*books, rebuilt):
            assert held.events == []
            assert len(held.orddelta) + len(held.quotes) + len(held.executions) == len(held.deltas)

    def test_no_delta_lands_among_the_events(self) -> None:
        # Refused first: a nested book and an undated order are refused by
        # kind, the book untouched; a trade is pruned before the fold.
        book = graph.BookEvent(CLOCK, "IBM").with_operations([order()])
        with pytest.raises(ValueError, match=r"\$\.operations\[0\]\.kind"):
            book.with_operations([graph.BookEvent(CLOCK, "IBM")])
        with pytest.raises(ValueError, match=r"\$\.operations\[0\]\.kind"):
            book.with_operations([graph.Order()])
        assert book.events == [] and len(book.orddelta) == len(book.deltas) == 1
        root = graph.ExecutionEvent(CLOCK + 1, crosscode="T-1", ticker="IBM", lastpx=D("101"), lastqty=1)
        fill = graph.ExecutionEvent(CLOCK + 1, crosscode="F-1", side="BUYS", ticker="IBM", lastpx=D("101"), lastqty=1)
        trade = graph.TradeEvent.from_parts(root, [fill])
        books = list(graph.BookIterator([order(), trade, quote(CLOCK + 1)]))
        assert all(held.events == [] for held in books)
        assert [delta.marketdatakind for held in books for delta in held.deltas] == [
            MarketDataKind.ORDR,
            MarketDataKind.QUOT,
        ]

    def test_alive_on_reads_one_side_best_first(self) -> None:
        unpriced = graph.OrderEvent(CLOCK, crosscode="B-M", side="BUYS", quantity=3, ticker="IBM")
        ask = graph.OrderEvent(CLOCK, crosscode="A-1", side="SELL", price=D("102"), quantity=1, ticker="IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations(
            [order(price="100", code="B-1"), unpriced, order(price="101", code="B-2"), ask]
        )
        bids = book.alive_on(Side.BUYS)
        assert all(isinstance(entry, graph.MarketData) for entry in bids)
        # Best price first, the entry stating no price last.
        assert [entry.crosscode for entry in bids] == ["10:1:B-2", "10:1:B-1", "10:1:B-M"]
        assert [entry.crosscode for entry in book.alive_on("SELL")] == ["10:2:A-1"]
        # A side that is neither a bid nor an ask holds nothing.
        assert book.alive_on(Side.UKNW) == []
        # `alive` is the bid side's entries, then the ask side's.
        assert book.alive == [*bids, *book.alive_on(Side.SELL)]
        with pytest.raises(ValueError):
            book.alive_on("SIDEWAYS")

    def test_a_two_sided_quote_rests_on_both_sides_as_one_entry(self) -> None:
        # A quote tagging no side rests on every side it states a leg for, as
        # one entry: `alive` lists it once, each side reads its own leg.
        two_sided = graph.QuoteEvent(
            CLOCK, crosscode="Q-1", ticker="IBM", bidpx=D("99"), bidqty=2, askpx=D("101"), askqty=3, state="NEW"
        )
        # Quoting both legs and tagging neither, it holds both sides.
        assert two_sided.crosscode == "14:0:Q-1" and two_sided.side is Side.BOTH
        book = graph.BookEvent(CLOCK, "IBM").with_operations([two_sided])
        assert [entry.crosscode for entry in book.alive_on(Side.BUYS)] == ["14:0:Q-1"]
        assert [entry.crosscode for entry in book.alive_on(Side.SELL)] == ["14:0:Q-1"]
        assert book.alive_on(Side.BUYS) == book.alive_on(Side.SELL)
        assert [entry.crosscode for entry in book.alive] == ["14:0:Q-1"]
        assert [delta.crosscode for delta in book.deltas] == ["14:0:Q-1"]
        assert [(limit["price"].as_py(), limit["quantity"].as_py()) for limit in book.limits(Side.BUYS)] == [
            (D("99"), D("2"))
        ]
        assert [(limit["price"].as_py(), limit["quantity"].as_py()) for limit in book.limits(Side.SELL)] == [
            (D("101"), D("3"))
        ]

    def test_keyed_is_the_empty_base_a_codes_first_book_rebuilds_over(self) -> None:
        listed = graph.OrderEvent(
            1,
            crosscode="B-1",
            securityids=[Identifier("isin", "CH0012214059")],
            ticker="HOLN",
            side="BUYS",
            price=D("99"),
            quantity=1,
            state="NEW",
        )
        (first,) = graph.BookIterator([listed])
        # Keyed by the instrument's ISIN, which beats its ticker.
        assert first.crosscode == "3:0:CH0012214059"
        assert first.ticker == "HOLN"
        # With no grid and no snapshot input, a book states its deltas alone.
        assert not first.is_complete
        assert first.alive == [] and first.alive_on(Side.BUYS) == [] and first.limits(Side.BUYS) == []
        assert [delta.crosscode for delta in first.deltas] == ["10:1:B-1"]
        assert decimal_of(first.best_price(Side.BUYS)) == D("99")
        # A book stating its deltas alone takes no operations.
        with pytest.raises(ValueError, match=r"\$\.alive"):
            first.with_operations([order(clock=2)])
        base = graph.BookEvent.keyed(1, "CH0012214059")
        assert base.crosscode == "3:0:CH0012214059"
        assert base.ticker is None and base.is_complete and base.alive == []
        whole = first.with_previous(base)
        assert whole is not None and whole.is_complete
        assert [entry.crosscode for entry in whole.alive] == ["10:1:B-1"]
        assert whole.curruuid == first.curruuid
        # An empty symbol keys the book by the number that states none.
        assert graph.BookEvent(CLOCK, "").crosscode == "3:0:XX0000000000"
        assert graph.BookEvent.keyed(CLOCK, "XX0000000000") == graph.BookEvent(CLOCK, "")

    def test_a_snapshot_control_folds(self) -> None:
        snapshot = graph.SnapshotEvent.snapshot(order(), "Symbol=IBM")
        book = graph.BookEvent(CLOCK, "IBM").with_operations([snapshot])
        assert book.alive == []

    def test_refusals_name_the_item(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM")
        with pytest.raises(TypeError, match=r"operations\[1\]: .*expected MarketData, a market leaf or a FixMsg, got int"):
            book.with_operations([order(), 1])  # type: ignore[list-item]
        with pytest.raises(ValueError, match=r"\$\.operations\[0\]\.kind: expected order_event.*got order"):
            book.with_operations([graph.Order()])

    def test_equality_hash_repr_copy_pickle(self) -> None:
        book = graph.BookEvent(CLOCK, "IBM").with_operations([order(), quote()])
        twin = pickle.loads(pickle.dumps(book))
        assert twin == book and hash(twin) == hash(book)
        assert twin.alive == book.alive
        assert twin.limits(Side.BUYS) == book.limits(Side.BUYS)
        assert copy.copy(book) == book and copy.deepcopy(book) == book
        assert repr(book) == f'BookEvent({book.curruuid.as_py()}, currunix={CLOCK}, crosscode="3:0:IBM")'
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
        # A walk records an execution where it pulls it: an instant only an
        # execution reached emits a book stating it alone, resting nowhere.
        execution = graph.ExecutionEvent(
            CLOCK + 1, crosscode="O-1", side="BUYS", lastpx=D("101"), lastqty=1, ticker="IBM"
        )
        walk = graph.BookIterator([order(), graph.MarketData(quote()), execution])
        books = list(walk)
        assert [type(book) for book in books] == [graph.BookEvent, graph.BookEvent]
        assert [book.currunix for book in books] == [CLOCK, CLOCK + 1]
        assert decimal_of(books[0].best_price(Side.BUYS)) == D("101")
        assert decimal_of(books[0].best_price(Side.SELL)) == D("102")
        assert [delta.marketdatakind for delta in books[0].deltas] == [MarketDataKind.ORDR, MarketDataKind.QUOT]
        assert [delta.marketdatakind for delta in books[1].deltas] == [MarketDataKind.EXEC]
        assert decimal_of(books[1].best_price(Side.BUYS)) == D("101")
        assert hash(walk.__class__) is not None and walk.__hash__ is None

    def test_a_filter_narrows_the_walk_and_never_widens_it(self) -> None:
        def inputs() -> list[Any]:
            return [
                graph.OrderEvent(1, crosscode="B-1", side="BUYS", price=D("100"), quantity=2, ticker="IBM", state="NEW"),
                graph.OrderEvent(2, crosscode="A-1", side="SELL", price=D("102"), quantity=1, ticker="IBM", state="NEW"),
                graph.ExecutionEvent(
                    3, crosscode="E-1", side="BUYS", price=D("100"), quantity=1, ticker="IBM", state="FILLED"
                ),
                graph.QuoteEvent(4, crosscode="B-2", side="BUYS", price=D("101"), quantity=1, ticker="IBM", state="NEW"),
            ]

        def instants(filter: Any = None) -> list[int]:
            return [book.currunix for book in graph.BookIterator(inputs(), filter=filter)]

        buys = list(graph.BookIterator(inputs(), 0, "side = 'BUYS'"))
        assert [book.currunix for book in buys] == [1, 3, 4]
        assert [delta.crosscode for delta in buys[1].deltas] == ["8:1:E-1"]
        assert [delta.crosscode for delta in buys[2].deltas] == ["14:0:B-2"]
        # A filter is any filter the expression layer reads: text, a Filter
        # or a Term over the `marketdata` row.
        assert instants(Filter("side = 'BUYS'")) == [1, 3, 4]
        assert instants(Term.column("marketdatakind").eq(Term.literal("QUOT"))) == [4]
        # An execution is recorded: a filter keeping it alone folds its book.
        assert instants("marketdatakind = 'EXEC'") == [3]
        # A filter keeping every row folds what no filter does.
        assert instants("true") == instants(None) == [1, 2, 3, 4]
        for refused in ("nope = 1", "price + 1"):
            with pytest.raises(ValueError):
                graph.BookIterator(inputs(), filter=refused)

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
