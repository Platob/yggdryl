"""Boundary cost of the FIX dictionary, against the native numbers.

Every case here is one crossing over a registry the core resolves: what is
measured is the coercion of the key - a tag, a name, an identifier - the
wrapper the answer is put in, and - for the two loads - the shard read the
boundary only names. Run after installing the release wheel with::

    python benchmarks/fix.py --iterations 2000

The generated registry is written to a temporary folder and removed on the way
out, so the only tracked input is the seed dictionary at ``config/fix``.
"""

from __future__ import annotations

import argparse
import copy
import decimal
import io
import pickle
import gc
import pathlib
import shutil
import statistics
import tempfile
import timeit
from collections.abc import Callable

import pyarrow as pa

from yggdryl import DataType, Field, MimeType, TextLine, types
from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
SEED = REPO / "config" / "fix"

SEED_REGISTRY = FixRegistry.from_handle(SEED)
ORDER = Field(
    "NewOrderSingle",
    DataType.from_fields(
        [
            SEED_REGISTRY.field_by_tag(55),
            SEED_REGISTRY.field_by_tag(38),
            SEED_REGISTRY.field_by_name("NoPartyIDs"),
            SEED_REGISTRY.field_by_name("Parties"),
        ]
    ),
    nullable=False,
)
MESSAGE = FixMsg(
    ORDER,
    {
        "symbol": "AAPL",
        "orderqty": decimal.Decimal("100"),
        "nopartyids": 1,
        "parties": [
            {"partyid": "BROKER", "partyidsource": "D", "partyrole": 1}
        ],
    },
    SEED_REGISTRY,
)
WIDE_FIELDS = 200
VENDOR_DIALECT = "cme"
VENDOR_FIELDS = 200
FIXML_LINE = b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|"
ULLINK_LINE = "ACCOUNT=A1|MSGTYPE=D|SYMBOL=AAPL"


def _vendor_registry() -> FixRegistry:
    """The seed beside a vendor dictionary's fields, stamped as the vendor's."""
    registry = copy.copy(SEED_REGISTRY)
    fields = []
    for offset in range(VENDOR_FIELDS):
        tag = 5000 + offset
        field = Field(f"Venue{offset}", "utf8")
        field.fix.tag = tag
        field.fix.branches = [VENDOR_DIALECT]
        field.fix.names = [f"VenueAlias{offset}"]
        fields.append(field)
    registry.add_fields(fields)
    return registry


TWO_DIALECTS = _vendor_registry()
TAGGED = Field("TradeID", "utf8")
TAGGED.fix.tag = 5001
TAGGED.fix.branches = [VENDOR_DIALECT]
SYMBOL_ID = SEED_REGISTRY.field_by_tag(55).fix.id
assert SYMBOL_ID is not None
VENDOR_ID = TWO_DIALECTS.field_by_tag(5001).fix.id
assert VENDOR_ID is not None
ABSENT_ID = TAGGED.fix.id
assert ABSENT_ID is not None and SEED_REGISTRY.get_field_by_id(ABSENT_ID) is None


def _generated(root: pathlib.Path) -> pathlib.Path:
    """Write a registry of ``WIDE_FIELDS`` fields and answer its folder."""
    fields = []
    for tag in range(1, WIDE_FIELDS + 1):
        field = Field(f"Field{tag}", "utf8")
        field.fix.tag = tag
        field.fix.names = [f"Alias{tag}"]
        fields.append(field)
    FixRegistry.from_fields(fields).write_into(root)
    return root


def _tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(55)


def _alternate_tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(20)


def _id_hit() -> object:
    return SEED_REGISTRY.get_field_by_id(SYMBOL_ID)


def _name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("Symbol")


def _folded_name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("symbol")


def _alias_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("ticker")


def _tag_miss() -> object:
    return SEED_REGISTRY.get_field_by_tag(9999)


def _name_miss() -> object:
    return SEED_REGISTRY.get_field_by_name("Nope")


def _id_miss() -> object:
    return SEED_REGISTRY.get_field_by_id(ABSENT_ID)


def _generic_tag_hit() -> object:
    return SEED_REGISTRY.get_field(55)


def _path_one_segment() -> object:
    return SEED_REGISTRY.field_by_path("NoPartyIDs")


def _path_two_segments() -> object:
    return SEED_REGISTRY.field_by_path("Parties.PartyID")


def _vendor_id_hit() -> object:
    return TWO_DIALECTS.get_field_by_id(VENDOR_ID)


def _vendor_name_hit() -> object:
    return TWO_DIALECTS.get_field_by_name("Venue1")


def _vendor_alias_hit() -> object:
    return TWO_DIALECTS.get_field_by_name("venuealias1")


