"""``IOBase`` and ``Url`` answer the questions ``pathlib.Path`` answers."""

from __future__ import annotations

import pathlib
import gzip as stdlib_gzip

import pyarrow as pa
import pyarrow.fs as pafs
import pytest

from yggdryl import IOBase, Url


@pytest.fixture
def lake(tmp_path: pathlib.Path) -> pathlib.Path:
    """Build a small Hive-partitioned lake with one private staging area."""
    root = tmp_path / "lake"
    for year in ("2024", "2025"):
        for month in ("01", "02"):
            leaf = root / f"year={year}" / f"month={month}"
            leaf.mkdir(parents=True)
            (leaf / "part-0.parquet").write_bytes(b"parquet")
            (leaf / "notes.txt").write_text("notes", encoding="utf-8")
    staging = root / ".staging"
    staging.mkdir()
    (staging / "part-0.parquet").write_bytes(b"draft")
    return root


class TestConstructionFromWhatIsAlreadyHeld:
    """A handle is built from whatever already names or holds the resource."""

    def test_an_open_file_captures_its_location(self, tmp_path: pathlib.Path) -> None:
        # A file object with a real filesystem name is a spelling of the
        # path, so the handle addresses the location - nothing is read.
        quotes = tmp_path / "quotes.json"
        quotes.write_text('{"symbol": "AAPL"}', encoding="utf-8")

        with quotes.open("rb") as stream:
            handle = IOBase(stream)

        assert handle.url is not None
        assert handle.name == "quotes.json"
        assert handle.read_text() == '{"symbol": "AAPL"}'

    def test_a_nameless_stream_captures_its_content(self) -> None:
        import io

        # A BytesIO names no location, so what it holds is what is taken.
        handle = IOBase(io.BytesIO(b"\x00\xff"))
        assert str(handle.url).startswith("mem://")
        assert handle.read_bytes() == b"\x00\xff"

        # A text stream captures as UTF-8 bytes.
        text = IOBase(io.StringIO("symbol,price\nAAPL,12.5\n"))
        assert text.read_text() == "symbol,price\nAAPL,12.5\n"

    def test_an_in_memory_handle_captures_content_and_media_type(self) -> None:
        source = IOBase.from_bytes(b'{"symbol": "AAPL"}')
        source.media_type = "application/json"

        captured = IOBase(source)

        assert captured.read_bytes() == b'{"symbol": "AAPL"}'
        assert str(captured.media_type) == str(source.media_type)
        # The capture is a copy: growing the source does not move the capture.
        source.write_bytes(b"{}")
        assert captured.read_bytes() == b'{"symbol": "AAPL"}'


class TestNativeHandleLayers:
    def test_buffered_reconfigures_one_core_page_cache_in_place(self) -> None:
        payload = bytes(range(256)) * 8
        handle = IOBase.from_bytes(payload)

        returned = handle.buffered(page_size=65, max_bytes=1, ttl=0.0)
        assert returned is handle
        assert handle.pread(60, 140) == payload[60:200]

        # Reconfiguration unwraps the first cache before installing the next,
        # and keeps the in-memory handle and its writes intact.
        assert handle.buffered(page_size=128, max_bytes=256, ttl=1.0) is handle
        handle.pwrite(127, b"updated")
        expected = payload[:127] + b"updated" + payload[134:]
        assert handle.read_bytes() == expected

        with pytest.raises(ValueError, match="ttl"):
            handle.buffered(ttl=float("nan"))
        assert handle.read_bytes() == expected

    def test_open_retains_media_metadata_until_close_then_refreshes(
        self, tmp_path: pathlib.Path
    ) -> None:
        location = tmp_path / "opened.arrows"
        replacement = tmp_path / "replacement.arrows"
        original = pa.table({"id": pa.array([1, 2], type=pa.int32())})
        changed = pa.table(
            {
                "id": pa.array([1, 2, 3], type=pa.int64()),
                "symbol": ["A", "B", "C"],
            }
        )
        filesystem = pafs.LocalFileSystem()
        handle = IOBase.from_arrow_fs(filesystem, location)
        handle.overwrite_arrow_table(original)
        IOBase(replacement).overwrite_arrow_table(changed)

        handle.open()
        assert handle.opened
        first = handle.read_arrow_field()
        assert [child.name for child in first.data_type] == ["id"]
        assert handle.row_size == 2

        # Replace the resource outside this handle. Repeated metadata reads in
        # the opened scope stay on the retained native media cache.
        IOBase.from_arrow_fs(filesystem, location).write_bytes(replacement.read_bytes())
        assert [child.name for child in handle.read_arrow_field().data_type] == ["id"]
        assert handle.row_size == 2

        handle.close()
        assert handle.closed
        assert [child.name for child in handle.read_arrow_field().data_type] == [
            "id",
            "symbol",
        ]
        assert handle.row_size == 3


