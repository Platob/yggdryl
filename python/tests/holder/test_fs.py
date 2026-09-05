"""Any ``pyarrow.fs.FileSystem`` becomes a handle, with no per-backend code."""

from __future__ import annotations

import io
import pathlib
from typing import Any

import pyarrow as pa
import pyarrow.fs as pafs
import pyarrow.parquet as pq
import pytest

from yggdryl import IOBase


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

    def get_file_info_selector(self, selector: pafs.FileSelector) -> list[pafs.FileInfo]:
        base = selector.base_dir.strip("/")
        prefix = f"{base}/" if base else ""
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
        self.delete_dir(path)

    def delete_root_dir_contents(self) -> None:
        self.files.clear()

    def delete_file(self, path: str) -> None:
        self.files.pop(path.strip("/"), None)

    def move(self, src: str, dest: str) -> None:
        self.files[dest.strip("/")] = self.files.pop(src.strip("/"))

    def copy_file(self, src: str, dest: str) -> None:
        self.files[dest.strip("/")] = self.files[src.strip("/")]

    def open_input_stream(self, path: str) -> pa.NativeFile:
        return pa.BufferReader(self.files[path.strip("/")])

    def open_input_file(self, path: str) -> pa.NativeFile:
        key = path.strip("/")
        if key not in self.files:
            raise FileNotFoundError(path)
        return pa.BufferReader(self.files[key])

    def open_output_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
        return pa.PythonFile(_MemorySink(self, path.strip("/")), mode="w")

    def open_append_stream(self, path: str, metadata: Any = None) -> pa.NativeFile:
        raise NotImplementedError

    def __eq__(self, other: object) -> bool:
        return self is other

    def __hash__(self) -> int:
        return id(self)


class _MemorySink(io.BytesIO):
    """The sink ``MemoryHandler`` writes through, storing what it collected.

    A whole-value write is the one shape an Arrow filesystem has, so the
    bytes are handed over when the stream closes - which is exactly when the
    handle publishes.
    """

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

        # Nothing filesystem-specific leaks into the surface: it is the same
        # class, with the same pathlib-shaped names.
        assert isinstance(handle, IOBase)
        assert handle.name == "trades.parquet"
        assert str(handle.media_type) == "application/vnd.apache.parquet"

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

    def test_bytes_round_trip_and_publish_on_close(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/staged.bin")
        # Positional writes are pieces of a value, so they stage: an Arrow
        # filesystem replaces whole files and must never see a half-written one.
        handle.pwrite(0, b"pend")
        handle.pwrite(4, b"ing")

        assert not pathlib.Path(root, "staged.bin").exists()
        handle.close()
        assert pathlib.Path(root, "staged.bin").read_bytes() == b"pending"

    def test_a_whole_value_write_publishes_without_a_close(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        handle = IOBase.from_fs(local, f"{root}/whole.bin")
        # A complete value is one store operation, so it needs no scope; the
        # staging above exists to fold many positional writes into one.
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

        handle = IOBase.from_fs(subtree, "trades/part-0.parquet")
        with handle:
            handle.overwrite_arrow_table(table())

        # Written through the prefix, and readable from the real path.
        assert (base / "trades" / "part-0.parquet").exists()
        assert pq.read_table(base / "trades" / "part-0.parquet").equals(table())

    def test_a_compressible_name_is_stored_as_the_bytes_it_was_given(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        # PyArrow's output stream infers a codec from the suffix unless told
        # not to, which would gzip a value the handle had already coded - and
        # store something nothing reads back.
        handle = IOBase.from_fs(local, f"{root}/trades.json.gz")
        handle.write_bytes(b"AAPL")
        handle.close()

        assert pathlib.Path(root, "trades.json.gz").read_bytes() == b"AAPL"
        assert handle.read_bytes() == b"AAPL"

    def test_a_content_coding_round_trips_over_a_foreign_filesystem(
        self, local: pafs.LocalFileSystem, root: str
    ) -> None:
        import gzip

        # The coding belongs to the handle, so what lands is gzip exactly once.
        handle = IOBase.from_fs(local, f"{root}/coded.json.gz")
        handle.write_bytes(gzip.compress(b'{"symbol":"AAPL"}'))
        handle.close()

        stored = pathlib.Path(root, "coded.json.gz").read_bytes()
        assert gzip.decompress(stored) == b'{"symbol":"AAPL"}'

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
        from yggdryl.media import iceberg

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
            def open_input_file(self, path: str) -> pa.NativeFile:
                raise PermissionError("the bucket refused the request: 403 Forbidden")

            def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
                return [
                    pafs.FileInfo(path.strip("/"), pafs.FileType.File, size=8)
                    for path in paths
                ]

        handle = IOBase.from_fs(pafs.PyFileSystem(Broken()), "bucket/key.bin")
        with pytest.raises(ValueError) as failure:
            handle.read_bytes()

        # The foreign message crosses unchanged rather than being reworded.
        assert "403 Forbidden" in str(failure.value)

    def test_an_exception_with_no_message_still_names_its_class(self) -> None:
        class Bare(MemoryHandler):
            def open_input_file(self, path: str) -> pa.NativeFile:
                # The shape normal Python code raises: the class is the message.
                raise PermissionError

            def get_file_info(self, paths: list[str]) -> list[pafs.FileInfo]:
                return [
                    pafs.FileInfo(path.strip("/"), pafs.FileType.File, size=8)
                    for path in paths
                ]

        handle = IOBase.from_fs(pafs.PyFileSystem(Bare()), "bucket/key.bin")
        with pytest.raises(ValueError) as failure:
            handle.read_bytes()

        # With no text to carry, the class is the whole of what the caller has.
        assert "PermissionError" in str(failure.value)

    def test_a_byte_stream_yields_one_failure_then_stays_fused(self) -> None:
        class FailsAfterOneChunk(MemoryHandler):
            def __init__(self) -> None:
                super().__init__()
                self.files["bucket/key.bin"] = b"abcdef"
                self.reads = 0

            def open_input_file(self, path: str) -> pa.NativeFile:
                self.reads += 1
                if self.reads > 1:
                    raise PermissionError("later streamed read failed")
                return super().open_input_file(path)

        handler = FailsAfterOneChunk()
        handle = IOBase.from_fs(
            pafs.PyFileSystem(handler), "bucket/key.bin"
        )
        stream = handle.pstream_bytes(batch_size=3)

        # Creating the iterator performs no ranged read.
        assert handler.reads == 0
        assert next(stream) == b"abc"
        with pytest.raises(ValueError, match="later streamed read failed"):
            next(stream)
        with pytest.raises(StopIteration):
            next(stream)
        with pytest.raises(StopIteration):
            next(stream)


class TestTables:
    """A table is a folder, and a foreign filesystem is where it can live."""

    def test_an_iceberg_table_lives_on_a_foreign_filesystem(self, root: str) -> None:
        from yggdryl.media import iceberg

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
