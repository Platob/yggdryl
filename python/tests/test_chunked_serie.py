"""``ChunkedSerie``: many columns under one field, held apart.

Pins ``python/src/chunked_serie.rs``: every verb answers what the core
``ChunkedSerie`` answers, a ``pyarrow.ChunkedArray`` crosses in and out one
chunk at a time with its buffers shared, a ``pyarrow.Table`` is one chunk per
batch, and ``from_`` reads ``Serie.from_``'s recognition ladder as chunks.
"""

from __future__ import annotations

import copy
import pathlib
import pickle

import numpy as np
import pyarrow as pa
import pytest

import yggdryl
from yggdryl import (
    ChunkedSerie,
    DataType,
    Field,
    IOBase,
    Scalar,
    Serie,
    SerieReader,
    SerieSerie,
    StructSerie,
)


def price() -> Field:
    return Field("price", "int64", nullable=False)


def quotes(ids: list[int], symbols: list[str]) -> pa.RecordBatch:
    return pa.record_batch(
        [pa.array(ids, pa.int64()), pa.array(symbols)],
        schema=pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("symbol", pa.utf8(), nullable=False),
            ]
        ),
    )


def table() -> pa.Table:
    return pa.Table.from_batches([quotes([1, 2], ["AAPL", "MSFT"]), quotes([3], ["AMD"])])


def prices() -> ChunkedSerie:
    return ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1, 2], [3]]), price())


def buffer_locations(array: pa.Array) -> list[tuple[int, int] | None]:
    return [
        None if buffer is None else (buffer.address, buffer.size)
        for buffer in array.buffers()
    ]


class TestChunkedArray:
    def test_a_chunked_array_is_its_chunks_and_goes_back_out_sharing_their_buffers(
        self,
    ) -> None:
        source = pa.chunked_array([[1, 2], [3]])
        chunked = ChunkedSerie.from_arrow_chunked_array(source)
        assert (chunked.num_chunks, len(chunked)) == (2, 3)
        assert chunked.field == Field("item", "int64", nullable=False)
        assert chunked.dtype == DataType("serie<item: int64 not null>")
        for held, original in zip(chunked.chunks, source.chunks):
            assert buffer_locations(held.into_arrow_array()) == buffer_locations(original)

        back = chunked.into_arrow_chunked_array()
        assert isinstance(back, pa.ChunkedArray)
        assert back.num_chunks == 2
        assert back.equals(source)
        for exported, original in zip(back.chunks, source.chunks):
            assert buffer_locations(exported) == buffer_locations(original)

    def test_an_absent_row_in_any_chunk_makes_the_column_nullable(self) -> None:
        chunked = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1], [None, 3]]))
        assert chunked.field == Field("item", "int64", nullable=True)
        assert chunked.null_count() == 1
        assert chunked.as_py() == [1, None, 3]

    def test_arrays_are_cast_into_a_declared_field_by_one_plan(self) -> None:
        arrays = [pa.array([1], pa.int32()), pa.array([2, 3], pa.int32())]
        chunked = ChunkedSerie.from_arrow_chunked_array(arrays, price())
        assert chunked.field == price()
        assert chunked.num_chunks == 2
        assert chunked.into_arrow_chunked_array().type == pa.int64()
        assert chunked.as_py() == [1, 2, 3]

    def test_one_array_is_its_one_chunk(self) -> None:
        array = pa.array([1, 2], pa.int64())
        chunked = ChunkedSerie.from_arrow_chunked_array(array, price())
        assert chunked.num_chunks == 1
        assert buffer_locations(chunked.chunks[0].into_arrow_array()) == buffer_locations(array)

    def test_a_chunk_laid_out_unlike_the_first_is_refused_naming_it(self) -> None:
        with pytest.raises(ValueError, match='chunk 1 of "item" is utf8, and the first chunk int64'):
            ChunkedSerie.from_arrow_chunked_array([pa.array([1]), pa.array(["a"])])

    def test_chunks_naming_their_list_items_otherwise_are_one_type(self) -> None:
        # pyarrow holds `item` and `element` lists as one type, and so does
        # the chunked serie: the second chunk is the same buffers relabelled.
        named = pa.chunked_array(
            [
                pa.array([[1]], pa.list_(pa.int64())),
                pa.array([[2]], pa.list_(pa.field("element", pa.int64()))),
            ]
        )
        assert ChunkedSerie.from_(named).as_py() == [[1], [2]]
        assert Serie.from_(named).as_py() == [[1], [2]]

    def test_a_join_arrow_would_panic_on_is_refused(self) -> None:
        # View text vocabularies are gathered whole rather than merged, and
        # two hundred values are past the largest int8 key.
        def venues(start: int) -> pa.Array:
            names = pa.array([f"v{at}" for at in range(start, start + 100)], pa.string_view())
            return pa.DictionaryArray.from_arrays(pa.array(range(100), pa.int8()), names)

        chunked = pa.chunked_array([venues(0), venues(100)])
        message = "gathers 200 dictionary values at \\$, past the largest int8 key"
        with pytest.raises(ValueError, match=message):
            ChunkedSerie.from_(chunked).into_serie()
        with pytest.raises(ValueError, match=message):
            Serie.from_(chunked)

    def test_a_value_the_field_refuses_is_refused_when_not_safe(self) -> None:
        text = pa.chunked_array([["1"], ["not a number"]])
        assert ChunkedSerie.from_arrow_chunked_array(text, "size: int64").as_py() == [1, None]
        with pytest.raises(ValueError, match="Cannot cast"):
            ChunkedSerie.from_arrow_chunked_array(text, "size: int64", safe=False)

    def test_a_chunkless_chunked_array_is_still_typed(self) -> None:
        empty = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([], pa.int64()))
        assert (empty.num_chunks, len(empty)) == (0, 0)
        assert empty.field == Field("item", "int64", nullable=False)
        back = empty.into_arrow_chunked_array()
        assert back.type == pa.int64() and back.num_chunks == 0

        # A list of no array states no type, so it needs a field.
        with pytest.raises(ValueError, match="names no field"):
            ChunkedSerie.from_arrow_chunked_array([])
        assert ChunkedSerie.from_arrow_chunked_array([], price()).field == price()

    def test_anything_else_is_refused_by_name(self) -> None:
        with pytest.raises(TypeError, match="ChunkedArray"):
            ChunkedSerie.from_arrow_chunked_array(7)


