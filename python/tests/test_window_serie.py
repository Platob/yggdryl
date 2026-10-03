"""``python/src/window_serie.rs``: a window over a serie, read and written
through it, every index window-relative.

Mirrors ``rust/tests/root/window_serie.rs`` case for case. One Python class is
both the core's shared and its mutable window: it holds the serie object, an
offset and a length - and, where ``window_by`` lent it, the windows of that
call and its place among them - and borrows the serie when each call is
made.
"""

from __future__ import annotations

import copy

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, Serie, SerieReader, WindowSerie


def column(values: list[int | None]) -> Serie:
    """Prices as an int64 column holding its buffer alone."""
    nullable = any(value is None for value in values)
    return Serie.from_arrow_array(
        pa.array(values, pa.int64()), Field("price", "int64", nullable=nullable)
    )


def test_a_window_reads_through_the_serie_window_relative_on_both_leaves() -> None:
    prices = column([1, 2, None, 4, 5, 6])
    run = Serie([1, 2, None, 4, 5, 6])
    for serie in (prices, run):
        window = serie.window(1, 3)
        assert isinstance(window, WindowSerie)
        assert (len(window), window.offset, window.is_empty()) == (3, 1, False)
        # The window holds the serie object itself, never a copy.
        assert window.serie is serie
        assert window.field == serie.field
        assert window.scalar(0).as_py() == 2
        assert window.is_null(1)
        row = window.get(2)
        assert row is not None and row.as_py() == 4
        assert window.get(3) is None
        assert window.null_count() == 1
        assert [row.as_py() for row in window.rows()] == [2, None, 4]
        assert [row.as_py() for row in window] == [2, None, 4]
        assert window[-1].as_py() == 4
        assert window.as_py() == [2, None, 4]
        assert window.dtype == DataType("serie<item: int64>")
        assert 0 < window.memory_size() <= serie.memory_size()
        assert 4 in window and 1 not in window

        # A narrower window rebases onto the serie.
        narrower = window.window(2, 1)
        assert (narrower.offset, len(narrower)) == (3, 1)
        assert narrower.scalar(0).as_py() == 4
        assert narrower.serie is serie
        assert window[1:].offset == 2
        assert window[1:].as_py() == [None, 4]

        # The window as a serie is `slice`.
        assert window.into_serie() == serie.slice(1, 3)
        assert len(serie.window(6, 0)) == 0


def test_every_edge_is_refused_naming_the_serie_and_both_counts() -> None:
    prices = column([1, 2, 3])
    with pytest.raises(ValueError, match=r"rows 2\.\.4 reach past the 3 rows price holds"):
        prices.window(2, 2)
    with pytest.raises(ValueError, match=r"reach past the 1 rows \$ holds"):
        Serie([1]).window(0, 2)
    window = prices.window(1, 2)
    with pytest.raises(ValueError, match="row 2 is past the 2 rows price holds"):
        window.scalar(2)
    with pytest.raises(ValueError):
        window.is_null(2)
    with pytest.raises(ValueError, match=r"rows 1\.\.3 reach past the 2 rows price holds"):
        window.window(1, 2)
    with pytest.raises(ValueError):
        window.set(2, 0)
    with pytest.raises(ValueError):
        window.swap(0, 2)
    with pytest.raises(
        ValueError, match="never grows or shrinks what it views: 0 rows cannot replace 1"
    ):
        window.splice(0, 1, [])
    with pytest.raises(ValueError, match=r"rows 0\.\.3 reach past the 2 rows price holds"):
        window.splice(0, 3, [1, 2, 3])
    with pytest.raises(ValueError, match="1 indices cannot rearrange 2 rows"):
        window.as_taken([0])
    # Nothing moved.
    assert prices.as_py() == [1, 2, 3]

    # Python's own spellings: an item past the window, a stepped slice.
    with pytest.raises(IndexError):
        window[2]
    assert window[-2].as_py() == 2
    with pytest.raises(ValueError, match="step of 1"):
        window[::2]

    # A window reads its serie as it is when asked: one taken before the
    # serie shrank is refused naming the serie and both counts.
    shrinking = column([1, 2, 3])
    stale = shrinking.window(1, 2)
    assert shrinking.pop() is not None
    with pytest.raises(ValueError, match=r"rows 1\.\.3 reach past the 2 rows price holds"):
        stale.scalar(0)


