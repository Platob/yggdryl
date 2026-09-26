"""Native ``Scalar``: the object conversion pair and the Arrow boundary."""

from __future__ import annotations

import copy
import datetime as dt
import math
import pickle
import struct
import sys
import zoneinfo
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, NamedTuple

import numpy as np
import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, Serie, json

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
    assert Scalar.from_(Quote("AAPL", 12.5)).kind == "struct"
    assert Scalar.from_(Point(2, 1)).kind == "struct"
    assert Scalar.from_(SlottedPoint(2, 1)).kind == "struct"
    assert Scalar.from_(MixedPoint(2, 1)).kind == "struct"
    assert Scalar.from_({"symbol": "AAPL"}).kind == "map"
    assert Scalar.from_(Quote("AAPL", 12.5)).as_py() == {
        "price": 12.5,
        "symbol": "AAPL",
    }
    assert Scalar.from_(SlottedPoint(2, 1)).as_py() == {"x": 1, "y": 2}
    assert Scalar.from_(MixedPoint(2, 1)).as_py() == {"x": 1, "y": 2}


def test_native_field_and_datatype_wrappers_cross_structurally() -> None:
    field = Field("items", "serie<int32>", nullable=False)
    dtype = Scalar.from_(field.dtype)
    field_value = Scalar.from_(field)

    assert dtype.kind == "map"
    assert dtype.as_py()["type"] == "serie"  # type: ignore[index]
    assert field_value.kind == "map"
    assert field_value.as_py()["name"] == "items"  # type: ignore[index]


def test_the_type_side_names_the_width_unit_scale_and_zone() -> None:
    # The width is named on the type, not chosen by a family factory: the six
    # `Scalar.float`/`date`/`time`/`datetime` factories are gone, and every
    # value they made is reachable here - plus `decimal32`/`decimal64`, which
    # no factory could ever produce.
    assert DataType("float16").scalar(1.5).kind == "f16"
    assert DataType("float32").scalar(1.5).kind == "f32"
    assert DataType("float64").scalar(1.5).kind == "f64"
    assert DataType("decimal32(9,2)").scalar(Decimal("1.50")).kind == "d32"
    assert DataType("decimal64(18,2)").scalar(Decimal("1.50")).kind == "d64"
    assert Scalar.decimal(150, 2).as_py() == Decimal("1.50")

    wide = "12345678901234567890123456789012345678901234567890"
    d256 = Scalar.decimal(wide, 4)
    assert d256.kind == "d256"
    assert d256.unscaled == int(wide)
    assert d256.scale == 4
    assert d256.count is None
    assert d256.unit is None
    assert d256.zone is None
    assert d256.as_py() == Decimal(f"{wide}E-4")
    assert Scalar.from_(Decimal(wide)).kind == "d256"

    assert DataType("date32").scalar(1).kind == "date32"
    assert DataType("date64").scalar(86_400_000).kind == "date64"
    assert DataType("time32(s)").scalar(1).kind == "time32"
    assert DataType("time64(us)").scalar(1).as_py() == dt.time(microsecond=1)
    # A time of day has no zone at all, so the type has nowhere to spell one -
    # the parser refuses a second parameter rather than a value refusing a zone.
    with pytest.raises(ValueError, match="expected"):
        DataType('time64(us,"UTC")')
    instant = DataType('datetime64(us,"UTC")').scalar(0)
    assert instant.count == 0
    assert instant.unit == "us"
    assert instant.zone == "UTC"
    assert instant.unscaled is None
    assert instant.scale is None
    assert instant.as_py() == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
    assert Scalar.duration(1, "ms").kind == "duration32"
    assert Scalar.duration(1, "us").as_py() == dt.timedelta(microseconds=1)
    assert Scalar.duration(2**31, "us").kind == "duration64"
    # An elapsed duration has no zone, so there is no parameter to pass one
    # that could only be refused - the type has nowhere to spell one either.
    with pytest.raises(ValueError, match="expected"):
        DataType('duration32(us,"UTC")')
    # A width that does not exist is refused by the type, where widths live.
    with pytest.raises(ValueError):
        DataType("float8")