class TestConstruction:
    def test_series_are_held_under_the_first_field_or_the_declared_one(self) -> None:
        first = Serie.from_scalars(price(), [1, 2])
        second = Serie.from_scalars(Field("price", "int64"), [None])
        agreed = ChunkedSerie.from_series([first, second])
        assert agreed.field == Field("price", "int64", nullable=True)
        assert agreed.num_chunks == 2
        assert agreed.as_py() == [1, 2, None]

        cast = ChunkedSerie.from_series([Serie.from_scalars(Field("p", "int32"), [1])], price())
        assert cast.field == price()
        assert cast.chunks[0].into_arrow_array().type == pa.int64()

        # With no field, the chunks are one datatype in pieces: another
        # datatype is refused naming the chunk, and only a field casts.
        narrow = Serie.from_scalars(Field("price", "int32"), [3])
        with pytest.raises(ValueError, match='chunk 1 of "price" is int32, and the first chunk int64'):
            ChunkedSerie.from_series([first, narrow])
        assert ChunkedSerie.from_series([first, narrow], price()).as_py() == [1, 2, 3]

    def test_a_chunk_already_under_the_field_is_held_as_it_stands(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        chunked = ChunkedSerie.from_series([column, column], price())
        for chunk in chunked.chunks:
            assert buffer_locations(chunk.into_arrow_array()) == buffer_locations(
                column.into_arrow_array()
            )

    def test_no_chunk_names_no_field(self) -> None:
        with pytest.raises(ValueError, match="names no field"):
            ChunkedSerie.from_series([])
        empty = ChunkedSerie.from_series([], price())
        assert empty.field == price() and empty.num_chunks == 0

    def test_a_run_names_no_field_and_is_refused(self) -> None:
        with pytest.raises(ValueError, match="run"):
            ChunkedSerie.from_serie(Serie([1, 2]))
        with pytest.raises(ValueError, match="run"):
            ChunkedSerie.from_series([Serie([1, 2])])
        with pytest.raises(ValueError, match="run"):
            ChunkedSerie.from_(Serie([1, 2]))

    def test_a_chunk_that_is_not_a_serie_is_refused_by_name(self) -> None:
        with pytest.raises(TypeError, match="Serie"):
            ChunkedSerie.from_series([pa.array([1])])

    def test_a_foreign_chunk_the_field_refuses_is_refused_when_not_safe(self) -> None:
        text = Serie.from_arrow_array(pa.array(["not a number"]))
        assert ChunkedSerie.from_series([text], Field("size", "int64")).as_py() == [None]
        with pytest.raises(ValueError, match="Cannot cast"):
            ChunkedSerie.from_series([text], Field("size", "int64"), safe=False)
        with pytest.raises(ValueError, match="nullability"):
            ChunkedSerie.from_series([text], Field("size", "int64"), nullability="lenient")

    def test_empty_and_from_serie(self) -> None:
        empty = ChunkedSerie.empty(price())
        assert empty.is_empty() and empty.num_chunks == 0
        assert empty.field == price()
        assert empty.into_serie() == Serie.empty(price())

        column = Serie.from_scalars(price(), [1, 2])
        one = ChunkedSerie.from_serie(column)
        assert one.num_chunks == 1
        assert one.field == price()
        assert buffer_locations(one.chunks[0].into_arrow_array()) == buffer_locations(
            column.into_arrow_array()
        )

    def test_there_is_no_constructor(self) -> None:
        with pytest.raises(TypeError):
            ChunkedSerie()  # type: ignore[call-arg]


class TestTable:
    def test_a_table_is_one_chunk_per_batch_and_goes_back_out_as_one(self) -> None:
        source = table()
        chunked = ChunkedSerie.from_arrow_reader(source)
        assert (chunked.num_chunks, len(chunked)) == (2, 3)
        assert chunked.field == Field.from_arrow_schema(source.schema)
        assert all(type(chunk) is StructSerie for chunk in chunked.chunks)
        assert chunked.as_py() == [
            {"id": 1, "symbol": "AAPL"},
            {"id": 2, "symbol": "MSFT"},
            {"id": 3, "symbol": "AMD"},
        ]

        out = chunked.into_arrow_table()
        assert isinstance(out, pa.Table)
        assert out.equals(source)
        assert [batch.num_rows for batch in out.to_batches()] == [2, 1]
        reader = chunked.into_arrow_reader()
        assert isinstance(reader, pa.RecordBatchReader)
        assert len(list(reader)) == 2

    def test_a_tables_column_is_the_child_of_every_batch(self) -> None:
        source = table()
        chunked = ChunkedSerie.from_arrow_reader(source)
        symbols = chunked.child("symbol")
        assert isinstance(symbols, ChunkedSerie)
        assert symbols.num_chunks == 2
        assert symbols.into_arrow_chunked_array().equals(source.column("symbol"))
        ids = chunked.child_at(0)
        assert ids is not None and ids.as_py() == [1, 2, 3]
        assert [child.field.name for child in chunked.children()] == ["id", "symbol"]
        assert chunked.get_child_by_path("symbol") == symbols
        assert chunked.child("venue") is None
        assert chunked.child_at(2) is None

        # An empty table still answers its columns, from its field.
        empty = ChunkedSerie.empty(Field.from_arrow_schema(source.schema))
        column = empty.child("symbol")
        assert column is not None and column.field == Field("symbol", "utf8", nullable=False)

    def test_a_serie_column_lends_its_items_in_every_chunk(self) -> None:
        legs = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[[1, 2]], [[3], []]]))
        assert all(isinstance(chunk, SerieSerie) for chunk in legs.chunks)
        items = legs.items()
        assert items is not None and items.as_py() == [1, 2, 3]
        assert prices().items() is None

    def test_a_root_casts_every_batch_by_one_plan(self) -> None:
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        chunked = ChunkedSerie.from_arrow_reader(table(), root)
        assert chunked.field == root and chunked.num_chunks == 2
        assert chunked.into_arrow_table().schema.field("id").type == pa.int32()

    def test_any_other_column_is_the_one_column_of_a_row(self) -> None:
        out = prices().into_arrow_table()
        assert out.column_names == ["price"]
        assert [batch.num_rows for batch in out.to_batches()] == [2, 1]

    def test_a_record_chunk_holding_an_absent_row_is_refused_as_a_table(self) -> None:
        records = ChunkedSerie.from_arrow_chunked_array(
            pa.chunked_array([pa.array([{"id": 1}]), pa.array([{"id": 2}, None])])
        )
        assert len(records) == 3
        with pytest.raises(ValueError, match="absent rows"):
            records.into_arrow_reader()
        with pytest.raises(ValueError, match="absent rows"):
            records.into_arrow_table()

    def test_frames_and_numpy_take_every_row_back(self) -> None:
        pandas = pytest.importorskip("pandas")
        polars = pytest.importorskip("polars")

        chunked = ChunkedSerie.from_arrow_reader(table())
        frame = chunked.into_pandas()
        assert isinstance(frame, pandas.DataFrame) and frame.shape == (3, 2)
        polars_frame = chunked.into_polars()
        assert isinstance(polars_frame, polars.DataFrame) and polars_frame.height == 3

        assert prices().into_numpy().tolist() == [1, 2, 3]
        # NumPy has no null mask, so an absent row is allowed to copy into nan.
        nulled = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1], [None]]))
        assert np.isnan(nulled.into_numpy()[1])