def test_identity_is_the_window_rows_alone_like_a_serie() -> None:
    prices = column([9, 1, 2, 9])
    run = Serie([1, 2])
    window = prices.window(1, 2)
    other = run.window(0, 2)
    assert window == other
    assert window == run
    assert run == window
    assert window != prices.window(2, 2)
    assert window < prices.window(2, 2)
    assert repr(window) == f"{prices!r}.window(1, 2)"
    # The serie under a window is mutable, so the window is unhashable.
    with pytest.raises(TypeError):
        hash(window)


def test_the_reads_of_a_window_answer_what_the_sliced_serie_answers() -> None:
    prices = column([9, 3, 1, 3, None, 0])
    window = prices.window(1, 4)
    assert not window.is_sorted()
    assert not window.is_unique()
    assert window.unique_count() == 3
    assert window.sort_indices().as_py() == [1, 0, 2, 3]
    assert window.into_sorted().as_py() == [1, 3, 3, None]
    assert window.into_unique().as_py() == [3, 1, None]
    assert window.into_reversed().as_py() == [None, 3, 1, 3]
    assert window.into_taken([1]).as_py() == [1]
    mask = [True, False, False, True]
    assert window.into_filtered(mask).as_py() == [3, None]
    keys = ["a", "b", "a", "b"]
    groups = window.partition_by(keys)
    assert len(groups) == 2
    assert groups[0][0].as_py() == "a"
    assert groups[0][1].as_py() == [3, 3]
    # Each answer is the leaf class the column is.
    assert type(window.into_sorted()) is type(prices)
    assert window.into_sorted().field == prices.field
    assert window.into_sorted(descending=True, nulls_first=True).as_py() == [None, 3, 3, 1]
    assert window.is_sorted() is False
    # The window wrote nothing.
    assert prices.as_py() == [9, 3, 1, 3, None, 0]


def test_every_write_goes_through_the_serie_on_the_rebased_range_and_never_past_the_window() -> (
    None
):
    prices = column([1, 2, 3, 4, 5])
    window = prices.window(1, 3)
    window.set(0, 20)
    window.swap(0, 2)
    assert window.as_py() == [4, 3, 20]
    window.splice(1, 3, [30, 40])
    assert prices.as_py() == [1, 4, 30, 40, 5]

    window.fill(7)
    assert prices.as_py() == [1, 7, 7, 7, 5]
    # A value the field refuses refuses the fill and leaves the column.
    with pytest.raises(ValueError):
        window.fill(None)
    with pytest.raises((TypeError, ValueError)):
        window.set(0, "x")
    assert prices.as_py() == [1, 7, 7, 7, 5]

    source = Serie([8, 9, 10])
    window.copy_from(source.window(0, 3))
    with pytest.raises(ValueError, match="2 rows cannot replace 3"):
        window.copy_from(source.window(0, 2))
    assert prices.as_py() == [1, 8, 9, 10, 5]
    # A `Serie` is its whole window, and a window over the same serie copies
    # the rows as they stood.
    window.copy_from(Serie([11, 12, 13]))
    assert prices.as_py() == [1, 11, 12, 13, 5]
    window.copy_from(prices.window(2, 3))
    assert prices.as_py() == [1, 12, 13, 5, 5]
    window[-1] = 6
    assert prices.as_py() == [1, 12, 13, 6, 5]

    # A run is written the same way.
    run = Serie([1, 2, 3, 4, 5])
    window = run.window(1, 3)
    window.set(1, "mixed")
    window.swap(0, 2)
    assert run.as_py() == [1, 4, "mixed", 2, 5]

    # A write made through the serie is what the window reads next.
    prices.set(1, 100)
    assert prices.window(1, 1).as_py() == [100]


