"""The record media surface: what a handle answers for IPC and for Parquet."""

from __future__ import annotations

import copy
import pathlib
import pickle
import struct

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from yggdryl import DataType, Field, IOBase, MediaType, MimeType, RecordOptions
from yggdryl.holder import LocalPath

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
        assert options.select.is_all
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
        assert LocalPath(tmp_path / "trades.arrows.gz").read_bytes()[:2] == b"\x1f\x8b"
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
        options.select = ["id"]
        options.filter = "id > 1"
        options.merge_by = ["id"]

        _, (state,) = options.__reduce__()
        assert state["name"] == "trade"
        assert state["field"] == options.field
        assert state["select"] == "id"
        assert state["filter"] == "id > 1"
        assert state["merge_by"] == "id"
        assert state["batch_row_size"] == 16
        assert not {"dtype", "metadata", "root_name", "batch_size"} & state.keys()

        restored = pickle.loads(pickle.dumps(options))
        assert restored == options
        assert restored.field == options.field
        assert restored.select == options.select
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


KNOWN_MIME_TYPES = {
    "OCTET_STREAM": "application/octet-stream",
    "JSON": "application/json",
    "JSON_LINES": "application/x-ndjson",
    "YAML": "application/yaml",
    "TOML": "application/toml",
    "CSV": "text/csv",
    "TSV": "text/tab-separated-values",
    "PARQUET": "application/vnd.apache.parquet",
    "ARROW_FILE": "application/vnd.apache.arrow.file",
    "ARROW_STREAM": "application/vnd.apache.arrow.stream",
    "AVRO": "application/avro",
    "ORC": "application/vnd.apache.orc",
    "PUFFIN": "application/vnd.apache.puffin",
    "PLAIN_TEXT": "text/plain",
    "ULLINK": "text/ullink",
    "FIX": "text/fix",
    "FIXUL": "text/fixul",
    "FIXML": "text/fixml",
    "MARKDOWN": "text/markdown",
    "HTML": "text/html",
    "CSS": "text/css",
    "JAVASCRIPT": "text/javascript",
    "XML": "application/xml",
    "HTTP": "message/http",
    "PDF": "application/pdf",
    "CBOR": "application/cbor",
    "MESSAGE_PACK": "application/vnd.msgpack",
    "PROTOBUF": "application/protobuf",
    "SQLITE3": "application/vnd.sqlite3",
    "PNG": "image/png",
    "JPEG": "image/jpeg",
    "GIF": "image/gif",
    "WEBP": "image/webp",
    "SVG": "image/svg+xml",
    "MP3": "audio/mpeg",
    "WAV": "audio/wav",
    "OGG": "audio/ogg",
    "FLAC": "audio/flac",
    "MP4": "video/mp4",
    "WEBM": "video/webm",
    "WOFF": "font/woff",
    "WOFF2": "font/woff2",
    "TTF": "font/ttf",
    "OTF": "font/otf",
    "XLS": "application/vnd.ms-excel",
    "XLSX": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "ODS": "application/vnd.oasis.opendocument.spreadsheet",
    "DOC": "application/msword",
    "DOCX": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "GZIP": "application/gzip",
    "ZSTD": "application/zstd",
    "BROTLI": "application/x-brotli",
    "ZLIB": "application/zlib",
    "COMPRESS": "application/x-compress",
    "BZIP2": "application/x-bzip2",
    "XZ": "application/x-xz",
    "LZ4": "application/x-lz4",
    "SNAPPY": "application/x-snappy-framed",
    "ZIP": "application/zip",
    "SEVEN_ZIP": "application/x-7z-compressed",
    "RAR": "application/vnd.rar",
    "TAR": "application/x-tar",
}


def test_mime_type_complete_known_constants_and_default() -> None:
    assert len(KNOWN_MIME_TYPES) == 62
    assert MimeType() == MimeType.OCTET_STREAM
    values = []
    for name, canonical in KNOWN_MIME_TYPES.items():
        value = getattr(MimeType, name)
        assert type(value) is MimeType
        assert str(value) == canonical
        assert value.is_known()
        assert MimeType.from_str(canonical) == value
        values.append(value)
    assert len(set(values)) == len(values)


