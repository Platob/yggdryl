"""``Serie``: many values, as a schema-free run or as a column's Arrow buffers.

Pins ``python/yggdryl/serie.py`` and the ``python/src/serie.rs`` redirects
under it: every verb answers what the core ``Serie`` answers, and Arrow
crosses in and out by sharing buffers. It is also the one boundary a foreign
columnar object crosses: a held column is a ``Serie``, a stream is a
``SerieReader``, and nothing else.
"""

from __future__ import annotations

import contextlib
import copy
import gc
import pathlib
import pickle
import sys
import threading
import time
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor

import numpy as np
import pyarrow as pa
import pytest

from yggdryl import (
    ArrowCastPlan,
    ChunkedSerie,
    DataType,
    Field,
    FieldPath,
    IOBase,
    FixedSizeSerieSerie,
    LargeSerieSerie,
    LargeSerieViewSerie,
    MapSerie,
    Scalar,
    Selector,
    Serie,
    SerieReader,
    SerieReaderWindows,
    SerieSerie,
    SerieViewSerie,
    SpillOptions,
    StructSerie,
    WindowSerie,
)


def price() -> Field:
    return Field("price", "int64", nullable=False)


def quotes() -> pa.RecordBatch:
    return pa.record_batch(
        [pa.array([1, 2], pa.int64()), pa.array(["AAPL", "MSFT"])],
        schema=pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("symbol", pa.utf8(), nullable=False),
            ]
        ),
    )


class TestConstruction:
    def test_values_alone_are_a_run_and_a_field_makes_them_a_column(self) -> None:
        run = Serie([1, "a", None])
        assert not run.is_column
        assert run.field is None
        assert len(run) == 3
        assert run.null_count() == 1

        column = Serie.from_scalars(price(), [125, 126])
        assert type(column) is Serie
        assert column.is_column
        assert column.field == price()
        assert column.dtype == DataType("serie<item: int64 not null>")
        assert column.as_py() == [125, 126]

    def test_each_constructor_names_the_column_it_builds(self) -> None:
        assert Serie.from_scalars(price(), [1, 2]) == Serie([1, 2])
        empty = Serie.empty(price())
        assert empty.is_column and empty.is_empty()
        assert Serie.with_capacity(price(), 64).is_empty()

    def test_a_value_the_field_refuses_is_refused_naming_the_column(self) -> None:
        with pytest.raises(ValueError, match="price"):
            Serie.from_scalars(price(), [1, None])
        column = Serie.from_scalars(price(), [1])
        with pytest.raises(ValueError, match="price"):
            column.push(None)
        assert column.as_py() == [1]


class TestArrow:
    def test_an_array_crosses_in_and_back_out(self) -> None:
        array = pa.array([125, 126, 127], pa.int64())
        column = Serie.from_arrow_array(array, price())
        assert column.as_py() == [125, 126, 127]
        assert column.into_arrow_array().equals(array)

        inferred = Serie.from_arrow_array(pa.array(["a", None]))
        assert inferred.field == Field("item", "utf8", nullable=True)
        assert inferred.as_py() == ["a", None]

    def test_an_array_of_another_layout_is_cast_into_the_field(self) -> None:
        column = Serie.from_arrow_array(pa.array([1, 2], pa.int32()), price())
        assert column.field == price()
        assert column.into_arrow_array().type == pa.int64()
        assert column.as_py() == [1, 2]

    def test_an_exact_layout_shares_its_buffers(self) -> None:
        array = pa.array([125, 126], pa.int64())
        column = Serie.from_arrow_array(array, price())
        assert column.into_arrow_array().buffers()[1].address == array.buffers()[1].address

    def test_an_absent_row_in_a_required_column_is_refused(self) -> None:
        absent = pa.array([1, None], pa.int64())
        with pytest.raises(ValueError, match="price"):
            Serie.from_arrow_array(absent, price())

    def test_a_refused_value_is_null_when_safe_and_raised_otherwise(self) -> None:
        wide = pa.array([1, 300], pa.int64())
        narrow = Field("price", "int8")
        assert Serie.from_arrow_array(wide, narrow).as_py() == [1, None]
        with pytest.raises(ValueError):
            Serie.from_arrow_array(wide, narrow, safe=False)

    def test_a_batch_and_a_reader_are_a_record_column_named_row(self) -> None:
        records = Serie.from_arrow_batch(quotes())
        assert records.field is not None
        assert records.field.name == "row"
        assert records.as_py() == [
            {"id": 1, "symbol": "AAPL"},
            {"id": 2, "symbol": "MSFT"},
        ]
        assert records.into_arrow_batch().equals(quotes())
        assert records.into_arrow_reader().read_all().equals(pa.Table.from_batches([quotes()]))

        reader = pa.RecordBatchReader.from_batches(quotes().schema, [quotes(), quotes()])
        drained = Serie.from_arrow_reader(reader)
        assert len(drained) == 4

    def test_a_batch_and_a_reader_cast_into_a_root(self) -> None:
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        records = Serie.from_arrow_batch(quotes(), root)
        assert records.field == root
        assert records.into_arrow_batch().schema.field("id").type == pa.int32()

        reader = pa.RecordBatchReader.from_batches(quotes().schema, [quotes(), quotes()])
        drained = Serie.from_arrow_reader(reader, root)
        assert drained.field == root
        assert drained.child("id").as_py() == [1, 2, 1, 2]

        strict = Field("row", "struct<id:int64,venue:utf8 not null>", nullable=False)
        with pytest.raises(ValueError, match="venue"):
            Serie.from_arrow_batch(quotes(), strict)

    def test_from_default_repeats_the_fields_canonical_default(self) -> None:
        assert Serie.from_default(price()).as_py() == [0]
        three = Serie.from_default(price(), 3)
        assert three.field == price()
        assert three.as_py() == [0, 0, 0]
        assert Serie.from_default(Field("symbol", "utf8", nullable=False), 2).as_py() == ["", ""]

    def test_cast_restates_a_column_under_another_field(self) -> None:
        column = Serie.from_scalars(Field("id", "int32"), [1, None])
        wide = column.cast(Field("id", "int64"))
        assert wide.field == Field("id", "int64")
        assert wide.as_py() == [1, None]

        # A datatype is the required column named `value` it declares, so an
        # absent row is refused by path rather than filled with the
        # datatype's canonical default.
        with pytest.raises(ValueError, match="value"):
            column.cast(DataType("int64"))

        # A column already under the target is itself.
        assert column.cast(Field("id", "int32")) == column

    def test_a_run_has_no_layout_to_cast(self) -> None:
        with pytest.raises(ValueError, match="run"):
            Serie([1, 2]).cast(Field("id", "int64"))

    def test_one_row_is_an_arrow_scalar(self) -> None:
        assert Serie.from_scalars(price(), [125]).into_arrow_scalar() == pa.scalar(125, pa.int64())
        with pytest.raises(ValueError, match="exactly one row"):
            Serie.from_scalars(price(), [1, 2]).into_arrow_scalar()

    def test_a_run_has_no_buffers(self) -> None:
        with pytest.raises(ValueError):
            Serie([1, 2]).into_arrow_array()


