"""Native FIX catalog snapshots, immutable views, and lazy stream parity."""
from __future__ import annotations

import copy
import json
import pickle
from typing import Any, Iterable

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, types
from yggdryl.fix import FixBranch, FixCodec, FixMessages, FixRegistry, MsgType, UlPlugin, UlPlugins, fix_crate_fields, fix_ulbridge_fields


def _field(name: str, tag: int, dtype: str = "utf8") -> Field:
    value = Field(name, dtype)
    value.fix.tag = tag
    return value


def _message(name: str, code: str, members: list[Field] | None = None) -> Field:
    value = Field(name, DataType.from_fields(members or []), nullable=False)
    value.fix.msgtype = code
    return value


def _catalog(members: Iterable[Field] = ()) -> FixRegistry:
    """`Party` referencing `PartyID` and then ``members``, restated by a group and a message."""
    registry = FixRegistry.from_fields([_field("NoPartyIDs", 453, "int32"), _field("PartyID", 448)])
    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    component = Field("Party", DataType.from_fields([member, *members]), nullable=False)
    registry.create_definition("components", component)
    group = types.list("Parties", component)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.create_definition("groups", group)
    group = registry.definition("groups", "Parties")
    group.fix.group = "Parties"
    counter = registry.field(453)
    counter.fix.field_ref = "NoPartyIDs"
    registry.create_definition("messages", _message("NewOrderSingle", "D", [counter, group]))
    return registry


def test_category_crud_refreshes_references_and_refuses_atomically(tmp_path: Any) -> None:
    registry = _catalog()
    before = registry.into_json()
    for category, name in [("fields", "PartyID"), ("components", "Party"), ("groups", "Parties")]:
        with pytest.raises(ValueError):
            registry.remove_definition(category, name)
        assert registry.into_json() == before
    with pytest.raises(ValueError):
        registry.create_definition("fields", _field("OtherName", 448))
    assert registry.into_json() == before
    for category, name in [("fields", "PartyID"), ("components", "Party"), ("groups", "Parties"), ("messages", "NewOrderSingle")]:
        original = registry.definition(category, name)
        changed = copy.copy(original)
        changed.set_name(name.lower())
        changed.fix.description = "Reviewed"
        assert registry.update_definition(category, changed) == original
        assert registry.definition(category, name).name == name
        assert registry.definition(category, name).fix.description == "Reviewed"
        with pytest.raises(ValueError):
            registry.create_definition(category, changed)
    assert registry.field_by_path("NewOrderSingle.Parties.PartyID").fix.description == "Reviewed"
    before = registry.into_json()
    with pytest.raises(ValueError):
        registry.update_definition("fields", _field("PartyID", 448, "int32"))
    assert registry.into_json() == before
    child = registry.field(448)
    child.fix.field_ref = "PartyID"
    child.fix.description = "An occurrence override"
    with pytest.raises(ValueError):
        registry.create_definition("components", Field("Invalid", DataType.from_fields([child]), nullable=False))
    assert registry.into_json() == before
    registry.write_into(tmp_path / "catalog")
    assert FixRegistry.from_handle(tmp_path / "catalog") == registry
    for category, name in [("messages", "NewOrderSingle"), ("groups", "Parties"), ("components", "Party"), ("fields", "PartyID"), ("fields", "NoPartyIDs")]:
        assert registry.remove_definition(category, name) is not None
        assert registry.get_definition(category, name) is None
        assert registry.remove_definition(category, name) is None
    # Only the crate's own fields are left: they seed every registry and
    # are never a definition a caller can remove.
    assert len(registry) == len(fix_crate_fields())


