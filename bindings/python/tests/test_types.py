"""Cross-cutting tests for the yggdryl Python bindings."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl import Binary, BinaryScalar, Field, StringScalar, Utf8


def test_binary_datatype_round_trips():
    b = Binary()
    assert b.name == "binary"
    assert str(b) == "binary"
    assert not b.is_large
    assert not b.is_utf8
    assert Binary(large=True).name == "large_binary"

    assert Binary.from_mapping(b.to_mapping()) == b
    assert Binary.from_bytes(b.to_bytes()) == b
    assert Binary.from_json(b.to_json()) == b


def test_utf8_datatype_aliases():
    s = Utf8()
    assert s.name == "string"
    assert s.is_utf8
    assert Utf8(large=True).name == "large_string"
    assert Utf8.from_bytes(b"string") == s


def test_datatypes_are_hashable_and_distinct():
    seen = {Binary(): "b", Utf8(): "s", Binary(large=True): "lb"}
    assert seen[Binary()] == "b"
    assert seen[Utf8()] == "s"
    assert len(seen) == 3
    assert Binary() != Utf8()


def test_field_round_trips_with_metadata():
    field = Field("payload", Binary(large=True), nullable=False, metadata={"unit": "bytes"})
    assert field.name == "payload"
    assert field.data_type == Binary(large=True)
    assert not field.nullable
    assert field.metadata == {"unit": "bytes"}

    assert Field.from_mapping(field.to_mapping()) == field
    assert Field.from_bytes(field.to_bytes()) == field
    assert Field.from_json(field.to_json()) == field


def test_field_defaults_nullable_true():
    assert Field("id", Utf8()).nullable is True


def test_field_with_helpers_do_not_mutate():
    field = Field("a", Binary(), nullable=True)
    renamed = field.with_name("b").with_nullable(False)
    assert field.name == "a" and field.nullable is True
    assert renamed.name == "b" and renamed.nullable is False


def test_field_rejects_non_datatype():
    with pytest.raises(ValueError):
        Field("bad", "binary")  # a plain string is not a yggdryl data type


def test_binary_scalar():
    scalar = BinaryScalar(b"\x00\x01\x02")
    assert scalar.value == b"\x00\x01\x02"
    assert not scalar.is_null
    assert len(scalar) == 3
    assert scalar.data_type == Binary()
    assert BinaryScalar().is_null
    assert BinaryScalar.null().value is None
    assert BinaryScalar.from_json(scalar.to_json()) == scalar


def test_string_scalar():
    scalar = StringScalar("yggdryl")
    assert scalar.value == "yggdryl"
    assert str(scalar) == "yggdryl"
    assert scalar.data_type == Utf8()
    assert StringScalar(None).is_null
    assert StringScalar.from_json(scalar.to_json()) == scalar


@pytest.mark.parametrize(
    "value",
    [
        Binary(),
        Binary(large=True),
        Utf8(),
        Field("c", Utf8(), nullable=False, metadata={"k": "v"}),
        BinaryScalar(b"abc"),
        BinaryScalar(),
        StringScalar("hi"),
        StringScalar(None),
    ],
)
def test_pickle_and_copy_round_trip(value):
    assert pickle.loads(pickle.dumps(value)) == value
    assert copy.copy(value) == value
    assert copy.deepcopy(value) == value


def test_module_exposes_expected_names():
    for name in ("Binary", "Utf8", "Field", "BinaryScalar", "StringScalar"):
        assert hasattr(yggdryl, name)
