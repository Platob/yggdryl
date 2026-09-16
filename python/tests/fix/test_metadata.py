"""Native FIX reference metadata: live views, removals, and atomic refusals."""

from __future__ import annotations

from collections.abc import Iterator

import pytest

from yggdryl import DataType, Field


PROPERTIES = [
    ("counter", 453, "453"),
    ("component", "Party", "party"),
    ("field_ref", "PartyID", "partyid"),
    ("group", "Parties", "parties"),
    ("msgtype", "P Report Ack", "P Report Ack"),
]


@pytest.mark.parametrize("property_name,value,stored", PROPERTIES)
def test_reference_properties_share_metadata_and_remove_with_none(
    property_name: str, value: object, stored: str
) -> None:
    field = Field("probe", "utf8")
    view = field.fix
    key = f"fix:{'field' if property_name == 'field_ref' else property_name}"
    assert getattr(view, property_name) is None

    setattr(view, property_name, value)
    assert field.metadata[key] == stored
    expected = int(stored) if property_name == "counter" else stored
    assert getattr(field.fix, property_name) == expected
    assert Field.from_arrow(field.into_arrow()).metadata[key] == stored

    setattr(view, property_name, None)
    assert getattr(view, property_name) is None
    assert key not in field.metadata
    setattr(view, property_name, None)

    field.metadata[key] = stored
    assert getattr(view, property_name) == expected
    del view["field" if property_name == "field_ref" else property_name]
    assert getattr(view, property_name) is None
    assert key not in field.metadata


@pytest.mark.parametrize("property_name", ["component", "field_ref", "group"])
@pytest.mark.parametrize("invalid", ["", "..", "../party", "Party ID"])
def test_invalid_catalog_reference_preserves_the_complete_field(
    property_name: str, invalid: str
) -> None:
    field = Field("probe", "utf8", metadata={"source": "test"})
    setattr(field.fix, property_name, "Party")
    before = field.into_json()
    with pytest.raises(ValueError, match="catalog name"):
        setattr(field.fix, property_name, invalid)
    assert field.into_json() == before


# A counter is a positive tag: 0 is what an unresolved arrival records, so it
# is refused with a negative (``rust/tests/fix/zero_entries.rs``).
@pytest.mark.parametrize(
    "invalid,error",
    [(True, TypeError), (1.5, TypeError), (2**31, OverflowError), (-1, ValueError), (0, ValueError)],
)
def test_counter_refuses_invalid_python_integers_atomically(
    invalid: object, error: type[Exception]
) -> None:
    field = Field("probe", "utf8")
    field.fix.counter = 453
    before = field.into_json()
    with pytest.raises(error):
        field.fix.counter = invalid
    assert field.into_json() == before


@pytest.mark.parametrize("invalid", ["", "D\x01", "D\n"])
def test_message_code_refuses_control_text_without_changing_metadata(invalid: str) -> None:
    field = Field("probe", "utf8")
    field.fix.msgtype = "BridgeReport"
    before = field.into_json()
    with pytest.raises(ValueError, match="message-code"):
        field.fix.msgtype = invalid
    assert field.into_json() == before


@pytest.mark.parametrize("property_name,value,stored", PROPERTIES)
def test_reference_properties_refuse_other_protocols_and_frozen_fields(
    property_name: str, value: object, stored: str
) -> None:
    field = Field("probe", "utf8")
    with pytest.raises(TypeError, match="fix property"):
        getattr(field.iceberg, property_name)
    with pytest.raises(TypeError, match="fix property"):
        setattr(field.iceberg, property_name, value)

    setattr(field.fix, property_name, value)
    root = Field("row", DataType.from_fields([field]), nullable=False)
    row_type = root.into_dataclass(name=f"FrozenFix{property_name}")
    frozen = row_type.into_field().dtype[0]
    before = frozen.into_json()
    for replacement in (value, None):
        with pytest.raises(TypeError, match="read-only"):
            setattr(frozen.fix, property_name, replacement)
        assert frozen.into_json() == before


def test_identifiers_resolve_python_iterables_into_canonical_member_order() -> None:
    client = Field("clordid", "utf8")
    client.fix.tag = 11
    client.fix.names = ["ClientOrder"]
    order = Field("orderid", "utf8")
    order.fix.tag = 37
    component = Field("order", DataType.from_fields([client, order]), nullable=False)
    view = component.fix
    assert view.identifiers == []
    view.identifiers = (name for name in ["37", "ClientOrder"])
    assert view.identifiers == ["clordid", "orderid"]
    assert component.metadata["fix:identifiers"] == "clordid,orderid"
    assert Field.from_arrow(component.into_arrow()).fix.identifiers == view.identifiers
    view.identifiers = ("ORDERID",)
    assert view.identifiers == ["orderid"]
    view.identifiers = []
    assert view.identifiers == []
    assert "fix:identifiers" not in component.metadata


@pytest.mark.parametrize(
    "invalid",
    [[""], ["absent"], ["nested"], ["nested.child"], ["clordid,orderid"], ["clordid", "11"]],
)
def test_identifier_refusals_leave_the_complete_field_unchanged(invalid: list[str]) -> None:
    client = Field("clordid", "utf8", metadata={"fix:tag": "11"})
    nested = Field("nested", DataType.from_fields([Field("child", "utf8")]))
    component = Field("order", DataType.from_fields([client, nested]), nullable=False)
    component.fix.identifiers = ["clordid"]
    before = component.into_json()
    with pytest.raises(ValueError, match="order.fix:identifiers"):
        component.fix.identifiers = invalid
    assert component.into_json() == before


def test_identifiers_refuse_nontext_other_protocols_and_readonly_declarations() -> None:
    component = Field("order", DataType.from_fields([Field("clordid", "utf8")]), nullable=False)
    component.fix.identifiers = ["clordid"]
    before = component.into_json()
    for invalid in ("clordid", 11, ["clordid", 11]):
        with pytest.raises(TypeError):
            component.fix.identifiers = invalid  # type: ignore[assignment]
        assert component.into_json() == before

    def interrupted() -> Iterator[str]:
        yield "clordid"
        raise RuntimeError("identifier input failed")

    with pytest.raises(RuntimeError, match="identifier input failed"):
        component.fix.identifiers = interrupted()
    assert component.into_json() == before
    with pytest.raises(TypeError, match="fix property"):
        component.iceberg.identifiers
    with pytest.raises(TypeError, match="fix property"):
        component.iceberg.identifiers = ["clordid"]
    assert component.into_json() == before
    frozen = component.into_dataclass(name="IdentifierOrder").into_field()
    before = frozen.into_json()
    with pytest.raises(TypeError, match="read-only"):
        frozen.fix.identifiers = []
    assert frozen.into_json() == before