class TestPathlibParity:
    """The same calls a ``Path`` answers, answered by the core."""

    def test_a_handle_reports_what_is_there(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert handle.exists()
        assert handle.is_dir()
        assert not handle.is_file()
        assert handle.name == "lake"

    def test_a_missing_location_is_empty_rather_than_an_error(
        self, tmp_path: pathlib.Path
    ) -> None:
        absent = IOBase(tmp_path / "absent.arrows")

        assert not absent.exists()
        # Reads skip, so probing a location needs no existence check first.
        assert absent.read_bytes() == b""
        assert absent.size == 0

    def test_a_handle_says_whether_it_holds_bytes_or_rows(
        self, lake: pathlib.Path, tmp_path: pathlib.Path
    ) -> None:
        notes = IOBase(tmp_path / "notes.txt")
        trades = IOBase(tmp_path / "trades.parquet")

        # The name is enough; neither location has been written to.
        assert notes.is_atomic()
        assert not notes.is_tabular()
        assert trades.is_tabular()
        assert not trades.is_atomic()

        # A folder is never one whole byte value, and reads as the table
        # beneath it.
        handle = IOBase(lake)
        assert not handle.is_atomic()
        assert handle.is_tabular()

    def test_io_capability_and_dimensions_come_from_core_media(
        self, tmp_path: pathlib.Path
    ) -> None:
        empty_folder = IOBase(tmp_path / "empty")
        empty_folder.mkdir()
        assert IOBase.from_bytes().kind == "memory"
        assert empty_folder.kind == "directory"
        assert IOBase(tmp_path / "missing").kind == "unknown"
        assert not empty_folder.is_io()
        assert IOBase(tmp_path / "notes.txt").is_io()

        def table(ids: list[int], wide: bool = False) -> pa.Table:
            columns: dict[str, list[object]] = {
                "id": ids,
                "venue": ["XNAS"] * len(ids),
            }
            if wide:
                columns["quantity"] = [10] * len(ids)
            return pa.table(columns)

        handle = IOBase(tmp_path / "dimensions.arrows")
        handle.overwrite_arrow_table(table([1, 2, 3, 4]))
        assert handle.kind == "file"
        assert handle.is_io()
        assert handle.row_size == 4
        assert handle.column_size == 2

        # A publication through an opened handle refreshes its row metadata.
        # Overwrite replaces rows, not the stored field: an extra incoming
        # column is safely completed onto the existing two-column shape.
        handle.open()
        assert (handle.row_size, handle.column_size) == (4, 2)
        handle.overwrite_arrow_table(table([9], wide=True))
        assert (handle.row_size, handle.column_size) == (1, 2)
        handle.close()

        typed_empty = IOBase(tmp_path / "typed-empty.arrows")
        typed_empty.overwrite_arrow_table(table([]))
        assert (typed_empty.row_size, typed_empty.column_size) == (0, 2)

        # A folder aggregates only leaves of its selected record encoding;
        # unrelated text never becomes a row in the table.
        lake = tmp_path / "dimensions-lake"
        IOBase(lake / "a.arrows").overwrite_arrow_table(table([1, 2]))
        IOBase(lake / "b.arrows").overwrite_arrow_table(table([3]))
        IOBase(lake / "notes.txt").write_text("not a table row")
        folder = IOBase(lake)
        assert (folder.row_size, folder.column_size) == (3, 2)

    def test_children_are_resolved_the_way_paths_are(self, lake: pathlib.Path) -> None:
        by_operator = IOBase(lake) / "year=2024" / "month=01" / "part-0.parquet"
        by_method = IOBase(lake).joinpath("year=2024", "month=01", "part-0.parquet")

        assert by_operator.name == by_method.name == "part-0.parquet"
        assert by_operator.read_bytes() == b"parquet"
        assert by_operator.parent.name == "month=01"

    def test_iterdir_skips_private_entries_by_default(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert sorted(entry.name for entry in handle.iterdir()) == [
            "year=2024",
            "year=2025",
        ]
        assert ".staging" in {entry.name for entry in handle.iterdir(include_private=True)}
        # Iterating the handle itself is iterdir, as it is for a Path.
        assert len(list(handle)) == 2

    def test_glob_and_rglob_select_the_same_leaves(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert len(list(handle.glob("**/*.parquet"))) == 4
        assert len(list(handle.rglob("*.parquet"))) == 4
        assert len(list(handle.glob("year=2024/**/*.parquet"))) == 2
        # One plain segment stays at one level, where there are no leaves.
        assert list(handle.glob("*.parquet")) == []

    def test_a_write_creates_and_a_read_returns_it(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "trades.txt")

        assert handle.write_text("AAPL") == 4
        assert handle.read_text() == "AAPL"
        assert handle.exists()
        assert handle.size == 4

        handle.unlink()
        assert handle.read_bytes() == b""

    def test_positional_access_needs_no_mode(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "positional.bin")
        handle.write_bytes(b"symbol,price")

        # Random access is the contract, so there is nothing to open or seek.
        assert handle.pread(0, 6) == b"symbol"
        handle.pwrite(0, b"SYMBOL")
        assert handle.read_bytes() == b"SYMBOL,price"
        assert handle.append(b"!") == 12

    def test_mkdir_and_touch_bring_a_location_into_being(
        self, tmp_path: pathlib.Path
    ) -> None:
        folder = IOBase(tmp_path / "nested" / "deep")
        folder.mkdir()
        assert folder.is_dir()

        leaf = folder / "empty.arrows"
        leaf.touch()
        assert leaf.exists()
        assert leaf.size == 0

        leaf.write_text("kept")
        leaf.touch()
        # `touch` never truncates an existing leaf, as it does not for a Path.
        assert leaf.read_text() == "kept"
        with pytest.raises(IsADirectoryError, match="got the directory"):
            folder.touch()

    def test_a_memory_handle_needs_no_location(self) -> None:
        handle = IOBase.from_bytes(b"AAPL")

        assert handle.read_bytes() == b"AAPL"
        # A buffer still has an identity, but not one the file system knows.
        assert handle.url.scheme == "mem"
        with pytest.raises(ValueError):
            handle.__fspath__()


class TestPartitions:
    """A Hive layout stores columns in the path, and the path is readable."""

    def test_a_leaf_knows_the_partitions_above_it(self, lake: pathlib.Path) -> None:
        leaf = IOBase(lake) / "year=2024" / "month=01" / "part-0.parquet"

        assert leaf.partitions == (("year", "2024"), ("month", "01"))

    def test_children_where_selects_the_parts_to_rewrite(
        self, lake: pathlib.Path
    ) -> None:
        handle = IOBase(lake)

        year = list(handle.children_where({"year": "2024"}))
        assert len(year) == 4
        assert all(entry.is_file() for entry in year)

        both = list(handle.children_where([("year", "2024"), ("month", "02")]))
        assert len(both) == 2
        assert list(handle.children_where({"year": "1999"})) == []
        # No filter is every leaf.
        assert len(list(handle.children_where({}))) == 8


class TestUrlPathlibParity:
    """A URL is a path with a scheme, so it answers the same questions."""

    def test_the_naming_properties_match_purepath(self) -> None:
        url = Url("file:///lake/trades/part-0.tar.gz")

        assert url.name == "part-0.tar.gz"
        assert url.stem == "part-0.tar"
        assert url.suffix == ".gz"
        assert url.suffixes == (".tar", ".gz")
        assert url.parts == ("lake", "trades", "part-0.tar.gz")
        assert url.is_absolute()

    def test_joining_matches_purepath(self) -> None:
        root = Url("file:///lake")

        assert str(root / "trades" / "part-0.arrows") == "file:///lake/trades/part-0.arrows"
        assert str(root.joinpath("trades", "part-0.arrows")) == "file:///lake/trades/part-0.arrows"
        assert str(root.joinpath(pathlib.PurePosixPath("trades"))) == "file:///lake/trades"

    def test_parents_run_from_the_closest_upwards(self) -> None:
        url = Url("file:///lake/trades/part-0.arrows")

        assert url.parent.name == "trades"
        assert [parent.name for parent in url.parents] == ["trades", "lake", ""]
        # A root is its own parent, which is what pathlib does.
        assert str(Url("file:///").parent) == "file:///"

    def test_renaming_matches_purepath(self) -> None:
        url = Url("file:///lake/part-0.arrows")

        assert Url(url).with_name("part-1.arrows").name == "part-1.arrows"
        assert Url(url).with_stem("part-1").name == "part-1.arrows"
        assert Url(url).with_suffix(".parquet").name == "part-0.parquet"
        assert Url(url).with_suffix("parquet").name == "part-0.parquet"
        assert Url(url).with_suffix("").name == "part-0"

    def test_matching_follows_the_gitignore_rule(self) -> None:
        url = Url("file:///lake/year=2024/part-0.parquet")

        assert url.match("*.parquet")
        assert url.match("lake/**/part-?.parquet")
        assert not url.match("lake/*.parquet")
        assert Url("file:///lake/**/*.parquet").is_glob()
        assert not url.is_glob()

    def test_relative_to_matches_purepath(self) -> None:
        root = Url("file:///lake")
        url = Url("file:///lake/year=2024/part-0.parquet")

        assert url.relative_to(root) == "year=2024/part-0.parquet"
        assert url.is_relative_to(root)
        assert not url.is_relative_to(Url("file:///other"))
        with pytest.raises(ValueError):
            url.relative_to(Url("file:///other"))

    def test_the_file_system_predicates_answer_for_a_local_url(
        self, lake: pathlib.Path
    ) -> None:
        url = Url(lake)

        assert url.exists()
        assert url.is_dir()
        assert not url.is_file()
        assert (url / "year=2024" / "month=01" / "part-0.parquet").is_file()
        assert (url / ".staging").is_private()

    def test_partitions_are_read_off_the_path(self) -> None:
        url = Url("file:///lake/year=2024/month=01/part-0.parquet")

        assert url.partitions == (("year", "2024"), ("month", "01"))
        assert url.partition("month") == "01"
        assert url.partition("day") is None


class TestReadLines:
    """Lines stream off any handle, decoded, one line at a time."""

    def test_plain_lines_including_the_unterminated_tail(
        self, tmp_path: pathlib.Path
    ) -> None:
        target = tmp_path / "rows.jsonl"
        target.write_bytes(b'{"id":1}\n{"id":2}\r\n{"id":3}')
        assert list(IOBase(target).read_lines()) == [
            '{"id":1}',
            '{"id":2}',
            '{"id":3}',
        ]

    def test_a_gzip_named_resource_streams_its_decoded_lines(
        self, tmp_path: pathlib.Path
    ) -> None:
        import gzip as stdlib_gzip

        target = tmp_path / "words.txt.gz"
        target.write_bytes(stdlib_gzip.compress(b"alpha\nbeta\n"))
        assert list(IOBase(target).read_lines()) == ["alpha", "beta"]

    def test_an_absent_resource_yields_no_lines(self, tmp_path: pathlib.Path) -> None:
        assert list(IOBase(tmp_path / "missing.txt").read_lines()) == []

    def test_the_iterator_outlives_the_handle(self, tmp_path: pathlib.Path) -> None:
        target = tmp_path / "kept.txt"
        target.write_bytes(b"kept\nalive\n")
        lines = IOBase(target).read_lines()
        assert list(lines) == ["kept", "alive"]

    def test_a_pattern_groups_lines_into_log_entries(
        self, tmp_path: pathlib.Path
    ) -> None:
        target = tmp_path / "app.log"
        target.write_bytes(
            (
                "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n"
                "  at frame one\n"
                "2024-02-01 10:00:01.000_000 [ii] [beta] fine\n"
            ).encode()
        )
        entries = list(
            IOBase(target).read_lines(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}")
        )
        assert entries == [
            "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one",
            "2024-02-01 10:00:01.000_000 [ii] [beta] fine",
        ]


LINE_PATTERN = (
    r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*"
    r" \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]"
)


class TestReadArrowLines:
    """Matched line records project into a lazy pyarrow.RecordBatchReader."""

    LOG = (
        "preamble carried from rotation\n"
        "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n"
        "  at frame one\n"
        "2024-02-01 10:00:01.500 [ii] [beta] fill 100 @ 187.23\n"
    )

    def test_records_arrive_as_typed_rows(self, tmp_path: pathlib.Path) -> None:
        import pyarrow as pa

        target = tmp_path / "app.log"
        target.write_bytes(self.LOG.encode())
        reader = IOBase(target).read_arrow_lines(LINE_PATTERN)
        assert isinstance(reader, pa.RecordBatchReader)
        table = reader.read_all()
        assert table.column_names == [
            "url",
            "rownum",
            "date",
            "time",
            "unix",
            "hash",
            "header",
            "message",
            "offset",
            "lines",
            "level",
            "logger",
        ]
        rows = table.to_pydict()
        assert rows["rownum"] == [1, 2, 3]
        assert rows["message"] == [
            "preamble carried from rotation",
            "boom\n  at frame one",
            "fill 100 @ 187.23",
        ]
        assert rows["level"] == [None, "ee", "ii"]
        # The preamble has no header, so its timestamp columns are null; the
        # matched rows carry naive nanoseconds since the epoch.
        assert rows["unix"] == [None, 1_706_781_600_000_000_000, 1_706_781_601_500_000_000]
        assert table.schema.field("unix").type == pa.int64()
        assert table.schema.field("time").type == pa.time64("us")

    def test_a_gzip_log_reads_identically(self, tmp_path: pathlib.Path) -> None:
        import gzip as stdlib_gzip

        plain = tmp_path / "app.log"
        plain.write_bytes(self.LOG.encode())
        coded = tmp_path / "app.log.gz"
        coded.write_bytes(stdlib_gzip.compress(self.LOG.encode()))

        from_plain = IOBase(plain).read_arrow_lines(LINE_PATTERN).read_all()
        from_coded = IOBase(coded).read_arrow_lines(LINE_PATTERN).read_all()
        # Identical apart from the url column that names each resource.
        assert from_coded.drop_columns(["url"]) == from_plain.drop_columns(["url"])

    def test_a_folder_streams_leaves_with_custom_fields(
        self, tmp_path: pathlib.Path
    ) -> None:
        logs = tmp_path / "logs"
        logs.mkdir()
        (logs / "a.log").write_text("2024-02-01 10:00:00 [ee] [a] one\n")
        (logs / "b.log").write_text("2024-02-01 11:00:00 [ii] [b] two\n")

        table = (
            IOBase(logs)
            .read_arrow_lines(
                LINE_PATTERN,
                batch_size=16,
                custom_fields={"venue": "XNAS", "session": 7},
            )
            .read_all()
        )
        rows = table.to_pydict()
        # Leaves read name-sorted, each restarting rownum at 1.
        assert rows["rownum"] == [1, 1]
        assert rows["message"] == ["one", "two"]
        assert rows["venue"] == ["XNAS", "XNAS"]
        assert rows["session"] == [7, 7]
        assert [url.rsplit("/", 1)[-1] for url in rows["url"]] == ["a.log", "b.log"]

    def test_absence_reads_as_zero_rows_with_the_schema(
        self, tmp_path: pathlib.Path
    ) -> None:
        reader = IOBase(tmp_path / "missing.log").read_arrow_lines(LINE_PATTERN)
        table = reader.read_all()
        assert table.num_rows == 0
        assert table.column_names[:2] == ["url", "rownum"]

    def test_named_captures_infer_and_declare_typed_columns(
        self, tmp_path: pathlib.Path
    ) -> None:
        import decimal

        import pyarrow as pa

        target = tmp_path / "typed.log"
        target.write_text("2024-02-01 10:00:00 [42] (info) qty=1.50\n")
        pattern = (
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}"
            r" \[(?<thread_id>\d+)\] \((?<log_level>\w+)\) qty=(?<qty>[0-9.]+)"
        )
        # `thread_id` types itself off its `\d+` sub-pattern; `qty` is
        # declared - a type expression, exactly as everywhere else.
        table = (
            IOBase(target)
            .read_arrow_lines(pattern, capture_types={"qty": "decimal(9, 2)"})
            .read_all()
        )
        assert table.schema.field("thread_id").type == pa.int64()
        assert table.schema.field("log_level").type == pa.string()
        assert table.schema.field("qty").type == pa.decimal128(9, 2)
        rows = table.to_pydict()
        assert rows["thread_id"] == [42]
        assert rows["log_level"] == ["info"]
        assert rows["qty"] == [decimal.Decimal("1.50")]

    def test_the_schema_builder_answers_without_a_reader(
        self, tmp_path: pathlib.Path
    ) -> None:
        from yggdryl import field_from_pattern

        pattern = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\d+)\]"
        schema = field_from_pattern(pattern, custom_fields={"venue": "XNAS"})
        assert schema.name == "row"
        assert str(schema.data_type["thread_id"].data_type) == "int64"
        assert str(schema.data_type["venue"].data_type) == "utf8"

        # The reader emits exactly the built schema, resource or none.
        reader = IOBase(tmp_path / "missing.log").read_arrow_lines(
            pattern, custom_fields={"venue": "XNAS"}
        )
        assert [field.name for field in reader.schema] == [
            child.name for child in schema.data_type
        ]

    def test_an_in_memory_handle_parses_like_a_file(self) -> None:
        from yggdryl import IOBase as Handle

        handle = Handle.from_bytes(self.LOG.encode())
        table = handle.read_arrow_lines(LINE_PATTERN).read_all()
        assert table.num_rows == 3
        assert table.to_pydict()["level"] == [None, "ee", "ii"]

    def test_any_mapping_shape_names_custom_columns(
        self, tmp_path: pathlib.Path
    ) -> None:
        import types

        target = tmp_path / "app.log"
        target.write_text("2024-02-01 10:00:00 [ee] [a] one\n")
        table = (
            IOBase(target)
            .read_arrow_lines(
                LINE_PATTERN,
                custom_fields=types.MappingProxyType({"venue": "XNAS"}),
            )
            .read_all()
        )
        assert table.to_pydict()["venue"] == ["XNAS"]

    def test_an_unspellable_custom_column_is_rejected_up_front(
        self, tmp_path: pathlib.Path
    ) -> None:
        import datetime

        # A duration column has no Iceberg spelling, so the projection
        # refuses it before any resource is read - never at append time.
        with pytest.raises(ValueError, match="Iceberg can express"):
            IOBase(tmp_path / "app.log").read_arrow_lines(
                LINE_PATTERN,
                custom_fields={"age": datetime.timedelta(seconds=1)},
            )


