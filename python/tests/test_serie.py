"""``Serie``: many values, as a schema-free run or as a column's Arrow buffers.

Pins ``python/yggdryl/serie.py`` and the ``python/src/serie.rs`` redirects
under it: every verb answers what the core ``Serie`` answers, and Arrow
crosses in and out by sharing buffers. It is also the one boundary a foreign
columnar object crosses: a held column is a ``Serie``, a stream is a
``SerieReader``, and nothing else.
"""

from __future__ import annotations

import copy
import importlib
import pathlib
import pickle
import threading
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor

import numpy as np
import pyarrow as pa
import pytest

import yggdryl
from yggdryl import (
    ArrowCastPlan,
    ChunkedSerie,
    DataType,
    Field,
    IOBase,
    FixedSizeSerieSerie,
    LargeSerieSerie,
    LargeSerieViewSerie,
    MapSerie,
    Scalar,
    Serie,
    SerieReader,
    SerieSerie,
    SerieViewSerie,
    StructSerie,
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

    def test_an_absent_row_is_repaired_by_default_and_refused_when_strict(self) -> None:
        absent = pa.array([1, None], pa.int64())
        assert Serie.from_arrow_array(absent, price()).as_py() == [1, 0]
        with pytest.raises(ValueError, match="price"):
            Serie.from_arrow_array(absent, price(), nullability="strict")

    def test_a_refused_value_is_null_when_safe_and_raised_otherwise(self) -> None:
        wide = pa.array([1, 300], pa.int64())
        narrow = Field("price", "int8")
        assert Serie.from_arrow_array(wide, narrow).as_py() == [1, None]
        with pytest.raises(ValueError):
            Serie.from_arrow_array(wide, narrow, safe=False)
        with pytest.raises(ValueError, match="nullability"):
            Serie.from_arrow_array(wide, narrow, nullability="lenient")

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
            Serie.from_arrow_batch(quotes(), strict, nullability="strict")

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

        # A datatype is the required column named `value` it declares, so the
        # absent row is repaired - or refused when asked to be strict.
        typed = column.cast(DataType("int64"))
        assert typed.field == Field("value", "int64", nullable=False)
        assert typed.as_py() == [1, 0]
        with pytest.raises(ValueError):
            column.cast(DataType("int64"), nullability="strict")

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
            SerieReader.from_arrow_reader(self.stream(), strict, nullability="strict")

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
    def test_the_arrow_shape_wrapper_is_retired(self) -> None:
        assert not hasattr(yggdryl, "ArrowScalar")
        with pytest.raises(ModuleNotFoundError):
            importlib.import_module("yggdryl.arrow")

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
        handle.write_arrow(records)
        assert [len(serie) for serie in handle.read_arrow()] == [2, 2]
        assert Serie.from_(handle.read_arrow()) == records


class TestHandles:
    @pytest.mark.parametrize(
        "name", ["quotes.json", "quotes.jsonl", "quotes.yaml", "quotes.toml"]
    )
    def test_every_structured_format_round_trips_a_table(
        self, tmp_path: pathlib.Path, name: str
    ) -> None:
        handle = IOBase(tmp_path / name)
        handle.write_arrow(quote_table())

        read = handle.read_arrow(field=quote_root())
        assert isinstance(read, SerieReader)
        assert Serie.from_(read).as_py() == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]

    def test_a_record_encoding_answers_its_stream(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow(quote_table())

        read = handle.read_arrow()
        assert isinstance(read, SerieReader)
        assert read.into_arrow_reader().read_all().num_rows == 2

    def test_a_document_is_written_whole_so_only_an_overwrite_applies(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.json")
        with pytest.raises(ValueError, match="overwrite"):
            handle.write_arrow(quote_table(), "append")

    def test_a_record_encoding_appends(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow(quote_table())
        handle.write_arrow(quote_table(), "append")
        assert handle.read_arrow().into_arrow_reader().read_all().num_rows == 4

    def test_a_frame_a_serie_and_a_reader_reach_the_same_publication_path(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.jsonl")
        handle.write_arrow(quote_table().to_pandas())
        assert len(Serie.from_(handle.read_arrow(field=quote_root()))) == 2

        column = IOBase(tmp_path / "column.arrows")
        column.write_arrow(Serie.from_(quote_table()))
        copied = IOBase(tmp_path / "copied.arrows")
        copied.write_arrow(column.read_arrow())
        assert copied.read_arrow().into_arrow_reader().read_all().equals(quote_table())


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
