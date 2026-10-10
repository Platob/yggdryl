"""Candles, their options and the candle walk: `python/src/graph/candle.rs`."""

from __future__ import annotations

import copy
import datetime
import decimal
import pickle
import subprocess
import sys
import types
from collections.abc import Iterable, Iterator
from typing import Any

import pytest

from yggdryl import Timezone, graph

D = decimal.Decimal
SECOND = 1_000_000_000
MINUTE = 60 * SECOND
HOUR = 60 * MINUTE
# `2026-03-29T00:00:00Z`, the day Europe/Zurich springs forward at `01:00Z`:
# its wall clock skips `02:00`-`03:00`.
SPRING_DAY = 1_774_742_400 * SECOND
# `2026-10-25T00:00:00Z`, the day Europe/Zurich falls back at `01:00Z`: its
# wall clock reads `02:00`-`03:00` twice.
FALL_DAY = 1_792_886_400 * SECOND
# `2026-01-05T10:00:00Z`.
OFFSET_DAY = 1_767_607_200 * SECOND
READINGS = ("bid", "ask", "mid", "spread")
CELLS = ("open", "high", "low", "close")
NAMES = [
    "crosscode",
    "ticker",
    "start",
    "end",
    *(f"{reading}{cell}" for reading in READINGS for cell in CELLS),
    "bidqty",
    "askqty",
    "books",
]


def quote(
    unix: int, ticker: str, code: str, side: str, price: str, quantity: int
) -> graph.QuoteEvent:
    """One quote of `ticker` going by `code`."""
    return graph.QuoteEvent(
        unix,
        crosscode=code,
        ticker=ticker,
        instcode=ticker,
        side=side,
        price=D(price),
        quantity=quantity,
        state="NEW",
    )


def books(operations: Iterable[Any]) -> list[graph.BookEvent]:
    """The books `operations` fold into, in stream order."""
    return list(graph.BookIterator(operations))


def candles(operations: Iterable[Any], options: Any) -> list[graph.Candle]:
    """The candles `operations` fold into under `options`."""
    return list(graph.CandleIterator(books(operations), options))


def empty_candles(instants: Iterable[int], options: Any) -> list[graph.Candle]:
    """The candles of empty books at `instants` under `options`."""
    return list(
        graph.CandleIterator([graph.BookEvent(unix, "ACME") for unix in instants], options)
    )


def reading(value: dict[str, Any] | None) -> tuple[Any, ...] | None:
    """One reading as its four decimals, open to close."""
    if value is None:
        return None
    assert list(value) == list(CELLS)
    return tuple(value[cell].as_py() for cell in CELLS)


def ohlc(open: str, high: str, low: str, close: str) -> tuple[Any, ...]:
    """The reading `open, high, low, close` spell."""
    return (D(open), D(high), D(low), D(close))


