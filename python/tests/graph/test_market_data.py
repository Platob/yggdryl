"""``graph.MarketData`` and its lifted Arrow doors: `python/src/graph/market_data.rs`."""

from __future__ import annotations

import copy
import decimal
import pickle
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import Field, graph

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal


def leaves() -> list[Any]:
    """One leaf of every kind, in `MarketData.kinds` order."""
    order = graph.OrderEvent(CLOCK, crosscode="O-1", side="BUY", price=D("101"), quantity=5, ticker="ACME")
    quote = graph.QuoteEvent(CLOCK, crosscode="Q-1", side="SELL", price=D("102"), quantity=3, ticker="ACME")
    execution = graph.ExecutionEvent(CLOCK + 1, crosscode="O-1", side="BUY", lastpx=D("101"), lastqty=5)
    trade = graph.TradeEvent.from_parts(
        graph.ExecutionEvent(CLOCK, crosscode="T-1", ticker="ACME"),
        [
            graph.ExecutionEvent(CLOCK, crosscode="E-1", side="BUY", lastpx=1, lastqty=1),
            graph.ExecutionEvent(CLOCK, crosscode="E-2", side="SELL", lastpx=1, lastqty=1),
        ],
    )
    book = graph.BookEvent(CLOCK, "ACME").with_operations([order, quote])
    return [
        graph.Order(crosscode="O-1", price=D("101")),
        graph.Quote(crosscode="Q-1"),
        graph.Execution(crosscode="E-1", lastqty=2),
        graph.BookSide("BUY").with_operation(order),
        order,
        quote,
        execution,
        trade,
        book,
        graph.SnapshotEvent.snapshot(order, "S"),
    ]


def test_every_leaf_wraps_and_names_its_kind() -> None:
    wrapped = [graph.MarketData(leaf) for leaf in leaves()]
    assert tuple(data.kind for data in wrapped) == graph.MarketData.kinds
    assert [data.is_event for data in wrapped] == [False] * 4 + [True] * 6
    for leaf, data in zip(leaves(), wrapped):
        assert type(data.into_leaf()) is type(leaf)
        assert data.into_leaf() == leaf
        assert graph.MarketData(data) == data
        # The element and market facts delegate to the leaf.
        assert (data.curruuid, data.crosscode, data.price, data.side) == (
            leaf.curruuid,
            leaf.crosscode,
            leaf.price,
            leaf.side,
        )


def test_as_leaf_borrows_the_leaf_it_is_and_none_otherwise() -> None:
    order = leaves()[4]
    data = graph.MarketData(order)
    assert data.as_order_event() == order
    assert data.into_leaf() == order
    for name in (
        "as_order",
        "as_quote",
        "as_execution",
        "as_book_side",
        "as_quote_event",
        "as_execution_event",
        "as_trade_event",
        "as_book_event",
        "as_snapshot_event",
    ):
        assert getattr(data, name)() is None, name


def test_book_answers_the_control_of_an_operation_or_a_snapshot() -> None:
    control = graph.BookRef(action="0", scope="S")
    assert graph.MarketData(graph.QuoteEvent(CLOCK, book=control)).book == control
    snapshot = graph.MarketData(leaves()[-1])
    assert snapshot.book is not None and snapshot.book.action == "snapshot"
    assert graph.MarketData(graph.Order()).book is None


def test_anything_but_a_leaf_is_refused() -> None:
    with pytest.raises(TypeError, match="expected MarketData or a market leaf, got int"):
        graph.MarketData(1)  # type: ignore[arg-type]


def test_following_crosses_operation_kinds_and_merging_no_variant() -> None:
    first = graph.MarketData(graph.OrderEvent(CLOCK, crosscode="O-1", price=1))
    later = graph.MarketData(graph.OrderEvent(CLOCK + 1, crosscode="O-1", price=2))
    followed = later.with_previous(first)
    assert followed is not None and followed.kind == "order_event"
    followed_leaf = followed.as_order_event()
    assert followed_leaf is not None and followed_leaf.prevuuid == first.curruuid
    # An execution follows the order it fills across kinds, keeping its own.
    execution = graph.MarketData(graph.ExecutionEvent(CLOCK + 1_000_000, crosscode="O-1"))
    fill = execution.with_previous(first)
    assert fill is not None and fill.kind == "execution_event"
    assert fill.as_execution_event().prevuuid == first.curruuid
    # A book side follows no other variant, and a merge never crosses one.
    assert graph.MarketData(graph.BookSide("BUY")).with_previous(first) is None
    assert first.merge_with(execution) is None
    assert first.merge_with(first) is None
    assert later.is_after(first) and first.is_before(later)
    # An undated value is neither after nor before anything.
    assert not graph.MarketData(graph.Order()).is_after(first)


