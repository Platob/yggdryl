"""Native Value and zero-copy Arrow boundary coverage."""

from __future__ import annotations

import copy
import datetime as dt
import pickle
import sys
from dataclasses import dataclass
from decimal import Decimal
from typing import NamedTuple

import pyarrow as pa
import pytest

from yggdryl import Field, Value


@dataclass
class Quote:
    symbol: str
    price: float


@dataclass
class Venue:
    id: int
    name: str | None


class Point(NamedTuple):
    y: int
    x: int


class SlottedPoint:
    __slots__ = ("y", "x")

    def __init__(self, y: int, x: int) -> None:
        self.y = y
        self.x = x


class MixedPoint:
    __slots__ = ("x", "__dict__")

    def __init__(self, y: int, x: int) -> None:
        self.y = y
        self.x = x


def test_python_records_are_distinct_from_arbitrary_mappings() -> None:
    assert Value.from_python(Quote("AAPL", 12.5)).kind == "record"
    assert Value.from_python(Point(2, 1)).kind == "record"
    assert Value.from_python(SlottedPoint(2, 1)).kind == "record"
    assert Value.from_python(MixedPoint(2, 1)).kind == "record"
    assert Value.from_python({"symbol": "AAPL"}).kind == "mapping"
    assert Value.from_python(Quote("AAPL", 12.5)).into_python() == {
        "price": 12.5,
        "symbol": "AAPL",
    }
    assert Value.from_python(SlottedPoint(2, 1)).into_python() == {"x": 1, "y": 2}
    assert Value.from_python(MixedPoint(2, 1)).into_python() == {"x": 1, "y": 2}


def test_precise_factories_keep_width_scale_unit_and_zone() -> None:
    assert Value.f16(1.5).kind == "f16"
    assert Value.f32(1.5).kind == "f32"
    assert Value.f64(1.5).kind == "f64"
    assert Value.d128(150, 2).into_python() == Decimal("1.50")

    wide = "12345678901234567890123456789012345678901234567890"
    d256 = Value.d256(wide, 4)
    assert d256.kind == "d256"
    assert d256.unscaled == int(wide)
    assert d256.scale == 4
    assert d256.count is None
    assert d256.unit is None
    assert d256.zone is None
    assert d256.into_python() == Decimal(f"{wide}E-4")
    assert Value.from_python(Decimal(wide)).kind == "d256"

    assert Value.date32(1).kind == "date32"
    assert Value.date64(86_400_000).kind == "date64"
    assert Value.time32(1, "s").kind == "time32"
    assert Value.time64(1, "us").into_python() == dt.time(microsecond=1)
    with pytest.raises(ValueError, match="timezone"):
        Value.time64(1, "us", "UTC")
    instant = Value.datetime64(0, "us", "UTC")
    assert instant.count == 0
    assert instant.unit == "us"
    assert instant.zone == "UTC"
    assert instant.unscaled is None
    assert instant.scale is None
    assert instant.into_python() == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
    assert Value.duration32(1, "ms").kind == "duration32"
    assert Value.duration64(1, "us").into_python() == dt.timedelta(microseconds=1)
    with pytest.raises(ValueError, match="NAIVE"):
        Value.duration64(1, "us", "UTC")


def test_value_is_hashable_and_has_typed_byte_accessors() -> None:
    assert Value.f32(1.0) == Value.f64(1.0)
    assert hash(Value.f32(1.0)) == hash(Value.f64(1.0))
    assert Value.from_python("text").as_utf8() == "text"
    assert Value.from_python("text").as_bytes() is None
    assert Value.from_python(b"bytes").as_bytes() == b"bytes"
    assert Value.from_python(b"bytes").as_utf8() is None
    value = Value.from_python({"answer": 42})
    assert value.as_json_bytes() == b'{"answer":42}'
    assert value.as_json_utf8() == '{"answer":42}'


def test_unsigned_stable_hash_maps_to_python_hash_without_overflow() -> None:
    value = next(
        Value.from_python(index)
        for index in range(10_000)
        if Value.from_python(index).stable_hash() > 2**63 - 1
    )
    stable = value.stable_hash()
    if sys.hash_info.width == 64:
        expected = stable if stable < 2**63 else stable - 2**64
    else:
        folded = (stable ^ (stable >> 32)) & 0xFFFF_FFFF
        expected = folded if folded < 2**31 else folded - 2**32
    if expected == -1:
        expected = -2
    assert hash(value) == expected
    assert hash(value) == hash(copy.copy(value))


