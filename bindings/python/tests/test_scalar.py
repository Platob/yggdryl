"""Tests for the scalar wrappers (yggdryl.scalar) in the Python binding."""

import pytest

from yggdryl import core, scalar

# (scalar, optional scalar, name, min, max)
INTEGERS = [
    (scalar.Int8Scalar, scalar.OptionalInt8Scalar, "int8", -(2 ** 7), 2 ** 7 - 1),
    (scalar.Int16Scalar, scalar.OptionalInt16Scalar, "int16", -(2 ** 15), 2 ** 15 - 1),
    (scalar.Int32Scalar, scalar.OptionalInt32Scalar, "int32", -(2 ** 31), 2 ** 31 - 1),
    (scalar.Int64Scalar, scalar.OptionalInt64Scalar, "int64", -(2 ** 63), 2 ** 63 - 1),
    (scalar.UInt8Scalar, scalar.OptionalUInt8Scalar, "uint8", 0, 2 ** 8 - 1),
    (scalar.UInt16Scalar, scalar.OptionalUInt16Scalar, "uint16", 0, 2 ** 16 - 1),
    (scalar.UInt32Scalar, scalar.OptionalUInt32Scalar, "uint32", 0, 2 ** 32 - 1),
    (scalar.UInt64Scalar, scalar.OptionalUInt64Scalar, "uint64", 0, 2 ** 64 - 1),
]

IDS = [case[2] for case in INTEGERS]

# (scalar, optional scalar, native float accessor, name)
FLOATS = [
    (scalar.Float16Scalar, scalar.OptionalFloat16Scalar, "as_f16", "float16"),
    (scalar.Float32Scalar, scalar.OptionalFloat32Scalar, "as_f32", "float32"),
    (scalar.Float64Scalar, scalar.OptionalFloat64Scalar, "as_f64", "float64"),
]

