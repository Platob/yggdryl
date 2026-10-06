"""``graph.MarketData`` and its lifted Arrow doors: `python/src/graph/market_data.rs`."""

from __future__ import annotations

import copy
import decimal
import inspect
import pickle
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, FieldPath, Identifier, MarketDataKind, Plan, Side, TimeInForce, enums, graph
from yggdryl.fix import FixCodec, FixMsg, FixRegistry

CLOCK = 1_700_000_000_000_000_000
D = decimal.Decimal

#: The Arrow layout of an identifier column: a sorted map from the key's
#: text - `src:type`, the type alone for the base source - to its value.
IDENTIFIERS_MAP = pa.map_(
    pa.field("key", pa.string(), nullable=False),
    pa.field("value", pa.string(), nullable=False),
    keys_sorted=True,
)


def leaves() -> list[Any]:
    """One leaf of every kind, in `MarketData.kinds` order."""
    order = graph.OrderEvent(CLOCK, crosscode="O-1", side="BUYS", price=D("101"), quantity=5, ticker="ACME")
    quote = graph.QuoteEvent(CLOCK, crosscode="Q-1", side="SELL", price=D("102"), quantity=3, ticker="ACME")
    execution = graph.ExecutionEvent(CLOCK + 1, crosscode="O-1", side="BUYS", lastpx=D("101"), lastqty=5)
    trade = graph.TradeEvent.from_parts(
        graph.ExecutionEvent(CLOCK, crosscode="T-1", ticker="ACME"),
        [
            graph.ExecutionEvent(CLOCK, crosscode="E-1", side="BUYS", lastpx=1, lastqty=1),
            graph.ExecutionEvent(CLOCK, crosscode="E-2", side="SELL", lastpx=1, lastqty=1),
        ],
    )
    book = graph.BookEvent(CLOCK, "ACME").with_operations([order, quote])
    return [
        graph.Order(crosscode="O-1", price=D("101"), strikepx=D("4600.5")),
        graph.Quote(crosscode="Q-1"),
        graph.Execution(crosscode="E-1", lastqty=2),
        order,
        quote,
        execution,
        trade,
        book,
        graph.SnapshotEvent.snapshot(order, "S"),
    ]


#: The `MarketDataKind` of each leaf `leaves()` answers, in its order.
KINDS = [
    MarketDataKind.ORDR,
    MarketDataKind.QUOT,
    MarketDataKind.EXEC,
    MarketDataKind.ORDR,
    MarketDataKind.QUOT,
    MarketDataKind.EXEC,
    MarketDataKind.TRAD,
    MarketDataKind.BOOK,
    MarketDataKind.BOOK,
]


def test_every_leaf_wraps_and_names_its_kind() -> None:
    wrapped = [graph.MarketData(leaf) for leaf in leaves()]
    # Every kind but `fix`, a FIX message held whole, is one of the leaves.
    assert tuple(data.kind for data in wrapped) + ("fix",) == graph.MarketData.kinds
    assert [data.marketdatakind for data in wrapped] == KINDS
    assert [leaf.marketdatakind for leaf in leaves()] == KINDS
    assert [data.is_event for data in wrapped] == [False] * 3 + [True] * 6
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
        assert isinstance(data.side, Side)


def test_as_leaf_borrows_the_leaf_it_is_and_none_otherwise() -> None:
    order = leaves()[3]
    data = graph.MarketData(order)
    assert data.as_order_event() == order
    assert data.into_leaf() == order
    for name in (
        "as_order",
        "as_quote",
        "as_execution",
        "as_quote_event",
        "as_execution_event",
        "as_trade_event",
        "as_book_event",
        "as_snapshot_event",
    ):
        assert getattr(data, name)() is None, name
    assert not hasattr(data, "as_book_side")


def test_book_answers_the_control_of_an_operation_or_a_snapshot() -> None:
    control = graph.BookRef(action="0", scope="S")
    assert graph.MarketData(graph.QuoteEvent(CLOCK, book=control)).book == control
    snapshot = graph.MarketData(leaves()[-1])
    assert snapshot.book is not None and snapshot.book.action == "snapshot"
    assert graph.MarketData(graph.Order()).book is None