def test_the_window_writes_sort_reverse_and_rearrange_in_place_within_the_window() -> None:
    # A primitive column holding its buffer alone sorts the window of its
    # native slice where it stands, absences gathered within the window.
    prices = column([9, None, 3, 1, 2, 0])
    window = prices.window(1, 4)
    assert window.as_sorted().as_reversed() is window
    assert prices.as_py() == [9, None, 3, 2, 1, 0]
    prices.window(1, 4).as_sorted(nulls_first=True)
    assert prices.as_py() == [9, None, 1, 2, 3, 0]
    prices.window(2, 3).as_taken([2, 0, 1])
    assert prices.as_py() == [9, None, 3, 1, 2, 0]

    # A string column writes the sorted rows back through splice.
    venues = Serie.from_arrow_array(
        pa.array(["z", "c", "a", "b", "y"]), Field("venue", "utf8", nullable=False)
    )
    venues.window(1, 3).as_sorted().as_reversed()
    assert venues.as_py() == ["z", "c", "b", "a", "y"]

    # A run sorts its values in place.
    run = Serie([9, 3, 1, 2, 0])
    run.window(1, 3).as_sorted(descending=True)
    assert run.as_py() == [9, 3, 2, 1, 0]
    run.window(0, 5).as_reversed()
    assert run.as_py() == [0, 1, 2, 3, 9]


def test_a_window_over_a_shared_column_copies_the_column_once_and_leaves_the_clone() -> None:
    prices = column([2, 1, 3])
    other = copy.copy(prices)
    other.window(0, 2).as_sorted()
    assert other.as_py() == [1, 2, 3]
    assert prices.as_py() == [2, 1, 3]
    # A column over pyarrow's buffers never writes them: the write copies.
    array = pa.array([2, 1, 3], pa.int64())
    shared = Serie.from_arrow_array(array, Field("price", "int64", nullable=False))
    shared.window(0, 2).as_sorted()
    assert shared.as_py() == [1, 2, 3]
    assert array.to_pylist() == [2, 1, 3]


def test_the_order_by_keys_sort_the_window_rows_alone_window_relative() -> None:
    prices = column([9, 1, 3, 2])
    window = prices.window(1, 3)
    order = window.sort_indices_by("price desc")
    assert order.field == Field("index", "uint32", nullable=False)
    assert order.as_py() == [1, 2, 0]
    sorted_ = window.into_sort_by("price")
    assert sorted_.as_py() == [1, 2, 3]
    assert sorted_.field == prices.field
    assert prices.as_py() == [9, 1, 3, 2]
    # In place: every row outside the window untouched.
    assert window.as_sort_by("price desc") is window
    assert prices.as_py() == [9, 3, 2, 1]
    # A refusal leaves the serie as it was.
    with pytest.raises(ValueError, match="tier"):
        window.as_sort_by("tier")
    with pytest.raises(ValueError, match="tier"):
        window.into_sort_by("tier")
    assert prices.as_py() == [9, 3, 2, 1]
    # A record window keys by its terms.
    quotes = Serie.from_scalars(
        Field("quote", "struct<venue: utf8 not null, price: int64 not null>", nullable=False),
        [["XNYS", 1], ["XNAS", 2], ["XNYS", 3], ["XNAS", 0]],
    )
    quotes.window(0, 3).as_sort_by(["venue", "price desc"])
    assert [row["price"] for row in quotes.as_py()] == [2, 3, 1, 0]


def test_a_window_reads_where_the_rows_of_the_serie_it_views_lie() -> None:
    prices = Serie.from_scalars(Field("price", "int64", nullable=False), list(range(1_024)))
    window = prices.window(10, 100)
    assert not window.is_spilled()
    assert window.resident_size() == window.memory_size() > 0
    prices.spill(byte_size=0)
    # The window reads the serie it holds as it is now.
    assert window.is_spilled()
    assert window.resident_size() == 0
    assert window.as_py() == list(range(10, 110))
    # A run's window is never spilled, its rows all resident.
    run = Serie([1, 2, 3]).window(1, 2)
    assert not run.is_spilled()
    assert run.resident_size() == run.memory_size()


# ---------------------------------------------------------------------------
# window_by: the windows of a window, and the record a window states
# ---------------------------------------------------------------------------