FLOAT_IDS = [case[3] for case in FLOATS]


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_scalar_holds_a_value_or_null(case):
    scalar_class, _, name, low, high = case
    answer = scalar_class(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.data_type().name() == name
    assert scalar_class(low).value() == low
    assert scalar_class(high).value() == high

    missing = scalar_class.null()
    assert missing.is_null() is True
    assert missing.value() is None


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_accessors_convert_exactly(case):
    scalar_class, _, _, _, high = case
    answer = scalar_class(42)
    # A small value converts to every numeric target.
    for accessor in ("as_i8", "as_i16", "as_i32", "as_i64",
                     "as_u8", "as_u16", "as_u32", "as_u64"):
        assert getattr(answer, accessor)() == 42
    assert answer.as_f32() == 42.0
    assert answer.as_f64() == 42.0
    # An integer is never a bool, a str or bytes: an actionable ValueError.
    with pytest.raises(ValueError, match="no bool conversion"):
        answer.as_bool()
    with pytest.raises(ValueError, match="no str conversion"):
        answer.as_str()
    with pytest.raises(ValueError, match="no bytes conversion"):
        answer.as_bytes()
    # A null scalar holds no value: every accessor raises.
    with pytest.raises(ValueError, match="is null"):
        scalar_class.null().as_i64()
    # The extreme converts only where it is exactly representable.
    if high <= 2 ** 7 - 1:
        assert scalar_class(high).as_i8() == high
    else:
        with pytest.raises(ValueError, match="not exactly representable"):
            scalar_class(high).as_i8()


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_optional_scalar_redirects_to_the_inner_scalar(case):
    _, optional, name, _, _ = case
    answer = optional(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.scalar().value() == 42
    assert answer.as_i64() == 42

    # The data type is the logical optional over union storage.
    opt_type = answer.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.arrow_format() == "+us:0,1"
    assert opt_type.byte_width() is None
    assert opt_type.value_type().name() == name

    missing = optional.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_i64()


def test_float_access_is_exact_or_raises():
    # 2**53 is the last contiguous integer in f64; 2**53 + 1 rounds.
    assert scalar.Int64Scalar(2 ** 53).as_f64() == float(2 ** 53)
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.Int64Scalar(2 ** 53 + 1).as_f64()
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.UInt64Scalar(2 ** 64 - 1).as_f64()
    # Sign changes never pass, and the error names the offending value.
    with pytest.raises(ValueError, match="-1 is not exactly representable"):
        scalar.Int8Scalar(-1).as_u64()


@pytest.mark.parametrize("case", FLOATS, ids=FLOAT_IDS)
def test_float_scalar_holds_a_value_or_null(case):
    scalar_class, _, native, name = case
    weight = scalar_class(1.5)  # halves are exact in both f32 and f64
    assert weight.is_null() is False
    assert weight.value() == 1.5
    assert weight.data_type().name() == name
    assert getattr(weight, native)() == 1.5
    assert weight.as_f64() == 1.5
    assert weight.to_pyvalue() == 1.5

    missing = scalar_class.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.to_pyvalue() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_f64()


@pytest.mark.parametrize("case", FLOATS, ids=FLOAT_IDS)
def test_float_scalar_reads_as_int_only_when_whole(case):
    scalar_class, _, _, _ = case
    # A whole float converts to every integer target it fits.
    assert scalar_class(42.0).as_i64() == 42
    assert scalar_class(42.0).as_u8() == 42
    # A fractional value is inexact for every integer target.
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar_class(1.5).as_i64()
    # A float is never a bool.
    with pytest.raises(ValueError, match="no bool conversion"):
        scalar_class(1.5).as_bool()


@pytest.mark.parametrize("case", FLOATS, ids=FLOAT_IDS)
def test_optional_float_scalar_redirects_to_the_inner_scalar(case):
    _, optional, _, name = case
    weight = optional(1.5)
    assert weight.is_null() is False
    assert weight.value() == 1.5
    assert weight.scalar().value() == 1.5
    assert weight.as_f64() == 1.5
    assert weight.to_pyvalue() == 1.5

    opt_type = weight.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.value_type().name() == name

    missing = optional.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.scalar() is None
    assert missing.to_pyvalue() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_f64()


def test_float16_widens_through_every_float_accessor():
    # The native half::f16 crosses as a Python float; every float accessor widens.
    weight = scalar.Float16Scalar(1.5)  # 1.5 is exact in f16
    assert weight.value() == 1.5
    assert weight.to_pyvalue() == 1.5
    assert weight.as_f16() == 1.5
    assert weight.as_f32() == 1.5
    assert weight.as_f64() == 1.5
    assert weight.data_type().name() == "float16"
    # A whole float16 reads as an int; a fractional one never does.
    assert scalar.Float16Scalar(3.0).as_i64() == 3
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.Float16Scalar(1.5).as_i64()
    # Null holds no value.
    missing = scalar.Float16Scalar.null()
    assert missing.is_null() is True
    assert missing.value() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_f16()


def test_as_f16_is_available_on_every_scalar():
    # as_f16 sits alongside as_f32 / as_f64 on every scalar, widening f16 to a float.
    assert scalar.Int64Scalar(3).as_f16() == 3.0
    assert scalar.Float64Scalar(0.5).as_f16() == 0.5
    # A value with no exact f16 raises, naming the fix.
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.Int64Scalar(123457).as_f16()
    # A non-numeric value has no f16 form.
    with pytest.raises(ValueError, match="no f16 conversion"):
        scalar.BinaryScalar(b"hi").as_f16()


def test_string_scalar_reads_text_and_bytes():
    greeting = scalar.Utf8Scalar("hi")
    assert greeting.is_null() is False
    assert greeting.value() == "hi"
    assert greeting.to_pyvalue() == "hi"
    assert greeting.as_str() == "hi"
    assert greeting.as_bytes() == b"hi"
    assert greeting.data_type().name() == "utf8"
    # Unicode round-trips as text, and its UTF-8 bytes are reachable.
    accented = scalar.Utf8Scalar("hé")
    assert accented.value() == "hé"
    assert accented.as_bytes() == b"h\xc3\xa9"
    # A string has no numeric form.
    with pytest.raises(ValueError, match="no i64 conversion"):
        greeting.as_i64()

    # The empty string and null are distinct states.
    assert scalar.Utf8Scalar("").is_null() is False
    missing = scalar.Utf8Scalar.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.to_pyvalue() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_str()


def test_optional_string_redirects_to_the_inner_scalar():
    some = scalar.OptionalUtf8Scalar("hi")
    assert some.is_null() is False
    assert some.value() == "hi"
    assert some.scalar().value() == "hi"
    assert some.as_str() == "hi"
    assert some.as_bytes() == b"hi"

    opt_type = some.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.value_type().name() == "utf8"
    assert opt_type.storage().name() == "union"

    missing = scalar.OptionalUtf8Scalar.null()
    assert missing.is_null() is True
    assert missing.scalar() is None
    assert missing.to_pyvalue() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_str()


def test_binary_scalar_reads_bytes_and_io():
    blob = scalar.BinaryScalar(b"\x01\x02\x03")
    assert blob.is_null() is False
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    assert blob.data_type().name() == "binary"
    # UTF-8 bytes convert to str; anything else raises naming the shape — and
    # an explicit core charset decodes instead.
    assert scalar.BinaryScalar(b"hi").as_str() == "hi"
    assert scalar.BinaryScalar(b"hi").as_str("utf8") == "hi"
    assert scalar.BinaryScalar(b"\xe9").as_str("latin1") == "é"
    with pytest.raises(ValueError, match="non-UTF-8"):
        scalar.BinaryScalar(b"\xff").as_str()
    with pytest.raises(ValueError, match="unknown charset"):
        scalar.BinaryScalar(b"hi").as_str("ascii")
    with pytest.raises(ValueError, match="no i64 conversion"):
        blob.as_i64()

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.byte_size() == 3
    assert io.to_bytes() == b"\x01\x02\x03"
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    # ... or as a full-window ByteBufferSlice for window-relative reads.
    window = blob.to_io_slice()
    assert window.byte_size() == 3
    assert window.pread_byte_one(1, core.Whence.Start) == 2
    assert window.pread_i8(2, core.Whence.Start) == 3

    # The empty value and null are distinct states.
    assert scalar.BinaryScalar(b"").is_null() is False
    missing = scalar.BinaryScalar.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.to_io() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()


def test_optional_binary_redirects_to_the_inner_scalar():
    some = scalar.OptionalBinaryScalar(b"hi")
    assert some.is_null() is False
    assert some.value() == b"hi"
    assert some.scalar().value() == b"hi"
    assert some.as_bytes() == b"hi"
    assert some.as_str() == "hi"

    opt_type = some.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.value_type().name() == "binary"
    assert opt_type.storage().name() == "union"

    missing = scalar.OptionalBinaryScalar.null()
    assert missing.is_null() is True
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()


def test_null_scalar():
    nothing = scalar.NullScalar()
    assert nothing.is_null() is True
    assert nothing.data_type().name() == "null"


# (serie scalar, value type name, min, max)
SERIES = [
    (scalar.Int8Serie, "int8", -(2 ** 7), 2 ** 7 - 1),
    (scalar.Int16Serie, "int16", -(2 ** 15), 2 ** 15 - 1),
    (scalar.Int32Serie, "int32", -(2 ** 31), 2 ** 31 - 1),
    (scalar.Int64Serie, "int64", -(2 ** 63), 2 ** 63 - 1),
    (scalar.UInt8Serie, "uint8", 0, 2 ** 8 - 1),
    (scalar.UInt16Serie, "uint16", 0, 2 ** 16 - 1),
    (scalar.UInt32Serie, "uint32", 0, 2 ** 32 - 1),
    (scalar.UInt64Serie, "uint64", 0, 2 ** 64 - 1),
]


@pytest.mark.parametrize("case", SERIES, ids=[case[1] for case in SERIES])
def test_serie_holds_a_sequence(case):
    serie_class, name, low, high = case
    numbers = serie_class([low, 2, high])
    assert numbers.is_null() is False
    assert numbers.is_empty() is False
    assert numbers.len() == 3
    assert numbers.to_pylist() == [low, 2, high]  # extremes survive the buffer
    assert numbers.get_at(0) == low
    assert numbers.get_at(1) == 2
    assert numbers.get_at(2) == high
    assert numbers.get_scalar_at(2).value() == high
    assert numbers.get_scalar_at(3) is None  # out of bounds
    assert numbers.data_type().name() == "list"
    assert numbers.data_type().value_type().name() == name
    with pytest.raises(ValueError):
        numbers.get_at(3)  # out of bounds
    with pytest.raises(OverflowError):
        numbers.get_at(-1)  # a negative index never converts

    # The empty serie and null are distinct states.
    empty = serie_class([])
    assert empty.is_null() is False
    assert empty.is_empty() is True
    assert empty.to_pylist() == []

    missing = serie_class.null()
    assert missing.is_null() is True
    assert missing.to_pylist() is None
    with pytest.raises(ValueError):
        missing.get_at(0)


# (serie scalar, value type name)
FLOAT_SERIES = [
    (scalar.Float16Serie, "float16"),
    (scalar.Float32Serie, "float32"),
    (scalar.Float64Serie, "float64"),
]


@pytest.mark.parametrize("case", FLOAT_SERIES, ids=[case[1] for case in FLOAT_SERIES])
def test_float_serie_holds_a_sequence(case):
    serie_class, name = case
    weights = serie_class([1.5, 2.5, 3.5])  # halves survive the buffer exactly
    assert weights.is_null() is False
    assert weights.is_empty() is False
    assert weights.len() == 3
    assert weights.to_pylist() == [1.5, 2.5, 3.5]
    assert weights.to_pyvalue() == [1.5, 2.5, 3.5]
    assert weights.get_at(1) == 2.5
    assert weights.get_scalar_at(2).value() == 3.5
    assert weights.get_scalar_at(3) is None  # out of bounds
    assert weights.data_type().name() == "list"
    assert weights.data_type().value_type().name() == name
    with pytest.raises(OverflowError):
        weights.get_at(-1)  # a negative index never converts

    # The empty serie and null are distinct states.
    empty = serie_class([])
    assert empty.is_null() is False
    assert empty.is_empty() is True
    assert empty.to_pylist() == []

    missing = serie_class.null()
    assert missing.is_null() is True
    assert missing.to_pylist() is None
    with pytest.raises(ValueError):
        missing.get_at(0)


@pytest.mark.parametrize(
    "serie_class,values",
    [(case[0], [1, 2, 3]) for case in SERIES]
    + [(case[0], [1.5, 2.5]) for case in FLOAT_SERIES],
)
def test_serie_iterates_its_element_scalars(serie_class, values):
    serie = serie_class(values)

    # `for scalar in serie` yields the element scalar objects.
    assert [element.value() for element in serie] == values
    # list(serie) materializes the same scalars.
    assert [element.value() for element in list(serie)] == values
    # scalars() is the explicit list, the typed counterpart of to_pylist().
    assert [element.value() for element in serie.scalars()] == values

    # The empty serie iterates as empty; scalars() is []; a null serie is None.
    assert list(serie_class([])) == []
    assert serie_class([]).scalars() == []
    assert serie_class.null().scalars() is None
    assert list(serie_class.null()) == []  # a null serie iterates as empty


def test_to_pyvalue_is_the_general_native_accessor():
    # One call per scalar: the whole native value, or None when null.
    assert scalar.NullScalar().to_pyvalue() is None
    assert scalar.BinaryScalar(b"\x01\x02").to_pyvalue() == b"\x01\x02"
    assert scalar.BinaryScalar.null().to_pyvalue() is None
    assert scalar.OptionalBinaryScalar(b"hi").to_pyvalue() == b"hi"
    assert scalar.OptionalBinaryScalar.null().to_pyvalue() is None
    for scalar_class, optional, _, low, high in INTEGERS:
        assert scalar_class(42).to_pyvalue() == 42
        assert scalar_class(low).to_pyvalue() == low
        assert scalar_class(high).to_pyvalue() == high
        assert scalar_class.null().to_pyvalue() is None
        assert optional(42).to_pyvalue() == 42
        assert optional.null().to_pyvalue() is None
    for serie_class, _, low, high in SERIES:
        assert serie_class([low, high]).to_pyvalue() == [low, high]
        assert serie_class([]).to_pyvalue() == []
        assert serie_class.null().to_pyvalue() is None
    # The record's native value is its singleton dataclass (see test_record.py).
    import dataclasses

    row = scalar.RecordScalar({"x": 1}).to_pyvalue()
    assert dataclasses.is_dataclass(row)
    assert row.x == 1


# (scalar class, a value, a different value) for the value-comparable atomic scalars.
HASHABLE_SCALARS = (
    [(case[0], 42, 7) for case in INTEGERS]
    + [(case[0], 1.5, 2.5) for case in FLOATS]  # halves are exact in every float width
    + [
        (scalar.BinaryScalar, b"hi", b"bye"),
        (scalar.Utf8Scalar, "hi", "bye"),
    ]
)


@pytest.mark.parametrize("scalar_class,a,b", HASHABLE_SCALARS)
def test_scalar_is_hashable_and_value_comparable(scalar_class, a, b):
    first, same, other = scalar_class(a), scalar_class(a), scalar_class(b)

    # (a) equal values compare equal, hash equal, and collapse to one set entry.
    assert first == same
    assert hash(first) == hash(same)
    assert len({first, same}) == 1

    # (b) a different value is unequal and (almost surely) a separate set entry.
    assert first != other
    assert len({first, same, other}) == 2

    # (c) a scalar works as a dict key.
    lookup = {first: "one", other: "two"}
    assert lookup[same] == "one"
    assert lookup[other] == "two"

    # A foreign type never compares equal and never raises.
    assert (first == a) is False
    assert first != scalar_class.null()

    # (d) null scalars of the same class are hashable and equal to each other.
    assert scalar_class.null() == scalar_class.null()
    assert hash(scalar_class.null()) == hash(scalar_class.null())
    assert len({scalar_class.null(), scalar_class.null()}) == 1


def test_null_scalar_is_hashable_and_equal():
    # (d) every NullScalar equals every other and shares one set/dict slot.
    a, b = scalar.NullScalar(), scalar.NullScalar()
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b}) == 1
    assert {a: "null"}[b] == "null"