class TestSerieReader:
    @staticmethod
    def stream() -> pa.RecordBatchReader:
        return pa.RecordBatchReader.from_batches(quotes().schema, [quotes(), quotes()])

    def test_one_serie_per_batch(self) -> None:
        series = SerieReader.from_arrow_reader(self.stream())
        assert series.field.name == "row"
        pulled = list(series)
        assert len(pulled) == 2
        assert all(type(serie) is StructSerie and len(serie) == 2 for serie in pulled)
        assert pulled[0].as_py() == [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": "MSFT"}]
        assert list(series) == []

    def test_a_root_casts_every_batch_by_one_plan(self) -> None:
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        series = SerieReader.from_arrow_reader(self.stream(), root)
        assert series.field == root
        for serie in series:
            assert serie.field == root
            assert serie.child("id").into_arrow_array().type == pa.int32()

    def test_a_plan_the_stream_cannot_meet_is_refused_before_a_batch(self) -> None:
        strict = Field("row", "struct<id:int64,venue:utf8 not null>", nullable=False)
        with pytest.raises(ValueError, match="venue"):
            SerieReader.from_arrow_reader(self.stream(), strict)

    def test_into_arrow_reader_hands_over_the_batches_not_yet_pulled(self) -> None:
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        series = SerieReader.from_arrow_reader(self.stream(), root)
        next(series)
        rest = series.into_arrow_reader()
        assert isinstance(rest, pa.RecordBatchReader)
        assert rest.schema.field("id").type == pa.int32()
        assert rest.read_all().num_rows == 2
        assert list(series) == []
        with pytest.raises(ValueError, match="into_arrow_reader"):
            series.into_arrow_reader()

    def test_building_the_reader_pulls_nothing_and_each_pull_costs_one_batch(self) -> None:
        stored = pa.schema([pa.field("id", pa.int32()), pa.field("symbol", pa.string())])
        pulled: list[int] = []

        def batches() -> Iterator[pa.RecordBatch]:
            for index in range(3):
                pulled.append(index)
                yield pa.record_batch(
                    {"id": pa.array([index], pa.int32()), "symbol": pa.array(["AAPL"])},
                    schema=stored,
                )

        root = Field("row", "struct<id:int64,symbol:utf8>", nullable=False)
        series = SerieReader.from_arrow_reader(
            pa.RecordBatchReader.from_batches(stored, batches()), root
        )
        assert pulled == []
        assert len(next(series)) == 1
        assert pulled == [0]

        # The rest crosses as a reader whose schema is the root's, still lazy.
        rest = series.into_arrow_reader()
        assert rest.schema.field("id").type == pa.int64()
        assert pulled == [0]
        assert rest.read_all().num_rows == 2
        assert pulled == [0, 1, 2]

    def test_every_table_frame_and_sequence_of_them_streams_in(self) -> None:
        pandas = pytest.importorskip("pandas")
        polars = pytest.importorskip("polars")

        root = Field("row", "struct<id:int64,symbol:utf8>", nullable=False)
        table = pa.table({"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", "MSFT"]})
        sources = [
            table,
            table.to_reader(),
            table.to_batches()[0],
            pandas.DataFrame({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
            polars.DataFrame({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
        ]
        for source in sources:
            drained = Serie.from_arrow_reader(source, root)
            assert drained.field == root
            assert drained.as_py() == [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": "MSFT"}]

        # A sequence of tables is one stream, each item read into the first
        # item's shape even when its columns are ordered differently.
        swapped = table.select(["symbol", "id"])
        chained = SerieReader.from_arrow_reader(iter([table, swapped]), root)
        assert [serie.child("symbol").as_py() for serie in chained] == [
            ["AAPL", "MSFT"],
            ["AAPL", "MSFT"],
        ]

    def test_a_polars_lazy_frame_streams_its_batches(self) -> None:
        pl = pytest.importorskip("polars")

        frame = pl.LazyFrame({"id": [1, 2, 3], "symbol": ["AAPL", "MSFT", "AMD"]})
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        series = SerieReader.from_arrow_reader(frame.collect_batches(chunk_size=1), root)
        pulled = [serie.as_py() for serie in series]
        assert [len(rows) for rows in pulled] == [1, 1, 1]
        assert [rows[0]["id"] for rows in pulled] == [1, 2, 3]


class TestProtocols:
    def test_a_serie_reads_as_a_python_sequence(self) -> None:
        column = Serie.from_scalars(price(), [1, 2, 3])
        assert column[0] == Scalar.from_(1)
        assert column[-1] == Scalar.from_(3)
        assert column[1:] == Serie([2, 3])
        assert list(column) == [Scalar.from_(1), Scalar.from_(2), Scalar.from_(3)]
        assert 2 in column and 9 not in column
        assert column.get(3) is None
        with pytest.raises(IndexError):
            column[3]
        with pytest.raises(ValueError):
            column[::2]
        with pytest.raises(TypeError):
            column[True]

    def test_item_assignment_and_deletion_write_through_the_field(self) -> None:
        column = Serie.from_scalars(price(), [1, 2, 3])
        column[-1] = 30
        del column[0]
        assert column.as_py() == [2, 30]
        with pytest.raises(ValueError, match="price"):
            column[0] = None

    def test_identity_is_the_rows_and_a_serie_is_unhashable(self) -> None:
        assert Serie.from_scalars(price(), [1, 2]) == Serie([1, 2])
        assert Serie([1, 2]) < Serie([1, 3])
        with pytest.raises(TypeError):
            hash(Serie([1]))

    def test_copies_and_pickles_keep_the_field(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        assert copy.copy(column) == column
        clone = copy.deepcopy(column)
        clone.push(3)
        assert len(column) == 2
        restored = pickle.loads(pickle.dumps(column))
        assert restored == column
        assert restored.field == price()
        assert pickle.loads(pickle.dumps(Serie([1, "a"]))) == Serie([1, "a"])


def venue() -> Field:
    return Field("venue", "utf8", nullable=False)


class TestLit:
    """The constant column - mirrors ``rust/tests/serie/lit.rs`` through
    ``Serie.lit`` and ``is_lit``."""

    def test_a_lit_holds_one_value_once_and_reads_every_row_as_it(self) -> None:
        column = Serie.lit(venue(), "XNAS", 1_000_000)
        assert column.is_lit and column.is_column
        assert len(column) == 1_000_000
        assert column.field == venue()
        assert column.null_count() == 0
        assert column[0].as_py() == "XNAS"
        assert column[-1].as_py() == "XNAS"
        assert column.scalar(999_999) == Scalar.from_("XNAS")
        with pytest.raises(IndexError):
            column[1_000_000]
        # One row resident, not a million; the laid-out estimate counts them.
        assert column.resident_size() < 1_024
        assert column.memory_size() >= 1_000_000
        assert not column.is_spilled()
        # Identity is the rows alone.
        assert Serie.lit(venue(), "XNAS", 16) == Serie.from_scalars(venue(), ["XNAS"] * 16)
        assert not Serie.from_scalars(venue(), ["XNAS"]).is_lit

    def test_the_value_is_proven_by_the_field(self) -> None:
        with pytest.raises(ValueError):
            Serie.lit(venue(), None, 3)
        absent = Serie.lit(Field("venue", "utf8"), None, 3)
        assert absent.null_count() == 3 and absent.as_py() == [None, None, None]
        # A text field reads an integer as its text; a sequence it cannot read.
        assert Serie.lit(venue(), 1, 2).as_py() == ["1", "1"]
        with pytest.raises(ValueError):
            Serie.lit(venue(), [1], 3)
        # The value lands canonical under the field, and a `Scalar` crosses as
        # itself.
        narrow = DataType("int32").scalar(7)
        count = Serie.lit(Field("n", "int64", nullable=False), narrow, 2)
        assert count.scalar(1).dtype == DataType("int64")
        assert count.as_py() == [7, 7]
        assert len(Serie.lit(venue(), "XNAS", 0)) == 0
        assert Serie.from_default(price(), 3).is_lit

    def test_a_slice_stays_lit_and_the_export_is_the_laid_out_column(self) -> None:
        column = Serie.lit(venue(), "XNAS", 5)
        window = column[1:4]
        assert window.is_lit and len(window) == 3
        assert column.slice(1, 2).is_lit
        array = column.into_arrow_array()
        assert array.equals(pa.array(["XNAS"] * 5, pa.utf8()))
        assert column.into_arrow_array().equals(array)
        assert window.into_arrow_array().equals(pa.array(["XNAS"] * 3, pa.utf8()))
        assert column.is_lit
        assert column.as_py() == ["XNAS"] * 5

    def test_a_write_of_the_value_moves_the_count_and_another_lays_the_column_out(
        self,
    ) -> None:
        column = Serie.lit(venue(), "XNAS", 3)
        column.push("XNAS")
        assert column.is_lit and len(column) == 4
        column.remove(0)
        assert column.is_lit and len(column) == 3
        with pytest.raises(ValueError):
            column.set(0, [1])
        assert column.is_lit
        column.set(1, "XLON")
        assert not column.is_lit
        assert column.as_py() == ["XNAS", "XLON", "XNAS"]
        assert type(column) is Serie

    def test_a_spill_forgets_the_built_array_and_writes_nothing(self) -> None:
        column = Serie.lit(venue(), "XNAS", 1_000)
        column.into_arrow_array()
        built = column.resident_size()
        assert column.as_spilled(byte_size=0) is column
        assert column.is_lit and not column.is_spilled()
        assert column.resident_size() < built
        assert column[3].as_py() == "XNAS"


class TestWrites:
    def test_every_write_is_spelled_over_splice(self) -> None:
        column = Serie.empty(price())
        column.push(1)
        column.extend([2, 3, 4])
        column.insert(0, 0)
        column.set(4, 40)
        assert column.remove(1) == Scalar.from_(1)
        assert column.pop() == Scalar.from_(40)
        column.splice(0, 1, [7, 8])
        assert column.as_py() == [7, 8, 2, 3]
        column.truncate(3)
        column.resize(5, 9)
        assert column.as_py() == [7, 8, 2, 9, 9]
        column.extend_from_serie(Serie.from_scalars(price(), [10]))
        assert column.as_py() == [7, 8, 2, 9, 9, 10]
        column.clear()
        assert column.is_empty() and column.field == price()
        assert Serie.empty(price()).pop() is None

    def test_a_record_column_writes_its_children_in_place(self) -> None:
        records = Serie.from_arrow_batch(quotes())
        symbols = records.child("symbol")
        assert symbols is not None and symbols.as_py() == ["AAPL", "MSFT"]
        assert records.child_at(0) == Serie([1, 2])
        assert len(records.children()) == 2
        assert records.get_child_by_path("symbol") == symbols

        records.set_cell("symbol", 1, "IBM")
        assert records.as_py()[1] == {"id": 2, "symbol": "IBM"}
        records.set_child(Serie.from_scalars(Field("active", "bool", nullable=False), [True, False]))
        assert records.as_py()[0] == {"id": 1, "symbol": "AAPL", "active": True}
        with pytest.raises(ValueError):
            Serie([1]).set_cell("symbol", 0, "x")

    def test_a_write_keeps_the_leaf_class(self) -> None:
        legs = Serie.from_arrow_array(pa.array([[1, 2], [3]], pa.list_(pa.int64())))
        assert isinstance(legs, SerieSerie)
        legs.push([4, 5, 6])
        legs[0] = []
        assert legs.offsets == [0, 0, 1, 4]
        assert legs.as_py() == [[], [3], [4, 5, 6]]
        assert type(copy.copy(legs)) is SerieSerie
        assert type(pickle.loads(pickle.dumps(legs))) is SerieSerie


class TestScalar:
    def test_a_scalar_sequence_holds_a_serie_and_a_serie_is_a_scalar(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        value = column.into_scalar()
        assert value.kind == "serie"
        assert value.as_serie() == column
        assert Scalar.from_(column) == value
        assert Scalar.from_(1).as_serie() is None
        assert column.into_run() == column
        assert not column.into_run().is_column


class TestNested:
    def test_each_serie_layout_is_its_own_class_and_lends_its_cut(self) -> None:
        rows = [[1, 2], None, [3]]
        legs = Serie.from_arrow_array(pa.array(rows, pa.list_(pa.int64())))
        assert isinstance(legs, SerieSerie) and isinstance(legs, Serie)
        assert legs.offsets == [0, 2, 2, 3]
        assert legs.range(0) == (0, 2)
        assert legs.range(3) is None
        first = legs.row(0)
        assert first is not None and first.as_py() == [1, 2]
        items = legs.items()
        assert items is not None and items.as_py() == [1, 2, 3]
        assert legs.as_py() == rows

        large = Serie.from_arrow_array(pa.array(rows, pa.large_list(pa.int64())))
        assert isinstance(large, LargeSerieSerie)
        assert large.offsets == [0, 2, 2, 3]
        assert large == legs

        view = Serie.from_arrow_array(pa.array(rows, pa.list_view(pa.int64())))
        assert isinstance(view, SerieViewSerie)
        assert view.sizes == [2, 0, 1]
        assert view.row(2) == Serie([3])

        large_view = Serie.from_arrow_array(pa.array(rows, pa.large_list_view(pa.int64())))
        assert isinstance(large_view, LargeSerieViewSerie)
        assert large_view.as_py() == rows

        fixed = Serie.from_arrow_array(pa.array([[1, 2], [3, 4]], pa.list_(pa.int64(), 2)))
        assert isinstance(fixed, FixedSizeSerieSerie)
        assert fixed.width == 2
        assert fixed.range(1) == (2, 4)
        second = fixed.row(1)
        assert second is not None and second.as_py() == [3, 4]

    def test_a_map_lends_its_entries_keys_and_values(self) -> None:
        array = pa.array(
            [[("AAPL", 1), ("MSFT", 2)], [], [("IBM", 3)]],
            pa.map_(pa.utf8(), pa.int64()),
        )
        books = Serie.from_arrow_array(array)
        assert isinstance(books, MapSerie)
        assert isinstance(books.entries, StructSerie)
        assert books.keys.as_py() == ["AAPL", "MSFT", "IBM"]
        assert books.values.as_py() == [1, 2, 3]
        assert books.offsets == [0, 2, 2, 3]
        assert not books.keys_sorted
        assert books.range(2) == (2, 3)
        row = books.row(0)
        assert isinstance(row, StructSerie) and len(row) == 2

    def test_nesting_hands_every_level_out_as_its_own_class(self) -> None:
        legs = pa.list_(
            pa.struct([pa.field("venue", pa.utf8()), pa.field("sizes", pa.list_(pa.int64()))])
        )
        batch = pa.record_batch(
            [
                pa.array([1, 2], pa.int64()),
                pa.array(
                    [
                        [{"venue": "XNYS", "sizes": [100, 200]}],
                        [{"venue": "XLON", "sizes": []}, {"venue": "XPAR", "sizes": [5]}],
                    ],
                    legs,
                ),
            ],
            names=["id", "legs"],
        )
        orders = Serie.from_arrow_batch(batch)
        assert isinstance(orders, StructSerie)
        assert orders.names == ["id", "legs"]

        column = orders.child("legs")
        assert isinstance(column, SerieSerie)
        records = column.items()
        assert isinstance(records, StructSerie)
        sizes = records.child("sizes")
        assert isinstance(sizes, SerieSerie)
        assert sizes.as_py() == [[100, 200], [], [5]]
        assert isinstance(orders.get_child_by_path("legs.sizes"), SerieSerie)
        assert [type(child) for child in orders.children()] == [Serie, SerieSerie]

        second = column.row(1)
        assert isinstance(second, StructSerie)
        assert second.as_py() == [
            {"venue": "XLON", "sizes": []},
            {"venue": "XPAR", "sizes": [5]},
        ]

        slim = orders.without_child("legs")
        assert isinstance(slim, StructSerie) and slim.names == ["id"]
        assert isinstance(orders.scalar(0).as_serie(), Serie)
        assert isinstance(orders.into_scalar().as_serie(), StructSerie)


pandas = pytest.importorskip("pandas")
polars = pytest.importorskip("polars")


def quote_table() -> pa.Table:
    return pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})


def quote_root() -> Field:
    return Field(
        "row",
        "struct<symbol: utf8 not null, size: int64 not null>",
        nullable=False,
    )


def buffer_locations(array: pa.Array) -> list[tuple[int, int] | None]:
    return [
        None if buffer is None else (buffer.address, buffer.size)
        for buffer in array.buffers()
    ]


class TestFrom:
    def test_a_held_batch_is_the_record_column_of_its_rows(self) -> None:
        records = Serie.from_(quote_table().to_batches()[0])
        assert type(records) is StructSerie
        assert len(records) == 2
        assert records.names == ["symbol", "size"]

    def test_a_table_is_a_stream_and_is_drained(self) -> None:
        records = Serie.from_(quote_table())
        assert len(records) == 2
        assert records.names == ["symbol", "size"]

    def test_a_column_and_a_pinned_row_are_told_apart(self) -> None:
        assert len(Serie.from_(pa.array([1, 2, 3]))) == 3
        pinned = Serie.from_(pa.scalar(7, pa.int64()))
        assert len(pinned) == 1
        assert pinned.field == Field("value", "int64", nullable=False)
        # As a value, one Arrow scalar is its row rather than a serie of one.
        assert Scalar.from_(pa.scalar(7, pa.int64())).as_py() == 7

    def test_a_chunked_column_is_combined_rather_than_truncated(self) -> None:
        assert len(Serie.from_(pa.chunked_array([[1, 2], [3]]))) == 3

    def test_a_chunkless_chunked_array_is_the_empty_column_of_its_type(self) -> None:
        column = Serie.from_(pa.chunked_array([], pa.int64()))
        assert len(column) == 0
        assert column.field == Field("item", "int64", nullable=False)

    def test_a_reader_crosses_without_being_pulled(self) -> None:
        reader = SerieReader.from_(quote_table().to_reader())
        assert reader.field == Field.from_arrow_schema(quote_table().schema)
        assert reader.into_arrow_reader().read_all().num_rows == 2
        assert len(Serie.from_(quote_table().to_reader())) == 2

    def test_a_pandas_frame_is_converted_by_pandas(self) -> None:
        assert len(Serie.from_(quote_table().to_pandas())) == 2

    def test_a_polars_frame_and_its_lazy_form_both_name_rows(self) -> None:
        frame = polars.from_arrow(quote_table())
        for source in (frame, frame.lazy()):
            assert len(Serie.from_(source)) == 2

    def test_a_series_is_one_column_in_either_library(self) -> None:
        for series in (
            pandas.Series([1, 2, 3], name="size"),
            polars.Series("size", [1, 2, 3]),
        ):
            assert Serie.from_(series).as_py() == [1, 2, 3]

    def test_a_plain_numpy_array_is_one_column(self) -> None:
        column = Serie.from_(np.array([1.5, 2.5]))
        assert column.field is not None
        assert column.field.dtype == DataType("float64")
        assert column.into_arrow_array().to_pylist() == [1.5, 2.5]

    def test_a_numpy_record_dtype_names_its_members_so_it_is_rows(self) -> None:
        records = np.array(
            [("AAPL", 100), ("MSFT", 250)],
            dtype=[("symbol", "U4"), ("size", "i8")],
        )
        column = Serie.from_(records)
        assert type(column) is StructSerie
        assert len(column) == 2
        assert column.into_arrow_batch().column_names == ["symbol", "size"]

    def test_more_than_one_numpy_dimension_is_refused_by_name(self) -> None:
        with pytest.raises(TypeError, match="one-dimensional"):
            Serie.from_(np.zeros((2, 2)))

    def test_any_other_value_is_read_as_a_scalar(self) -> None:
        # A sequence is its rows, typed by the field it infers.
        rows = Serie.from_([1, None, 3])
        assert rows.is_column
        assert rows.as_py() == [1, None, 3]
        # Anything else is one row.
        assert Serie.from_(7).as_py() == [7]
        assert Serie.from_(7, price()).field == price()
        with pytest.raises(ValueError, match="empty Sequence"):
            Serie.from_([])
        assert len(Serie.from_([], price())) == 0

    def test_a_native_serie_is_shared_rather_than_re_read(self) -> None:
        column = Serie.from_(pa.array([1, 2, 3]))
        again = Serie.from_(column)
        assert again == column
        assert buffer_locations(again.into_arrow_array()) == buffer_locations(
            column.into_arrow_array()
        )


class TestDeclaredField:
    def test_the_declared_field_casts_in_rust(self) -> None:
        column = Serie.from_(pa.array([1, 2, 3]), "price: float64 not null")
        assert column.field == Field("price", "float64", nullable=False)
        assert column.into_arrow_array().type == pa.float64()

    def test_a_declared_root_reorders_and_retypes_a_table(self) -> None:
        declared = Field(
            "row",
            "struct<size: decimal128(12, 2) not null, symbol: utf8 not null>",
            nullable=False,
        )
        column = Serie.from_(quote_table(), declared)
        assert column.into_arrow_table().column_names == ["size", "symbol"]
        reader = SerieReader.from_(quote_table(), declared)
        assert reader.field == declared

    def test_safe_decides_whether_a_failed_conversion_is_null_or_an_error(self) -> None:
        text = pa.array(["not a number"])
        # `safe` is Arrow's own answer: a supported conversion that fails
        # becomes null when it is true, and an error when it is false.
        nulled = Serie.from_(text, "size: int64", safe=True)
        assert nulled.into_arrow_array().to_pylist() == [None]

        with pytest.raises(ValueError, match="Cannot cast"):
            Serie.from_(text, "size: int64", safe=False)


class TestCrossings:
    def test_a_held_column_shares_its_buffers_back_and_stays_readable(self) -> None:
        held = Serie.from_(quote_table().to_batches()[0])
        assert held.into_arrow_batch().num_rows == 2
        assert held.into_pandas().shape == (2, 2)
        assert held.into_polars().height == 2
        assert len(held) == 2

    def test_a_stream_crosses_once_and_says_so(self) -> None:
        streamed = SerieReader.from_(quote_table())
        assert streamed.into_arrow_reader().read_all().num_rows == 2
        with pytest.raises(ValueError, match="already handed over"):
            streamed.into_arrow_reader()
        with pytest.raises(ValueError, match="already handed over"):
            Serie.from_(streamed)

    def test_every_arrow_export_answers_its_own_pyarrow_class(self) -> None:
        batch = quote_table().to_batches()[0]
        assert isinstance(Serie.from_(batch).into_arrow_batch(), pa.RecordBatch)
        assert isinstance(Serie.from_(batch).into_arrow_table(), pa.Table)
        assert isinstance(Serie.from_(batch).into_arrow_reader(), pa.RecordBatchReader)
        assert isinstance(Serie.from_(pa.array([1])).into_arrow_array(), pa.Array)
        assert isinstance(
            Serie.from_(pa.scalar(1, pa.int64())).into_arrow_scalar(), pa.Scalar
        )

    def test_only_a_one_row_column_becomes_an_arrow_scalar(self) -> None:
        with pytest.raises(ValueError, match="one row"):
            Serie.from_(pa.array([1, 2, 3])).into_arrow_scalar()

    def test_numpy_takes_every_column_back(self) -> None:
        assert list(Serie.from_(pa.array([1, 2, 3])).into_numpy()) == [1, 2, 3]
        assert Serie.from_(pa.scalar(7, pa.int64())).into_numpy().tolist() == [7]

        # NumPy has no counterpart for Arrow's null mask or its nested
        # layouts, so both are allowed to copy rather than being refused.
        nulled = Serie.from_(pa.array([1, None, 3])).into_numpy()
        assert np.isnan(nulled[1])

        # A struct column has no NumPy layout, so PyArrow's own conversion
        # answers an object array of mappings rather than a record array.
        rows = Serie.from_(quote_table().to_batches()[0]).into_numpy()
        assert rows[0] == {"symbol": "AAPL", "size": 100}

    def test_a_columnar_scalar_is_a_serie_sharing_the_columns_buffers(self) -> None:
        array = pa.array([1, 2, 3])
        value = Scalar.from_(array)
        assert value.kind == "serie"
        assert value.as_py() == [1, 2, 3]
        held = value.as_serie()
        assert held is not None and held.is_column
        assert buffer_locations(held.into_arrow_array()) == buffer_locations(array)
        # A stream is never a Scalar: it is drained into the column it holds.
        assert Scalar.from_(quote_table()).as_py() == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]
        assert Scalar.from_({"rows": quote_table().to_batches()[0]}).as_py() == {
            "rows": [
                {"symbol": "AAPL", "size": 100},
                {"symbol": "MSFT", "size": 250},
            ]
        }

    def test_as_py_names_what_the_field_types(self) -> None:
        rows = Serie.from_(quote_table().to_batches()[0], quote_root()).as_py()
        assert rows == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]
        assert Serie.from_(pa.array([1, 2])).as_py() == [1, 2]
        assert Serie.from_(pa.scalar(7, pa.int64())).as_py() == [7]


class TestReaderFrom:
    def test_a_held_column_is_the_one_item_of_its_stream(self) -> None:
        records = Serie.from_arrow_batch(quotes())
        reader = SerieReader.from_serie(records)
        assert reader.field == records.field
        assert list(reader) == [records]

    def test_any_other_column_is_the_one_child_of_a_row(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        reader = SerieReader.from_serie(column)
        assert reader.field == Field("row", DataType.from_fields([price()]), nullable=False)
        (only,) = list(reader)
        assert only.child("price") == column

    def test_from_wraps_a_held_column_before_applying_the_root(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        root = Field(
            "row",
            "struct<price: float64 not null>",
            nullable=False,
        )
        reader = SerieReader.from_(column, root)
        assert reader.field == root
        (only,) = list(reader)
        assert only.field == root
        assert only.child("price").as_py() == [1.0, 2.0]

    @pytest.mark.parametrize("value", [7, [1, 2], (1, 2)])
    def test_from_reads_every_serie_value_as_one_stream_item(self, value: object) -> None:
        expected = SerieReader.from_serie(Serie.from_(value))
        reader = SerieReader.from_(value)
        assert reader.field == expected.field
        assert list(reader) == list(expected)

    def test_from_keeps_an_eager_value_refusal(self) -> None:
        cyclic: list[object] = []
        cyclic.append(cyclic)
        with pytest.raises(TypeError, match="cyclic Python values") as expected:
            Serie.from_(cyclic)
        with pytest.raises(TypeError, match="cyclic Python values") as actual:
            SerieReader.from_(cyclic)
        assert str(actual.value) == str(expected.value)

    def test_from_keeps_an_iterator_of_tables_streaming(self) -> None:
        pulled: list[int] = []

        def tables() -> Iterator[pa.Table]:
            for identifier in (1, 2):
                pulled.append(identifier)
                yield pa.table({"id": [identifier]})

        reader = SerieReader.from_(tables())
        assert pulled == [1]
        assert next(reader).child("id").as_py() == [1]
        assert pulled == [1]
        assert next(reader).child("id").as_py() == [2]
        assert pulled == [1, 2]

    def test_a_record_column_holding_an_absent_row_is_refused(self) -> None:
        records = Serie.from_(pa.array([{"id": 1}, None]))
        assert type(records) is StructSerie
        with pytest.raises(ValueError, match="absent rows"):
            SerieReader.from_serie(records)

    def test_from_reads_held_and_streamed_values_alike(self) -> None:
        for value in (quotes(), pa.Table.from_batches([quotes()]), Serie.from_(quotes())):
            reader = SerieReader.from_(value)
            assert Serie.from_(reader) == Serie.from_arrow_batch(quotes())

    @pytest.mark.parametrize(
        "records",
        [
            [{"id": 1, "symbol": "AAPL"}],
            ({"id": 1, "symbol": "AAPL"},),
        ],
    )
    def test_from_reads_concrete_mapping_sequences_as_record_rows(
        self, records: object
    ) -> None:
        reader = SerieReader.from_(records)
        assert reader.field == Field(
            "row", "struct<id: int64, symbol: utf8>", nullable=False
        )
        assert Serie.from_(reader).as_py() == [{"id": 1, "symbol": "AAPL"}]

    def test_from_reuses_the_first_reader_of_a_concrete_batch_sequence(self) -> None:
        class OneShotReader:
            def __init__(self, identifier: int) -> None:
                batch = pa.record_batch({"id": [identifier]})
                self.reader = pa.RecordBatchReader.from_batches(batch.schema, [batch])
                self.exports = 0

            def __arrow_c_stream__(self, requested_schema: object | None = None) -> object:
                self.exports += 1
                if self.exports != 1:
                    raise AssertionError("an Arrow stream was exported more than once")
                return self.reader.__arrow_c_stream__(requested_schema)

        sources = [OneShotReader(1), OneShotReader(2)]
        reader = SerieReader.from_(sources)
        assert [source.exports for source in sources] == [1, 0]
        assert [batch.child("id").as_py() for batch in reader] == [[1], [2]]
        assert [source.exports for source in sources] == [1, 1]


class TestChunked:
    """Chunks - a ``ChunkedSerie`` or a ``pyarrow.ChunkedArray`` - on the one ladder."""

    @staticmethod
    def chunked() -> ChunkedSerie:
        return ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1, 2], [3]]), price())

    def test_serie_from_is_the_one_join(self) -> None:
        joined = Serie.from_(self.chunked())
        assert type(joined) is Serie
        assert joined.field == price()
        assert joined.as_py() == [1, 2, 3]
        assert joined == self.chunked()
        declared = Serie.from_(self.chunked(), "price: float64 not null")
        assert declared.field == Field("price", "float64", nullable=False)
        assert Serie.from_(pa.chunked_array([[1], [2, 3]])).as_py() == [1, 2, 3]

    def test_one_chunk_joins_as_itself_sharing_its_buffers(self) -> None:
        array = pa.array([1, 2], pa.int64())
        joined = Serie.from_(pa.chunked_array([array]))
        assert buffer_locations(joined.into_arrow_array()) == buffer_locations(array)

    def test_a_stream_of_chunks_is_one_record_column_per_chunk(self) -> None:
        root = Field("row", DataType.from_fields([price()]), nullable=False)
        for reader in (
            SerieReader.from_chunked(self.chunked()),
            SerieReader.from_(self.chunked()),
        ):
            assert reader.field == root
            pulled = list(reader)
            assert [len(serie) for serie in pulled] == [2, 1]
            assert all(type(serie) is StructSerie for serie in pulled)
            assert [serie.child("price").as_py() for serie in pulled] == [[1, 2], [3]]

        # A chunked array streams one item per chunk, its column named `item`.
        streamed = SerieReader.from_(pa.chunked_array([[1, 2], [3]]))
        assert [serie.child("item").as_py() for serie in streamed] == [[1, 2], [3]]

    def test_a_record_chunk_is_the_batch_it_is(self) -> None:
        records = ChunkedSerie.from_arrow_reader(pa.Table.from_batches([quotes(), quotes()]))
        reader = SerieReader.from_chunked(records)
        assert reader.field == records.field
        assert list(reader) == records.chunks
        batches = SerieReader.from_chunked(records).into_arrow_reader()
        assert [batch.num_rows for batch in batches] == [2, 2]

    def test_a_root_is_applied_to_every_chunk(self) -> None:
        root = Field("row", "struct<price: float64 not null>", nullable=False)
        reader = SerieReader.from_(self.chunked(), root)
        assert reader.field == root
        pulled = list(reader)
        assert len(pulled) == 2
        assert all(serie.field == root for serie in pulled)
        assert [serie.child("price").as_py() for serie in pulled] == [[1.0, 2.0], [3.0]]

    def test_no_chunk_is_the_empty_stream_of_its_root(self) -> None:
        reader = SerieReader.from_chunked(ChunkedSerie.empty(price()))
        assert reader.field == Field("row", DataType.from_fields([price()]), nullable=False)
        assert list(reader) == []

    def test_a_record_chunk_holding_an_absent_row_is_refused(self) -> None:
        records = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[{"id": 1}, None]]))
        with pytest.raises(ValueError, match="absent rows"):
            SerieReader.from_chunked(records)
        with pytest.raises(ValueError, match="absent rows"):
            SerieReader.from_(records)

    def test_a_scalar_holds_the_joined_column(self) -> None:
        value = Scalar.from_(self.chunked())
        assert value.kind == "serie"
        assert value.as_py() == [1, 2, 3]
        assert value.as_serie() == Serie.from_scalars(price(), [1, 2, 3])

    def test_a_chunked_serie_is_written_one_batch_per_chunk(
        self, tmp_path: pathlib.Path
    ) -> None:
        records = ChunkedSerie.from_arrow_reader(pa.Table.from_batches([quotes(), quotes()]))
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_serie(records)
        assert [len(serie) for serie in handle.read_serie()] == [2, 2]
        assert Serie.from_(handle.read_serie()) == records


