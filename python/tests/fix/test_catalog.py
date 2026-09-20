"""Components, groups and message types, all through the field doors.

The registry is one namespace: a Struct is a component, a List of Structs or
a Map a group, and a message a component carrying ``FIX:msgtype``. There is
no category argument and no definition door of its own - ``insert``,
``update``, ``remove`` and the ``field_by_*`` pairs reach every one of them,
filing each by the shape it has.
"""

from __future__ import annotations

import copy
import json
import pathlib
import pickle
from typing import Any, Iterable

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, TextLine, types
from yggdryl.fix import FixCodec, FixMessages, FixMsg, FixRegistry, MsgType, fix_crate_fields


def _field(name: str, tag: int, dtype: str = "utf8") -> Field:
    value = Field(name, dtype)
    value.fix.tag = tag
    return value


def _message(name: str, code: str, members: list[Field] | None = None) -> Field:
    value = Field(name, DataType.from_fields(members or []), nullable=False)
    value.fix.msgtype = code
    return value


def _catalog(members: Iterable[Field] = ()) -> FixRegistry:
    """`Party` referencing `PartyID`, restated by a group and a message."""
    registry = FixRegistry.from_fields([_field("NoPartyIDs", 453, "int32"), _field("PartyID", 448)])
    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    registry.insert(Field("Party", DataType.from_fields([member, *members]), nullable=False))
    component = registry.field_by_name("Party")
    group = types.list("Parties", component)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.insert(group)
    group = registry.field_by_name("Parties")
    group.fix.group = "Parties"
    counter = registry.field(453)
    counter.fix.field_ref = "NoPartyIDs"
    registry.insert(_message("NewOrderSingle", "D", [counter, group]))
    return registry


def test_a_definition_is_filed_by_the_shape_it_has() -> None:
    """One `insert`: a Struct is a component, a List or a Map a group."""
    registry = _catalog()

    component = registry.field_by_name("Party")
    assert component.dtype.is_nested
    assert [member.name for member in component] == ["PartyID"]

    group = registry.field_by_name("Parties")
    assert group.fix.counter == 453
    assert group.fix.component == "party"
    # A group is reached by the counter it opens as well as by its name, and
    # the counter itself is still the scalar field it is.
    assert registry.field_by_counter(453).name == "Parties"
    assert registry.get_field_by_counter(9999) is None
    assert registry.field_by_tag(453).dtype == DataType("int32")

    # A message is a component carrying `FIX:msgtype`, so the same door
    # reaches it and the message-type view names it.
    message = registry.field_by_name("NewOrderSingle")
    assert message.fix.msgtype == "D"
    assert registry.msgtype("D").name == "NewOrderSingle"
    # The path grammar walks through the group into its member.
    assert registry.field_by_path("NewOrderSingle.Parties.PartyID").fix.tag == 448

    # A definition answers the tag door too, under the identity the catalog
    # derived for it - never under a tag a caller could claim.
    tag = component.fix.tag
    assert tag is not None
    assert registry.get_field_by_tag(tag).name == "Party"


def test_update_merges_a_definition_and_remove_keeps_a_referenced_one() -> None:
    registry = _catalog()
    before = registry.into_json()

    # A definition another one references stays, and answers `None`.
    for name in ("PartyID", "Party", "Parties"):
        assert registry.remove(name) is None
    assert registry.into_json() == before

    # `update` merges into the definition the folded name reaches, and every
    # reference to it sees the change without holding a copy.
    component = registry.field_by_name("Party")
    component.fix.description = "Reviewed"
    registry.update(component)
    assert registry.field_by_name("Party").fix.description == "Reviewed"
    assert registry.field_by_path("NewOrderSingle.Parties.PartyID") is not None
    assert registry.field_by_name("parties").fix.counter == 453

    # A definition nothing holds is refused by `update`.
    with pytest.raises(ValueError):
        registry.update(Field("Nope", DataType.from_fields([]), nullable=False))

    # Removed in reference order, each answers the definition it took out.
    for name in ("NewOrderSingle", "Parties", "Party", "PartyID", "NoPartyIDs"):
        assert registry.remove(name) is not None, name
        assert registry.get_field_by_name(name) is None, name
        assert registry.remove(name) is None, name

    # Only what every registry is built with is left.
    assert len(registry) == len(FixRegistry())
    assert [registry.field(tag).name for tag in (52, 60)] == ["sendingtime", "transacttime"]
    assert registry.field_by_counter(65020).name == "identifiers"


