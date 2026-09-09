"""Native FIX reference metadata: live views, removals, and atomic refusals."""

from __future__ import annotations

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


@pytest.mark.parametrize(
    "invalid,error", [(True, TypeError), (1.5, TypeError), (2**31, OverflowError), (-1, ValueError)]
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
    field.fix.msgtype = "ConfigurationPlugin"
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
    frozen = row_type.field().dtype[0]
    before = frozen.into_json()
    for replacement in (value, None):
        with pytest.raises(TypeError, match="read-only"):
            setattr(frozen.fix, property_name, replacement)
        assert frozen.into_json() == before
