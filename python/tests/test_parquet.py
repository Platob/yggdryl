"""Apache Parquet over a handle, and the file another reader gets."""

from __future__ import annotations

import pathlib

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from yggdryl import IOBase, RecordOptions

ROW_COUNT = 1_000
SCHEMA = pa.schema(
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
        schema=SCHEMA,
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
        file.write_arrow_batch_reader(_table())

        assert file.read_arrow_batch_reader().read_all() == _table()
        assert file.read_arrow_field().name == "row"
        assert len(file.read_arrow_field().data_type) == 3

    def test_an_absent_file_holds_no_batches(self, file: IOBase) -> None:
        assert not file.exists()
        assert file.read_arrow_batch_reader().read_all().num_rows == 0


class TestColumnPushdownAlsoSkipsReading:
    """A column chunk is separately addressable, so a projection reads less."""

    def test_a_projected_read_materializes_less(self, file: IOBase) -> None:
        file.write_arrow_batch_reader(_table())
        options = file.record_options()
        options.schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        whole = file.read_arrow_batch_reader().read_all()
        subset = file.read_arrow_batch_reader(options=options).read_all()

        assert subset.column_names == ["id"]
        assert subset.nbytes * 2 < whole.nbytes

    def test_the_cast_reorders_what_the_projection_only_selected(
        self, file: IOBase
    ) -> None:
        file.write_arrow_batch_reader(_table())
        options = file.record_options()
        options.schema = pa.schema(
            [pa.field("venue", pa.string()), pa.field("id", pa.int64(), nullable=False)]
        )

        # The mask selects without reordering; the cast produces the declared
        # order, so the caller sees the shape it asked for.
        assert file.read_arrow_batch_reader(options=options).schema.names == [
            "venue",
            "id",
        ]


class TestOptions:
    """The settings a file format has that a stream does not."""

    def test_row_groups_and_footer_metadata_reach_the_file(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        options = file.record_options()
        options.max_row_group_size = 100
        options.key_value_metadata = {"writer": "yggdryl"}

        with file as handle:
            handle.write_arrow_batch_reader(_table(), options=options)

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

            handle.write_arrow_batch_reader(_table(), options=options)
            # Nothing on the read side names it: the footer records the codec.
            assert handle.read_arrow_batch_reader().read_all().num_rows == ROW_COUNT
            sizes.append(handle.size)

        assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    def test_an_unknown_compression_is_refused(self, file: IOBase) -> None:
        options = file.record_options()

        # The parquet crate's own parser is what accepts the spelling.
        with pytest.raises(ValueError, match="compression"):
            options.compression = "definitely not a codec"

    def test_a_batch_size_bounds_what_a_read_yields(self, file: IOBase) -> None:
        file.write_arrow_batch_reader(_table())
        options = file.record_options()
        options.batch_size = 250

        counts = [batch.num_rows for batch in file.read_arrow_batch_reader(options=options)]
        assert counts == [250, 250, 250, 250]

    def test_an_outer_content_coding_is_rejected_rather_than_doubled(
        self, tmp_path: pathlib.Path
    ) -> None:
        # Parquet compresses pages internally, so a gzip suffix would produce a
        # file no Parquet reader could open.
        compressed = IOBase(tmp_path / "trades.parquet.gz")

        with pytest.raises(ValueError, match="compresses internally"):
            compressed.write_arrow_batch_reader(_table())


class TestWhatAnotherReaderSees:
    """The bytes are Parquet, so PyArrow reads them and we read PyArrow's."""

    def test_pyarrow_reads_what_this_wrote(
        self, file: IOBase, tmp_path: pathlib.Path
    ) -> None:
        with file as handle:
            handle.write_arrow_batch_reader(_table())

        # Closing published the file at its exact length; a footer-first reader
        # needs that, because it looks for the magic bytes at the end.
        assert pq.read_table(tmp_path / "trades.parquet") == _table()

    def test_this_reads_what_pyarrow_wrote(self, tmp_path: pathlib.Path) -> None:
        pq.write_table(_table(), tmp_path / "external.parquet")

        handle = IOBase(tmp_path / "external.parquet")
        assert handle.read_arrow_batch_reader().read_all() == _table()
        assert len(handle.read_arrow_field().data_type) == 3

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
            handle.write_arrow_batch_reader([rows])

        stored = file.read_arrow_field().data_type[0]
        assert stored.parquet_field_id == 17
        assert pq.ParquetFile(tmp_path / "trades.parquet").schema_arrow.field(
            "id"
        ).metadata == {b"PARQUET:field_id": b"17"}


class TestWritesAndMerges:
    """The three methods behave here exactly as they do on a stream."""

    def test_appending_rewrites_the_file(self, file: IOBase) -> None:
        rows = pa.record_batch(
            {"id": [1], "symbol": ["AAPL"], "venue": ["XNAS"]}, schema=SCHEMA
        )

        file.append_arrow_batch_reader([rows])
        file.append_arrow_batch_reader([rows])

        assert file.read_arrow_batch_reader().read_all().num_rows == 2

    def test_a_match_key_merges_into_the_file(self, file: IOBase) -> None:
        file.write_arrow_batch_reader(
            [
                pa.record_batch(
                    {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
                    schema=SCHEMA,
                )
            ]
        )
        options = file.record_options()
        options.merge_by_names = ["id"]

        file.write_arrow_batch_reader(
            [
                pa.record_batch(
                    {"id": [2, 3], "symbol": ["MSFT.O", "NVDA"], "venue": ["XNAS", "XNAS"]},
                    schema=SCHEMA,
                )
            ],
            options=options,
        )

        table = file.read_arrow_batch_reader().read_all()
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
        options.schema = schema
        for venue in ("XNAS", "XNYS"):
            (tmp_path / f"venue={venue}").mkdir()
        lake = IOBase(tmp_path)
        lake.write_arrow_batch_reader(
            [
                pa.record_batch(
                    {"price": [10, 20, 30], "venue": ["XNAS", "XNAS", "XNYS"]},
                    schema=schema,
                )
            ],
            options=options,
        )

        # The column lives in the directory name, not in the file.
        leaf = lake / "venue=XNAS" / "part-0.parquet"
        assert len(leaf.read_arrow_field().data_type) == 1

        restored = lake.read_arrow_batch_reader(options=options).read_all()
        assert restored.column("venue").to_pylist() == ["XNAS", "XNAS", "XNYS"]

        selected = lake.children_where({"venue": "XNAS"})
        assert len(selected) == 1
