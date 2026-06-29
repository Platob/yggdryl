"""Cross-cutting tests for the yggdryl Python bindings."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl import Binary, BinaryType, Field, Utf8, Whence


def test_binary_type_round_trips():
    b = BinaryType()
    assert b.name == "binary"
    assert str(b) == "binary"
    assert not b.is_large
    assert BinaryType(large=True).name == "large_binary"

    assert BinaryType.from_str("large_binary") == BinaryType(large=True)
    assert BinaryType.from_mapping(b.to_mapping()) == b
    assert BinaryType.from_bytes(b.to_bytes()) == b
    assert BinaryType.from_json(b.to_json()) == b


def test_utf8_type_aliases():
    s = Utf8()
    assert s.name == "string"
    assert s.is_utf8
    assert Utf8.from_str("utf8") == s
    assert Utf8.from_str("large_utf8") == Utf8(large=True)


def test_types_are_hashable_and_distinct():
    seen = {BinaryType(): "b", Utf8(): "s", BinaryType(large=True): "lb"}
    assert seen[BinaryType()] == "b"
    assert len(seen) == 3
    assert BinaryType() != Utf8()


def test_field_round_trips_with_metadata():
    field = Field("payload", BinaryType(large=True), nullable=False, metadata={"unit": "bytes"})
    assert field.name == "payload"
    assert field.data_type == BinaryType(large=True)
    assert not field.nullable
    assert field.metadata == {"unit": "bytes"}

    assert Field.from_mapping(field.to_mapping()) == field
    assert Field.from_bytes(field.to_bytes()) == field
    assert Field.from_json(field.to_json()) == field


def test_field_defaults_nullable_true_and_rejects_non_type():
    assert Field("id", Utf8()).nullable is True
    with pytest.raises(ValueError):
        Field("bad", "binary")  # a plain string is not a yggdryl data type


def test_binary_buffer_value_and_serialization():
    buf = Binary(b"\x00\x01\x02")
    assert bytes(buf) == b"\x00\x01\x02"
    assert buf.to_bytes() == b"\x00\x01\x02"
    assert len(buf) == 3
    assert buf.data_type == BinaryType()

    assert Binary.from_bytes(buf.to_bytes()) == buf
    assert Binary.from_mapping(buf.to_mapping()) == buf
    assert Binary.from_json(buf.to_json()) == buf

    large = Binary(b"x", large=True)
    assert large.data_type == BinaryType(large=True)
    assert Binary.from_mapping(large.to_mapping()) == large


def test_binary_implements_io():
    buf = Binary()
    assert buf.write(b"hello ") == 6
    assert buf.write(b"world") == 5
    assert buf.size == 11
    assert buf.capacity >= 11

    buf.seek(0, Whence.Start)
    assert bytes(buf.read(5)) == b"hello"
    assert buf.tell() == 5
    assert bytes(buf.pread(6, 5)) == b"world"

    buf.pwrite(0, b"HELLO")
    assert bytes(buf) == b"HELLO world"

    buf.resize(5, ord("."))
    assert bytes(buf) == b"HELLO"
    buf.resize(7, ord("!"))
    assert bytes(buf) == b"HELLO!!"


def test_binary_seek_errors():
    buf = Binary(b"0123456789")
    assert buf.seek(-1, Whence.End) == 9
    with pytest.raises(ValueError):
        buf.seek(-100, Whence.Start)


@pytest.mark.parametrize(
    "value",
    [
        BinaryType(),
        BinaryType(large=True),
        Utf8(),
        Field("c", Utf8(), nullable=False, metadata={"k": "v"}),
        Binary(b"abc"),
        Binary(b"x", large=True),
        Binary(),
    ],
)
def test_pickle_and_copy_round_trip(value):
    assert pickle.loads(pickle.dumps(value)) == value
    assert copy.copy(value) == value
    assert copy.deepcopy(value) == value


def test_module_exposes_expected_names():
    for name in ("BinaryType", "Utf8", "Field", "Binary", "Whence"):
        assert hasattr(yggdryl, name)