def local_hour(zone: str, unix: int) -> int:
    """The wall-clock hour of the day `zone` reads at `unix`."""
    return Timezone(zone).into_local(unix // SECOND) % 86_400 // 3_600


ONE_MINUTE = [
    quote(10 * SECOND, "ACME", "B", "BUY", "100", 5),
    quote(10 * SECOND, "ACME", "A", "SELL", "103", 7),
    quote(20 * SECOND, "ACME", "B", "BUY", "102", 5),
    quote(30 * SECOND, "ACME", "A", "SELL", "102.5", 7),
    quote(30 * SECOND, "ACME", "B", "BUY", "99", 5),
    quote(40 * SECOND, "ACME", "B", "BUY", "101", 8),
    quote(40 * SECOND, "ACME", "A", "SELL", "103.5", 9),
]


def full_candle() -> graph.Candle:
    """A candle stating every cell."""
    cells: dict[str, Any] = {
        "crosscode": "ACME",
        "ticker": "ACME",
        "start": MINUTE,
        "end": 2 * MINUTE,
        "bidqty": 8,
        "askqty": 9,
        "books": 4,
    }
    for name, values in (
        ("bid", ("100", "102", "99", "101")),
        ("ask", ("103", "103.5", "102.5", "103.5")),
        ("mid", ("101.5", "102.5", "100.75", "102.25")),
        ("spread", ("3", "3.5", "1", "2.5")),
    ):
        cells.update(zip((f"{name}{cell}" for cell in CELLS), map(D, values)))
    return graph.Candle.from_scalar(cells)


def empty_candle() -> graph.Candle:
    """A candle of a book that stated nothing."""
    return graph.Candle.from_scalar(
        {"crosscode": "3:0:ACME", "start": 0, "end": MINUTE, "books": 1}
    )


class TestCandleOptions:
    def test_an_interval_must_be_positive(self) -> None:
        for interval in (0, -1, -(2**63)):
            with pytest.raises(ValueError, match=rf"\$\.interval.*got {interval}"):
                graph.CandleOptions(interval)
        options = graph.CandleOptions(1)
        assert (options.interval, options.spelling) == (1, "1ns")
        assert options.timezone == Timezone.UTC and options.timezone.is_utc()

    def test_every_spelling_reads_and_writes_back(self) -> None:
        for spelling, interval in (
            ("30s", 30 * SECOND),
            ("1m", MINUTE),
            ("5m", 5 * MINUTE),
            ("1h", HOUR),
            ("1d", 24 * HOUR),
            ("1w", 7 * 24 * HOUR),
            ("250ms", 250_000_000),
            ("7us", 7_000),
            ("3ns", 3),
        ):
            options = graph.CandleOptions(spelling)
            assert (options.interval, options.spelling) == (interval, spelling)
            assert options.timezone.is_utc()
        # The widest unit that divides exactly is the one written.
        assert graph.CandleOptions(90 * SECOND).spelling == "90s"
        assert graph.CandleOptions(120 * SECOND).spelling == "2m"
        assert graph.CandleOptions(1_500_000_000).spelling == "1500ms"
        assert graph.CandleOptions(14 * 24 * HOUR).spelling == "2w"
        for refused in (
            "",
            "m",
            "0s",
            "1x",
            "1.5m",
            "1 m",
            " 1m",
            "-1m",
            "1M",
            "1min",
            "99999999999999999999s",
            "100000000000w",
        ):
            with pytest.raises(ValueError, match=r"\$\.interval"):
                graph.CandleOptions(refused)

    def test_the_intake_takes_an_int_a_spelling_or_options(self) -> None:
        zurich = graph.CandleOptions("1h", "Europe/Zurich")
        assert zurich == graph.CandleOptions(HOUR, Timezone("Europe/Zurich"))
        assert zurich != graph.CandleOptions(HOUR)
        assert str(zurich.timezone) == "Europe/Zurich"
        # Options given again are those options, re-zoned where a zone is given.
        assert graph.CandleOptions(zurich) == zurich
        assert graph.CandleOptions(zurich, "UTC") == graph.CandleOptions("1h")
        for wrong in (True, 1.5, None, b"1m"):
            with pytest.raises(TypeError):
                graph.CandleOptions(wrong)  # type: ignore[arg-type]

    def test_equality_hash_repr_copy_pickle(self) -> None:
        options = graph.CandleOptions("5m", "Europe/Zurich")
        twin = graph.CandleOptions(5 * MINUTE, "Europe/Zurich")
        assert options == twin and hash(options) == hash(twin) and len({options, twin}) == 1
        assert repr(options) == 'CandleOptions("5m", timezone="Europe/Zurich")'
        assert copy.copy(options) == options and copy.deepcopy(options) == options
        assert pickle.loads(pickle.dumps(options)) == options


class TestCandleIterator:
    def test_the_ohlc_of_every_reading_over_one_minute(self) -> None:
        folded = books(ONE_MINUTE)
        assert len(folded) == 4, "one book per instant"
        walk = graph.CandleIterator(folded, "1m")
        assert walk.options == graph.CandleOptions("1m")
        (candle,) = list(walk)
        assert (candle.crosscode, candle.ticker, candle.start, candle.end) == (
            "3:0:ACME",
            "ACME",
            0,
            MINUTE,
        )
        assert reading(candle.bid) == ohlc("100", "102", "99", "101")
        assert reading(candle.ask) == ohlc("103", "103.5", "102.5", "103.5")
        assert reading(candle.mid) == ohlc("101.5", "102.5", "100.75", "102.25")
        assert reading(candle.spread) == ohlc("3", "3.5", "1", "2.5")
        assert candle.bidqty is not None and candle.bidqty.as_py() == D(8)
        assert candle.askqty is not None and candle.askqty.as_py() == D(9)
        assert candle.books == 4
        assert walk.__hash__ is None

    def test_a_one_sided_book_states_no_mid_or_spread(self) -> None:
        (candle,) = candles(
            [
                quote(10 * SECOND, "ACME", "B", "BUY", "100", 5),
                quote(20 * SECOND, "ACME", "B", "BUY", "101", 6),
            ],
            "1m",
        )
        assert reading(candle.bid) == ohlc("100", "101", "100", "101")
        assert candle.ask is None and candle.mid is None and candle.spread is None
        assert candle.bidqty is not None and candle.bidqty.as_py() == D(6)
        assert candle.askqty is None
        assert candle.books == 2
        assert candle.as_py()["askopen"] is None

    def test_a_walk_records_a_fill_whose_book_moves_no_price(self) -> None:
        # A book records an execution: an instant only a fill reached emits a
        # book stating it alone, which opens its bucket and counts one book
        # at the top of book the fill left standing.
        fill = graph.ExecutionEvent(
            90 * SECOND,
            crosscode="E-1",
            ticker="ACME",
            instcode="ACME",
            side="BUY",
            price=D("100"),
            lastqty=4,
            state="FILLED",
        )
        [first, filled] = candles([*ONE_MINUTE, fill], "1m")
        assert [first] == candles(ONE_MINUTE, "1m")
        assert (filled.start, filled.end, filled.books) == (60 * SECOND, 120 * SECOND, 1)
        assert reading(filled.bid) == ohlc("101", "101", "101", "101")
        assert reading(filled.ask) == ohlc("103.5", "103.5", "103.5", "103.5")

    def test_candles_from_delta_books_equal_candles_from_complete_books(self) -> None:
        # A candle reads a book's top of book, which every book states whether
        # it is complete or a delta book: the candles of a walk's delta
        # books - the first following no book - are the candles of those
        # books rebuilt complete.
        operations = [
            quote(OFFSET_DAY + SECOND, "ACME", "B-1", "BUY", "100", 10),
            quote(OFFSET_DAY + SECOND, "ACME", "A-1", "SELL", "102", 5),
            quote(OFFSET_DAY + 20 * SECOND, "ACME", "B-2", "BUY", "101", 4),
            quote(OFFSET_DAY + 70 * SECOND, "ACME", "A-1", "SELL", "103", 5),
            quote(OFFSET_DAY + 90 * SECOND, "ACME", "B-1", "BUY", "99", 1),
            quote(OFFSET_DAY + 130 * SECOND, "ACME", "A-2", "SELL", "101.5", 2),
        ]
        delta_books = books(operations)
        assert not any(book.is_complete for book in delta_books)
        whole: list[graph.BookEvent] = []
        for book in delta_books:
            previous = whole[-1] if whole else graph.BookEvent.keyed(book.transunix, "ACME")
            rebuilt = book.with_previous(previous)
            assert rebuilt is not None
            whole.append(rebuilt)
        assert all(book.is_complete for book in whole)
        folded = list(graph.CandleIterator(delta_books, MINUTE))
        assert len(folded) == 3
        assert folded == list(graph.CandleIterator(whole, MINUTE))

    def test_a_reading_a_later_book_lacks_keeps_the_earlier_ones(self) -> None:
        # The ask side empties at the second book: the ask, the mid and the
        # spread keep what the first book read, the touch quantities are the
        # last book's.
        cancel = graph.QuoteEvent(
            20 * SECOND,
            crosscode="A",
            ticker="ACME",
            instcode="ACME",
            side="SELL",
            price=D("102"),
            quantity=0,
            state="CANCELED",
        )
        operations = [
            quote(10 * SECOND, "ACME", "B", "BUY", "100", 5),
            quote(10 * SECOND, "ACME", "A", "SELL", "102", 7),
            cancel,
        ]
        folded = books(operations)
        assert len(folded) == 2 and folded[1].best_price("SELL") is None
        (candle,) = list(graph.CandleIterator(folded, "1m"))
        assert reading(candle.ask) == ohlc("102", "102", "102", "102")
        assert reading(candle.mid) == ohlc("101", "101", "101", "101")
        assert reading(candle.spread) == ohlc("2", "2", "2", "2")
        assert reading(candle.bid) == ohlc("100", "100", "100", "100")
        assert candle.askqty is None
        assert candle.bidqty is not None and candle.bidqty.as_py() == D(5)
        assert candle.books == 2

    def test_buckets_close_when_the_stream_moves_past_them(self) -> None:
        found = candles(
            [
                quote(10 * SECOND, "ACME", "B", "BUY", "100", 5),
                quote(59 * SECOND, "ACME", "B", "BUY", "101", 5),
                quote(60 * SECOND, "ACME", "B", "BUY", "102", 5),
                quote(200 * SECOND, "ACME", "B", "BUY", "103", 5),
            ],
            MINUTE,
        )
        assert [(candle.start, candle.end, candle.books) for candle in found] == [
            (0, MINUTE, 2),
            (MINUTE, 2 * MINUTE, 1),
            (3 * MINUTE, 4 * MINUTE, 1),
        ], "an empty bucket yields no candle"
        assert found[0].bid is not None and found[0].bid["close"].as_py() == D(101)
        assert found[1].bid is not None and found[1].bid["open"].as_py() == D(102)

    def test_two_cross_codes_interleave_and_emit_in_cross_code_order(self) -> None:
        found = candles(
            [
                quote(10 * SECOND, "IBM", "IBM-B", "BUY", "100", 5),
                quote(20 * SECOND, "AAPL", "AAPL-B", "BUY", "200", 5),
                quote(30 * SECOND, "IBM", "IBM-B", "BUY", "101", 5),
                quote(70 * SECOND, "IBM", "IBM-B", "BUY", "102", 5),
                quote(80 * SECOND, "AAPL", "AAPL-B", "BUY", "201", 5),
            ],
            "1m",
        )
        assert [(candle.crosscode, candle.start, candle.books) for candle in found] == [
            ("3:0:AAPL", 0, 1),
            ("3:0:IBM", 0, 2),
            ("3:0:AAPL", MINUTE, 1),
            ("3:0:IBM", MINUTE, 1),
        ]
        assert reading(found[1].bid) == ohlc("100", "101", "100", "101")
        assert found[1].ticker == "IBM"

    def test_a_book_stating_no_ticker_states_none_on_its_candle(self) -> None:
        # A keyed book states no ticker until an input does.
        assert empty_candles([10 * SECOND], MINUTE)[0].ticker is None
        (candle,) = list(graph.CandleIterator([graph.BookEvent(10 * SECOND, "US0378331005")], MINUTE))
        # A book keyed by a code is the candle's code.
        assert candle.ticker is None and candle.crosscode == "3:0:US0378331005"

    def test_market_data_holding_a_book_folds_and_anything_else_is_refused(self) -> None:
        held = [graph.MarketData(book) for book in books(ONE_MINUTE)]
        assert list(graph.CandleIterator(held, "1m")) == candles(ONE_MINUTE, "1m")
        with pytest.raises(TypeError, match=r"\$\.kind: expected book_event, got order_event"):
            list(graph.CandleIterator([graph.OrderEvent(SECOND, crosscode="O-1")], "1m"))
        with pytest.raises(TypeError):
            list(graph.CandleIterator([5], "1m"))
        with pytest.raises(TypeError):
            graph.CandleIterator(5, "1m")  # type: ignore[arg-type]
        with pytest.raises(ValueError, match=r"\$\.interval"):
            graph.CandleIterator([], "1x")
        with pytest.raises(TypeError):
            graph.CandleIterator([], 1.5)  # type: ignore[arg-type]

    def test_an_unsorted_stream_is_refused_at_the_book_and_the_walk_fuses(self) -> None:
        walk = graph.CandleIterator(
            [
                graph.BookEvent(2_000, "ACME"),
                graph.BookEvent(2_000, "ACME"),
                graph.BookEvent(1_000, "ACME"),
                graph.BookEvent(3_000, "ACME"),
            ],
            MINUTE,
        )
        with pytest.raises(
            ValueError,
            match=r"^invalid record value at \$\.book\.transunix: "
            r"expected an instant at or after 2000, got 1000$",
        ):
            next(walk)
        assert next(walk, None) is None, "the walk fuses"
        assert next(walk, None) is None

    def test_a_python_failure_follows_the_completed_buckets_and_is_raised_as_itself(
        self,
    ) -> None:
        def items() -> Iterator[Any]:
            yield graph.BookEvent(10 * SECOND, "ACME")
            yield graph.BookEvent(70 * SECOND, "ACME")
            raise RuntimeError("the source gave up")

        walk = graph.CandleIterator(items(), "1m")
        first = next(walk)
        assert (first.start, first.end, first.books) == (0, MINUTE, 1)
        with pytest.raises(RuntimeError, match="the source gave up"):
            next(walk)
        assert next(walk, None) is None, "the open bucket is dropped, not emitted"

    def test_an_empty_stream_yields_no_candle(self) -> None:
        walk = graph.CandleIterator([], "1m")
        assert list(walk) == []
        assert next(walk, None) is None
        assert walk.options.interval == MINUTE

    def test_candles_is_the_walk_drained(self) -> None:
        folded = books(ONE_MINUTE)
        assert graph.candles(folded, "1m") == list(graph.CandleIterator(folded, "1m"))
        assert graph.candles(folded, MINUTE) == graph.candles(folded, graph.CandleOptions("1m"))
        # `00:00:10Z` reads `05:30:10` in Kolkata, whose hour opens at `23:30Z`.
        zoned = graph.candles(folded, "1h", "Asia/Kolkata")
        assert [(candle.start, candle.end) for candle in zoned] == [(-30 * MINUTE, 30 * MINUTE)]
        assert graph.candles([], "1m") == []
        with pytest.raises(TypeError):
            graph.candles(folded, 1.5)  # type: ignore[arg-type]


CONCURRENT_PULLS_SCRIPT = r"""
import threading
import time

from yggdryl import graph

MINUTE = 60_000_000_000
INSTANTS = [minute * MINUTE + 1 for minute in range(6)]


def pausing():
    # Each pull sleeps - releasing the GIL - while the puller holds the walk.
    for unix in INSTANTS:
        time.sleep(0.05)
        yield graph.BookEvent(unix, "ACME")


expected = graph.candles([graph.BookEvent(unix, "ACME") for unix in INSTANTS], "1m")
walk = graph.CandleIterator(pausing(), "1m")
pulled = [[], []]
read = []


def pull(into):
    for candle in walk:
        into.append(candle)


def options():
    time.sleep(0.02)
    for _ in range(5):
        read.append(walk.options.interval)
        time.sleep(0.03)


threads = [threading.Thread(target=pull, args=(into,)) for into in pulled]
threads.append(threading.Thread(target=options))
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()
got = sorted(pulled[0] + pulled[1], key=lambda candle: candle.start)
assert got == expected, (got, expected)
assert read == [MINUTE] * 5, read
print("ok")
"""


def test_two_threads_pulling_one_walk_never_hold_the_gil_waiting() -> None:
    """A walk pulls the caller's iterable under its lock, and the iterable may
    release the GIL: a thread waiting on that lock attached - another
    ``next()``, or ``options`` - would keep the puller from taking the GIL
    back, and the process would hang. Two threads drain one walk over a
    generator that sleeps on every item while a third reads its options;
    every candle arrives once. In a process of its own under a deadline,
    because the failure is a hang.
    """
    try:
        result = subprocess.run(
            [sys.executable, "-c", CONCURRENT_PULLS_SCRIPT],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
    except subprocess.TimeoutExpired as hung:
        raise AssertionError("two threads pulling one candle walk hung") from hung
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.strip().endswith("ok")


class TestZones:
    def test_buckets_align_to_a_zone_with_a_half_hour_offset(self) -> None:
        # Asia/Kolkata is +05:30: `10:45Z` reads `16:15`, whose hour opens at
        # `16:00` local, `10:30Z`.
        instants = [OFFSET_DAY + 45 * MINUTE, OFFSET_DAY + 89 * MINUTE]
        found = empty_candles(instants, graph.CandleOptions("1h", "Asia/Kolkata"))
        assert [(candle.start, candle.end, candle.books) for candle in found] == [
            (OFFSET_DAY + 30 * MINUTE, OFFSET_DAY + 90 * MINUTE, 2)
        ]
        # The same instants in UTC open on the UTC hour.
        utc = empty_candles(instants, "1h")
        assert [(candle.start, candle.end, candle.books) for candle in utc] == [
            (OFFSET_DAY, OFFSET_DAY + HOUR, 1),
            (OFFSET_DAY + HOUR, OFFSET_DAY + 2 * HOUR, 1),
        ]

    def test_hourly_candles_skip_the_hour_a_spring_forward_removes(self) -> None:
        # Europe/Zurich, 2026-03-29: `00:30Z` reads `01:30 CET`, `01:30Z`
        # reads `03:30 CEST` - the wall clock never reads `02:xx`.
        found = empty_candles(
            [SPRING_DAY + 30 * MINUTE, SPRING_DAY + 90 * MINUTE, SPRING_DAY + 150 * MINUTE],
            graph.CandleOptions("1h", "Europe/Zurich"),
        )
        assert [(candle.start, candle.end) for candle in found] == [
            (SPRING_DAY, SPRING_DAY + HOUR),
            (SPRING_DAY + HOUR, SPRING_DAY + 2 * HOUR),
            (SPRING_DAY + 2 * HOUR, SPRING_DAY + 3 * HOUR),
        ], "the buckets abut in UTC"
        assert [local_hour("Europe/Zurich", candle.start) for candle in found] == [1, 3, 4], (
            "no candle opens at the hour the zone skipped"
        )

    def test_a_daily_candle_spans_twenty_three_hours_on_a_spring_forward_day(self) -> None:
        (candle,) = empty_candles(
            [SPRING_DAY + 30 * MINUTE, SPRING_DAY + 20 * HOUR],
            graph.CandleOptions("1d", "Europe/Zurich"),
        )
        # Local midnight is `23:00Z` the day before; the next is `22:00Z`.
        assert (candle.start, candle.end) == (SPRING_DAY - HOUR, SPRING_DAY + 22 * HOUR)
        assert candle.end - candle.start == 23 * HOUR
        assert local_hour("Europe/Zurich", candle.start) == 0
        assert local_hour("Europe/Zurich", candle.end) == 0
        assert candle.books == 2

    def test_a_fall_back_folds_the_repeated_hour_into_one_rising_bucket(self) -> None:
        # Europe/Zurich, 2026-10-25: `00:30Z` reads `02:30 CEST` and `01:30Z`
        # reads `02:30 CET`; both are the local hour `02`, one two-hour bucket.
        hourly = empty_candles(
            [FALL_DAY + 30 * MINUTE, FALL_DAY + 90 * MINUTE],
            graph.CandleOptions("1h", "Europe/Zurich"),
        )
        assert [(candle.start, candle.end, candle.books) for candle in hourly] == [
            (FALL_DAY, FALL_DAY + 2 * HOUR, 2)
        ]
        # Half-hour buckets: `02:00` opens once, at `00:00Z`, and `02:30` holds
        # every instant from `00:30Z` until the wall clock first reads `03:00`,
        # at `02:00Z`, so the edges rise and no bucket is re-entered.
        halves = empty_candles(
            [
                FALL_DAY + 15 * MINUTE,
                FALL_DAY + 45 * MINUTE,
                FALL_DAY + 75 * MINUTE,
                FALL_DAY + 125 * MINUTE,
            ],
            graph.CandleOptions("30m", "Europe/Zurich"),
        )
        assert [(candle.start, candle.end, candle.books) for candle in halves] == [
            (FALL_DAY, FALL_DAY + 30 * MINUTE, 1),
            (FALL_DAY + 30 * MINUTE, FALL_DAY + 2 * HOUR, 2),
            (FALL_DAY + 2 * HOUR, FALL_DAY + 150 * MINUTE, 1),
        ]


class TestCandle:
    def test_the_field_declares_every_cell(self) -> None:
        field = graph.Candle.field()
        assert field.name == "candle" and not field.nullable
        declared = [
            f"{child.name}: {child.dtype}{'' if child.nullable else ' not null'}"
            for child in field.dtype
        ]
        assert declared == [
            "crosscode: utf8 not null",
            "ticker: utf8",
            'start: datetime64(ns,"UTC") not null',
            'end: datetime64(ns,"UTC") not null',
            *(f"{reading}{cell}: decimal" for reading in READINGS for cell in CELLS),
            "bidqty: decimal",
            "askqty: decimal",
            "books: uint64 not null",
        ]
        assert [child.name for child in field.dtype] == NAMES

    def test_the_scalar_round_trips_as_the_named_struct_and_the_native_dict(self) -> None:
        (candle,) = candles(ONE_MINUTE, "1m")
        for value in (candle, full_candle(), empty_candle()):
            named = value.into_scalar()
            assert named.kind == "struct"
            assert graph.Candle.from_scalar(named) == value
            assert graph.Candle.from_scalar(value.as_py()) == value
            assert graph.Candle.from_scalar(types.MappingProxyType(value.as_py())) == value
        native = candle.as_py()
        assert sorted(native) == sorted(NAMES)
        assert (native["crosscode"], native["ticker"], native["books"]) == ("3:0:ACME", "ACME", 4)
        assert (native["bidopen"], native["spreadclose"]) == (D(100), D("2.5"))
        assert native["start"] == datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
        assert native["end"] == datetime.datetime(1970, 1, 1, 0, 1, tzinfo=datetime.timezone.utc)
        assert empty_candle().as_py()["ticker"] is None
        assert empty_candle().as_py()["bidqty"] is None

    def test_a_named_struct_restates_what_the_door_restates(self) -> None:
        # A text price and a bare count land as the column would hold them;
        # a name left out is a null.
        read = graph.Candle.from_scalar(
            {
                "crosscode": "ACME",
                "start": 0,
                "end": MINUTE,
                "bidopen": "100.5",
                "bidhigh": "100.5",
                "bidlow": "100.5",
                "bidclose": "100.5",
                "books": 1,
            }
        )
        assert reading(read.bid) == ohlc("100.5", "100.5", "100.5", "100.5")
        assert read.ticker is None and read.ask is None and read.bidqty is None
        assert (read.start, read.end, read.books) == (0, MINUTE, 1)

    def test_a_scalar_of_another_shape_is_refused_under_the_candle(self) -> None:
        for wrong in (1, None, [1, 2]):
            with pytest.raises(ValueError, match=r"\$\.candle"):
                graph.Candle.from_scalar(wrong)  # type: ignore[arg-type]
        # A required cell absent is a null the door refuses.
        absent = full_candle().as_py()
        del absent["crosscode"]
        with pytest.raises(ValueError, match="crosscode"):
            graph.Candle.from_scalar(absent)
        # A reading stating some of its four cells is refused by this reading.
        partial = full_candle().as_py()
        partial["bidhigh"] = None
        with pytest.raises(ValueError, match=r"\$\.candle.*four decimals or four nulls"):
            graph.Candle.from_scalar(partial)
        # A name the struct should not hold.
        extra = full_candle().as_py()
        extra["vwap"] = None
        with pytest.raises(ValueError, match="vwap"):
            graph.Candle.from_scalar(extra)
        # A candle is built by a walk or read back, never constructed bare.
        with pytest.raises(TypeError):
            graph.Candle()  # type: ignore[call-arg]

    def test_equality_hash_repr_copy_pickle(self) -> None:
        candle = full_candle()
        twin = full_candle()
        assert candle == twin and hash(candle) == hash(twin) and len({candle, twin}) == 1
        assert candle != empty_candle()
        assert repr(candle) == f'Candle("ACME", start={MINUTE}, end={2 * MINUTE}, books=4)'
        assert copy.copy(candle) == candle and copy.deepcopy(candle) == candle
        assert pickle.loads(pickle.dumps(candle)) == candle
        assert pickle.loads(pickle.dumps(empty_candle())) == empty_candle()
        (walked,) = candles(ONE_MINUTE, "1m")
        assert pickle.loads(pickle.dumps(walked)) == walked