def test_a_fix_message_is_held_whole_and_split_where_it_is_written() -> None:
    codec = FixCodec(FixRegistry(), default_sending_time=DataType('datetime64(ns,"UTC")').scalar(CLOCK))
    message = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20240102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|59=1|10=0|"
    )
    data = graph.MarketData(message)
    assert data.kind == "fix"
    # The kind is the category the message's dictionary files it under.
    assert data.marketdatakind is MarketDataKind.ORDR == message.marketdatakind
    assert data.is_event
    held = data.as_fix()
    assert isinstance(held, FixMsg) and held == message
    assert held.timeinforce is TimeInForce.GTC
    assert isinstance(data.into_leaf(), FixMsg)
    assert data.as_order_event() is None
    # Pickle carries the message itself, never the rows it splits into.
    assert pickle.loads(pickle.dumps(data)) == data
    assert copy.copy(data) == data
    # The Arrow writer and the book split it into the leaves it reports.
    [leaf] = message.market_data()
    rows = list(graph.MarketData.from_arrow_reader(graph.MarketData.arrow_reader([data, message])))
    assert [row.kind for row in rows] == ["order_event", "order_event"]
    assert rows[0] == leaf
    assert len(list(graph.BookIterator([message]))) == 1


def test_anything_but_a_leaf_is_refused() -> None:
    with pytest.raises(TypeError, match="expected MarketData, a market leaf or a FixMsg, got int"):
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
    fill_leaf = fill.as_execution_event()
    assert fill_leaf is not None and fill_leaf.prevuuid == first.curruuid
    # A book follows no operation, and a merge never crosses a variant.
    assert graph.MarketData(graph.BookEvent(CLOCK + 2, "O-1")).with_previous(first) is None
    assert first.merge_with(execution) is None
    assert first.merge_with(first) is None
    assert later.is_after(first) and first.is_before(later)
    # An undated value is neither after nor before anything.
    assert not graph.MarketData(graph.Order()).is_after(first)


@pytest.mark.parametrize("index", range(9), ids=lambda index: graph.MarketData.kinds[index])
def test_equality_hash_repr_copy_pickle(index: int) -> None:
    leaf = leaves()[index]
    data = graph.MarketData(leaf)
    twin = pickle.loads(pickle.dumps(data))
    assert twin == data and hash(twin) == hash(data) == hash(leaf)
    assert copy.copy(data) == data and copy.deepcopy(data) == data
    assert repr(data) == (
        f'MarketData({data.curruuid.as_py()}, kind="{data.kind}", '
        f'marketdatakind={data.marketdatakind}, crosscode="{data.crosscode}")'
    )
    # Every leaf pickles through the same one-row stream.
    assert pickle.loads(pickle.dumps(leaf)) == leaf
    assert data != leaf


def test_the_field_is_the_lifted_marketdata_struct() -> None:
    field = graph.MarketData.field()
    assert isinstance(field, Field)
    assert field.name == "marketdata" and not field.nullable
    names = [child.name for child in field]
    assert names[:7] == [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "srcuuids",
        "currunix",
    ]
    assert names.index("marketdatakind") == 15
    assert names[16] == "marketdatatype"
    # The strike is the market fact after the ticker, before the metadata.
    assert len(names) == 63
    assert names.index("strikepx") == names.index("ticker") + 1 == 48
    assert names[49] == "metadata"
    assert str(field["strikepx"].dtype) == "decimal"
    for name in (
        "currunix",
        "price",
        "side",
        "isincode",
        "fxrates",
        "bidpx",
        "bidqty",
        "bidccy",
        "askpx",
        "askqty",
        "askccy",
        "identifiers",
        "bookscope",
        "alive",
        "deltas",
        "executions",
        "bidlimits",
        "asklimits",
    ):
        assert name in names, name
    for retired in (
        "kind",
        "bid",
        "ask",
        "mdupdateaction",
        "bidside",
        "askside",
        "snapshotpartitions",
        "live",
        "limits",
        "userids",
        "marketoperationid",
        "spread",
    ):
        assert retired not in names, retired


def test_arrow_reader_round_trips_every_variant() -> None:
    items = leaves()
    reader = graph.MarketData.arrow_reader(items)
    assert isinstance(reader, pa.RecordBatchReader)
    table = reader.read_all()
    assert table.num_rows == 9
    assert table.column("marketdatakind").to_pylist() == [int(kind) for kind in KINDS]
    assert table.column("strikepx").to_pylist() == [D("4600.5")] + [None] * 8
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
    with pytest.raises(Exception, match="expected MarketData, a market leaf or a FixMsg, got int"):
        graph.MarketData.arrow_reader([1]).read_all()  # type: ignore[list-item]


