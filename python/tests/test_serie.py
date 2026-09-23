"""``Serie``: many values, as a schema-free run or as a column's Arrow buffers.

Pins ``python/yggdryl/serie.py`` and the ``python/src/serie.rs`` redirects
under it: every verb answers what the core ``Serie`` answers, and Arrow
crosses in and out by sharing buffers.
"""

from __future__ import annotations

import copy
import pickle
from collections.abc import Iterator

import pyarrow as pa
import pytest

from yggdryl import (
    DataType,
    Field,
    FixedSizeListSerie,
    LargeListSerie,
    LargeListViewSerie,
    ListSerie,
    ListViewSerie,
    MapSerie,
    Scalar,
    Serie,
    SerieReader,
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
        assert column.dtype == DataType("list<item: int64 not null>")
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
        assert isinstance(legs, ListSerie)
        legs.push([4, 5, 6])
        legs[0] = []
        assert legs.offsets == [0, 0, 1, 4]
        assert legs.as_py() == [[], [3], [4, 5, 6]]
        assert type(copy.copy(legs)) is ListSerie
        assert type(pickle.loads(pickle.dumps(legs))) is ListSerie


class TestScalar:
    def test_a_scalar_sequence_holds_a_serie_and_a_serie_is_a_scalar(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        value = column.into_scalar()
        assert value.kind == "list"
        assert value.as_serie() == column
        assert Scalar.from_(column) == value
        assert Scalar.from_(1).as_serie() is None
        assert column.into_run() == column
        assert not column.into_run().is_column


class TestNested:
    def test_each_list_layout_is_its_own_class_and_lends_its_cut(self) -> None:
        rows = [[1, 2], None, [3]]
        legs = Serie.from_arrow_array(pa.array(rows, pa.list_(pa.int64())))
        assert isinstance(legs, ListSerie) and isinstance(legs, Serie)
        assert legs.offsets == [0, 2, 2, 3]
        assert legs.range(0) == (0, 2)
        assert legs.range(3) is None
        first = legs.row(0)
        assert first is not None and first.as_py() == [1, 2]
        items = legs.items()
        assert items is not None and items.as_py() == [1, 2, 3]
        assert legs.as_py() == rows

        large = Serie.from_arrow_array(pa.array(rows, pa.large_list(pa.int64())))
        assert isinstance(large, LargeListSerie)
        assert large.offsets == [0, 2, 2, 3]
        assert large == legs

        view = Serie.from_arrow_array(pa.array(rows, pa.list_view(pa.int64())))
        assert isinstance(view, ListViewSerie)
        assert view.sizes == [2, 0, 1]
        assert view.row(2) == Serie([3])

        large_view = Serie.from_arrow_array(pa.array(rows, pa.large_list_view(pa.int64())))
        assert isinstance(large_view, LargeListViewSerie)
        assert large_view.as_py() == rows

        fixed = Serie.from_arrow_array(pa.array([[1, 2], [3, 4]], pa.list_(pa.int64(), 2)))
        assert isinstance(fixed, FixedSizeListSerie)
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
        assert isinstance(column, ListSerie)
        records = column.items()
        assert isinstance(records, StructSerie)
        sizes = records.child("sizes")
        assert isinstance(sizes, ListSerie)
        assert sizes.as_py() == [[100, 200], [], [5]]
        assert isinstance(orders.get_child_by_path("legs.sizes"), ListSerie)
        assert [type(child) for child in orders.children()] == [Serie, ListSerie]

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