def quote_rows(rows: list[tuple[str | None, int] | None]) -> Serie:
    """Quotes `{venue, price}` whose root is nullable, so a row may be absent."""
    root = Field("quote", "struct<venue: utf8, price: int64 not null>", nullable=True)
    return Serie.from_scalars(
        root, [None if row is None else [row[0], row[1]] for row in rows]
    )


def cuts(windows: list[tuple[Scalar, WindowSerie]]) -> list[tuple[object, int, int]]:
    return [(key.as_py(), window.offset, len(window)) for key, window in windows]


def test_a_window_window_by_lent_states_its_key_windownum_and_rownum() -> None:
    quotes = quote_rows([("XNAS", 1), ("XNAS", 2), ("XNYS", 3), ("XNAS", 4)])
    windows = quotes.window_by("venue")
    assert [window.static_values.as_py() for _, window in windows if window.static_values] == [
        {"venue": "XNAS", "windownum": 0, "rownum": 0},
        {"venue": "XNYS", "windownum": 1, "rownum": 2},
        {"venue": "XNAS", "windownum": 2, "rownum": 3},
    ]
    # Two terms are two key cells, named as their projections are.
    (_, first), *_ = quotes.window_by("venue, price > 1 as late")
    record = first.static_values
    assert record is not None
    assert record.as_py() == {"venue": "XNAS", "late": False, "windownum": 0, "rownum": 0}


def test_a_gathered_window_states_a_null_rownum() -> None:
    quotes = quote_rows([("XNYS", 1), ("XNAS", 2), ("XNYS", 3), None, ("XNAS", 5)])
    windows = quotes.window_by("venue", True)
    # Gathered once in stable key order, an absent row's key last.
    assert cuts(windows) == [(["XNAS"], 0, 2), (["XNYS"], 2, 2), (None, 4, 1)]
    gathered = windows[0][1].serie
    assert gathered is not quotes
    assert all(window.serie is gathered for _, window in windows)
    assert gathered == quotes.into_taken([1, 4, 0, 2, 3])
    assert [window.static_values.as_py() for _, window in windows if window.static_values] == [
        {"venue": "XNAS", "windownum": 0, "rownum": None},
        {"venue": "XNYS", "windownum": 1, "rownum": None},
        {"venue": None, "windownum": 2, "rownum": None},
    ]
    # Over keys already in order nothing is gathered, and every rownum stands.
    ordered = quote_rows([("XNAS", 1), ("XNYS", 2)]).window_by("venue", True)
    assert [window.static_values.as_py()["rownum"] for _, window in ordered if window.static_values] == [0, 1]


def test_a_plain_or_narrowed_window_states_none() -> None:
    quotes = quote_rows([("XNAS", 1), ("XNAS", 2), ("XNYS", 3)])
    assert quotes.window(0, 2).static_values is None
    (_, xnas), _ = quotes.window_by("venue")
    assert xnas.static_values is not None
    assert xnas.window(0, 1).static_values is None
    assert xnas[1:].static_values is None
    # The record is never the window's identity, and stays with the window.
    assert xnas == quotes.window(0, 2)
    assert xnas.into_serie() == quotes.slice(0, 2)


def test_the_record_is_read_through_the_generic_scalar_accessors() -> None:
    quotes = quote_rows([("XNAS", 1), ("XNYS", 2)])
    _, (_, xnys) = quotes.window_by("venue")
    record = xnys.static_values
    assert isinstance(record, Scalar)
    assert record["venue"].as_py() == "XNYS"
    windownum = record.get("windownum")
    assert windownum is not None and windownum.as_py() == 1
    rownum = record.path("rownum")
    assert rownum is not None and rownum.as_py() == 1
    assert sorted(key.as_py() for key in record.keys()) == ["rownum", "venue", "windownum"]
    assert record.get("price") is None
    # Read again, it is the same value.
    assert xnys.static_values == record


