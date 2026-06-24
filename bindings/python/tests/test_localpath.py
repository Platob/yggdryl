"""Tests for the yggdryl Python extension's LocalPath and IoStats.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import os
import tempfile

import pytest

import yggdryl


def _temp(name: str, data: bytes) -> str:
    path = os.path.join(tempfile.gettempdir(), f"yggdryl_py_{os.getpid()}_{name}")
    yggdryl.LocalPath.write(path, data)
    return path


def test_open_read_seek_and_random_access():
    path = _temp("read", b"hello world")
    try:
        io = yggdryl.LocalPath(path)
        assert io.location == path
        assert io.exists()
        assert len(io) == 11

        # Streamed read advances the cursor.
        assert io.read(5) == b"hello"
        assert io.tell() == 5
        # Positioned read leaves the cursor put.
        assert io.read_at(6, 5) == b"world"
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


def test_missing_path_raises():
    with pytest.raises(ValueError):
        yggdryl.LocalPath("/no/such/yggdryl/path")