def test_checked_arithmetic_accepts_native_values_and_python_operands() -> None:
    value = Value.from_python(8)

    assert value.add(2).into_python() == 10
    assert value.subtract(2).into_python() == 6
    assert value.multiply(2).into_python() == 16
    assert value.divide(2).into_python() == 4
    assert value.remainder(3).into_python() == 2
    assert value.negate().into_python() == -8
    assert Value.from_python(-8).absolute().into_python() == 8

    assert (value + 2).into_python() == 10
    assert (2 + value).into_python() == 10
    assert (value - 2).into_python() == 6
    assert (10 - value).into_python() == 2
    assert (value * 2).into_python() == 16
    assert (2 * value).into_python() == 16
    assert (value / 2).into_python() == 4
    assert (16 / value).into_python() == 2
    assert (value % 3).into_python() == 2
    assert (10 % value).into_python() == 2
    assert (-value).into_python() == -8
    assert abs(Value.from_python(-8)).into_python() == 8

    assert (Value.f16(1.5) + Value.f32(0.5)).kind == "f32"
    assert (Value.d128(105, 2) + Decimal("0.20")).into_python() == Decimal("1.25")
    assert Value.d128(1).divide(Value.d128(2)) == Value.d128(5, 1)
    assert Value.d128(1).divide(Value.d128(128)) == Value.d128(78_125, 7)


def test_checked_arithmetic_preserves_python_error_categories() -> None:
    with pytest.raises(TypeError, match="invalid addition"):
        _ = Value.from_python("a") + "b"
    with pytest.raises(OverflowError, match="overflows"):
        _ = Value.from_python(2**63 - 1) + 1
    with pytest.raises(ZeroDivisionError, match="by zero"):
        _ = Value.from_python(1) / 0
    with pytest.raises(ArithmeticError, match="no exact"):
        _ = Value.d128(1) / Value.d128(3)


def test_native_value_traversal_keeps_exact_children() -> None:
    instant = Value.datetime64(1, "ns", "UTC")
    tree = Value.from_python(
        {"instant": instant, "legs": [{"price": Value.f32(12.5)}, None]}
    )

    assert len(tree) == 2
    assert not tree.is_empty()
    assert isinstance(tree["instant"], Value)
    assert tree["instant"].kind == "datetime64"
    assert (tree["instant"].count, tree["instant"].unit, tree["instant"].zone) == (
        1,
        "ns",
        "UTC",
    )
    assert tree.path("legs.0.price") is not None
    assert tree.path("legs.0.price").kind == "f32"  # type: ignore[union-attr]
    assert tree.path("legs.9.price") is None
    assert tree.get("missing") is None
    assert tree.has("legs") and "legs" in tree

    sequence = tree["legs"]
    assert sequence.at(-1) is not None and sequence.at(-1).kind == "null"
    assert sequence[-2].path("price") is not None
    assert [child.kind for child in sequence] == ["mapping", "null"]
    with pytest.raises(IndexError):
        _ = sequence[9]


def test_native_value_mapping_and_record_updates_are_persistent() -> None:
    mapping = Value.from_python({"symbol": "AAPL", "venue": None})
    updated = mapping.set("venue", "XNAS").set("price", Value.f32(12.5))
    removed = updated.remove("symbol")

    assert mapping["venue"].kind == "null"
    assert updated["venue"].as_utf8() == "XNAS"
    assert updated["price"].kind == "f32"
    assert removed.get("symbol") is None
    assert [key.into_python() for key in updated.keys()] == [
        "symbol",
        "venue",
        "price",
    ]
    assert [value.kind for value in updated.values()] == ["string", "string", "f32"]
    assert [key.into_python() for key, _ in updated.items()] == [
        "symbol",
        "venue",
        "price",
    ]

    record = Value.from_python(Quote("AAPL", 12.5))
    moved = record.set("symbol", "MSFT").remove("price")
    assert record["symbol"].as_utf8() == "AAPL"
    assert moved["symbol"].as_utf8() == "MSFT"
    assert moved.get("price") is None
    assert [key.into_python() for key in moved.keys()] == ["symbol"]
    assert [child.kind for child in record] == ["f64", "string"]
    with pytest.raises(TypeError, match="record keys"):
        record.set(0, "invalid")
    with pytest.raises(TypeError, match="remove"):
        mapping.remove(0)  # type: ignore[arg-type]