def test_a_lifecycle_shaped_batch_reads_into_events() -> None:
    # Event and operation columns in their own order, a foreign column, no
    # book column, the names folded, the types castable: the door resolves
    # what it knows once and ignores the rest.
    batch = pa.table(
        {
            "foreign": [1, 2],
            # A row states the stored cross code: its kind, its side, its base.
            "CrossCode": ["10:1:O-1", "8:1:O-1"],
            "MarketDataKind": pa.array([10, 8], pa.int32()),
            "currunix": pa.array([CLOCK, CLOCK + 1], pa.int64()),
            "side": pa.array([1, 1], pa.int32()),
            "price": ["101", None],
            "lastqty": [None, "5"],
            "identifiers": pa.array(
                [[("orderid", "O-1")], None],
                IDENTIFIERS_MAP,
            ),
        }
    )
    order, execution = (data.into_leaf() for data in graph.MarketData.from_arrow_reader(batch))
    assert isinstance(order, graph.OrderEvent) and isinstance(execution, graph.ExecutionEvent)
    assert (order.currunix, order.crosscode, order.side) == (CLOCK, "10:1:O-1", Side.BUYS)
    assert order.price is not None and order.price.as_py() == D("101")
    assert order.identifiers.get_from("orderid") == "O-1"
    assert [(i.key, i.src, i.type, i.value) for i in order.identifiers] == [
        ("orderid", "base", "orderid", "O-1")
    ]
    assert execution.lastqty is not None and execution.lastqty.as_py() == 5
    assert execution.currunix == CLOCK + 1


def test_from_arrow_reader_refuses_by_name_and_fuses() -> None:
    with pytest.raises(ValueError, match=r"\$\[0\]\.marketdatakind: expected ORDR, QUOT.*got null"):
        list(graph.MarketData.from_arrow_reader(pa.table({"currunix": [CLOCK]})))
    with pytest.raises(ValueError, match=r"\$\[0\]\.marketdatakind: .*got ACCT"):
        list(graph.MarketData.from_arrow_reader(pa.table({"marketdatakind": pa.array([1], pa.int32())})))
    rows = graph.MarketData.from_arrow_reader(
        pa.table(
            {
                "marketdatakind": pa.array([10, 21], pa.int32()),
                "currunix": pa.array([CLOCK, None], pa.int64()),
            }
        )
    )
    assert next(rows).kind == "order_event"
    with pytest.raises(ValueError, match=r"\$\[1\]\.marketdatakind: expected a dated TRAD row"):
        next(rows)
    assert list(rows) == []


def test_an_undated_row_needs_no_clock() -> None:
    [data] = graph.MarketData.from_arrow_reader(
        pa.table({"marketdatakind": pa.array([10], pa.int32()), "crosscode": ["10:0:X"]})
    )
    assert data.kind == "order" and data.crosscode == "10:0:X"
    assert data.as_order() == graph.Order(crosscode="X")


def test_a_row_states_the_stored_cross_code_and_nothing_else() -> None:
    # The code is derived from the row's kind and side, so an unprefixed code,
    # or one of another kind or side, is refused where it is read.
    for stated in ("X", "8:0:X", "10:1:X"):
        with pytest.raises(ValueError, match=r"\$\[0\]\.crosscode: expected the value derived from the row"):
            list(
                graph.MarketData.from_arrow_reader(
                    pa.table({"marketdatakind": pa.array([10], pa.int32()), "crosscode": [stated]})
                )
            )


# The root's nested columns: what a flat view drops.
NESTED = ("alive", "deltas", "executions", "bidlimits", "asklimits")
ISIN = "US0378331005"


def _stream() -> pa.RecordBatchReader:
    """Every leaf kind, and an order stating its ISIN."""
    identified = graph.OrderEvent(
        CLOCK + 5, crosscode="O-5", side="BUYS", securityids=[Identifier("isin", ISIN)]
    )
    return graph.MarketData.arrow_reader([*leaves(), identified])


def _root() -> pa.Schema:
    return graph.MarketData.field().into_arrow_schema()


def _flat() -> list[str]:
    return [name for name in _root().names if name not in NESTED]


def _prefixed(column: str, prefix: str) -> list[str]:
    """The children of one nested column's item, each under ``prefix``."""
    held = _root().field(column).type
    item = held.value_type if isinstance(held, pa.ListType) else held
    return [f"{prefix}.{child.name}" for child in item]


def _view(view: str, lifts: Any = (), **crosscode: str) -> pa.Table:
    return graph.MarketData.apply_view(view, _stream(), lifts, **crosscode).read_all()