def test_a_window_windows_by_its_own_rows_over_the_serie() -> None:
    quotes = quote_rows([("XNYS", 0), ("XNAS", 1), ("XNAS", 2), ("XNYS", 3), ("XNAS", 4)])
    tail = quotes.window(1, 4)
    windows = tail.window_by("venue")
    # Offsets are the serie's, every window over the serie object itself.
    assert cuts(windows) == [(["XNAS"], 1, 2), (["XNYS"], 3, 1), (["XNAS"], 4, 1)]
    assert all(window.serie is quotes for _, window in windows)
    # A plain window states no record of its own, so its windows number
    # their rows from its first.
    assert [window.static_values.as_py()["rownum"] for _, window in windows if window.static_values] == [0, 2, 3]
    # Sorted over keys out of order, the gather takes this window's rows alone.
    gathered = tail.window_by("venue", sorted=True)
    assert cuts(gathered) == [(["XNAS"], 0, 3), (["XNYS"], 3, 1)]
    assert gathered[0][1].serie == quotes.into_taken([1, 2, 4, 3])


def day_quotes(rows: list[tuple[str, int, int]]) -> Serie:
    """Quotes `{venue, day, price}` under a required root, as the core's pins."""
    root = Field(
        "quote",
        "struct<venue: utf8 not null, day: int32 not null, price: int64 not null>",
        nullable=False,
    )
    return Serie.from_scalars(root, [list(row) for row in rows])


def records(windows: list[tuple[Scalar, WindowSerie]]) -> list[object]:
    """Every window's record as the Python value it is, in window order."""
    return [None if window.static_values is None else window.static_values.as_py() for _, window in windows]


def test_a_lent_window_refuses_a_key_cell_named_as_a_cell_its_record_keeps() -> None:
    quotes = day_quotes([("XNAS", 1, 10), ("XNYS", 1, 11)])
    (_, xnas), _ = quotes.window_by("venue")
    for sorted_ in (False, True):
        # A text that does not parse is refused first, as everywhere.
        with pytest.raises(ValueError, match="expected a value or a name"):
            xnas.window_by("venue,", sorted_)
        # Its record keeps its venue: keyed by the venue again, the inner
        # record would name it twice.
        with pytest.raises(
            ValueError, match=r'at quote: the key cell "VENUE" collides with the static value "venue"; alias'
        ):
            xnas.window_by("VENUE", sorted_)
        assert records(xnas.window_by("venue as desk", sorted_)) == [
            {"venue": "XNAS", "desk": "XNAS", "windownum": 0, "rownum": 0}
        ]
        # A window of the rows alone keeps nothing to collide with, and a
        # narrower window of no row cuts none.
        assert len(quotes.window(0, 1).window_by("venue", sorted_)) == 1
        assert xnas[0:0].window_by("price", sorted_) == []


def test_a_window_of_a_lent_window_keeps_the_outer_cells_and_an_absolute_rownum() -> None:
    quotes = day_quotes(
        [("XNAS", 1, 10), ("XNYS", 1, 11), ("XNYS", 1, 12), ("XNAS", 2, 13), ("XNAS", 2, 14), ("XLON", 2, 15)]
    )
    days = quotes.window_by("day")
    seen = []
    for _, day in days:
        venues = day.window_by("venue")
        # Over the serie object itself, at its offsets.
        assert all(window.serie is quotes for _, window in venues)
        for _, venue in venues:
            record = venue.static_values
            assert record is not None
            # The rownum is the window's offset in the serie windowed first.
            assert record["rownum"].as_py() == venue.offset
            seen.append(record.as_py())
    assert seen == [
        {"day": 1, "venue": "XNAS", "windownum": 0, "rownum": 0},
        {"day": 1, "venue": "XNYS", "windownum": 1, "rownum": 1},
        {"day": 2, "venue": "XNAS", "windownum": 0, "rownum": 3},
        {"day": 2, "venue": "XLON", "windownum": 1, "rownum": 5},
    ]
    # Three levels keep both outer cells, the rownum absolute throughout.
    (_, xnas), _ = days[1][1].window_by("venue")
    assert records(xnas.window_by("price")) == [
        {"day": 2, "venue": "XNAS", "price": 13, "windownum": 0, "rownum": 3},
        {"day": 2, "venue": "XNAS", "price": 14, "windownum": 1, "rownum": 4},
    ]

    # A stream window of a stream window states the same record over the
    # same rows: the outer key cell first, then its own, its place and the
    # absolute rownum.
    for sorted_ in (False, True):
        streamed_days = SerieReader.from_serie(quotes).window_by("day", sorted_)
        for (_, day), stream_day in zip(quotes.window_by("day", sorted_), streamed_days, strict=True):
            inner = stream_day.window_by("venue")
            assert [child.name for child in inner.static_field] == ["day", "venue", "windownum", "rownum"]
            streamed = [window.static_values for window in inner]
            held = [window.static_values for _, window in day.window_by("venue")]
            assert streamed == held, sorted_
            assert all(record is not None for record in held)