def declared_batch(keys_sorted: bool) -> tuple[Field, pa.RecordBatch]:
    mapping = pa.map_(pa.string(), pa.string(), keys_sorted=keys_sorted)
    lookup = pa.field("lookup", mapping, metadata={b"owner": b"lookup"})
    nested = pa.field("nested", pa.struct([lookup]), metadata={b"owner": b"nested"})
    history = pa.field("history", pa.list_(lookup), metadata={b"owner": b"history"})
    category = Field("category", "dictionary(int16,utf8)")
    category.set_dictionary_options(29, True)
    category.metadata["owner"] = "category"
    root = Field(
        "row",
        DataType.from_fields(
            [Field.from_arrow(field) for field in [lookup, nested, history]] + [category]
        ),
        nullable=False,
    )
    root.metadata["owner"] = "root"
    schema = root.into_arrow_schema()
    values = [None, [], [("first", "one"), ("second", None)]]
    arrays = [
        pa.array(values, type=mapping),
        pa.array([{"lookup": value} for value in values], type=nested.type),
        pa.array([[value] for value in values], type=history.type),
        pa.DictionaryArray.from_arrays(
            pa.array([0, None, 1], type=pa.int16()),
            pa.array(["one", "two"]),
            ordered=True,
        ),
    ]
    return root, pa.RecordBatch.from_arrays(arrays, schema=schema)


