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
    assert codes == [*range(18), 99]
    assert Side.UKNW == 0 and Side.BUYS == 1 and Side.SELL == 2
    assert Side.SELU == 17
    # Both sides at once - a book's, a two-sided quote's - stands last.
    assert Side.BOTH == 99
    assert [side.name for side in Side][:6] == ["UKNW", "BUYS", "SELL", "BUYM", "SELP", "SSHT"]
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
    assert Side.UKNW.fix_code is None
    assert Side.BUYS.fix_code == "1"
    assert Side.CRSX.fix_code == "A"
    assert Side.SELU.fix_code == "H"
    assert Side.BUYS.is_bid() and not Side.BUYS.is_ask()
    assert Side.SSEX.is_ask() and not Side.SSEX.is_bid()
    assert not Side.CROS.is_bid() and not Side.CROS.is_ask()
    assert not Side.UKNW.is_bid() and not Side.UKNW.is_ask()
    # Both sides at once takes neither leg, and no message carries it.
    assert Side.BOTH.fix_code is None
    assert not Side.BOTH.is_bid() and not Side.BOTH.is_ask()


def test_a_spelling_reads_to_one_member() -> None:
    assert Side.from_spelling("1") is Side.BUYS
    assert Side.from_spelling("BUYS") is Side.BUYS
    assert Side.from_spelling("ssht") is Side.SSHT
    assert Side.from_spelling("SellShort") is Side.SSHT
    # The names stored before the four-letter codes are still read, never written.
    assert Side.from_spelling("sshort") is Side.SSHT
    assert Side.from_spelling("BUY") is Side.BUYS
    assert Side.from_spelling("UNKNOWN") is Side.UKNW
    assert Side.SSHT.name == "SSHT"
    assert Side.from_spelling("H") is Side.SELU
    assert Side.from_spelling("both") is Side.BOTH
    assert Side.from_spelling("two-sided") is Side.BOTH
    assert Side.from_spelling("X") is None


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.side("side", nullable=False)
    assert field.dtype == DataType("side")
    assert DataType("side").kind == "enum"
    for given in ("BUYS", "BUY", "1", 1, Side.BUYS):
        value = DataType("side").scalar(given)
        assert value.kind == "side"
        assert value.as_py() is Side.BUYS
    assert DataType("side").scalar(99).as_py() is Side.BOTH
    with pytest.raises(ValueError):
        DataType("side").scalar(98)
    with pytest.raises(ValueError):
        DataType("side").scalar("not a side")
    held = Scalar.from_(Side.SELL)
    assert held.kind == "side"
    assert pickle.loads(pickle.dumps(held)).as_py() is Side.SELL


def test_the_enums_package_holds_no_side() -> None:
    assert not hasattr(yggdryl.enums, "Side")
    assert not hasattr(yggdryl.enums, "SIDE")
    assert not hasattr(yggdryl.codes, "side")


def test_an_arrow_column_is_uint8_under_its_extension() -> None:
    arrow = Field("side", "side").into_arrow()
    assert arrow.type.storage_type == pa.uint8()
    assert arrow.type.extension_name == "yggdryl.side"
