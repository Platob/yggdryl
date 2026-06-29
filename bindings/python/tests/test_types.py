"""Cross-cutting tests for the yggdryl Python bindings."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl import Binary, BinaryType, Field, JsonFormat, Utf8, Utf8Type, Whence


def test_data_types():
    assert BinaryType().name == "binary"
    assert BinaryType(large=True).name == "large_binary"
    assert Utf8Type().name == "string"
    assert Utf8Type.from_str("utf8") == Utf8Type()
    assert BinaryType.from_bytes(BinaryType().to_bytes()) == BinaryType()
    assert Utf8Type.from_mapping(Utf8Type().to_mapping()) == Utf8Type()
    assert BinaryType() != Utf8Type()
    assert {BinaryType(): 1, Utf8Type(): 2}[Utf8Type()] == 2


def test_field_with_string_type():
    field = Field("name", Utf8Type(), nullable=False, metadata={"k": "v"})
    assert field.data_type == Utf8Type()
    assert Field.from_json(field.to_json()) == field
    with pytest.raises(ValueError):
        Field("bad", "string")  # not a yggdryl data type


def test_binary_value_and_io():
    buf = Binary(b"\x00\x01\x02")
    assert bytes(buf) == b"\x00\x01\x02"
    assert len(buf) == 3
    assert buf.data_type == BinaryType()
    assert Binary.from_bytes(buf.to_bytes()) == buf
    assert Binary.from_json(buf.to_json()) == buf

    buf = Binary()
    buf.write(b"hello ")
    buf.write(b"world")
    buf.seek(0, Whence.Start)
    assert bytes(buf.read(5)) == b"hello"
    assert bytes(buf.pread(6, 5)) == b"world"
    buf.resize(5, ord("."))
    assert bytes(buf) == b"hello"


def test_utf8_value():
    s = Utf8("héllo")
    assert s.value == "héllo"
    assert str(s) == "héllo"
    assert len(s) == len("héllo".encode())
    assert s.data_type == Utf8Type()
    assert Utf8.from_bytes(s.to_bytes()) == s
    assert Utf8.from_mapping(s.to_mapping()) == s
    assert Utf8.from_json(s.to_json()) == s
    with pytest.raises(ValueError):
        Utf8.from_bytes(b"\xff\xfe")


def test_cast_and_set_data_type():
    buf = Binary(b"hi")

    # cast binary -> string and back
    text = buf.cast(Utf8Type())
    assert isinstance(text, Utf8)
    assert text.value == "hi"
    assert text.cast(BinaryType()) == buf

    # cast binary -> string fails on non-UTF-8
    with pytest.raises(ValueError):
        Binary(b"\xff\xfe").cast(Utf8Type())

    # set_data_type: same family ok, cross family errors
    buf.set_data_type(BinaryType(large=True))
    assert buf.data_type == BinaryType(large=True)
    with pytest.raises(ValueError):
        Binary(b"hi").set_data_type(Utf8Type())


@pytest.mark.parametrize(
    "value",
    [
        BinaryType(),
        Utf8Type(large=True),
        Field("c", Utf8Type(), nullable=False, metadata={"k": "v"}),
        Binary(b"abc"),
        Binary(b"x", large=True),
        Utf8("hi"),
        Utf8("hi", large=True),
    ],
)
def test_pickle_and_copy_round_trip(value):
    assert pickle.loads(pickle.dumps(value)) == value
    assert copy.deepcopy(value) == value


def test_global_json_format():
    field = Field("c", BinaryType(), nullable=True)
    assert "\n" not in field.to_json()
    try:
        yggdryl.set_json_format(JsonFormat(pretty=True, indent=2))
        assert yggdryl.json_format().is_pretty
        assert yggdryl.json_format() == JsonFormat(pretty=True, indent=2)
        assert "\n" in field.to_json()
    finally:
        yggdryl.reset_json_format()
    assert "\n" not in field.to_json()
    assert yggdryl.json_format() == JsonFormat()


def test_module_exposes_expected_names():
    for name in ("BinaryType", "Utf8Type", "Field", "Binary", "Utf8", "Whence", "JsonFormat"):
        assert hasattr(yggdryl, name)