@pytest.mark.parametrize("whole_schema", [False, True])
def test_schema_capsule_import_calls_the_exporter_once_without_python_field_attributes(
    whole_schema: bool,
) -> None:
    root, batch = declared_batch(True)
    foreign = batch.schema if whole_schema else batch.schema.field("nested")
    calls = 0

    class SchemaExporter:
        def __arrow_c_schema__(self) -> object:
            nonlocal calls
            calls += 1
            return foreign.__arrow_c_schema__()

    source = SchemaExporter()
    assert not hasattr(source, "type")
    if whole_schema:
        imported = Field.from_arrow_schema(source, name=root.name)
        assert imported == root
        assert imported.into_arrow_schema().equals(batch.schema, check_metadata=True)
    else:
        imported = Field.from_arrow(source)
        assert imported == root.dtype["nested"]
        assert imported.into_arrow().equals(foreign, check_metadata=True)
    assert calls == 1


@pytest.mark.parametrize("keys_sorted", [False, True])
@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_preserve_nested_map_flags_metadata_and_shared_buffers(
    keys_sorted: bool, streamed: bool
) -> None:
    root, source = declared_batch(keys_sorted)
    held = Serie.from_(source)
    if streamed:
        reader = SerieReader.from_serie(held).into_arrow_reader()
        assert reader.schema.equals(source.schema, check_metadata=True)
        result = reader.read_next_batch()
        with pytest.raises(StopIteration):
            reader.read_next_batch()
    else:
        result = held.into_arrow_batch()

    assert result.num_rows == source.num_rows == 3
    assert result.schema.equals(source.schema, check_metadata=True)
    assert result.equals(source, check_metadata=True)
    restored = Field.from_arrow_schema(result.schema, name=root.name)
    assert restored == root
    assert restored.dtype["category"].dictionary_id == 29
    assert restored.dtype["category"].dictionary_is_ordered is True
    assert restored.metadata["owner"] == "root"
    for mapping in [
        result.schema.field("lookup").type,
        result.schema.field("nested").type.field("lookup").type,
        result.schema.field("history").type.value_type,
    ]:
        assert mapping.keys_sorted is keys_sorted
        assert not mapping.key_field.nullable
        assert mapping.item_field.nullable
    for original, exported in zip(source.columns, result.columns):
        assert buffer_locations(exported) == buffer_locations(original)
    assert buffer_locations(result.column(3).dictionary) == buffer_locations(
        source.column(3).dictionary
    )


@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_keep_nonzero_row_counts_with_no_columns(streamed: bool) -> None:
    empty = pa.array([{}, {}, {}], type=pa.struct([]))
    source = pa.RecordBatch.from_struct_array(empty).replace_schema_metadata(
        {b"owner": b"empty"}
    )
    held = Serie.from_(source)
    result = (
        held.into_arrow_reader().read_next_batch()
        if streamed
        else held.into_arrow_batch()
    )
    assert result.num_columns == 0
    assert result.num_rows == 3
    assert result.schema.equals(source.schema, check_metadata=True)


def test_reader_export_pulls_one_batch_at_a_time_and_fuses_after_a_source_error() -> None:
    source = pa.record_batch({"value": [1, 2, 3]})
    pulled: list[str] = []

    def batches() -> Iterator[pa.RecordBatch]:
        pulled.append("first")
        yield source
        pulled.append("failure")
        raise ValueError("batch source refused")

    incoming = pa.RecordBatchReader.from_batches(source.schema, batches())
    reader = SerieReader.from_(incoming).into_arrow_reader()
    assert pulled == []
    assert reader.schema.equals(source.schema, check_metadata=True)
    assert pulled == []
    first = reader.read_next_batch()
    assert pulled == ["first"]
    assert first.equals(source)
    assert buffer_locations(first.column(0)) == buffer_locations(source.column(0))
    with pytest.raises(pa.ArrowInvalid, match="batch source refused"):
        reader.read_next_batch()
    assert pulled == ["first", "failure"]
    for _ in range(2):
        with pytest.raises(StopIteration):
            reader.read_next_batch()
    assert pulled == ["first", "failure"]


def test_reader_export_can_be_consumed_by_a_python_worker_thread() -> None:
    _, source = declared_batch(True)
    constructed_on = threading.get_ident()
    pulled_on: list[int] = []

    def batches() -> Iterator[pa.RecordBatch]:
        for _ in range(2):
            pulled_on.append(threading.get_ident())
            yield source

    incoming = pa.RecordBatchReader.from_batches(source.schema, batches())
    reader = SerieReader.from_(incoming).into_arrow_reader()
    assert pulled_on == []
    with ThreadPoolExecutor(max_workers=1) as pool:
        exported = pool.submit(lambda: list(reader)).result(timeout=10)
    assert len(exported) == len(pulled_on) == 2
    assert all(thread != constructed_on for thread in pulled_on)
    for result in exported:
        assert result.equals(source, check_metadata=True)
        for original, column in zip(source.columns, result.columns):
            assert buffer_locations(column) == buffer_locations(original)


@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_rehydrate_registered_extension_identity_without_copying(
    streamed: bool,
) -> None:
    class BatchExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.serie.batch-export")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> BatchExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = BatchExtension()
    pa.register_extension_type(extension)
    try:
        storage = pa.array([1, None, 3], type=pa.int32())
        array = pa.ExtensionArray.from_storage(extension, storage)
        schema = pa.schema(
            [pa.field("payload", extension, metadata={b"owner": b"payload"})],
            metadata={b"owner": b"extension"},
        )
        source = pa.RecordBatch.from_arrays([array], schema=schema)
        held = Serie.from_(source)
        if streamed:
            reader = held.into_arrow_reader()
            assert reader.schema.equals(schema, check_metadata=True)
            result = reader.read_next_batch()
        else:
            result = held.into_arrow_batch()
        assert result.equals(source, check_metadata=True)
        assert result.schema.equals(schema, check_metadata=True)
        assert result.column(0).type == extension
        assert result.column(0).type.__arrow_ext_serialize__() == b"v1"
        assert buffer_locations(result.column(0).storage) == buffer_locations(storage)
    finally:
        pa.unregister_extension_type(extension.extension_name)


@pytest.mark.parametrize("keys_sorted", [False, True])
@pytest.mark.parametrize("scalar", [False, True])
def test_nonexact_datatype_cast_exports_nested_map_flags_and_metadata(
    keys_sorted: bool, scalar: bool
) -> None:
    def nested(value_type: pa.DataType) -> pa.DataType:
        return pa.struct(
            [
                pa.field(
                    "lookup",
                    pa.map_(pa.string(), value_type, keys_sorted=keys_sorted),
                    metadata={b"owner": b"lookup"},
                )
            ]
        )

    source = pa.array(
        [{"lookup": [("first", 1), ("second", None)]}, {"lookup": None}],
        type=nested(pa.int16()),
    )
    expected_type = nested(pa.int64())
    target = Field("value", DataType.from_arrow(expected_type), nullable=False)
    result: pa.Scalar | pa.Array
    if scalar:
        original = source[0]
        result = Serie.from_arrow_array(source.slice(0, 1), target).into_arrow_scalar()
        assert result.as_py() == original.as_py()
    else:
        original = source
        result = Serie.from_arrow_array(original, target).into_arrow_array()
        assert result.to_pylist() == original.to_pylist()
    assert result is not original
    assert result.type.equals(expected_type, check_metadata=True)
    mapping = result.type.field("lookup")
    assert mapping.metadata == {b"owner": b"lookup"}
    assert mapping.type.keys_sorted is keys_sorted
    assert not mapping.type.key_field.nullable
    assert mapping.type.item_field.nullable


@pytest.mark.parametrize("keys_sorted", [False, True])
def test_exact_map_batch_casts_share_the_callers_buffers(keys_sorted: bool) -> None:
    _, batch = declared_batch(keys_sorted)
    source = batch.select(["lookup", "nested", "history"]).replace_schema_metadata(None)
    field = Field.from_arrow_schema(source.schema, name="row")
    plan = ArrowCastPlan(source.schema, field)
    assert plan.is_identity

    for cast in (Serie.from_arrow_batch(source, field), plan.apply(source)):
        assert cast.field == field
        shared = cast.into_arrow_batch()
        assert shared.equals(source)
        for column, original in zip(shared.columns, source.columns):
            assert buffer_locations(column) == buffer_locations(original)


class TestPyCapsule:
    """Every class exports the Arrow PyCapsule Interface, buffers shared."""

    def test_a_column_crosses_to_any_consumer_sharing_its_buffers(self) -> None:
        source = pa.array([1, None, 3], pa.int64())
        column = Serie.from_(source)
        exported = pa.array(column)
        assert exported.equals(source)
        assert buffer_locations(exported) == buffer_locations(source)
        field = pa.Field._import_from_c_capsule(column.__arrow_c_schema__())
        assert field.equals(pa.field("item", pa.int64()), check_metadata=True)
        # A column is one array, as a `pyarrow.Array` is: only a record
        # column is also a stream.
        assert not hasattr(column, "__arrow_c_stream__")

    @pytest.mark.parametrize("route", ["batch", "table", "stream"])
    def test_a_record_column_crosses_under_its_exact_exchange_schema(self, route: str) -> None:
        _, source = declared_batch(True)
        held = Serie.from_(source)
        assert isinstance(held, StructSerie)
        if route == "batch":
            exported = pa.record_batch(held)
        elif route == "table":
            (exported,) = pa.table(held).to_batches()
        else:
            exported = pa.RecordBatchReader.from_stream(held).read_next_batch()
        assert exported.schema.equals(source.schema, check_metadata=True)
        assert exported.equals(source, check_metadata=True)
        restored = Field.from_arrow_schema(exported.schema, name="row")
        assert restored.dtype["category"].dictionary_id == 29
        assert exported.schema.field("lookup").type.keys_sorted
        for original, column in zip(source.columns, exported.columns):
            assert buffer_locations(column) == buffer_locations(original)

    def test_a_stream_crosses_to_pyarrow_and_polars_sharing_its_buffers(self) -> None:
        column = pa.array(range(1_024), pa.int64())
        table = pa.table({"value": column})
        exported = pa.RecordBatchReader.from_stream(SerieReader.from_(table)).read_all()
        assert exported.equals(table)
        assert buffer_locations(exported.column(0).chunk(0)) == buffer_locations(column)
        frame = polars.DataFrame(SerieReader.from_(table))
        assert frame["value"].to_list() == column.to_pylist()
        address, offset, length = frame["value"]._get_buffer_info()
        assert (address, offset, length) == (column.buffers()[1].address, 0, 1_024)

    def test_a_requested_schema_is_applied_by_the_one_cast(self) -> None:
        column = Serie.from_(pa.array([1, 2, 300], pa.int64()))
        assert pa.array(column, type=pa.int16()).equals(pa.array([1, 2, 300], pa.int16()))
        # Best effort, as `Serie.cast` is: a value the target cannot hold is
        # null under the default safe cast rather than a refusal.
        assert pa.array(column, type=pa.int8()).to_pylist() == [1, 2, None]
        wide = pa.schema([pa.field("id", pa.float64(), nullable=False), ("symbol", pa.utf8())])
        records = Serie.from_(quotes())
        assert pa.record_batch(records, schema=wide).schema.field("id").type == pa.float64()
        streamed = pa.RecordBatchReader.from_stream(SerieReader.from_(quotes()), schema=wide)
        assert streamed.read_all().column("id").to_pylist() == [1.0, 2.0]

    def test_a_run_has_no_layout_to_hand_over(self) -> None:
        run = Serie([1, 2])
        for export in (run.__arrow_c_array__, run.__arrow_c_schema__, run.into_arrow_array):
            with pytest.raises(ValueError, match="run declares no field"):
                export()

    def test_an_unconsumed_capsule_is_released(self) -> None:
        column = Serie.from_(pa.array(range(4_096), pa.int64()))
        records = Serie.from_(quotes())
        gc.collect()
        allocated = pa.total_allocated_bytes()
        for _ in range(64):
            column.__arrow_c_array__()
            column.__arrow_c_schema__()
            records.__arrow_c_stream__()
            # The capsule is all that holds this table's buffers.
            SerieReader.from_(pa.table({"value": list(range(4_096))})).__arrow_c_stream__()
        gc.collect()
        assert pa.total_allocated_bytes() == allocated

    def test_a_stream_capsule_is_consumed_by_a_worker_thread(self) -> None:
        table = pa.table({"value": list(range(16))})
        reader = pa.RecordBatchReader.from_stream(SerieReader.from_(table))
        with ThreadPoolExecutor(max_workers=1) as pool:
            assert pool.submit(reader.read_all).result(timeout=10).equals(table)

    @pytest.mark.parametrize("native", ["reader", "serie", "chunked"])
    def test_a_native_value_lands_as_what_it_holds_without_crossing_c(self, native: str) -> None:
        _, source = declared_batch(True)
        held = Serie.from_(source)
        value: object = {
            "reader": SerieReader.from_serie(held),
            "serie": held,
            "chunked": ChunkedSerie.from_serie(held),
        }[native]
        landed = Serie.from_arrow_reader(value)
        assert landed == held
        assert landed.field == held.field
        lookup = landed.child("lookup")
        assert isinstance(lookup, MapSerie) and lookup.keys_sorted
        assert landed.field.dtype["category"].dictionary_id == 29