class TestScans:
    """Lazy scans over any holder: native pushdown when possible."""

    def test_scan_polars_is_a_lazy_frame_over_a_local_leaf(
        self, tmp_path: pathlib.Path
    ) -> None:
        pl = __import__("pytest").importorskip("polars")
        import pyarrow as pa

        handle = IOBase(tmp_path / "trades.parquet")
        handle.overwrite_arrow_table(pa.table({"id": [1, 2], "venue": ["XNAS", "XPAR"]}))

        lazy = handle.scan_polars()
        assert isinstance(lazy, pl.LazyFrame)
        # Projection stays lazy and pushes down into the native scan.
        assert lazy.select("id").collect().to_series().to_list() == [1, 2]

        # A memory holder answers too, through the native reader.
        buffered = IOBase.from_bytes(handle.read_bytes())
        buffered.media_type = "application/vnd.apache.parquet"
        assert isinstance(buffered.scan_polars(), pl.LazyFrame)

    def test_scan_arrow_is_a_dataset_scanner(self, tmp_path: pathlib.Path) -> None:
        import pyarrow as pa
        import pyarrow.dataset as pads

        # An Arrow *stream* is not a dataset format, so it streams through
        # the native reader; the file format scans natively.
        stream = IOBase(tmp_path / "trades.arrows")
        stream.overwrite_arrow_table(pa.table({"id": [3, 4]}))
        scanner = stream.scan_arrow()
        assert isinstance(scanner, pads.Scanner)
        assert scanner.to_table().num_rows == 2

        handle = IOBase(tmp_path / "trades.parquet")
        handle.overwrite_arrow_table(pa.table({"id": [5, 6, 7]}))
        native = handle.scan_arrow()
        assert isinstance(native, pads.Scanner)
        assert native.to_table().num_rows == 3

    def test_a_scan_that_was_told_something_answers_for_it(
        self, tmp_path: pathlib.Path
    ) -> None:
        """The fast path hands a path to another engine, which was told nothing.

        A local Parquet leaf is the one shape where a scan can be answered by
        pyarrow or polars opening the file themselves, and that answer is only
        the same answer when the call carried no options - otherwise the
        projection, the filter, and the refusal of a column that is not there
        all go missing while the scan still succeeds.
        """
        import pyarrow as pa

        handle = IOBase(tmp_path / "trades.parquet")
        handle.overwrite_arrow_table(
            pa.table(
                {
                    "venue": ["XNAS", "XPAR", "XNAS"],
                    "id": pa.array([1, 2, 3], pa.int64()),
                }
            )
        )

        selected = handle.record_options()
        selected.select_by_names = ["id"]
        assert handle.scan_arrow(options=selected).to_table().schema.names == ["id"]
        filtered = handle.record_options()
        filtered.filter_partitions = [("venue", "XNAS")]
        assert (
            handle.scan_arrow(options=filtered).to_table().num_rows == 2
        )
        # The same refusal the eager read gives, rather than every column.
        absent = handle.record_options()
        absent.select_by_names = ["absent"]
        with pytest.raises(ValueError, match="absent"):
            handle.scan_arrow(options=absent).to_table()

        pl = pytest.importorskip("polars")
        lazy = handle.scan_polars(options=selected)
        assert isinstance(lazy, pl.LazyFrame)
        assert lazy.collect().columns == ["id"]