class TestRows:
    def test_a_row_is_read_out_of_the_chunk_that_holds_it(self) -> None:
        chunked = prices()
        assert chunked.scalar(2) == Scalar.from_(3)
        assert chunked.get(1) == Scalar.from_(2)
        assert chunked.get(3) is None
        assert not chunked.is_null(0)
        assert chunked.null_count() == 0
        assert chunked.rows() == [Scalar.from_(1), Scalar.from_(2), Scalar.from_(3)]
        assert chunked.as_py() == [1, 2, 3]
        chunk = chunked.chunk(1)
        assert chunk is not None and chunk.as_py() == [3]
        assert chunked.chunk(2) is None

    def test_a_row_past_the_end_is_refused_naming_both_counts(self) -> None:
        with pytest.raises(ValueError, match="row 3 is past the end of 3 rows"):
            prices().scalar(3)
        with pytest.raises(ValueError, match="past the end"):
            prices().is_null(3)

    def test_a_window_keeps_the_chunks_it_reaches(self) -> None:
        chunked = prices()
        window = chunked.slice(1, 2)
        assert window.num_chunks == 2 and window.as_py() == [2, 3]
        assert chunked.slice(0, 2).num_chunks == 1
        assert chunked.slice(1, 0).num_chunks == 0
        assert window.field == price()
        with pytest.raises(ValueError, match="past"):
            chunked.slice(2, 5)

    def test_a_chunked_serie_reads_as_a_python_sequence(self) -> None:
        chunked = prices()
        assert chunked[0] == Scalar.from_(1)
        assert chunked[-1] == Scalar.from_(3)
        tail = chunked[1:]
        assert isinstance(tail, ChunkedSerie) and tail.as_py() == [2, 3]
        assert list(chunked) == [Scalar.from_(1), Scalar.from_(2), Scalar.from_(3)]
        assert 3 in chunked and 9 not in chunked
        assert len(chunked) == 3
        with pytest.raises(IndexError):
            chunked[3]
        with pytest.raises(IndexError):
            chunked[-4]
        with pytest.raises(ValueError, match="step"):
            chunked[::2]
        with pytest.raises(TypeError):
            chunked[True]
        with pytest.raises(TypeError):
            chunked["price"]