def test_repr_remains_total_when_python_temporal_projection_would_be_lossy() -> None:
    nanosecond = Value.datetime64(1, "ns", "UTC")
    with pytest.raises(ValueError, match="microsecond"):
        nanosecond.into_python()
    assert eval(repr(nanosecond), {"Value": Value}) == nanosecond

    outside_python_date = Value.date64(2**63 - 1)
    with pytest.raises((OverflowError, ValueError)):
        outside_python_date.into_python()
    assert "date64" in repr(outside_python_date)


def test_exact_repr_and_pickle_preserve_every_native_value_variant() -> None:
    scalar_states: list[tuple[object, ...]] = [
        ("null",),
        ("bool", True),
        ("i8", -(2**7)),
        ("i16", -(2**15)),
        ("i32", -(2**31)),
        ("i64", -(2**63)),
        ("u8", 2**8 - 1),
        ("u16", 2**16 - 1),
        ("u32", 2**32 - 1),
        ("u64", 2**64 - 1),
        ("i128", -(2**127)),
        ("u128", 2**128 - 1),
        ("f16", 0x8000),  # negative zero
        ("f32", 0x8000_0000),
        ("f64", 0x8000_0000_0000_0000),
        ("d128", ("-170141183460469231731687303715884105728", -7)),
        (
            "d256",
            (
                "123456789012345678901234567890123456789012345678901234567890",
                18,
            ),
        ),
        ("string", "naïve"),
        ("bytes", b"\x00\xff"),
        ("geospatial", b"\x01\x01\x00\x00\x00"),
        ("date32", (1, "d", "NAIVE")),
        ("date64", (86_400_000, "ms", "NAIVE")),
        ("time32", (1, "s", "NAIVE")),
        ("time64", (1, "us", "NAIVE")),
        ("datetime64", (1, "ns", "UTC")),
        ("duration32", (1, "ms", "NAIVE")),
        ("duration64", (1, "ns", "NAIVE")),
    ]
    record_state = (
        "record",
        (
            ("amount", scalar_states[16]),
            ("when", scalar_states[25]),
        ),
    )
    mapping_state = (
        "mapping",
        (
            (("string", "row"), record_state),
            (("i16", 7), ("sequence", (("f32", 0x3FC0_0000), ("null",)))),
        ),
    )
    states = [*scalar_states, record_state, mapping_state]

    for state in states:
        value = Value._from_pickle(state)
        restored = pickle.loads(pickle.dumps(value))
        represented = eval(repr(value), {"Value": Value})
        assert restored == value and restored.kind == value.kind
        assert represented == value and represented.kind == value.kind
        assert restored.stable_hash() == value.stable_hash()
        assert copy.copy(value) == value
        assert copy.deepcopy(value) == value

    assert Value._from_pickle(record_state).kind == "record"
    assert Value._from_pickle(mapping_state).kind == "mapping"
    with pytest.raises(ValueError, match="unknown"):
        Value._from_pickle(("future", None))


@pytest.mark.parametrize(
    ("scalar", "kind"),
    [
        (pa.scalar(1.5, type=pa.float16()), "f16"),
        (pa.scalar(1.5, type=pa.float32()), "f32"),
        (pa.scalar(1.5, type=pa.float64()), "f64"),
        (pa.scalar(dt.date(2026, 8, 23), type=pa.date32()), "date32"),
        (pa.scalar(dt.date(2026, 8, 23), type=pa.date64()), "date64"),
        (pa.scalar(dt.time(1, 2), type=pa.time32("ms")), "time32"),
        (pa.scalar(dt.time(1, 2), type=pa.time64("us")), "time64"),
        (
            pa.scalar(
                dt.datetime(2026, 8, 23, tzinfo=dt.timezone.utc),
                type=pa.timestamp("ns", tz="UTC"),
            ),
            "datetime64",
        ),
        (pa.scalar(dt.timedelta(microseconds=7), type=pa.duration("us")), "duration64"),
    ],
)
def test_arrow_scalar_round_trip_keeps_physical_type(
    scalar: pa.Scalar, kind: str
) -> None:
    value = Value.from_arrow_scalar(scalar)
    restored = value.into_arrow_scalar()
    assert value.kind == kind
    assert restored.type == scalar.type
    assert restored == scalar


