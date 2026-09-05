"""Arrow filesystem and bound-location conformance at the Python boundary."""

from __future__ import annotations

import errno
import io
import pathlib
from typing import Any

import pyarrow as pa
import pyarrow.fs as pafs
import pytest
from yggdryl import IOBase

from .test_fs import MemoryHandler


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

    handle = IOBase.from_fs(pafs.PyFileSystem(Refuses()), "bucket/key")
    with pytest.raises(PermissionError):
        handle.info()
    with pytest.raises(PermissionError):
        handle.exists()


def test_selector_absence_policy_and_strict_deletes_are_typed() -> None:
    filesystem = pafs.PyFileSystem(MemoryHandler())
    missing = IOBase.from_fs(filesystem, "missing")

    assert list(missing.iterdir()) == []
    with pytest.raises(FileNotFoundError):
        missing.delete_dir()
    with pytest.raises(FileNotFoundError):
        missing.delete_file()


def test_directory_operations_are_distinct(tmp_path: pathlib.Path) -> None:
    filesystem = pafs.LocalFileSystem()

    empty = tmp_path / "empty"
    empty.mkdir()
    IOBase.from_fs(filesystem, empty.as_posix()).delete_dir()
    assert not empty.exists()

    kept = tmp_path / "kept"
    kept.mkdir()
    (kept / "child").write_bytes(b"x")
    IOBase.from_fs(filesystem, kept.as_posix()).delete_dir_contents()
    assert kept.is_dir() and list(kept.iterdir()) == []

    nonempty = tmp_path / "nonempty"
    nonempty.mkdir()
    (nonempty / "child").write_bytes(b"x")
    with pytest.raises(OSError) as local_failure:
        IOBase.from_fs(filesystem, nonempty.as_posix()).delete_dir()
    assert local_failure.value.errno == errno.ENOTEMPTY
    assert (nonempty / "child").read_bytes() == b"x"

    class StrictHandler(MemoryHandler):
        def delete_dir(self, path: str) -> None:
            raise OSError(errno.ENOTEMPTY, "directory not empty")

        def delete_file(self, path: str) -> None:
            raise IsADirectoryError(path)

    strict_handler = StrictHandler()
    strict_handler.files["nonempty/child"] = b"x"
    strict = pafs.PyFileSystem(strict_handler)
    with pytest.raises(OSError) as failure:
        IOBase.from_fs(strict, "nonempty").delete_dir()
    assert failure.value.errno == errno.ENOTEMPTY

    with pytest.raises(IsADirectoryError):
        IOBase.from_fs(strict, "nonempty").delete_file()


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