def test_a_gathered_window_of_a_lent_window_keeps_the_outer_cells_and_has_no_rownum() -> None:
    quotes = day_quotes([("XNYS", 1, 10), ("XNAS", 1, 11), ("XNYS", 1, 12), ("XNAS", 2, 13)])
    (_, first), _ = quotes.window_by("day")
    # Sorted over keys out of order, the gather takes this window's rows
    # alone into one new serie every window shares.
    gathered = first.window_by("venue", sorted=True)
    assert cuts(gathered) == [(["XNAS"], 0, 1), (["XNYS"], 1, 2)]
    assert gathered[0][1].serie is not quotes
    assert all(window.serie is gathered[0][1].serie for _, window in gathered)
    assert gathered[0][1].serie == quotes.into_taken([1, 0, 2])
    assert records(gathered) == [
        {"day": 1, "venue": "XNAS", "windownum": 0, "rownum": None},
        {"day": 1, "venue": "XNYS", "windownum": 1, "rownum": None},
    ]
    # A window of a gathered window views the gathered serie and keeps its
    # cells; with no number to add to, it has none either.
    (_, xnys) = gathered[1]
    prices = xnys.window_by("price")
    assert all(window.serie is xnys.serie for _, window in prices)
    assert cuts(prices) == [([10], 1, 1), ([12], 2, 1)]
    assert records(prices) == [
        {"day": 1, "venue": "XNYS", "price": 10, "windownum": 0, "rownum": None},
        {"day": 1, "venue": "XNYS", "price": 12, "windownum": 1, "rownum": None},
    ]


def test_lent_windows_outlive_the_serie_object_they_were_cut_from() -> None:
    quotes = day_quotes([("XNAS", 1, 10), ("XNYS", 1, 11), ("XNAS", 2, 12)])
    days = quotes.window_by("day")
    gathered = quotes.window_by("venue", sorted=True)
    del quotes
    (_, first), (_, second) = days
    # Each window holds the serie object, and its windows what they view.
    assert second.as_py() == [{"venue": "XNAS", "day": 2, "price": 12}]
    assert second.static_values is not None
    assert second.static_values.as_py() == {"day": 2, "windownum": 1, "rownum": 2}
    venues = first.window_by("venue")
    del days, first, second
    assert [window.as_py() for _, window in venues] == [
        [{"venue": "XNAS", "day": 1, "price": 10}],
        [{"venue": "XNYS", "day": 1, "price": 11}],
    ]
    assert records(venues) == [
        {"day": 1, "venue": "XNAS", "windownum": 0, "rownum": 0},
        {"day": 1, "venue": "XNYS", "windownum": 1, "rownum": 1},
    ]
    (_, xnas), _ = gathered
    del gathered
    assert len(xnas) == 2
    assert xnas.static_values is not None
    assert xnas.static_values.as_py() == {"venue": "XNAS", "windownum": 0, "rownum": None}


def test_a_lent_window_states_its_rows_as_window_by_cut_them() -> None:
    quotes = day_quotes([("XNAS", 1, 10), ("XNYS", 2, 11)])
    (_, xnas), _ = quotes.window_by("venue")
    # A write through the window reaches the serie; the record is what
    # window_by read when it cut the window.
    xnas.set(0, ["XLON", 1, 10])
    assert quotes.as_py()[0] == {"venue": "XLON", "day": 1, "price": 10}
    assert xnas.static_values is not None
    assert xnas.static_values.as_py() == {"venue": "XNAS", "windownum": 0, "rownum": 0}
    # A window taken before its serie shrank is refused, as every call is.
    quotes.truncate(0)
    with pytest.raises(ValueError, match=r"rows 0\.\.1 reach past the 0 rows quote holds"):
        xnas.window_by("price")