def test_io_identity_is_derived_from_the_unencoded_mime_value() -> None:
    assert MimeType.CSV.is_io()
    assert MediaType.from_parts(MimeType.CSV, [MimeType.GZIP]).is_io()
    directory = MimeType("inode/directory")
    assert not directory.is_io()
    assert not MediaType(directory).is_io()


def test_mime_type_native_parsing_views_and_value_protocols(
    tmp_path: pathlib.Path,
) -> None:
    custom = MimeType("Application/Vnd.Example+JSON")
    assert str(custom) == "application/vnd.example+json"
    assert custom.top_level == "application"
    assert custom.subtype == "vnd.example+json"
    assert custom.structured_suffix == "json"
    assert custom.extension == "json"
    assert custom.content_coding is None
    assert custom.format == "json"
    assert custom.is_structured()

    assert MimeType.from_extension(".json") == MimeType.JSON
    assert MimeType.from_extension(".puffin") == MimeType.PUFFIN
    assert MimeType.PUFFIN.extension == "puffin"
    assert MimeType.PUFFIN.is_binary()
    assert MimeType.PUFFIN.is_structured()
    assert not MimeType.PUFFIN.is_tabular()
    assert MimeType.from_path(tmp_path / "events.csv") == MimeType.CSV
    assert MimeType.from_content_type('Application/JSON; charset="utf-8"') == MimeType.JSON
    assert MimeType.from_content_coding("gzip") == MimeType.GZIP
    assert MimeType.GZIP.content_coding == "gzip"
    assert MimeType.JSON.format == "json"
    assert MimeType.JSON == copy.copy(MimeType.JSON)
    assert MimeType.JSON == copy.deepcopy(MimeType.JSON)
    assert MimeType.JSON == pickle.loads(pickle.dumps(MimeType.JSON))
    assert MimeType.from_json(MimeType.JSON.into_json()) == MimeType.JSON
    assert hash(MimeType.JSON) == hash(MimeType.from_value(MimeType.JSON))
    assert MimeType.JSON.stable_hash() == MimeType.from_str("application/json").stable_hash()

    with pytest.raises(ValueError):
        MimeType.from_content_type("application/json; charset")
    with pytest.raises(ValueError):
        MimeType.from_content_coding("identity")
    with pytest.raises(TypeError):
        MimeType.from_value(object())


def test_media_type_default_compound_headers_and_detached_snapshots() -> None:
    assert MediaType().base == MimeType.OCTET_STREAM
    assert MediaType().encodings == ()
    media = MediaType.from_parts(MimeType.CSV, (value for value in ["gzip", "zstd"]))
    assert str(media) == "text/csv;encodings=application/gzip,application/zstd"
    assert media.base == MimeType.CSV
    assert media.encodings == (MimeType.GZIP, MimeType.ZSTD)
    assert media.encoding == MimeType.ZSTD
    assert media.extensions == ["csv", "gz", "zst"]
    assert list(media) == [MimeType.GZIP, MimeType.ZSTD]
    assert media[-1] == MimeType.ZSTD
    assert MimeType.GZIP in media
    assert len(media) == 2

    detached = media.encodings
    iterator = iter(media)
    media.clear_encodings()
    assert detached == (MimeType.GZIP, MimeType.ZSTD)
    assert list(iterator) == [MimeType.GZIP, MimeType.ZSTD]
    assert media.encodings == ()

    from_headers = MediaType.from_content_headers(
        "Application/JSON; Charset=utf-8", " gzip ,\tbr, compress "
    )
    assert from_headers.base == MimeType.JSON
    assert from_headers.encodings == (
        MimeType.GZIP,
        MimeType.BROTLI,
        MimeType.COMPRESS,
    )
    assert from_headers.charset == "utf-8"
    assert MediaType.from_str("text/csv").charset is None
    assert MediaType.from_content_headers().base == MimeType.OCTET_STREAM
    assert MediaType.from_path("events.json.gz").encodings == (MimeType.GZIP,)
    assert MediaType.from_file_name("events.csv.zst").base == MimeType.CSV
    assert MediaType.from_extension("json").base == MimeType.JSON
    assert MediaType.from_extensions(value for value in ["json", "gz"]) == MediaType.from_parts(
        MimeType.JSON, [MimeType.GZIP]
    )
    relative = MediaType.from_str("folder/orders.csv.gz")
    assert relative.base == MimeType.CSV
    assert relative.encodings == (MimeType.GZIP,)
    assert MediaType.from_str("application/vnd.example.report+json").base == MimeType(
        "application/vnd.example.report+json"
    )
    assert MediaType.from_extension(" .TBZ2\t") == MediaType.from_parts(
        MimeType.TAR, [MimeType.BZIP2]
    )