class TestCursor:
    """One position over the same handle, advancing with reads and writes."""

    def test_a_cursor_shares_the_handle_and_owns_its_position(self) -> None:
        handle = IOBase.from_bytes()
        cursor = handle.cursor()

        assert cursor.write(b"symbol,") == 7
        assert cursor.write(b"price\n") == 6
        assert cursor.tell() == 13
        # The write landed on the handle itself, not on a copy.
        assert handle.read_bytes() == b"symbol,price\n"

        assert cursor.seek(7) == 7
        assert cursor.read(5) == b"price"
        assert cursor.seek(-6, 2) == 7
        assert cursor.read() == b"price\n"
        assert cursor.read() == b""

        # Two cursors advance independently.
        ahead = handle.cursor(7)
        assert ahead.read(5) == b"price"
        assert cursor.tell() == 13


class TestByteStreams:
    """Byte streams stay lazy, bounded, and tied to their native source."""

    def test_the_default_stream_starts_at_zero_and_uses_64_kib_chunks(self) -> None:
        import inspect

        payload = bytes(range(256)) * 257
        chunks = list(IOBase.from_bytes(payload).pstream_bytes())

        assert inspect.signature(IOBase.pstream_bytes).parameters[
            "batch_size"
        ].default == 65536
        assert [len(chunk) for chunk in chunks] == [64 * 1024, 256]
        assert b"".join(chunks) == payload

    def test_an_explicit_position_and_batch_size_shape_the_chunks(self) -> None:
        handle = IOBase.from_bytes(b"0123456789")

        assert list(handle.pstream_bytes(2, 3)) == [b"234", b"567", b"89"]
        assert list(handle.pstream_bytes(100, 3)) == []

    def test_construction_is_lazy_and_the_iterator_keeps_its_handle_alive(self) -> None:
        handle = IOBase.from_bytes(b"before")
        stream = handle.pstream_bytes(batch_size=3)

        # Construction retained the handle but did not read or snapshot it.
        handle.write_bytes(b"after")
        del handle

        assert list(stream) == [b"aft", b"er"]

    def test_a_zero_batch_is_rejected_before_iteration(self) -> None:
        handle = IOBase.from_bytes(b"unread")

        with pytest.raises(ValueError, match="batch_size.*greater than zero"):
            handle.pstream_bytes(batch_size=0)
        with pytest.raises(ValueError, match="batch_size.*greater than zero"):
            handle.cursor().stream_bytes(0)

    def test_a_cursor_advances_as_chunks_are_consumed_then_can_be_reused(self) -> None:
        import inspect

        cursor = IOBase.from_bytes(b"0123456789").cursor(2)
        stream = cursor.stream_bytes(3)

        assert inspect.signature(cursor.stream_bytes).parameters[
            "batch_size"
        ].default == 65536
        assert cursor.tell() == 2
        assert next(stream) == b"234"
        assert cursor.tell() == 5

        # Each dynamic-language pull borrows the shared cursor anew, so an
        # explicit seek between pulls becomes the next chunk's start.
        cursor.seek(7)
        assert next(stream) == b"789"
        assert cursor.tell() == 10

        # Dropping an unfinished stream releases no hidden cursor state. The
        # ordinary cursor protocol resumes from the last yielded byte.
        cursor.seek(5)
        del stream
        assert cursor.read(2) == b"56"
        assert list(cursor.stream_bytes(2)) == [b"78", b"9"]
        assert cursor.tell() == 10

    def test_end_of_stream_stays_fused(self) -> None:
        stream = IOBase.from_bytes(b"abc").pstream_bytes(batch_size=2)

        assert list(stream) == [b"ab", b"c"]
        with pytest.raises(StopIteration):
            next(stream)
        with pytest.raises(StopIteration):
            next(stream)


class TestStructuredValues:
    """Structured values cross through the native core, including codings."""

    def test_a_handle_infers_json_and_gzip(self, tmp_path: pathlib.Path) -> None:
        path = tmp_path / "trade.json.gz"
        handle = IOBase(path)
        expected = {"quantity": 2, "symbol": "AAPL"}

        handle.write_value(expected)

        assert stdlib_gzip.decompress(path.read_bytes()) == (
            b'{"quantity":2,"symbol":"AAPL"}'
        )
        assert handle.read_value() == expected

    def test_a_field_directs_the_native_parse(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "trade.yaml.zst")
        handle.write_value({"quantity": 2, "symbol": "AAPL"})
        field = (
            "trade: struct<quantity: int32 not null, "
            "symbol: utf8 not null> not null"
        )

        assert handle.read_value(field) == {"quantity": 2, "symbol": "AAPL"}

        invalid = IOBase(tmp_path / "invalid.json")
        invalid.write_text('{"quantity":"many","symbol":"AAPL"}')
        with pytest.raises(ValueError, match="quantity"):
            invalid.read_value(field)
