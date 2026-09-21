"""Any ``pyarrow.fs.FileSystem`` becomes a handle, and answers the contract."""

from __future__ import annotations

import errno
import gzip as stdlib_gzip
import io
import pathlib
from typing import Any

import pyarrow as pa
import pyarrow.fs as pafs
import pyarrow.parquet as pq
import pytest

from yggdryl import IOBase, TextOptions
from yggdryl.coding import Gzip
from yggdryl.holder import FsPath
from yggdryl.media import Parquet

@pytest.fixture
def local() -> pafs.LocalFileSystem:
    """PyArrow's own local filesystem - a real outside implementation."""
    return pafs.LocalFileSystem()


@pytest.fixture
def root(tmp_path: pathlib.Path) -> str:
    """The filesystem-relative spelling of a temporary directory."""
    return tmp_path.as_posix()


def table() -> pa.Table:
    """Two rows in two columns, the fixture every record test writes."""
    return pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})


class MemoryHandler(pafs.FileSystemHandler):
    """A custom in-memory filesystem, the way a caller writes their own.

    This is what ``PyFileSystem(FileSystemHandler)`` is for, and it is also
    how ``fsspec`` arrives, so proving this shape works proves both.
    """

    def __init__(self) -> None:
        self.files: dict[str, bytes] = {}
        self.input_file_opens: list[str] = []
        self.input_stream_opens: list[str] = []

    def get_type_name(self) -> str:
        return "memory"

    def normalize_path(self, path: str) -> str:
        return path.strip("/")

    def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
        found = []
        for path in paths:
            key = path.strip("/")
            if key in self.files:
                found.append(
                    pafs.FileInfo(key, pafs.FileType.File, size=len(self.files[key]))
                )
            elif any(name.startswith(f"{key}/") for name in self.files):
                found.append(pafs.FileInfo(key, pafs.FileType.Directory))
            else:
                found.append(pafs.FileInfo(key, pafs.FileType.NotFound))
        return found

    def get_file_info_selector(
        self, selector: pafs.FileSelector
    ) -> list[pafs.FileInfo]:
        base = selector.base_dir.strip("/")
        prefix = f"{base}/" if base else ""
        if base in self.files:
            raise NotADirectoryError(base)
        if base and not any(name.startswith(prefix) for name in self.files):
            if selector.allow_not_found:
                return []
            raise FileNotFoundError(base)
        found = []
        directories = set()
        for name, data in self.files.items():
            if not name.startswith(prefix):
                continue
            rest = name[len(prefix) :]
            if not rest:
                continue
            if "/" in rest:
                directories.add(prefix + rest.split("/", 1)[0])
                if not selector.recursive:
                    continue
            found.append(pafs.FileInfo(name, pafs.FileType.File, size=len(data)))
        found.extend(
            pafs.FileInfo(name, pafs.FileType.Directory) for name in sorted(directories)
        )
        return found

    def create_dir(self, path: str, recursive: bool) -> None:
        # A directory is a prefix here, exactly as on an object store.
        return None

    def delete_dir(self, path: str) -> None:
        for name in [n for n in self.files if n.startswith(path.strip("/"))]:
            del self.files[name]

    def delete_dir_contents(self, path: str, missing_dir_ok: bool = False) -> None:
        key = path.strip("/")
        if key in self.files:
            raise NotADirectoryError(path)
        prefix = f"{key}/" if key else ""
        children = [name for name in self.files if name.startswith(prefix)]
        if key and not children and not missing_dir_ok:
            raise FileNotFoundError(path)
        for name in children:
            del self.files[name]

    def delete_root_dir_contents(self) -> None:
        self.files.clear()

    def delete_file(self, path: str) -> None:
        key = path.strip("/")
        if key in self.files:
            del self.files[key]
            return
        if any(name.startswith(f"{key}/") for name in self.files):
            raise IsADirectoryError(path)
        raise FileNotFoundError(path)

    def move(self, src: str, dest: str) -> None:
        source = src.strip("/")
        if source not in self.files:
            raise FileNotFoundError(src)
        self.files[dest.strip("/")] = self.files.pop(source)

    def copy_file(self, src: str, dest: str) -> None:
        source = src.strip("/")
        if source not in self.files:
            raise FileNotFoundError(src)
        self.files[dest.strip("/")] = self.files[source]

    def open_input_stream(self, path: str) -> pa.NativeFile:
        key = path.strip("/")
        self.input_stream_opens.append(key)
        if key not in self.files:
            raise FileNotFoundError(path)
        return pa.BufferReader(self.files[key])

    def open_input_file(self, path: str) -> pa.NativeFile:
        key = path.strip("/")
        self.input_file_opens.append(key)
        if key not in self.files:
            raise FileNotFoundError(path)
        return pa.BufferReader(self.files[key])

    def open_output_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
        return pa.PythonFile(_MemorySink(self, path.strip("/")), mode="w")

    def open_append_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
        key = path.strip("/")
        sink = _MemorySink(self, key)
        sink.write(self.files.get(key, b""))
        return pa.PythonFile(sink, mode="w")

    def __eq__(self, other: object) -> bool:
        return self is other

    def __hash__(self) -> int:
        return id(self)


class _MemorySink(io.BytesIO):
    """A test backend stream that publishes its received bytes on close."""

    def __init__(self, handler: MemoryHandler, path: str) -> None:
        super().__init__()
        self._handler = handler
        self._path = path

    def close(self) -> None:
        if not self.closed:
            self._handler.files[self._path] = self.getvalue()
        super().close()


