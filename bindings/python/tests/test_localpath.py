"""Tests for the yggdryl Python extension's LocalPath and IoStats.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import os
import shutil
import tempfile

import yggdryl


def _temp(name: str, data: bytes) -> str:
    path = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_{name}")
    yggdryl.LocalPath(path).write(data)
    return path


def test_open_factory_dispatches_on_scheme():
    path = _temp("factory", b"by-the-factory")
    try:
        # A bare path (and a file:// URL) resolves to a LocalPath handle.
        io = yggdryl.open(path)
        assert isinstance(io, yggdryl.LocalPath)
        assert io.getvalue() == b"by-the-factory"
        assert yggdryl.open("file://" + path).getvalue() == b"by-the-factory"
        # A remote scheme is served by HttpSession, not this factory.
        import pytest

        with pytest.raises(ValueError):
            yggdryl.open("https://example.com/x")
    finally:
        os.remove(path)


def test_open_read_seek_and_random_access():
    path = _temp("read", b"hello world")
    try:
        io = yggdryl.LocalPath(path)
        assert io.location == path
        assert io.exists()
        assert len(io) == 11
        assert io.url.scheme == "file"

        # Streamed read advances the cursor.
        assert io.read(5) == b"hello"
        assert io.tell() == 5
        # The cross-language Io cursor accessors mirror `tell` / the file size.
        assert io.stream_position() == 5
        assert io.stream_len() == 11
        # Positional pread leaves the cursor put (size, offset, whence=0).
        assert io.pread(5, 6) == b"world"
        assert io.tell() == 5
        # getvalue returns the whole file regardless of the cursor.
        assert io.getvalue() == b"hello world"
        # Rewind and read the rest.
        io.seek(0)
        assert io.read() == b"hello world"
    finally:
        os.remove(path)


def test_stats():
    path = _temp("stats", b"0123456789")
    try:
        stats = yggdryl.LocalPath(path).stats()
        assert stats.size == 10
        assert stats.mtime is not None and stats.mtime > 0
        assert stats.content_type is None
    finally:
        os.remove(path)


def test_media_type_inferred_from_extension():
    path = _temp("media.csv", b"a,b,c\n1,2,3\n")
    try:
        io = yggdryl.LocalPath(path)
        media = io.media_type()
        assert media is not None
        assert media.first.subtype == "csv"
        # Surfaced through stats() too.
        assert io.stats().media_type is not None
    finally:
        os.remove(path)


def test_stats_classify_kind():
    # Missing — the instance is still constructible, with kind "missing".
    missing = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_nope")
    assert yggdryl.LocalPath(missing).stats().kind == "missing"
    assert not yggdryl.LocalPath(missing).stats().exists
    assert not yggdryl.LocalPath(missing).exists()

    # File.
    f = _temp("kind_file", b"hello")
    file_stats = yggdryl.LocalPath(f).stats()
    assert file_stats.kind == "file"
    assert file_stats.is_file and file_stats.exists
    assert file_stats.size == 5

    # Directory.
    d = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_kind_dir")
    os.makedirs(d, exist_ok=True)
    try:
        dir_stats = yggdryl.LocalPath(d).stats()
        assert dir_stats.kind == "directory"
        assert dir_stats.is_dir
    finally:
        os.remove(f)
        os.rmdir(d)


def test_write_auto_creates_missing_parent_dirs():
    base = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_autodir")
    nested = os.path.join(base, "a", "b", "c.bin")
    try:
        # The parent directories do not exist yet; the write creates them.
        yggdryl.LocalPath(nested).write(b"deep")
        assert yggdryl.LocalPath(nested).read() == b"deep"
    finally:
        shutil.rmtree(base, ignore_errors=True)


def test_cached_stats_get_set():
    f = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_cached.bin")
    yggdryl.LocalPath(f).write(b"hello")
    try:
        lp = yggdryl.LocalPath(f)
        # Held since construction -> always present for a path.
        assert lp.cached_stats().size == 5
        # Override and read it back through the cache peek and stats().
        lp.set_stats(yggdryl.IoStats(size=7, content_type="text/plain"))
        assert lp.cached_stats().content_type == "text/plain"
        assert lp.stats().content_type == "text/plain"
    finally:
        os.remove(f)
