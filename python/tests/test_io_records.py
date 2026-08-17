"""The generic record entry points: anything in, ``PyArrow`` readers out."""

from __future__ import annotations

import gc
import pathlib
import sys
import unittest.mock
import weakref
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pyarrow.dataset as pads
import pytest

from yggdryl import IOBase

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def rows_batch(ids: list[int], venues: list[str | None]) -> pa.RecordBatch:
    """Build one batch of the fixture schema, on any supported PyArrow."""
    return pa.Table.from_pydict({"id": ids, "venue": venues}, schema=SCHEMA).to_batches()[0]


@pytest.fixture
def batch() -> pa.RecordBatch:
    """Two rows, one of them carrying a null."""
    # `pa.record_batch` grew its mapping form after this package's PyArrow
    # floor, so the rows are named through the table form it has always had.
    return rows_batch([1, 2], ["XNAS", None])


@pytest.fixture
def table(batch: pa.RecordBatch) -> pa.Table:
    """The same two rows as a table."""
    return pa.Table.from_batches([batch])


def rows_of(handle: IOBase) -> list[tuple[int, str | None]]:
    """Read a handle back as sorted ``(id, venue)`` pairs."""
    columns = handle.read_arrow().read_all().to_pydict()
    return sorted(zip(columns["id"], columns["venue"], strict=True))


