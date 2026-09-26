"""The exact decimal field factories and the fixed leaves: `python/yggdryl/decimal.py`."""

from __future__ import annotations

import decimal
import pickle

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, scalar

D = decimal.Decimal


def test_the_fixed_leaves_are_datatypes_of_their_own() -> None:
    fixed = DataType("decimal")
    assert fixed.id == "decimal" and str(fixed) == "decimal"
    wide = DataType("bigdecimal")
    assert wide.id == "bigdecimal" and str(wide) == "bigdecimal"
    # The parameterized family is still what `decimal(p, s)` spells.
    assert DataType("decimal(10, 2)").id == "decimal64"
    assert fixed != DataType("decimal128(38, 18)")


def test_the_field_factories() -> None:
    price = yggdryl.decimal("px")
    assert isinstance(price, Field)
    assert price.name == "px" and price.dtype == DataType("decimal") and price.nullable
    assert not yggdryl.decimal("px", nullable=False).nullable
    # A stated precision picks the narrowest width that holds it.
    assert yggdryl.decimal("px", 10, 2).dtype == DataType("decimal64(10, 2)")
    assert yggdryl.decimal("px", 38).dtype == DataType("decimal128(38, 0)")
    assert yggdryl.bigdecimal("n").dtype == DataType("bigdecimal")
    # A scale alone says nothing a fixed leaf could take.
    with pytest.raises(TypeError, match="a scale needs a precision"):
        yggdryl.decimal("px", scale=2)  # type: ignore[call-overload]


def test_a_value_is_restated_at_scale_eighteen_and_a_nineteenth_digit_refused() -> None:
    price = Field("px", "decimal")
    value = price.scalar(D("82.5"))
    assert value.kind == "decimal"
    assert value.dtype == DataType("decimal")
    assert value.as_py() == D("82.5")
    assert price.scalar(1).as_py() == D("1")
    with pytest.raises(ValueError, match=r"\$\.px: expected a decimal representable at scale 18"):
        price.scalar(D("0.0000000000000000001"))
    big = Field("n", "bigdecimal").scalar(D("12345678901234567890123456789012345678901234567890.5"))
    assert big.kind == "bigdecimal"
    assert big.as_py() == D("12345678901234567890123456789012345678901234567890.5")


def test_a_value_pickles_as_itself() -> None:
    for value in (
        Field("px", "decimal").scalar(D("-82.125")),
        Field("n", "bigdecimal").scalar(D("7")),
    ):
        twin = pickle.loads(pickle.dumps(value))
        assert twin == value and twin.kind == value.kind
        assert twin.as_py() == value.as_py()


def test_arrow_storage_is_the_widths_under_the_leafs_own_extension_name() -> None:
    fixed = Field("px", "decimal").into_arrow()
    assert fixed.type == pa.decimal128(38, 18)
    assert fixed.metadata[b"ARROW:extension:name"] == b"yggdryl.decimal"
    wide = Field("n", "bigdecimal").into_arrow()
    assert wide.type == pa.decimal256(76, 18)
    assert wide.metadata[b"ARROW:extension:name"] == b"yggdryl.bigdecimal"
    assert DataType("decimal").into_arrow() == pa.decimal128(38, 18)


@scalar
class Quote:
    px: decimal.Decimal
    size: int


def test_a_decimal_hint_is_a_decimal_column() -> None:
    assert DataType(decimal.Decimal) == DataType("decimal")
    root = Quote.into_field()
    assert root.dtype["px"].dtype == DataType("decimal")
    assert not root.dtype["px"].nullable
    # A row of the class holds its price at the leaf's scale.
    row = root.scalar([D("101.25"), 3])
    assert row[0].kind == "decimal" and row[0].as_py() == D("101.25")
