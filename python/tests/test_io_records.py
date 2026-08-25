"""The generic record entry points: anything in, ``PyArrow`` readers out."""

from __future__ import annotations

import gc
import pathlib
import sys
import unittest.mock
import weakref
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

import pyarrow as pa
import pyarrow.dataset as pads
import pytest

from yggdryl import IOBase, scalar

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
    columns = handle.read_arrow_reader().read_all().to_pydict()
    return sorted(zip(columns["id"], columns["venue"], strict=True))


class TestTypedArrowWrites:
    """Each Python-held Arrow shape redirects through its typed adapter."""

    def test_a_table_a_batch_and_a_reader_all_write(
        self, tmp_path: pathlib.Path, table: pa.Table, batch: pa.RecordBatch
    ) -> None:
        for name, write in (
            ("table", lambda handle: handle.overwrite_arrow_table(table)),
            ("batch", lambda handle: handle.overwrite_arrow_batch(batch)),
            ("reader", lambda handle: handle.overwrite_arrow_reader(table.to_reader())),
        ):
            handle = IOBase(tmp_path / f"{name}.parquet")
            write(handle)

            assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_scanner_reader_writes_without_being_collected(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        dataset = pads.dataset(table)
        handle = IOBase(tmp_path / "scanner.parquet")
        handle.overwrite_arrow_reader(dataset.scanner().to_reader())

        assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_sequence_of_tables_is_written_end_to_end(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "sequence.parquet")
        reader = pa.RecordBatchReader.from_batches(
            SCHEMA, (batch for item in (table, table) for batch in item.to_batches())
        )
        handle.overwrite_arrow_reader(reader)

        assert len(rows_of(handle)) == 4

    def test_a_generator_of_tables_is_never_collected(
        self, tmp_path: pathlib.Path
    ) -> None:
        # Each item's liveness is recorded when the next one is pulled: a chain
        # that holds one item at a time has already dropped the one before it,
        # and a chain that collected the generator would still hold every one.
        held: list[bool] = []

        def batches() -> Iterator[pa.RecordBatch]:
            previous: weakref.ref[pa.Table] | None = None
            for index in range(4):
                item = pa.table({"id": [index], "venue": ["X"]}, schema=SCHEMA)
                if previous is not None:
                    gc.collect()
                    held.append(previous() is not None)
                previous = weakref.ref(item)
                yield item.to_batches()[0]

        handle = IOBase(tmp_path / "generated.parquet")
        handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(SCHEMA, batches()))

        assert held == [False, False, False]
        assert len(rows_of(handle)) == 4

    def test_mappings_become_rows_with_an_inferred_schema(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "mappings.parquet")
        handle.overwrite_records([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": None}])

        assert rows_of(handle) == [(1, "XNAS"), (2, None)]

    def test_a_generator_of_rows_is_grouped_into_bounded_batches(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "rowgen.parquet")
        options = handle.record_options()
        options.batch_size = 2

        handle.overwrite_records(
            ({"id": index, "venue": "X"} for index in range(5)), options=options
        )

        assert len(rows_of(handle)) == 5

    def test_a_declared_field_types_the_rows_and_names_positional_ones(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "positional.parquet")
        options = handle.record_options()
        options.field = SCHEMA

        handle.overwrite_records([(1, "XNAS"), (2, None)], options=options)

        assert rows_of(handle) == [(1, "XNAS"), (2, None)]
        assert handle.read_arrow_reader().schema.field("id").type == pa.int64()

    def test_a_positional_row_without_a_field_says_what_is_missing(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "unnamed.parquet")

        with pytest.raises(TypeError, match="no field on the options naming the columns"):
            handle.overwrite_records([(1, "XNAS")])

    def test_an_empty_iterable_names_no_schema(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "empty.parquet")

        with pytest.raises(ValueError, match="empty records cannot infer a field"):
            handle.overwrite_records([])

        options = handle.record_options()
        options.field = SCHEMA
        handle.overwrite_records([], options=options)
        assert handle.read_arrow_reader().read_all().num_rows == 0

    def test_one_mapping_says_which_of_the_two_things_it_could_be(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "mapping.parquet")

        # Iterating a mapping yields its keys, so a bare one would otherwise
        # arrive as a sequence of strings and fail several frames later.
        with pytest.raises(TypeError, match="wrap it in a list"):
            handle.overwrite_records({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

    def test_a_value_that_holds_no_rows_is_refused_by_name(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "refused.parquet")

        with pytest.raises(TypeError, match="iterable of mapping"):
            handle.overwrite_records(object())

    def test_append_arrow_adds_to_what_is_there(
        self, tmp_path: pathlib.Path, table: pa.Table, batch: pa.RecordBatch
    ) -> None:
        handle = IOBase(tmp_path / "appended.parquet")
        handle.overwrite_arrow_table(table)
        handle.append_records([{"id": 3, "venue": "XLON"}])
        handle.append_arrow_batch(batch)

        assert len(rows_of(handle)) == 5

    def test_read_arrow_is_the_same_call_as_the_core_read(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "same.parquet")
        handle.overwrite_arrow_table(table)

        assert handle.read_arrow_reader().read_all() == handle.read_arrow_reader().read_all()

    def test_record_options_are_one_keyword_only(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "options.parquet")

        with pytest.raises(TypeError, match="unexpected keyword argument 'field'"):
            handle.read_arrow_reader(field=SCHEMA)


class TestExplicitIntent:
    """Every held Arrow shape exposes overwrite, append, and keyed merge."""

    @staticmethod
    def merge_options(handle: IOBase) -> Any:
        options = handle.record_options()
        options.merge_by_names = ["id"]
        return options

    def test_arrow_reader_triplet(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "reader.parquet")
        first = rows_batch([1], ["XNAS"])
        second = rows_batch([2], ["XNYS"])
        updated = rows_batch([2, 3], ["XLON", "XPAR"])

        def reader(batch: pa.RecordBatch) -> pa.RecordBatchReader:
            return pa.RecordBatchReader.from_batches(SCHEMA, [batch])

        handle.overwrite_arrow_reader(reader(first))
        handle.append_arrow_reader(reader(second))
        handle.merge_arrow_reader(reader(updated), options=self.merge_options(handle))

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    def test_arrow_table_triplet(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "table.parquet")

        def make(ids: list[int], venues: list[str]) -> pa.Table:
            return pa.Table.from_batches([rows_batch(ids, venues)])

        handle.overwrite_arrow_table(make([1], ["XNAS"]))
        handle.append_arrow_table(make([2], ["XNYS"]))
        handle.merge_arrow_table(
            make([2, 3], ["XLON", "XPAR"]), options=self.merge_options(handle)
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    def test_arrow_batch_triplet(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "batch.parquet")

        handle.overwrite_arrow_batch(rows_batch([1], ["XNAS"]))
        handle.append_arrow_batch(rows_batch([2], ["XNYS"]))
        handle.merge_arrow_batch(
            rows_batch([2, 3], ["XLON", "XPAR"]),
            options=self.merge_options(handle),
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    @pytest.mark.parametrize(
        ("method", "keys", "message"),
        [
            ("overwrite_arrow_reader", ["id"], "write mode overwrite does not accept"),
            ("append_arrow_reader", ["id"], "write mode append does not accept"),
            ("merge_arrow_reader", [], "write mode merge requires at least one"),
            ("overwrite_arrow_table", ["id"], "write mode overwrite does not accept"),
            ("append_arrow_table", ["id"], "write mode append does not accept"),
            ("merge_arrow_table", [], "write mode merge requires at least one"),
            (
                "overwrite_arrow_batch",
                ["id"],
                "write mode overwrite does not accept",
            ),
            ("append_arrow_batch", ["id"], "write mode append does not accept"),
            ("merge_arrow_batch", [], "write mode merge requires at least one"),
        ],
    )
    def test_invalid_arrow_intent_does_not_inspect_input(
        self,
        tmp_path: pathlib.Path,
        method: str,
        keys: list[str],
        message: str,
    ) -> None:
        class Untouched:
            pulls = 0

            def __getattribute__(self, name: str) -> Any:
                if name == "pulls":
                    return object.__getattribute__(self, name)
                type(self).pulls += 1
                raise AssertionError("the invalid write inspected its reader")

        handle = IOBase(tmp_path / f"{method}.parquet")
        options = handle.record_options()
        options.merge_by_names = keys
        source = Untouched()

        with pytest.raises(ValueError, match=message):
            getattr(handle, method)(source, options=options)
        assert source.pulls == 0


class TestGenericIOMode:
    """Every generic representation routes through the required core mode."""

    @staticmethod
    def merge_options(handle: IOBase) -> Any:
        options = handle.record_options()
        options.merge_by_names = ["id"]
        return options

    @pytest.mark.parametrize(
        "method",
        ["write_arrow_reader", "write_arrow_table", "write_arrow_batch"],
    )
    def test_each_arrow_shape_dispatches_all_modes(
        self, tmp_path: pathlib.Path, method: str
    ) -> None:
        handle = IOBase(tmp_path / f"generic-{method}.parquet")

        def source(ids: list[int], venues: list[str]) -> Any:
            batch = rows_batch(ids, venues)
            if method == "write_arrow_reader":
                return pa.RecordBatchReader.from_batches(SCHEMA, [batch])
            if method == "write_arrow_table":
                return pa.Table.from_batches([batch])
            return batch

        write = getattr(handle, method)
        write(source([1], ["XNAS"]), "overwrite")
        write(source([2], ["XNYS"]), "APPEND")
        write(
            source([2, 3], ["XLON", "XPAR"]),
            " merge ",
            options=self.merge_options(handle),
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    def test_records_dispatch_all_modes(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "generic-records.parquet")
        handle.write_records([{"id": 1, "venue": "XNAS"}], "overwrite")
        handle.write_records([{"id": 2, "venue": "XNYS"}], "append")
        handle.write_records(
            [{"id": 2, "venue": "XLON"}, {"id": 3, "venue": "XPAR"}],
            "merge",
            options=self.merge_options(handle),
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    @pytest.mark.parametrize(
        ("mode", "keys", "message"),
        [
            ("replace", [], "invalid mode"),
            ("overwrite", ["id"], "write mode overwrite does not accept"),
            ("merge", [], "write mode merge requires at least one"),
        ],
    )
    def test_mode_preflight_does_not_inspect_input(
        self,
        tmp_path: pathlib.Path,
        mode: str,
        keys: list[str],
        message: str,
    ) -> None:
        class Untouched:
            pulls = 0

            def __getattribute__(self, name: str) -> Any:
                if name == "pulls":
                    return object.__getattribute__(self, name)
                type(self).pulls += 1
                raise AssertionError("invalid mode inspected its input")

        handle = IOBase(tmp_path / f"invalid-{mode}.parquet")
        options = handle.record_options()
        options.merge_by_names = keys
        source = Untouched()

        with pytest.raises((TypeError, ValueError), match=message):
            handle.write_arrow_reader(source, mode, options=options)
        assert source.pulls == 0

    @pytest.mark.parametrize("intent", ["overwrite", "append", "merge"])
    def test_typed_arrow_adapters_reject_cross_shape_inputs(
        self,
        tmp_path: pathlib.Path,
        intent: str,
        table: pa.Table,
        batch: pa.RecordBatch,
    ) -> None:
        handle = IOBase(tmp_path / f"cross-{intent}.parquet")
        options = handle.record_options()
        if intent == "merge":
            options.merge_by_names = ["id"]

        with pytest.raises(TypeError, match="Arrow C stream reader"):
            getattr(handle, f"{intent}_arrow_reader")(table, options=options)
        with pytest.raises(TypeError, match="pyarrow.Table"):
            getattr(handle, f"{intent}_arrow_table")(batch, options=options)
        with pytest.raises(TypeError, match="pyarrow.RecordBatch"):
            getattr(handle, f"{intent}_arrow_batch")(table, options=options)

    @pytest.mark.parametrize(
        ("method", "keys", "message"),
        [
            ("overwrite_records", ["id"], "write mode overwrite does not accept"),
            ("append_records", ["id"], "write mode append does not accept"),
            ("merge_records", [], "write mode merge requires at least one"),
        ],
    )
    def test_invalid_records_intent_does_not_iterate_input(
        self,
        tmp_path: pathlib.Path,
        method: str,
        keys: list[str],
        message: str,
    ) -> None:
        class Untouched:
            pulls = 0

            def __iter__(self) -> Iterator[object]:
                self.pulls += 1
                raise AssertionError("the invalid write iterated its records")

        handle = IOBase(tmp_path / f"{method}.parquet")
        options = handle.record_options()
        options.merge_by_names = keys
        source = Untouched()

        with pytest.raises(ValueError, match=message):
            getattr(handle, method)(source, options=options)
        assert source.pulls == 0


@scalar
class Trade:
    id: int
    venue: str | None


@dataclass
class PlainTrade:
    id: int
    venue: str | None


@scalar
class TradeId:
    id: int


class TestDataclassRecords:
    def test_decorated_dataclass_infers_its_cached_struct_field(
        self, tmp_path: pathlib.Path
    ) -> None:
        cached = Trade.field()
        assert Trade.field() is cached
        handle = IOBase(tmp_path / "dataclass.parquet")

        handle.overwrite_records([Trade(1, "XNAS"), Trade(2, None)])

        assert Trade.field() is cached
        assert [field.name for field in handle.read_arrow_field().data_type] == ["id", "venue"]
        assert list(handle.read_records(Trade)) == [Trade(1, "XNAS"), Trade(2, None)]
        assert list(handle.read_records()) == [
            {"id": 1, "venue": "XNAS"},
            {"id": 2, "venue": None},
        ]

    def test_plain_dataclass_reads_one_row_at_a_time(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "plain.parquet")
        handle.overwrite_records([PlainTrade(1, "XNAS"), PlainTrade(2, None)])

        records = handle.read_records(PlainTrade)
        assert iter(records) is records
        assert list(records) == [PlainTrade(1, "XNAS"), PlainTrade(2, None)]

    def test_requested_decorated_class_projects_before_building_rows(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "projected.parquet")
        handle.overwrite_arrow_table(table)

        assert list(handle.read_records(TradeId)) == [TradeId(1), TradeId(2)]

    def test_absent_resource_yields_no_mapping_records(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "absent.arrows")

        assert list(handle.read_records()) == []

    def test_read_records_refuses_a_non_dataclass_type(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "records.arrows")

        with pytest.raises(TypeError, match="expected a dataclass type or None"):
            handle.read_records(dict)

    def test_records_triplet_merges_dataclass_rows(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "records.parquet")
        handle.overwrite_records([Trade(1, "XNAS")])
        handle.append_records([Trade(2, "XNYS")])
        options = handle.record_options()
        options.merge_by_names = ["id"]
        handle.merge_records([Trade(2, "XLON"), Trade(3, "XPAR")], options=options)

        assert list(handle.read_records(Trade)) == [
            Trade(1, "XNAS"),
            Trade(2, "XLON"),
            Trade(3, "XPAR"),
        ]


class TestPandas:
    """``pandas`` frames cross in both directions, and only when asked for."""

    def test_a_frame_writes_and_reads_back_whole(self, tmp_path: pathlib.Path) -> None:
        pandas = pytest.importorskip("pandas")
        frame = pandas.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

        handle = IOBase(tmp_path / "pandas.parquet")
        handle.overwrite_pandas_frame(frame)

        assert list(handle.read_pandas_frame()["venue"]) == ["XNAS", "XNYS"]

    def test_reading_yields_one_frame_per_batch_lazily(
        self, tmp_path: pathlib.Path
    ) -> None:
        pytest.importorskip("pandas")
        handle = IOBase(tmp_path / "lazy.parquet")
        options = handle.record_options()
        options.max_row_group_size = 2
        handle.overwrite_records(
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
        handle.overwrite_pandas(frames)

        assert len(rows_of(handle)) == 3

    def test_frame_and_stream_entry_points_have_explicit_intent_triplets(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")

        for suffix, plural in (("frame", False), ("frames", True)):
            handle = IOBase(tmp_path / f"pandas-{suffix}.parquet")
            first = pandas.DataFrame({"id": [1], "venue": ["XNAS"]})
            second = pandas.DataFrame({"id": [2], "venue": ["XNYS"]})
            merged = pandas.DataFrame({"id": [2, 3], "venue": ["XLON", "XPAR"]})
            options = handle.record_options()
            options.merge_by_names = ["id"]
            if plural:
                handle.overwrite_pandas([first])
                handle.append_pandas([second])
                handle.merge_pandas([merged], options=options)
            else:
                handle.overwrite_pandas_frame(first)
                handle.append_pandas_frame(second)
                handle.merge_pandas_frame(merged, options=options)

            assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    @pytest.mark.parametrize("method", ["write_pandas", "write_pandas_frame"])
    def test_generic_frame_entry_points_dispatch_all_modes(
        self, tmp_path: pathlib.Path, method: str
    ) -> None:
        pandas = pytest.importorskip("pandas")
        handle = IOBase(tmp_path / f"generic-{method}.parquet")
        whole = method.endswith("_frame")

        def source(ids: list[int], venues: list[str]) -> Any:
            frame = pandas.DataFrame({"id": ids, "venue": venues})
            return frame if whole else (frame,)

        write = getattr(handle, method)
        write(source([1], ["XNAS"]), "overwrite")
        write(source([2], ["XNYS"]), "append")
        options = handle.record_options()
        options.merge_by_names = ["id"]
        write(
            source([2, 3], ["XLON", "XPAR"]),
            "merge",
            options=options,
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    def test_the_named_entry_point_refuses_another_library(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        pytest.importorskip("pandas")
        polars = pytest.importorskip("polars")
        handle = IOBase(tmp_path / "wrong.parquet")

        with pytest.raises(TypeError, match="expected one pandas frame"):
            handle.overwrite_pandas_frame(table)

        # A frame of another library iterates over its own columns, so the
        # plural form has to refuse it by name rather than take it apart.
        with pytest.raises(TypeError, match="expected one pandas frame"):
            handle.overwrite_pandas([polars.DataFrame({"id": [1]})])
        with pytest.raises(TypeError, match="pandas frame or an iterable of them"):
            handle.overwrite_pandas(object())

    def test_a_frame_is_refused_by_the_arrow_reader_entry_point(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")

        handle = IOBase(tmp_path / "generic.parquet")
        with pytest.raises(TypeError, match="Arrow C stream reader"):
            handle.overwrite_arrow_reader(
                pandas.DataFrame({"id": [1], "venue": ["XNAS"]})
            )


class TestPolars:
    """``polars`` crosses the same way, including its lazy frame."""

    def test_a_frame_writes_and_reads_back_whole(self, tmp_path: pathlib.Path) -> None:
        polars = pytest.importorskip("polars")
        frame = polars.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]})

        handle = IOBase(tmp_path / "polars.parquet")
        handle.overwrite_polars_frame(frame)

        assert handle.read_polars_frame()["venue"].to_list() == ["XNAS", "XNYS"]

    def test_a_lazy_frame_is_collected_because_polars_offers_nothing_else(
        self, tmp_path: pathlib.Path
    ) -> None:
        polars = pytest.importorskip("polars")
        frame = polars.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]}).lazy()

        handle = IOBase(tmp_path / "lazyframe.parquet")
        handle.overwrite_polars(frame)

        assert len(rows_of(handle)) == 2

    def test_reading_yields_one_frame_per_batch(self, tmp_path: pathlib.Path) -> None:
        pytest.importorskip("polars")
        handle = IOBase(tmp_path / "polarslazy.parquet")
        handle.overwrite_records([{"id": 1, "venue": "X"}, {"id": 2, "venue": "Y"}])

        assert sum(frame.height for frame in handle.read_polars()) == 2

    def test_frame_and_stream_entry_points_have_explicit_intent_triplets(
        self, tmp_path: pathlib.Path
    ) -> None:
        polars = pytest.importorskip("polars")

        for suffix, plural in (("frame", False), ("frames", True)):
            handle = IOBase(tmp_path / f"polars-{suffix}.parquet")
            first = polars.DataFrame({"id": [1], "venue": ["XNAS"]})
            second = polars.DataFrame({"id": [2], "venue": ["XNYS"]})
            merged = polars.DataFrame({"id": [2, 3], "venue": ["XLON", "XPAR"]})
            options = handle.record_options()
            options.merge_by_names = ["id"]
            if plural:
                handle.overwrite_polars([first])
                handle.append_polars([second])
                handle.merge_polars([merged], options=options)
            else:
                handle.overwrite_polars_frame(first)
                handle.append_polars_frame(second)
                handle.merge_polars_frame(merged, options=options)

            assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    @pytest.mark.parametrize("method", ["write_polars", "write_polars_frame"])
    def test_generic_frame_entry_points_dispatch_all_modes(
        self, tmp_path: pathlib.Path, method: str
    ) -> None:
        polars = pytest.importorskip("polars")
        handle = IOBase(tmp_path / f"generic-{method}.parquet")
        whole = method.endswith("_frame")

        def source(ids: list[int], venues: list[str]) -> Any:
            frame = polars.DataFrame({"id": ids, "venue": venues})
            return frame if whole else (frame,)

        write = getattr(handle, method)
        write(source([1], ["XNAS"]), "overwrite")
        write(source([2], ["XNYS"]), "append")
        options = handle.record_options()
        options.merge_by_names = ["id"]
        write(
            source([2, 3], ["XLON", "XPAR"]),
            "merge",
            options=options,
        )

        assert rows_of(handle) == [(1, "XNAS"), (2, "XLON"), (3, "XPAR")]

    def test_a_frame_is_refused_by_the_arrow_reader_entry_point(
        self, tmp_path: pathlib.Path
    ) -> None:
        polars = pytest.importorskip("polars")

        handle = IOBase(tmp_path / "polarsgeneric.parquet")
        with pytest.raises(TypeError, match="Arrow C stream reader"):
            handle.overwrite_arrow_reader(
                polars.DataFrame({"id": [1], "venue": ["XNAS"]})
            )


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
            handle.overwrite_arrow_table(table)
            handle.append_records([{"id": 3, "venue": "XLON"}])

        assert len(rows_of(handle)) == 3

    def test_a_pandas_frame_writes_while_polars_is_absent(
        self, tmp_path: pathlib.Path
    ) -> None:
        pandas = pytest.importorskip("pandas")
        frame = pandas.DataFrame({"id": [1], "venue": ["XNAS"]})
        handle = IOBase(tmp_path / "pandasonly.parquet")

        with self.without("polars"):
            handle.overwrite_pandas_frame(frame)

        assert rows_of(handle) == [(1, "XNAS")]

    def test_reading_into_an_absent_library_names_it(
        self, tmp_path: pathlib.Path, table: pa.Table
    ) -> None:
        handle = IOBase(tmp_path / "absent.parquet")
        handle.overwrite_arrow_table(table)

        with self.without("polars"), pytest.raises(ImportError, match="needs polars installed"):
            handle.read_polars_frame()

        with self.without("pandas"), pytest.raises(ImportError, match="needs pandas installed"):
            handle.read_pandas()