class TestWhatWriteArrowAccepts:
    """Every shape a Python caller is likely to be holding becomes one reader."""

    def test_a_table_a_batch_and_a_reader_all_write(
        self, tmp_path: pathlib.Path, table: pa.Table, batch: pa.RecordBatch
    ) -> None:
        for name, rows in (
            ("table", table),
            ("batch", batch),
            ("reader", table.to_reader()),
        ):
            handle = IOBase(tmp_path / f"{name}.parquet")
            handle.write_arrow(rows)

            assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_dataset_and_a_scanner_write_without_being_collected(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        dataset = pads.dataset(table)
        for name, rows in (("dataset", dataset), ("scanner", dataset.scanner())):
            handle = IOBase(tmp_path / f"{name}.parquet")
            handle.write_arrow(rows)

            assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_sequence_of_tables_is_written_end_to_end(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "sequence.parquet")
        handle.write_arrow([table, table])

        assert len(rows_of(handle)) == 4

    def test_a_generator_of_tables_is_never_collected(
        self, tmp_path: pathlib.Path
    ) -> None:
        # Each item's liveness is recorded when the next one is pulled: a chain
        # that holds one item at a time has already dropped the one before it,
        # and a chain that collected the generator would still hold every one.
        held: list[bool] = []

        def tables() -> Iterator[pa.Table]:
            previous: weakref.ref[pa.Table] | None = None
            for index in range(4):
                item = pa.table({"id": [index], "venue": ["X"]}, schema=SCHEMA)
                if previous is not None:
                    gc.collect()
                    held.append(previous() is not None)
                previous = weakref.ref(item)
                yield item

        handle = IOBase(tmp_path / "generated.parquet")
        handle.write_arrow(tables())

        assert held == [False, False, False]
        assert len(rows_of(handle)) == 4

    def test_mappings_become_rows_with_an_inferred_schema(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "mappings.parquet")
        handle.write_arrow([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": None}])

        assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_generator_of_rows_is_grouped_into_bounded_batches(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "rowgen.parquet")
        options = handle.record_options()
        options.batch_size = 2

        handle.write_arrow(({"id": index, "venue": "X"} for index in range(5)), options=options)

        assert len(rows_of(handle)) == 5

    def test_a_declared_schema_types_the_rows_and_names_positional_ones(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "positional.parquet")
        options = handle.record_options()
        options.schema = SCHEMA

        handle.write_arrow([(1, "XNAS"), (2, None)], options=options)

        assert rows_of(handle) == [(1, "XNAS"), (2, None)]
        assert handle.read_arrow().schema.field("id").type == pa.int64()

    def test_a_positional_row_without_a_schema_says_what_is_missing(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "unnamed.parquet")

        with pytest.raises(TypeError, match="no schema on the options naming the columns"):
            handle.write_arrow([(1, "XNAS")])

    def test_an_empty_iterable_names_no_schema(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "empty.parquet")

        with pytest.raises(ValueError, match="empty iterable"):
            handle.write_arrow([])

    def test_one_mapping_says_which_of_the_two_things_it_could_be(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "mapping.parquet")

        # Iterating a mapping yields its keys, so a bare one would otherwise
        # arrive as a sequence of strings and fail several frames later.
        with pytest.raises(TypeError, match="wrap it in a list"):
            handle.write_arrow({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

    def test_a_value_that_holds_no_rows_is_refused_by_name(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "refused.parquet")

        with pytest.raises(TypeError, match="pandas or polars frame"):
            handle.write_arrow(object())

    def test_append_arrow_adds_to_what_is_there(
        self, tmp_path: pathlib.Path, table: pa.Table, batch: pa.RecordBatch
    ) -> None:
        handle = IOBase(tmp_path / "appended.parquet")
        handle.write_arrow(table)
        handle.append_arrow([{"id": 3, "venue": "XLON"}])
        handle.append_arrow(batch)

        assert len(rows_of(handle)) == 5

    def test_read_arrow_is_the_same_call_as_the_core_read(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "same.parquet")
        handle.write_arrow(table)

        assert handle.read_arrow().read_all() == handle.read_arrow_batch_reader().read_all()


class TestPandas:
    """``pandas`` frames cross in both directions, and only when asked for."""

    def test_a_frame_writes_and_reads_back_whole(self, tmp_path: pathlib.Path) -> None:
        pandas = pytest.importorskip("pandas")
        frame = pandas.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

        handle = IOBase(tmp_path / "pandas.parquet")
        handle.write_pandas_frame(frame)

        assert list(handle.read_pandas_frame()["venue"]) == ["XNAS", "XNYS"]

    def test_reading_yields_one_frame_per_batch_lazily(
        self, tmp_path: pathlib.Path
    ) -> None:
        pytest.importorskip("pandas")
        handle = IOBase(tmp_path / "lazy.parquet")
        options = handle.record_options()
        options.max_row_group_size = 2
        handle.write_arrow(
            [{"id": index, "venue": "X"} for index in range(4)], options=options
        )

        frames = handle.read_pandas()

        # Nothing is read until the iterator is pulled, which is what lets a
        # resource larger than memory be read frame by frame.
        assert iter(frames) is frames
        assert sum(len(frame) for frame in frames) == 4

    def test_a_sequence_of_frames_streams_into_one_write(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")
        frames = (
            pandas.DataFrame({"id": [index], "venue": ["X"]}) for index in range(3)
        )

        handle = IOBase(tmp_path / "frames.parquet")
        handle.write_pandas(frames)

        assert len(rows_of(handle)) == 3

    def test_the_named_entry_point_refuses_another_library(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        pytest.importorskip("pandas")
        polars = pytest.importorskip("polars")
        handle = IOBase(tmp_path / "wrong.parquet")

        with pytest.raises(TypeError, match="expected one pandas frame"):
            handle.write_pandas_frame(table)

        # A frame of another library iterates over its own columns, so the
        # plural form has to refuse it by name rather than take it apart.
        with pytest.raises(TypeError, match="expected one pandas frame"):
            handle.write_pandas([polars.DataFrame({"id": [1]})])
        with pytest.raises(TypeError, match="pandas frame or an iterable of them"):
            handle.write_pandas(object())

    def test_a_frame_also_arrives_through_the_generic_entry_point(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")

        handle = IOBase(tmp_path / "generic.parquet")
        handle.write_arrow(pandas.DataFrame({"id": [1], "venue": ["XNAS"]}))

        assert rows_of(handle) == [(1, "XNAS")]


class TestPolars:
    """``polars`` crosses the same way, including its lazy frame."""

    def test_a_frame_writes_and_reads_back_whole(self, tmp_path: pathlib.Path) -> None:
        polars = pytest.importorskip("polars")
        frame = polars.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

        handle = IOBase(tmp_path / "polars.parquet")
        handle.write_polars_frame(frame)

        assert handle.read_polars_frame()["venue"].to_list() == ["XNAS", "XNYS"]

    def test_a_lazy_frame_is_collected_because_polars_offers_nothing_else(
        self, tmp_path: pathlib.Path
    ) -> None:
        polars = pytest.importorskip("polars")
        frame = polars.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]}).lazy()

        handle = IOBase(tmp_path / "lazyframe.parquet")
        handle.write_polars(frame)

        assert len(rows_of(handle)) == 2

    def test_reading_yields_one_frame_per_batch(self, tmp_path: pathlib.Path) -> None:
        pytest.importorskip("polars")
        handle = IOBase(tmp_path / "polarslazy.parquet")
        handle.write_arrow([{"id": 1, "venue": "X"}, {"id": 2, "venue": "Y"}])

        assert sum(frame.height for frame in handle.read_polars()) == 2

    def test_a_frame_also_arrives_through_the_generic_entry_point(
        self, tmp_path: pathlib.Path
    ) -> None:
        polars = pytest.importorskip("polars")

        handle = IOBase(tmp_path / "polarsgeneric.parquet")
        handle.write_arrow(polars.DataFrame({"id": [1], "venue": ["XNAS"]}))

        assert rows_of(handle) == [(1, "XNAS")]


class TestAbsentLibrary:
    """A library nobody installed costs nothing and is named when it is needed."""

    @staticmethod
    def without(package: str) -> Any:
        """Make one package unimportable for the duration of a block."""
        return unittest.mock.patch.dict(sys.modules, {package: None})

    def test_a_write_that_does_not_need_polars_never_imports_it(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "nopolars.parquet")

        with self.without("polars"):
            handle.write_arrow(table)
            handle.append_arrow([{"id": 3, "venue": "XLON"}])

        assert len(rows_of(handle)) == 3

    def test_a_pandas_frame_writes_while_polars_is_absent(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")
        frame = pandas.DataFrame({"id": [1], "venue": ["XNAS"]})
        handle = IOBase(tmp_path / "pandasonly.parquet")

        with self.without("polars"):
            handle.write_arrow(frame)
            handle.write_pandas_frame(frame)

        assert rows_of(handle) == [(1, "XNAS")]

    def test_reading_into_an_absent_library_names_it(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "absent.parquet")
        handle.write_arrow(table)

        with self.without("polars"), pytest.raises(ImportError, match="needs polars installed"):
            handle.read_polars_frame()

        with self.without("pandas"), pytest.raises(ImportError, match="needs pandas installed"):
            handle.read_pandas()
