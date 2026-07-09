"""ByteCursor.write — runtime type inference (bytes / str / int-seq / float-seq)."""

import pytest

from yggdryl.io import ByteBuffer, Whence


def test_write_bytes():
    c = ByteBuffer().byte_cursor()
    assert c.write(b"hello") == 5
    assert c.as_bytes() == b"hello"


def test_write_bytearray():
    c = ByteBuffer().byte_cursor()
    assert c.write(bytearray(b"abc")) == 3
    assert c.as_bytes() == b"abc"


def test_write_str_utf8():
    c = ByteBuffer().byte_cursor()
    n = c.write("café")
    assert n == len("café".encode())
    assert c.as_bytes() == "café".encode()


def test_write_int_sequence_as_i64():
    c = ByteBuffer().byte_cursor()
    assert c.write([1, 2, 3]) == 24  # 3 * 8 bytes
    assert c.pread_i64_array(3, Whence.Start) == [1, 2, 3]


def test_write_float_sequence_as_f64():
    c = ByteBuffer().byte_cursor()
    assert c.write([1.5, 2.5]) == 16
    assert c.pread_f64_array(2, Whence.Start) == [1.5, 2.5]


def test_write_accepts_tuple():
    c = ByteBuffer().byte_cursor()
    assert c.write((10, 20)) == 16


def test_write_empty_sequence_is_zero():
    c = ByteBuffer().byte_cursor()
    assert c.write([]) == 0
    assert c.as_bytes() == b""


def test_write_at_whence_auto_grows():
    c = ByteBuffer(b"ab").byte_cursor()
    c.seek(5, Whence.Start)
    c.write(b"XY", Whence.Current)
    assert c.as_bytes() == b"ab\x00\x00\x00XY"


def test_write_bool_sequence_rejected():
    c = ByteBuffer().byte_cursor()
    with pytest.raises(ValueError, match="BooleanBuffer"):
        c.write([True, False])


def test_write_mixed_sequence_rejected():
    c = ByteBuffer().byte_cursor()
    with pytest.raises(ValueError):
        c.write([1, 2.5])


def test_write_unsupported_types():
    c = ByteBuffer().byte_cursor()
    with pytest.raises(TypeError):
        c.write(42)  # a bare int is not bytes/str/sequence
    with pytest.raises(TypeError):
        c.write(["a", "b"])  # sequence of str
