"""Tests for the yggdryl Python extension's BytesIO.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def test_mode_and_open():
    io = yggdryl.BytesIO(b"hello")
    assert io.mode == "r"
    # Read open keeps the bytes; child carries the mode and stream flag.
    child = io.open("rb", stream=False)
    assert child.mode == "r"
    assert child.getvalue() == b"hello"
    assert child.stream is False
    # Write truncates; append (a) positions at the end.
    assert yggdryl.BytesIO(b"abc").open("w").getvalue() == b""
    appender = yggdryl.BytesIO(b"abc").open("a")
    assert appender.mode == "a"
    assert appender.tell() == 3
    # `a+`/`r+` are read-write (cursor at the start).
    rw = yggdryl.BytesIO(b"abc").open("a+")
    assert rw.mode == "r+"
    assert rw.tell() == 0
    with pytest.raises(ValueError):
        io.open("nope")


def test_capacity_reserve_and_truncate():
    io = yggdryl.BytesIO.with_capacity(64)
    assert io.capacity >= 64
    io.reserve_capacity(128)
    assert io.capacity >= 128
    io.write(b"abc")
    # truncate grows (zero-fill) and shrinks.
    assert io.truncate(5) == 5
    assert io.getvalue() == b"abc\x00\x00"
    assert io.truncate(2) == 2
    assert io.getvalue() == b"ab"


def test_url_pread_pwrite():
    io = yggdryl.BytesIO(b"0123456789")
    # Every IO carries a URL; in-memory uses the mem scheme.
    assert io.url.scheme == "mem"
    io.seek(4)
    # Positional pread/pwrite leave the cursor put (whence=0 default).
    assert io.pread(2, 6) == b"67"
    assert io.tell() == 4
    assert io.pwrite(b"AB", 0) == 2
    assert io.getvalue()[:2] == b"AB"
    assert io.tell() == 4
    # Cursor-relative (whence=1) uses and advances the cursor.
    assert io.pread(2, 0, 1) == b"45"
    assert io.tell() == 6


def test_read_advances_the_cursor():
    io = yggdryl.BytesIO(b"hello world")
    assert io.read(5) == b"hello"
    assert io.tell() == 5
    assert io.read(1) == b" "
    # No size (or a negative one) reads the rest.
    assert io.read() == b"world"
    assert io.read(-1) == b""
    assert io.tell() == 11
    assert len(io) == 11


def test_getvalue_ignores_the_cursor():
    io = yggdryl.BytesIO(b"abcdef")
    io.read(3)
    assert io.getvalue() == b"abcdef"
    assert io.tell() == 3


def test_stream_flag_keeps_the_cursor_fixed():
    io = yggdryl.BytesIO(b"abcdef", stream=False)
    assert not io.stream
    assert io.read(3) == b"abc"
    assert io.read(3) == b"abc"
    assert io.tell() == 0
    # Toggling the flag back on resumes streaming.
    io.stream = True
    assert io.read(3) == b"abc"
    assert io.tell() == 3


def test_seek_whences_and_errors():
    io = yggdryl.BytesIO(b"0123456789")
    assert io.seek(4) == 4
    assert io.seek(2, 1) == 6
    assert io.seek(-1, 2) == 9
    assert io.read() == b"9"
    with pytest.raises(ValueError):
        io.seek(-1)
    with pytest.raises(ValueError):
        io.seek(0, 9)


def test_write_overwrites_and_zero_fills():
    io = yggdryl.BytesIO(b"abc")
    io.seek(1)
    assert io.write(b"XY") == 2
    assert io.getvalue() == b"aXY"
    io.seek(5)
    io.write(b"Z")
    assert io.getvalue() == b"aXY\x00\x00Z"


def test_readline_and_iteration():
    io = yggdryl.BytesIO(b"one\ntwo\nthree")
    assert io.readline() == b"one\n"
    assert list(io) == [b"two\n", b"three"]


def test_truncate():
    io = yggdryl.BytesIO(b"abcdef")
    io.seek(3)
    assert io.truncate() == 3
    assert io.getvalue() == b"abc"
    assert io.truncate(1) == 1
    assert io.getvalue() == b"a"