def test_a_code_set_is_the_dictionarys_and_a_snapshot_preserves_every_definition() -> None:
    registry = _catalog()
    # The members are stated first: a dictionary refuses a field naming a
    # vocabulary nothing states.
    registry.set_codeset("partyidcodeset", [{"value": "B", "name": "Broker"}])
    value = registry.field(448)
    value.fix.codeset = "partyidcodeset"
    # Membership is metadata like any other: it travels with the field.
    value.fix.branches = ["Pending"]
    registry.update(value)
    # Every reference reaches the one field, and the field names the one set.
    member = registry.field_by_path("NewOrderSingle.Parties.PartyID")
    assert member.fix.codeset == "partyidcodeset"
    assert member.metadata["FIX:codes"] == "partyidcodeset"
    codes = registry.codeset_of(member)
    assert codes is not None and [code["name"] for code in codes] == ["Broker"]
    assert registry.dialects() == ["pending"]

    document = json.loads(registry.into_json())
    # The vocabularies are the fourth key, and they lead the document: a
    # field names the set it reads by, so a reader holds the sets before it
    # meets a field naming one.
    assert set(document) == {"codesets", "fields", "components", "groups"}
    assert [held["name"] for held in document["codesets"]] == ["partyidcodeset"]
    with pytest.raises(TypeError):
        hash(registry)

    for restored in (
        FixRegistry.from_json(registry.into_json()),
        copy.copy(registry),
        copy.deepcopy(registry),
        pickle.loads(pickle.dumps(registry)),
    ):
        assert restored == registry
        assert restored.stable_hash() == registry.stable_hash()
        assert restored.dialects() == ["pending"]
        assert restored.field(448).fix.has_branch("PENDING")
        assert restored.field_by_name("Parties").fix.counter == 453
        assert restored.msgtype("D").name == "NewOrderSingle"

    changed = copy.copy(registry)
    message = changed.field_by_name("NewOrderSingle")
    message.fix.description = "Different message definition"
    changed.update(message)
    assert changed != registry
    assert changed.stable_hash() != registry.stable_hash()


def test_a_store_round_trips_every_definition(tmp_path: Any) -> None:
    registry = _catalog()
    registry.set_codeset("partyidcodeset", [{"value": "B", "name": "Broker"}])
    member = registry.field(448)
    member.fix.codeset = "partyidcodeset"
    registry.update(member)
    root = tmp_path / "catalog"
    registry.write_into(root)

    # The definitions land in the folders their shapes name, beside the
    # crate's own dump of the fixed row; a vocabulary is the fourth folder,
    # filed under the name the field that reads by it states.
    assert (root / "components" / "Party.json").exists()
    assert (root / "groups" / "Parties.json").exists()
    assert (root / "components" / "fixmsg.json").exists()
    assert (root / "codesets" / "partyidcodeset.json").exists()

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_path("NewOrderSingle.Parties.PartyID").fix.tag == 448
    assert [code["name"] for code in reloaded.codeset("partyidcodeset")] == ["Broker"]
    assert reloaded.msgtype("D").get_group_by_tag(453).name == "Parties"


def test_a_message_type_is_an_immutable_view_that_pins_its_registry() -> None:
    registry = _catalog()
    singleton = registry.msgtype("D")
    assert isinstance(singleton, MsgType)
    assert singleton.name == "NewOrderSingle"
    assert singleton.value == "D"
    assert str(singleton) == "D"
    assert repr(singleton) == 'MsgType("NewOrderSingle", "D")'
    assert singleton.field.dtype == registry.field_by_name("NewOrderSingle").dtype
    assert singleton.get_group_by_tag(453).name == "Parties"
    assert singleton.get_group_by_tag(9999) is None

    # The view is read-only and pins the registry while it lives.
    with pytest.raises((AttributeError, TypeError, ValueError)):
        singleton.field.set_name("Changed")
    with pytest.raises(ValueError, match="shared"):
        registry.update(registry.field_by_name("NewOrderSingle"))

    for restored in (
        copy.copy(singleton),
        copy.deepcopy(singleton),
        pickle.loads(pickle.dumps(singleton)),
    ):
        assert restored == singleton
        assert hash(restored) == hash(singleton)
        assert restored.stable_hash() == singleton.stable_hash()

    with pytest.raises(TypeError):
        MsgType()


def test_a_wire_code_reaches_the_message_it_names() -> None:
    registry = FixRegistry()
    registry.insert(_message("D", "X"))
    registry.insert(_message("NewOrderSingle", "D"))
    registry.insert(_message("BridgeReport", "P Report Ack"))

    # A new registry seeds no message type of its own.
    assert FixRegistry().get_msgtype("D") is None
    assert registry.msgtype("D").name == "NewOrderSingle"
    assert registry.msgtype("bridgereport").value == "P Report Ack"
    assert registry.get_msgtype("p report ack") is None
    assert registry.msgtype("neworder_single").value == "D"

    # Ordering is the definition's own, so a sort is the core's.
    left, right = registry.msgtype("D"), registry.msgtype("bridgereport")
    assert (left < right) != (right < left)
    assert sorted([right, left]) in ([left, right], [right, left])
    with pytest.raises(TypeError):
        left < "ZZ"