def test_inline_codes_are_per_field_and_snapshot_preserves_all_categories() -> None:
    registry = _catalog()
    registry.set_branch(FixBranch("pending", version="5.1.258", aliases=["future"]))
    value = registry.field(448)
    value.metadata["fix:codes"] = '{"codes":[{"value":"B","name":"Broker"}]}'
    registry.update_definition("fields", value)
    assert "Broker" in registry.field_by_path("NewOrderSingle.Parties.PartyID").metadata["fix:codes"]
    document = json.loads(registry.into_json())
    assert set(document) == {"fields", "components", "groups", "messages", "branches"}
    with pytest.raises(ValueError):
        registry.definitions("codesets")
    with pytest.raises(TypeError):
        hash(registry)
    for restored in [FixRegistry.from_json(registry.into_json()), copy.copy(registry), copy.deepcopy(registry), pickle.loads(pickle.dumps(registry))]:
        assert restored == registry
        assert restored.stable_hash() == registry.stable_hash()
        assert restored.branch_named("future") == registry.branch_named("pending")
        assert restored.definition("groups", "Parties").fix.counter == 453
        assert restored.msgtype("D").name == "NewOrderSingle"
    changed = copy.copy(registry)
    message = changed.definition("messages", "NewOrderSingle")
    message.fix.description = "Different message definition"
    changed.update_definition("messages", message)
    assert changed != registry
    assert changed.stable_hash() != registry.stable_hash()


def test_category_iterators_and_singletons_pin_their_registry() -> None:
    registry = _catalog()
    iterator = registry.definitions("components")
    assert iter(iterator) is iterator
    assert next(iterator).name == "Party"
    assert next(iterator, None) is None
    assert next(iterator, None) is None
    with pytest.raises(ValueError, match="shared"):
        registry.remove_definition("messages", "NewOrderSingle")
    del iterator
    singleton = registry.msgtype("D")
    assert isinstance(singleton, MsgType)
    assert singleton.value == "D"
    assert str(singleton) == "D"
    assert singleton.field.dtype == registry.definition("messages", "NewOrderSingle").dtype
    assert singleton.get_group_by_counter("453:").name == "Parties"
    with pytest.raises((AttributeError, TypeError, ValueError)):
        singleton.field.set_name("Changed")
    with pytest.raises(ValueError, match="shared"):
        registry.update_definition("messages", registry.definition("messages", "NewOrderSingle"))
    independent = copy.copy(registry)
    assert independent.remove_definition("messages", "NewOrderSingle") is not None
    del registry
    assert singleton.field.name == "NewOrderSingle"
    for restored in [copy.copy(singleton), copy.deepcopy(singleton), pickle.loads(pickle.dumps(singleton))]:
        assert restored == singleton
        assert hash(restored) == hash(singleton)
        assert restored.stable_hash() == singleton.stable_hash()
    with pytest.raises(TypeError):
        MsgType()


def test_singleton_iteration_keeps_identity_when_a_name_is_another_wire_code() -> None:
    registry = FixRegistry()
    registry.create_definition("messages", _message("D", "X"))
    registry.create_definition("messages", _message("NewOrderSingle", "D"))
    registry.create_definition("messages", _message("BridgeReport", "P Report Ack"))
    values = list(registry.msgtypes())
    assert [(value.name, value.value) for value in values] == [("BridgeReport", "P Report Ack"), ("D", "X"), ("NewOrderSingle", "D")]
    assert registry.msgtype("D").name == "NewOrderSingle"
    assert registry.msgtype("bridgereport").value == "P Report Ack"
    assert registry.get_msgtype("p report ack") is None
    restored = pickle.loads(pickle.dumps(values[1]))
    assert (restored.name, restored.value) == ("D", "X")
    del values
    ambiguous = _message("AnotherOrder", "D")
    registry.create_definition("messages", ambiguous)
    assert registry.get_msgtype("D") is None
    with pytest.raises(KeyError):
        registry.msgtype("D")


def _wildcard(size: int = 2) -> dict[str, Any]:
    return {"request": {"mbean": "com.ullink.ulbridge.sessioninterfaces.plugins:*", "type": "read"}, "value": {f"com.ullink.ulbridge.sessioninterfaces.plugins:name=Item{index},plugin-type=FIX,type=Plugin": {"Name": f"Item{index}", "CurrentPort": 9000 + index} for index in range(size)}, "status": 200}


