"""``Serie``: many values, as a schema-free run or as a column's Arrow buffers.

Pins ``python/yggdryl/serie.py`` and the ``python/src/serie.rs`` redirects
under it: every verb answers what the core ``Serie`` answers, and Arrow
crosses in and out by sharing buffers.
"""

from __future__ import annotations

import copy
import pickle

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

    def test_an_array_of_another_layout_is_refused(self) -> None:
        with pytest.raises(ValueError, match="price"):
            Serie.from_arrow_array(pa.array([1, 2], pa.int32()), price())

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

    def test_a_run_has_no_buffers(self) -> None:
        with pytest.raises(ValueError):
            Serie([1, 2]).into_arrow_array()


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