@pytest.mark.parametrize(
    ("view", "kind", "rows"),
    [
        ("orders", MarketDataKind.ORDR, 3),
        ("quotes", MarketDataKind.QUOT, 2),
        ("executions", MarketDataKind.EXEC, 2),
    ],
)
def test_an_operation_view_keeps_its_kind_and_every_flat_column(
    view: str, kind: MarketDataKind, rows: int
) -> None:
    table = _view(view)
    assert table.schema.names == _flat()
    assert table.column("marketdatakind").to_pylist() == [int(kind)] * rows


def test_a_trade_is_one_row_per_execution_its_own_columns_beside_it() -> None:
    table = _view("trades")
    assert table.schema.names == [*_flat(), *_prefixed("executions", "execution")]
    assert table.column("crosscode").to_pylist() == ["21:0:T-1", "21:0:T-1"]
    assert sorted(table.column("execution.crosscode").to_pylist()) == ["8:1:E-1", "8:2:E-2"]


def test_a_book_is_one_row_of_its_own() -> None:
    books = _view("books")
    kept = [name for name in _root().names if name != "executions"]
    assert books.schema.names == kept
    # The snapshot control states no alive entries: the view keeps books.
    assert books.column("marketdatakind").to_pylist() == [int(MarketDataKind.BOOK)]
    assert books.column("crosscode").to_pylist() == ["3:0:ACME"]


def test_a_lifecycle_is_one_chain_ordered_and_needs_its_crosscode() -> None:
    chain = [
        graph.OrderEvent(CLOCK + step, crosscode="C-1")
        for step in (30, 10, 20)
    ]
    other = graph.OrderEvent(CLOCK, crosscode="C-2")
    assert chain[0].crosscode == "10:0:C-1", "a chain is named by its stored code"
    source = graph.MarketData.arrow_reader([*chain, other])
    table = graph.MarketData.apply_view("lifecycle", source, crosscode="10:0:C-1").read_all()
    assert table.schema.names == _flat()
    assert table.column("currunix").cast(pa.int64()).to_pylist() == [CLOCK + 10, CLOCK + 20, CLOCK + 30]
    # The view filters on the exact stored code: the base alone names no chain.
    bare = graph.MarketData.apply_view("lifecycle", graph.MarketData.arrow_reader([*chain, other]), crosscode="C-1")
    assert bare.read_all().num_rows == 0
    with pytest.raises(ValueError, match="crosscode"):
        graph.MarketData.plan("lifecycle")
    with pytest.raises(ValueError, match="crosscode"):
        graph.MarketData.plan("orders", crosscode="C-1")
    with pytest.raises(ValueError, match="books, lifecycle"):
        graph.MarketData.plan("book_sides")


def test_a_lift_reads_one_key_null_where_missing_and_refuses_a_missing_column() -> None:
    table = _view(
        "orders",
        ["securityids['isin'] as isin", FieldPath("securityids['wkn'] as wkn")],
    )
    assert table.schema.names == [*_flat(), "isin", "wkn"]
    by_code = dict(zip(table.column("crosscode").to_pylist(), table.column("isin").to_pylist()))
    assert by_code == {"10:0:O-1": None, "10:1:O-1": None, "10:1:O-5": ISIN}
    assert table.column("wkn").null_count == table.num_rows
    with pytest.raises(Exception, match="nothing"):
        _view("orders", ["nothing['ISIN'] as isin"])
    # A path is resolved once at the boundary: text that is none is refused.
    with pytest.raises(ValueError):
        graph.MarketData.plan("orders", ["securityids["])
    with pytest.raises(TypeError):
        graph.MarketData.plan("orders", [1])  # type: ignore[list-item]


@pytest.mark.parametrize("view", enums.MARKET_VIEWS)
def test_a_view_plan_reads_back_as_its_text(view: str) -> None:
    crosscode = {"crosscode": "C-1"} if view == "lifecycle" else {}
    plan = graph.MarketData.plan(view, **crosscode)
    assert isinstance(plan, Plan)
    assert Plan(str(plan)) == plan
    lifted = graph.MarketData.plan(view, ["securityids['isin'] as isin"], **crosscode)
    assert "securityids['isin'] as isin" in str(lifted)
    assert Plan(str(lifted)) == lifted
    # The case of a spelling is not a view of its own.
    assert graph.MarketData.plan(view.upper(), **crosscode) == plan