def test_arrow_decimal256_scalar_round_trip() -> None:
    scalar = pa.scalar(
        Decimal("1234567890123456789012345678901234567890.12"),
        pa.decimal256(50, 2),
    )
    value = Value.from_arrow_scalar(scalar)
    assert value.kind == "d256"
    # A Value retains the decimal width, coefficient, and scale, while a
    # declared Field retains spare precision that is not part of a value.
    assert value.into_arrow_scalar().type == pa.decimal256(42, 2)
    field = Field.from_arrow(pa.field("value", scalar.type))
    assert value.into_arrow_scalar(field).type == scalar.type
    assert value.into_arrow_scalar().as_py() == scalar.as_py()


def test_arrow_array_uses_c_data_and_requires_a_field_only_when_ambiguous() -> None:
    array = pa.array([1, None, 3], type=pa.int16())
    value = Value.from_arrow_array(array)
    restored = value.into_arrow_array()
    assert restored.type == array.type
    assert restored.to_pylist() == array.to_pylist()

    empty = Value.from_arrow_array(pa.array([], type=pa.int16()))
    with pytest.raises(ValueError, match="empty Sequence"):
        empty.into_arrow_array()
    restored_empty = empty.into_arrow_array(Field("item", "int16"))
    assert restored_empty.type == pa.int16() and len(restored_empty) == 0


def test_record_batch_and_table_round_trip_through_native_rows() -> None:
    batch = pa.record_batch(
        [pa.array([1, 2], type=pa.int32()), pa.array(["A", "B"])],
        names=["id", "symbol"],
    )
    field = Field.from_arrow_schema(batch.schema)
    rows = Value.from_arrow_record_batch(batch)
    assert rows.into_python() == [[1, "A"], [2, "B"]]
    restored_batch = rows.into_arrow_record_batch(field)
    assert restored_batch.equals(batch)

    table = pa.Table.from_batches([batch, batch])
    table_rows = Value.from_arrow_table(table)
    restored_table = table_rows.into_arrow_table(field)
    assert restored_table.equals(table.combine_chunks())


def test_record_rows_infer_struct_field_names() -> None:
    rows = Value.from_python([Quote("AAPL", 12.5), Quote("MSFT", 9.0)])
    batch = rows.into_arrow_record_batch()
    assert batch.schema.names == ["price", "symbol"]
    assert batch.to_pylist() == [
        {"price": 12.5, "symbol": "AAPL"},
        {"price": 9.0, "symbol": "MSFT"},
    ]

    nullable = Value.from_python([Venue(1, None), Venue(2, "XNAS")])
    nullable_batch = nullable.into_arrow_record_batch()
    assert nullable_batch.schema.field("name").nullable
    assert nullable_batch.column("name").to_pylist() == [None, "XNAS"]


def test_value_field_accessors_redirect_to_core_inference() -> None:
    scalar = Value.from_python(42).into_field()
    assert scalar.name == "value"
    assert str(scalar.data_type) == "int64"
    assert not scalar.nullable

    item = Value.from_python([1, None]).into_array_field()
    assert item.name == "item"
    assert str(item.data_type) == "int64"
    assert item.nullable

    root = Value.from_python([Venue(1, None), Venue(2, "XNAS")]).into_struct_field()
    assert root.name == "row"
    assert not root.nullable
    children = list(root.data_type)
    assert [child.name for child in children] == ["id", "name"]
    assert children[1].nullable

    with pytest.raises(ValueError, match="empty Sequence"):
        Value.from_python([]).into_array_field()
    with pytest.raises(ValueError, match="field names"):
        Value.from_python([[1]]).into_struct_field()


def test_empty_rows_require_the_known_arrow_root_on_output() -> None:
    batch = pa.record_batch([pa.array([], type=pa.int32())], names=["id"])
    rows = Value.from_arrow_record_batch(batch)
    with pytest.raises(ValueError, match="empty rows"):
        rows.into_arrow_record_batch()
    assert rows.into_arrow_record_batch(Field.from_arrow_schema(batch.schema)).equals(batch)