class TestConstruction:
    """A filesystem plus a path is a handle, inferred or spelled out."""

    def test_the_explicit_constructor_names_the_filesystem_and_the_path(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/trades.bin")

        # Per the laziness contract nothing exists until something is written.
        assert not handle.exists()
        assert handle.size == 0
        assert handle.read_bytes() == b""

        handle.write_bytes(b"AAPL")
        handle.close()
        assert handle.read_bytes() == b"AAPL"
        assert pathlib.Path(root, "trades.bin").read_bytes() == b"AAPL"

    def test_the_constructor_infers_a_filesystem_first_argument(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        # IOBase(fs, path) means the same as IOBase.from_fs(fs, path).
        inferred = IOBase(local, f"{root}/inferred.bin")
        explicit = IOBase.from_fs(local, f"{root}/inferred.bin")
        assert str(inferred.url) == str(explicit.url)

        inferred.write_bytes(b"same")
        inferred.close()
        assert explicit.read_bytes() == b"same"

    def test_the_returned_handle_is_an_ordinary_iobase(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/trades.parquet")

        # Nothing filesystem-specific leaks into the surface: the same
        # contract, with the same pathlib-shaped names. What the class says is
        # which implementation answers them - the record encoding the name
        # declares, over the foreign filesystem it stands on.
        assert isinstance(handle, IOBase)
        assert isinstance(handle, Parquet)
        assert handle.name == "trades.parquet"
        assert str(handle.media_type) == "application/vnd.apache.parquet"
        # Descending spends the handle, so it is the last thing asked.
        assert isinstance(handle.into_handle(), FsPath)

    def test_a_non_filesystem_first_argument_is_refused_by_name(self) -> None:
        with pytest.raises(ValueError) as failure:
            IOBase.from_fs(object(), "bucket/key")
        assert "expected a pyarrow.fs.FileSystem" in str(failure.value)
        assert "object" in str(failure.value)

        # A path with nothing to resolve it against is refused too.
        with pytest.raises(ValueError) as missing:
            IOBase("some/path", "another/path")
        assert "expected a pyarrow.fs.FileSystem" in str(missing.value)


class TestBytesAndFolders:
    """The byte and hierarchy surface, over PyArrow's own local filesystem."""

    def test_positional_writes_reach_the_backend_without_a_close(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/direct.bin")
        handle.pwrite(0, b"pend")
        handle.pwrite(4, b"ing")

        assert pathlib.Path(root, "direct.bin").read_bytes() == b"pending"

    def test_a_whole_value_write_publishes_without_a_close(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/whole.bin")
        handle.write_bytes(b"published")
        assert pathlib.Path(root, "whole.bin").read_bytes() == b"published"

    def test_a_with_block_publishes_what_it_wrote(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        with IOBase.from_fs(local, f"{root}/scoped.bin") as handle:
            handle.write_bytes(b"scoped")
        assert pathlib.Path(root, "scoped.bin").read_bytes() == b"scoped"

    def test_folders_list_glob_and_carry_the_filesystem(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        lake = pathlib.Path(root, "lake")
        for year in ("2024", "2025"):
            leaf = lake / f"year={year}"
            leaf.mkdir(parents=True)
            (leaf / "part-0.parquet").write_bytes(b"PAR1")
            (leaf / "notes.txt").write_text("notes", encoding="utf-8")

        folder = IOBase.from_fs(local, lake.as_posix())
        assert folder.is_dir()
        assert len(list(folder.iterdir())) == 2
        assert len(list(folder.ls(recursive=True))) == 6

        parts = list(folder.glob("**/*.parquet"))
        assert len(parts) == 2

        # A child still carries the filesystem, so it reads through it.
        child = folder / "year=2024" / "part-0.parquet"
        assert child.read_bytes() == b"PAR1"
        assert child.parent.name == "year=2024"

        # Hive partitions come off the location, as they do anywhere else.
        selected = list(folder.children_where({"year": "2024"}))
        assert len(selected) == 2

    def test_a_missing_location_reads_empty_rather_than_raising(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        absent = IOBase.from_fs(local, f"{root}/nowhere/absent.arrows")
        assert absent.read_bytes() == b""
        assert absent.size == 0
        assert not absent.exists()


class TestRecords:
    """The three record methods, and interop with PyArrow both directions."""

    @pytest.mark.parametrize("name", ["trades.parquet", "trades.arrows"])
    def test_records_round_trip_through_the_wrapper(
        self, local: pafs.LocalFileSystem, root: str, name: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/{name}")
        with handle:
            handle.overwrite_arrow_table(table())

        read = handle.read_arrow_reader().read_all()
        assert read.num_rows == 2
        assert read.column_names == ["id", "symbol"]

        # Appending is the third method, and it reads-adds-rewrites.
        with handle:
            handle.append_arrow_table(table())
        assert handle.read_arrow_reader().read_all().num_rows == 4

    def test_yggdryl_writes_what_pyarrow_reads(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/written.parquet")
        with handle:
            handle.overwrite_arrow_table(table())

        # The outside implementation reads the bytes back, byte for byte the
        # file Yggdryl published.
        outside = pq.read_table(f"{root}/written.parquet")
        assert outside.equals(table())

    def test_pyarrow_writes_what_yggdryl_reads(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        pq.write_table(table(), f"{root}/foreign.parquet")

        handle = IOBase.from_fs(local, f"{root}/foreign.parquet")
        assert handle.read_arrow_reader().read_all().equals(table())

        # And the bytes the wrapper reads are the bytes on disk.
        assert handle.read_bytes() == pathlib.Path(root, "foreign.parquet").read_bytes()


class TestFramedText:
    """Logical records retain their contract through every Arrow filesystem."""

    @pytest.mark.parametrize("filesystem_kind", ["local", "subtree", "custom"])
    def test_schema_precedes_iteration_and_leaves_never_share_a_record(
        self,
        filesystem_kind: str,
        local: pafs.LocalFileSystem,
        tmp_path: pathlib.Path,
    ) -> None:
        first = b"[A] first\ncontinued in a"
        second = b"leading in b\n[B] second"
        handler: MemoryHandler | None = None

        if filesystem_kind == "custom":
            handler = MemoryHandler()
            handler.files["logs/a.log"] = first
            handler.files["logs/b.log"] = second
            filesystem: pafs.FileSystem = pafs.PyFileSystem(handler)
            location = "logs"
        else:
            storage = tmp_path / filesystem_kind / "logs"
            storage.mkdir(parents=True)
            (storage / "a.log").write_bytes(first)
            (storage / "b.log").write_bytes(second)
            if filesystem_kind == "subtree":
                filesystem = pafs.SubTreeFileSystem(
                    (tmp_path / filesystem_kind).as_posix(), local
                )
                location = "logs"
            else:
                filesystem = local
                location = storage.as_posix()

        options = TextOptions()
        options.framing = True
        options.leading_fragment = "keep"
        options.max_record_byte_size = 64
        options.rowheader = r"^\[(?<kind>[A-Z])\] "
        options.start_rownum = 1
        options.batch_row_size = 1

        reader = IOBase.from_fs(filesystem, location).read_arrow_reader(options=options)
        assert reader.schema.names[16:] == [
            "sourceurl",
            "rownum",
            "mtime",
            "body",
            "dropped_byte_size",
            "kind",
        ]
        assert reader.schema.names[:16] == [
            "currunix",
            "creaunix",
            "exprtime",
            "prevunix",
            "snapunix",
            "curruuid",
            "crossuuid",
            "crosscode",
            "currhashcode",
            "crosshashcode",
            "prevuuid",
            "seqnum",
            "parentuuids",
            "srcuuids",
            "identifiers",
            "state",
        ]
        assert reader.schema.field("dropped_byte_size") == pa.field(
            "dropped_byte_size", pa.uint64(), nullable=True
        )
        if handler is not None:
            assert handler.input_file_opens == []
            assert handler.input_stream_opens == []

        table = reader.read_all()
        assert table.column("body").to_pylist() == [
            "[A] first\ncontinued in a",
            "leading in b",
            "[B] second",
        ]
        assert table.column("rownum").to_pylist() == [1, 1, 2]
        assert table.column("kind").to_pylist() == ["A", None, "B"]
        assert table.column("dropped_byte_size").to_pylist() == [None, None, None]
        urls = table.column("sourceurl").to_pylist()
        assert urls[0].endswith("a.log")
        assert urls[1].endswith("b.log") and urls[2].endswith("b.log")
        if handler is not None:
            assert handler.input_file_opens == []
            assert handler.input_stream_opens == ["logs/a.log", "logs/b.log"]

    def test_s3_schema_is_known_without_contacting_the_endpoint(self) -> None:
        filesystem = pafs.S3FileSystem(
            anonymous=True,
            scheme="http",
            endpoint_override="127.0.0.1:9",
            connect_timeout=0.05,
            request_timeout=0.05,
        )
        options = TextOptions()
        options.framing = True
        options.rowheader = r"^\[(?<kind>[A-Z])\] "
        options.max_record_byte_size = 64

        reader = IOBase.from_fs(filesystem, "bucket/missing.log").read_arrow_reader(
            options=options
        )

        assert reader.schema.names[16:] == ["sourceurl", "mtime", "body", "dropped_byte_size", "kind"]
        assert reader.schema.field("dropped_byte_size").type == pa.uint64()


class TestCustomFilesystems:
    """A custom handler and a wrapped store, with no code of their own here."""

    def test_a_custom_filesystem_handler_is_a_handle(self) -> None:
        handler = MemoryHandler()
        filesystem = pafs.PyFileSystem(handler)

        handle = IOBase.from_fs(filesystem, "bucket/trades.parquet")
        with handle:
            handle.overwrite_arrow_table(table())

        # The rows landed in the caller's own storage, not on any disk.
        assert "bucket/trades.parquet" in handler.files
        assert handle.read_arrow_reader().read_all().num_rows == 2

    def test_a_custom_filesystem_lists_its_own_prefixes(self) -> None:
        handler = MemoryHandler()
        handler.files["lake/year=2024/part-0.parquet"] = b"PAR1"
        handler.files["lake/year=2025/part-0.parquet"] = b"PAR1"
        filesystem = pafs.PyFileSystem(handler)

        folder = IOBase.from_fs(filesystem, "lake")
        assert folder.is_dir()
        assert len(list(folder.glob("**/*.parquet"))) == 2

    def test_a_subtree_filesystem_is_transparent(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        # SubTreeFileSystem is the S3-shaped case without a network: a store
        # rooted at a prefix, where paths are relative to that prefix.
        base = pathlib.Path(root, "warehouse")
        base.mkdir()
        subtree = pafs.SubTreeFileSystem(base.as_posix(), local)
        subtree.create_dir("trades")

        handle = IOBase.from_fs(subtree, "trades/part-0.parquet")
        with handle:
            handle.overwrite_arrow_table(table())

        # Written through the prefix, and readable from the real path.
        assert (base / "trades" / "part-0.parquet").exists()
        assert pq.read_table(base / "trades" / "part-0.parquet").equals(table())

    def test_a_compressible_name_is_stored_as_the_bytes_the_handle_codes(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        # PyArrow's output stream infers a codec from the suffix unless told
        # not to, which would gzip a value the handle had already coded - and
        # store something nothing reads back. The name declares the coding, so
        # the handle is the one that applies it, exactly once.
        handle = IOBase.from_fs(local, f"{root}/trades.json.gz")
        assert isinstance(handle, Gzip)
        handle.write_bytes(b"AAPL")
        handle.close()

        assert stdlib_gzip.decompress(pathlib.Path(root, "trades.json.gz").read_bytes()) == (
            b"AAPL"
        )
        assert handle.read_bytes() == b"AAPL"

    def test_a_content_coding_round_trips_over_a_foreign_filesystem(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        # The coding belongs to the handle, so what lands is gzip exactly once
        # and what the stored-byte role reads is the coded form.
        handle = IOBase.from_fs(local, f"{root}/coded.json.gz")
        handle.write_bytes(b'{"symbol":"AAPL"}')
        handle.close()

        stored = pathlib.Path(root, "coded.json.gz").read_bytes()
        assert stdlib_gzip.decompress(stored) == b'{"symbol":"AAPL"}'
        assert IOBase.from_fs(local, f"{root}/coded.json.gz").read_bytes() == (
            b'{"symbol":"AAPL"}'
        )

    def test_mkdir_creates_the_container_on_the_same_filesystem(self) -> None:
        handler = MemoryHandler()
        handle = IOBase.from_fs(pafs.PyFileSystem(handler), "bucket/lake")

        # A location does not say which backend it belongs to, so mkdir must
        # not quietly rebuild the handle on the local disk.
        handle.mkdir()
        child = handle / "part-0.bin"
        child.write_bytes(b"AAPL")
        child.close()

        assert handler.files.get("bucket/lake/part-0.bin") == b"AAPL"
        assert not pathlib.Path("bucket/lake").exists()

    def test_a_table_hands_back_a_root_on_its_own_filesystem(self) -> None:
        from yggdryl import iceberg

        handler = MemoryHandler()
        warehouse = IOBase.from_fs(pafs.PyFileSystem(handler), "warehouse/trades")
        stored = iceberg.Table.create(warehouse, table().schema)
        stored.append(table())

        # The root is the folder the table actually lives in, not the local
        # path its recorded location happens to spell.
        root_handle = stored.root
        assert root_handle.is_dir()
        assert "metadata" in [entry.name for entry in root_handle.iterdir()]
        assert len(list(root_handle.glob("data/**/*.parquet"))) == 1

    def test_a_handler_that_raises_surfaces_its_own_message(self) -> None:
        class Broken(MemoryHandler):
            def open_input_stream(self, path: str) -> pa.NativeFile:
                raise PermissionError("the bucket refused the request: 403 Forbidden")

            def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
                return [
                    pafs.FileInfo(path.strip("/"), pafs.FileType.File, size=8)
                    for path in paths
                ]

        handle = IOBase.from_fs(pafs.PyFileSystem(Broken()), "bucket/key.bin")
        with pytest.raises(PermissionError) as failure:
            handle.read_bytes()

        # The foreign message crosses unchanged rather than being reworded.
        assert "403 Forbidden" in str(failure.value)

    def test_an_exception_with_no_message_still_names_its_class(self) -> None:
        class Bare(MemoryHandler):
            def open_input_stream(self, path: str) -> pa.NativeFile:
                # The shape normal Python code raises: the class is the message.
                raise PermissionError

            def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
                return [
                    pafs.FileInfo(path.strip("/"), pafs.FileType.File, size=8)
                    for path in paths
                ]

        handle = IOBase.from_fs(pafs.PyFileSystem(Bare()), "bucket/key.bin")
        with pytest.raises(PermissionError) as failure:
            handle.read_bytes()

        # With no text to carry, the class is the whole of what the caller has.
        assert "PermissionError" in str(failure.value)

    def test_a_byte_stream_opens_once_and_stays_fused(self) -> None:
        class CountsInputOpens(MemoryHandler):
            def __init__(self) -> None:
                super().__init__()
                self.files["bucket/key.bin"] = b"abcdef"
                self.reads = 0

            def open_input_file(self, path: str) -> pa.NativeFile:
                self.reads += 1
                return super().open_input_file(path)

        handler = CountsInputOpens()
        handle = IOBase.from_fs(pafs.PyFileSystem(handler), "bucket/key.bin")
        stream = handle.pstream_bytes(batch_size=3)

        # One native input file is retained across every bounded read.
        assert handler.reads == 1
        assert next(stream) == b"abc"
        assert next(stream) == b"def"
        with pytest.raises(StopIteration):
            next(stream)
        with pytest.raises(StopIteration):
            next(stream)
        assert handler.reads == 1


class TestTables:
    """A table is a folder, and a foreign filesystem is where it can live."""

    def test_an_iceberg_table_lives_on_a_foreign_filesystem(self, root: str) -> None:
        from yggdryl import iceberg

        handler = MemoryHandler()
        warehouse = IOBase.from_fs(pafs.PyFileSystem(handler), "warehouse/trades")

        table_handle = iceberg.Table.create(warehouse, table().schema)
        table_handle.append(table())

        rows = table_handle.scan().read_all()
        assert rows.num_rows == 2
        assert rows.column_names == ["id", "symbol"]

        # Every byte of that table went through the caller's own handler.
        assert any(name.endswith(".metadata.json") for name in handler.files)
        assert any(name.endswith(".parquet") for name in handler.files)


class _CountingSource(io.BytesIO):
    """A backend stream that records the size of every read asked of it."""

    def __init__(self, data: bytes, reads: list[int]) -> None:
        super().__init__(data)
        self._reads = reads

    def readinto(self, buffer: Any) -> int:  # type: ignore[override]
        self._reads.append(len(buffer))
        return super().readinto(buffer)

    def read(self, size: int | None = -1) -> bytes:
        self._reads.append(len(self.getvalue()) if size is None or size < 0 else size)
        return super().read(size)


class CountingHandler(MemoryHandler):
    """``MemoryHandler`` that records what each read asks the backend for."""

    def __init__(self) -> None:
        super().__init__()
        self.reads: list[int] = []

    def open_input_stream(self, path: str) -> pa.NativeFile:
        key = path.strip("/")
        self.input_stream_opens.append(key)
        if key not in self.files:
            raise FileNotFoundError(path)
        return pa.PythonFile(_CountingSource(self.files[key], self.reads), mode="r")


def _log_payload(lines: int) -> tuple[bytes, int]:
    """Log text gzip cannot shrink away, so the object spans fetch windows."""
    alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
    state = 0x2545F491
    body = bytearray()
    for index in range(lines):
        body += b"[INFO] id=%d " % index
        for _ in range(1024):
            state ^= (state << 13) & 0xFFFFFFFF
            state ^= state >> 17
            state ^= (state << 5) & 0xFFFFFFFF
            body.append(alphabet[state % len(alphabet)])
        body += b"\n"
    return bytes(body), lines


def test_a_compressed_log_is_read_one_fetch_window_at_a_time() -> None:
    """A foreign filesystem is asked for windows, not for decoder-sized reads.

    A gzip stream pulls 32 KiB at a time, and on an object store - or across
    this binding, where every read is a call into Python - that is what a scan
    would cost without a window between the transport and the decoder.
    """
    plain, lines = _log_payload(2_000)
    handler = CountingHandler()
    handler.files["logs/app.log.gz"] = stdlib_gzip.compress(plain)
    encoded_size = len(handler.files["logs/app.log.gz"])
    assert encoded_size > 1024 * 1024, encoded_size

    options = TextOptions()
    options.framing = True
    options.rowheader = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "

    handle = IOBase.from_fs(pafs.PyFileSystem(handler), "logs/app.log.gz")
    table = handle.read_arrow_reader(options=options).read_all()

    assert table.num_rows == lines
    assert handler.input_stream_opens == ["logs/app.log.gz"]
    assert handler.input_file_opens == []
    # One ask per window, not one per decoder pull, and no one-byte probe.
    windows = -(-encoded_size // (1024 * 1024))
    assert len(handler.reads) <= windows + 1, handler.reads
    assert min(handler.reads) > 64 * 1024, handler.reads


def test_injected_path_is_opaque_and_retains_all_bound_facts() -> None:
    filesystem = pafs._MockFileSystem()
    filesystem.create_dir("bucket")
    path = "bucket/v=a%2Fb+%25.bin"
    uri = "s3://key:secret@bucket/v=a%2Fb+%25.bin?session_token=hidden"
    handle = IOBase.from_fs(filesystem, path, uri=uri)

    with handle.open_output_stream(compression=None) as stream:
        stream.write(b"literal")

    assert filesystem.get_file_info(path).type == pafs.FileType.File
    assert handle.open_input_file().read() == b"literal"
    assert handle.filesystem is filesystem
    assert handle.path == path
    assert handle.uri == uri
    assert "secret" not in handle.masked_uri
    assert "hidden" not in handle.masked_uri
    assert "secret" not in repr(handle)
    assert "hidden" not in repr(handle)


@pytest.mark.parametrize(
    ("uri", "path"),
    [
        ("s3://bucket/key", "bucket/key"),
        ("s3a://bucket/key", "bucket/key"),
        ("s3n://bucket/key", "bucket/key"),
        ("s3://key:secret@bucket/key", "bucket/key"),
        ("s3://key:secret@minio:9000/bucket/key", "bucket/key"),
        (
            "s3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1",
            "bucket/key",
        ),
        ("s3://bucket.s3.eu-west-1.amazonaws.com/key", "bucket/key"),
        ("s3://bucket/v=a%2Fb", "bucket/v=a%2Fb"),
    ],
)
def test_s3_uri_resolution_needs_no_network(uri: str, path: str) -> None:
    options = None if "key:secret@" in uri else {"anonymous": True}
    handle = IOBase.from_uri(uri, options=options)
    assert isinstance(handle.filesystem, pafs.S3FileSystem)
    assert handle.path == path
    assert handle.uri == uri
    assert "secret" not in repr(handle)


def test_file_uri_resolution_binds_a_local_filesystem(tmp_path: pathlib.Path) -> None:
    target = tmp_path / "literal-%2F.bin"
    handle = IOBase.from_uri(target.as_uri())

    assert isinstance(handle.filesystem, pafs.LocalFileSystem)
    assert handle.uri == target.as_uri()
    with handle.open_output_stream(compression=None) as stream:
        stream.write(b"local")
    assert target.read_bytes() == b"local"


def test_uri_resolution_errors_mask_credentials() -> None:
    with pytest.raises(Exception) as failure:
        IOBase.from_uri("s3://access:do-not-leak@")
    assert "do-not-leak" not in str(failure.value)


def test_identity_uses_filesystem_equality_and_exact_path(
    tmp_path: pathlib.Path,
) -> None:
    local = pafs.LocalFileSystem()
    equal = pafs.LocalFileSystem()
    left = IOBase.from_fs(local, "same/path")
    right = IOBase.from_fs(equal, "same/path")
    assert left.same_location(right)
    assert not left.same_location(IOBase.from_fs(equal, "same//path"))

    first_root = tmp_path / "first"
    second_root = tmp_path / "second"
    first_root.mkdir()
    second_root.mkdir()
    first = pafs.SubTreeFileSystem(first_root.as_posix(), local)
    second = pafs.SubTreeFileSystem(second_root.as_posix(), local)
    assert not IOBase.from_fs(first, "key").same_location(IOBase.from_fs(second, "key"))

    one = pafs.PyFileSystem(MemoryHandler())
    two = pafs.PyFileSystem(MemoryHandler())
    assert not IOBase.from_fs(one, "key").same_location(IOBase.from_fs(two, "key"))


def test_listing_is_sorted_and_children_keep_the_same_filesystem() -> None:
    handler = MemoryHandler()
    handler.files["lake/z.bin"] = b"z"
    handler.files["lake/a.bin"] = b"a"
    filesystem = pafs.PyFileSystem(handler)
    root = IOBase.from_fs(filesystem, "lake", uri="s3://lake")

    children = list(root.iterdir())
    assert [child.path for child in children] == ["lake/a.bin", "lake/z.bin"]
    assert all(child.filesystem is filesystem for child in children)
    assert all(child.parent.filesystem is filesystem for child in children)


@pytest.mark.parametrize("kind", ["local", "mock", "subtree", "custom"])
def test_hierarchy_keeps_raw_paths_and_filesystem_across_arrow_shapes(
    kind: str, tmp_path: pathlib.Path
) -> None:
    if kind == "local":
        filesystem: pafs.FileSystem = pafs.LocalFileSystem()
        path = (tmp_path / "local" / "lake").as_posix()
        filesystem.create_dir(path, recursive=True)
    elif kind == "mock":
        filesystem = pafs._MockFileSystem()
        path = "lake"
        filesystem.create_dir(path, recursive=True)
    elif kind == "subtree":
        base = tmp_path / "subtree"
        base.mkdir()
        filesystem = pafs.SubTreeFileSystem(base.as_posix(), pafs.LocalFileSystem())
        path = "lake"
        filesystem.create_dir(path, recursive=True)
    else:
        handler = MemoryHandler()
        filesystem = pafs.PyFileSystem(handler)
        path = "lake"

    names = ("v=a%2Fb.bin", "z+%25.bin")
    if kind == "custom":
        handler.files.update({f"{path}/{name}": name.encode() for name in names})
    else:
        for name in names:
            with filesystem.open_output_stream(f"{path}/{name}") as stream:
                stream.write(name.encode())

    uri = "s3://key:secret@bucket/base?session_token=hidden"
    root = IOBase.from_fs(filesystem, path, uri=uri)
    children = list(root.iterdir())
    expected = [f"{path}/{name}" for name in names]
    assert [child.path for child in children] == expected
    assert [child.path for child in root.glob("*.bin")] == expected
    assert all(child.filesystem is filesystem for child in children)
    assert all(
        "secret" not in repr(child) and "hidden" not in repr(child)
        for child in children
    )

    joined = root.joinpath(names[0])
    assert joined.path == expected[0]
    assert joined.filesystem is filesystem
    assert joined.same_location(children[0])
    assert joined.parent.same_location(root)


def test_file_info_preserves_size_and_nanosecond_mtime() -> None:
    class InfoHandler(MemoryHandler):
        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            return [
                pafs.FileInfo(
                    path,
                    pafs.FileType.File,
                    size=9_007_199_254_740_993,
                    mtime_ns=1_725_000_000_123_456_789,
                )
                for path in paths
            ]

    info = IOBase.from_fs(pafs.PyFileSystem(InfoHandler()), "bucket/key").info()
    assert info.size == 9_007_199_254_740_993
    assert info.mtime_ns == 1_725_000_000_123_456_789

    class OptionalInfoHandler(MemoryHandler):
        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            return [pafs.FileInfo(path, pafs.FileType.File) for path in paths]

    optional = IOBase.from_fs(
        pafs.PyFileSystem(OptionalInfoHandler()), "bucket/unknown-size"
    ).info()
    assert optional.type == pafs.FileType.File
    assert optional.size is None
    assert optional.mtime_ns is None


def test_output_options_reach_the_foreign_stream() -> None:
    class MetadataHandler(MemoryHandler):
        metadata: Any = None

        def open_output_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
            self.metadata = metadata
            return super().open_output_stream(path, metadata)

    handler = MetadataHandler()
    handle = IOBase.from_fs(pafs.PyFileSystem(handler), "bucket/key")
    with handle.open_output_stream(
        compression=None, buffer_size=17, metadata={"content-type": "binary"}
    ) as stream:
        stream.write(b"x")

    assert dict(handler.metadata) == {b"content-type": b"binary"}


def test_same_filesystem_copy_and_move_use_one_native_call() -> None:
    class CountingHandler(MemoryHandler):
        def __init__(self) -> None:
            super().__init__()
            self.copies = 0
            self.moves = 0
            self.input_opens = 0
            self.output_opens = 0

        def copy_file(self, src: str, dest: str) -> None:
            self.copies += 1
            super().copy_file(src, dest)

        def move(self, src: str, dest: str) -> None:
            self.moves += 1
            super().move(src, dest)

        def open_input_file(self, path: str) -> pa.NativeFile:
            self.input_opens += 1
            return super().open_input_file(path)

        def open_output_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
            self.output_opens += 1
            return super().open_output_stream(path, metadata)

    handler = CountingHandler()
    handler.files["source"] = b"payload"
    filesystem = pafs.PyFileSystem(handler)
    source = IOBase.from_fs(filesystem, "source")
    copied = IOBase.from_fs(filesystem, "copied")
    assert source.copy_into(copied) == 7
    assert (handler.copies, handler.input_opens, handler.output_opens) == (1, 0, 0)

    moved = IOBase.from_fs(filesystem, "moved")
    returned = copied.move_into(moved)
    assert returned.same_location(moved)
    assert (handler.moves, handler.input_opens, handler.output_opens) == (1, 0, 0)
    assert handler.files["moved"] == b"payload"


def test_missing_copy_never_changes_or_creates_a_target() -> None:
    source_store = MemoryHandler()
    target_store = MemoryHandler()
    target_store.files["existing"] = b"original"
    missing = IOBase.from_fs(pafs.PyFileSystem(source_store), "missing")

    with pytest.raises(FileNotFoundError):
        missing.copy_into(IOBase.from_fs(pafs.PyFileSystem(target_store), "existing"))
    assert target_store.files["existing"] == b"original"

    with pytest.raises(FileNotFoundError):
        missing.copy_into(IOBase.from_fs(pafs.PyFileSystem(target_store), "new"))
    assert "new" not in target_store.files


def test_typed_permission_errors_are_not_absence() -> None:
    class Refuses(MemoryHandler):
        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            raise PermissionError("denied")

        def open_input_stream(self, path: str) -> pa.NativeFile:
            raise PermissionError("denied")

        def open_input_file(self, path: str) -> pa.NativeFile:
            raise PermissionError("denied")

    handle = IOBase.from_fs(pafs.PyFileSystem(Refuses()), "bucket/key")
    with pytest.raises(PermissionError):
        handle.info()
    with pytest.raises(PermissionError):
        handle.exists()
    with pytest.raises(PermissionError):
        handle.read_bytes()
    with pytest.raises(PermissionError):
        handle.read_range_bytes(0, 1)
    for predicate in (handle.is_io, handle.is_atomic, handle.is_tabular):
        with pytest.raises(PermissionError):
            predicate()


def test_foreign_errors_keep_their_type_without_leaking_credentials() -> None:
    secret_uri = "s3://access:never-show@bucket/key?session_token=also-hidden"

    class Refuses(MemoryHandler):
        def open_input_stream(self, path: str) -> pa.NativeFile:
            raise PermissionError(f"refused {secret_uri}")

        def open_input_file(self, path: str) -> pa.NativeFile:
            raise PermissionError(f"refused {secret_uri}")

        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            return [
                pafs.FileInfo(path, pafs.FileType.File, size=1) for path in paths
            ]

    handle = IOBase.from_fs(pafs.PyFileSystem(Refuses()), "bucket/key", uri=secret_uri)
    for operation in (handle.read_bytes, handle.open_input_file):
        with pytest.raises(PermissionError) as failure:
            operation()
        message = str(failure.value)
        assert "never-show" not in message
        assert "also-hidden" not in message


def test_selector_absence_policy_and_strict_deletes_are_typed() -> None:
    filesystem = pafs.PyFileSystem(MemoryHandler())
    missing = IOBase.from_fs(filesystem, "missing")

    assert list(missing.iterdir()) == []
    with pytest.raises(io.UnsupportedOperation):
        missing.delete_dir()
    with pytest.raises(FileNotFoundError):
        missing.delete_file()


def test_directory_operations_are_distinct(tmp_path: pathlib.Path) -> None:
    filesystem = pafs.LocalFileSystem()

    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(io.UnsupportedOperation):
        IOBase.from_fs(filesystem, empty.as_posix()).delete_dir()
    assert empty.is_dir()

    kept = tmp_path / "kept"
    kept.mkdir()
    (kept / "child").write_bytes(b"x")
    IOBase.from_fs(filesystem, kept.as_posix()).delete_dir_contents()
    assert kept.is_dir() and list(kept.iterdir()) == []

    nonempty = tmp_path / "nonempty"
    nonempty.mkdir()
    (nonempty / "child").write_bytes(b"x")
    with pytest.raises(io.UnsupportedOperation):
        IOBase.from_fs(filesystem, nonempty.as_posix()).delete_dir()
    assert (nonempty / "child").read_bytes() == b"x"
    IOBase.from_fs(filesystem, nonempty.as_posix()).remove(recursive=True)
    assert not nonempty.exists()

    class StrictHandler(MemoryHandler):
        def delete_file(self, path: str) -> None:
            raise IsADirectoryError(path)

    strict_handler = StrictHandler()
    strict_handler.files["nonempty/child"] = b"x"
    strict = pafs.PyFileSystem(strict_handler)
    with pytest.raises(IsADirectoryError):
        IOBase.from_fs(strict, "nonempty").delete_file()

    for operation in (
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).delete_file(),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).read_bytes(),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).read_range_bytes(0, 1),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).open_input_file(),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).open_input_stream(
            compression=None
        ),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).open_output_stream(
            compression=None
        ),
        lambda: IOBase.from_fs(filesystem, empty.as_posix()).open_append_stream(
            compression=None
        ),
    ):
        with pytest.raises(IsADirectoryError):
            operation()

    source_path = tmp_path / "source.bin"
    source_path.write_bytes(b"source")
    source = IOBase.from_fs(filesystem, source_path.as_posix())
    directory = IOBase.from_fs(filesystem, empty.as_posix())
    with pytest.raises(IsADirectoryError):
        source.copy_into(directory)
    with pytest.raises(IsADirectoryError):
        source.move_into(directory)
    assert source_path.read_bytes() == b"source"


def test_explicit_unsupported_errno_remains_typed() -> None:
    class NoAppend(MemoryHandler):
        def open_append_stream(
            self, path: str, metadata: Any = None
        ) -> pa.NativeFile:
            del metadata
            raise OSError(errno.ENOTSUP, "append unavailable")

    handler = NoAppend()
    handler.files["value"] = b"x"
    handle = IOBase.from_fs(pafs.PyFileSystem(handler), "value")
    with pytest.raises(io.UnsupportedOperation):
        handle.append_bytes(b"y")


def test_root_contents_deletion_requires_an_explicit_root_binding() -> None:
    handler = MemoryHandler()
    handler.files.update({"one": b"1", "nested/two": b"2"})
    filesystem = pafs.PyFileSystem(handler)

    with pytest.raises(io.UnsupportedOperation):
        IOBase.from_fs(filesystem, "nested").delete_root_dir_contents()
    assert handler.files == {"one": b"1", "nested/two": b"2"}

    IOBase.from_fs(filesystem, "").delete_root_dir_contents()
    assert handler.files == {}


def test_cursor_is_a_binary_file_object() -> None:
    filesystem = pafs._MockFileSystem()
    filesystem.create_dir("bucket")
    handle = IOBase.from_fs(filesystem, "bucket/value")
    handle.write_bytes(b"abcdef")

    cursor = handle.cursor()
    assert cursor.readable() and cursor.writable() and cursor.seekable()
    target = bytearray(3)
    assert cursor.readinto(target) == 3
    assert target == b"abc"
    assert cursor.tell() == 3
    assert cursor.seek(-1, 1) == 2
    assert cursor.read(2) == b"cd"
    with pa.PythonFile(handle.cursor(), mode="r") as stream:
        assert stream.read() == b"abcdef"

    cursor.close()
    cursor.close()
    assert cursor.closed
    with pytest.raises(ValueError, match="closed"):
        cursor.read(1)
    for capability in (cursor.readable, cursor.writable, cursor.seekable):
        with pytest.raises(ValueError, match="closed"):
            capability()


def test_native_handles_expose_real_arrow_streams() -> None:
    handle = IOBase.from_bytes(b"abcdef")

    with handle.open_input_file() as stream:
        assert isinstance(stream, pa.NativeFile)
        assert stream.read_at(3, 2) == b"cde"
    with handle.open_input_stream(compression=None, buffer_size=2) as stream:
        assert stream.read(2) == b"ab"
        assert stream.read() == b"cdef"

    with handle.open_output_stream(compression=None, buffer_size=2) as stream:
        stream.write(b"new")
    assert handle.read_bytes() == b"new"

    with handle.open_append_stream(compression=None, buffer_size=2) as stream:
        stream.write(pa.py_buffer(b"-tail"))
    assert handle.read_bytes() == b"new-tail"

    with pytest.raises(io.UnsupportedOperation):
        handle.open_output_stream(metadata={"content-type": "binary"})


def test_cursor_read_to_end_needs_no_size_and_readinto_validates_first() -> None:
    class UnknownSize(MemoryHandler):
        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            return [pafs.FileInfo(path, pafs.FileType.File) for path in paths]

    handler = UnknownSize()
    handler.files["value"] = b"payload"
    cursor = IOBase.from_fs(pafs.PyFileSystem(handler), "value").cursor()
    assert cursor.read() == b"payload"

    cursor.seek(0)
    with pytest.raises(TypeError):
        cursor.readinto(memoryview(b"xxxx"))
    assert cursor.tell() == 0
    target = bytearray(4)
    assert cursor.readinto(target) == 4
    assert target == b"payl"


def test_cursor_retains_one_random_access_file_across_reads_and_seeks() -> None:
    class CountingHandler(MemoryHandler):
        def __init__(self) -> None:
            super().__init__()
            self.files["value"] = b"0123456789"
            self.infos = 0

        def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
            self.infos += 1
            return super().get_file_info(paths)

    handler = CountingHandler()
    cursor = IOBase.from_fs(pafs.PyFileSystem(handler), "value").cursor()
    with pa.PythonFile(cursor, mode="r") as stream:
        assert stream.read(3) == b"012"
        assert stream.read(3) == b"345"
        assert stream.seek(-2, 1) == 4
        assert stream.read(2) == b"45"
    assert handler.input_file_opens == ["value"]
    assert handler.infos == 0


def test_cursor_replays_a_typed_close_failure_without_closing_twice() -> None:
    class FailingReader(io.BytesIO):
        def __init__(self) -> None:
            super().__init__(b"payload")
            self.close_calls = 0

        def close(self) -> None:
            self.close_calls += 1
            raise PermissionError("reader close failed")

    class FailingCloseHandler(MemoryHandler):
        reader: FailingReader

        def open_input_file(self, path: str) -> pa.NativeFile:
            self.reader = FailingReader()
            return pa.PythonFile(self.reader, mode="r")

    handler = FailingCloseHandler()
    cursor = IOBase.from_fs(pafs.PyFileSystem(handler), "value").cursor()
    assert cursor.read(1) == b"p"

    for _ in range(2):
        with pytest.raises(PermissionError, match="reader close failed"):
            cursor.close()
    assert handler.reader.close_calls == 1


def test_cross_filesystem_write_failure_closes_once_and_stays_primary() -> None:
    class FailingSink(io.RawIOBase):
        def __init__(self) -> None:
            super().__init__()
            self.close_calls = 0

        def writable(self) -> bool:
            return True

        def tell(self) -> int:
            return 0

        def write(self, data: bytes) -> int:
            raise PermissionError("write failed")

        def close(self) -> None:
            if self.close_calls == 0:
                self.close_calls += 1
                raise PermissionError("close failed")

    class FailingTarget(MemoryHandler):
        sink: FailingSink

        def open_output_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
            self.sink = FailingSink()
            return pa.PythonFile(self.sink, mode="w")

    source_store = MemoryHandler()
    source_store.files["source"] = b"payload"
    target_store = FailingTarget()
    source = IOBase.from_fs(pafs.PyFileSystem(source_store), "source")
    target = IOBase.from_fs(pafs.PyFileSystem(target_store), "target")

    with pytest.raises(PermissionError, match="write failed"):
        source.copy_into(target)
    assert target_store.sink.close_calls == 1
    assert "target" not in target_store.files
