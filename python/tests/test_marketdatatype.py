"""``MarketDataType``, the type of its kind a market element is, as a ``uint16``
enum: `python/yggdryl/marketdatatype.py` and `python/src/marketdatatype.rs`."""

from __future__ import annotations

import pickle
import re
from pathlib import Path

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, MarketDataKind, MarketDataType, Scalar
from yggdryl.fix import FixRegistry


def test_the_members_group_by_the_code_set_they_type() -> None:
    assert MarketDataType.UKNW == 0
    assert MarketDataType.ORDMKT == 101 and MarketDataType.ORDLIMIT == 102
    assert MarketDataType.ORDOTHER == 199
    assert MarketDataType.QUOTRAD == 201 and MarketDataType.QUOOTHER == 299
    assert MarketDataType.TRDBLOCK == 301 and MarketDataType.TRDOTHER == 399
    assert MarketDataType.BOOKBID == 400 and MarketDataType.BOOKOTHER == 499
    # The four request and report sets, each closed by its catch-all.
    assert MarketDataType.TRPTSUBMIT == 500 and MarketDataType.TRPTOTHER == 599
    assert MarketDataType.QRQMANUAL == 601 and MarketDataType.QRQOTHER == 699
    assert MarketDataType.MCXSECURITY == 701 and MarketDataType.MCXOTHER == 799
    assert MarketDataType.MDRSNAPSHOT == 800 and MarketDataType.MDROTHER == 899
    codes = [int(member) for member in MarketDataType]
    assert codes == sorted(codes)
    for member in MarketDataType:
        assert str(member) == member.name
        assert f"{member}" == member.name
        assert member.description


def test_the_stub_lists_the_native_table() -> None:
    stub = (Path(yggdryl.__file__).parent / "marketdatatype.pyi").read_text()
    listed = dict(re.findall(r"^    ([A-Z]+) = (\d+)$", stub, re.MULTILINE))
    assert {name: int(code) for name, code in listed.items()} == {
        member.name: int(member) for member in MarketDataType
    }


def test_a_name_or_a_fix_word_reads_to_one_member() -> None:
    assert MarketDataType.from_spelling("ORDLIMIT") is MarketDataType.ORDLIMIT
    assert MarketDataType.from_spelling("ordmkt") is MarketDataType.ORDMKT
    assert MarketDataType.from_spelling("Limit") is MarketDataType.ORDLIMIT
    assert MarketDataType.from_spelling("block trade") is MarketDataType.TRDBLOCK
    # A wire value is no spelling, and a stored code is an integer, never text.
    assert MarketDataType.from_spelling("2") is None
    assert MarketDataType.from_spelling("102") is None
    assert MarketDataType.from_spelling("not a type") is None


def test_a_fix_wire_value_reads_and_answers_back() -> None:
    assert MarketDataType.from_fix(40, "2") is MarketDataType.ORDLIMIT
    assert MarketDataType.from_fix(537, "1") is MarketDataType.QUOTRAD
    assert MarketDataType.from_fix(828, "1") is MarketDataType.TRDBLOCK
    assert MarketDataType.ORDLIMIT.fix_code == (40, "2")
    # A value no member names reads as its set's catch-all, a field that
    # types nothing as none.
    assert MarketDataType.from_fix(828, "999") is MarketDataType.TRDOTHER
    assert MarketDataType.from_fix(54, "1") is None
    assert MarketDataType.UKNW.fix_code is None
    assert MarketDataType.ORDOTHER.fix_code is None
    for member in MarketDataType:
        pair = member.fix_code
        if pair is not None:
            assert MarketDataType.from_fix(*pair) is member
    assert MarketDataType.fix_tags(MarketDataKind.ORDR) == (40,)
    assert MarketDataType.fix_tags(MarketDataKind.QUOT) == (537, 40)
    assert MarketDataType.fix_tags(MarketDataKind.BOOK) == (269, 828, 537, 40)
    # The request and report sets read off their own fields.
    assert MarketDataType.from_fix(856, "0") is MarketDataType.TRPTSUBMIT
    assert MarketDataType.from_fix(303, "1") is MarketDataType.QRQMANUAL
    assert MarketDataType.from_fix(530, "1") is MarketDataType.MCXSECURITY
    assert MarketDataType.from_fix(263, "0") is MarketDataType.MDRSNAPSHOT
    assert MarketDataType.from_fix(263, "Z") is MarketDataType.MDROTHER