class TestIdentity:
    def test_identity_is_the_rows_however_they_are_cut(self) -> None:
        recut = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1], [2, 3]]))
        assert recut == prices()
        assert recut.field != prices().field
        assert prices() < ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1, 2, 4]]))

    def test_a_serie_of_the_same_rows_is_the_same_value_on_either_side(self) -> None:
        assert prices() == Serie.from_scalars(price(), [1, 2, 3])
        assert prices() == Serie([1, 2, 3])
        assert Serie([1, 2, 3]) == prices()
        assert prices() != Serie([1, 2])
        assert prices() < Serie([1, 2, 4])
        assert Serie([1, 2, 4]) > prices()
        assert prices() >= Serie([1, 2, 3])

    def test_any_other_value_is_not_comparable(self) -> None:
        assert prices() != [1, 2, 3]
        with pytest.raises(TypeError):
            prices() < 5  # noqa: B015

    def test_a_chunked_serie_is_unhashable(self) -> None:
        with pytest.raises(TypeError):
            hash(prices())

    def test_repr_is_the_call_that_rebuilds_a_leaf(self) -> None:
        chunked = prices()
        spelled = repr(chunked)
        assert spelled.startswith("ChunkedSerie.from_series([Serie.from_scalars(")
        rebuilt = eval(spelled, vars(yggdryl))
        assert rebuilt == chunked
        assert rebuilt.field == chunked.field
        assert rebuilt.num_chunks == 2

    def test_copies_share_and_pickles_keep_the_chunks_and_the_field(self) -> None:
        chunked = prices()
        shallow = copy.copy(chunked)
        deep = copy.deepcopy(chunked)
        for clone in (shallow, deep):
            assert clone == chunked
            assert buffer_locations(clone.chunks[0].into_arrow_array()) == buffer_locations(
                chunked.chunks[0].into_arrow_array()
            )
        shallow.push_chunk(Serie.from_scalars(price(), [4]))
        assert len(chunked) == 3

        restored = pickle.loads(pickle.dumps(chunked))
        assert restored == chunked
        assert restored.field == price()
        assert restored.num_chunks == 2
        records = ChunkedSerie.from_arrow_reader(table())
        assert pickle.loads(pickle.dumps(records)) == records


