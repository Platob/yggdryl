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

from yggdryl import DataType, Field, Scalar, Serie, WindowSerie


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
    keys = Serie.from_scalars(Field("group", "utf8", nullable=False), ["a", "b", "a", "b"])
    groups = window.partition_by(keys)
    assert len(groups) == 2
    assert groups[0].key.as_py() == ["a"]
    assert groups[0].rows.child("price").as_py() == [3, 3]
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


# Owned key items produced by a live borrowed window.

def quote_rows(rows: list[tuple[str | None, int] | None]) -> Serie:
    root = Field("quote", "struct<venue: utf8, price: int64 not null>", nullable=True)
    return Serie.from_scalars(root, [None if row is None else list(row) for row in rows])


def test_a_live_window_clusters_its_current_rows_with_absolute_positions() -> None:
    rows = quote_rows([("a", 1), ("a", 2), ("b", 3), ("a", 4)])
    live = rows.window(1, 3)
    groups = live.window_by("venue as desk")
    assert [(item.key.as_py(), item.rownum, len(item.rows)) for item in groups] == [(["a"], 1, 1), (["b"], 2, 1), (["a"], 3, 1)]
    assert [field.name for field in groups.field] == ["desk", "price"]
    assert str(groups[0].key_paths[0]) == "venue"
    rows.set(1, ["c", 20])
    assert groups[0].rows.child("price").as_py() == [2]
    assert live.window_by("venue")[0].key.as_py() == ["c"]


def test_owned_keys_outlive_the_original_and_mutate_a_snapshot() -> None:
    rows = quote_rows([("a", 1), ("a", 2), ("b", 3)])
    groups = rows.window(0, 3).window_by("venue")
    payload = groups[0].rows
    payload.set(0, [10])
    assert rows.child("price").as_py() == [1, 2, 3]
    assert groups[0].rows.child("price").as_py() == [1, 2]
    del rows
    assert payload.child("price").as_py() == [10, 2]


def test_sorted_live_window_keys_are_stable_and_have_no_source_position() -> None:
    rows = quote_rows([("b", 1), ("a", 2), ("b", 3), None, ("a", 5)])
    groups = rows.window(0, len(rows)).window_by("venue", True)
    assert [item.key.as_py() for item in groups] == [["a"], ["b"], [None]]
    assert all(item.rownum is None for item in groups)
    assert groups[0].rows.child("price").as_py() == [2, 5]
    ordered = quote_rows([("a", 1), ("b", 2)]).window_by("venue", True)
    assert [item.rownum for item in ordered] == [0, 1]


def test_borrowed_windows_have_no_synthetic_static_values() -> None:
    rows = quote_rows([("a", 1), ("b", 2)])
    live = rows.window(0, 2)
    assert not hasattr(live, "static_values")
    assert live.window(0, 1).as_py() == rows.slice(0, 1).as_py()
    assert [len(item.rows) for item in live.partition_by("venue")] == [1, 1]


def test_composed_keys_keep_outer_cells_and_source_positions() -> None:
    field = Field("rows", "struct<venue:utf8 not null, day:int64 not null, price:int64 not null>", nullable=False)
    rows = Serie.from_scalars(field, [["a", 1, 10], ["b", 1, 11], ["b", 1, 12], ["a", 2, 13], ["a", 2, 14], ["c", 2, 15]])
    days = rows.window_by("day")
    seen = []
    for day in days:
        for item in day.window_by("venue"):
            assert [field.name for field in item.key_field] == ["day", "venue"]
            seen.append((item.key.as_py(), item.rownum, len(item.rows)))
    assert seen == [([1, "a"], 0, 1), ([1, "b"], 1, 2), ([2, "a"], 3, 2), ([2, "c"], 5, 1)]
    deeper = days[1].window_by("venue")[0].window_by("price")
    assert [(item.key.as_py(), item.rownum) for item in deeper] == [([2, "a", 13], 3), ([2, "a", 14], 4)]
    streamed = rows.into_stream().window_by("day").window_by("venue")
    actual = []
    for item in streamed:
        actual.append((item.key.as_py(), item.rownum, len(item.into_stream().collect())))
    assert actual == seen


def test_composed_gathers_drop_positions_and_allow_ordinary_key_names() -> None:
    rows = quote_rows([("b", 1), ("a", 2), ("b", 3)])
    outer = rows.window_by("price > 0 as group")
    inner = outer[0].window_by("venue as windownum", True)
    assert [field.name for field in inner.key_field] == ["group", "windownum"]
    assert [item.key.as_py() for item in inner] == [[True, "a"], [True, "b"]]
    assert all(item.rownum is None for item in inner)
    with pytest.raises(ValueError, match="alias the key cell"):
        rows.window_by("price + 1 as venue")
