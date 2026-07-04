"""Tests for the type-inference factory (yggdryl.factory) in the Python binding."""

import pytest

from yggdryl import dtype, factory, field, scalar


def test_scalar_infers_the_type_from_the_value():
    # int -> int64, bytes -> binary, None -> null, list[int] -> int64 serie.
    answer = factory.scalar(42)
    assert answer.data_type().name() == "int64"
    assert answer.as_i64() == 42

    blob = factory.scalar(b"\x01\x02\x03")
    assert blob.data_type().name() == "binary"
    assert blob.as_bytes() == b"\x01\x02\x03"

    nothing = factory.scalar(None)
    assert nothing.data_type().name() == "null"
    assert nothing.is_null()

    numbers = factory.scalar([1, 2, 3])
    assert numbers.data_type().name() == "list"
    assert numbers.to_pylist() == [1, 2, 3]

    # bytearray also infers to binary.
    assert factory.scalar(bytearray(b"hi")).data_type().name() == "binary"
    # An empty list defaults to the int64 serie.
    assert factory.scalar([]).data_type().name() == "list"


def test_dict_infers_a_record_a_struct_and_a_struct_field():
    # A dict of named values -> a struct: scalar builds the RecordScalar row...
    row = factory.scalar({"x": 1, "blob": b"hi", "scores": [1, 2]})
    assert row.data_type().name() == "struct"
    assert row.field_names() == ["x", "blob", "scores"]
    assert row.to_pydict() == {"x": 1, "blob": b"hi", "scores": [1, 2]}

    # ...dtype the StructType...
    struct = factory.dtype({"x": 1, "gap": None})
    assert struct.name() == "struct"
    assert struct.child_count() == 2
    assert struct.field_names() == ["x", "gap"]

    # ...and field the StructField (nullable is respected).
    column = factory.field("row", {"x": 1}, nullable=False)
    assert column.name() == "row"
    assert column.data_type().name() == "struct"
    assert column.data_type().field_names() == ["x"]
    assert column.is_nullable() is False


def test_a_scalar_object_yields_a_new_handle_of_the_same_class():
    # Every model scalar object is an inference input: same class, same value,
    # a distinct handle.
    for original in (
        scalar.NullScalar(),
        scalar.BinaryScalar(b"hi"),
        scalar.Int64Scalar(42),
        scalar.UInt8Scalar(7),
        scalar.Int16Serie([1, 2]),
        scalar.RecordScalar({"x": 1}),
    ):
        copy = factory.scalar(original)
        assert type(copy) is type(original)
        assert copy is not original
    assert factory.scalar(scalar.Int64Scalar(42)).value() == 42
    assert factory.scalar(scalar.Int16Serie([1, 2])).to_pylist() == [1, 2]
    assert factory.scalar(scalar.RecordScalar({"x": 1})).get("x") == 1


def test_a_dtype_object_yields_its_default_scalar():
    assert factory.scalar(dtype.NullType()).is_null()
    assert factory.scalar(dtype.BinaryType()).value() == b""
    assert factory.scalar(dtype.Int64Type()).value() == 0
    assert factory.scalar(dtype.UInt8Type()).value() == 0
    empty = factory.scalar(dtype.Int32SerieType())
    assert empty.is_null() is False
    assert empty.to_pylist() == []


def test_dtype_infers_the_type_from_the_value():
    assert factory.dtype(42).name() == "int64"
    assert factory.dtype(b"x").name() == "binary"
    assert factory.dtype(None).name() == "null"
    assert factory.dtype([1, 2, 3]).name() == "list"

    # A model dtype object yields a same-type new instance...
    original = dtype.Int32Type()
    copy = factory.dtype(original)
    assert type(copy) is type(original)
    assert copy is not original
    struct = dtype.StructType({"x": 1})
    assert factory.dtype(struct).field_names() == ["x"]

    # ...a model scalar object its data type...
    assert factory.dtype(scalar.NullScalar()).name() == "null"
    assert factory.dtype(scalar.UInt16Scalar(3)).name() == "uint16"
    assert factory.dtype(scalar.Int8Serie([1])).value_type().name() == "int8"
    assert factory.dtype(scalar.RecordScalar({"x": 1})).field_names() == ["x"]

    # ...and a model field object its data type too.
    assert factory.dtype(field.Int64Field("id")).name() == "int64"
    assert factory.dtype(field.BinaryField("payload")).name() == "binary"
    assert factory.dtype(field.UInt8SerieField("bits")).name() == "list"
    assert factory.dtype(field.StructField("row", struct)).field_names() == ["x"]


def test_field_infers_the_type_and_keeps_the_name():
    id_field = factory.field("id", 42)
    assert id_field.name() == "id"
    assert id_field.data_type().name() == "int64"
    assert id_field.is_nullable()  # nullable defaults to True

    # nullable is respected.
    payload = factory.field("payload", b"x", nullable=False)
    assert payload.name() == "payload"
    assert payload.data_type().name() == "binary"
    assert not payload.is_nullable()

    scores = factory.field("scores", [1, 2, 3])
    assert scores.data_type().name() == "list"

    missing = factory.field("maybe", None)
    assert missing.data_type().name() == "null"

    # A model dtype object pairs the name with its field...
    assert type(factory.field("id", dtype.Int64Type())) is field.Int64Field
    assert type(factory.field("bits", dtype.UInt8SerieType())) is field.UInt8SerieField
    assert factory.field("gap", dtype.NullType()).data_type().name() == "null"
    struct_field = factory.field("row", dtype.StructType({"x": 1}), nullable=False)
    assert type(struct_field) is field.StructField
    assert struct_field.is_nullable() is False

    # ...and a model scalar object the field of its data type.
    assert type(factory.field("id", scalar.UInt32Scalar(1))) is field.UInt32Field
    assert type(factory.field("gap", scalar.NullScalar())) is field.NullField
    assert type(factory.field("scores", scalar.Int64Serie([1]))) is field.Int64SerieField
    assert type(factory.field("row", scalar.RecordScalar({"x": 1}))) is field.StructField


@pytest.mark.parametrize("value", [1.5, "text", True, {"a": 1.5}, [1, "x"]])
def test_unsupported_values_raise(value):
    # A float, str, bool, a dict with an uninferable value, or a non-int list
    # has no matching model type.
    with pytest.raises(ValueError):
        factory.scalar(value)
    with pytest.raises(ValueError):
        factory.dtype(value)


def test_an_int_outside_int64_raises():
    with pytest.raises(ValueError):
        factory.scalar(2 ** 64)


def test_a_non_str_dict_key_raises():
    with pytest.raises(ValueError, match="str field name"):
        factory.scalar({1: 2})