class TestReaderRoot:
    def test_a_held_columns_own_root_hands_back_the_same_records(self) -> None:
        held = Serie.from_(quotes())
        (record,) = list(SerieReader.from_(held, held.field))
        assert record == held
        assert record.field == held.field
        for name in ("id", "symbol"):
            original = held.child(name)
            shared = record.child(name)
            assert original is not None and shared is not None
            assert buffer_locations(shared.into_arrow_array()) == buffer_locations(
                original.into_arrow_array()
            )

    def test_a_narrowing_root_refuses_naming_the_column(self) -> None:
        held = Serie.from_(pa.record_batch({"quantity": pa.array([1, 300], pa.int64())}))
        root = Field("row", "struct<quantity: int8>", nullable=False)
        with pytest.raises(ValueError, match="quantity"):
            list(SerieReader.from_(held, root, safe=False))
        (safe,) = list(SerieReader.from_(held, root))
        assert safe.as_py() == [{"quantity": 1}, {"quantity": None}]


class TestReaderCast:
    def test_a_held_reader_is_cast_at_the_call_and_spent(self) -> None:
        held = Serie.from_(quotes())
        wide = Field(
            "row",
            "struct<id: float64 not null, symbol: utf8 not null, price: float64>",
            nullable=False,
        )
        reader = SerieReader.from_serie(held)
        cast = reader.cast(wide)
        assert cast.field == wide
        (record,) = list(cast)
        assert record.field == wide
        assert record.child("id").as_py() == [float(value) for value in held.child("id").as_py()]
        # Spent: a second hand-over is refused, as after into_arrow_reader.
        with pytest.raises(ValueError, match="already"):
            reader.into_arrow_reader()

    def test_a_refused_option_or_target_leaves_the_reader_usable(self) -> None:
        held = Serie.from_(quotes())
        reader = SerieReader.from_serie(held)
        with pytest.raises(ValueError):
            reader.cast(held.field, representation="whenever")
        with pytest.raises(TypeError):
            reader.cast(object())
        (record,) = list(reader.cast(held.field))
        assert record == held

    def test_a_stream_is_cast_as_it_is_pulled(self) -> None:
        batches = [
            pa.record_batch({"quantity": pa.array([1, 2], pa.int64())}),
            pa.record_batch({"quantity": pa.array([3, 300], pa.int64())}),
        ]
        source = pa.RecordBatchReader.from_batches(batches[0].schema, batches)
        narrow = Field("row", "struct<quantity: int8>", nullable=False)
        cast = SerieReader.from_arrow_reader(source).cast(narrow, safe=False)
        assert next(cast).as_py() == [{"quantity": 1}, {"quantity": 2}]
        with pytest.raises(ValueError, match="quantity"):
            next(cast)
        wide = Field("row", "struct<quantity: float64>", nullable=False)
        source = pa.RecordBatchReader.from_batches(batches[0].schema, batches)
        table = SerieReader.from_arrow_reader(source).cast(wide).into_arrow_reader().read_all()
        assert table.schema.field("quantity").type == pa.float64()
        assert table.column("quantity").to_pylist() == [1.0, 2.0, 3.0, 300.0]


def test_a_source_error_holding_a_nul_surfaces_as_arrow_invalid() -> None:
    def tables() -> Iterator[pa.Table]:
        yield pa.table({"value": [1]})
        raise ValueError("refused \x00 here")

    reader = SerieReader.from_(tables()).into_arrow_reader()
    assert reader.read_next_batch().num_rows == 1
    # The C stream's error text is a C string: the NUL is restated, so the
    # message crosses whole instead of aborting the process.
    with pytest.raises(pa.ArrowInvalid, match="refused \ufffd here"):
        reader.read_next_batch()
    with pytest.raises(StopIteration):
        reader.read_next_batch()


@contextlib.contextmanager
def no_forced_switch() -> Iterator[None]:
    """Hand the GIL over only when its holder lets go.

    A thread waiting on the GIL forces a switch after `sys.getswitchinterval`,
    whether or not the holder released it; raised far beyond any call here,
    a second thread runs during a call exactly when the call releases the
    GIL - never during one that holds it - and each loop below yields it
    back with `time.sleep(0)`.
    """
    interval = sys.getswitchinterval()
    sys.setswitchinterval(60)
    try:
        yield
    finally:
        sys.setswitchinterval(interval)


def spun_during(operation: Callable[[], object]) -> int:
    """How far a second Python thread counts while `operation` runs."""
    count = 0
    started = threading.Event()
    stop = threading.Event()

    def spin() -> None:
        nonlocal count
        started.set()
        while not stop.is_set():
            count += 1
            if count % 1_000 == 0:
                time.sleep(0)

    with no_forced_switch():
        spinner = threading.Thread(target=spin)
        spinner.start()
        started.wait()
        before = count
        try:
            operation()
        finally:
            after = count
            stop.set()
            spinner.join()
    return after - before


def test_a_whole_drain_releases_the_gil() -> None:
    batch = pa.RecordBatch.from_arrays(
        [pa.array(range(1_024), pa.int64()) for _ in range(64)],
        names=[f"c{index}" for index in range(64)],
    )
    table = pa.Table.from_batches([batch] * 64)
    # Held throughout, the GIL would leave the spinner not one turn during
    # the call; released, it counts for the whole drain.
    assert spun_during(lambda: Serie.from_arrow_reader(table)) > 1_000
    held = Serie.from_(table).child("c0")
    assert held is not None
    assert spun_during(held.as_py) == 0


def test_a_long_cast_leaves_the_serie_writable_from_another_thread() -> None:
    column = Serie.from_(pa.array(range(200_000), pa.int64()))
    target = Field("item", "float64")
    casting = threading.Event()
    done = threading.Event()
    errors: list[BaseException] = []
    writes = 0

    def write() -> None:
        nonlocal writes
        casting.wait()
        while not done.is_set():
            try:
                column.set(0, writes)
            except BaseException as error:  # noqa: BLE001 - the pin is that none is raised
                errors.append(error)
                return
            writes += 1
            time.sleep(0)

    with no_forced_switch():
        writer = threading.Thread(target=write)
        writer.start()
        casting.set()
        try:
            # The writer runs only while a cast has released the GIL, and a
            # cast holding its borrow over that would make each write raise.
            for _ in range(8):
                assert len(column.cast(target)) == 200_000
        finally:
            done.set()
            writer.join()
    assert errors == []
    assert writes > 0


# Ordering, uniqueness and grouping - mirrors ``rust/tests/serie/order.rs``
# case for case, through the binding's own spellings: two keyword booleans
# for the ordering, and indices, masks and keys read from a ``Serie``, any
# columnar object or an iterable of values.


def int64_column(values: list[int | None]) -> Serie:
    nullable = any(value is None for value in values)
    return Serie.from_arrow_array(
        pa.array(values, pa.int64()), Field("price", "int64", nullable=nullable)
    )


def utf8_column(values: list[str | None]) -> Serie:
    nullable = any(value is None for value in values)
    return Serie.from_arrow_array(
        pa.array(values, pa.utf8()), Field("venue", "utf8", nullable=nullable)
    )


def quote_rows(rows: list[tuple[str, int]]) -> Serie:
    root = Field(
        "quote", "struct<venue: utf8 not null, price: int64 not null>", nullable=False
    )
    return Serie.from_scalars(root, [[venue, price] for venue, price in rows])


def nested_columns() -> list[Serie]:
    """Every nested, encoded and viewed layout: the ladder's lower rungs."""
    records = quote_rows([("XNYS", 2), ("XNAS", 2), ("XNAS", 1), ("XNYS", 2)])
    lists = Serie.from_arrow_array(pa.array([[2, 3], [1], [2, 3], [2]], pa.list_(pa.int64())))
    dictionary = Serie.from_arrow_array(
        pa.DictionaryArray.from_arrays(
            pa.array([1, 0, 1, 2], pa.int8()), pa.array(["XNAS", "XNYS", "XPAR"])
        )
    )
    views = Serie.from_arrow_array(
        pa.array(
            ["XNYS", "XNAS", "a view longer than twelve bytes", "XNYS"], pa.string_view()
        )
    )
    booleans = Serie.from_arrow_array(pa.array([True, False, None, True]))
    return [records, lists, dictionary, views, booleans]