def test_float_zero_and_nan_hashing_caveats():
    # (e) 0.0 and -0.0 are equal and hash equal, collapsing in a set.
    assert scalar.Float64Scalar(0.0) == scalar.Float64Scalar(-0.0)
    assert hash(scalar.Float64Scalar(0.0)) == hash(scalar.Float64Scalar(-0.0))
    assert len({scalar.Float64Scalar(0.0), scalar.Float64Scalar(-0.0)}) == 1
    # NaN is never equal to itself (the documented caveat), yet hashing it never raises.
    nan1, nan2 = scalar.Float64Scalar(float("nan")), scalar.Float64Scalar(float("nan"))
    assert nan1 != nan2
    hash(nan1)  # does not raise


# (serie class, values, different values) for the value-comparable series.
HASHABLE_SERIES = [(case[0], [1, 2, 3], [4, 5, 6]) for case in SERIES] + [
    (case[0], [1.5, 2.5], [3.5, 4.5]) for case in FLOAT_SERIES
]


@pytest.mark.parametrize("serie_class,a,b", HASHABLE_SERIES)
def test_serie_is_hashable_and_value_comparable(serie_class, a, b):
    first, same, other = serie_class(a), serie_class(a), serie_class(b)

    # (a) equal series compare equal, hash equal, and collapse to one set entry.
    assert first == same
    assert hash(first) == hash(same)
    assert len({first, same}) == 1

    # (b) a different serie is unequal and (almost surely) a separate set entry.
    assert first != other
    assert len({first, same, other}) == 2

    # (c) a serie works as a dict key.
    assert {first: "a", other: "b"}[same] == "a"

    # (d) null series of the same class are hashable and equal to each other.
    assert serie_class.null() == serie_class.null()
    assert hash(serie_class.null()) == hash(serie_class.null())
    assert len({serie_class.null(), serie_class.null()}) == 1