@pytest.mark.parametrize("index", range(10), ids=lambda index: graph.MarketData.kinds[index])
def test_equality_hash_repr_copy_pickle(index: int) -> None:
    leaf = leaves()[index]
    data = graph.MarketData(leaf)
    twin = pickle.loads(pickle.dumps(data))
    assert twin == data and hash(twin) == hash(data) == hash(leaf)
    assert copy.copy(data) == data and copy.deepcopy(data) == data
    assert repr(data) == (
        f'MarketData({data.curruuid.as_py()}, kind="{data.kind}", crosscode="{data.crosscode}")'
    )
    # Every leaf pickles through the same one-row stream.
    assert pickle.loads(pickle.dumps(leaf)) == leaf
    assert data != leaf


def test_the_field_is_the_lifted_marketdata_struct() -> None:
    field = graph.MarketData.field()
    assert isinstance(field, Field)
    assert field.name == "marketdata" and not field.nullable
    names = [child.name for child in field]
    assert names[0] == "kind"
    for name in ("currunix", "price", "altids", "bid", "ask", "mdupdateaction", "executions"):
        assert name in names
    for name in ("bidside", "askside", "snapshotpartitions", "live", "deltas"):
        assert name in names


def test_arrow_reader_round_trips_every_variant() -> None:
    items = leaves()
    reader = graph.MarketData.arrow_reader(items)
    assert isinstance(reader, pa.RecordBatchReader)
    table = reader.read_all()
    assert table.num_rows == 10
    assert table.column("kind").to_pylist() == list(graph.MarketData.kinds)
    back = list(graph.MarketData.from_arrow_reader(table))
    assert back == [graph.MarketData(item) for item in items]
    assert [data.into_leaf() for data in back] == items


def test_arrow_reader_takes_market_data_and_bounds_its_batches() -> None:
    items = [graph.MarketData(graph.OrderEvent(CLOCK + step, crosscode=f"O-{step}")) for step in range(5)]
    reader = graph.MarketData.arrow_reader(iter(items), batch_row_size=2)
    assert [batch.num_rows for batch in reader] == [2, 2, 1]
    assert graph.MarketData.arrow_reader([]).read_all().num_rows == 0


def test_arrow_reader_pulls_lazily_and_surfaces_a_python_failure() -> None:
    pulled: list[int] = []

    def items() -> Iterator[graph.OrderEvent]:
        for step in range(3):
            pulled.append(step)
            yield graph.OrderEvent(CLOCK + step)
        raise RuntimeError("the source gave up")

    reader = graph.MarketData.arrow_reader(items())
    assert pulled == []
    with pytest.raises(Exception, match="the source gave up"):
        reader.read_all()
    with pytest.raises(Exception, match="expected MarketData or a market leaf, got int"):
        graph.MarketData.arrow_reader([1]).read_all()  # type: ignore[list-item]


def test_a_lifecycle_shaped_batch_reads_into_events() -> None:
    # Event and operation columns in their own order, a foreign column, no
    # book column, the names folded, the types castable: the door resolves
    # what it knows once and ignores the rest.
    batch = pa.table(
        {
            "foreign": [1, 2],
            "CrossCode": ["O-1", "O-1"],
            "Kind": ["order_event", "execution_event"],
            "currunix": pa.array([CLOCK, CLOCK + 1], pa.int64()),
            "side": ["BUY", "BUY"],
            "price": ["101", None],
            "lastqty": [None, "5"],
            "altids": pa.array([[("ORDERID", "O-1")], None], pa.map_(pa.string(), pa.string())),
        }
    )
    order, execution = (data.into_leaf() for data in graph.MarketData.from_arrow_reader(batch))
    assert isinstance(order, graph.OrderEvent) and isinstance(execution, graph.ExecutionEvent)
    assert (order.currunix, order.crosscode, order.side.as_py()) == (CLOCK, "O-1", "BUY")
    assert order.price is not None and order.price.as_py() == D("101")
    assert order.altids == {"ORDERID": "O-1"}
    assert execution.lastqty is not None and execution.lastqty.as_py() == 5
    assert execution.currunix == CLOCK + 1


def test_from_arrow_reader_refuses_by_name_and_fuses() -> None:
    with pytest.raises(ValueError, match=r"\$\[0\]\.kind: expected order, quote.*got null"):
        list(graph.MarketData.from_arrow_reader(pa.table({"currunix": [CLOCK]})))
    with pytest.raises(ValueError, match=r'\$\[0\]\.kind: .*got "nope"'):
        list(graph.MarketData.from_arrow_reader(pa.table({"kind": ["nope"]})))
    rows = graph.MarketData.from_arrow_reader(
        pa.table({"kind": ["order_event", "order_event"], "currunix": pa.array([CLOCK, None], pa.int64())})
    )
    assert next(rows).kind == "order_event"
    with pytest.raises(ValueError, match=r"\$\[1\]\.currunix: expected the instant a dated leaf happened at"):
        next(rows)
    assert list(rows) == []


def test_an_undated_row_needs_no_clock() -> None:
    [data] = graph.MarketData.from_arrow_reader(pa.table({"kind": ["order"], "crosscode": ["X"]}))
    assert data.kind == "order" and data.crosscode == "X"
    assert data.as_order() == graph.Order(crosscode="X")