class TestWrites:
    def test_a_chunk_is_appended_as_it_stands_or_cast(self) -> None:
        chunked = prices()
        chunked.push_chunk(Serie.from_scalars(price(), [4]))
        chunked.push_chunk(Serie.from_scalars(Field("p", "int32"), [5]))
        assert chunked.num_chunks == 4
        assert chunked.as_py() == [1, 2, 3, 4, 5]
        assert chunked.field == price()
        assert chunked.chunks[3].into_arrow_array().type == pa.int64()

    def test_a_refused_chunk_leaves_it_as_it_was(self) -> None:
        chunked = prices()
        with pytest.raises(ValueError, match="run"):
            chunked.push_chunk(Serie([4]))
        text = Serie.from_arrow_array(pa.array(["not a number"]))
        with pytest.raises(ValueError, match="Cannot cast"):
            chunked.push_chunk(text, safe=False)
        with pytest.raises(TypeError):
            chunked.push_chunk([4])  # type: ignore[arg-type]
        assert chunked.num_chunks == 2 and chunked.as_py() == [1, 2, 3]


class TestCast:
    def test_cast_restates_every_chunk_under_another_field(self) -> None:
        wide = prices().cast(Field("price", "float64"))
        assert wide.num_chunks == 2
        assert wide.field == Field("price", "float64")
        assert wide.as_py() == [1.0, 2.0, 3.0]

        typed = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[1], [None]])).cast(
            DataType("int64")
        )
        assert typed.field == Field("value", "int64", nullable=False)
        assert typed.as_py() == [1, 0]

        # A chunked serie already under the target is itself, chunks shared.
        same = prices().cast(price())
        assert same == prices() and same.num_chunks == 2

    def test_into_serie_is_the_one_join(self) -> None:
        joined = prices().into_serie()
        assert type(joined) is Serie
        assert joined.field == price()
        assert joined.as_py() == [1, 2, 3]
        assert len(joined.into_arrow_array().buffers()) == 2

        # One chunk is itself, sharing its buffers; a record joins as its class.
        column = Serie.from_scalars(price(), [1, 2])
        alone = ChunkedSerie.from_serie(column).into_serie()
        assert buffer_locations(alone.into_arrow_array()) == buffer_locations(
            column.into_arrow_array()
        )
        assert type(ChunkedSerie.from_arrow_reader(table()).into_serie()) is StructSerie


