"""The graph vocabulary: the typed market leaves and :class:`MarketData`, the one value over them.

An order, a quote or an execution is a leaf of its own, undated
(:class:`Order`, :class:`Quote`, :class:`Execution`) or dated
(:class:`OrderEvent`, :class:`QuoteEvent`, :class:`ExecutionEvent`); a
composite trade is a :class:`TradeEvent`, a book a :class:`BookEvent` over two
:class:`BookSide`\\ s, and the control a full-snapshot message is a
:class:`SnapshotEvent`. Every leaf answers the same fact getters - the
element, event, market and operation facts it states - and
:class:`MarketData` holds any one of them, answering the element and market
facts its leaf answers, with the lifted ``marketdata`` Arrow doors
(``field``, ``arrow_reader``, ``from_arrow_reader``) and the named views over
them (``plan``, ``apply_view``, one of ``enums.MARKET_VIEWS`` each).
:class:`Lane` is one side of a quote; :class:`BookRef` the typed
book-control facts a market-data entry carries; :class:`SnapshotPartition`
one scope a full snapshot replaces. A :class:`BookSide` answers its
``limits`` and ``depth``, a :class:`BookEvent` its ``spread``,
``is_locked`` and ``imbalance``.
:class:`BookIterator` folds a sorted stream of leaves into books;
:class:`EventIterator` chains a stream of leaves to the live element each
follows.

The operation leaves are built from named facts keyed by column name - a
fact given as ``...`` is skipped and ``None`` clears it - each checked by its
column's own field. Every class here is immutable; a verb that states a
change - ``with_previous``, ``merge_with``, ``restating``, ``with_book``,
``with_operations``, ``with_operation`` - answers a new value rather than
changing the one it was called on. Nothing here resolves, folds, merges or
validates a fact: every constructor redirects to the native core door named
beside it.
"""

from __future__ import annotations

from .._native import (
    ENTRY_ID,
    ENTRY_REF_ID,
    FOLLOWED_ALTIDS,
    GLOBAL_SYMBOL,
    BookEvent,
    BookIterator,
    BookRef,
    BookSide,
    EventIterator,
    Execution,
    ExecutionEvent,
    Lane,
    MarketData,
    MarketDataRowIterator,
    Order,
    OrderEvent,
    Quote,
    QuoteEvent,
    SnapshotEvent,
    SnapshotPartition,
    TradeEvent,
)

__all__ = [
    "ENTRY_ID",
    "ENTRY_REF_ID",
    "FOLLOWED_ALTIDS",
    "GLOBAL_SYMBOL",
    "BookEvent",
    "BookIterator",
    "BookRef",
    "BookSide",
    "EventIterator",
    "Execution",
    "ExecutionEvent",
    "Lane",
    "MarketData",
    "MarketDataRowIterator",
    "Order",
    "OrderEvent",
    "Quote",
    "QuoteEvent",
    "SnapshotEvent",
    "SnapshotPartition",
    "TradeEvent",
]