def test_the_view_doors_state_their_defaults() -> None:
    # No lift is the empty sequence, and only the lifecycle takes a crosscode.
    assert str(inspect.signature(graph.MarketData.plan)) == "(view, lifts=(), *, crosscode=None)"
    assert str(inspect.signature(graph.MarketData.apply_view)) == (
        "(view, source, lifts=(), *, crosscode=None)"
    )
    # A lift list is a sequence of paths, never one path's characters.
    with pytest.raises(TypeError):
        graph.MarketData.plan("orders", "securityids['isin'] as isin")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        graph.MarketData.apply_view("orders", _stream(), "securityids['isin'] as isin")  # type: ignore[arg-type]
    # None is no lifts, as Node reads null: the view's own plan at both doors.
    assert graph.MarketData.plan("orders", None) == graph.MarketData.plan("orders")
    assert graph.MarketData.plan("lifecycle", None, crosscode="C-1") == (
        graph.MarketData.plan("lifecycle", crosscode="C-1")
    )
    assert graph.MarketData.apply_view("orders", _stream(), None).read_all().equals(_view("orders"))


def test_the_plans_the_views_are() -> None:
    nested = ", ".join(NESTED)
    assert str(graph.MarketData.plan("orders", ["securityids['isin'] as isin"])) == (
        f"select * exclude ({nested}), securityids['isin'] as isin "
        "where marketdatakind = 'ORDR'"
    )
    # A book states its deltas - a complete one its alive entries beside
    # them - where a snapshot control states neither.
    assert str(graph.MarketData.plan("books")) == (
        "select * exclude (executions) where marketdatakind = 'BOOK' and deltas is not null"
    )
    # Applying a view is applying its plan.
    plan = graph.MarketData.plan("trades")
    assert plan.apply_arrow_reader(_stream()).read_all().equals(_view("trades"))


def test_an_identifier_column_is_a_sorted_map_from_its_key_to_its_value() -> None:
    order = graph.OrderEvent(CLOCK, crosscode="O-1", identifiers=[Identifier("orderid", "O-1")])
    column = graph.MarketData.arrow_reader([order]).read_all().column("identifiers")
    assert pa.types.is_map(column.type) and column.type.keys_sorted
    assert column.type.key_type == pa.string() and column.type.item_type == pa.string()
    assert column.to_pylist() == [[("orderid", "O-1")]]
    # The three sets are maps alike, and a leaf stating none states an empty one.
    table = graph.MarketData.arrow_reader([graph.QuoteEvent(CLOCK)]).read_all()
    assert [pa.types.is_map(table.schema.field(name).type) for name in ("securityids", "identifiers", "partyids")] == [
        True,
        True,
        True,
    ]


def test_a_row_stating_a_key_no_identifier_reads_is_refused() -> None:
    table = pa.table(
        {
            "marketdatakind": pa.array([10], pa.int32()),
            "currunix": pa.array([CLOCK], pa.int64()),
            "identifiers": pa.array([[("fix:", "O-1")]], IDENTIFIERS_MAP),
        }
    )
    with pytest.raises(ValueError, match=r"fix:"):
        list(graph.MarketData.from_arrow_reader(table))


def test_a_row_naming_its_keys_in_any_spelling_reads_them_closed() -> None:
    table = pa.table(
        {
            "marketdatakind": pa.array([10], pa.int32()),
            "currunix": pa.array([CLOCK], pa.int64()),
            "identifiers": pa.array([[("ullink:clordid", "C-1")]], IDENTIFIERS_MAP),
        }
    )
    [order] = (data.into_leaf() for data in graph.MarketData.from_arrow_reader(table))
    assert order.identifiers.into_dict() == {"clordid": "C-1", "ullink:clordid": "C-1"}


def test_a_lift_reaches_one_identifier_of_a_map_by_its_key() -> None:
    source = graph.MarketData.arrow_reader(
        [
            graph.OrderEvent(CLOCK, crosscode="C-1", side="BUYS", identifiers=[Identifier("clordid", "C-1")]),
            graph.OrderEvent(CLOCK + 1, crosscode="C-2", side="BUYS"),
        ]
    )
    table = graph.MarketData.apply_view("orders", source, ["identifiers['clordid'] as clordid"]).read_all()
    assert table.column("clordid").to_pylist() == ["C-1", None], "null where the key is missing"
    assert table.column("crosscode").to_pylist() == ["10:1:C-1", "10:1:C-2"]
    plan = graph.MarketData.plan("orders", ["identifiers['clordid'] as clordid"])
    assert "identifiers['clordid'] as clordid" in str(plan) and Plan(str(plan)) == plan
