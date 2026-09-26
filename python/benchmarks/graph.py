"""Boundary cost of the graph vocabulary, against the native numbers.

Every case here is one crossing over the typed market leaves and
``MarketData``: building an order event from named facts, reading a fact back
typed, dating and undating an element, a book folding a stream of operations,
a book's limits and imbalance, the lazy book and event walks, the lifted
Arrow doors, the named views, and a FIX capture through the sorted market
doors. Run after installing the release wheel with::

    python benchmarks/graph.py --iterations 2000
"""

from __future__ import annotations

import argparse
import decimal
import gc
import pathlib
import statistics
import timeit
from collections.abc import Callable

from yggdryl import DataType, graph
from yggdryl.fix import FixCodec, FixRegistry

FOLD_OPERATION_COUNT = 512
CLOCK = 1_700_000_000_000_000_000
SEED = pathlib.Path(__file__).resolve().parent.parent.parent / "config" / "fix"


def _order_event(clock: int = CLOCK, **facts: object) -> graph.OrderEvent:
    base: dict[str, object] = {
        "crosscode": "G-1",
        "side": "BUY",
        "ticker": "ACME",
        "price": decimal.Decimal("100.25"),
        "currency": "USD",
        "quantity": 10,
    }
    base.update(facts)
    return graph.OrderEvent(clock, **base)


ORDER_EVENT = _order_event()
ORDER = ORDER_EVENT.into_element()
DATA = graph.MarketData(ORDER_EVENT)
LANE = graph.Lane(price=decimal.Decimal("100.25"), currency="USD", quantity=10)

# One order per price tick, all on the bid side of one symbol at one instant -
# the atomic group a book folds when it replays a session's orders.
FOLD_OPERATIONS = [
    _order_event(
        crosscode=f"G-{index}",
        price=decimal.Decimal("100") + decimal.Decimal(index),
    )
    for index in range(FOLD_OPERATION_COUNT)
]
FOLD_BOOK = graph.BookEvent(CLOCK, "ACME").with_operations(FOLD_OPERATIONS)
FOLD_SIDE = FOLD_BOOK.bid
FOLD_ROWS = graph.MarketData.arrow_reader(FOLD_OPERATIONS).read_all()

# One order a millisecond, one price tick each, as the codec parses them:
# the capture the sorted market doors expand and order.
CODEC = FixCodec(
    FixRegistry.from_handle(SEED),
    default_sending_time=DataType('datetime64(ns,"UTC")').scalar(CLOCK),
    exclude_msgtypes=[],
    threads=1,
)
CAPTURE = [
    CODEC.parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260921-10:00:%02d.%03d|11=C%d|55=ACME|54=1|44=%d|38=5|10=0|"
        % (index // 1000, index % 1000, index, 100 + index)
    )
    for index in range(FOLD_OPERATION_COUNT)
]


def _order_event_from_kwargs() -> graph.OrderEvent:
    return _order_event()


def _order_from_kwargs() -> graph.Order:
    return graph.Order(crosscode="G-1", side="BUY", price=decimal.Decimal("100.25"))


def _order_event_read_price() -> object:
    return ORDER_EVENT.price


def _order_event_read_bid_lane() -> object:
    return ORDER_EVENT.bid


def _order_event_read_altids() -> object:
    return ORDER_EVENT.altids


def _order_at() -> graph.OrderEvent:
    return ORDER.at(CLOCK)


def _order_event_into_element() -> graph.Order:
    return ORDER_EVENT.into_element()


def _market_data_wrap() -> graph.MarketData:
    return graph.MarketData(ORDER_EVENT)


def _market_data_into_leaf() -> object:
    return DATA.into_leaf()


def _lane_from_kwargs() -> graph.Lane:
    return graph.Lane(price=decimal.Decimal("100.25"), currency="USD", quantity=10)


def _lane_read_price() -> object:
    return LANE.price


def _book_fold() -> graph.BookEvent:
    return graph.BookEvent(CLOCK, "ACME").with_operations(FOLD_OPERATIONS)


def _book_iterator_drain() -> int:
    return sum(1 for _ in graph.BookIterator(FOLD_OPERATIONS))


def _event_iterator_drain() -> int:
    return sum(1 for _ in graph.EventIterator(FOLD_OPERATIONS))


def _operations_arrow_reader() -> int:
    return graph.MarketData.arrow_reader(FOLD_OPERATIONS).read_all().num_rows


def _operations_from_arrow_reader() -> int:
    reader = graph.MarketData.arrow_reader(FOLD_OPERATIONS)
    return sum(1 for _ in graph.MarketData.from_arrow_reader(reader))


def _book_arrow_reader() -> int:
    return graph.MarketData.arrow_reader([FOLD_BOOK]).read_all().num_rows


def _book_from_arrow_reader() -> int:
    reader = graph.MarketData.arrow_reader([FOLD_BOOK])
    return sum(1 for _ in graph.MarketData.from_arrow_reader(reader))


def _book_side_limits() -> int:
    return len(FOLD_SIDE.limits)


def _book_side_depth() -> object:
    return FOLD_SIDE.depth(10)


def _book_imbalance() -> object:
    return FOLD_BOOK.imbalance(10)


def _view_plan() -> object:
    return graph.MarketData.plan("orders", ["securityids['ISIN'] as isin"])


def _view_apply() -> int:
    return graph.MarketData.apply_view("orders", FOLD_ROWS).read_all().num_rows


def _market_operations() -> int:
    return sum(1 for _ in CODEC.market_operations(CAPTURE))


def _market_arrow_reader() -> int:
    return CODEC.market_arrow_reader(CAPTURE).read_all().num_rows


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=3)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:36} {nanoseconds:14.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=2_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    gc.disable()
    try:
        _measure("order event from kwargs", _order_event_from_kwargs, args.iterations)
        _measure("order from kwargs", _order_from_kwargs, args.iterations)
        _measure("order event read price", _order_event_read_price, args.iterations)
        _measure("order event read bid lane", _order_event_read_bid_lane, args.iterations)
        _measure("order event read altids map", _order_event_read_altids, args.iterations)
        _measure("order at", _order_at, args.iterations)
        _measure("order event into element", _order_event_into_element, args.iterations)
        _measure("market data wrap", _market_data_wrap, args.iterations)
        _measure("market data into leaf", _market_data_into_leaf, args.iterations)
        _measure("lane from kwargs", _lane_from_kwargs, args.iterations)
        _measure("lane read price", _lane_read_price, args.iterations)
        folds = max(1, args.iterations // 50)
        count = FOLD_OPERATION_COUNT
        _measure(f"book fold/{count}", _book_fold, folds)
        _measure(f"book iterator drain/{count}", _book_iterator_drain, folds)
        _measure(f"event iterator drain/{count}", _event_iterator_drain, folds)
        _measure(f"operations arrow_reader/{count}", _operations_arrow_reader, folds)
        _measure(f"operations from_arrow_reader/{count}", _operations_from_arrow_reader, folds)
        _measure("book arrow_reader", _book_arrow_reader, folds)
        _measure("book from_arrow_reader", _book_from_arrow_reader, folds)
        _measure(f"book side limits/{count}", _book_side_limits, folds)
        _measure("book side depth/10", _book_side_depth, args.iterations)
        _measure("book imbalance/10", _book_imbalance, args.iterations)
        _measure("view plan orders+lift", _view_plan, args.iterations)
        _measure(f"view apply orders/{count}", _view_apply, folds)
        _measure(f"fix market_operations/{count}", _market_operations, folds)
        _measure(f"fix market_arrow_reader/{count}", _market_arrow_reader, folds)
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