def _vendor_tag_hit() -> object:
    return TWO_DIALECTS.get_field_by_tag(5001)


def _standard_hit_over_two_dialects() -> object:
    return TWO_DIALECTS.get_field_by_tag(55)


def _field_branches() -> object:
    return TAGGED.fix.branches


def _field_has_branch() -> object:
    return TAGGED.fix.has_branch(VENDOR_DIALECT)


def _field_id() -> object:
    return TAGGED.fix.id


def _registry_dialects() -> object:
    return TWO_DIALECTS.dialects()


def _message_get_by_tag() -> object:
    return MESSAGE.get_by_tag(55)


def _message_get_by_id() -> object:
    return MESSAGE.get_by_id(SYMBOL_ID)


def _message_get_by_name() -> object:
    return MESSAGE.get_by_name("Symbol")


def _message_get_by_path() -> object:
    return MESSAGE.get_by_path("Parties[0].PartyID")


def _infer_fixml_protocol() -> object:
    return MimeType.infer_bytes(FIXML_LINE)


def _infer_ullink_msgtype() -> object:
    return FixCodec.infer_msgtype_text(ULLINK_LINE)



def _catalog() -> FixRegistry:
    counter = Field("NoPartyIDs", "int32")
    counter.fix.tag = 453
    member = Field("PartyID", "utf8")
    member.fix.tag = 448
    registry = FixRegistry.from_fields([counter, member])
    member.fix.field_ref = "PartyID"
    # One door files each definition by the shape it has: a Struct is a
    # component, a List of Structs a group.
    registry.insert(Field("Party", DataType.from_fields([member]), nullable=False))
    component = registry.field_by_name("Party")
    group = types.list("Parties", component)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.insert(group)
    group = registry.field_by_name("Parties")
    group.fix.group = "Parties"
    counter.fix.field_ref = "NoPartyIDs"
    message = Field("NewOrderSingle", DataType.from_fields([counter, group]), nullable=False)
    message.fix.msgtype = "D"
    registry.insert(message)
    return registry


CATALOG = _catalog()
CATALOG_JSON = CATALOG.into_json()
CATALOG_PICKLE = pickle.dumps(CATALOG)
CODEC = FixCodec(SEED_REGISTRY)
NUMERIC_GROUP = b"8=FIX.4.4|35=D|453=1|448=BROKER|447=D|452=1|10=0|"
assert CODEC.parse_fix_line(NUMERIC_GROUP).by_tag(453).as_py() == 1
assert CODEC.parse_fix_line(NUMERIC_GROUP).by_path("Parties[0].PartyID").as_py() == "BROKER"
assert FixRegistry.from_json(CATALOG_JSON) == CATALOG
assert pickle.loads(CATALOG_PICKLE) == CATALOG

# The stream and Arrow doors, over a capture of wide orders: what is measured
# is the crossing - one pull per line, one batch per pull - beside the parse.
FIXED_SCHEMA = fix_schema(SEED_REGISTRY)
LINES = [b"8=FIX.4.4|35=D|11=ORDER-%06d|55=AAPL|54=1|38=100|58=%s|10=0|" % (index, b"x" * 200) for index in range(256)]
CAPTURE = pa.table({"body": pa.array(LINES, pa.binary())})
PARSED = next(CODEC.parse_line(LINES[0]))
PARSED_ROW = PARSED.into_row(FIXED_SCHEMA)
PARSED_BATCH = CODEC.parse_text_arrow_reader(CAPTURE).read_all()
assert PARSED_BATCH.num_rows == len(LINES)
assert len(list(CODEC.parse_lines(LINES))) == len(LINES)
ORDER_TYPE = SEED_REGISTRY.msgtype("D")
ORDER_DECLARATION = ORDER_TYPE.field
# A parse fills what the line implied, so the identifiers are on the message
# the parse answered rather than behind a pass of its own.
assert PARSED.identifiers == {"clordid": "ORDER-000000"}
assert [field.name for field, _ in ORDER_TYPE.identifier_values(PARSED)] == ["clordid"]
WALKED = next(iter(CODEC.lifecycle([PARSED])))

# The record door with a `msgpluginid` capture on every row: the capture
# fills the crate's `msgpluginid` field and selects nothing, so this is what
# a row costs with one more fill beside the parse.
MSGPLUGINID_CODEC = FixCodec(SEED_REGISTRY, capture_names=["msgpluginid"])
MSGPLUGINID_LINES = [
    TextLine(index, line, ["ULB" if index % 2 == 0 else "OMS_X1_TradeCapture"])
    for index, line in enumerate(LINES)
]
assert len(list(MSGPLUGINID_CODEC.parse_text_lines(MSGPLUGINID_LINES))) == len(LINES)


