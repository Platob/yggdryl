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

    # str -> utf8, symmetric with bytes -> binary.
    text = factory.scalar("héllo")
    assert text.data_type().name() == "utf8"
    assert text.value() == "héllo"
    assert text.as_bytes() == "héllo".encode("utf-8")

    # float -> float64; a list carrying a fractional value -> a float64 serie.
    weight = factory.scalar(1.5)
    assert weight.data_type().name() == "float64"
    assert weight.as_f64() == 1.5
    weights = factory.scalar([1.5, 2.5])
    assert weights.data_type().value_type().name() == "float64"
    assert weights.to_pylist() == [1.5, 2.5]
    # An all-whole list stays an int64 serie.
    assert factory.scalar([1, 2]).data_type().value_type().name() == "int64"


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
    assert factory.dtype(1.5).name() == "float64"
    assert factory.dtype("x").name() == "utf8"
    assert factory.dtype(b"x").name() == "binary"
    assert factory.dtype(None).name() == "null"
    assert factory.dtype([1, 2, 3]).name() == "list"
    assert factory.dtype([1.5, 2.5]).value_type().name() == "float64"

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

    weight = factory.field("w", 1.5)
    assert weight.name() == "w"
    assert weight.data_type().name() == "float64"

    label = factory.field("label", "text")
    assert label.name() == "label"
    assert label.data_type().name() == "utf8"

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


def test_float_handles_round_trip():
    # The float scalar / dtype / field handles are accepted like the integer ones.
    assert type(factory.scalar(scalar.Float64Scalar(1.5))) is scalar.Float64Scalar
    assert factory.scalar(scalar.Float32Serie([1.5])).to_pylist() == [1.5]
    assert factory.scalar(dtype.Float64Type()).value() == 0.0
    assert factory.scalar(dtype.Float32SerieType()).to_pylist() == []
    assert factory.dtype(scalar.Float32Scalar(1.5)).name() == "float32"
    assert factory.dtype(dtype.Float64Type()).name() == "float64"
    assert factory.dtype(field.Float64Field("w")).name() == "float64"
    assert factory.dtype(field.Float32SerieField("ws")).value_type().name() == "float32"
    assert type(factory.field("w", dtype.Float64Type())) is field.Float64Field
    assert type(factory.field("w", scalar.Float32Scalar(1.5))) is field.Float32Field
    assert type(factory.field("ws", dtype.Float64SerieType())) is field.Float64SerieField


def test_float16_and_string_handles_round_trip():
    # The float16 scalar / dtype / field / serie handles are accepted like the rest.
    assert type(factory.scalar(scalar.Float16Scalar(1.5))) is scalar.Float16Scalar
    assert factory.scalar(scalar.Float16Serie([1.5])).to_pylist() == [1.5]
    assert factory.scalar(dtype.Float16Type()).value() == 0.0
    assert factory.scalar(dtype.Float16SerieType()).to_pylist() == []
    assert factory.dtype(scalar.Float16Scalar(1.5)).name() == "float16"
    assert factory.dtype(field.Float16Field("w")).name() == "float16"
    assert factory.dtype(field.Float16SerieField("ws")).value_type().name() == "float16"
    assert type(factory.field("w", dtype.Float16Type())) is field.Float16Field
    assert type(factory.field("w", scalar.Float16Scalar(1.5))) is field.Float16Field

    # The string scalar / dtype / field handles are accepted too.
    assert type(factory.scalar(scalar.StringScalar("hi"))) is scalar.StringScalar
    assert factory.scalar(dtype.StringType()).value() == ""
    assert factory.dtype(scalar.StringScalar("hi")).name() == "utf8"
    assert factory.dtype(dtype.StringType()).name() == "utf8"
    assert factory.dtype(field.StringField("s")).name() == "utf8"
    assert type(factory.field("s", dtype.StringType())) is field.StringField
    assert type(factory.field("s", scalar.StringScalar("hi"))) is field.StringField


@pytest.mark.parametrize("value", [True, [1, "x"]])
def test_unsupported_values_raise(value):
    # A bool, or a list mixing numbers and non-numbers has no matching model type
    # (a float, a str and a dict of inferable values are all supported).
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