def test_media_type_mutation_is_atomic_hash_locked_and_round_trips() -> None:
    media = MediaType.from_parts(MimeType.JSON, [MimeType.GZIP])
    before = str(media)

    def invalid_values() -> object:
        yield MimeType.ZSTD
        yield object()

    with pytest.raises(TypeError):
        media.set_encodings(invalid_values())
    assert str(media) == before
    with pytest.raises(ValueError):
        media.push_encoding(MimeType.ZIP)
    assert str(media) == before
    with pytest.raises(TypeError):
        media.set_base(object())
    assert str(media) == before
    media.set_base(MimeType.CSV)
    media.set_encodings(value for value in [MimeType.GZIP, MimeType.ZSTD])
    media.push_encoding(MimeType.BROTLI)
    assert media.base == MimeType.CSV
    assert media.encodings == (MimeType.GZIP, MimeType.ZSTD, MimeType.BROTLI)
    assert MediaType.from_json(media.into_json()) == media
    assert copy.copy(media) == media
    assert copy.deepcopy(media) == media
    assert pickle.loads(pickle.dumps(media)) == media
    assert media.stable_hash() == MediaType.from_str(str(media)).stable_hash()
    assert hash(media) == hash(MediaType.from_str(str(media)))

    keyed = {media: "stable"}
    with pytest.raises(TypeError, match="hashed MediaType is frozen"):
        media.set_base(MimeType.JSON)
    with pytest.raises(TypeError, match="hashed MediaType is frozen"):
        media.clear_encodings()
    assert keyed[media] == "stable"

    copied = copy.copy(media)
    copied.set_base(MimeType.JSON)
    assert copied != media
    restored = pickle.loads(pickle.dumps(media))
    restored.clear_encodings()
    assert restored != media

    with pytest.raises(TypeError, match="iterable"):
        MediaType.from_parts(MimeType.JSON, "gzip")
    with pytest.raises(TypeError, match="iterable"):
        MediaType(media).set_encodings("gzip")
    with pytest.raises(ValueError):
        MediaType.from_content_headers(None, "identity")

ROW_COUNT = 1_000
SCHEMA_parquet = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
        pa.field("venue", pa.string()),
    ]
)


def _table() -> pa.Table:
    """Build a table wide enough for a projection to be worth measuring."""
    return pa.table(
        {
            "id": list(range(ROW_COUNT)),
            "symbol": ["AAPL"] * ROW_COUNT,
            "venue": ["XNAS"] * ROW_COUNT,
        },
        schema=SCHEMA_parquet,
    )


@pytest.fixture
def file(tmp_path: pathlib.Path) -> IOBase:
    """A handle whose name says it holds a Parquet file."""
    return IOBase(tmp_path / "trades.parquet")