def test_message_singleton_identifiers_are_compiled_once() -> None:
    client, order = _field("clordid", 11), _field("orderid", 37)
    numeric = _field("numericid", 9001, "int64")
    declaration = _message("order", "D", [client, order, numeric])
    declaration.fix.identifiers = ["9001", "37", "11"]
    registry = FixRegistry.from_fields([client, order, numeric])
    registry.insert(declaration)

    row = Field(
        "row",
        DataType.from_fields([_field("venueorder", 37), client, numeric]),
        nullable=False,
    )
    numeric_value = 9_007_199_254_740_993
    message = FixMsg(row, ["O-1", "C-1", numeric_value], registry)
    singleton = registry.msgtype("D")
    selected = singleton.identifier_values(message)
    assert [(field.name, value.as_py()) for field, value in selected] == [
        ("clordid", "C-1"),
        ("orderid", "O-1"),
        ("numericid", numeric_value),
    ]
    assert selected[2][1].dtype == DataType("int64")
    assert type(selected[2][1].as_py()) is int
    with pytest.raises(TypeError, match="read-only"):
        selected[0][0].set_name("changed")

    absent = FixMsg(row, [None, "C-1", None], registry)
    assert [field.name for field, _ in singleton.identifier_values(absent)] == ["clordid"]
    with pytest.raises(TypeError):
        singleton.identifier_values("not a message")  # type: ignore[arg-type]


def test_identifier_declarations_merge_whole() -> None:
    client, order = _field("clordid", 11), _field("orderid", 37)
    registry = FixRegistry.from_fields([client, order])
    for member in (client, order):
        member.fix.field_ref = member.name
    stored = _message("order", "D", [client, order])
    stored.fix.identifiers = ["clordid"]
    registry.insert(stored)

    incoming = _message("order", "D", [order, client])
    incoming.fix.identifiers = ["37", "11"]
    assert incoming.fix.identifiers == ["orderid", "clordid"]
    # An incoming declaration replaces the previous one whole.
    registry.update(incoming)
    held = registry.field_by_name("order").fix.identifiers
    assert sorted(held) == ["clordid", "orderid"]

    restored = FixRegistry.from_json(registry.into_json())
    assert restored == registry
    assert restored.field_by_name("order").fix.identifiers == held

    malformed = copy.copy(incoming)
    malformed.metadata["FIX:identifiers"] = "clordid,,orderid"
    before = registry.into_json()
    with pytest.raises(ValueError, match="FIX:identifiers"):
        registry.update(malformed)
    assert registry.into_json() == before


def test_merge_with_folds_definitions_and_unions_their_membership() -> None:
    target, source = _catalog(), _catalog()
    # One name, two statements of it: the fold is the dictionary's to make,
    # and the field only ever carries the name.
    for registry, code, name in ((target, "B", "Broker"), (source, "C", "Client")):
        registry.set_codeset("partyidcodeset", [{"value": code, "name": name}])
        member = registry.field(448)
        member.fix.codeset = "partyidcodeset"
        registry.update(member)
    message = source.field_by_name("NewOrderSingle")
    message.set_name("IncomingOrder")
    message.fix.msgtype = "I"
    source.insert(message)
    before_source = source.into_json()

    # The counts are over the fields, which both dictionaries already hold.
    added, merged = target.merge_with(source)
    assert (added, merged) == (0, merged)
    assert merged >= 2
    for path in ("PartyID", "Party.PartyID", "Parties.PartyID", "NewOrderSingle.Parties.PartyID"):
        member = target.field_by_path(path)
        assert member.fix.codeset == "partyidcodeset", path
        codes = target.codeset_of(member)
        assert codes is not None, path
        assert {item["value"]: item["name"] for item in codes} == {"B": "Broker", "C": "Client"}, path
    assert target.msgtype("I").get_group_by_tag(453).name == "Parties"
    assert source.into_json() == before_source, "the source is untouched"

    restored = FixRegistry.from_json(target.into_json())
    assert restored == target
    assert restored.stable_hash() == target.stable_hash()