def test_wildcard_values_are_lazy_owned_views_with_envelope_identity() -> None:
    document = _wildcard()
    iterator = UlPlugin.from_json_scalar(document)
    assert isinstance(iterator, UlPlugins)
    assert iter(iterator) is iterator
    first = next(iterator)
    document["value"].clear()
    assert next(iterator).name == "Item1"
    assert next(iterator, None) is None
    assert next(iterator, None) is None
    sibling = _wildcard()
    list(sibling["value"].values())[1]["CurrentPort"] = 9999
    same = next(UlPlugin.from_json_scalar(sibling))
    assert same == first
    assert hash(same) == hash(first)
    assert same.stable_hash() == first.stable_hash()
    sibling["status"] = 503
    changed = next(UlPlugin.from_json_scalar(sibling))
    assert changed != first
    assert changed.stable_hash() != first.stable_hash()
    rebuilt = UlPlugin(first.attributes, mbean=first.mbean, envelope=first.envelope)
    assert rebuilt == first
    assert rebuilt.stable_hash() == first.stable_hash()
    restored = pickle.loads(pickle.dumps(first))
    assert restored == first
    assert restored.envelope == first.envelope
    with pytest.raises(ValueError, match=r"ulconfig\[1\]"):
        UlPlugin.from_json_scalar([_wildcard(), None])


def test_bulk_messages_preserve_error_requests_source_columns_and_fuse() -> None:
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    codec = FixCodec(registry, branch="ulbridge")
    error = {"request": {"mbean": "com.ullink.ulbridge:type=Bridge", "type": "read"}, "status": 404, "error": "missing"}
    request = {"mbean": "com.ullink.ulbridge:type=Bridge", "type": "read"}
    raw = json.dumps([_wildcard(), error, request]).encode()
    messages = codec.parse_line(raw)
    assert isinstance(messages, FixMessages)
    assert iter(messages) is messages
    del codec
    values = list(messages)
    assert len(values) == 4
    assert [value.by_name("Name").as_py() for value in values[:2]] == ["Item0", "Item1"]
    assert values[2].by_name("Status").as_py() == 404
    assert values[2].by_name("Error").as_py() == "missing"
    assert values[3].by_name("Operation").as_py() == "read"
    assert all(value.get_by_name("SessionInterfaces") is None for value in values)
    assert next(messages, None) is None
    assert next(messages, None) is None
    capture = pa.table({"url": ["capture.log"], "rownum": [17], "body": pa.array([raw], type=pa.binary())})
    # One byte a batch is a batch a row; a bulk document is still one row per MBean.
    output = FixCodec(registry, branch="ulbridge", batch_byte_size=1).parse_text_arrow_reader(capture).read_all()
    assert output.num_rows == 4
    assert output.column("url").to_pylist() == ["capture.log"] * 4
    assert output.column("rownum").to_pylist() == [17] * 4
    assert output.column("body").to_pylist() == [raw] * 4
    config = UlPlugin({"CurrentPort": float("nan")})
    with pytest.raises(ValueError, match="non-finite"):
        config.into_fixmsg(FixCodec(registry))


# Only `ulbridge` is registered on request: the crate's own fields seed every
# registry, so there is no `with_crate_fields` left to refuse.
@pytest.mark.parametrize("method, vocabulary", [("with_ulbridge_fields", fix_ulbridge_fields)])
def test_registering_vocabulary_refusals_preserve_every_category(method: str, vocabulary: Any) -> None:
    registry = _catalog()
    conflict = vocabulary()[-1]
    conflict.set_name("ConflictingName")
    registry.insert(conflict)
    before = registry.into_json()
    with pytest.raises(ValueError):
        getattr(registry, method)()
    assert registry.into_json() == before
    assert registry.definition("groups", "Parties").fix.counter == 453


