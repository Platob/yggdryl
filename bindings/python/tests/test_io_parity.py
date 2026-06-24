"""Parity tests: yggdryl.BytesIO and yggdryl.LocalPath behave the same for the
``stream`` flag and ``open``. The same assertions run against both handles.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def _bytesio(_tmp_path, data):
    return yggdryl.BytesIO(data)


def _localpath(tmp_path, data):
    path = str(tmp_path / "parity.bin")
    yggdryl.LocalPath(path).write(data)
    return yggdryl.LocalPath(path)


@pytest.fixture(params=[_bytesio, _localpath], ids=["bytesio", "localpath"])
def make(request, tmp_path):
    """Returns a factory `make(data) -> handle` for one handle kind."""
    return lambda data: request.param(tmp_path, data)


def test_stream_parity(make):
    # Streaming (the default): each read advances the cursor.
    io = make(b"abcdef")
    assert io.stream is True
    assert io.read(3) == b"abc"
    assert io.tell() == 3
    assert io.read() == b"def"
    assert io.tell() == 6

    # Non-streaming: the cursor stays put, so reads repeat.
    io = make(b"abcdef")
    io.stream = False
    assert io.stream is False
    assert io.read(3) == b"abc"
    assert io.read(3) == b"abc"
    assert io.tell() == 0


def test_stats_and_len_parity(make):
    io = make(b"abcdef")
    assert len(io) == 6
    assert io.stats().size == 6
    assert io.stats().kind == "file"


def test_readline_and_iteration_parity(make):
    io = make(b"one\ntwo\nthree")
    assert io.readline() == b"one\n"
    # Line iteration yields the rest.
    assert list(io) == [b"two\n", b"three"]


def test_context_manager_parity(make):
    # The handle itself works as a context manager (`with ... as io`).
    with make(b"abcdef") as io:
        assert io.read(3) == b"abc"

    # And so does the BytesIO returned by `open` (Python `with open(...)`).
    with make(b"abcdef").open("r") as f:
        assert f.read() == b"abcdef"


def test_open_parity(make):
    # Read open keeps the bytes, carries the stream flag and the mode.
    child = make(b"abcdef").open("r", stream=False)
    assert child.mode == "r"
    assert child.stream is False
    assert child.getvalue() == b"abcdef"

    # Write open truncates.
    child = make(b"abcdef").open("w")
    assert child.mode == "w"
    assert child.getvalue() == b""

    # Append open keeps the bytes with the cursor at the end.
    child = make(b"abcdef").open("a")
    assert child.mode == "a"
    assert child.tell() == 6
    assert child.getvalue() == b"abcdef"
