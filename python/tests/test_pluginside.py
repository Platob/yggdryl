"""``PluginSide``, the role of a FIX plugin, as a ``uint8`` enum:
`python/yggdryl/pluginside.py` and `python/src/pluginside.rs`."""

from __future__ import annotations

import pickle
import re
from pathlib import Path

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, PluginSide, Scalar, Serie, Side

BUY_SIDE_TYPE = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock"
SELL_SIDE_TYPE = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock"


def test_the_members_are_the_three_roles() -> None:
    assert PluginSide.UKNW == 0 and PluginSide.BUYS == 1 and PluginSide.SELL == 2
    assert len(PluginSide) == 3
    for member in PluginSide:
        assert str(member) == member.name
        assert f"{member}" == member.name
        assert member.description
    assert "Buy-Side" in PluginSide.BUYS.description


def test_the_stub_lists_the_native_table() -> None:
    stub = (Path(yggdryl.__file__).parent / "pluginside.pyi").read_text()
    listed = dict(re.findall(r"^    ([A-Z]+) = (\d+)$", stub, re.MULTILINE))
    assert {name: int(code) for name, code in listed.items()} == {
        member.name: int(member) for member in PluginSide
    }


def test_a_stored_name_or_a_role_name_reads_to_one_member() -> None:
    assert PluginSide.from_spelling("SELL") is PluginSide.SELL
    assert PluginSide.from_spelling("buys") is PluginSide.BUYS
    assert PluginSide.from_spelling("BuySide") is PluginSide.BUYS
    assert PluginSide.from_spelling("sell-side") is PluginSide.SELL
    assert PluginSide.from_spelling("sell_side") is PluginSide.SELL
    assert PluginSide.from_spelling("Unknown") is PluginSide.UKNW
    assert PluginSide.from_spelling("uknw") is PluginSide.UKNW
    assert PluginSide.from_spelling("not a role") is None


def test_a_plugin_class_name_states_its_role_in_its_last_segment() -> None:
    assert PluginSide.from_plugin_type(BUY_SIDE_TYPE) is PluginSide.BUYS
    assert PluginSide.from_plugin_type(SELL_SIDE_TYPE) is PluginSide.SELL
    assert PluginSide.from_plugin_type("x.Buy_Side_FIXCPluginCBlock") is PluginSide.BUYS
    assert PluginSide.from_plugin_type("x.FIXCPluginCBlock") is PluginSide.UKNW
    # Only the last segment is read: a package naming a side names no role.
    assert PluginSide.from_plugin_type("buyside.FIXCPluginCBlock") is PluginSide.UKNW
    assert PluginSide.from_plugin_type("") is PluginSide.UKNW


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.pluginside("pluginside", nullable=False)
    assert field.dtype == DataType("pluginside")
    assert DataType("pluginside").kind == "enum"
    for given in ("SELL", "SellSide", 2, PluginSide.SELL):
        value = DataType("pluginside").scalar(given)
        assert value.kind == "pluginside"
        assert value.as_py() is PluginSide.SELL
    with pytest.raises(ValueError):
        DataType("pluginside").scalar(3)
    with pytest.raises(ValueError):
        DataType("pluginside").scalar("not a role")
    held = Scalar.from_(PluginSide.BUYS)
    assert held.kind == "pluginside"
    assert pickle.loads(pickle.dumps(held)).as_py() is PluginSide.BUYS


def test_it_is_no_side_though_two_members_are_spelled_alike() -> None:
    # Two enums: a member of one is never a member of the other.
    assert Scalar.from_(PluginSide.BUYS).kind == "pluginside"
    assert Scalar.from_(Side.BUYS).kind == "side"
    assert Scalar.from_(PluginSide.BUYS) != Scalar.from_(Side.BUYS)
    assert not hasattr(yggdryl.codes, "pluginside")


def test_an_arrow_column_is_uint8_under_its_extension() -> None:
    arrow = Field("pluginside", "pluginside").into_arrow()
    assert arrow.type.storage_type == pa.uint8()
    assert arrow.type.extension_name == "yggdryl.pluginside"


def test_a_column_casts_from_any_integer_and_to_integers_and_text() -> None:
    column = Serie.from_(pa.array([1, 2, None], pa.int64()), field=Field("role", "pluginside"))
    assert [cell.as_py() for cell in column] == [PluginSide.BUYS, PluginSide.SELL, None]
    assert column.into_arrow_array().type.storage_type == pa.uint8()
    assert [cell.as_py() for cell in column.cast(Field("role", "utf8"))] == ["BUYS", "SELL", None]
    assert [cell.as_py() for cell in column.cast(Field("role", "int16"))] == [1, 2, None]
