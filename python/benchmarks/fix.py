"""Boundary cost of the FIX dictionary, against the native numbers.

Every case here is one crossing over a registry the core resolves: what is
measured is the coercion of the key - a tag, a branch, an identifier - the
wrapper the answer is put in, and - for the two loads - the shard read the
boundary only names. Run after installing the release wheel with::

    python benchmarks/fix.py --iterations 2000

The generated registry is written to a temporary folder and removed on the way
out, so the only tracked input is the seed dictionary at ``config/fix``.
"""

from __future__ import annotations

import argparse
import copy
import io
import json
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
from yggdryl.fix import STANDARD_BRANCH, ULBRIDGE_BRANCH, FixBranch, FixCodec, FixMsg, FixRegistry, UlPlugin, fix_schema

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
SEED = REPO / "config" / "fix"

SEED_REGISTRY = FixRegistry.from_handle(SEED)
ORDER = Field(
    "NewOrderSingle",
    DataType.from_fields(
        [
            SEED_REGISTRY.field_by_tag(55),
            SEED_REGISTRY.field_by_tag(38),
            SEED_REGISTRY.field_by_name("NoPartyIDs", STANDARD_BRANCH),
            SEED_REGISTRY.definition("groups", "Parties"),
        ]
    ),
    nullable=False,
)
MESSAGE = FixMsg(
    ORDER,
    {
        "symbol": "AAPL",
        "orderqty": 100.0,
        "nopartyids": 1,
        "parties": [
            {"partyid": "BROKER", "partyidsource": "D", "partyrole": 1}
        ],
    },
    SEED_REGISTRY,
)
WIDE_FIELDS = 200
VENDOR_BRANCH = "cme"
VENDOR_FIELDS = 200
FIXML_LINE = b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|"
ULLINK_LINE = "ACCOUNT=A1|MSGTYPE=D|SYMBOL=AAPL"


def _vendor_registry() -> FixRegistry:
    """The seed beside a vendor dictionary, for the cross-branch rows."""
    registry = copy.copy(SEED_REGISTRY)
    fields = []
    for offset in range(VENDOR_FIELDS):
        tag = 5000 + offset
        field = Field(f"Venue{offset}", "utf8")
        field.fix.id = f"{tag}:{VENDOR_BRANCH}"
        field.fix.aliases = [f"VenueAlias{offset}"]
        fields.append(field)
    registry.add_fields(fields)
    return registry


TWO_BRANCHES = _vendor_registry()
TAGGED = Field("TradeID", "utf8")
TAGGED.fix.id = f"5001:{VENDOR_BRANCH}"


def _generated(root: pathlib.Path) -> pathlib.Path:
    """Write a registry of ``WIDE_FIELDS`` fields and answer its folder."""
    fields = []
    for tag in range(1, WIDE_FIELDS + 1):
        field = Field(f"Field{tag}", "utf8")
        field.fix.tag = tag
        field.fix.aliases = [f"Alias{tag}"]
        fields.append(field)
    FixRegistry.from_fields(fields).write_into(root)
    return root


def _tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(55)


def _alternate_tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(20)


def _id_hit() -> object:
    return SEED_REGISTRY.get_field_by_id("55:")


def _name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("Symbol", STANDARD_BRANCH)


def _folded_name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("symbol", STANDARD_BRANCH)


def _alias_hit() -> object:
    return SEED_REGISTRY.get_field_by_name("ticker", STANDARD_BRANCH)


def _tag_miss() -> object:
    return SEED_REGISTRY.get_field_by_tag(9999)


def _name_miss() -> object:
    return SEED_REGISTRY.get_field_by_name("Nope", STANDARD_BRANCH)


def _id_miss() -> object:
    return SEED_REGISTRY.get_field_by_id("5001:cme")


def _generic_tag_hit() -> object:
    return SEED_REGISTRY.get_field(55)


def _path_one_segment() -> object:
    return SEED_REGISTRY.field_by_path("NoPartyIDs", STANDARD_BRANCH)


def _path_two_segments() -> object:
    return SEED_REGISTRY.field_by_path("Parties.PartyID", STANDARD_BRANCH)


def _vendor_id_hit() -> object:
    return TWO_BRANCHES.get_field_by_id("5001:cme")


def _vendor_name_hit() -> object:
    return TWO_BRANCHES.get_field_by_name("Venue1", VENDOR_BRANCH)


def _vendor_alias_hit() -> object:
    return TWO_BRANCHES.get_field_by_name("venuealias1", VENDOR_BRANCH)


def _vendor_tag_hit_inferred() -> object:
    return TWO_BRANCHES.get_field_by_tag(5001)


def _standard_hit_over_two_branches() -> object:
    return TWO_BRANCHES.get_field_by_tag(55)


def _field_branch() -> object:
    return TAGGED.fix.branch


def _field_id() -> object:
    return TAGGED.fix.id


def _message_get_by_tag() -> object:
    return MESSAGE.get_by_tag(55)