def _parse_lines_drain() -> int:
    return sum(1 for _ in CODEC.parse_lines(LINES))


def _parse_text_lines_drain() -> int:
    return sum(
        1 for _ in CODEC.parse_text_lines(TextLine(index, line) for index, line in enumerate(LINES))
    )


def _parse_text_lines_msgpluginid_drain() -> int:
    return sum(1 for _ in MSGPLUGINID_CODEC.parse_text_lines(MSGPLUGINID_LINES))


def _parse_text_arrow_reader() -> int:
    return CODEC.parse_text_arrow_reader(CAPTURE).read_all().num_rows


def _field_identifiers() -> object:
    return ORDER_DECLARATION.fix.identifiers


def _identifier_values() -> object:
    return ORDER_TYPE.identifier_values(PARSED)


def _identifiers_map() -> object:
    return PARSED.identifiers


def _event_facts() -> object:
    return PARSED.event()


def _header_facts() -> object:
    return PARSED.header()


def _message_entries() -> object:
    return PARSED.entries()


def _arrow_reader_with_identifiers() -> int:
    return CODEC.arrow_reader(FIXED_SCHEMA, (PARSED for _ in LINES)).read_all().num_rows


def _lifecycle_drain() -> int:
    return sum(1 for _ in CODEC.lifecycle(PARSED for _ in LINES))


def _lifecycle_arrow_reader() -> int:
    return CODEC.lifecycle_arrow_reader(PARSED_BATCH).read_all().num_rows


def _messages_drain() -> int:
    return sum(1 for _ in CODEC.messages(PARSED_BATCH))


def _arrow_reader_over_lines() -> int:
    return CODEC.arrow_reader(FIXED_SCHEMA, CODEC.parse_lines(LINES)).read_all().num_rows


def _write_arrow_reader() -> int:
    return CODEC.write_arrow_reader(PARSED_BATCH, io.BytesIO())


def _message_set() -> None:
    copy.copy(PARSED).set(55, "MSFT")


def _message_remove() -> object:
    return copy.copy(PARSED).remove(55)


def _message_from_row() -> object:
    return FixMsg.from_row(FIXED_SCHEMA, PARSED_ROW, SEED_REGISTRY)



def _definition_mutation(operation: str) -> FixRegistry:
    registry = copy.copy(CATALOG)
    if operation == "insert":
        message = Field("OrderCancel", DataType.from_fields([]), nullable=False)
        message.fix.msgtype = "F"
        registry.insert(message)
    elif operation == "remove":
        registry.remove("NewOrderSingle")
    else:
        component = registry.field_by_name("Party")
        component.fix.description = "Reviewed"
        registry.update(component)
    return registry


# The lenient field verb, both answers: a name folding to a stored one merges
# into it, a name nothing answers to arrives.
FOLDING_FIELD = Field("party_id", "utf8")
FOLDING_FIELD.fix.tag = 9001
ARRIVING_FIELD = Field("Symbol", "utf8")
ARRIVING_FIELD.fix.tag = 55
assert copy.copy(CATALOG).add_field(FOLDING_FIELD) is False
assert copy.copy(CATALOG).add_field(ARRIVING_FIELD) is True