class TestOrder:
    def test_sort_indices_answers_a_uint32_index_column_stable_on_both_leaves(self) -> None:
        for serie in (Serie([3, 1, 2, 1]), int64_column([3, 1, 2, 1])):
            order = serie.sort_indices()
            assert order.field == Field("index", "uint32", nullable=False)
            # The two equal rows keep their order: stable.
            assert order.as_py() == [1, 3, 2, 0]
            assert serie.sort_indices(descending=True).as_py() == [0, 2, 1, 3]

    def test_absent_rows_gather_to_the_end_the_options_name(self) -> None:
        for serie in (
            Serie([None, 2, None, 1]),
            int64_column([None, 2, None, 1]),
            utf8_column([None, "b", None, "a"]),
        ):
            assert serie.sort_indices().as_py() == [3, 1, 0, 2]
            first = serie.sort_indices(descending=True, nulls_first=True)
            assert first.as_py() == [0, 2, 1, 3]

    def test_is_sorted_reads_adjacent_rows_under_the_options_on_both_leaves(self) -> None:
        for serie in (Serie([1, 1, 2, None]), int64_column([1, 1, 2, None])):
            assert serie.is_sorted()
            assert not serie.is_sorted(descending=True)
            assert not serie.is_sorted(nulls_first=True)
            assert serie.into_reversed().is_sorted(descending=True, nulls_first=True)
        assert Serie([]).is_sorted()
        assert int64_column([1]).is_sorted(descending=True, nulls_first=True)

    def test_uniqueness_counts_an_absent_row_as_one_value_on_both_leaves(self) -> None:
        for serie in (Serie([1, None, 1, None, 2]), int64_column([1, None, 1, None, 2])):
            assert not serie.is_unique()
            assert serie.unique_count() == 3
            unique = serie.into_unique()
            assert unique.as_py() == [1, None, 2]
            assert unique.is_unique()
            assert unique.field == serie.field
            # The serie is as it was.
            assert len(serie) == 5
        assert Serie([1, None]).is_unique()

    def test_into_sorted_answers_a_new_serie_under_the_same_field_and_leaves_this_one(
        self,
    ) -> None:
        column = int64_column([3, None, 1])
        ordered = column.into_sorted()
        assert ordered.as_py() == [1, 3, None]
        assert ordered.field == column.field
        assert ordered.is_sorted()
        assert column.scalar(0).as_py() == 3

        run = Serie(["b", "a"])
        ordered = run.into_sorted()
        assert ordered.as_py() == ["a", "b"]
        assert ordered.field is None

    def test_into_reversed_reverses_both_leaves_with_their_absences(self) -> None:
        assert int64_column([1, None, 3]).into_reversed().as_py() == [3, None, 1]
        assert Serie([1, 2]).into_reversed().as_py() == [2, 1]
        assert Serie([]).into_reversed().is_empty()

    def test_into_taken_reads_indices_of_any_integer_width_and_refuses_what_names_no_row(
        self,
    ) -> None:
        column = int64_column([10, 20, 30])
        run = Serie([10, 20, 30])
        every_spelling: list[object] = [
            Serie.from_scalars(Field("index", "uint32"), [2, 0, 2]),
            Serie([2, 0, 2]),
            int64_column([2, 0, 2]),
            pa.array([2, 0, 2], pa.uint8()),
            np.array([2, 0, 2], dtype=np.int16),
            [2, 0, 2],
            (index for index in (2, 0, 2)),
        ]
        for serie in (column, run):
            for indices in every_spelling:
                if not isinstance(indices, (list, Serie, pa.Array, np.ndarray)):
                    indices = (index for index in (2, 0, 2))
                taken = serie.into_taken(indices)
                assert taken.as_py() == [30, 10, 30]
                assert taken.field == serie.field
            assert serie.into_taken([]).is_empty()
            for refused in ([3], [-1], [None], ["0"], int64_column([None])):
                with pytest.raises(ValueError, match="names no row of the 3"):
                    serie.into_taken(refused)
        for spelling in ("0", b"0", {"index": 0}, 0):
            with pytest.raises(TypeError, match="expected indices as a Serie"):
                column.into_taken(spelling)

    def test_into_filtered_keeps_what_the_mask_keeps_and_an_absent_mask_row_keeps_nothing(
        self,
    ) -> None:
        column = int64_column([10, 20, 30])
        run = Serie([10, 20, 30])
        masks: list[object] = [
            Serie([True, None, True]),
            Serie.from_arrow_array(pa.array([True, None, True])),
            pa.array([True, None, True]),
            [True, None, True],
        ]
        for serie in (column, run):
            for mask in masks:
                kept = serie.into_filtered(mask)
                assert kept.as_py() == [10, 30]
                assert kept.field == serie.field
            with pytest.raises(ValueError, match="a mask of 1 rows cannot filter the 3 rows"):
                serie.into_filtered([True])
            with pytest.raises(ValueError, match="neither a boolean nor absent"):
                serie.into_filtered([1, 1, 1])

    def test_partition_by_groups_in_first_occurrence_order_and_slices_sorted_keys_zero_copy(
        self,
    ) -> None:
        prices = int64_column([1, 2, 3, 4])
        groups = prices.partition_by(utf8_column(["a", "a", "b", None]))
        assert [(key.as_py(), rows.as_py()) for key, rows in groups] == [
            ("a", [1, 2]),
            ("b", [3]),
            (None, [4]),
        ]
        # Sorted keys: every group is a slice sharing the column's buffer.
        held = prices.into_arrow_array().buffers()[1]
        for _, rows in groups:
            values = rows.into_arrow_array().buffers()[1]
            assert held.address <= values.address < held.address + held.size

        groups = prices.partition_by(["b", "a", "b", "a"])
        assert [(key.as_py(), rows.as_py()) for key, rows in groups] == [
            ("b", [1, 3]),
            ("a", [2, 4]),
        ]

        # A run partitions the same way, by a run of keys.
        groups = Serie([1, 2, 3, 4]).partition_by(Serie(["b", "a", "b", None]))
        assert len(groups) == 3
        assert groups[2][0].as_py() is None
        assert groups[2][1].as_py() == [4]

        with pytest.raises(ValueError, match="1 keys cannot partition the 4 rows"):
            prices.partition_by([1])
        assert Serie([]).partition_by([]) == []

    def test_partition_by_paths_keys_a_record_by_the_run_of_its_cells(self) -> None:
        quotes = quote_rows([("XNAS", 1), ("XNYS", 2), ("XNAS", 1), ("XNAS", 3)])
        groups = quotes.partition_by_paths(["venue", FieldPath("price")])
        assert len(groups) == 3
        assert groups[0][0].as_py() == ["XNAS", 1]
        assert len(groups[0][1]) == 2
        assert groups[0][1].field == quotes.field
        assert type(groups[0][1]) is StructSerie
        assert groups[1][0].as_py() == ["XNYS", 2]
        # One path alone is a path, never the characters of its text.
        by_venue = quotes.partition_by_paths("venue")
        assert len(by_venue) == 2
        assert by_venue[0][0].as_py() == ["XNAS"]
        assert len(by_venue[0][1]) == 3
        # One child is the same ask through `child`.
        venue = quotes.child("venue")
        assert venue is not None
        by_child = quotes.partition_by(venue)
        assert by_child[0][0].as_py() == "XNAS"
        assert by_child[0][1] == by_venue[0][1]

        with pytest.raises(ValueError, match="partitions by no path"):
            quotes.partition_by_paths([])
        with pytest.raises(ValueError, match="tier reaches no column of quote"):
            quotes.partition_by_paths(["tier"])
        with pytest.raises(ValueError, match="a schema-free run partitions by no path"):
            Serie([1]).partition_by_paths(["price"])
        with pytest.raises(TypeError, match="expected a FieldPath, a path string"):
            quotes.partition_by_paths(3)  # type: ignore[arg-type]

    def test_memory_size_counts_a_column_as_its_slice_and_a_run_as_its_values(self) -> None:
        column = int64_column(list(range(1_000)))
        whole = column.memory_size()
        assert whole >= 8_000
        assert column.slice(0, 10).memory_size() < whole // 10
        assert Serie([1, 2, 3]).memory_size() > 0
        assert Serie([]).memory_size() == 0

    def test_as_sorted_rewrites_a_primitive_column_in_place_gathering_its_absences(
        self,
    ) -> None:
        for descending, nulls_first, expected in [
            (False, False, [1, 2, 3, None, None]),
            (True, False, [3, 2, 1, None, None]),
            (False, True, [None, None, 1, 2, 3]),
            (True, True, [None, None, 3, 2, 1]),
        ]:
            column = int64_column([3, None, 1, None, 2])
            field = column.field
            assert column.as_sorted(descending=descending, nulls_first=nulls_first) is column
            assert column.as_py() == expected
            assert column.null_count() == 2
            assert column.field == field
            assert column.is_sorted(descending=descending, nulls_first=nulls_first)
            assert column.into_arrow_array().null_count == 2
        # No absence: the slice alone.
        column = int64_column([3, 1, 2])
        assert column.as_sorted().as_reversed() is column
        assert column.as_py() == [3, 2, 1]

    def test_as_sorted_on_a_shared_column_copies_once_and_leaves_the_other_holder_alone(
        self,
    ) -> None:
        column = int64_column([2, 1])
        other = copy.copy(column)
        other.as_sorted()
        assert other.as_py() == [1, 2]
        assert column.as_py() == [2, 1]
        # A column over pyarrow's buffers never writes them: the write copies.
        array = pa.array([2, 1], pa.int64())
        shared = Serie.from_arrow_array(array, Field("price", "int64", nullable=False))
        shared.as_sorted().as_reversed()
        assert shared.as_py() == [2, 1] and shared.as_sorted().as_py() == [1, 2]
        assert array.to_pylist() == [2, 1]

    def test_every_as_write_brings_a_run_or_a_kernel_leaf_into_the_state_and_chains(
        self,
    ) -> None:
        run = Serie(["b", None, "a", "b"])
        assert run.as_sorted().as_unique().as_reversed() is run
        assert run.as_py() == [None, "b", "a"]
        assert run.as_taken([2, 1]).as_filtered([False, True]) is run
        assert run.as_py() == ["b"]

        strings = utf8_column(["b", None, "a", "b"])
        field = strings.field
        strings.as_sorted().as_unique().as_reversed()
        assert strings.as_py() == [None, "b", "a"]
        assert strings.field == field
        strings.as_taken([2, 1]).as_filtered([False, True])
        assert strings.as_py() == ["b"]
        assert strings.field == field

    def test_a_refused_write_leaves_the_serie_as_it_was(self) -> None:
        column = int64_column([2, 1])
        with pytest.raises(ValueError, match="names no row"):
            column.as_taken([5])
        with pytest.raises(ValueError, match="cannot filter"):
            column.as_filtered([])
        with pytest.raises(TypeError, match="expected mask"):
            column.as_filtered("01")
        assert column.as_py() == [2, 1]

    def test_as_reversed_reverses_a_primitive_column_in_place_with_its_validity(self) -> None:
        column = int64_column([1, None, None, 4, 5])
        assert column.as_reversed() is column
        assert column.as_py() == [5, 4, None, None, 1]
        assert column.null_count() == 2

    @pytest.mark.parametrize(
        "column",
        nested_columns(),
        ids=["records", "lists", "dictionary", "views", "booleans"],
    )
    def test_every_layout_answers_every_verb_through_the_ladder(self, column: Serie) -> None:
        assert len(column.sort_indices()) == 4
        ordered = column.into_sorted()
        assert ordered.is_sorted()
        assert ordered.field is not None and column.field is not None
        assert ordered.field.dtype == column.field.dtype
        assert ordered.field.name == column.field.name
        # A record's whole-row sort declares every column as the order its
        # rows keep; no other leaf states one.
        declared = ["venue", "price"] if type(column) is StructSerie else None
        assert ordered.declared_order() == declared
        # Each answer is handed out as its leaf's class.
        assert type(ordered) is type(column)
        # The sort agrees with the values' own order, absences last.
        present = sorted(row for row in column.rows() if row.as_py() is not None)
        absent = [row for row in column.rows() if row.as_py() is None]
        assert ordered.rows() == present + absent
        assert column.into_sorted(descending=True).is_sorted(descending=True)
        assert not column.is_unique()
        assert column.unique_count() == 3
        unique = column.into_unique()
        assert len(unique) == 3
        assert unique.is_unique()
        assert unique.scalar(0) == column.scalar(0)
        assert column.into_reversed().scalar(0) == column.scalar(3)
        groups = column.partition_by(column)
        assert len(groups) == 3
        assert len(groups[0][1]) == 2
        written = copy.copy(column)
        written.as_sorted().as_unique().as_reversed()
        assert len(written) == 3
        assert written.field is not None
        assert written.field.dtype == column.field.dtype
        # Ascending with absences last, reversed, is descending with
        # absences first - which a record's root then declares.
        assert written.is_sorted(descending=True, nulls_first=True)
        reversed_ = ["venue desc nulls first", "price desc nulls first"]
        assert written.declared_order() == (reversed_ if declared else None)
        assert column.memory_size() > 0


# ---------------------------------------------------------------------------
# window_by: the rows cut where the key changes - mirrors the window_by cases
# of rust/tests/serie/order.rs and the reader's of rust/tests/serie/arrow.rs
# ---------------------------------------------------------------------------

MINUTE_NS = 60_000_000_000


def quote_field() -> Field:
    """The record `quote{venue, count, ts}`, its root nullable so a row may be
    absent."""
    return Field(
        "quote",
        "struct<venue: utf8, count: int64 not null, ts: timestamp(ns, UTC)>",
        nullable=True,
    )


def quote_column(rows: list[tuple[str | None, int, int] | None]) -> Serie:
    """Quotes, each its venue, its count and its instant in minutes."""
    return Serie.from_scalars(
        quote_field(),
        [None if row is None else [row[0], row[1], row[2] * MINUTE_NS] for row in rows],
    )


def venue_runs() -> Serie:
    """The venues XNAS, XNAS, XNYS, XNAS at minutes 0, 14, 15 and 31."""
    return quote_column([("XNAS", 1, 0), ("XNAS", 2, 14), ("XNYS", 3, 15), ("XNAS", 4, 31)])


def window_cuts(windows: list[tuple[Scalar, WindowSerie]]) -> list[tuple[object, int, int]]:
    return [(key.as_py(), window.offset, len(window)) for key, window in windows]


def reader_rows(window: SerieReader) -> list[object]:
    return [row for piece in window for row in piece.as_py()]


class TestWindowBy:
    @pytest.mark.parametrize("sorted_", [False, True])
    def test_window_by_refuses_a_run_an_empty_key_an_unnest_and_a_term_naming_no_column(
        self, sorted_: bool
    ) -> None:
        for serie in (venue_runs(), quote_column([])):
            with pytest.raises(ValueError, match="expected a value or a name"):
                serie.window_by("venue,", sorted_)
            for empty in ("*", []):
                with pytest.raises(
                    ValueError,
                    match="expected at least one column to window by, got an empty match key",
                ):
                    serie.window_by(empty, sorted_)
            with pytest.raises(ValueError, match="in a key"):
                serie.window_by("unnest(items)", sorted_)
            with pytest.raises(ValueError, match="tier"):
                serie.window_by("tier", sorted_)
            for text in ("minutes(ts, 0)", "minutes(ts, count)"):
                with pytest.raises(ValueError):
                    serie.window_by(text, sorted_)
            # A key cell named as a static value is refused before any row.
            with pytest.raises(ValueError, match='collides with the static value "rownum"'):
                serie.window_by("count as ROWNUM", sorted_)
        with pytest.raises(ValueError, match="a schema-free run windows by no term"):
            Serie([1, 2]).window_by("price", sorted_)
        with pytest.raises(TypeError):
            venue_runs().window_by("venue", 1)  # type: ignore[arg-type]

    def test_window_by_cuts_runs_of_equal_adjacent_keys_in_row_order(self) -> None:
        quotes = venue_runs()
        windows = quotes.window_by("venue")
        assert window_cuts(windows) == [(["XNAS"], 0, 2), (["XNYS"], 2, 1), (["XNAS"], 3, 1)]
        for key, window in windows:
            assert isinstance(window, WindowSerie)
            # Every window is over the serie object itself.
            assert window.serie is quotes
            assert key.as_py() == [window[0].as_py()[0]]
        # `sorted` is positional or keyword, and `None` is its default.
        for spelled in (
            quotes.window_by("venue", False),
            quotes.window_by("venue", None),
            quotes.window_by(by="venue", sorted=None),
        ):
            assert window_cuts(spelled) == window_cuts(windows)
        # Two terms key a two-cell run in selector order; a list is
        # projection texts; a period keys its number since the epoch.
        expected = [(["XNAS", 0], 0, 2), (["XNYS", 1], 2, 1), (["XNAS", 2], 3, 1)]
        assert window_cuts(quotes.window_by("venue, minutes(ts, 15) as bucket")) == expected
        assert window_cuts(quotes.window_by(["venue", "minutes(ts, 15) as bucket"])) == expected
        assert window_cuts(quotes.window_by("VENUE")) == window_cuts(windows)
        assert len(quotes.window_by("count")) == 4
        assert quote_column([]).window_by("venue") == []

    def test_window_by_keys_consecutive_absent_rows_as_one_null_window(self) -> None:
        quotes = quote_column([("XNAS", 1, 0), None, None, (None, 4, 0), (None, 5, 0)])
        assert window_cuts(quotes.window_by("venue")) == [
            (["XNAS"], 0, 1),
            (None, 1, 2),
            ([None], 3, 2),
        ]

    def test_window_by_sorted_gathers_the_rows_once_in_stable_key_order(self) -> None:
        quotes = quote_column(
            [("XNYS", 1, 0), ("XNAS", 2, 0), ("XNYS", 3, 0), None, ("XNAS", 5, 0)]
        )
        windows = quotes.window_by("venue", True)
        assert window_cuts(windows) == [(["XNAS"], 0, 2), (["XNYS"], 2, 2), (None, 4, 1)]
        gathered = windows[0][1].serie
        assert gathered is not quotes
        assert isinstance(gathered, StructSerie)
        assert all(window.serie is gathered for _, window in windows)
        assert gathered == quotes.into_taken([1, 4, 0, 2, 3])
        # Over keys already in order, nothing is gathered.
        ordered = venue_runs().window_by("minutes(ts, 15)", sorted=True)
        assert all(window.serie is not gathered for _, window in ordered)
        assert window_cuts(ordered) == [([0], 0, 2), ([1], 2, 1), ([2], 3, 1)]

    def test_a_non_record_column_windows_by_its_own_name(self) -> None:
        venues = Serie.from_arrow_array(
            pa.array(["XNAS", "XNAS", "XNYS"]), Field("venue", "utf8", nullable=False)
        )
        windows = venues.window_by("venue")
        assert window_cuts(windows) == [(["XNAS"], 0, 2), (["XNYS"], 2, 1)]
        assert [window.static_values.as_py() for _, window in windows if window.static_values] == [
            {"venue": "XNAS", "windownum": 0, "rownum": 0},
            {"venue": "XNYS", "windownum": 1, "rownum": 2},
        ]