def _message_get_by_id() -> object:
    return MESSAGE.get_by_id("55:")


def _message_get_by_name() -> object:
    return MESSAGE.get_by_name("Symbol")


def _message_get_by_path() -> object:
    return MESSAGE.get_by_path("Parties[0].PartyID")


def _message_branch() -> object:
    return MESSAGE.branch


def _message_into_latest() -> object:
    return MESSAGE.into_latest()


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
    component = Field("Party", DataType.from_fields([member]), nullable=False)
    registry.create_definition("components", component)
    group = types.list("Parties", component)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.create_definition("groups", group)
    group = registry.definition("groups", "Parties")
    group.fix.group = "Parties"
    counter.fix.field_ref = "NoPartyIDs"
    message = Field("NewOrderSingle", DataType.from_fields([counter, group]), nullable=False)
    message.fix.msgtype = "D"
    registry.create_definition("messages", message)
    return registry


CATALOG = _catalog()
CATALOG_JSON = CATALOG.into_json()
CATALOG_PICKLE = pickle.dumps(CATALOG)
CODEC = FixCodec(SEED_REGISTRY)
BRIDGE_REGISTRY = copy.copy(SEED_REGISTRY)
BRIDGE_REGISTRY.with_ulbridge_fields()
BRIDGE_CODEC = FixCodec(BRIDGE_REGISTRY, branch="ulbridge")
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

# The record door with a dialect each row names for itself: every other row
# spells an alias of the codec's own branch, the rest a plugin no branch is
# named after, which keeps the pin. The alias is declared on a copy of the
# bridge dictionary so the cases above keep their setup. A plugin's dialect
# resolves off the codec's memo after the first row spelling it, so this is
# what a row costs read under a dialect it names for itself.
PLUGIN_REGISTRY = copy.copy(BRIDGE_REGISTRY)
_DECLARED = PLUGIN_REGISTRY.branch_named(ULBRIDGE_BRANCH)
assert _DECLARED is not None
PLUGIN_REGISTRY.set_branch(
    FixBranch(_DECLARED.name, version=_DECLARED.version, aliases=[*_DECLARED.aliases, "ulb"])
)
PLUGIN_CODEC = FixCodec(PLUGIN_REGISTRY, branch=ULBRIDGE_BRANCH, capture_names=["pluginid"])
PLUGIN_LINES = [
    TextLine(index, line, ["ULB" if index % 2 == 0 else "OMS_X1_TradeCapture"])
    for index, line in enumerate(LINES)
]
assert len(list(PLUGIN_CODEC.parse_text_lines(PLUGIN_LINES))) == len(LINES)


def _parse_lines_drain() -> int:
    return sum(1 for _ in CODEC.parse_lines(LINES))


def _parse_text_lines_drain() -> int:
    return sum(
        1 for _ in CODEC.parse_text_lines(TextLine(index, line) for index, line in enumerate(LINES))
    )


def _parse_text_lines_pluginid_drain() -> int:
    return sum(1 for _ in PLUGIN_CODEC.parse_text_lines(PLUGIN_LINES))


def _parse_text_arrow_reader() -> int:
    return CODEC.parse_text_arrow_reader(CAPTURE).read_all().num_rows


def _enrich_messages_arrow_reader() -> int:
    return CODEC.enrich_messages_arrow_reader(PARSED_BATCH).read_all().num_rows


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



def _category_mutation(operation: str) -> FixRegistry:
    registry = copy.copy(CATALOG)
    if operation == "create":
        message = Field("OrderCancel", DataType.from_fields([]), nullable=False)
        message.fix.msgtype = "F"
        registry.create_definition("messages", message)
    elif operation == "remove":
        registry.remove_definition("messages", "NewOrderSingle")
    else:
        component = registry.definition("components", "Party")
        component.fix.description = "Reviewed"
        if operation == "update":
            registry.update_definition("components", component)
        elif operation == "add":
            registry.add_definition("components", component)
        else:
            registry.insert_definition("components", component)
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


def _register_bridge_vocabulary() -> FixRegistry:
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    return registry

