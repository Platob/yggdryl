"""The graph vocabulary: the typed market leaves and :class:`MarketData`, the one value over them.

An order, a quote or an execution is a leaf of its own, undated
(:class:`Order`, :class:`Quote`, :class:`Execution`) or dated
(:class:`OrderEvent`, :class:`QuoteEvent`, :class:`ExecutionEvent`); a
composite trade is a :class:`TradeEvent`, a book a :class:`BookEvent` - the
deltas since the book before it and, on a complete book, the live entries of
both sides and each side's price levels - and the control a full-snapshot
message is a :class:`SnapshotEvent`. Every leaf answers the same fact getters
- the element, event, market and operation facts it states, and the
``MarketDataKind`` it is filed under - and :class:`MarketData` holds any one
of them, answering the element and market facts its leaf answers, with the
lifted ``marketdata`` Arrow doors (``field``, ``arrow_reader``,
``from_arrow_reader``) and the named views over them (``plan``,
``apply_view``, one of ``enums.MARKET_VIEWS`` each). :class:`BookRef` is the
typed book-control facts a market-data entry carries. A :class:`BookEvent`
answers each side's ``alive_on``, ``limits``, ``best_price`` and ``depth``,
and its ``spread``, ``is_locked`` and ``imbalance``; it reads its entries by
kind - ``ordlive`` the orders resting, ``orddelta``, ``quotes``,
``executions`` and ``events`` the deltas, ``events`` every delta that is no
order, quote or execution, empty by construction today; ``is_complete`` says
whether it holds its sides, and ``with_previous`` makes a book stating its
deltas alone whole over the book before it, ``BookEvent.keyed`` the empty
one. :class:`BookIterator` folds a sorted stream of orders, quotes and
snapshot controls into books - an execution is recorded among its book's
deltas and a trade pruned, and a ``filter`` narrows the walk further;
:class:`EventIterator` chains a stream of leaves to the live element each
follows. :class:`CandleIterator` folds a sorted stream of books into
:class:`Candle` values - one OHLC of the best bid, the best ask, their
midpoint and the spread per book cross code and bucket, the bucket aligned
to the zone :class:`CandleOptions` names - and :func:`candles` is that walk
drained into a list.

The operation leaves are built from named facts keyed by column name - a
fact given as ``...`` is skipped and ``None`` clears it - each checked by its
column's own field. Every class here is immutable; a verb that states a
change - ``with_previous``, ``merge_with``, ``restating``, ``with_book``,
``with_operations`` - answers a new value rather than changing the one it
was called on. Nothing here resolves, folds, merges or validates a fact:
every constructor redirects to the native core door named beside it.
"""

from __future__ import annotations

from .._native import (
    ENTRY_ID,
    ENTRY_REF_ID,
    BookEvent,
    BookIterator,
    BookRef,
    Candle,
    CandleIterator,
    CandleOptions,
    EventIterator,
    Execution,
    ExecutionEvent,
    MarketData,
    MarketDataRowIterator,
    Order,
    OrderEvent,
    Quote,
    QuoteEvent,
    SnapshotEvent,
    TradeEvent,
    candles,
)

__all__ = [
    "ENTRY_ID",
    "ENTRY_REF_ID",
    "BookEvent",
    "BookIterator",
    "BookRef",
    "Candle",
    "CandleIterator",
    "CandleOptions",
    "EventIterator",
    "Execution",
    "ExecutionEvent",
    "MarketData",
    "MarketDataRowIterator",
    "Order",
    "OrderEvent",
    "Quote",
    "QuoteEvent",
    "SnapshotEvent",
    "TradeEvent",
    "candles",
]
