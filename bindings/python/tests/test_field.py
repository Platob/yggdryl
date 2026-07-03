"""Tests for the field wrappers (yggdryl.field) in the Python binding."""

import pytest

from yggdryl import dtype, field

# (field, name)
INTEGERS = [
    (field.Int8, "int8"),
    (field.Int16, "int16"),
    (field.Int32, "int32"),
    (field.Int64, "int64"),
    (field.UInt8, "uint8"),
    (field.UInt16, "uint16"),
    (field.UInt32, "uint32"),
    (field.UInt64, "uint64"),
]

IDS = [case[1] for case in INTEGERS]


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_field_pairs_a_name_with_the_type(case):
    field_class, name = case
    column = field_class("id", False)
    assert column.name() == "id"
    assert column.data_type().name() == name
    assert column.is_nullable() is False
    assert field_class("maybe").is_nullable() is True  # nullable by default


# (optional field, value type name)
OPTIONALS = [
    (field.OptionalInt8, "int8"),
    (field.OptionalInt16, "int16"),
    (field.OptionalInt32, "int32"),
    (field.OptionalInt64, "int64"),
    (field.OptionalUInt8, "uint8"),
    (field.OptionalUInt16, "uint16"),
    (field.OptionalUInt32, "uint32"),
    (field.OptionalUInt64, "uint64"),
]


@pytest.mark.parametrize("case", OPTIONALS, ids=[case[1] for case in OPTIONALS])
def test_optional_field(case):
    field_class, name = case
    score = field_class("score")
    assert score.name() == "score"
    assert score.is_nullable() is True
    assert score.data_type().name() == "optional"
    assert score.data_type().value_type().name() == name


def test_binary_field():
    payload = field.Binary("payload")
    assert payload.name() == "payload"
    assert payload.is_nullable() is True
    assert payload.data_type().name() == "binary"
    assert field.Binary("id", False).is_nullable() is False


def test_optional_binary_field():
    payload = field.OptionalBinary("payload")
    assert payload.name() == "payload"
    assert payload.data_type().name() == "optional"
    assert payload.data_type().value_type().name() == "binary"


def test_null_field():
    gap = field.Null("gap")
    assert (gap.name(), gap.data_type().name(), gap.is_nullable()) == ("gap", "null", True)


def test_union_field():
    union = dtype.Int64().optional().storage()
    value = field.Union("value", union)
    assert value.name() == "value"
    assert value.is_nullable() is True
    assert value.data_type().arrow_format() == "+us:0,1"
