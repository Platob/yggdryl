"""``Side``, FIX's ``Side(54)`` as an ``int32`` enum: `python/yggdryl/side.py` and
`python/src/side.rs`."""

from __future__ import annotations

import pickle
import re
from pathlib import Path

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar, Side


def test_the_members_are_the_cores_in_code_order() -> None:
    codes = [int(side) for side in Side]
    assert codes == list(range(18))
    assert Side.UNKNOWN == 0 and Side.BUY == 1 and Side.SELL == 2
    assert Side.SELLUND == 17
    for side in Side:
        assert str(side) == side.name
        assert f"{side}" == side.name
        assert side.description


def test_the_stub_lists_the_native_table() -> None:
    stub = (Path(yggdryl.__file__).parent / "side.pyi").read_text()
    listed = dict(re.findall(r"^    ([A-Z]+) = (\d+)$", stub, re.MULTILINE))
    assert {name: int(code) for name, code in listed.items()} == {
        side.name: int(side) for side in Side
    }


def test_the_wire_code_and_the_two_lanes_are_the_cores() -> None:
    assert Side.UNKNOWN.fix_code is None
    assert Side.BUY.fix_code == "1"
    assert Side.CROSSSHX.fix_code == "A"
    assert Side.SELLUND.fix_code == "H"
    assert Side.BUY.is_bid() and not Side.BUY.is_ask()
    assert Side.SSHORTEX.is_ask() and not Side.SSHORTEX.is_bid()
    assert not Side.CROSS.is_bid() and not Side.CROSS.is_ask()
    assert not Side.UNKNOWN.is_bid() and not Side.UNKNOWN.is_ask()


def test_a_spelling_reads_to_one_member() -> None:
    assert Side.from_spelling("1") is Side.BUY
    assert Side.from_spelling("SellShort") is Side.SSHORT
    assert Side.from_spelling("sshort") is Side.SSHORT
    assert Side.from_spelling("H") is Side.SELLUND
    assert Side.from_spelling("X") is None


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.side("side", nullable=False)
    assert field.dtype == DataType("side")
    assert DataType("side").kind == "enum"
    for given in ("BUY", "1", 1, Side.BUY):
        value = DataType("side").scalar(given)
        assert value.kind == "side"
        assert value.as_py() is Side.BUY
    with pytest.raises(ValueError):
        DataType("side").scalar(99)
    with pytest.raises(ValueError):
        DataType("side").scalar("not a side")
    held = Scalar.from_(Side.SELL)
    assert held.kind == "side"
    assert pickle.loads(pickle.dumps(held)).as_py() is Side.SELL


def test_the_enums_package_holds_no_side() -> None:
    assert not hasattr(yggdryl.enums, "Side")
    assert not hasattr(yggdryl.enums, "SIDE")
    assert not hasattr(yggdryl.codes, "side")


def test_an_arrow_column_is_int32_under_its_extension() -> None:
    arrow = Field("side", "side").into_arrow()
    assert arrow.type == pa.int32()
    assert arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.side"