def test_scalar_identity_accessors_name_the_exact_leaf_and_family() -> None:
    values = [
        (Scalar.from_(None), "null", "null"),
        (Scalar.from_(True), "boolean", "boolean"),
        (Scalar.from_(1), "int64", "integer"),
        (DataType("float32").scalar(1.5), "float32", "floating"),
        (Scalar.decimal(150, 2), "decimal128", "decimal"),
        (DataType("date32").scalar(1), "date32", "temporal"),
        (Scalar.from_("AAPL"), "utf8", "text"),
        (
            json.loads(
                '"USD"', field=Field("value", "ccy", False), cls=Scalar
            ),
            "ccy",
            "code",
        ),
        (
            json.loads(
                '"00112233-4455-6677-8899-aabbccddeeff"',
                field=Field("value", "uuid", False),
                cls=Scalar,
            ),
            "uuid",
            "uuid",
        ),
        (
            json.loads('"5.0.1"', field=Field("value", "version", False), cls=Scalar),
            "version",
            "text",
        ),
        (Scalar.from_(b"bytes"), "binary", "bytes"),
        (Scalar.from_({"id": 1}), "map", "nested"),
    ]
    for value, expected_id, expected_family in values:
        assert value.id == expected_id
        assert value.family == expected_family
        assert value.id == value.dtype.id
        assert value.family == value.dtype.kind


def test_exact_intervals_retain_their_flat_python_layouts() -> None:
    def typed(document: str, dtype: str) -> Scalar:
        return json.loads(
            document,
            field=Field("span", dtype, False),
            cls=Scalar,
        )

    assert typed("12", "interval(year_month)").as_py() == 12
    assert typed("[2,3]", "interval(day_time)").as_py() == [2, 3]
    assert typed("[1,2,3]", "interval(month_day_nano)").as_py() == [1, 2, 3]


def test_exact_width_factories_are_private_reconstruction_details() -> None:
    for name in (
        "f16",
        "f32",
        "f64",
        "d128",
        "d256",
        "date32",
        "date64",
        "time32",
        "time64",
        "datetime64",
        "duration32",
        "duration64",
    ):
        assert not hasattr(Scalar, name)


def test_enumeration_reads_a_member_as_the_text_a_column_holds() -> None:
    # An enum member's datatype is `string`, so the value is its canonical
    # name and the vocabulary is what `from_enum` validates against, not
    # something the value carries.
    value = Scalar.from_enum("IOMode", "append")

    assert value.kind == "string"
    assert value.as_py() == "append"
    assert value.as_str() == "append"
    assert value == Scalar.from_("append")
    assert hash(value) == hash(copy.copy(value))
    assert pickle.loads(pickle.dumps(value)) == value
    with pytest.raises(ValueError, match="unknown"):
        Scalar.from_enum("IOMode", "missing")


def test_value_is_hashable_and_has_typed_byte_accessors() -> None:
    assert DataType("float32").scalar(1.0) == DataType("float64").scalar(1.0)
    assert hash(DataType("float32").scalar(1.0)) == hash(DataType("float64").scalar(1.0))
    assert Scalar.from_("text").as_str() == "text"
    assert Scalar.from_("text").as_bytes() is None
    assert Scalar.from_(b"bytes").as_bytes() == b"bytes"
    assert Scalar.from_(b"bytes").as_str() is None
    value = Scalar.from_({"answer": 42})
    assert value.into_json_bytes() == b'{"answer":42}'
    assert value.into_json() == '{"answer":42}'


