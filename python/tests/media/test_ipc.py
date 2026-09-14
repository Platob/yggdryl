"""Arrow IPC over a handle: readers in, readers out, nothing copied."""

from __future__ import annotations

import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase, RecordOptions
from yggdryl.holder import Path

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
        pa.field("venue", pa.string()),
    ]
)


def _batch(start: int = 0) -> pa.RecordBatch:
    """Build one two-row batch of the shared schema."""
    return pa.record_batch(
        {
            "id": [start, start + 1],
            "symbol": ["AAPL", None],
            "venue": ["XNAS", "XNAS"],
        },
        schema=SCHEMA,
    )


def _reader(*batches: pa.RecordBatch) -> pa.RecordBatchReader:
    """Wrap held batches in the exact streamed input shape."""
    return pa.RecordBatchReader.from_batches(SCHEMA, batches)


@pytest.fixture
def stream(tmp_path: pathlib.Path) -> IOBase:
    """A handle whose name says it holds an Arrow IPC stream."""
    return IOBase(tmp_path / "trades.arrows")


class TestTheEncodingComesFromTheHandle:
    """No record call takes a format argument, here or in the core."""

    def test_the_media_type_names_the_encoding(self, stream: IOBase) -> None:
        options = stream.record_options()

        assert str(options.mime_type) == "application/vnd.apache.arrow.stream"
        assert options.name == "row"
        assert options.field is None
        assert options.selector.is_all
        assert options.filter.is_always_true
        assert options.merge_by.is_all
        assert options.batch_row_size is None

    def test_an_encoding_this_build_lacks_is_named_rather_than_guessed(
        self, tmp_path: pathlib.Path
    ) -> None:
        with pytest.raises(ValueError, match="text/csv"):
            IOBase(tmp_path / "trades.csv").record_options()

    def test_a_buffer_declares_what_it_holds(self) -> None:
        handle = IOBase.from_bytes()
        handle.media_type = "application/vnd.apache.arrow.stream"

        handle.overwrite_arrow_reader(_reader(_batch()))
        assert handle.read_arrow_reader().read_all().num_rows == 2

    def test_the_coding_comes_from_the_name(self, tmp_path: pathlib.Path) -> None:
        compressed = IOBase(tmp_path / "trades.arrows.gz")
        compressed.overwrite_arrow_reader(_reader(_batch()))

        # Identical calls on both sides; only the name changed. The handle
        # presents the decoded stream, so the coded form is what the
        # stored-byte role reads.
        assert Path(tmp_path / "trades.arrows.gz").read_bytes()[:2] == b"\x1f\x8b"
        assert compressed.read_arrow_reader().read_all().num_rows == 2


class TestBatchesCrossAsReaders:
    """A `pyarrow.RecordBatchReader` is the one shape in both directions."""

    def test_a_read_returns_a_reader_that_knows_its_schema_first(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(_reader(_batch(), _batch(2)))

        reader = stream.read_arrow_reader()
        assert isinstance(reader, pa.RecordBatchReader)
        # The schema is known before a single batch is decoded.
        assert reader.schema.names == ["id", "symbol", "venue"]
        assert reader.read_next_batch().num_rows == 2
        assert reader.read_all().num_rows == 2

    def test_each_held_arrow_shape_uses_its_typed_adapter(
        self, tmp_path: pathlib.Path
    ) -> None:
        batch = _batch()
        writes = [
            lambda handle: handle.overwrite_arrow_batch(batch),
            lambda handle: handle.overwrite_arrow_table(pa.Table.from_batches([batch])),
            lambda handle: handle.overwrite_arrow_reader(_reader(batch)),
        ]

        for index, write in enumerate(writes):
            handle = IOBase(tmp_path / f"source-{index}.arrows")
            write(handle)
            assert handle.read_arrow_reader().read_all() == pa.Table.from_batches([batch])

    def test_a_reader_written_lazily_is_never_materialized(
        self, stream: IOBase
    ) -> None:
        produced = 0

        def batches() -> object:
            nonlocal produced
            for start in range(0, 8, 2):
                produced += 1
                yield _batch(start)

        stream.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(SCHEMA, batches()))

        assert produced == 4
        assert stream.read_arrow_reader().read_all().num_rows == 8

    def test_batches_come_back_as_they_were_written(self, stream: IOBase) -> None:
        stream.overwrite_arrow_reader(_reader(_batch(), _batch(2)))

        assert sum(1 for _ in stream.read_arrow_reader()) == 2

    def test_what_is_not_a_stream_of_batches_is_refused(self, stream: IOBase) -> None:
        with pytest.raises(TypeError):
            stream.overwrite_arrow_reader(object())
        with pytest.raises(TypeError, match="Arrow C stream reader"):
            stream.overwrite_arrow_reader([])