def test_a_message_type_states_its_own_fields_before_its_kind() -> None:
    # A trade capture report is typed by what the report is before the
    # trade it reports; a message type with no rule reads its kind's.
    assert MarketDataType.fix_tags_of("AE", MarketDataKind.TRAD) == (856, 828, 40)
    assert MarketDataType.fix_tags_of("R", MarketDataKind.QUOT) == (303, 537, 40)
    assert MarketDataType.fix_tags_of("q", MarketDataKind.ORDR) == (530,)
    assert MarketDataType.fix_tags_of("V", MarketDataKind.BOOK) == (263,)
    assert MarketDataType.fix_tags_of("D", MarketDataKind.ORDR) == MarketDataType.fix_tags(MarketDataKind.ORDR)


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.marketdatatype("type", nullable=False)
    assert field.dtype == DataType("marketdatatype")
    assert DataType("marketdatatype").kind == "enum"
    for given in ("ORDLIMIT", 102, MarketDataType.ORDLIMIT):
        value = DataType("marketdatatype").scalar(given)
        assert value.kind == "marketdatatype"
        assert value.as_py() is MarketDataType.ORDLIMIT
    with pytest.raises(ValueError):
        DataType("marketdatatype").scalar(9999)
    with pytest.raises(ValueError):
        DataType("marketdatatype").scalar("not a type")
    held = Scalar.from_(MarketDataType.TRDBLOCK)
    assert held.kind == "marketdatatype"
    assert pickle.loads(pickle.dumps(held)).as_py() is MarketDataType.TRDBLOCK


def test_an_arrow_column_is_uint16_under_its_extension() -> None:
    arrow = Field("type", "marketdatatype").into_arrow()
    assert arrow.type.storage_type == pa.uint16()
    assert arrow.type.extension_name == "yggdryl.marketdatatype"


def test_the_registry_reads_the_crates_own_code_sets() -> None:
    registry = FixRegistry()
    assert registry.marketdatatype_of(40, "2") is MarketDataType.ORDLIMIT
    assert registry.marketdatatype_of(54, "1") is None
    assert registry.marketdatatype_of(40, "2") is MarketDataType.from_fix(40, "2")
    for tag, wire, member in registry.marketdatatype_sources():
        assert registry.marketdatatype_of(tag, wire) is member


def test_a_field_maps_its_wire_values_onto_members() -> None:
    field = Field("OrdType", "utf8")
    field.fix.tag = 40
    assert field.fix.marketdatatypes == []
    field.fix.marketdatatypes = [("Z", MarketDataType.ORDPEGGED), ("Y", "ordlimit"), ("X", 101)]
    assert field.fix.marketdatatypes == [
        ("Z", MarketDataType.ORDPEGGED),
        ("Y", MarketDataType.ORDLIMIT),
        ("X", MarketDataType.ORDMKT),
    ]
    registry = FixRegistry()
    registry.insert(field)
    assert registry.marketdatatype_of(40, "Z") is MarketDataType.ORDPEGGED
    assert (40, "Z", MarketDataType.ORDPEGGED) in registry.marketdatatype_sources()
    with pytest.raises(ValueError):
        field.fix.marketdatatypes = [("Z", "not a type")]
    with pytest.raises(ValueError):
        field.fix.marketdatatypes = [("Z", 101), ("Z", 102)]
    field.fix.marketdatatypes = []
    assert field.fix.marketdatatypes == []


def test_a_leaf_answers_the_member_it_is_typed_as() -> None:
    order = yggdryl.graph.OrderEvent(1, crosscode="O-1")
    assert order.marketdatatype is MarketDataType.UKNW
    typed = yggdryl.graph.OrderEvent(1, crosscode="O-1", marketdatatype=MarketDataType.ORDLIMIT)
    assert typed.marketdatatype is MarketDataType.ORDLIMIT