def _add_field(field: Field) -> FixRegistry:
    registry = copy.copy(CATALOG)
    registry.add_field(field)
    return registry


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=3)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:36} {nanoseconds:14.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=2_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-bench-fix-"))
    gc.disable()
    try:
        generated = _generated(workspace / "generated")
        _measure("tag hit", _tag_hit, args.iterations)
        _measure("alternate tag hit", _alternate_tag_hit, args.iterations)
        _measure("id hit", _id_hit, args.iterations)
        _measure("name hit", _name_hit, args.iterations)
        _measure("name hit, folded query", _folded_name_hit, args.iterations)
        _measure("alias hit", _alias_hit, args.iterations)
        _measure("tag miss", _tag_miss, args.iterations)
        _measure("name miss", _name_miss, args.iterations)
        _measure("id miss", _id_miss, args.iterations)
        _measure("generic tag hit", _generic_tag_hit, args.iterations)
        _measure("field_by_path, one segment", _path_one_segment, args.iterations)
        _measure("field_by_path, two segments", _path_two_segments, args.iterations)
        _measure("vendor id hit, two dialects", _vendor_id_hit, args.iterations)
        _measure("vendor name hit, two dialects", _vendor_name_hit, args.iterations)
        _measure("vendor alias hit, two dialects", _vendor_alias_hit, args.iterations)
        _measure("vendor tag hit", _vendor_tag_hit, args.iterations)
        _measure(
            "standard tag hit, two dialects",
            _standard_hit_over_two_dialects,
            args.iterations,
        )
        _measure("field.fix.branches", _field_branches, args.iterations)
        _measure("field.fix.has_branch", _field_has_branch, args.iterations)
        _measure("field.fix.id", _field_id, args.iterations)
        _measure("registry dialects", _registry_dialects, args.iterations)
        _measure("message get_by_tag", _message_get_by_tag, args.iterations)
        _measure("message get_by_id", _message_get_by_id, args.iterations)
        _measure("message get_by_name", _message_get_by_name, args.iterations)
        _measure("message get_by_path", _message_get_by_path, args.iterations)
        _measure("infer FIXML protocol", _infer_fixml_protocol, args.iterations)
        _measure("infer Ullink MsgType", _infer_ullink_msgtype, args.iterations)
        _measure("field iterator first", lambda: next(iter(SEED_REGISTRY)), args.iterations)
        _measure("field iterator drain", lambda: list(SEED_REGISTRY), max(1, args.iterations // 50))
        _measure("component lookup", lambda: SEED_REGISTRY.field_by_name("Party"), args.iterations)
        _measure("group lookup", lambda: SEED_REGISTRY.field_by_name("Parties"), args.iterations)
        _measure("group by counter", lambda: SEED_REGISTRY.field_by_counter(453), args.iterations)
        _measure("message singleton lookup", lambda: SEED_REGISTRY.msgtype("D"), args.iterations)
        _measure("catalog snapshot encode", CATALOG.into_json, args.iterations)
        _measure("catalog snapshot decode", lambda: FixRegistry.from_json(CATALOG_JSON), args.iterations)
        _measure("catalog pickle encode", lambda: pickle.dumps(CATALOG), args.iterations)
        _measure("catalog pickle decode", lambda: pickle.loads(CATALOG_PICKLE), args.iterations)
        _measure("catalog stable hash", CATALOG.stable_hash, args.iterations)
        _measure("catalog copy baseline", lambda: copy.copy(CATALOG), args.iterations)
        for operation in ("insert", "update", "remove"):
            _measure(f"definition {operation} including copy", lambda operation=operation: _definition_mutation(operation), args.iterations)
        _measure("catalog add_field merging including copy", lambda: _add_field(FOLDING_FIELD), args.iterations)
        _measure("catalog add_field arriving including copy", lambda: _add_field(ARRIVING_FIELD), args.iterations)
        singleton = CATALOG.msgtype("D")
        _measure("singleton stable hash", singleton.stable_hash, args.iterations)
        _measure("singleton field view", lambda: singleton.field, args.iterations)

        _measure("numeric group, compiled plan", lambda: CODEC.parse_fix_line(NUMERIC_GROUP), args.iterations)
        _measure("message set", _message_set, args.iterations)
        _measure("message remove", _message_remove, args.iterations)
        _measure("message from_row", _message_from_row, args.iterations)
        _measure("field.fix.identifiers", _field_identifiers, args.iterations)
        _measure("MsgType.identifier_values", _identifier_values, args.iterations)
        _measure("identifiers native map crossing", _identifiers_map, args.iterations)
        _measure("message event holder", _event_facts, args.iterations)
        _measure("message header holder", _header_facts, args.iterations)
        _measure("message entries", _message_entries, args.iterations)
        streams = max(1, args.iterations // 50)
        _measure(f"arrow_reader with identifiers/{len(LINES)}", _arrow_reader_with_identifiers, streams)
        _measure(f"parse_lines drain/{len(LINES)}", _parse_lines_drain, streams)
        _measure(f"parse_text_lines drain/{len(LINES)}", _parse_text_lines_drain, streams)
        _measure(
            f"parse_text_lines msgpluginid drain/{len(LINES)}",
            _parse_text_lines_msgpluginid_drain,
            streams,
        )
        _measure(f"parse_text_arrow_reader/{len(LINES)}", _parse_text_arrow_reader, streams)
        _measure(f"lifecycle drain/{len(LINES)}", _lifecycle_drain, streams)
        _measure(f"lifecycle_arrow_reader/{len(LINES)}", _lifecycle_arrow_reader, streams)
        _measure(f"messages drain/{len(LINES)}", _messages_drain, streams)
        _measure(f"arrow_reader over parse_lines/{len(LINES)}", _arrow_reader_over_lines, streams)
        _measure(f"write_arrow_reader/{len(LINES)}", _write_arrow_reader, streams)
        loads = max(1, args.iterations // 100)
        _measure(
            "from_handle, the seed",
            lambda: FixRegistry.from_handle(SEED),
            loads,
        )
        _measure(
            f"from_handle, {WIDE_FIELDS} fields",
            lambda: FixRegistry.from_handle(generated),
            loads,
        )
    finally:
        gc.enable()
        shutil.rmtree(workspace, ignore_errors=True)


if __name__ == "__main__":
    main()