class TestReaderWindowBy:
    def test_reader_window_by_refuses_before_any_pull(self) -> None:
        # A text that does not parse leaves the reader usable.
        reader = SerieReader.from_serie(venue_runs())
        with pytest.raises(ValueError, match="expected a value or a name"):
            reader.window_by("venue,")
        with pytest.raises(TypeError):
            reader.window_by("venue", 1)  # type: ignore[arg-type]
        assert len(list(reader)) == 1
        # A key the root refuses spends it, as a refused cast does.
        for refused, reason in (
            ("*", "empty match key"),
            ("tier", "tier"),
            ("unnest(items)", "in a key"),
            ("count as windownum", 'collides with the static value "windownum"'),
        ):
            reader = SerieReader.from_serie(venue_runs())
            with pytest.raises(ValueError, match=reason):
                reader.window_by(refused)
            assert list(reader) == []
            with pytest.raises(ValueError, match="already handed over"):
                reader.window_by("venue")

    def test_reader_window_by_yields_one_lazy_reader_per_window(self) -> None:
        quotes = venue_runs()
        stream = SerieReader.from_chunked(
            ChunkedSerie.from_series([quotes.slice(0, 1), quotes.slice(1, 3)], quotes.field)
        )
        walk = stream.window_by("venue")
        assert isinstance(walk, SerieReaderWindows)
        assert iter(walk) is walk
        # Both records are known before a batch is pulled; the root a held
        # column streams under is required.
        assert walk.field == SerieReader.from_serie(quotes).field
        assert not walk.field.nullable
        assert [child.name for child in walk.static_field] == ["venue", "windownum", "rownum"]
        assert repr(walk).startswith("SerieReaderWindows(field=")
        with pytest.raises(TypeError):
            hash(walk)
        xnas = next(walk)
        assert isinstance(xnas, SerieReader)
        assert xnas.field == walk.field
        assert xnas.static_values is not None
        assert xnas.static_values.as_py() == {"venue": "XNAS", "windownum": 0, "rownum": 0}
        # The run crossing the batch edge is one window, one piece per batch.
        assert [len(piece) for piece in xnas] == [1, 1]
        xnys = next(walk)
        assert reader_rows(xnys) == quotes.slice(2, 1).as_py()
        tail = next(walk)
        assert tail.static_values is not None
        assert tail.static_values.as_py() == {"venue": "XNAS", "windownum": 2, "rownum": 3}
        with pytest.raises(StopIteration):
            next(walk)
        # Walking without reading raises nothing.
        for _ in SerieReader.from_serie(quotes).window_by("venue", sorted=None):
            pass

    def test_reader_window_by_refuses_a_window_the_walk_passed(self) -> None:
        windows = list(SerieReader.from_serie(venue_runs()).window_by("venue"))
        assert len(windows) == 3
        with pytest.raises(
            ValueError,
            match="window 0 was passed by its walk with rows unread; read each window before "
            "taking the next",
        ):
            next(windows[0])
        # Fused after the refusal.
        assert list(windows[0]) == []

    def test_reader_window_by_sorted_refuses_a_key_going_backwards_naming_batch_and_row(
        self,
    ) -> None:
        quotes = quote_column([("XLON", 1, 0), ("XNYS", 2, 0), ("XNAS", 3, 0)])
        walk = SerieReader.from_serie(quotes).window_by("venue", True)
        assert len(reader_rows(next(walk))) == 1
        assert len(reader_rows(next(walk))) == 1
        with pytest.raises(
            ValueError,
            match=r"window by expects keys in order, ascending with absent keys last: batch 0 "
            r"row 2",
        ):
            next(walk)
        assert list(walk) == []
        # Unsorted, every key is windowed where it arrives.
        unsorted = SerieReader.from_serie(quotes).window_by("venue")
        venues = [window.static_values.as_py()["venue"] for window in unsorted if window.static_values]
        assert venues == ["XLON", "XNYS", "XNAS"]

    def test_a_reader_window_states_the_record_a_held_window_states(self) -> None:
        quotes = venue_runs()
        for by in ("venue", "minutes(ts, 15) as bucket, venue"):
            for sorted_ in (False, True):
                held = [
                    window.static_values.as_py()
                    for _, window in quotes.window_by(by, sorted_)
                    if window.static_values
                ]
                walk = SerieReader.from_serie(quotes).window_by(by, sorted_)
                if by == "venue" and sorted_:
                    # Out of order, the held rows gather and state no rownum,
                    # where the stream refuses the key going back.
                    assert [record["rownum"] for record in held] == [None, None]
                    with pytest.raises(ValueError, match="expects keys in order"):
                        list(walk)
                    continue
                streamed = [
                    window.static_values.as_py() for window in walk if window.static_values
                ]
                assert streamed == held
        # A window of a stream window keeps the outer cells and an absolute
        # rownum.
        outer = next(SerieReader.from_serie(quotes).window_by("minutes(ts, 30) as half"))
        inner = [
            window.static_values.as_py() for window in outer.window_by("venue") if window.static_values
        ]
        assert inner == [
            {"half": 0, "venue": "XNAS", "windownum": 0, "rownum": 0},
            {"half": 0, "venue": "XNYS", "windownum": 1, "rownum": 2},
        ]

    def test_reader_static_values_survive_cast_and_hand_over_and_never_reach_a_batch(
        self,
    ) -> None:
        quotes = venue_runs()
        assert SerieReader.from_serie(quotes).static_values is None
        window = next(SerieReader.from_serie(quotes).window_by("venue"))
        record = window.static_values
        assert record is not None
        wider = Field(
            "quote",
            "struct<venue: utf8, count: float64 not null, ts: timestamp(ns, UTC)>",
            nullable=True,
        )
        cast = window.cast(wider)
        assert cast.static_values == record
        batches = cast.into_arrow_reader()
        assert cast.static_values == record
        assert batches.schema.names == ["venue", "count", "ts"]
        assert batches.read_all().num_rows == 2


# ---------------------------------------------------------------------------
# Spill, `order by` keys and joins - mirrors rust/tests/serie/spill.rs,
# the `*_by` cases of rust/tests/serie/order.rs, rust/tests/root/join.rs and
# rust/tests/serie/join.rs through the binding's spellings: a `SpillOptions`
# or its keywords, `by` as one `Scalar` or a `Selector`, `how` as a word.
# ---------------------------------------------------------------------------


def spill_prices(rows: int) -> Serie:
    return Serie.from_scalars(Field("price", "int64", nullable=False), list(range(rows)))


def keyed_quotes() -> Serie:
    """`quote{venue, price}`, a price absent - the `sort_indices_by` example."""
    return Serie.from_scalars(
        Field("quote", "struct<venue: utf8 not null, price: int64>", nullable=False),
        [["XNYS", 1], ["XNAS", 2], ["XNAS", None], ["XNYS", 3]],
    )


class TestSpill:
    def test_a_column_built_on_the_heap_is_resident_whole_and_not_spilled(self) -> None:
        run = Serie([1, 2])
        assert run.resident_size() == run.memory_size()
        assert not run.is_spilled()
        column = spill_prices(1_024)
        assert column.resident_size() == column.memory_size() > 0
        assert not column.is_spilled()
        assert not Serie.empty(price()).is_spilled()

    def test_a_bound_of_zero_spills_whole_and_a_write_brings_the_rows_back(self) -> None:
        column = spill_prices(1_024)
        before = copy.copy(column)
        column.spill(byte_size=0)
        assert column.is_spilled()
        assert column.resident_size() == 0
        assert column.memory_size() == before.memory_size()
        assert column == before
        assert column.scalar(7).as_py() == 7
        # The copy taken before keeps its heap bytes.
        assert not before.is_spilled()
        # Every read reaches the mapping: an export, a cast, a sort.
        assert column.into_arrow_array().equals(before.into_arrow_array())
        assert column.cast(DataType("float64")).as_py()[:2] == [0.0, 1.0]
        assert column.into_sorted(descending=True).scalar(0).as_py() == 1_023
        # A write brings the rows it touches back to the heap, once.
        column.push(1_024)
        assert not column.is_spilled()
        assert column[-1].as_py() == 1_024

    def test_never_spills_nothing_and_a_run_is_never_spilled(self) -> None:
        column = spill_prices(1_024)
        column.spill(SpillOptions(SpillOptions.NEVER))
        assert not column.is_spilled()
        column.spill(byte_size=column.resident_size())
        assert not column.is_spilled()
        column.spill(byte_size=column.resident_size() - 1)
        assert column.is_spilled()
        run = Serie([1, 2, 3])
        run.spill(byte_size=0)
        assert not run.is_spilled()
        assert run.resident_size() == run.memory_size()

    def test_a_record_spills_its_heaviest_child_first_and_keeps_its_class(self) -> None:
        records = Serie.from_scalars(
            Field("row", "struct<big: utf8 not null, small: int8 not null>", nullable=False),
            [["x" * 64, index % 100] for index in range(256)],
        )
        small = records.child("small")
        assert small is not None
        records.spill(byte_size=small.memory_size())
        assert type(records) is StructSerie
        big, kept = records.child("big"), records.child("small")
        assert big is not None and kept is not None
        assert big.is_spilled() and not kept.is_spilled()
        assert not records.is_spilled()
        assert records.resident_size() == kept.memory_size()
        assert records.as_py()[3] == {"big": "x" * 64, "small": 3}

    def test_as_spilled_answers_the_serie_and_into_spilled_a_copy_of_its_class(self) -> None:
        column = spill_prices(1_024)
        copied = column.into_spilled(byte_size=0)
        assert copied.is_spilled() and copied.resident_size() == 0
        assert not column.is_spilled()
        assert copied == column
        assert column.as_spilled(SpillOptions(0)) is column
        assert column.is_spilled() and column == copied
        records = Serie.from_scalars(
            Field("row", "struct<big: utf8 not null>", nullable=False),
            [["x" * 64] for _ in range(64)],
        )
        spilled = records.into_spilled(byte_size=0)
        assert type(spilled) is StructSerie
        assert spilled.as_py() == records.as_py()
        assert records.as_spilled(byte_size=SpillOptions.NEVER) is records
        assert not records.is_spilled()
        # A run spills nothing either way, and a refused keyword spills nothing.
        run = Serie([1, 2, 3])
        assert not run.into_spilled(byte_size=0).is_spilled()
        fresh = spill_prices(64)
        with pytest.raises(TypeError):
            fresh.into_spilled(byte_size="0")  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            fresh.as_spilled(byte_size="0")  # type: ignore[arg-type]
        assert not fresh.is_spilled()


class TestSortBy:
    def test_sort_indices_by_keys_a_record_by_its_terms_and_a_column_by_itself(self) -> None:
        quotes = keyed_quotes()
        order = quotes.sort_indices_by("venue, price desc nulls first")
        assert order.field == Field("index", "uint32", nullable=False)
        assert order.as_py() == [2, 1, 3, 0]
        prices = Serie.from_scalars(price(), [3, 1, 2])
        assert prices.sort_indices_by("price desc").as_py() == [0, 2, 1]

    def test_every_spelling_of_the_keys_is_one_order(self) -> None:
        quotes = keyed_quotes()
        expected = quotes.sort_indices_by("venue, price desc nulls first").as_py()
        for by in (
            ["venue", "price desc nulls first"],
            ("venue", "price desc nulls first"),
            [
                {"term": "venue"},
                {"term": "price", "descending": True, "nulls_first": True},
            ],
            ["venue", {"term": "price", "descending": True, "nulls_first": True}],
        ):
            assert quotes.sort_indices_by(by).as_py() == expected, by
        # A selector is every projection, ascending with nulls last.
        assert quotes.sort_indices_by(Selector("venue, price")).as_py() == (
            quotes.sort_indices_by("venue, price").as_py()
        )
        assert quotes.sort_indices_by({"term": "price", "descending": True}).as_py() == [
            3,
            1,
            0,
            2,
        ]

    def test_into_sort_by_answers_a_new_serie_declaring_its_order(self) -> None:
        quotes = quote_rows([("XNYS", 1), ("XNAS", 2), ("XNYS", 3)])
        assert quotes.declared_order() is None
        sorted_ = quotes.into_sort_by("venue desc, price desc")
        assert type(sorted_) is StructSerie
        assert sorted_.as_py() == [
            {"venue": "XNYS", "price": 3},
            {"venue": "XNYS", "price": 1},
            {"venue": "XNAS", "price": 2},
        ]
        assert sorted_.declared_order() == ["venue desc", "price desc"]
        # The serie is as it was.
        assert quotes.as_py()[0] == {"venue": "XNYS", "price": 1}

    def test_the_declared_order_is_what_sort_by_states_and_a_write_keeps_or_clears(
        self,
    ) -> None:
        quotes = quote_rows([("XNYS", 2), ("XNAS", 1)])
        sorted_ = quotes.into_sort_by("venue, price desc")
        assert sorted_.declared_order() == ["venue", "price desc"]
        field = sorted_.field
        assert field is not None
        assert field.metadata["SORT:by"] == '["venue","price desc"]'
        # What the declaration states is answered without a pass.
        assert sorted_.sort_indices_by("venue").as_py() == [0, 1]
        held = copy.copy(sorted_)
        held.push(["XNYS", 1])
        assert held.declared_order() == ["venue", "price desc"]
        held.push(["AAAA", 0])
        assert held.declared_order() is None
        # A whole-row sort declares every column.
        whole = quotes.into_sorted()
        assert whole.declared_order() == ["venue", "price"]
        assert Serie([1, 2]).declared_order() is None
        assert spill_prices(3).declared_order() is None

    def test_as_sort_by_sorts_in_place_and_a_refusal_leaves_the_serie(self) -> None:
        prices = Serie.from_scalars(price(), [2, 3, 1])
        assert prices.as_sort_by("price desc").as_reversed() is prices
        assert prices.as_py() == [1, 2, 3]
        with pytest.raises(ValueError, match="tier"):
            prices.as_sort_by("tier")
        assert prices.as_py() == [1, 2, 3]

    def test_refusals_come_before_any_row_and_name_what_failed(self) -> None:
        quotes = keyed_quotes()
        with pytest.raises(ValueError, match="tier"):
            quotes.sort_indices_by("tier")
        with pytest.raises(ValueError):
            quotes.into_sort_by("venue desc desc")
        with pytest.raises(ValueError, match="sorts by no term"):
            Serie([3, 1]).sort_indices_by("price")
        with pytest.raises(ValueError):
            quotes.sort_indices_by([])
        with pytest.raises(ValueError, match="term"):
            quotes.sort_indices_by([{"descending": True}])