class TestFrom:
    def test_a_chunked_array_is_its_chunks(self) -> None:
        source = pa.chunked_array([[1, 2], [3]])
        chunked = ChunkedSerie.from_(source)
        assert chunked.num_chunks == 2
        for held, original in zip(chunked.chunks, source.chunks):
            assert buffer_locations(held.into_arrow_array()) == buffer_locations(original)
        declared = ChunkedSerie.from_(source, "price: float64 not null")
        assert declared.field == Field("price", "float64", nullable=False)
        assert declared.num_chunks == 2

    def test_a_table_and_a_reader_are_one_chunk_per_batch(self) -> None:
        for source in (table(), table().to_reader()):
            chunked = ChunkedSerie.from_(source)
            assert chunked.num_chunks == 2 and len(chunked) == 3
        root = Field("row", "struct<id:int32,symbol:utf8>", nullable=False)
        assert ChunkedSerie.from_(table(), root).field == root

    def test_a_frame_is_one_chunk_per_batch_and_a_series_one_chunk(self) -> None:
        pandas = pytest.importorskip("pandas")
        polars = pytest.importorskip("polars")

        frames = (table().to_pandas(), polars.from_arrow(table()))
        for frame in frames:
            chunked = ChunkedSerie.from_(frame)
            assert len(chunked) == 3
            assert chunked.child("symbol") is not None
        for series in (pandas.Series([1, 2, 3], name="size"), polars.Series("size", [1, 2, 3])):
            chunked = ChunkedSerie.from_(series)
            assert chunked.num_chunks == 1 and chunked.as_py() == [1, 2, 3]

    def test_each_held_column_is_one_chunk(self) -> None:
        assert ChunkedSerie.from_(np.array([1.5, 2.5])).as_py() == [1.5, 2.5]
        pinned = ChunkedSerie.from_(pa.scalar(7, pa.int64()))
        assert pinned.num_chunks == 1
        assert pinned.field == Field("value", "int64", nullable=False)
        batch = ChunkedSerie.from_(quotes([1], ["AAPL"]))
        assert batch.num_chunks == 1 and type(batch.chunks[0]) is StructSerie
        array = ChunkedSerie.from_(pa.array([1, 2]))
        assert array.num_chunks == 1 and array.as_py() == [1, 2]

    def test_a_native_serie_is_its_one_chunk_shared(self) -> None:
        column = Serie.from_scalars(price(), [1, 2])
        chunked = ChunkedSerie.from_(column)
        assert chunked.num_chunks == 1
        assert buffer_locations(chunked.chunks[0].into_arrow_array()) == buffer_locations(
            column.into_arrow_array()
        )

    def test_a_native_chunked_serie_is_shared_or_cast_into_a_field(self) -> None:
        chunked = prices()
        again = ChunkedSerie.from_(chunked)
        assert again == chunked and again.num_chunks == 2
        assert buffer_locations(again.chunks[0].into_arrow_array()) == buffer_locations(
            chunked.chunks[0].into_arrow_array()
        )
        wide = ChunkedSerie.from_(chunked, "price: float64")
        assert wide.field == Field("price", "float64") and wide.num_chunks == 2

    def test_a_serie_reader_is_taken_and_drained_one_chunk_per_batch(self) -> None:
        reader = SerieReader.from_(table())
        chunked = ChunkedSerie.from_(reader)
        assert chunked.num_chunks == 2 and len(chunked) == 3
        # Drained, it yields nothing, and handing it over again is refused.
        assert list(reader) == []
        with pytest.raises(ValueError, match="already handed over"):
            ChunkedSerie.from_(reader)

    def test_a_serie_reader_keeps_its_own_root(self) -> None:
        root = Field("trades", "struct<id: int64>", nullable=False)
        chunked = ChunkedSerie.from_(SerieReader.from_(pa.table({"id": [1, 2]}), root))
        assert chunked.field == root
        assert Serie.from_(SerieReader.from_(pa.table({"id": [1, 2]}), root)).field == root

    def test_any_other_value_is_read_as_serie_from_reads_it(self) -> None:
        rows = ChunkedSerie.from_([1, None, 3])
        assert rows.num_chunks == 1 and rows.as_py() == [1, None, 3]
        assert ChunkedSerie.from_([1, 2], price()).field == price()
        assert ChunkedSerie.from_(7).as_py() == [7]

        # And refused exactly as `Serie.from_` refuses it.
        cyclic: list[object] = []
        cyclic.append(cyclic)
        with pytest.raises(TypeError, match="cyclic Python values") as expected:
            Serie.from_(cyclic)
        with pytest.raises(TypeError, match="cyclic Python values") as actual:
            ChunkedSerie.from_(cyclic)
        assert str(actual.value) == str(expected.value)