def test_message_singleton_ordering_delegates_to_native_fields() -> None:
    registry = FixRegistry()
    registry.create_definition("messages", _message("Alpha", "ZZ"))
    registry.create_definition("messages", _message("Beta", "A"))
    left, right = registry.msgtype("ZZ"), registry.msgtype("A")
    assert left < right
    assert right > left
    assert left <= copy.copy(left)
    assert left >= copy.copy(left)
    assert left != right
    assert sorted([right, left]) == [left, right]
    with pytest.raises(TypeError):
        left < "ZZ"


def test_standalone_branch_pickle_and_repr_keep_aliases() -> None:
    branch = FixBranch("venue", version="5.1.258", aliases=["counterparty"])
    restored = pickle.loads(pickle.dumps(branch))
    assert restored == branch
    assert restored.aliases == ["counterparty"]
    assert eval(repr(branch), {"FixBranch": FixBranch}) == branch


def test_branch_equality_hash_and_order_stay_within_native_values() -> None:
    branch = FixBranch("venue")
    same = FixBranch.from_value("venue")
    assert branch == same
    assert hash(branch) == hash(same)
    assert branch <= same and branch >= same
    assert branch != "venue"
    assert "venue" != branch
    assert branch != object()
    assert len({branch, same, "venue"}) == 2
    with pytest.raises(TypeError):
        branch < "venue"


def test_catalog_merge_refreshes_every_reference_with_the_inline_code_union() -> None:
    target, source = _catalog(), _catalog()
    for registry, code, name in [(target, "B", "Broker"), (source, "C", "Client")]:
        member = registry.field(448)
        member.metadata["fix:codes"] = json.dumps({"codes": [{"value": code, "name": name}]}, separators=(",", ":"))
        registry.update_definition("fields", member)
    message = source.definition("messages", "NewOrderSingle")
    message.set_name("IncomingOrder")
    message.fix.msgtype = "I"
    source.create_definition("messages", message)
    before_source = source.into_json()

    assert target.merge_with(source) == (0, 2)
    for path in ["PartyID", "Party.PartyID", "Parties.PartyID", "NewOrderSingle.Parties.PartyID", "IncomingOrder.Parties.PartyID"]:
        member = target.field_by_path(path)
        codes = json.loads(member.metadata["fix:codes"])["codes"]
        assert {item["value"]: item["name"] for item in codes} == {"B": "Broker", "C": "Client"}, path
    assert target.msgtype("I").get_group_by_counter("453:").name == "Parties"
    assert source.into_json() == before_source
    restored = FixRegistry.from_json(target.into_json())
    assert restored == target
    assert restored.stable_hash() == target.stable_hash()


def test_registry_add_definition_answers_whether_the_definition_arrived_or_folded() -> None:
    """`add_definition` extends the definition its name reaches; `"fields"` redirects to `add_field`."""
    registry = _catalog()
    assert registry.add_field(_field("PartyNote", 9002)) is True
    note = registry.field(9002)
    note.fix.field_ref = "PartyNote"
    extended = registry.definition("components", "Party")
    extended.set_dtype(DataType.from_fields([*extended, note]))
    assert registry.add_definition("components", extended) is False

    # The stored members keep their order and the incoming one is appended;
    # the group and the message that reference the component see it without
    # holding a copy, and the reference resolves again.
    assert [held.name for held in registry.definition("components", "Party")] == ["PartyID", "PartyNote"]
    for path in ("Party.PartyNote", "Parties.PartyNote", "NewOrderSingle.Parties.PartyNote"):
        member = registry.field_by_path(path)
        assert member.fix.tag == 9002, path
        assert member.fix.field_ref == "partynote", path
    assert FixRegistry.from_json(registry.into_json()) == registry

    # A message extends the same way and keeps its wire code.
    order = registry.definition("messages", "NewOrderSingle")
    order.set_dtype(DataType.from_fields([*order, Field("Text", "utf8")]))
    assert registry.add_definition("messages", order) is False
    order = registry.definition("messages", "NewOrderSingle")
    assert order.fix.msgtype == "D"
    assert [held.name for held in order] == ["NoPartyIDs", "Parties", "Text"]

    # A name no definition reaches arrives whole, and `"fields"` redirects a
    # scalar to `add_field`.
    hop = Field("Hop", DataType.from_fields([Field("HopID", "utf8")]), nullable=False)
    assert registry.add_definition("components", hop) is True
    assert registry.add_definition("fields", _field("Symbol", 55)) is True
    assert registry.field(55).name == "Symbol"

    # One level deep: a member both sides declare stays the stored one, so an
    # incoming member restating it under another datatype refuses the whole
    # call, and the strict verb still refuses the name outright.
    before = registry.into_json()
    disagreeing = Field("Party", DataType.from_fields([Field("partynote", "int32")]), nullable=False)
    with pytest.raises(ValueError, match="Party.PartyNote"):
        registry.add_definition("components", disagreeing)
    assert registry.into_json() == before
    with pytest.raises(ValueError):
        registry.create_definition("components", registry.definition("components", "Party"))
    assert registry.into_json() == before