def test_unsigned_stable_hash_maps_to_python_hash_without_overflow() -> None:
    value = next(
        Scalar.from_(index)
        for index in range(10_000)
        if Scalar.from_(index).stable_hash() > 2**63 - 1
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


def test_checked_arithmetic_accepts_native_scalars_and_python_operands() -> None:
    value = Scalar.from_(8)

    assert value.add(2).as_py() == 10
    assert value.subtract(2).as_py() == 6
    assert value.multiply(2).as_py() == 16
    assert value.divide(2).as_py() == 4
    assert value.remainder(3).as_py() == 2
    assert value.negate().as_py() == -8
    assert Scalar.from_(-8).absolute().as_py() == 8

    assert (value + 2).as_py() == 10
    assert (2 + value).as_py() == 10
    assert (value - 2).as_py() == 6
    assert (10 - value).as_py() == 2
    assert (value * 2).as_py() == 16
    assert (2 * value).as_py() == 16
    assert (value / 2).as_py() == 4
    assert (16 / value).as_py() == 2
    assert (value % 3).as_py() == 2
    assert (10 % value).as_py() == 2
    assert (-value).as_py() == -8
    assert abs(Scalar.from_(-8)).as_py() == 8

    assert (DataType("float16").scalar(1.5) + DataType("float32").scalar(0.5)).kind == "f32"
    assert (Scalar.decimal(105, 2) + Decimal("0.20")).as_py() == Decimal("1.25")
    assert Scalar.decimal(1).divide(Scalar.decimal(2)) == Scalar.decimal(5, 1)
    assert Scalar.decimal(1).divide(Scalar.decimal(128)) == Scalar.decimal(78_125, 7)


def test_addition_joins_text_bytes_and_sequences() -> None:
    # Text, bytes and sequences have no sum, so `+` joins them - through the
    # Python operator too, not just the named method.
    assert (Scalar.from_("AA") + "PL").as_py() == "AAPL"
    assert (Scalar.from_(b"\x01") + b"\x02").as_py() == b"\x01\x02"
    assert (Scalar.from_([1]) + [2]).as_py() == [1, 2]


def test_checked_arithmetic_preserves_python_error_categories() -> None:
    # Two repertoires still do not join, and only `+` joins at all.
    with pytest.raises(TypeError, match="invalid addition"):
        _ = Scalar.from_("a") + 1
    with pytest.raises(TypeError, match="invalid subtraction"):
        _ = Scalar.from_("a") - "b"
    with pytest.raises(OverflowError, match="overflows"):
        _ = Scalar.from_(2**63 - 1) + 1
    with pytest.raises(ZeroDivisionError, match="by zero"):
        _ = Scalar.from_(1) / 0
    with pytest.raises(ArithmeticError, match="no exact"):
        _ = Scalar.decimal(1) / Scalar.decimal(3)


def test_the_fixed_decimal_leaves_keep_an_exact_remainder_and_refuse_a_zero_divisor() -> None:
    price = DataType("decimal")
    wide = DataType("bigdecimal")
    # `%` is exact at scale eighteen, its sign the dividend's, as Python's
    # own Decimal answers it.
    rest = price.scalar(Decimal("7.5")) % price.scalar(2)
    assert rest.kind == "decimal" and rest.as_py() == Decimal("1.5")
    assert (price.scalar(Decimal("-7.5")) % 2).as_py() == Decimal("-7.5") % 2
    assert price.scalar(Decimal("7.5")).remainder(Decimal("-2")).as_py() == Decimal("1.5")
    # A bigdecimal on either side answers one.
    widened = wide.scalar(Decimal("7.5")) % price.scalar(2)
    assert widened.kind == "bigdecimal" and widened.as_py() == Decimal("1.5")
    # A divisor of nothing is a division by zero - ZeroDivisionError, as for
    # every other exact value, where it used to be a TypeError.
    for dividend in (price.scalar(1), wide.scalar(1)):
        with pytest.raises(ZeroDivisionError, match="by zero"):
            _ = dividend / price.scalar(0)
        with pytest.raises(ZeroDivisionError, match="by zero"):
            _ = dividend % price.scalar(0)
        with pytest.raises(ZeroDivisionError, match="by zero"):
            _ = dividend % 0


def test_native_scalar_traversal_keeps_exact_children() -> None:
    instant = DataType('datetime64(ns,"UTC")').scalar(1)
    tree = Scalar.from_(
        {"instant": instant, "legs": [{"price": DataType("float32").scalar(12.5)}, None]}
    )

    assert len(tree) == 2
    assert not tree.is_empty()
    assert isinstance(tree["instant"], Scalar)
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
    assert [child.kind for child in sequence] == ["map", "null"]
    with pytest.raises(IndexError):
        _ = sequence[9]


def test_native_scalar_mapping_and_record_updates_are_persistent() -> None:
    mapping = Scalar.from_({"symbol": "AAPL", "venue": None})
    updated = mapping.set("venue", "XNAS").set("price", DataType("float32").scalar(12.5))
    removed = updated.remove("symbol")

    assert mapping["venue"].kind == "null"
    assert updated["venue"].as_str() == "XNAS"
    assert updated["price"].kind == "f32"
    assert removed.get("symbol") is None
    assert [key.as_py() for key in updated.keys()] == [
        "symbol",
        "venue",
        "price",
    ]
    assert [value.kind for value in updated.values()] == ["string", "string", "f32"]
    assert [key.as_py() for key, _ in updated.items()] == [
        "symbol",
        "venue",
        "price",
    ]

    record = Scalar.from_(Quote("AAPL", 12.5))
    moved = record.set("symbol", "MSFT").remove("price")
    assert record["symbol"].as_str() == "AAPL"
    assert moved["symbol"].as_str() == "MSFT"
    assert moved.get("price") is None
    assert [key.as_py() for key in moved.keys()] == ["symbol"]
    assert [child.kind for child in record] == ["f64", "string"]
    with pytest.raises(TypeError, match="struct keys"):
        record.set(0, "invalid")
    with pytest.raises(TypeError, match="remove"):
        mapping.remove(0)  # type: ignore[arg-type]


def test_repr_remains_total_when_python_temporal_projection_would_be_lossy() -> None:
    # `as_py` floors to the microsecond `datetime` holds, so the projection is
    # lossy; `repr` is what stays exact, and round-trips the whole value.
    nanosecond = DataType('datetime64(ns,"UTC")').scalar(1)
    assert nanosecond.as_py() == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
    assert eval(repr(nanosecond), {"Scalar": Scalar}) == nanosecond

    # The largest whole-day millisecond an i64 holds. `Scalar.date` took any
    # count and only failed when projected; the type checks the value is a
    # whole day up front, so the subject has to be a legal one.
    outside_python_date = DataType("date64").scalar(((2**63 - 1) // 86_400_000) * 86_400_000)
    with pytest.raises((OverflowError, ValueError)):
        outside_python_date.as_py()
    assert "date64" in repr(outside_python_date)


def test_exact_repr_and_pickle_preserve_every_native_scalar_variant() -> None:
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
        # Any leaf but plain `utf8` pickles its name beside the text, and a
        # fixed or sized leaf its number; the name says the charset.
        ("string", ("fixed_ascii", 4, "USD")),
        ("string", ("large_cp1252_view", None, "café")),
        ("ccy", "USD"),
        ("side", "BUY"),
        ("version", "5.0.1"),
        ("bytes", b"\x00\xff"),
        ("bytes", ("fixed_size_binary", 4, b"\x00\x01\x02\x03")),
        ("bytes", ("binary_view", None, b"\xff" * 40)),
        ("geospatial", b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 0.0, 0.0)),
        ("date32", (1, "d", "NAIVE")),
        ("date64", (86_400_000, "ms", "NAIVE")),
        ("time32", (1, "s", "NAIVE")),
        ("time64", (1, "us", "NAIVE")),
        ("datetime64", (1, "ns", "UTC")),
        ("duration32", (1, "ms", "NAIVE")),
        ("duration64", (1, "ns", "NAIVE")),
        # A value keeps its column's maximum, so a sized leaf pickles it as a
        # fixed leaf does its width. Last, so the references below keep
        # naming what they name.
        ("string", ("sized_utf8", 32, "abc")),
        ("bytes", ("sized_binary", 16, b"\x01\x02")),
    ]
    # Every registered code, because "every native scalar variant" is what this
    # test claims: `state` and `timeinforce` used to raise "unsupported Scalar
    # representation in pickle state" here, being the two the pickle listing
    # had been left out of. Kept after the list above so the positional
    # references into it below stay pinned to what they name.
    code_states: list[tuple[object, ...]] = [
        ("country", "FR"),
        ("ccy", "USD"),
        ("mic", "XPAR"),
        ("cfi", "ESVUFR"),
        ("isin", "US0378331005"),
        ("side", "BUY"),
        ("state", "20NEW"),
        ("timeinforce", "GTC"),
        ("cusip", "037833100"),
        ("sedol", "B0YBKJ7"),
    ]
    record_state = (
        "struct",
        (
            ("amount", scalar_states[16]),
            ("when", scalar_states[31]),
        ),
    )
    mapping_state = (
        "map",
        (
            (("string", "row"), record_state),
            (("i16", 7), ("serie", (("f32", 0x3FC0_0000), ("null",)))),
        ),
    )
    states = [*scalar_states, *code_states, record_state, mapping_state]

    for state in states:
        value = Scalar._from_pickle(state)
        restored = pickle.loads(pickle.dumps(value))
        represented = eval(repr(value), {"Scalar": Scalar})
        assert restored == value and restored.kind == value.kind
        assert represented == value and represented.kind == value.kind
        assert restored.stable_hash() == value.stable_hash()
        assert copy.copy(value) == value
        assert copy.deepcopy(value) == value

    assert Scalar._from_pickle(record_state).kind == "struct"
    assert Scalar._from_pickle(mapping_state).kind == "map"
    with pytest.raises(ValueError, match="unknown"):
        Scalar._from_pickle(("future", None))
    with pytest.raises(ValueError, match="unknown"):
        Scalar._from_pickle(("currency", "USD"))


@pytest.mark.parametrize(
    ("legacy", "kind"),
    [
        ("list", "serie"),
        ("list_view", "serie_view"),
        ("fixed_size_list", "fixed_size_serie"),
        ("large_list", "large_serie"),
        ("large_list_view", "large_serie_view"),
    ],
)
def test_a_sequence_state_written_under_its_list_tag_still_loads(legacy: str, kind: str) -> None:
    # A repr written before the serie rename tags a sequence with the Arrow
    # list word its layout was spelled by; the same state loads as that
    # layout, and represents itself again under the serie name.
    items = (("i64", 1), ("null",), ("i64", 3))
    value = Scalar._from_pickle((legacy, items))

    assert value.kind == kind
    assert value == Scalar._from_pickle((kind, items))
    assert value.as_py() == [1, None, 3]
    assert repr(value).startswith(f"Scalar._from_pickle(('{kind}',")
    represented = eval(repr(value), {"Scalar": Scalar})
    assert represented == value and represented.kind == kind


@pytest.mark.parametrize(
    ("scalar", "kind"),
    [
        (pa.scalar(np.float16(1.5), type=pa.float16()), "f16"),
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
    # One Arrow scalar is its row, read under its own type.
    value = Scalar.from_(scalar)
    restored = value.into_arrow_scalar()
    assert value.kind == kind
    assert restored.type == scalar.type
    assert restored == scalar


def test_arrow_decimal256_scalar_round_trip() -> None:
    scalar = pa.scalar(
        Decimal("1234567890123456789012345678901234567890.12"),
        pa.decimal256(50, 2),
    )
    value = Scalar.from_(scalar)
    assert value.kind == "d256"
    # A Scalar retains the decimal width, coefficient, and scale, while a
    # declared Field retains spare precision that is not part of a value.
    assert value.into_arrow_scalar().type == pa.decimal256(42, 2)
    field = Field.from_arrow(pa.field("value", scalar.type))
    assert value.into_arrow_scalar(field).type == scalar.type
    assert value.into_arrow_scalar().as_py() == scalar.as_py()


def test_arrow_array_uses_c_data_and_requires_a_field_only_when_ambiguous() -> None:
    array = pa.array([1, None, 3], type=pa.int16())
    # A column is held as a serie sharing its buffers.
    value = Scalar.from_(array)
    assert value.kind == "serie"
    assert value.as_py() == [1, None, 3]
    restored = value.into_arrow_array()
    assert restored.type == array.type
    assert restored.to_pylist() == array.to_pylist()

    empty = Scalar.from_(pa.array([], type=pa.int16()))
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
    rows = Scalar.from_(batch)
    # A record column's rows read under its field, so each is a mapping.
    assert rows.as_py() == [{"id": 1, "symbol": "A"}, {"id": 2, "symbol": "B"}]
    assert rows == Scalar.from_(Serie.from_(batch))
    restored_batch = rows.into_arrow_batch(field)
    assert restored_batch.equals(batch)

    # A table is a stream, so it is drained into the one column it holds.
    table = pa.Table.from_batches([batch, batch])
    table_rows = Scalar.from_(table)
    assert len(table_rows) == 4
    restored_table = table_rows.into_arrow_table(field)
    assert restored_table.equals(table.combine_chunks())


def test_record_rows_infer_struct_field_names() -> None:
    rows = Scalar.from_([Quote("AAPL", 12.5), Quote("MSFT", 9.0)])
    batch = rows.into_arrow_batch()
    assert batch.schema.names == ["price", "symbol"]
    assert batch.to_pylist() == [
        {"price": 12.5, "symbol": "AAPL"},
        {"price": 9.0, "symbol": "MSFT"},
    ]

    nullable = Scalar.from_([Venue(1, None), Venue(2, "XNAS")])
    nullable_batch = nullable.into_arrow_batch()
    assert nullable_batch.schema.field("name").nullable
    assert nullable_batch.column("name").to_pylist() == [None, "XNAS"]


def test_value_field_accessors_redirect_to_core_inference() -> None:
    scalar = Scalar.from_(42).into_field()
    assert scalar.name == "value"
    assert str(scalar.dtype) == "int64"
    assert not scalar.nullable

    item = Scalar.from_([1, None]).into_array_field()
    assert item.name == "item"
    assert str(item.dtype) == "int64"
    assert item.nullable

    root = Scalar.from_([Venue(1, None), Venue(2, "XNAS")]).into_struct_field()
    assert root.name == "row"
    assert not root.nullable
    children = list(root.dtype)
    assert [child.name for child in children] == ["id", "name"]
    assert children[1].nullable

    with pytest.raises(ValueError, match="empty Sequence"):
        Scalar.from_([]).into_array_field()
    with pytest.raises(ValueError, match="field names"):
        Scalar.from_([[1]]).into_struct_field()


def test_empty_rows_require_the_known_arrow_root_on_output() -> None:
    batch = pa.record_batch([pa.array([], type=pa.int32())], names=["id"])
    rows = Scalar.from_(batch)
    with pytest.raises(ValueError, match="empty rows"):
        rows.into_arrow_batch()
    assert rows.into_arrow_batch(Field.from_arrow_schema(batch.schema)).equals(batch)


def test_truthiness_reads_absence_zero_and_emptiness_as_false() -> None:
    # Before `__bool__` existed, `bool(scalar)` fell through to `__len__`,
    # which counts entries and answers zero for everything that is not a
    # container - so all four of these read False. They are the reversal.
    assert bool(Scalar.from_(5)) is True
    assert bool(Scalar.from_("abc")) is True
    assert bool(Scalar.from_(b"ab")) is True
    assert bool(Scalar.from_(True)) is True

    assert bool(Scalar.from_(0)) is False
    assert bool(Scalar.from_(False)) is False
    assert bool(Scalar.from_("")) is False
    assert bool(Scalar.from_([])) is False

    # Text a column spells false with, and the "struct all empty" case.
    assert bool(Scalar.from_("off")) is False
    assert bool(Scalar.from_({"a": None, "b": ""})) is False
    assert bool(Scalar.from_({"a": None, "b": 1})) is True


def crosses(value: object) -> Any:
    """Cross the native boundary without a lossy document intermediate."""

    return Scalar.from_(value).as_py()


def test_a_decimal_keeps_its_width_coefficient_and_scale() -> None:
    assert str(crosses(Decimal("1.50"))) == "1.50"
    assert str(crosses(Decimal("1.5"))) == "1.5"
    assert crosses(Decimal("1.05E+5")) == Decimal("1.05E+5")
    assert str(crosses(Decimal("1.05E+5"))) == "1.05E+5"

    widest_d128 = Decimal(2**127 - 1)
    assert Scalar.from_(widest_d128).kind == "d128"
    assert crosses(widest_d128) == widest_d128

    d256 = Decimal("1" * 40)
    assert Scalar.from_(d256).kind == "d256"
    assert crosses(d256) == d256


def test_a_non_finite_decimal_becomes_the_float_that_can_name_it() -> None:
    assert math.isnan(crosses(Decimal("NaN")))
    assert crosses(Decimal("-Infinity")) == -math.inf
    # Decimal stores no sign bit on its zero coefficient in the native model.
    assert str(crosses(Decimal("-0.00"))) == "0.00"


def test_a_decimal_wider_than_d256_is_refused_not_rounded() -> None:
    with pytest.raises(OverflowError, match="256 bits"):
        Scalar.from_(Decimal("1" * 80))
    with pytest.raises(OverflowError, match="no scale in -128..=127"):
        Scalar.from_(Decimal("1E+200"))


def test_natural_json_has_no_private_value_envelopes() -> None:
    encoded = json.dumps(
        {
            "price": Decimal("1.50"),
            "at": dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        }
    )
    assert json.loads(encoded) == {
        "price": "1.50",
        "at": "2026-08-15T12:03:04.000005",
    }


def test_temporals_cross_as_typed_native_scalars() -> None:
    values = [
        dt.date(2026, 8, 15),
        dt.time(23, 59, 59, 999_999),
        dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        dt.timedelta(days=-2, seconds=3, microseconds=4),
    ]
    assert [crosses(value) for value in values] == values
    assert [Scalar.from_(value).kind for value in values] == [
        "date32",
        "time64",
        "datetime64",
        "duration64",
    ]


def test_an_aware_datetime_preserves_the_instant_and_zone() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    value = dt.datetime(2026, 8, 15, 12, 3, 4, 5, tzinfo=paris)
    restored = crosses(value)
    assert restored == value
    assert restored.tzinfo.key == "Europe/Paris"

    offset = dt.timezone(dt.timedelta(hours=-3, minutes=-30))
    fixed = dt.datetime(2026, 1, 1, tzinfo=offset)
    assert crosses(fixed) == fixed


def test_an_ambiguous_zoned_reading_keeps_the_selected_instant() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    repeated = [
        dt.datetime(2026, 10, 25, 2, 30, fold=fold, tzinfo=paris)
        for fold in (0, 1)
    ]
    restored = [crosses(value) for value in repeated]
    assert [value.fold for value in restored] == [0, 1]
    assert [value.utcoffset() for value in restored] == [
        dt.timedelta(hours=2),
        dt.timedelta(hours=1),
    ]


def test_a_naive_fold_is_dropped_and_zoned_times_are_refused() -> None:
    assert crosses(dt.datetime(2026, 10, 25, 2, 30, fold=1)) == dt.datetime(
        2026, 10, 25, 2, 30
    )
    with pytest.raises(ValueError, match="timezone"):
        Scalar.from_(dt.time(1, 2, tzinfo=dt.timezone.utc))


def test_a_temporal_finer_than_python_holds_is_floored_not_refused() -> None:
    # `datetime` counts microseconds, so a nanosecond reading crosses floored
    # rather than withheld - the discarded remainder is under a microsecond,
    # and the Arrow path still carries the whole reading.
    assert DataType('datetime64(ns,"UTC")').scalar(1).as_py() == dt.datetime(
        1970, 1, 1, tzinfo=dt.timezone.utc
    )
    assert DataType('datetime64(ns,"UTC")').scalar(1_500).as_py() == dt.datetime(
        1970, 1, 1, 0, 0, 0, 1, tzinfo=dt.timezone.utc
    )
    # Floored, not truncated toward zero, so the rounding stays monotonic
    # across the epoch and two ordered values stay ordered.
    assert DataType('datetime64(ns,"UTC")').scalar(-1).as_py() == dt.datetime(
        1969, 12, 31, 23, 59, 59, 999_999, tzinfo=dt.timezone.utc
    )

    with pytest.raises(OverflowError, match="microseconds a duration counts"):
        Scalar.from_(dt.timedelta.max)

    # `Scalar.time` built this and only refused when projected to Python. The
    # type refuses the count against its unit's range at construction, which
    # is earlier and says more.
    with pytest.raises(ValueError, match="must be in 0..86400"):
        DataType("time32(s)").scalar(99_999_999)


def test_a_zone_with_no_rules_anywhere_is_named_in_the_error() -> None:
    with pytest.raises(ValueError, match='"Mars/Olympus"'):
        DataType('datetime64(s,"Mars/Olympus")').scalar(0).as_py()


def test_a_coarser_unit_is_restated_exactly() -> None:
    value = DataType('datetime64(s,"UTC")').scalar(1_700_000_000)
    assert value.as_py() == dt.datetime(
        2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc
    )


def test_none_crosses_everywhere_a_native_scalar_goes() -> None:
    assert crosses(None) is None
    assert crosses([None, 1]) == [None, 1]
    assert crosses({"gap": None}) == {"gap": None}
    assert crosses({None: 1}) == {None: 1}


def test_mapping_keys_cross_in_a_hashable_python_shape() -> None:
    assert crosses({(1, 2): "pair"}) == {(1, 2): "pair"}
    assert crosses({frozenset({1}): "one"}) == {(1,): "one"}


def test_equal_cross_width_numbers_share_a_hash() -> None:
    f32 = DataType("float32").scalar(1.0)
    f64 = DataType("float64").scalar(1.0)
    assert f32 == f64
    assert hash(f32) == hash(f64)


def test_a_value_answers_what_shape_it_is_without_lowering_it() -> None:
    assert Scalar.from_(None).is_null()
    assert not Scalar.from_(0).is_null()

    assert Scalar.from_([1, 2]).is_container()
    assert Scalar.from_({"a": 1}).is_container()
    assert not Scalar.from_("text").is_container()

    assert Scalar.from_(1).is_number()
    assert DataType("float64").scalar(1.5).is_number()
    assert Scalar.decimal(150, 2).is_number()
    assert not Scalar.from_("1").is_number()
    assert not Scalar.from_(True).is_number()

    assert Scalar.from_(1).is_integer()
    assert not DataType("float64").scalar(1.5).is_integer()
    assert not Scalar.decimal(150, 2).is_integer()


def test_a_value_narrows_to_the_python_number_it_is() -> None:
    assert Scalar.from_(True).as_bool() is True
    assert Scalar.from_(1).as_bool() is None

    # Python integers are unbounded, so nothing wraps and nothing narrows.
    assert Scalar.from_(7).as_int() == 7
    assert Scalar.from_(-7).as_int() == -7
    assert Scalar.from_(2**63).as_int() == 2**63
    assert Scalar.from_("7").as_int() is None
    assert DataType("float64").scalar(1.5).as_int() is None

    # A 32-bit float widens exactly, so both widths answer here.
    assert DataType("float32").scalar(1.5).as_float() == 1.5
    assert DataType("float64").scalar(1.5).as_float() == 1.5
    assert Scalar.from_(1).as_float() is None


def test_the_payload_bytes_of_a_value_carry_no_tag_and_no_length() -> None:
    assert Scalar.from_("AAPL").as_value_bytes() == b"AAPL"
    assert Scalar.from_(b"\x01\x02").as_value_bytes() == b"\x01\x02"
    assert Scalar.from_(1).as_value_bytes() == (1).to_bytes(8, "little")

    # Null and a container have no payload of their own.
    assert Scalar.from_(None).as_value_bytes() is None
    assert Scalar.from_([1]).as_value_bytes() is None

    # `as_bytes` is the narrower question: the bytes a byte value holds.
    assert Scalar.from_("AAPL").as_bytes() is None


def test_a_record_says_that_its_names_are_field_names() -> None:
    record = Scalar.from_struct({"b": 2, "a": 1})
    assert record.kind == "struct"
    assert record.as_py() == {"a": 1, "b": 2}

    # A Python mapping is a mapping; a record is what a struct row resolves to.
    assert Scalar.from_({"a": 1}).kind == "map"
    assert Scalar.from_struct([("a", 1), ("b", 2)]).kind == "struct"

    with pytest.raises(ValueError):
        Scalar.from_struct([("a", 1), ("a", 2)])
    with pytest.raises(TypeError):
        Scalar.from_struct([(1, "a")])


def test_a_default_answers_for_an_absent_name_and_for_a_stored_null() -> None:
    value = Scalar.from_({"symbol": "AAPL", "venue": None})

    assert value.get_or("symbol", "?").as_py() == "AAPL"
    # A stored null is no answer, which is what a configuration default means.
    assert value.get_or("venue", "XNAS").as_py() == "XNAS"
    assert value.get_or("absent", "XNAS").as_py() == "XNAS"

    record = Scalar.from_struct({"symbol": "MSFT"})
    assert record.get_or("symbol", "?").as_py() == "MSFT"
    assert record.get_or("absent", 0).as_py() == 0


def test_value_bytes_carry_any_value_and_pickle_rides_them() -> None:
    import pickle

    value = DataType("int32").scalar(7)
    data = value.into_value_bytes()
    assert isinstance(data, bytes)
    # The version, the identifier, then four little-endian bytes.
    assert data[0] == 0 and len(data) == 6 and data[2:] == b"\x07\x00\x00\x00"
    assert Scalar.from_value_bytes(data) == value
    assert Scalar.from_value_bytes(bytearray(data)) == value

    quote = Scalar.from_struct({"symbol": "AAPL", "sizes": [100, None]})
    assert Scalar.from_value_bytes(quote.into_value_bytes()) == quote
    assert Scalar.from_value_bytes(quote.into_value_bytes()).kind == "struct"

    for raw, expected in [
        ({"symbol": "AAPL", "sizes": [100, None]}, {"symbol": "AAPL", "sizes": [100, None]}),
        (7, 7),
        (None, None),
    ]:
        held = DataType("variant").scalar(raw)
        assert held.kind == "variant"
        assert held.as_py() == expected
        restored = Scalar.from_value_bytes(held.into_value_bytes())
        assert restored.kind == "variant"
        assert restored == held
        assert pickle.loads(pickle.dumps(held)) == held
    # Pickle hands the same bytes to `_from_pickle`.
    rebuilder, (state,) = quote.__reduce__()
    assert state == quote.into_value_bytes()
    assert rebuilder(state) == quote
    assert pickle.loads(pickle.dumps(quote)) == quote

    long = Scalar.from_("x" * (4 * 1024 + 1))
    data = long.into_value_bytes()
    assert data[2] == 1 and len(data) < 64
    assert Scalar.from_value_bytes(data) == long

    with pytest.raises(ValueError, match="version 1"):
        Scalar.from_value_bytes(b"\x01\x00")
    with pytest.raises(ValueError, match="bytes left"):
        Scalar.from_value_bytes(b"\x00\x00\x00")