class TestTheSameTwoMethods:
    """Parquet is reached through the record surface, not its own one."""

    def test_the_media_type_names_the_encoding(self, file: IOBase) -> None:
        options = file.record_options()

        assert str(options.mime_type) == "application/vnd.apache.parquet"
        assert options.max_row_group_size == 1_048_576

    def test_a_round_trip_keeps_every_row_and_the_root(self, file: IOBase) -> None:
        file.overwrite_arrow_table(_table())

        assert file.read_arrow_reader().read_all() == _table()
        assert file.read_arrow_field().name == "row"
        assert len(file.read_arrow_field().dtype) == 3

    def test_an_absent_file_holds_no_batches(self, file: IOBase) -> None:
        assert not file.exists()
        assert file.read_arrow_reader().read_all().num_rows == 0


class TestColumnPushdownAlsoSkipsReading:
    """A column chunk is separately addressable, so a projection reads less."""

    def test_a_projected_read_materializes_less(self, file: IOBase) -> None:
        file.overwrite_arrow_table(_table())
        options = file.record_options()
        options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        whole = file.read_arrow_reader().read_all()
        subset = file.read_arrow_reader(options=options).read_all()

        assert subset.column_names == ["id"]
        assert subset.nbytes * 2 < whole.nbytes

    def test_the_cast_reorders_what_the_projection_only_selected(
        self, file: IOBase
    ) -> None:
        file.overwrite_arrow_table(_table())
        options = file.record_options()
        options.field = pa.schema(
            [pa.field("venue", pa.string()), pa.field("id", pa.int64(), nullable=False)]
        )

        # The mask selects without reordering; the cast produces the declared
        # order, so the caller sees the shape it asked for.
        assert file.read_arrow_reader(options=options).schema.names == [
            "venue",
            "id",
        ]


