"""The ordering, uniqueness, grouping and window doors of ``Serie``,
``ChunkedSerie``, ``WindowSerie`` and ``SerieReader``, each beside the PyArrow
compute kernel that answers the same ask where one exists.

What a row measures is the boundary: reading the keyword options, the
indices, mask or keys argument, the call off the GIL, and handing the answer
back as its leaf class. The kernels are the core's, so the gap to PyArrow is
the crossing, not the sort. The ``as_*`` writes run on a fresh clone each
time - a clone shares the buffers, so the write copies them once, which is
the shared-buffer path.

Run after ``maturin develop`` with::

    python benchmarks/types/serie.py --iterations 1000
"""

from __future__ import annotations

import argparse
import copy
import gc
import statistics
import timeit
from collections.abc import Callable

import pyarrow as pa
import pyarrow.compute as pc

from yggdryl import ChunkedSerie, Field, Serie, SerieReader, SerieReaderWindows

ROWS = 4_096
VALUES = pa.array([(index * 7_919) % 1_024 for index in range(ROWS)], pa.int64())
VENUES = pa.array([f"X{index % 16:03}" for index in range(ROWS)], pa.utf8())
MASK = pa.array([index % 3 != 0 for index in range(ROWS)], pa.bool_())
INDICES = pa.array(list(range(ROWS - 1, -1, -2)), pa.uint32())
SORTED_KEYS = pa.array(sorted(f"X{index % 16:03}" for index in range(ROWS)), pa.utf8())
CHUNKED_VALUES = pa.chunked_array([VALUES[: ROWS // 2], VALUES[ROWS // 2 :]])
CHUNKED_VENUES = pa.chunked_array([VENUES[: ROWS // 2], VENUES[ROWS // 2 :]])

PRICES = Serie.from_arrow_array(VALUES, Field("price", "int64", nullable=False))
HELD_VENUES = Serie.from_arrow_array(VENUES, Field("venue", "utf8", nullable=False))
HELD_MASK = Serie.from_arrow_array(MASK)
HELD_INDICES = Serie.from_arrow_array(INDICES)
HELD_SORTED_KEYS = Serie.from_arrow_array(SORTED_KEYS)
QUOTES = Serie.from_(pa.record_batch({"venue": VENUES, "price": VALUES}))
CHUNKED_PRICES = ChunkedSerie.from_(CHUNKED_VALUES, Field("price", "int64", nullable=False))
HELD_CHUNKED_VENUES = ChunkedSerie.from_(CHUNKED_VENUES)
WINDOW_OFFSET = ROWS // 4
WINDOW_ROWS = ROWS // 2
WINDOW_MASK = Serie.from_arrow_array(MASK[:WINDOW_ROWS])
WINDOW_INDICES = Serie.from_arrow_array(
    pa.array(list(range(WINDOW_ROWS - 1, -1, -1)), pa.uint32())
)
WINDOW_KEYS = Serie.from_arrow_array(VENUES[:WINDOW_ROWS])
WINDOW_SOURCE = Serie.from_arrow_array(
    VALUES[:WINDOW_ROWS], Field("price", "int64", nullable=False)
)
WINDOW_ROWS_PY = list(range(16))
# Quotes a minute apart, their venues in sorted runs: the keys `window_by`
# cuts by a period, by a column in order, and - over QUOTES - out of order.
TICKS = pa.array([index * 60_000_000_000 for index in range(ROWS)], pa.timestamp("ns", "UTC"))
TICKED_BATCH = pa.record_batch({"venue": SORTED_KEYS, "ts": TICKS})
TICKED = Serie.from_(TICKED_BATCH)
CHUNKED_TICKED = ChunkedSerie.from_(
    pa.Table.from_batches([TICKED_BATCH.slice(0, ROWS // 2), TICKED_BATCH.slice(ROWS // 2)])
)
LENT = TICKED.window_by("venue")[1][1]
# The last of the 274 windows a quarter hour cuts: its record skips every
# window before it.
LENT_LAST = TICKED.window_by("minutes(ts, 15)")[-1][1]


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    nanoseconds = statistics.median(samples) * 1_000_000_000 / iterations
    print(f"{name:52} {nanoseconds:14.1f} ns/op")


def _drained(windows: SerieReaderWindows) -> int:
    """Every row of every window of a stream, each window read in turn."""
    return sum(len(piece) for window in windows for piece in window)


def _fresh() -> Serie:
    """A clone of the prices sharing their buffers: the next write copies."""
    return copy.copy(PRICES)


def _fresh_chunked() -> ChunkedSerie:
    return copy.copy(CHUNKED_PRICES)


def _cases() -> list[tuple[str, Callable[[], object]]]:
    window = PRICES.window(WINDOW_OFFSET, WINDOW_ROWS)
    held = _fresh()
    writable = held.window(WINDOW_OFFSET, WINDOW_ROWS)
    return [
        # Serie: the reads.
        ("Serie.sort_indices (yggdryl)", PRICES.sort_indices),
        ("Serie.sort_indices (pyarrow)", lambda: pc.sort_indices(VALUES)),
        ("Serie.sort_indices descending", lambda: PRICES.sort_indices(descending=True)),
        ("Serie.is_sorted", PRICES.is_sorted),
        ("Serie.is_unique", PRICES.is_unique),
        ("Serie.unique_count (yggdryl)", PRICES.unique_count),
        ("Serie.unique_count (pyarrow)", lambda: pc.count_distinct(VALUES)),
        ("Serie.into_sorted (yggdryl)", PRICES.into_sorted),
        ("Serie.into_sorted (pyarrow)", lambda: pc.take(VALUES, pc.sort_indices(VALUES))),
        ("Serie.into_unique (yggdryl)", PRICES.into_unique),
        ("Serie.into_unique (pyarrow)", lambda: pc.unique(VALUES)),
        ("Serie.into_reversed (yggdryl)", PRICES.into_reversed),
        ("Serie.into_reversed (pyarrow)", lambda: VALUES[::-1]),
        ("Serie.into_taken, Serie (yggdryl)", lambda: PRICES.into_taken(HELD_INDICES)),
        ("Serie.into_taken, pyarrow array", lambda: PRICES.into_taken(INDICES)),
        ("Serie.into_taken (pyarrow)", lambda: pc.take(VALUES, INDICES)),
        ("Serie.into_filtered, Serie (yggdryl)", lambda: PRICES.into_filtered(HELD_MASK)),
        ("Serie.into_filtered (pyarrow)", lambda: pc.filter(VALUES, MASK)),
        ("Serie.partition_by, 16 keys", lambda: PRICES.partition_by(HELD_VENUES)),
        ("Serie.partition_by, sorted keys", lambda: PRICES.partition_by(HELD_SORTED_KEYS)),
        ("Serie.partition_by_paths, one path", lambda: QUOTES.partition_by_paths("venue")),
        ("Serie.memory_size (yggdryl)", PRICES.memory_size),
        ("Serie.memory_size (pyarrow nbytes)", lambda: VALUES.nbytes),
        # Serie: the writes, each on a clone sharing the buffers.
        ("Serie.as_sorted, shared clone", lambda: _fresh().as_sorted()),
        ("Serie.as_unique, shared clone", lambda: _fresh().as_unique()),
        ("Serie.as_reversed, shared clone", lambda: _fresh().as_reversed()),
        ("Serie.as_taken, shared clone", lambda: _fresh().as_taken(HELD_INDICES)),
        ("Serie.as_filtered, shared clone", lambda: _fresh().as_filtered(HELD_MASK)),
        ("Serie.as_reversed, held alone", held.as_reversed),
        # ChunkedSerie.
        ("ChunkedSerie.sort_indices", CHUNKED_PRICES.sort_indices),
        ("ChunkedSerie.is_sorted", CHUNKED_PRICES.is_sorted),
        ("ChunkedSerie.is_unique", CHUNKED_PRICES.is_unique),
        ("ChunkedSerie.unique_count (yggdryl)", CHUNKED_PRICES.unique_count),
        ("ChunkedSerie.unique_count (pyarrow)", lambda: pc.count_distinct(CHUNKED_VALUES)),
        ("ChunkedSerie.into_sorted", CHUNKED_PRICES.into_sorted),
        ("ChunkedSerie.into_unique (yggdryl)", CHUNKED_PRICES.into_unique),
        ("ChunkedSerie.into_unique (pyarrow)", lambda: pc.unique(CHUNKED_VALUES)),
        ("ChunkedSerie.into_reversed", CHUNKED_PRICES.into_reversed),
        ("ChunkedSerie.into_taken", lambda: CHUNKED_PRICES.into_taken(HELD_INDICES)),
        ("ChunkedSerie.into_filtered (yggdryl)", lambda: CHUNKED_PRICES.into_filtered(HELD_MASK)),
        ("ChunkedSerie.into_filtered (pyarrow)", lambda: pc.filter(CHUNKED_VALUES, MASK)),
        ("ChunkedSerie.partition_by, Serie keys", lambda: CHUNKED_PRICES.partition_by(HELD_VENUES)),
        (
            "ChunkedSerie.partition_by, keys cut alike",
            lambda: CHUNKED_PRICES.partition_by(HELD_CHUNKED_VENUES),
        ),
        ("ChunkedSerie.memory_size", CHUNKED_PRICES.memory_size),
        ("ChunkedSerie.as_sorted, shared clone", lambda: _fresh_chunked().as_sorted()),
        ("ChunkedSerie.as_unique, shared clone", lambda: _fresh_chunked().as_unique()),
        ("ChunkedSerie.as_reversed, shared clone", lambda: _fresh_chunked().as_reversed()),
        (
            "ChunkedSerie.as_taken, shared clone",
            lambda: _fresh_chunked().as_taken(HELD_INDICES),
        ),
        (
            "ChunkedSerie.as_filtered, shared clone",
            lambda: _fresh_chunked().as_filtered(HELD_MASK),
        ),
        # WindowSerie: taking a window, its reads, its writes.
        ("Serie.window", lambda: PRICES.window(WINDOW_OFFSET, WINDOW_ROWS)),
        ("WindowSerie.scalar", lambda: window.scalar(7)),
        ("WindowSerie.null_count", window.null_count),
        ("WindowSerie.rows", window.rows),
        ("WindowSerie.memory_size", window.memory_size),
        ("WindowSerie.window", lambda: window.window(1, 8)),
        ("WindowSerie.into_serie", window.into_serie),
        ("WindowSerie.is_sorted", window.is_sorted),
        ("WindowSerie.is_unique", window.is_unique),
        ("WindowSerie.unique_count", window.unique_count),
        ("WindowSerie.sort_indices", window.sort_indices),
        ("WindowSerie.into_sorted", window.into_sorted),
        ("WindowSerie.into_unique", window.into_unique),
        ("WindowSerie.into_reversed", window.into_reversed),
        ("WindowSerie.into_taken", lambda: window.into_taken(WINDOW_INDICES)),
        ("WindowSerie.into_filtered", lambda: window.into_filtered(WINDOW_MASK)),
        ("WindowSerie.partition_by", lambda: window.partition_by(WINDOW_KEYS)),
        ("WindowSerie.set, held alone", lambda: writable.set(3, 42)),
        ("WindowSerie.swap, held alone", lambda: writable.swap(3, 4)),
        ("WindowSerie.fill, held alone", lambda: writable.window(0, 16).fill(7)),
        ("WindowSerie.splice, 16 rows", lambda: writable.splice(0, 16, WINDOW_ROWS_PY)),
        ("WindowSerie.copy_from, a window", lambda: writable.copy_from(WINDOW_SOURCE)),
        ("WindowSerie.as_sorted, held alone", writable.as_sorted),
        ("WindowSerie.as_reversed, held alone", writable.as_reversed),
        ("WindowSerie.as_taken, held alone", lambda: writable.as_taken(WINDOW_INDICES)),
        # window_by: held windows, their records, chunks and a stream.
        ("Serie.window_by, minutes(ts, 15)", lambda: TICKED.window_by("minutes(ts, 15)")),
        ("Serie.window_by, 16 keys in order", lambda: TICKED.window_by("venue")),
        ("Serie.window_by sorted, 16 keys gathered", lambda: QUOTES.window_by("venue", True)),
        (
            "WindowSerie.window_by",
            lambda: TICKED.window(WINDOW_OFFSET, WINDOW_ROWS).window_by("venue"),
        ),
        ("WindowSerie.window_by, a lent window", lambda: LENT.window_by("minutes(ts, 15)")),
        ("WindowSerie.static_values", lambda: LENT.static_values),
        ("WindowSerie.static_values, the last of 274", lambda: LENT_LAST.static_values),
        ("ChunkedSerie.window_by", lambda: CHUNKED_TICKED.window_by("venue")),
        ("ChunkedSerie.window_by sorted", lambda: CHUNKED_TICKED.window_by("venue", True)),
        (
            "SerieReader.window_by, drained",
            lambda: _drained(SerieReader.from_serie(TICKED).window_by("venue")),
        ),
        (
            "SerieReader.window_by sorted, drained",
            lambda: _drained(SerieReader.from_serie(TICKED).window_by("venue", True)),
        ),
    ]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=1_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    # Every pair answers the same rows before either is measured.
    assert PRICES.into_sorted().into_arrow_array().equals(
        pc.take(VALUES, pc.sort_indices(VALUES))
    )
    assert PRICES.into_filtered(HELD_MASK).into_arrow_array().equals(pc.filter(VALUES, MASK))
    assert PRICES.unique_count() == pc.count_distinct(VALUES).as_py()
    assert len(TICKED.window_by("venue")) == len(CHUNKED_TICKED.window_by("venue")) == 16
    assert _drained(SerieReader.from_serie(TICKED).window_by("venue", True)) == ROWS
    last = LENT_LAST.static_values
    assert last is not None and last["windownum"].as_py() == 273
    gc.disable()
    try:
        for name, operation in _cases():
            _measure(name, operation, args.iterations)
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
