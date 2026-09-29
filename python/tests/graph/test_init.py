"""The package surface: `python/yggdryl/graph/__init__.py` and `python/src/graph/mod.rs`."""

from __future__ import annotations

import yggdryl
from yggdryl import enums, graph

CLASSES = (
    "BookRef",
    "Order",
    "Quote",
    "Execution",
    "OrderEvent",
    "QuoteEvent",
    "ExecutionEvent",
    "TradeEvent",
    "BookEvent",
    "SnapshotEvent",
    "BookIterator",
    "MarketData",
    "MarketDataRowIterator",
    "EventIterator",
    "Candle",
    "CandleOptions",
    "CandleIterator",
)


def test_every_native_class_is_re_exported() -> None:
    for name in CLASSES:
        assert name in graph.__all__
        assert getattr(graph, name).__module__ == "yggdryl._native"
    assert sorted(graph.__all__) == sorted(
        [*CLASSES, "ENTRY_ID", "ENTRY_REF_ID", "FOLLOWED_ALTIDS", "candles"]
    )
    # The one convenience is a function over the walk, not a class.
    assert callable(graph.candles) and graph.candles.__doc__


def test_no_retired_name_survives() -> None:
    for name in (
        "MarketEnvelope",
        "MarketOperation",
        "Trade",
        "Book",
        "BookControl",
        "BookInput",
        "OperationEventData",
        "Lane",
        "BookSide",
        "SnapshotPartition",
        "GLOBAL_SYMBOL",
    ):
        assert not hasattr(graph, name)
        assert not hasattr(yggdryl._native, name)


def test_the_package_is_reachable_off_the_root() -> None:
    assert yggdryl.graph is graph
    assert "graph" in yggdryl.__all__


def test_the_three_constants_are_exported() -> None:
    assert graph.ENTRY_ID == "MDENTRYID"
    assert graph.ENTRY_REF_ID == "MDENTRYREFID"
    # A tuple, so no caller can change the module constant for every importer.
    assert isinstance(graph.FOLLOWED_ALTIDS, tuple)
    assert graph.FOLLOWED_ALTIDS
    assert all(isinstance(name, str) for name in graph.FOLLOWED_ALTIDS)


def test_the_enum_listings_name_the_column_vocabulary() -> None:
    # A named fact is a column name: every one the three listings spell is
    # a keyword an operation event is built from.
    event = graph.OrderEvent(1, crosscode="O-1")
    for column in (*enums.EVENT_COLUMNS, *enums.MARKET_COLUMNS, *enums.OPERATION_COLUMNS):
        assert hasattr(event, column), column
    assert len(enums.MARKET_COLUMNS) == 27 and len(enums.OPERATION_COLUMNS) == 3
    assert enums.MARKET_KINDS == graph.MarketData.kinds
    assert "book_side" not in enums.MARKET_KINDS


def test_every_public_member_has_a_docstring() -> None:
    undocumented = []
    for name in CLASSES:
        cls = getattr(graph, name)
        if not cls.__doc__:
            undocumented.append(name)
        for member_name, member in vars(cls).items():
            if member_name.startswith("_") or member_name == "kinds":
                continue
            if not getattr(member, "__doc__", None):
                undocumented.append(f"{name}.{member_name}")
    assert undocumented == []