class TestParquetOptions:
    """The settings a file format has that a stream does not."""

    def test_row_groups_and_footer_metadata_reach_the_file(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        options = file.record_options()
        options.max_row_group_size = 100
        options.key_value_metadata = {"writer": "yggdryl"}

        with file as handle:
            handle.overwrite_arrow_table(_table(), options=options)

        written = pq.ParquetFile(tmp_path / "trades.parquet")
        assert written.num_row_groups == ROW_COUNT // 100
        assert written.metadata.metadata[b"writer"] == b"yggdryl"
        assert options.key_value_metadata == {"writer": "yggdryl"}

    def test_page_compression_is_named_the_way_the_format_names_it(
        self, tmp_path: pathlib.Path
    ) -> None:
        sizes = []
        for compression in ("uncompressed", "snappy", "zstd(1)"):
            handle = IOBase(tmp_path / f"trades-{compression}.parquet")
            options = handle.record_options()
            options.compression = compression
            assert options.compression == compression

            handle.overwrite_arrow_table(_table(), options=options)
            # Nothing on the read side names it: the footer records the codec.
            assert handle.read_arrow_reader().read_all().num_rows == ROW_COUNT
            sizes.append(handle.size)

        assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    def test_an_unknown_compression_is_refused(self, file: IOBase) -> None:
        options = file.record_options()

        # The parquet crate's own parser is what accepts the spelling.
        with pytest.raises(ValueError, match="compression"):
            options.compression = "definitely not a codec"

    def test_a_batch_row_size_bounds_what_a_read_yields(self, file: IOBase) -> None:
        file.overwrite_arrow_table(_table())
        options = file.record_options()
        options.batch_row_size = 250

        counts = [batch.num_rows for batch in file.read_arrow_reader(options=options)]
        assert counts == [250, 250, 250, 250]

    def test_an_outer_content_coding_is_rejected_rather_than_doubled(
        self, tmp_path: pathlib.Path
    ) -> None:
        # Parquet compresses pages internally, so a gzip suffix would produce a
        # file no Parquet reader could open.
        compressed = IOBase(tmp_path / "trades.parquet.gz")

        with pytest.raises(ValueError, match="compresses internally"):
            compressed.overwrite_arrow_table(_table())


class TestStatistics:
    """Footer metadata stays cheap; WKB recomputation names its scan."""

    def test_footer_statistics_cross_as_native_scalars(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        options = file.record_options()
        options.max_row_group_size = 250
        # A sequence is used on read so duplicate footer keys would survive.
        options.key_value_metadata = {"writer": "python"}
        file.overwrite_arrow_table(_table(), options=options)

        statistics = file.read_parquet_statistics()
        external = pq.ParquetFile(tmp_path / "trades.parquet").metadata

        assert statistics["num_rows"] == external.num_rows == ROW_COUNT
        assert statistics["created_by"]
        assert len(statistics["row_groups"]) == external.num_row_groups == 4
        assert {entry["key"]: entry["value"] for entry in statistics["key_value_metadata"]}[
            "writer"
        ] == "python"
        first = statistics["row_groups"][0]
        assert first["num_rows"] == 250
        identifier = next(column for column in first["columns"] if column["path"] == "id")
        assert isinstance(identifier["min_bytes"], bytes)
        assert isinstance(identifier["max_bytes"], bytes)

    def test_non_parquet_media_is_refused_before_footer_parsing(
        self, tmp_path: pathlib.Path
    ) -> None:
        ipc = IOBase(tmp_path / "trades.arrows")

        with pytest.raises(ValueError, match="expected Parquet media"):
            ipc.read_parquet_statistics()

        with pytest.raises(ValueError, match="expected Parquet media"):
            ipc.read_parquet_geospatial_statistics("shape")

    def test_geospatial_statistics_are_recomputed_from_the_projected_column(
        self, tmp_path: pathlib.Path
    ) -> None:
        def point(x: float, y: float) -> bytes:
            return b"\x01\x01\x00\x00\x00" + struct.pack("<dd", x, y)

        schema = pa.schema(
            [
                pa.field(
                    "shape",
                    pa.binary(),
                    metadata={
                        b"ARROW:extension:name": b"geoarrow.wkb",
                        b"ARROW:extension:metadata": b'{"crs":"OGC:CRS84"}',
                    },
                )
            ]
        )
        handle = IOBase(tmp_path / "shapes.parquet")
        handle.overwrite_arrow_table(
            pa.table({"shape": [point(1.0, 2.0), None, point(-3.0, 7.0)]}, schema=schema)
        )

        scanned = handle.read_parquet_geospatial_statistics("shape")
        footer = handle.read_parquet_statistics()["row_groups"][0]["columns"][0][
            "geospatial"
        ]

        assert scanned == footer
        assert scanned["bounding_box"] == {
            "mmax": None,
            "mmin": None,
            "xmax": 1.0,
            "xmin": -3.0,
            "ymax": 7.0,
            "ymin": 2.0,
            "zmax": None,
            "zmin": None,
        }
        assert scanned["geometry_types"] == [1]


class TestTheLimits:
    """`max_row_size` counts result rows and `max_byte_size` Arrow bytes."""

    def test_a_zero_row_limit_reads_the_schema_and_no_batches(
        self, file: IOBase
    ) -> None:
        file.overwrite_arrow_table(_table())

        options = file.record_options()
        options.max_row_size = 0
        reader = file.read_arrow_reader(options=options)
        # `0` is a valid ask, not an error: the shaped schema still answers.
        assert reader.schema.names == ["id", "symbol", "venue"]
        assert reader.read_all().num_rows == 0

    def test_a_row_limit_is_exact_over_a_bigger_file(self, file: IOBase) -> None:
        file.overwrite_arrow_table(_table())
        options = file.record_options()
        options.max_row_size = 10

        assert options.max_row_size == 10
        assert (
            file.read_arrow_reader(options=options).read_all().num_rows == 10
        )

    def test_a_small_byte_limit_still_yields_at_least_one_row(
        self, file: IOBase
    ) -> None:
        file.overwrite_arrow_table(_table())

        # One byte admits no whole row, but a bounded read must never be a
        # silent total loss: only a limit of zero yields nothing.
        options = file.record_options()
        options.max_byte_size = 1
        assert file.read_arrow_reader(options=options).read_all().num_rows == 1

    def test_a_limit_with_a_match_key_is_refused_naming_both(
        self, file: IOBase
    ) -> None:
        file.overwrite_arrow_table(_table())
        options = file.record_options()
        options.max_row_size = 10
        options.merge_by = ["id"]

        with pytest.raises(ValueError, match="max_row_size = 10.*merge_by"):
            file.merge_arrow_table(_table(), options=options)


class TestWhatAnotherReaderSees:
    """The bytes are Parquet, so PyArrow reads them and we read PyArrow's."""

    def test_pyarrow_reads_what_this_wrote(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        with file as handle:
            handle.overwrite_arrow_table(_table())

        # Closing published the file at its exact length; a footer-first reader
        # needs that, because it looks for the magic bytes at the end.
        assert pq.read_table(tmp_path / "trades.parquet") == _table()

    def test_this_reads_what_pyarrow_wrote(self, tmp_path: pathlib.Path) -> None:
        pq.write_table(_table(), tmp_path / "external.parquet")

        handle = IOBase(tmp_path / "external.parquet")
        assert handle.read_arrow_reader().read_all() == _table()
        assert len(handle.read_arrow_field().dtype) == 3

    def test_a_field_identifier_survives_the_round_trip(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        identified = pa.schema(
            [
                pa.field(
                    "id",
                    pa.int64(),
                    nullable=False,
                    metadata={b"PARQUET:field_id": b"17"},
                )
            ]
        )
        rows = pa.record_batch({"id": [1, 2]}, schema=identified)

        with file as handle:
            handle.overwrite_arrow_batch(rows)

        stored = file.read_arrow_field().dtype[0]
        assert stored.parquet_field_id == 17
        assert pq.ParquetFile(tmp_path / "trades.parquet").schema_arrow.field(
            "id"
        ).metadata == {b"PARQUET:field_id": b"17"}


class TestParquetWritesAndMerges:
    """The three methods behave here exactly as they do on a stream."""

    def test_appending_rewrites_the_file(self, file: IOBase) -> None:
        rows = pa.record_batch(
            {"id": [1], "symbol": ["AAPL"], "venue": ["XNAS"]}, schema=SCHEMA_parquet
        )

        file.append_arrow_batch(rows)
        file.append_arrow_batch(rows)

        assert file.read_arrow_reader().read_all().num_rows == 2

    def test_a_match_key_merges_into_the_file(self, file: IOBase) -> None:
        file.overwrite_arrow_batch(
            pa.record_batch(
                {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
                schema=SCHEMA_parquet,
            )
        )
        options = file.record_options()
        options.merge_by = ["id"]

        file.merge_arrow_batch(
            pa.record_batch(
                {"id": [2, 3], "symbol": ["MSFT.O", "NVDA"], "venue": ["XNAS", "XNAS"]},
                schema=SCHEMA_parquet,
            ),
            options=options,
        )

        table = file.read_arrow_reader().read_all()
        assert table.column("id").to_pylist() == [1, 2, 3]
        assert table.column("symbol").to_pylist() == ["AAPL", "MSFT.O", "NVDA"]

    def test_a_partitioned_lake_of_files_reads_as_one_table(
        self, tmp_path: pathlib.Path
    ) -> None:
        schema = pa.schema(
            [
                pa.field("price", pa.int64(), nullable=False),
                pa.field("venue", pa.string(), nullable=False),
            ]
        )
        options = RecordOptions("part.parquet")
        options.field = schema
        for venue in ("XNAS", "XNYS"):
            (tmp_path / f"venue={venue}").mkdir()
        lake = IOBase(tmp_path)
        lake.overwrite_arrow_batch(
            pa.record_batch(
                {"price": [10, 20, 30], "venue": ["XNAS", "XNAS", "XNYS"]},
                schema=schema,
            ),
            options=options,
        )

        # The column lives in the directory name, not in the file.
        leaf = lake / "venue=XNAS" / "part-0.parquet"
        assert len(leaf.read_arrow_field().dtype) == 1

        restored = lake.read_arrow_reader(options=options).read_all()
        assert restored.column("venue").to_pylist() == ["XNAS", "XNAS", "XNYS"]

        selected = list(lake.children_where({"venue": "XNAS"}))
        assert len(selected) == 1