def join_trades() -> Serie:
    """`(id, name)`: a key repeated twice, a null key, a key the right lacks."""
    return Serie.from_scalars(
        Field("trade", "struct<id: int64, name: utf8 not null>", nullable=False),
        [[1, "a"], [2, "b"], [2, "b2"], [None, "n"], [4, "d"]],
    )


def join_values() -> Serie:
    """`(id, value)`: a key repeated twice, a null key, a key the left lacks,
    the matched key last."""
    return Serie.from_scalars(
        Field("value", "struct<id: int64, value: int64 not null>", nullable=False),
        [[2, 20], [2, 21], [3, 30], [None, 99], [1, 10]],
    )


def nullables(serie: Serie) -> list[bool]:
    return [child.field.nullable for child in serie.children() if child.field is not None]


def pairs(name: str, rows: int, keys: int) -> Serie:
    """`(id, value)` rows: `id` cycles through `0..keys`, `value` counts up."""
    value = "left_value" if name == "l" else "right_value"
    return Serie.from_scalars(
        Field(name, f"struct<id: int64 not null, {value}: int64 not null>", nullable=False),
        [[index % keys, index] for index in range(rows)],
    )


def counted_pairs(name: str, batches: int, rows: int, pulled: list[int]) -> SerieReader:
    """A stream of `batches` batches of `pairs` rows, counting its pulls."""
    value = "left_value" if name == "l" else "right_value"
    schema = pa.schema(
        [pa.field("id", pa.int64(), nullable=False), pa.field(value, pa.int64(), nullable=False)]
    )

    def produce() -> Iterator[pa.RecordBatch]:
        for batch in range(batches):
            pulled.append(batch)
            start = batch * rows
            yield pa.record_batch(
                [
                    pa.array([index % 3 for index in range(start, start + rows)], pa.int64()),
                    pa.array(list(range(start, start + rows)), pa.int64()),
                ],
                schema=schema,
            )

    root = Field(name, f"struct<id: int64 not null, {value}: int64 not null>", nullable=False)
    return SerieReader.from_arrow_reader(pa.RecordBatchReader.from_batches(schema, produce()), root)


class TestJoin:
    def test_the_rustdoc_example_joins_trades_with_their_venues(self) -> None:
        trades = Serie.from_scalars(
            Field("trade", "struct<id: int64 not null, venue: utf8 not null>", nullable=False),
            [[1, "XNAS"], [2, "XNYS"]],
        )
        venues = Serie.from_scalars(
            Field("venue", "struct<venue: utf8 not null, city: utf8 not null>", nullable=False),
            [["XNAS", "New York"]],
        )
        joined = trades.join_with(venues, "venue", "left")
        assert isinstance(joined, StructSerie)
        assert joined.names == ["id", "venue", "city"]
        assert len(joined) == 2
        assert joined.as_py()[1] == {"id": 2, "venue": "XNYS", "city": None}
        # The other side is anything `Serie.from_` reads.
        batch = pa.record_batch({"venue": ["XNAS"], "city": ["New York"]})
        assert trades.join_with(batch, "venue", "left").as_py() == joined.as_py()

    def test_an_inner_join_multiplies_duplicates_and_skips_null_keys(self) -> None:
        joined = join_trades().join_with(join_values(), "id", build="right")
        assert isinstance(joined, StructSerie)
        assert joined.names == ["id", "name", "value"]
        assert joined.field is not None and joined.field.name == "trade"
        assert [tuple(row.values()) for row in joined.as_py()] == [
            (1, "a", 10),
            (2, "b", 20),
            (2, "b", 21),
            (2, "b2", 20),
            (2, "b2", 21),
        ]

    def test_each_kind_keeps_the_rows_and_the_nullability_it_states(self) -> None:
        matched = [(1, "a", 10), (2, "b", 20), (2, "b", 21), (2, "b2", 20), (2, "b2", 21)]
        left = join_trades().join_with(join_values(), "id", "left", build="right")
        assert nullables(left) == [True, False, True]
        assert [tuple(row.values()) for row in left.as_py()] == [
            *matched,
            (None, "n", None),
            (4, "d", None),
        ]
        right = join_trades().join_with(join_values(), "id", "right", build="right")
        assert nullables(right) == [True, True, False]
        assert [tuple(row.values()) for row in right.as_py()] == [
            *matched,
            (3, None, 30),
            (None, None, 99),
        ]
        full = join_trades().join_with(join_values(), "id", "full", build="right")
        assert nullables(full) == [True, True, True]
        assert len(full) == 9
        semi = join_trades().join_with(join_values(), "id", "semi", build="right")
        assert semi.field == join_trades().field
        assert [tuple(row.values()) for row in semi.as_py()] == [(1, "a"), (2, "b"), (2, "b2")]
        anti = join_trades().join_with(join_values(), "id", "anti", build="right")
        assert [tuple(row.values()) for row in anti.as_py()] == [(None, "n"), (4, "d")]

    def test_a_plain_column_joins_as_the_one_child_of_a_row_record(self) -> None:
        ids = Serie.from_scalars(Field("id", "int64", nullable=False), [2, 1, 9])
        joined = ids.join_with(join_values(), "id", build="right")
        assert isinstance(joined, StructSerie)
        assert joined.field is not None and joined.field.name == "row"
        assert joined.names == ["id", "value"]
        assert [tuple(row.values()) for row in joined.as_py()] == [(2, 20), (2, 21), (1, 10)]

    def test_refusals_come_before_any_row_and_name_what_failed(self) -> None:
        with pytest.raises(ValueError, match="run"):
            Serie([1]).join_with(join_values(), "id")
        with pytest.raises(ValueError, match="venue"):
            join_trades().join_with(join_values(), "venue")
        with pytest.raises(ValueError, match="share no datatype"):
            join_trades().join_with(join_values(), "name = value")

    def test_the_three_verbs_agree_on_the_rows(self) -> None:
        left, right = pairs("l", 7, 3), pairs("r", 5, 2)
        held = left.join_with(right, "id", "full")
        chunked = ChunkedSerie.from_serie(left).join_with(
            ChunkedSerie.from_serie(right), "id", "full"
        )
        streamed = SerieReader.from_serie(left).join_with(right, "id", "full")
        assert type(streamed) is SerieReader
        assert held.rows() == chunked.rows()
        assert held.rows() == [row for batch in streamed for row in batch.rows()]


class TestReaderOrderSpillAndJoin:
    def test_into_sorted_drains_the_stream_and_keeps_its_root(self) -> None:
        quotes = quote_rows([("XNYS", 2), ("XNAS", 1), ("XNYS", 1)])
        sorted_ = SerieReader.from_serie(quotes).into_sorted(descending=True)
        assert sorted_.field.name == "quote"
        assert [row for batch in sorted_ for row in batch.as_py()] == [
            {"venue": "XNYS", "price": 2},
            {"venue": "XNYS", "price": 1},
            {"venue": "XNAS", "price": 1},
        ]

    def test_into_sort_by_reads_the_keys_before_the_stream(self) -> None:
        quotes = quote_rows([("XNYS", 2), ("XNAS", 1), ("XNYS", 1)])
        sorted_ = SerieReader.from_serie(quotes).into_sort_by("venue, price desc")
        assert [row for batch in sorted_ for row in batch.as_py()] == [
            {"venue": "XNAS", "price": 1},
            {"venue": "XNYS", "price": 2},
            {"venue": "XNYS", "price": 1},
        ]
        pulled: list[int] = []
        stream = counted_pairs("l", 2, 3, pulled)
        # Text that does not parse leaves the reader usable; a key the root
        # refuses spends it, with no batch pulled.
        with pytest.raises(ValueError):
            stream.into_sort_by("id desc desc")
        with pytest.raises(ValueError, match="tier"):
            stream.into_sort_by("tier")
        assert pulled == []
        with pytest.raises(ValueError, match="handed over"):
            stream.into_sort_by("id")
        # Every batch is pulled before the first sorted one is answered.
        pulled.clear()
        ordered = counted_pairs("l", 2, 3, pulled).into_sort_by("left_value desc")
        assert pulled == [0, 1]
        assert [row["left_value"] for batch in ordered for row in batch.as_py()] == [
            5,
            4,
            3,
            2,
            1,
            0,
        ]

    def test_a_held_reader_spills_the_records_it_holds_and_a_stream_holds_none(self) -> None:
        column = quote_rows([("XNYS", index) for index in range(512)])
        reader = SerieReader.from_serie(column)
        assert reader.resident_size() == column.resident_size()
        assert not reader.is_spilled()
        reader.spill(byte_size=0)
        assert reader.is_spilled()
        assert reader.resident_size() == 0
        record = next(reader)
        assert record.is_spilled()
        assert record == column
        # Drained, it holds nothing and is not spilled.
        assert list(reader) == []
        assert reader.resident_size() == 0
        assert not reader.is_spilled()
        pulled: list[int] = []
        stream = counted_pairs("l", 1, 3, pulled)
        stream.spill(byte_size=0)
        assert stream.resident_size() == 0
        assert not stream.is_spilled()
        assert not next(stream).is_spilled()
        # A reader handed over holds nothing, and refuses a write.
        stream.into_arrow_reader()
        assert stream.resident_size() == 0
        assert not stream.is_spilled()
        with pytest.raises(ValueError, match="handed over"):
            stream.spill(byte_size=0)

    def test_a_reader_spills_in_a_chain_and_into_a_reader_that_takes_its_records(self) -> None:
        column = quote_rows([("XNYS", index) for index in range(512)])
        reader = SerieReader.from_serie(column)
        assert reader.as_spilled(byte_size=0) is reader
        assert reader.is_spilled()
        assert next(reader) == column
        moved = SerieReader.from_serie(column)
        # A refused keyword is read before the reader is taken.
        with pytest.raises(TypeError):
            moved.into_spilled(byte_size="0")  # type: ignore[arg-type]
        spilled = moved.into_spilled(SpillOptions(0))
        assert spilled.is_spilled() and spilled.field == moved.field
        assert [record.is_spilled() for record in spilled] == [True]
        # The reader it was taken from is handed over, and refuses both.
        assert moved.resident_size() == 0
        with pytest.raises(ValueError, match="handed over"):
            moved.into_spilled(byte_size=0)
        with pytest.raises(ValueError, match="handed over"):
            moved.as_spilled(byte_size=0)
        # A stream holds no record between pulls, so it spills none.
        pulled: list[int] = []
        stream = counted_pairs("l", 1, 3, pulled).into_spilled(byte_size=0)
        assert pulled == []
        assert not stream.is_spilled()
        assert not next(stream).is_spilled()

    def test_a_stream_probes_one_batch_at_a_time_and_collects_nothing(self) -> None:
        pulled: list[int] = []
        joined = counted_pairs("l", 3, 2, pulled).join_with(pairs("r", 4, 2), "id")
        # The held side is built; the stream is not pulled until asked.
        assert pulled == []
        assert joined.field.name == "l"
        first = next(joined)
        assert pulled == [0]
        rest = list(joined)
        assert pulled == [0, 1, 2]
        assert len(first) + sum(len(batch) for batch in rest) == 8

    def test_a_stream_against_a_stream_holds_the_right_one(self) -> None:
        left_pulls: list[int] = []
        right_pulls: list[int] = []
        joined = counted_pairs("l", 2, 3, left_pulls).join_with(
            counted_pairs("r", 2, 3, right_pulls), "id", "left"
        )
        # The right stream is the build side: drained before the left is pulled.
        assert right_pulls == [0, 1]
        assert left_pulls == []
        assert sum(len(batch) for batch in joined) == 12
        assert left_pulls == [0, 1]

    def test_a_refused_argument_leaves_both_sides_and_a_spent_reader_takes_neither(
        self,
    ) -> None:
        stream = SerieReader.from_serie(pairs("l", 4, 2))
        other = SerieReader.from_serie(pairs("r", 4, 2))
        with pytest.raises(ValueError, match="cross"):
            stream.join_with(other, "id", "cross")
        with pytest.raises(ValueError, match="build"):
            stream.join_with(other, "id", build="middle")
        joined = stream.join_with(other, "id")
        assert sum(len(batch) for batch in joined) == 8
        with pytest.raises(ValueError, match="handed over"):
            stream.join_with(SerieReader.from_serie(pairs("r", 4, 2)), "id")
        spare = SerieReader.from_serie(pairs("r", 4, 2))
        with pytest.raises(ValueError, match="handed over"):
            stream.join_with(spare, "id")
        # The other reader was not taken by the refused join.
        assert sum(len(batch) for batch in spare) == 4
