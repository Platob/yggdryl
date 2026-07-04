"""Tests for the field wrappers (yggdryl.field) in the Python binding."""

import pytest

from yggdryl import dtype, field

# (field, name)
INTEGERS = [
    (field.Int8Field, "int8"),
    (field.Int16Field, "int16"),
    (field.Int32Field, "int32"),
    (field.Int64Field, "int64"),
    (field.UInt8Field, "uint8"),
    (field.UInt16Field, "uint16"),
    (field.UInt32Field, "uint32"),
    (field.UInt64Field, "uint64"),
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
    (field.OptionalInt8Field, "int8"),
    (field.OptionalInt16Field, "int16"),
    (field.OptionalInt32Field, "int32"),
    (field.OptionalInt64Field, "int64"),
    (field.OptionalUInt8Field, "uint8"),
    (field.OptionalUInt16Field, "uint16"),
    (field.OptionalUInt32Field, "uint32"),
    (field.OptionalUInt64Field, "uint64"),
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
    payload = field.BinaryField("payload")
    assert payload.name() == "payload"
    assert payload.is_nullable() is True
    assert payload.data_type().name() == "binary"
    assert field.BinaryField("id", False).is_nullable() is False


def test_optional_binary_field():
    payload = field.OptionalBinaryField("payload")
    assert payload.name() == "payload"
    assert payload.data_type().name() == "optional"
    assert payload.data_type().value_type().name() == "binary"


def test_null_field():
    gap = field.NullField("gap")
    assert (gap.name(), gap.data_type().name(), gap.is_nullable()) == ("gap", "null", True)


def test_union_field():
    union = dtype.Int64Type().optional().storage()
    value = field.UnionField("value", union)
    assert value.name() == "value"
    assert value.is_nullable() is True
    assert value.data_type().arrow_format() == "+us:0,1"


def test_data_type_field_factory_matches_the_field_class():
    # The data type's field() factory builds the same field as the class,
    # including the shared nullable-by-default.
    built = dtype.Int64Type().field("id")
    direct = field.Int64Field("id")
    assert (built.name(), built.data_type().name(), built.is_nullable()) == (
        direct.name(),
        direct.data_type().name(),
        direct.is_nullable(),
    )


# (serie field, serie type, value type name)
SERIES = [
    (field.Int8SerieField, dtype.Int8SerieType, "int8"),
    (field.Int16SerieField, dtype.Int16SerieType, "int16"),
    (field.Int32SerieField, dtype.Int32SerieType, "int32"),
    (field.Int64SerieField, dtype.Int64SerieType, "int64"),
    (field.UInt8SerieField, dtype.UInt8SerieType, "uint8"),
    (field.UInt16SerieField, dtype.UInt16SerieType, "uint16"),
    (field.UInt32SerieField, dtype.UInt32SerieType, "uint32"),
    (field.UInt64SerieField, dtype.UInt64SerieType, "uint64"),
]


@pytest.mark.parametrize("case", SERIES, ids=[case[2] for case in SERIES])
def test_serie_field(case):
    field_class, serie_type, name = case
    scores = field_class("scores")
    assert scores.name() == "scores"
    assert scores.is_nullable() is True
    assert scores.data_type().name() == "list"
    assert scores.data_type().value_type().name() == name
    assert field_class("scores", False).is_nullable() is False
    # The data type's factory builds the same field.
    assert serie_type().field("scores").data_type().name() == "list"


def test_display_renders_name_and_type():
    # repr/str/display render `name: type`, with a trailing `?` when nullable.
    strict = field.Int64Field("id", False)
    assert strict.display() == "id: int64"
    assert repr(strict) == "id: int64"
    assert str(strict) == "id: int64"
    assert field.Int64Field("age").display() == "age: int64?"  # nullable
    assert field.Int64SerieField("scores", False).display() == "scores: list<int64>"