class TestColumnPushdown:
    """A declared field selects and casts in one pass over the data."""

    def _reading(self, stream: IOBase, schema: object) -> RecordOptions:
        """The options one projected read runs under."""
        options = stream.record_options()
        options.field = schema
        return options

    def test_a_subset_is_pushed_down_to_the_encoding(self, stream: IOBase) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        projected = stream.read_arrow_reader(
            options=self._reading(stream, wanted)
        ).read_all()
        assert projected.column_names == ["id"]
        # The resource is unchanged: it still holds all three.
        assert len(stream.read_arrow_field().dtype) == 3

    def test_field_selection_lives_only_on_options(self, stream: IOBase) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        projected = stream.read_arrow_reader(
            options=self._reading(stream, wanted)
        ).read_all()
        assert projected.column_names == ["id"]

    def test_a_column_the_stream_lacks_is_supplied_by_the_cast(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        invented = pa.schema(
            [pa.field("id", pa.int64(), nullable=False), pa.field("nowhere", pa.string())]
        )

        read = stream.read_arrow_reader(options=self._reading(stream, invented))
        table = read.read_all()
        # A projection cannot invent a column, so the encoding read everything
        # and the cast produced the declared shape with nulls in the new column.
        assert table.column_names == ["id", "nowhere"]
        assert table.column("nowhere").null_count == 2

    def test_the_field_accepts_the_ways_python_spells_a_schema(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        native = Field.from_str("row:struct<id:int64 not null> not null")

        for spelling in (
            native,
            str(native),
            pa.schema([pa.field("id", pa.int64(), nullable=False)]),
        ):
            options = self._reading(stream, spelling)
            assert stream.read_arrow_reader(options=options).schema.names == ["id"]


class TestWritesAndMerges:
    """Overwrite, append, and keyed merge carry explicit intent."""

    def test_appending_reads_what_is_there_and_rewrites(self, stream: IOBase) -> None:
        stream.append_arrow_reader(_reader(_batch()))
        stream.append_arrow_reader(_reader(_batch(2)))

        assert stream.read_arrow_reader().read_all().num_rows == 4
        assert sum(1 for _ in stream.read_arrow_reader()) == 2

    def test_a_match_key_updates_a_stored_row_and_appends_a_new_one(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        options = stream.record_options()
        options.merge_by = ["id"]
        assert options.merge_by.names == ["id"]

        updated = pa.record_batch(
            {"id": [1, 7], "symbol": ["MSFT", "NVDA"], "venue": ["XNAS", "XNAS"]},
            schema=SCHEMA,
        )
        stream.merge_arrow_reader(_reader(updated), options=options)

        table = stream.read_arrow_reader().read_all()
        assert table.column("id").to_pylist() == [0, 1, 7]
        assert table.column("symbol").to_pylist() == ["AAPL", "MSFT", "NVDA"]

    def test_an_empty_match_key_replaces_the_resource(self, stream: IOBase) -> None:
        stream.overwrite_arrow_reader(_reader(_batch(), _batch(2)))
        assert stream.read_arrow_reader().read_all().num_rows == 4

        stream.overwrite_arrow_reader(_reader(_batch(10)))
        table = stream.read_arrow_reader().read_all()
        assert table.num_rows == 2
        assert table.column("id").to_pylist() == [10, 11]

    def test_a_write_stores_the_declared_root(self, stream: IOBase) -> None:
        options = stream.record_options()
        options.field = "row:struct<id:int64 not null> not null"
        stream.overwrite_arrow_batch(
            pa.record_batch(
                {"id": [1, 2]},
                schema=pa.schema([pa.field("id", pa.int64(), nullable=False)]),
            ),
            options=options,
        )

        assert stream.read_arrow_field().name == "row"
        assert len(stream.read_arrow_field().dtype) == 1


class TestPartitionColumns:
    """A folder is one table; the three methods resolve its leaves."""

    def test_a_folder_write_leaves_the_path_columns_out(
        self, tmp_path: pathlib.Path
    ) -> None:
        schema = pa.schema(
            [
                pa.field("price", pa.int64(), nullable=False),
                pa.field("year", pa.int32(), nullable=False),
                pa.field("month", pa.string(), nullable=False),
            ]
        )
        (tmp_path / "year=2024" / "month=01").mkdir(parents=True)
        rows = pa.record_batch(
            {"price": [10, 20], "year": [2024, 2024], "month": ["01", "01"]},
            schema=schema,
        )

        lake = IOBase(tmp_path)
        options = RecordOptions("part.arrows")
        options.field = schema
        lake.overwrite_arrow_batch(rows, options=options)

        # Only `price` reached the leaf; the other two are the directory names.
        leaf = lake / "year=2024" / "month=01" / "part-0.arrows"
        assert len(leaf.read_arrow_field().dtype) == 1

        restored = lake.read_arrow_reader(options=options).read_all()
        assert restored.column_names == ["price", "year", "month"]
        assert restored.schema.field("year").type == pa.int32()


class TestOptions:
    """One settings value, shared across the encodings."""

    def test_the_settings_are_the_ones_every_encoding_shares(
        self, stream: IOBase
    ) -> None:
        options = RecordOptions("trades.arrows")
        options.name = "trade"
        options.safe = True
        options.batch_row_size = 1
        options.commit_row_size = 2
        options.level = 9
        options.merge_by = ["id"]

        assert (
            options.name,
            options.safe,
            options.batch_row_size,
            options.commit_row_size,
            options.level,
            options.merge_by.names,
        ) == ("trade", True, 1, 2, 9, ["id"])
        options.commit_row_size = None
        assert options.commit_row_size is None
        # Options carry no field until one is declared.
        assert options.field is None
        options.field = SCHEMA
        assert options.field is not None
        assert options.field.name == "trade"

    def test_a_parquet_only_setting_is_refused_by_name(self) -> None:
        options = RecordOptions("trades.arrows")

        assert options.max_row_group_size is None
        assert options.key_value_metadata is None
        assert options.compression is None
        with pytest.raises(ValueError, match="arrow.stream"):
            options.max_row_group_size = 128
        with pytest.raises(ValueError, match="arrow.stream"):
            options.compression = "snappy"

    def test_options_may_be_given_as_the_media_type_alone(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(
            _reader(_batch()), options="application/vnd.apache.arrow.stream"
        )

        assert (
            stream.read_arrow_reader(options="trades.arrows").read_all().num_rows
            == 2
        )

    def test_the_declared_root_name_names_an_inferred_schema(
        self, stream: IOBase
    ) -> None:
        stream.overwrite_arrow_reader(_reader(_batch()))
        options = stream.record_options()
        options.name = "trade"

        assert stream.read_arrow_field(options=options).name == "trade"


class TestTheDeclaredRootIsOneSection:
    """`field` is the `create` section of the plan the options run."""

    def test_a_declared_field_is_stored_whole(self) -> None:
        options = RecordOptions("trades.arrows")
        assert options.field is None

        declared = Field(
            "trade",
            "struct<id: int64 not null>",
            nullable=True,
            metadata={"owner": "tests"},
        )
        options.field = declared
        assert options.name == "trade"
        # Nullability is not a declaration: the stored root is non-null.
        assert options.field is not None
        assert not options.field.nullable
        assert options.field.dtype == DataType("struct<id: int64 not null>")
        assert dict(options.field.metadata.items()) == {"owner": "tests"}
        # The name is one part of the field, so renaming renames it.
        options.name = "row"
        assert options.field.name == "row"
        assert declared.name == "trade"
        # A bare Arrow schema takes the declared name.
        options.name = "trade"
        options.field = SCHEMA
        assert options.field is not None
        assert options.field.name == "trade"
        # The field is the plan's `create` section, spelled as one.
        assert str(options.plan).startswith("create trade (")
        assert options.plan.field() == options.field

    def test_field_takes_every_root_spelling_and_none_clears(self) -> None:
        options = RecordOptions("trades.arrows")
        expected = Field("row", "struct<id: int64>", nullable=False)

        options.field = expected
        assert options.field == expected
        options.field = pa.schema([pa.field("id", pa.int64())])
        assert options.field == expected
        with pytest.raises(ValueError):
            options.field = Field("row", "struct<id: not a datatype>")
        assert options.field == expected

        options.field = None
        assert options.field is None
        assert options.plan.field() is None

    def test_equality_and_hash_read_the_declared_field(self) -> None:
        root = Field(
            "row",
            "struct<id: int64 not null>",
            nullable=False,
            metadata={"owner": "tests"},
        )
        by_field = RecordOptions("trades.arrows")
        by_field.field = root
        by_plan = RecordOptions("trades.arrows")
        by_plan.plan = root

        assert by_field == by_plan
        assert by_field.stable_hash() == by_plan.stable_hash()
        assert hash(by_field) == hash(by_plan)
        assert len({by_field, by_plan}) == 1

        by_plan = RecordOptions("trades.arrows")
        by_plan.field = Field("row", "struct<id: int64 not null>", nullable=False)
        assert by_field != by_plan

    def test_pickle_state_carries_the_sections(self) -> None:
        options = RecordOptions("trades.arrows")
        options.field = Field(
            "trade",
            "struct<id: int64 not null>",
            nullable=False,
            metadata={"owner": "tests"},
        )
        options.batch_row_size = 16
        options.selector = ["id"]
        options.filter = "id > 1"
        options.merge_by = ["id"]

        _, (state,) = options.__reduce__()
        assert state["name"] == "trade"
        assert state["field"] == options.field
        assert state["selector"] == "id"
        assert state["filter"] == "id > 1"
        assert state["merge_by"] == "id"
        assert state["batch_row_size"] == 16
        assert not {"dtype", "metadata", "root_name", "batch_size"} & state.keys()

        restored = pickle.loads(pickle.dumps(options))
        assert restored == options
        assert restored.field == options.field
        assert restored.selector == options.selector
        assert restored.batch_row_size == 16

        options.field = None
        _, (state,) = options.__reduce__()
        assert state["field"] is None
        assert pickle.loads(pickle.dumps(options)).field is None

    def test_a_batch_of_no_rows_is_refused(self) -> None:
        options = RecordOptions("trades.arrows")
        options.batch_row_size = 8

        with pytest.raises(
            ValueError,
            match=r"^expected a positive row count for batch_row_size, got 0; "
            r"pass None for no bound$",
        ):
            options.batch_row_size = 0
        assert options.batch_row_size == 8
        options.batch_row_size = None
        assert options.batch_row_size is None


class TestAbsenceAndScope:
    """Reading follows the laziness rule; a scope publishes."""

    def test_a_resource_that_is_not_there_holds_no_batches(
        self, stream: IOBase
    ) -> None:
        assert not stream.exists()
        assert stream.read_arrow_reader().read_all().num_rows == 0

    def test_a_scope_opens_and_publishes(self, tmp_path: pathlib.Path) -> None:
        path = tmp_path / "scoped.arrows"

        with IOBase(path) as handle:
            # Context entry opens the native media cache but never creates;
            # the resource appears only as a consequence of the write.
            assert handle.opened
            assert not handle.closed
            assert not path.exists()
            handle.overwrite_arrow_reader(_reader(_batch()))
            assert handle.opened
            assert not handle.closed

        # Closing published the bytes at their exact length, which is what
        # another reader needs to find the end of the stream.
        assert handle.closed
        assert path.stat().st_size == IOBase(path).size