def test_catalog_merge_extends_a_referenced_definition_and_refuses_a_changed_member() -> None:
    """A member the source adds is appended to the stored definition; one that disagrees refuses."""
    target = _catalog()
    member = target.field(448)
    member.metadata["fix:codes"] = '{"codes":[{"value":"C","name":"Client"}]}'
    source = FixRegistry.from_fields([member])
    source.set_branch("incoming")
    member = source.field(448)
    member.fix.field_ref = "PartyID"
    source.create_definition("components", Field("Party", DataType.from_fields([member, Field("Extra", "int32")]), nullable=False))
    before_source = source.into_json()

    # Nothing arrived - the source's one field is the target's own tag 448,
    # which merges - and the member it adds to `Party` is appended after the
    # member the target already declared, keeping the stored order. The group
    # and the message restate the component through references, so both see
    # it without holding a copy.
    assert target.merge_with(source) == (0, 1)
    assert [held.name for held in target.definition("components", "Party")] == ["PartyID", "Extra"]
    for path in ("Party.Extra", "Parties.Extra", "NewOrderSingle.Parties.Extra"):
        assert target.field_by_path(path).dtype == DataType("int32"), path
    assert target.field_by_path("NewOrderSingle.Parties.PartyID").fix.field_ref == "partyid"
    assert target.msgtype("D").get_group_by_counter("453:").name == "Parties"
    assert "Client" in target.field(448).metadata["fix:codes"]
    assert FixRegistry.from_json(target.into_json()) == target
    assert source.into_json() == before_source

    # Folding the same source again changes nothing at all.
    extended = target.into_json()
    extended_hash = target.stable_hash()
    assert target.merge_with(source) == (0, 1)
    assert target.into_json() == extended

    # A member both sides declare, under another datatype, refuses the whole
    # merge: the target is what it was, byte for byte and hash for hash.
    member = source.field(448)
    member.fix.field_ref = "PartyID"
    source.insert_definition("components", Field("Party", DataType.from_fields([member, Field("Extra", "int64")]), nullable=False))
    with pytest.raises(ValueError, match="Party.Extra"):
        target.merge_with(source)
    assert target.into_json() == extended
    assert target.stable_hash() == extended_hash


def test_folded_named_merges_keep_canonical_names_and_references() -> None:
    """Every spelling the one fold reads as a stored name folds into that definition."""
    original = _catalog()
    definitions = [("components", "Party"), ("groups", "Parties"), ("messages", "NewOrderSingle")]
    # Another case, and a separator the fold drops: two spellings of one name.
    for respell in (str.upper, "_{}".format):
        target = _catalog()
        source = FixRegistry.from_fields(list(target))
        for category, name in definitions:
            incoming = target.definition(category, name)
            incoming.set_name(respell(name))
            source.create_definition(category, incoming)

        # Nothing arrived: the two fields are the target's own and each
        # definition folds into the one its name spells, which keeps its
        # canonical name, its members and every reference to it.
        assert target.merge_with(source) == (0, 2)
        assert target == original
        for category, name in definitions:
            assert target.definition(category, respell(name)).name == name
            assert target.definition(category, name).name == name
        assert target.field_by_path("NewOrderSingle.Parties.PartyID").fix.field_ref == "partyid"
        assert target.msgtype("D").get_group_by_counter("453:").name == "Parties"
        assert FixRegistry.from_json(target.into_json()) == target

        # The strict verb reads the same fold and still refuses the name.
        with pytest.raises(ValueError):
            target.create_definition("components", source.definition("components", respell("Party")))
        assert target == original


