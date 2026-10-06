"""``MarketDataKind``, FIX's MsgCat code set as a ``uint8`` enum:
`python/yggdryl/marketdatakind.py` and `python/src/marketdatakind.rs`."""

from __future__ import annotations

import pickle
import re
from pathlib import Path

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, MarketDataKind, Scalar


def test_the_members_are_the_cores_in_code_order() -> None:
    codes = [int(kind) for kind in MarketDataKind]
    assert codes == list(range(26))
    assert MarketDataKind.UNKN == 0 and MarketDataKind.BOOK == 3
    assert MarketDataKind.EXEC == 8 and MarketDataKind.ORDR == 10
    assert MarketDataKind.QUOT == 14 and MarketDataKind.TRAD == 21
    assert MarketDataKind.ORDB == 22 and MarketDataKind.QUOB == 23
    assert MarketDataKind.EXEB == 24 and MarketDataKind.TRDB == 25
    for kind in MarketDataKind:
        assert str(kind) == kind.name
        assert f"{kind}" == kind.name
        assert kind.description


def test_the_stub_lists_the_native_table() -> None:
    stub = (Path(yggdryl.__file__).parent / "marketdatakind.pyi").read_text()
    listed = dict(re.findall(r"^    ([A-Z]+) = (\d+)$", stub, re.MULTILINE))
    assert {name: int(code) for name, code in listed.items()} == {
        kind.name: int(kind) for kind in MarketDataKind
    }


def test_a_code_or_a_word_reads_to_one_member() -> None:
    assert MarketDataKind.from_spelling("ORDR") is MarketDataKind.ORDR
    assert MarketDataKind.from_spelling("ordr") is MarketDataKind.ORDR
    assert MarketDataKind.from_spelling("quotation") is MarketDataKind.QUOT
    assert MarketDataKind.from_spelling("market_structure") is MarketDataKind.MKST
    assert MarketDataKind.from_spelling("order_batch") is MarketDataKind.ORDB
    # A stored code is an integer, never text.
    assert MarketDataKind.from_spelling("10") is None
    assert MarketDataKind.from_spelling("not a kind") is None


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.marketdatakind("kind", nullable=False)
    assert field.dtype == DataType("marketdatakind")
    assert DataType("marketdatakind").kind == "enum"
    for given in ("ORDR", 10, MarketDataKind.ORDR):
        value = DataType("marketdatakind").scalar(given)
        assert value.kind == "marketdatakind"
        assert value.as_py() is MarketDataKind.ORDR
    with pytest.raises(ValueError):
        DataType("marketdatakind").scalar(99)
    with pytest.raises(ValueError):
        DataType("marketdatakind").scalar("not a kind")
    held = Scalar.from_(MarketDataKind.TRAD)
    assert held.kind == "marketdatakind"
    assert pickle.loads(pickle.dumps(held)).as_py() is MarketDataKind.TRAD


def test_an_arrow_column_is_uint8_under_its_extension() -> None:
    arrow = Field("kind", "marketdatakind").into_arrow()
    assert arrow.type.storage_type == pa.uint8()
    assert arrow.type.extension_name == "yggdryl.marketdatakind"
