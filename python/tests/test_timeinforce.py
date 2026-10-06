"""``TimeInForce``, how long an order stands, as a ``uint8`` enum:
`python/yggdryl/timeinforce.py` and `python/src/timeinforce.rs`."""

from __future__ import annotations

import pickle
import re
from pathlib import Path

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar, Serie, TimeInForce


def test_the_members_follow_the_wire_order() -> None:
    assert TimeInForce.UKNW == 0
    assert TimeInForce.DAY == 1 and TimeInForce.GTC == 2 and TimeInForce.IOC == 4
    assert TimeInForce.GFM == 13 and TimeInForce.OTHER == 99
    assert len(TimeInForce) == 15
    codes = [int(member) for member in TimeInForce]
    assert codes == sorted(codes)
    for member in TimeInForce:
        assert str(member) == member.name
        assert f"{member}" == member.name
        assert member.description


def test_the_stub_lists_the_native_table() -> None:
    stub = (Path(yggdryl.__file__).parent / "timeinforce.pyi").read_text()
    listed = dict(re.findall(r"^    ([A-Z]+) = (\d+)$", stub, re.MULTILINE))
    assert {name: int(code) for name, code in listed.items()} == {
        member.name: int(member) for member in TimeInForce
    }


def test_a_name_a_fix_word_or_a_wire_value_reads_to_one_member() -> None:
    assert TimeInForce.from_spelling("GTC") is TimeInForce.GTC
    assert TimeInForce.from_spelling("gtc") is TimeInForce.GTC
    assert TimeInForce.from_spelling("GoodTillCancel") is TimeInForce.GTC
    assert TimeInForce.from_spelling("immediate_or_cancel") is TimeInForce.IOC
    # One field states every wire value, so a wire value is a spelling too.
    assert TimeInForce.from_spelling("3") is TimeInForce.IOC
    assert TimeInForce.from_spelling("not a time in force") is None


def test_a_fix_wire_value_reads_and_answers_back() -> None:
    assert TimeInForce.from_fix("0") is TimeInForce.DAY
    assert TimeInForce.from_fix("1") is TimeInForce.GTC
    assert TimeInForce.from_fix("C") is TimeInForce.GFM
    assert TimeInForce.GTC.fix_code == "1"
    # A venue's own value reads as the catch-all, which answers no one value.
    assert TimeInForce.from_fix("Z") is TimeInForce.OTHER
    assert TimeInForce.OTHER.fix_code is None
    assert TimeInForce.UKNW.fix_code is None
    for member in TimeInForce:
        wire = member.fix_code
        if wire is not None:
            assert TimeInForce.from_fix(wire) is member


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.timeinforce("tif", nullable=False)
    assert field.dtype == DataType("timeinforce")
    assert DataType("timeinforce").kind == "enum"
    for given in ("GTC", "GoodTillCancel", 2, TimeInForce.GTC):
        value = DataType("timeinforce").scalar(given)
        assert value.kind == "timeinforce"
        assert value.as_py() is TimeInForce.GTC
    with pytest.raises(ValueError):
        DataType("timeinforce").scalar(98)
    with pytest.raises(ValueError):
        DataType("timeinforce").scalar("not a time in force")
    held = Scalar.from_(TimeInForce.IOC)
    assert held.kind == "timeinforce"
    assert pickle.loads(pickle.dumps(held)).as_py() is TimeInForce.IOC


def test_it_is_no_registered_code() -> None:
    assert not hasattr(yggdryl.codes, "timeinforce")
    assert not hasattr(yggdryl.codes, "TimeInForceField")


def test_an_arrow_column_is_uint8_under_its_extension() -> None:
    arrow = Field("tif", "timeinforce").into_arrow()
    assert arrow.type.storage_type == pa.uint8()
    assert arrow.type.extension_name == "yggdryl.timeinforce"


def test_a_column_casts_from_any_integer_and_to_integers_and_text() -> None:
    column = Serie.from_(pa.array([1, 4, None], pa.int64()), field=Field("tif", "timeinforce"))
    assert [cell.as_py() for cell in column] == [TimeInForce.DAY, TimeInForce.IOC, None]
    assert column.into_arrow_array().type.storage_type == pa.uint8()
    assert [cell.as_py() for cell in column.cast(Field("tif", "utf8"))] == ["DAY", "IOC", None]
    assert [cell.as_py() for cell in column.cast(Field("tif", "int16"))] == [1, 4, None]


def test_a_leaf_answers_the_member_it_stands_for() -> None:
    order = yggdryl.graph.OrderEvent(1, crosscode="O-1")
    assert order.timeinforce is None
    for given in (TimeInForce.IOC, 4, "IOC", "ImmediateOrCancel"):
        stated = yggdryl.graph.OrderEvent(1, crosscode="O-1", timeinforce=given)
        assert stated.timeinforce is TimeInForce.IOC