def _wildcard(count: int) -> bytes:
    return json.dumps({
        "request": {"mbean": "com.ullink.ulbridge.sessioninterfaces.plugins:*", "type": "read"},
        "value": {f"com.ullink.ulbridge.sessioninterfaces.plugins:name=Item{index:04},type=Plugin": {"Name": f"Item{index:04}", "CurrentPort": 9000 + index} for index in range(count)},
        "status": 200,
    }).encode()

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
        _measure("vendor id hit, two branches", _vendor_id_hit, args.iterations)
        _measure("vendor name hit, two branches", _vendor_name_hit, args.iterations)
        _measure("vendor alias hit, two branches", _vendor_alias_hit, args.iterations)
        _measure("inferred vendor tag hit", _vendor_tag_hit_inferred, args.iterations)
        _measure(
            "standard tag hit, two branches",
            _standard_hit_over_two_branches,
            args.iterations,
        )
        _measure("field.fix.branch", _field_branch, args.iterations)
        _measure("field.fix.id", _field_id, args.iterations)
        _measure("message get_by_tag", _message_get_by_tag, args.iterations)
        _measure("message get_by_id", _message_get_by_id, args.iterations)
        _measure("message get_by_name", _message_get_by_name, args.iterations)
        _measure("message get_by_path", _message_get_by_path, args.iterations)
        _measure("message branch", _message_branch, args.iterations)
        _measure("message into_latest", _message_into_latest, args.iterations)
        _measure("infer FIXML protocol", _infer_fixml_protocol, args.iterations)
        _measure("infer Ullink MsgType", _infer_ullink_msgtype, args.iterations)
        for category in ("fields", "components", "groups", "messages"):
            _measure(f"{category} iterator first", lambda category=category: next(SEED_REGISTRY.definitions(category)), args.iterations)
            _measure(f"{category} iterator drain", lambda category=category: list(SEED_REGISTRY.definitions(category)), max(1, args.iterations // 50))
        _measure("category group lookup", lambda: SEED_REGISTRY.definition("groups", "Parties"), args.iterations)
        _measure("message singleton lookup", lambda: SEED_REGISTRY.msgtype("D"), args.iterations)
        _measure("message singleton first", lambda: next(SEED_REGISTRY.msgtypes()), args.iterations)
        _measure("message singleton drain", lambda: list(SEED_REGISTRY.msgtypes()), max(1, args.iterations // 50))
        _measure("catalog snapshot encode", CATALOG.into_json, args.iterations)
        _measure("catalog snapshot decode", lambda: FixRegistry.from_json(CATALOG_JSON), args.iterations)
        _measure("catalog pickle encode", lambda: pickle.dumps(CATALOG), args.iterations)
        _measure("catalog pickle decode", lambda: pickle.loads(CATALOG_PICKLE), args.iterations)
        _measure("catalog stable hash", CATALOG.stable_hash, args.iterations)
        _measure("catalog copy baseline", lambda: copy.copy(CATALOG), args.iterations)
        for operation in ("create", "insert", "update", "add", "remove"):
            _measure(f"catalog {operation} including copy", lambda operation=operation: _category_mutation(operation), args.iterations)
        _measure("catalog add_field merging including copy", lambda: _add_field(FOLDING_FIELD), args.iterations)
        _measure("catalog add_field arriving including copy", lambda: _add_field(ARRIVING_FIELD), args.iterations)
        _measure("register bridge vocabulary", _register_bridge_vocabulary, args.iterations)
        singleton = CATALOG.msgtype("D")
        _measure("singleton stable hash", singleton.stable_hash, args.iterations)
        _measure("singleton field view", lambda: singleton.field, args.iterations)

        _measure("numeric group, compiled plan", lambda: CODEC.parse_fix_line(NUMERIC_GROUP), args.iterations)
        _measure("message set", _message_set, args.iterations)
        _measure("message remove", _message_remove, args.iterations)
        _measure("message from_row", _message_from_row, args.iterations)
        streams = max(1, args.iterations // 50)
        _measure(f"parse_lines drain/{len(LINES)}", _parse_lines_drain, streams)
        _measure(f"parse_text_lines drain/{len(LINES)}", _parse_text_lines_drain, streams)
        _measure(
            f"parse_text_lines pluginid drain/{len(LINES)}",
            _parse_text_lines_pluginid_drain,
            streams,
        )
        _measure(f"parse_text_arrow_reader/{len(LINES)}", _parse_text_arrow_reader, streams)
        _measure(f"enrich_messages_arrow_reader/{len(LINES)}", _enrich_messages_arrow_reader, streams)
        _measure(f"messages drain/{len(LINES)}", _messages_drain, streams)
        _measure(f"arrow_reader over parse_lines/{len(LINES)}", _arrow_reader_over_lines, streams)
        _measure(f"write_arrow_reader/{len(LINES)}", _write_arrow_reader, streams)
        for count in (1, 32, 64):
            body = _wildcard(count)
            assert len(list(UlPlugin.from_json_bytes(body))) == count
            assert len(list(BRIDGE_CODEC.parse_line(body))) == count
            _measure(f"UlPlugins first/{count}", lambda body=body: next(UlPlugin.from_json_bytes(body)), args.iterations)
            _measure(f"UlPlugins drain/{count}", lambda body=body: list(UlPlugin.from_json_bytes(body)), args.iterations)
            _measure(f"FixMessages first/{count}", lambda body=body: next(BRIDGE_CODEC.parse_line(body)), args.iterations)
            _measure(f"FixMessages drain/{count}", lambda body=body: list(BRIDGE_CODEC.parse_line(body)), args.iterations)
            _measure(f"line messages drain/{count}", lambda body=body: list(BRIDGE_CODEC.parse_text_line(TextLine(0, body))), args.iterations)
            first = next(UlPlugin.from_json_bytes(body))
            _measure(f"UlPlugin hash/{count}", first.stable_hash, args.iterations)
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