def test_a_json_row_is_one_unknown_message_keeping_its_source_columns() -> None:
    registry = FixRegistry()
    # A document states no message type, so this codec reads the untyped row
    # the default refusals drop.
    codec = FixCodec(registry, exclude_msgtypes=[])
    raw = json.dumps({"request": {"type": "read"}, "status": 200}).encode()

    messages = codec.parse_line(raw)
    assert isinstance(messages, FixMessages)
    assert iter(messages) is messages
    values = list(messages)
    assert len(values) == 1
    assert values[0].field.name == "unknown"
    assert values[0].entries() == []
    assert values[0].get_by_name("status") is None
    assert next(messages, None) is None

    # One byte a batch is a batch a row, and a document row keeps the
    # columns it arrived with.
    capture = pa.table(
        {"url": ["capture.log"], "rownum": [17], "body": pa.array([raw], type=pa.binary())}
    )
    output = (
        FixCodec(registry, batch_byte_size=1, exclude_msgtypes=[])
        .parse_text_arrow_reader(capture)
        .read_all()
    )
    assert output.num_rows == 1
    assert output.column("url").to_pylist() == ["capture.log"]
    assert output.column("rownum").to_pylist() == [17]

    # Any JSON object is the same row through every door.
    stranger = b'{"a":1}'
    assert next(codec.parse_line(stranger)).field.name == "unknown"
    assert next(codec.parse_text_line(TextLine(17, stranger))).field.name == "unknown"


def _numeric_group_registry(scoped: bool) -> FixRegistry:
    """A venue's counted group under tags 6000..6002, stamped as the venue's."""
    registry = FixRegistry.from_fields(
        [_field("MsgType", 35), _field("Symbol", 55), _field("CheckSum", 10)]
    )

    def venue_field(name: str, tag: int, dtype: str) -> Field:
        value = Field(name, dtype)
        value.fix.tag = tag
        value.fix.branches = ["alpha"]
        return value

    counter = venue_field("NoAlphaRows", 6000, "int32")
    member = venue_field("AlphaID", 6001, "int32")
    tail = venue_field("AlphaValue", 6002, "int32")
    registry.add_fields([counter, member, tail])
    component = Field("AlphaRowsEntry", DataType.from_fields([member]), nullable=False)
    component.fix.branches = ["alpha"]
    registry.insert(component)
    held = types.list("AlphaRows", component)
    held.fix.branches = ["alpha"]
    held.fix.counter = 6000
    held.fix.component = component.name
    registry.insert(held)
    if scoped:
        message = _message("AlphaMessage", "X", [counter, held, tail])
        message.fix.branches = ["alpha"]
        registry.insert(message)
    return registry


@pytest.mark.parametrize("scoped", [False, True])
def test_numeric_groups_resolve_through_the_one_namespace(scoped: bool) -> None:
    """Membership is provenance: a venue's counted group needs no pin."""
    registry = _numeric_group_registry(scoped)
    assert registry.dialects() == ["alpha"]
    assert registry.field_by_counter(6000).name == "AlphaRows"
    assert registry.field_by_tag(6001).fix.has_branch("alpha")

    codec = FixCodec(registry)
    wire = b"35=X|6000=1|6001=42|6002=7|55=AAPL|10=0|"
    message = codec.parse_fix_line(wire)
    assert message.by_name("NoAlphaRows").as_py() == 1
    assert message.by_path("AlphaRows[0].AlphaID").as_py() == 42
    assert message.by_name("AlphaValue").as_py() == 7
    assert message.by_tag(55).as_py() == "AAPL"
    # The header the parse read the frame at leads the wire, and the row
    # follows it as the line stated it.
    assert message.into_bytes(ord("|")).endswith(b"6000=1|6001=42|6002=7|55=AAPL|10=0|")
    # A message root the codec builds is not a dictionary member.
    assert message.field.fix.branches == []

    # A counter no group declares is kept under its own spelling.
    loose = codec.parse_fix_line(b"6100=1|6101=42|55=AAPL|")
    assert loose.get_by_name("AlphaRows") is None
    assert loose.by_name("6100").as_py() == "1"
    assert loose.by_name("6101").as_py() == "42"


def test_the_crate_map_groups_are_groups_a_message_may_reference(tmp_path: Any) -> None:
    registry = FixRegistry()
    mapping = registry.field_by_counter(65020)
    assert mapping.name == "identifiers"
    mapping.fix.group = "identifiers"
    registry.insert(_message("identified", "ID", [mapping]))

    snapshot = json.loads(registry.into_json())
    assert "identifiers" in [group["name"] for group in snapshot["groups"]]
    assert FixRegistry.from_json(registry.into_json()) == registry

    location = tmp_path / "identifiers-reference"
    registry.write_into(location)
    assert (location / "groups" / "identifiers.json").exists()
    assert FixRegistry.from_handle(location) == registry