def _numeric_branch_registry(scoped: bool) -> FixRegistry:
    registry = FixRegistry.from_fields([_field("MsgType", 35), _field("Symbol", 55), _field("CheckSum", 10)])

    def branch_field(name: str, tag: int, dtype: str, branch: str) -> Field:
        value = Field(name, dtype)
        value.fix.id = f"{tag}:{branch}"
        return value

    def group(name: str, counter: int, member: Field, branch: str) -> Field:
        component = Field(f"{name}Entry", DataType.from_fields([member]), nullable=False)
        component.fix.branch = branch
        registry.create_definition("components", component)
        held = types.list(name, component)
        held.fix.branch = branch
        held.fix.counter = counter
        held.fix.component = component.name
        registry.create_definition("groups", held)
        return held

    for branch, name, dtype in [("alpha", "Alpha", "int32"), ("beta", "Beta", "utf8")]:
        counter = branch_field(f"No{name}Rows", 6000, "int32", branch)
        member = branch_field(f"{name}ID", 6001, dtype, branch)
        tail = branch_field(f"{name}Value", 6002, dtype, branch)
        registry.add_fields([counter, member, tail])
        held = group(f"{name}Rows", 6000, member, branch)
        if scoped:
            message = _message(f"{name}Message", "X", [counter, held, tail])
            message.fix.branch = branch
            registry.create_definition("messages", message)
    counter = branch_field("NoAlphaOnlyRows", 6100, "int32", "alpha")
    member = branch_field("AlphaOnlyID", 6101, "int32", "alpha")
    registry.add_fields([counter, member])
    group("AlphaOnlyRows", 6100, member, "alpha")
    return registry


@pytest.mark.parametrize("scoped", [False, True])
@pytest.mark.parametrize(("branch", "name", "dtype", "member", "tail"), [
    ("alpha", "Alpha", "int32", 42, 7),
    ("beta", "Beta", "utf8", "42", "7"),
])
def test_numeric_fields_and_groups_follow_the_pinned_branch(scoped: bool, branch: str, name: str, dtype: str, member: Any, tail: Any) -> None:
    codec = FixCodec(_numeric_branch_registry(scoped), branch=branch)
    wire = b"35=X|6000=1|6001=42|6002=7|55=AAPL|10=0|"
    message = codec.parse_fix_line(wire)
    assert message.by_name(f"No{name}Rows").as_py() == 1
    assert message.by_path(f"{name}Rows.0.{name}ID").as_py() == member
    assert message.by_name(f"{name}Value").as_py() == tail
    assert message.field.field_by_path(f"{name}Value").dtype == DataType(dtype)
    assert message.by_tag(55).as_py() == "AAPL"
    assert message.into_bytes(ord("|")) == wire


def test_pinned_branch_does_not_borrow_another_venues_counter() -> None:
    codec = FixCodec(_numeric_branch_registry(False), branch="beta")
    wire = b"6100=1|6101=42|55=AAPL|"
    message = codec.parse_fix_line(wire)
    for name in ["NoAlphaOnlyRows", "AlphaOnlyRows", "AlphaOnlyID"]:
        assert message.get_by_name(name) is None
    assert message.by_name("6100").as_py() == "1"
    assert message.by_name("6101").as_py() == "42"
    assert message.by_tag(55).as_py() == "AAPL"
    assert message.into_bytes(ord("|")) == wire
