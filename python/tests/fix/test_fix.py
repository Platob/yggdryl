"""The FIX boundary: the typed vocabulary, the registry, and the message.

Every answer here is the core's; what these check is the crossing - the key
coercion, the exception each core failure maps to, the storage locations a
Python caller names, and the Python protocols the two wrappers implement.
An identifier crosses as the ``int`` the core derives from a tag and a name,
and a dictionary's contribution is a list of names on the field, so neither
has a class of its own and every refusal is the native one.
"""

from __future__ import annotations

import copy
import datetime as dt
import decimal
import json
import logging
import pathlib
import pickle
import subprocess
import sys
from typing import Any, Iterable, Iterator

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase, MimeType, Scalar, TextLine, Url, refresh_logging
from yggdryl.fix import (
    FixMsg,
    FixMessages,
    MsgType,
    Plugins,
    FixCodec,
    FixLifecycle,
    FixRegistry,
    PLUGIN_DIALECT,
    PLUGINCONFIG_CODE_NAME,
    Plugin,
    fix_cfb_fields,
    fix_crate_fields,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
    fix_plugin_fields,
    fix_plugin_message,
    global_registry,
    install_global_registry,
)

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"

# The crate's own scalar fields: tags 65001 to 65019 and 65021 to 65027, the
# retired 65000 never reused. ``fix_crate_fields`` also lists the ``altids``
# Map group at 65020, which registry length and scalar iteration never count
# (``rust/tests/fix/digest.rs``).
CRATED = 26
CRATE_TAGS = [*range(65001, 65020), *range(65021, 65028)]
# What ``FixRegistry()`` holds before anything is inserted: the crate's own
# scalar fields and the two seeded standard clocks, SendingTime (52) and
# TransactTime (60). A loaded dictionary states its own 52 and 60, so it holds
# its fields plus ``CRATED`` (``seeded_fields`` and ``crated_fields`` in
# ``rust/tests/fix.rs``).
SEEDED = CRATED + 2

# The one intake clock undated test bytes take, so a parse repeats; replay
# never consults now (``fixed_codec`` in ``rust/tests/fix.rs``).
CLOCK_NS = 1_704_190_530_000_000_000
CLOCK = Scalar.datetime(CLOCK_NS, "ns", "UTC")
CLOCK_INSTANT = dt.datetime(2024, 1, 2, 10, 15, 30, tzinfo=dt.timezone.utc)
# The hour ``CLOCK`` falls in, as the instant it is: 2024-01-02T10:00Z.
CLOCK_PARTITION = dt.datetime(2024, 1, 2, 10, 0, tzinfo=dt.timezone.utc)

# The seven members every message carries, non-null, in the order a message
# built without them appends them.
BUNDLE = ["updatedat", "createdat", "uuid", "puuid", "code", "snapshotat", "sendingtime"]
UPDATEDAT_TAG = 65003
INSTUUID_TAG = 65016
UUID_TAG = 65017
PUUID_TAG = 65018
PREVUPDATEDAT_TAG = 65021
PREVUUID_TAG = 65022
CREATEDAT_TAG = 65023
CODE_TAG = 65024
SNAPSHOTAT_TAG = 65025
SOURCEURL_TAG = 65026
NOFIXENTRIES_TAG = 65027


def _fixed(registry: FixRegistry, **pins: Any) -> FixCodec:
    """A codec whose undated messages all take ``CLOCK`` as their SendingTime."""
    return FixCodec(registry, default_sending_time=CLOCK, **pins)

# One Ullink CBlock in the shape a production file has: a vocabulary of a
# specification tag and a venue one, and a grammar binding whose root the
# registry form answers and the vocabulary form drops.
CBLOCK = """<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration version="1.2" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<vocabulary>
		<vocabulary-tag name="55" alt="Symbol" type="string">
			<description>Ticker symbol.</description>
		</vocabulary-tag>
		<vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />
	</vocabulary>
	<grammar-binding type="7">
		<grammar>
			<tag-constraint name="55" part="body" required="true" />
		</grammar>
	</grammar-binding>
</cplugin-configuration>
"""


def _field(
    name: str,
    dtype: str,
    tag: int,
    *,
    branches: Iterable[str] = (),
    tags: Iterable[int] = (),
    aliases: Iterable[str] = (),
    description: str | None = None,
    nullable: bool = True,
) -> Field:
    """One FIX field, written through the protocol view alone."""
    field = Field(name, dtype, nullable=nullable)
    field.fix.tag = tag
    if branches:
        field.fix.branches = branches
    if tags:
        field.fix.tags = tags
    if aliases:
        field.fix.aliases = aliases
    if description is not None:
        field.fix.description = description
    return field


@pytest.fixture(scope="module")
def _seed_catalog() -> FixRegistry:
    """The dictionary the repository tracks at ``config/fix``."""
    return FixRegistry.from_handle(SEED)


@pytest.fixture
def seed(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


def test_protocol_view_carries_the_typed_fix_vocabulary() -> None:
    field = Field("OrderQty", "decimal128(20, 8)")
    field.fix.tag = 38
    field.fix.tags = [1088]
    field.fix.aliases = ["Qty", "Quantity"]
    field.fix.description = "Quantity ordered."

    assert field.fix.tag == 38
    assert field.fix.tags == [1088]
    assert field.fix.aliases == ["Qty", "Quantity"]
    assert field.fix.description == "Quantity ordered."
    # Ordinary namespaced text, in the one metadata map.
    assert field.metadata["fix:aliases"] == "Qty,Quantity"
    assert field.fix["tag"] == "38"
    # Three, not four: a description is a fact about the column rather than a
    # FIX fact, so it lives on the generic key every catalog reads.
    assert field.metadata["description"] == "Quantity ordered."
    assert "fix:description" not in field.metadata
    assert len(field.fix) == 3

    # An empty list removes the property; `del` removes any of them.
    field.fix.tags = []
    assert field.fix.tags == []
    assert "tags" not in field.fix
    field.fix.aliases = ()
    assert field.fix.aliases == []
    del field.fix["tag"]
    assert field.fix.tag is None

    absent = Field("Symbol", "utf8")
    assert absent.fix.tag is None
    assert absent.fix.tags == []
    assert absent.fix.aliases == []
    assert absent.fix.description is None


def test_typed_vocabulary_is_only_on_the_fix_view() -> None:
    field = Field("Symbol", "utf8")
    field.fix.tag = 55

    for view, scheme in ((field.http, "http"), (field.iceberg, "iceberg")):
        with pytest.raises(TypeError, match=scheme):
            view.tag
        with pytest.raises(TypeError, match=scheme):
            view.aliases
        with pytest.raises(TypeError, match=scheme):
            view.branches
        with pytest.raises(TypeError, match=scheme):
            view.id
        with pytest.raises(TypeError, match=scheme):
            view.tag = 55
        with pytest.raises(TypeError, match=scheme):
            view.branches = ["cme"]
        with pytest.raises(TypeError, match=scheme):
            view.add_branch("cme")
        with pytest.raises(TypeError, match=scheme):
            view.has_branch("cme")
        with pytest.raises(TypeError, match=scheme):
            view.directions
        with pytest.raises(TypeError, match=scheme):
            view.directions = [{"code": "S", "patterns": ["^TX "]}]
        with pytest.raises(TypeError, match=scheme):
            view.derivation
        with pytest.raises(TypeError, match=scheme):
            view.derivation = "orderqty - cumqty"
    # The mapping protocol still works on every view, including this one.
    assert field.protocol("fix")["tag"] == "55"


def test_a_derivation_crosses_as_canonical_text() -> None:
    """One term over the message's fields, stored as its canonical
    spelling; None removes it, and a text that is not a term refuses."""
    field = Field("leavesqty", "float64")
    field.fix.tag = 151
    assert field.fix.derivation is None

    field.fix.derivation = "orderqty-cumqty"
    assert field.fix.derivation == "orderqty - cumqty"
    assert field.metadata["fix:derivation"] == "orderqty - cumqty"

    with pytest.raises(ValueError):
        field.fix.derivation = "orderqty -"
    assert field.fix.derivation == "orderqty - cumqty"

    # An edited derivation is what the reader fills by (decision 38).
    registry = FixRegistry.from_handle(SEED)
    leaves = registry.get_field_by_tag(151)
    leaves.fix.derivation = "case when msgtype in ('8', '9') then orderqty * 2 end"
    registry.update(leaves)
    codec = _fixed(registry)
    held = codec.enrich_message(next(codec.parse_line(b"8=FIX.4.4|35=8|37=A|38=100|14=0|10=0|")))
    assert held.by_tag(151).as_py() == 200.0

    field.fix.derivation = None
    assert field.fix.derivation is None
    assert "fix:derivation" not in field.metadata


def test_direction_rules_cross_as_a_list() -> None:
    """One record per code of the set, the patterns decoded, on tag 385."""
    field = Field("MsgDirection", "utf8")
    field.fix.tag = 385
    assert field.fix.directions == []

    rules = [
        {"code": "S", "patterns": ["(?i)^TX\\b"]},
        {"code": "R", "patterns": ["(?i)^RX\\b"]},
    ]
    field.fix.directions = rules
    assert field.fix.directions == rules
    # The stored text is the canonical document, backslashes escaped.
    assert field.metadata["fix:directions"] == (
        '{"directions":[{"code":"S","patterns":["(?i)^TX\\\\b"]},'
        '{"code":"R","patterns":["(?i)^RX\\\\b"]}]}'
    )
    assert json.loads(field.metadata["fix:directions"]) == {"directions": rules}

    # A codec compiles the rules of the dictionary it is built over, once,
    # and the line door fills tag 385 from them; the verb table no longer
    # applies under a stated table.
    registry = FixRegistry()
    registry.insert(field)
    codec = _fixed(registry)
    assert next(codec.parse_line(b"TX 8=FIX.4.4|35=D|10=0|")).by_tag(385).as_py() == "S"
    assert next(codec.parse_line(b"RX 8=FIX.4.4|35=D|10=0|")).by_tag(385).as_py() == "R"
    assert next(codec.parse_line(b"sending >> 8=FIX.4.4|35=D|10=0|")).get_by_tag(385) is None

    # A pattern the regex crate refuses is refused whole, the field unchanged.
    with pytest.raises(ValueError, match="valid byte regex"):
        field.fix.directions = [{"code": "S", "patterns": ["("]}]
    assert field.fix.directions == rules
    # A record is read as a mapping: a missing key is the mapping's own error.
    with pytest.raises(KeyError):
        field.fix.directions = [{"code": "S"}]
    assert field.fix.directions == rules

    # An empty iterable removes the property.
    field.fix.directions = []
    assert field.fix.directions == []
    assert "fix:directions" not in field.metadata
    assert Field("MsgDirection", "utf8").fix.directions == []


def test_tag_rejects_bool_and_refuses_to_narrow() -> None:
    field = Field("Symbol", "utf8")

    with pytest.raises(TypeError, match="not bool"):
        field.fix.tag = True
    with pytest.raises(OverflowError):
        field.fix.tag = 2**31
    with pytest.raises(TypeError, match="not bool"):
        field.fix.tags = [55, False]
    with pytest.raises(OverflowError):
        field.fix.tags = [2**31]
    # A refusal leaves the field untouched.
    assert field.fix.tag is None
    assert not field.fix

    with pytest.raises(ValueError, match="fix:tag"):
        field.fix.tag = -1
    with pytest.raises(ValueError, match="fix:tags"):
        field.fix.tags = [55, 55]
    with pytest.raises(ValueError, match="fix:aliases"):
        field.fix.aliases = ["Sym", "sym"]


def test_a_tag_counter_and_alternate_are_positive_and_a_refusal_writes_nothing() -> None:
    """Tag 0 is what an unresolved arrival records, never a field's identity.

    Pinned by ``registry_tag_writers_refuse_nonpositive_values_atomically`` and
    ``externally_stated_zero_identity_is_refused_without_mutating_the_registry``
    in ``rust/tests/fix/zero_entries.rs``.
    """
    field = Field("positive", "utf8")
    field.fix.tag = 1
    field.fix.counter = 2
    field.fix.tags = [3, 2**31 - 1]
    before = field.into_json()
    for tag in (0, -1, -(2**31)):
        for key, write in (
            ("fix:tag", lambda: setattr(field.fix, "tag", tag)),
            ("fix:counter", lambda: setattr(field.fix, "counter", tag)),
            ("fix:tags", lambda: setattr(field.fix, "tags", [4, tag])),
        ):
            with pytest.raises(ValueError, match=key) as refused:
                write()
            assert "from 1 to 2147483647" in str(refused.value), (key, tag)
            assert field.into_json() == before, (key, tag)
    field.fix.tag = 2**31 - 1
    field.fix.counter = 2**31 - 1
    assert field.fix.tag == 2**31 - 1
    assert field.fix.counter == 2**31 - 1
    field.fix.tags = []
    assert "fix:tags" not in field.metadata

    # A stored spelling reads the same way: zero, a sign or a value past `i32`
    # is refused where it is read, and a registry refuses to take the field.
    for key in ("fix:tag", "fix:counter", "fix:tags"):
        for text in ("0", "000", "-1", "+1", "2147483648"):
            incoming = Field("incoming", "utf8", metadata={"fix:tag": "90001"})
            incoming.metadata[key] = text
            with pytest.raises(ValueError, match=key):
                getattr(incoming.fix, key.removeprefix("fix:"))
            registry = FixRegistry()
            snapshot = registry.into_json()
            with pytest.raises(ValueError):
                registry.insert(incoming)
            assert registry.into_json() == snapshot, (key, text)
    padded = Field(
        "positive",
        "utf8",
        metadata={"fix:tag": "0001", "fix:counter": "0001", "fix:tags": "0001"},
    )
    assert (padded.fix.tag, padded.fix.counter, padded.fix.tags) == (1, 1, [1])


def test_id_is_the_tag_under_the_name_and_never_stored() -> None:
    trade = Field("TradeID", "utf8")
    # There is no identity without a tag.
    assert trade.fix.id is None
    trade.fix.tag = 5001
    held = trade.fix.id
    assert isinstance(held, int) and not isinstance(held, bool)
    assert -(2**31) <= held < 2**31
    # Derived on every read from `fix:tag` and the name, so nothing stores it
    # and a rename is never stale.
    assert "fix:id" not in trade.metadata
    assert set(trade.fix) == {"tag"}
    trade.fix.tag = 5002
    assert trade.fix.id != held
    trade.fix.tag = 5001
    assert trade.fix.id == held
    renamed = copy.copy(trade)
    renamed.set_name("TradeRef")
    assert renamed.fix.id != held

    # One fold: ASCII case, `_`, `-` and space are not part of the name.
    for spelling in ("MsgType", "msgtype", "MSGTYPE", "Msg_Type", "msg-type", "Msg Type"):
        field = Field(spelling, "utf8")
        field.fix.tag = 35
        assert field.fix.id == Field("MsgType", "utf8", metadata={"fix:tag": "35"}).fix.id, spelling
    # Membership is not identity: two dictionaries speaking one field share it.
    stamped = _field("MsgType", "utf8", 35, branches=["cme", "ice"])
    assert stamped.fix.id == _field("MsgType", "utf8", 35).fix.id

    # Read-only: the id is what the tag and the name say, and nothing else
    # can say it.
    with pytest.raises(AttributeError):
        trade.fix.id = 7  # type: ignore[misc]
    assert trade.fix.id == held

    # Nothing gates a tag on a dictionary any more: any positive tag. Zero is
    # the unresolved arrival's, and refused.
    for tag in (1, 35, 4999, 5000, 10_000, 40_000, 2**31 - 1):
        field = Field("Venue", "utf8")
        field.fix.tag = tag
        assert field.fix.tag == tag and field.fix.id is not None, tag

    # The registry answers the same integer, exactly: no alias, alternate
    # tag or fold is consulted.
    registry = FixRegistry.from_fields([trade, _field("Symbol", "utf8", 55, tags=[65], aliases=["Ticker"])])
    assert registry.field_by_id(held).name == "TradeID"
    assert registry.field_by_tag(55).fix.id == _field("symbol", "utf8", 55).fix.id
    assert registry.get_field_by_id(registry.field_by_tag(55).fix.id) == registry.field_by_tag(55)
    assert registry.get_field_by_id(_field("Ticker", "utf8", 55).fix.id) is None
    assert registry.get_field_by_id(_field("Symbol", "utf8", 65).fix.id) is None
    for field in registry:
        assert registry.field_by_id(field.fix.id) == field, field.name


def test_membership_round_trips_as_a_sorted_list() -> None:
    trade = Field("TradeID", "utf8")
    # An absent property is an empty list, and there is no key behind it.
    assert trade.fix.branches == []
    assert not trade.fix.has_branch("cme")
    assert "fix:branches" not in trade.metadata

    # Folded to ASCII lowercase, deduplicated under the fold, sorted, and
    # stored comma-joined under the one key.
    trade.fix.branches = ["ICE", "cme", "Cme", "bloomberg"]
    assert trade.fix.branches == ["bloomberg", "cme", "ice"]
    assert trade.metadata["fix:branches"] == "bloomberg,cme,ice"
    assert trade.fix.has_branch("CME") and trade.fix.has_branch("ice")
    assert not trade.fix.has_branch("morgan")
    # Any iterable of names, and a tuple is one.
    trade.fix.branches = ("cme",)
    assert trade.fix.branches == ["cme"]

    # `add_branch` is idempotent under the fold and keeps the list sorted.
    trade.fix.add_branch("Bloomberg")
    trade.fix.add_branch("CME")
    assert trade.fix.branches == ["bloomberg", "cme"]

    # An empty iterable removes the property, the way every list does.
    trade.fix.branches = []
    assert trade.fix.branches == []
    assert "fix:branches" not in trade.metadata

    # A name is held to the alias grammar - non-empty, no separator - and a
    # refusal writes nothing.
    trade.fix.branches = ["cme"]
    with pytest.raises(ValueError, match="fix:branches"):
        trade.fix.branches = ["cme", ""]
    with pytest.raises(ValueError, match="fix:branches"):
        trade.fix.branches = ["c,me"]
    with pytest.raises(ValueError, match="fix:branches"):
        trade.fix.add_branch("")
    assert trade.fix.branches == ["cme"]
    # Names are text, never numbers.
    with pytest.raises(TypeError):
        trade.fix.branches = 5001  # type: ignore[assignment]
    with pytest.raises(TypeError):
        trade.fix.branches = [5001]  # type: ignore[list-item]
    with pytest.raises(TypeError):
        trade.fix.has_branch(5001)  # type: ignore[arg-type]
    assert trade.fix.branches == ["cme"]

    # The registry lists the distinct names its fields and definitions
    # carry, sorted; a registry of the crate's fields alone lists none.
    assert FixRegistry().dialects() == []
    registry = FixRegistry.from_fields(
        [
            _field("Symbol", "utf8", 55),
            _field("VenueSym", "utf8", 5055, branches=["cme", "ICE"]),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
        ]
    )
    assert registry.dialects() == ["cme", "ice"]
    assert registry.field_by_tag(5055).fix.branches == ["cme", "ice"]
    assert registry.field_by_tag(55).fix.branches == []
    # Membership travels through the snapshot like any other metadata.
    assert FixRegistry.from_json(registry.into_json()).dialects() == ["cme", "ice"]
    assert FixRegistry.from_json(registry.into_json()) == registry


def test_registry_resolves_every_key_the_way_the_core_does(seed: FixRegistry) -> None:
    # The store's fields, and the crate's own beside them: a store never
    # writes those, so a loaded dictionary holds the crate's definition. The
    # store states its own SendingTime and TransactTime, so no seed adds one.
    assert len(seed) == 6241 + CRATED
    assert bool(seed)

    assert seed.field_by_tag(55).name == "symbol"
    assert seed.get_field_by_tag(55) == seed.field_by_tag(55)
    symbol_id = seed.field_by_tag(55).fix.id
    assert symbol_id is not None
    assert seed.field_by_id(symbol_id).name == "symbol"
    assert seed.get_field_by_id(symbol_id) == seed.field_by_tag(55)
    assert seed.field_by_tag(150).name == "exectype"
    # The order's state is declared twice, as `OrdStatus` and as `ExecType`,
    # and both take the crate's `state`: one lifecycle vocabulary, ranked so
    # the stored bytes sort from first state to terminal. Every other code
    # set keeps its base type.
    assert seed.field_by_tag(39).dtype == DataType("state")
    assert seed.field_by_tag(150).dtype == DataType("state")
    assert seed.field_by_tag(40).dtype == DataType("utf8")
    # The published dictionary states no alternate tags, so the alternate
    # tier is exercised where one is actually declared.
    alternate = _field("exectype", "utf8", 150)
    alternate.fix.tags = [20]
    aliased = FixRegistry.from_fields([alternate])
    assert aliased.field_by_tag(20).name == "exectype"
    # An alternate tag is not an identity: the one id is the canonical tag's.
    assert aliased.field_by_id(alternate.fix.id).name == "exectype"
    assert aliased.get_field_by_id(_field("exectype", "utf8", 20).fix.id) is None
    # A name answers the canonical spelling whatever case it was asked in.
    assert seed.field_by_name("symbol").name == "symbol"
    assert seed.field_by_name("SYMBOL").name == "symbol"
    # The published dictionary declares no aliases, so the alias tier is
    # exercised where one is actually declared.
    aliased = _field("symbol", "utf8", 55)
    aliased.fix.aliases = ["ticker"]
    named = FixRegistry.from_fields([aliased])
    assert named.field_by_name("ticker").name == "symbol"
    # A path reaches a repeating group and one of its members.
    assert seed.field_by_path("NoPartyIDs").fix.tag == 453
    # An occurrence is not a path segment: the walk steps through the list
    # and the member is spelled directly under the named group.
    assert seed.field_by_tag(453).dtype == DataType("int32")
    assert seed.field_by_path("parties.partyid").fix.tag == 448
    assert seed.field_by_path("parties.partyrole").name == "partyrole"
    assert seed.get_field_by_path("parties.partyid.partyid") is None

    # The generic pair answers exactly what the specialized one does.
    for key in (55, "Symbol", "nopartyids", "parties.partyid"):
        assert seed.get_field(key) == seed[key]
        assert seed.field(key) == seed[key]
        assert key in seed
    assert 9999 not in seed
    assert "Nope" not in seed
    assert seed.get_field(9999) is None
    assert seed.get(9999) is None
    assert seed.get(9999, "fallback") == "fallback"
    assert seed.get(55) == seed[55]


def test_protocol_and_msgtype_inference_stays_native_and_shallow() -> None:
    cases = (
        (b"prefix 8=FIX.4.4|35=D|55=AAPL|10=001| Symbol=suffix", MimeType.FIX, b"D"),
        (b"ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL", MimeType.ULLINK, b"8"),
        (b"8=FIX.4.4|35=UL|#SYMBOL=TTF|10=001|", MimeType.FIXUL, b"UL"),
        (
            b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|",
            MimeType.FIXUL,
            b"D",
        ),
        (b"level=INFO message=random", MimeType.KEYVALUE, None),
        (
            # A bridge configuration document is JSON, which is what it is:
            # what makes one *this* reader's is a shape the codec reads
            # rather than a name the scan gives it.
            b'{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:'
            b'name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"}',
            MimeType.JSON,
            b"Plugin",
        ),
    )
    for line, protocol, msgtype in cases:
        assert MimeType.infer_bytes(line) == protocol
        assert MimeType.infer_bytes(bytearray(line)) == protocol
        assert MimeType.infer_bytes(memoryview(line)) == protocol
        assert FixCodec.infer_msgtype_bytes(line) == msgtype
        assert FixCodec.infer_msgtype_bytes(bytearray(line)) == msgtype
        assert FixCodec.infer_msgtype_bytes(memoryview(line)) == msgtype
        text = line.decode()
        assert MimeType.infer_text(text) == protocol
        assert FixCodec.infer_msgtype_text(text) == (
            msgtype.decode() if msgtype is not None else None
        )

    assert FixCodec.infer_msgtype_bytes(b"35=AE|") == b"AE"
    assert FixCodec.infer_msgtype_text("MSGTYPE=AE|") == "AE"

    # A bridge configuration document states no half of the exchange on its
    # own, and the `send` its own payload spells is never read as the marker:
    # which way it moved is the prose in front of it, read into FIX's own
    # tag 385 by the rules the dictionary carries on that field.
    answered = (
        '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",'
        '"type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:'
        'name=Router_TradeCapture,plugin-type=FIX,type=Plugin":'
        '{"Name":"Router_TradeCapture"}},"status":200}'
    )
    assert MimeType.infer_text(answered) == MimeType.JSON
    # The ObjectName the answer keys its `value` by states the type, and it
    # is the first one the shallow scan reaches: the wildcard the request
    # echoes names none.
    assert FixCodec.infer_msgtype_text(answered) == "Plugin"
    codec = _fixed(FixRegistry())
    assert next(codec.parse_line(answered.encode())).get_by_tag(385) is None
    assert next(codec.parse_line(("Response: " + answered).encode())).by_tag(385).as_py() == "R"
    selected = (
        '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:'
        'name=Router_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},'
        '"value":{"Name":"Router_TradeCapture"},"status":200}'
    )
    assert next(codec.parse_line(selected.encode())).get_by_tag(385) is None
    assert next(codec.parse_line(("Request: " + selected).encode())).by_tag(385).as_py() == "S"
    # A direction is the line's and a message is the document's, read apart:
    # a read that selected nothing and a request not yet answered both name
    # no plugin, so neither states a message - there is no envelope left to
    # make a row out of - and the prose in front of one makes it no more one.
    empty = (
        '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",'
        '"type":"read"},"value":{},"status":200}'
    )
    asked = '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}'
    for body, verb in ((empty, "Response"), (asked, "Request")):
        assert next(codec.parse_line(body.encode()), None) is None
        prosed = f"[Jolokia] (DEBUG) {verb}: {body}"
        assert next(codec.parse_line(prosed.encode()), None) is None


def test_one_namespace_folds_a_venues_field_by_name_and_keeps_it_by_tag() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55, aliases=["Ticker"]),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
        ]
    )
    symbol_id = registry.field_by_tag(55).fix.id
    trade_id = registry.field_by_tag(5001).fix.id

    # The same folded name under another tag is the same field spelled with
    # another number: it merges into the holder, which gains the tag as an
    # alternate, the alias, and the membership. No second field.
    venue = _field("symbol", "utf8", 5055, branches=["cme"], aliases=["VenueTicker"])
    assert registry.add_field(venue) is False
    assert len(registry) == 2 + SEEDED
    holder = registry.field_by_tag(55)
    assert holder.fix.tags == [5055]
    assert holder.fix.aliases == ["Ticker", "VenueTicker"]
    assert holder.fix.branches == ["cme"]
    assert holder.fix.id == symbol_id
    assert registry.get_field_by_tag(5055) == holder
    assert registry.field_by_name("VENUETICKER") == holder
    assert registry.get_field_by_id(venue.fix.id) is None
    assert registry.dialects() == ["cme"]

    # The same tag under another name is a new thing a dialect defined over
    # a tag it reused: it stands beside the holder under its own id, the
    # holder gains the name as an alias, and the bare tag keeps answering
    # the holder; the newcomer is reached by its name or its id.
    reused = _field("VenueSym", "utf8", 55, branches=["ice"])
    assert registry.add_field(reused) is True
    assert len(registry) == 3 + SEEDED
    assert registry.field_by_tag(55).name == "symbol"
    assert registry.field_by_tag(55).fix.aliases == ["Ticker", "VenueTicker", "VenueSym"]
    assert registry.field_by_id(reused.fix.id).name == "VenueSym"
    assert registry.field_by_name("venuesym").fix.id == reused.fix.id
    assert registry.get_field_by_tag(55).fix.branches == ["cme"]
    assert registry.dialects() == ["cme", "ice"]
    # Tag-major, then by id: both fields on tag 55 walk between the seeded
    # SendingTime (52) and TransactTime (60), and before the venue's.
    walked = [field.fix.id for field in registry]
    assert [registry.field_by_id(held).fix.tag for held in walked[:5]] == [52, 55, 55, 60, 5001]
    assert walked[:5] == sorted(walked[:5], key=lambda held: (registry.field_by_id(held).fix.tag, held))
    assert {registry.field_by_id(held).name for held in walked[1:3]} == {"symbol", "VenueSym"}
    assert walked[4] == trade_id

    # A colon-bearing string is a name, never an identifier, and a bare int
    # is a tag, never an id.
    assert registry.get_field("symbol").fix.id == symbol_id
    assert registry.get_field("5055:cme") is None
    assert "5055:cme" not in registry
    assert registry.get("5001:cme", "fallback") == "fallback"
    assert registry.remove("5055:cme") is None

    # A field leaves by its identifier - the only spelling that names one of
    # two fields on a tag - or, alone on its tag, by that tag.
    assert registry.remove_by_id(_field("Nowhere", "utf8", 9999).fix.id) is None
    removed = registry.remove_by_id(reused.fix.id)
    assert removed is not None and removed.name == "VenueSym"
    assert len(registry) == 2 + SEEDED
    assert registry.get_field_by_id(reused.fix.id) is None
    assert registry.field_by_tag(55).fix.id == symbol_id
    removed = registry.remove_by_id(trade_id)
    assert removed is not None and removed.name == "TradeID"
    assert registry.get_field_by_tag(5001) is None


def test_registry_absence_is_a_key_error_carrying_the_core_message(
    seed: FixRegistry,
) -> None:
    # ``KeyError`` renders its argument as a repr, so the native message is
    # read off the argument itself rather than off the rendering.
    with pytest.raises(KeyError) as by_tag:
        seed.field_by_tag(9999)
    assert by_tag.value.args[0] == 'expected a fix field at "tag 9999", got nothing'

    absent_id = _field("TradeID", "utf8", 5001).fix.id
    assert absent_id is not None
    with pytest.raises(KeyError) as by_id:
        seed.field_by_id(absent_id)
    assert by_id.value.args[0] == f'expected a fix field at "identifier {absent_id}", got nothing'

    with pytest.raises(KeyError) as by_name:
        seed.field_by_name("Nope")
    assert 'name \\"Nope\\"' in by_name.value.args[0]

    with pytest.raises(KeyError) as by_path:
        seed.field_by_path("Symbol.absent")
    assert "path Symbol.absent" in by_path.value.args[0]

    with pytest.raises(KeyError):
        seed[9999]
    assert seed.get_field_by_name("Nope") is None
    assert seed.get_field_by_path("Symbol.absent") is None
    assert seed.get_field_by_id(absent_id) is None


def test_registry_keys_are_an_int_tag_or_a_str_name(seed: FixRegistry) -> None:
    with pytest.raises(TypeError, match="not bool"):
        seed[True]
    with pytest.raises(TypeError, match="not bool"):
        seed.field_by_tag(True)
    with pytest.raises(OverflowError):
        seed.field_by_tag(2**31)
    with pytest.raises(OverflowError):
        seed[2**31]
    with pytest.raises(TypeError, match="int tag or a str name"):
        seed[3.5]
    with pytest.raises(TypeError):
        seed.field_by_name(55)


def test_registry_coerces_every_identifier_argument(seed: FixRegistry) -> None:
    # An identifier is the `int` a field answers, and nothing else: text is
    # never parsed into one, a bool is refused by name, and a value outside
    # `i32` is the overflow the extraction reports rather than a narrowed id.
    for wrong in ("55", "55:", "cme:x", None, 3.5):
        with pytest.raises(TypeError):
            seed.field_by_id(wrong)  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            seed.get_field_by_id(wrong)  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            seed.remove_by_id(wrong)  # type: ignore[arg-type]
    with pytest.raises(TypeError, match="not bool"):
        seed.field_by_id(True)
    with pytest.raises(OverflowError):
        seed.get_field_by_id(2**31)
    with pytest.raises(OverflowError):
        seed.field_by_id(-(2**31) - 1)
    # A name and a path take one argument: there is no dictionary to pin.
    with pytest.raises(TypeError):
        seed.get_field_by_name("Symbol", "")  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        seed.field_by_path("Symbol", None)  # type: ignore[call-arg]
    # The counter tables are keyed by counter tag.
    assert seed.group_by_tag(453).name == "parties"
    assert seed.get_group_by_tag(9999) is None
    with pytest.raises(TypeError, match="not bool"):
        seed.get_group_by_tag(True)
    with pytest.raises(TypeError):
        seed.group_by_tag("453:")  # type: ignore[arg-type]
    with pytest.raises(KeyError):
        seed.group_by_tag(9999)


def test_registry_iterates_lazily_in_ascending_identifier_order() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
            _field("Price", "decimal128(20, 8)", 44),
            _field("VenueQty", "int64", 5002, branches=["cme"]),
            _field("Account", "utf8", 1),
            _field("Tail", "utf8", 9001),
        ]
    )
    # Tag-major, then by identifier. The seeded SendingTime (52) and
    # TransactTime (60) walk among the dictionary's own tags, and the crate's
    # own fields close every walk, above any tag a test claims.
    assert [field.fix.tag for field in registry] == [1, 44, 52, 55, 60, 5001, 5002, 9001, *CRATE_TAGS]

    walk = iter(registry)
    assert next(walk).name == "Account"
    assert next(walk).name == "Price"
    # An unfinished walk shares the registry, so a mutation refuses until it
    # is dropped rather than moving the fields under the cursor.
    with pytest.raises(ValueError, match="shared with a message"):
        registry.remove(1)
    del walk
    assert registry.remove(1) is not None
    assert [field.fix.tag for field in registry] == [44, 52, 55, 60, 5001, 5002, 9001, *CRATE_TAGS]


def test_seed_iterates_in_canonical_tag_order(seed: FixRegistry) -> None:
    names = [field.name for field in seed]
    assert names[:4] == ["account", "advid", "advrefid", "advside"]
    assert len(names) == len(seed)

    tags = [field.fix.tag for field in seed]
    assert tags == sorted(tags)
    # The crate's own twenty-five close the walk, above every tag the
    # specification publishes; the store's own SendingTime and TransactTime
    # stand where their tags put them.
    assert tags[-CRATED:] == CRATE_TAGS
    assert seed.field_by_tag(52).name == "sendingtime"
    assert seed.field_by_tag(60).name == "transacttime"
    # Every stored field is a specification field and the crate's own are
    # standard fields too, so nothing here states a membership.
    assert all(field.fix.branches == [] for field in seed)
    assert all("fix:branches" not in field.metadata for field in seed)
    assert seed.dialects() == []


def test_registry_takes_every_storage_location(
    seed: FixRegistry, tmp_path: pathlib.Path
) -> None:
    absolute = SEED.resolve()
    for location in (
        absolute,
        str(absolute),
        absolute.as_uri(),
        Url(absolute),
        IOBase(absolute),
    ):
        assert FixRegistry.from_handle(location) == seed

    # A folder that is not there loads as a new registry - the crate's own
    # fields and the two seeded clocks - and is not created.
    missing = tmp_path / "missing"
    assert FixRegistry.from_handle(missing) == FixRegistry()
    assert len(FixRegistry.from_handle(missing)) == SEEDED
    assert not missing.exists()


def test_a_malformed_native_field_shard_is_located(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "invalid"
    (root / "fields").mkdir(parents=True)
    (root / "fields" / "0.json").write_text("not JSON", encoding="utf-8")
    with pytest.raises(ValueError, match="0.json"):
        FixRegistry.from_handle(root)


def test_registry_round_trips_through_the_three_categories(
    seed: FixRegistry, tmp_path: pathlib.Path
) -> None:
    root = tmp_path / "dictionary"
    seed.write_into(root)

    assert (root / "fields" / "0.json").is_file()
    # Every category comes back exactly as heavy as the committed catalog.
    # The crate's own `pluginconfig` is not written, for the reason the
    # crate's own fields are not: every registry holds it from construction,
    # so a store that wrote it would claim to define what it inherited
    # (decision 19).
    for category in ("fields", "components", "groups"):
        assert len(list((root / category).glob("*.json"))) == len(
            list((SEED / category).glob("*.json"))
        )
    assert not (root / "components" / "pluginconfig.json").exists()
    assert not (root / "groups" / "altids.json").exists()
    # The crate's own fields are never written either: they are the crate's
    # rather than the store's, so the shard their tag block would take is in
    # no category at all, and the reload holds them all the same.
    assert not (root / "fields" / f"{CRATE_TAGS[0] // 100}.json").exists()
    assert FixRegistry.from_handle(root) == seed

    reloaded = FixRegistry.from_handle(IOBase(root))
    assert reloaded.remove(453) is None
    assert reloaded.remove("PartyID") is None
    assert reloaded == seed
    reloaded.insert(_field("LocalValue", "utf8", 9999))
    assert reloaded.remove(9999) is not None
    reloaded.write_into(root)
    assert FixRegistry.from_handle(root) == seed


def test_a_vendor_field_shards_by_its_tag_and_keeps_its_membership(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "dictionary"
    registry = FixRegistry.from_fields(
        [
            _field("MsgType", "utf8", 35),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
        ]
    )
    registry.write_into(root)

    # One shard arithmetic for every field: 5001 / 100 is 50, and nothing is
    # keyed by a dictionary.
    assert (root / "fields" / "0.json").exists()
    assert (root / "fields" / "50.json").exists()
    assert not (root / "fields" / "cme").exists()
    assert not (root / "branches.json").exists()

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    trade_id = registry.field_by_tag(5001).fix.id
    assert reloaded.field_by_id(trade_id).name == "TradeID"
    assert reloaded.field_by_name("tradeid").fix.branches == ["cme"]
    assert reloaded.get_field_by_tag(5001) == reloaded.field_by_id(trade_id)
    assert reloaded.dialects() == ["cme"]


def test_registry_insert_update_and_remove(seed: FixRegistry) -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55, aliases=["Ticker"]),
            _field("Price", "decimal128(20, 8)", 44, aliases=["Px"]),
        ]
    )
    assert len(registry) == 2 + SEEDED
    assert registry.insert(_field("Side", "utf8", 54)) is None
    assert registry.field_by_tag(54).name == "Side"

    # A key another field holds is refused, naming both; nothing changes.
    with pytest.raises(ValueError, match="held by symbol"):
        registry.insert(_field("SymbolSfx", "utf8", 65, aliases=["ticker"]))
    assert len(registry) == 3 + SEEDED

    # One namespace: the same alias under a venue's tag is the same conflict.
    with pytest.raises(ValueError, match="held by symbol"):
        registry.insert(_field("VenueSym", "utf8", 5055, branches=["cme"], aliases=["ticker"]))
    assert len(registry) == 3 + SEEDED
    assert registry.get_field_by_tag(5055) is None

    # A merge concatenates the two list properties, incoming first.
    registry.update(_field("SYMBOL", "utf8", 55, tags=[65], aliases=["Sym"]))
    merged = registry.field_by_tag(65)
    assert merged.name == "symbol"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(_field("symbol", "large_utf8", 55))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    removed = registry.remove("sym")
    assert removed is not None and removed.name == "symbol"
    assert registry.get_field_by_tag(65) is None
    assert registry.remove(9999) is None

    # A field with no tag cannot enter at all.
    with pytest.raises(ValueError, match="fix:tag"):
        registry.insert(Field("Untagged", "utf8"))
    assert seed.get_field_by_name("Untagged") is None


def test_registry_add_field_answers_whether_the_field_arrived_or_folded() -> None:
    """`add_field` is the one-field verb `add_fields` folds through: True arrived, False merged."""
    registry = FixRegistry.from_fields(
        [
            _field("Symbol", "utf8", 55, tags=[65], aliases=["Ticker"], description="stored"),
            _field("Price", "float64", 44),
        ]
    )

    # A name that folds to a stored name merges into that field: the stored
    # identity, spelling and nullability stand, the alternate tags and the
    # aliases are the union - stored order first, the incoming canonical tag
    # last - and the incoming metadata wins a shared key.
    incoming = _field(
        "symbol", "utf8", 9001, tags=[66], aliases=["Sym", "TICKER"], description="incoming"
    )
    assert registry.add_field(incoming) is False
    assert len(registry) == 2 + SEEDED
    stored = registry.field_by_tag(55)
    assert stored.name == "Symbol"
    assert stored.fix.id == _field("symbol", "utf8", 55).fix.id
    assert stored.fix.tags == [65, 66, 9001]
    assert stored.fix.aliases == ["Ticker", "Sym"]
    assert stored.description == "incoming"

    # Every spelling the incoming field carried now reaches the stored one.
    for key in (9001, 66, 65, "sym", "TICKER"):
        assert registry.field(key).name == "Symbol", key

    # Folding it again changes nothing, and a field nothing answers to
    # arrives whole.
    before = registry.into_json()
    assert registry.add_field(incoming) is False
    assert registry.into_json() == before
    # TransactTime (60) is a seeded clock every registry already holds, so the
    # field that arrives here is one no registry seeds.
    assert registry.add_field(_field("Text", "utf8", 58)) is True
    assert len(registry) == 3 + SEEDED

    # A datatype that disagrees with the stored field is refused, and the
    # refusal writes nothing: merging metadata never redeclares a datatype.
    before = registry.into_json()
    with pytest.raises(ValueError, match="utf8"):
        registry.add_field(_field("SYMBOL", "int32", 9002))
    assert registry.into_json() == before
    assert registry.get_field_by_tag(9002) is None

    # One of this crate's own tags is every dictionary's already: neither
    # added nor merged.
    assert registry.add_field(fix_crate_fields()[0]) is False
    assert registry.into_json() == before


def test_registry_add_fields_adds_what_is_absent_and_merges_what_is_present() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55, aliases=["Ticker"]),
            _field("Price", "utf8", 44),
        ]
    )

    # Tag 55 is stored and folds; 58 is new; the venue's 5055 shares the tag
    # and the name of nothing, and arrives with its membership.
    added, merged = registry.add_fields(
        [
            _field("SYMBOL", "utf8", 55, tags=[65], aliases=["Sym"]),
            _field("Text", "utf8", 58),
            _field("VenueSym", "utf8", 5055, branches=["cme"]),
        ]
    )
    assert (added, merged) == (2, 1)
    assert len(registry) == 4 + SEEDED
    assert registry.field_by_tag(5055).fix.branches == ["cme"]

    # The fold kept what only the stored field declared and added the rest.
    folded = registry.field_by_tag(65)
    assert folded.name == "symbol"
    assert folded.fix.aliases == ["Sym", "Ticker"]

    # One mutation: a refusal partway leaves the dictionary as it was, so
    # neither the field before it nor the one after arrives.
    with pytest.raises(ValueError):
        registry.add_fields(
            [
                _field("Side", "utf8", 54),
                _field("symbol", "large_utf8", 55),
                _field("Account", "utf8", 1),
            ]
        )
    assert len(registry) == 4 + SEEDED
    assert registry.get_field_by_tag(54) is None
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    # No ``fix:tag`` is no identity, so there is nothing to add or fold under.
    with pytest.raises(ValueError, match="fix:tag"):
        registry.add_fields([Field("Untagged", "utf8")])
    assert len(registry) == 4 + SEEDED


def test_merge_with_folds_the_fields_and_unions_their_membership() -> None:
    dictionary = FixRegistry.from_fields(
        [_field("symbol", "utf8", 55), _field("VenueSym", "utf8", 5055, branches=["cme"])]
    )
    other = FixRegistry.from_fields(
        [
            _field("SYMBOL", "utf8", 55, branches=["ice"]),
            _field("VenueTime", "utf8", 5060, branches=["cme"]),
        ]
    )

    # The other dictionary holds the crate's own fields as every registry
    # does, and they are never folded: they are the crate's definition, not
    # something a dictionary states. `symbol` merges, and so do the two seeded
    # standard clocks, which are ordinary definitions.
    assert dictionary.merge_with(other) == (1, 3)
    assert len(dictionary) == 3 + SEEDED
    assert dictionary.field_by_tag(55).name == "symbol"

    # Membership unions onto the field it merges into and arrives whole with
    # a field nothing held; the registry lists every name either side spoke.
    assert dictionary.field_by_tag(55).fix.branches == ["ice"]
    assert dictionary.field_by_tag(5060).fix.branches == ["cme"]
    assert dictionary.dialects() == ["cme", "ice"]
    # Unioned, not replaced: a second merge of a dictionary that also speaks
    # `symbol` adds its name beside the one already there.
    third = FixRegistry.from_fields([_field("Symbol", "utf8", 55, branches=["cme"])])
    assert dictionary.merge_with(third) == (0, 3)
    assert dictionary.field_by_tag(55).fix.branches == ["cme", "ice"]


def test_a_cblock_reads_in_whole_and_stamps_its_dialect_on_every_field(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "MSFIX44.cfb"
    path.write_text(CBLOCK, encoding="utf-8")

    # The file's two fields arrive, and the parsed dictionary's two seeded
    # standard clocks merge into this one's.
    registry = FixRegistry()
    assert registry.add_cfb_file(path, "Morgan") == (2, 2)
    assert len(registry) == 2 + SEEDED

    # Membership means "this dictionary speaks it": the standard tag and the
    # venue's own are both stamped, folded once, and the registry lists it.
    assert registry.field_by_tag(55).fix.branches == ["morgan"]
    assert registry.field_by_tag(10001).fix.branches == ["morgan"]
    assert registry.dialects() == ["morgan"]

    # With no name the file's own stem stands in, and a second dictionary
    # speaking a field unions onto it rather than replacing anything.
    assert registry.add_cfb_file(path) == (0, 4)
    assert registry.field_by_tag(55).fix.branches == ["morgan", "msfix44"]
    assert registry.dialects() == ["morgan", "msfix44"]

    # One mutation: a refusal leaves the dictionary exactly as it was.
    retyped = tmp_path / "retyped.cfb"
    retyped.write_text(CBLOCK.replace('name="55" alt="Symbol" type="string"', 'name="55" alt="Symbol" type="integer"'), encoding="utf-8")
    with pytest.raises(ValueError):
        registry.add_cfb_file(retyped, "morgan")
    assert len(registry) == 2 + SEEDED
    assert registry.dialects() == ["morgan", "msfix44"]
    # The keyword is `dialect`, and a name the grammar refuses is refused
    # before anything is read.
    with pytest.raises(ValueError, match="fix:branches"):
        registry.add_cfb_file(path, dialect="mor,gan")
    assert registry.dialects() == ["morgan", "msfix44"]


def test_a_cblock_answers_its_vocabulary_and_folds_into_a_dictionary(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "bloomberg.cfb"
    path.write_text(CBLOCK, encoding="utf-8")

    # A CBlock never names itself, so with no dialect the file's own stem
    # does, and every field it produces carries it - the standard tag too.
    fields = fix_cfb_fields(path)
    assert [field.name for field in fields] == ["symbol", "excludeddealers"]
    assert fields[0].fix.branches == ["bloomberg"]
    assert fields[1].fix.branches == ["bloomberg"]
    # The file's own spelling is kept beside the folded name, and the
    # description travels on the key every catalog reads.
    assert fields[0].display == "Symbol"
    assert fields[0].description == "Ticker symbol."

    # Every location the registry takes, the vocabulary takes - and a CBlock is
    # a file, so a location held as a container would read no bytes at all.
    for location in (path, str(path), path.as_uri(), Url(path), IOBase(path)):
        assert [field.name for field in fix_cfb_fields(location, "bloomberg")] == [
            "symbol",
            "excludeddealers",
        ]

    # The registry form is the same file read whole: the same vocabulary, plus
    # the message roots its grammar bindings describe.
    registry, roots = FixRegistry.from_cfb_file(path, dialect="Bloomberg")
    assert len(registry) == len(fields) + SEEDED
    assert [root.name for root in roots] == ["7"]
    # The message definition the file produces is stamped like its fields.
    assert next(registry.msgtypes()).field.fix.branches == ["bloomberg"]
    assert registry.dialects() == ["bloomberg"]
    # The registry form stamps nothing when no dialect is named.
    unstamped, _ = FixRegistry.from_cfb_file(path)
    assert unstamped.dialects() == []
    assert unstamped.field_by_tag(10001).fix.branches == []

    # The vocabulary folds into a dictionary that already exists, and the
    # membership unions onto the field it merges into.
    dictionary = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    assert dictionary.add_fields(fix_cfb_fields(path, "bloomberg")) == (1, 1)
    assert dictionary.field_by_tag(55).description == "Ticker symbol."
    assert dictionary.field_by_tag(55).fix.branches == ["bloomberg"]
    assert dictionary.field_by_name("excludeddealers").fix.tag == 10001

    # A stem or a dialect that carries a comma is refused rather than stored.
    unnamed = tmp_path / "ms,44.cfb"
    unnamed.write_text(CBLOCK, encoding="utf-8")
    with pytest.raises(ValueError, match="fix:branches"):
        fix_cfb_fields(unnamed)
    with pytest.raises(ValueError, match="fix:branches"):
        fix_cfb_fields(path, "b,loomberg")


def test_registering_a_message_type_names_it_and_describes_it(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "bloomberg.cfb"
    path.write_text(
        """<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration version="1.2" fix-version="4.4">
	<message-types>
		<message-type value="7" description="Advertisement" supported="false" />
	</message-types>
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>
""",
        encoding="utf-8",
    )
    registry, _ = FixRegistry.from_cfb_file(path, "bloomberg")

    # The file's own message types arrived with it, valued the way the column
    # takes them: `7` fits and is itself, described as the file described it.
    codes = registry.field_by_tag(35).metadata["fix:codes"]
    assert '"value":"7"' in codes
    assert "Advertisement" in codes

    # A type a bridge invents is added under the name and wording it is given,
    # and the spelling stays a spelling of it.
    value = registry.register_msgtype(
        "P Report Ack", "AllocationReportAck", "Allocation Report ACK"
    )
    assert isinstance(value, MsgType)
    assert value.value == "P Report Ack"
    codes = registry.field_by_tag(35).metadata["fix:codes"]
    assert '"value":"P Report Ack"' in codes
    assert '"name":"AllocationReportAck"' in codes
    assert '"P Report Ack"' in codes
    assert '"Allocation Report ACK"' in codes

    # The borrowed singleton pins the registry until the view is released.
    snapshot = pickle.dumps(value)
    with pytest.raises(ValueError, match="shared"):
        registry.register_msgtype("P Report Ack")
    del value
    assert registry.register_msgtype("P Report Ack") == pickle.loads(snapshot)


def test_a_cblock_warns_about_the_declaration_it_dropped(
    tmp_path: pathlib.Path,
    caplog: pytest.LogCaptureFixture,
) -> None:
    # A CBlock is read for what it says, so a declaration this reader cannot
    # make a field of is dropped and the rest of the file is still a
    # dictionary. The native sentence crosses whole through `logging`: the
    # byte, what was expected, what arrived, and the element the file spells
    # it in.
    broken = tmp_path / "bloomberg.cfb"
    broken.write_text(
        CBLOCK.replace('name="55" alt="Symbol" type="string"', 'name="55" alt="Symbol" type="decimal"'),
        encoding="utf-8",
    )
    caplog.set_level(logging.WARNING)
    # The bridge caches each Python logger's effective level, so a level set
    # after import reaches it only through this call.
    refresh_logging()
    registry, _ = FixRegistry.from_cfb_file(broken, "bloomberg")
    warnings = [
        record.getMessage()
        for record in caplog.records
        if record.name.startswith("yggdryl") and record.levelno == logging.WARNING
    ]
    assert warnings, "the native reader reported nothing"
    rendered = warnings[0]
    assert "invalid cfb expression at byte" in rendered
    assert '"decimal"' in rendered
    assert 'vocabulary-tag name=\\"55\\"' in rendered

    # The tag went; every other declaration the file made stands.
    assert [field.name for field in fix_cfb_fields(broken)] == ["excludeddealers"]
    assert registry.field_by_name("excludeddealers").fix.tag == 10001

    # A document that stops with an element open leaves nothing to keep, and
    # is one of the two things still refused - through both doors, with one
    # sentence.
    truncated = tmp_path / "truncated.cfb"
    truncated.write_text(CBLOCK.replace("</vocabulary>", ""), encoding="utf-8")
    with pytest.raises(ValueError) as refused:
        FixRegistry.from_cfb_file(truncated, "bloomberg")
    rendered = str(refused.value)
    assert "invalid cfb expression at byte" in rendered
    with pytest.raises(ValueError) as also:
        fix_cfb_fields(truncated)
    assert str(also.value) == rendered


def test_a_cblock_description_keeps_its_words_and_loses_its_layout(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "bloomberg.cfb"
    path.write_text(
        CBLOCK.replace(
            "<description>Ticker symbol.</description>",
            "<description>Ticker symbol.\n\t\t\tOne per instrument, and never a &lt;SOH&gt;.</description>",
        ),
        encoding="utf-8",
    )
    fields = fix_cfb_fields(path)
    assert (
        fields[0].description
        == "Ticker symbol. One per instrument, and never a <SOH>."
    )


def test_registry_mutation_refuses_while_something_shares_it(
    seed: FixRegistry,
) -> None:
    root = Field(
        "row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False
    )
    message = FixMsg(root, {"symbol": "AAPL"}, seed)

    for mutation in (
        lambda: seed.insert(_field("Side", "utf8", 54)),
        lambda: seed.update(_field("symbol", "utf8", 55)),
        lambda: seed.remove(55),
    ):
        with pytest.raises(ValueError, match="shared with a message"):
            mutation()
    assert message.registry == seed

    # The registry a message shares is still readable, and a copy is writable.
    assert seed.field_by_tag(55).name == "symbol"
    fresh = copy.copy(seed)
    fresh.insert(_field("LocalValue", "utf8", 9999))
    assert fresh.remove(9999) is not None


def _order(seed: FixRegistry) -> Field:
    """A root that carries a group, one tag no dictionary explains, and its SendingTime.

    A message built by hand reads UTC now for a SendingTime it does not state,
    so the root states one and two builds of it are one message.
    """
    return Field(
        "NewOrderSingle",
        DataType.from_fields(
            [
                seed.field_by_tag(55),
                seed.field_by_tag(38),
                seed.field_by_name("NoPartyIDs"),
                seed.definition("groups", "Parties"),
                Field("9999", "utf8"),
                seed.field_by_tag(52),
            ]
        ),
        nullable=False,
    )


ORDER_VALUE: dict[str, Any] = {
    "symbol": "AAPL",
    "orderqty": 100.0,
    "nopartyids": 1,
    "parties": [
        {"partyid": "BROKER", "partyidsource": "D", "partyrole": 1},
    ],
    "9999": "custom",
    "sendingtime": CLOCK,
}


def test_message_resolves_through_the_registry_it_carries(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)

    # The root's own children lead, and every member of the settled bundle it
    # lacks is appended after them, each non-null
    # (``rust/tests/fix/content_identity.rs``).
    assert message.field != root
    assert [child.name for child in message.field] == [
        *(child.name for child in root),
        "updatedat",
        "createdat",
        "uuid",
        "puuid",
        "code",
        "snapshotat",
    ]
    assert all(not child.nullable for child in message.field if child.name in BUNDLE)
    assert message.registry == seed
    assert len(message) == 12
    symbol_id = seed.field_by_tag(55).fix.id
    assert symbol_id is not None
    assert message.by_tag(55).as_py() == "AAPL"
    assert message.by_id(symbol_id).as_py() == "AAPL"
    assert message.by_name("symbol").as_py() == "AAPL"
    assert message.by_tag(38).as_py() == 100.0
    assert message.by_path("parties[0].partyid").as_py() == "BROKER"
    # An unknown tag is retained under its rendered name, never dropped.
    assert message.by_tag(9999).as_py() == "custom"
    # An identifier is exact: a field the dictionary does not hold misses.
    absent_id = _field("TradeID", "utf8", 5001).fix.id
    assert absent_id is not None
    assert message.get_by_id(absent_id) is None

    assert message[55] == message.by_tag(55)
    assert message["symbol"] == message.by_tag(55)
    assert message.get(55) == message.by_tag(55)
    assert message.get(1234) is None
    assert message.get(1234, "fallback") == "fallback"
    assert message.get_by_name("nope") is None
    assert message.get_by_path("Parties.PartyID") is None
    with pytest.raises(KeyError) as by_tag:
        message.by_tag(1234)
    assert by_tag.value.args[0] == 'expected a fix value at "tag 1234", got nothing'
    with pytest.raises(KeyError) as by_id:
        message.by_id(absent_id)
    assert by_id.value.args[0] == f'expected a fix value at "identifier {absent_id}", got nothing'
    with pytest.raises(KeyError) as by_name:
        message.by_name("nope")
    assert 'name \\"nope\\"' in by_name.value.args[0]
    with pytest.raises(KeyError) as by_path:
        message.by_path("Parties.PartyID")
    assert "path Parties.PartyID" in by_path.value.args[0]
    with pytest.raises(TypeError, match="not bool"):
        message[True]
    # An identifier is an `int` and nothing else is read as one.
    with pytest.raises(TypeError):
        message.by_id("55")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        message.get_by_id("cme:")  # type: ignore[arg-type]
    with pytest.raises(TypeError, match="not bool"):
        message.get_by_id(True)
    with pytest.raises(OverflowError):
        message.get_by_id(2**31)

    # The mapping input became the ordered row the message declares.
    pairs = [(name, value) for name, value in message]
    assert [name for name, _ in pairs] == [child.name for child in message.field]
    assert pairs[0][0] == "symbol" and pairs[0][1].as_py() == "AAPL"
    # With no clock of its own, every clock the bundle settles is SendingTime.
    assert message.by_tag(52) == CLOCK
    assert message.updatedat() == CLOCK
    assert message.createdat() == CLOCK
    assert message.by_tag(SNAPSHOTAT_TAG) == CLOCK
    assert message.by_tag(CODE_TAG).as_py() == ""

    # A native Scalar names the same row under the message's own root, the
    # identities it states included.
    assert FixMsg(message.field, message.value, seed) == message


def test_a_venues_field_and_msgtype_are_both_reachable_from_a_venue_message() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("MsgType", "utf8", 35),
            _field("TradeID", "utf8", 5001, branches=["cme"], aliases=["VenueTrade"]),
            _field("Symbol", "utf8", 55, aliases=["Ticker"]),
        ]
    )
    # The venue re-spells `Symbol` under its own tag: one field, an
    # alternate tag, and a second alias.
    assert registry.add_field(_field("Symbol", "utf8", 5055, branches=["cme"], aliases=["VenueTicker"])) is False
    root = Field(
        "VenueOrder",
        DataType.from_fields(
            [
                Field("MsgType", "utf8"),
                Field("TradeID", "utf8"),
                Field("Symbol", "utf8"),
            ]
        ),
        nullable=False,
    )
    message = FixMsg(
        root, {"MsgType": "D", "TradeID": "T-1", "Symbol": "AAPL"}, registry
    )

    # One namespace: the venue's field, its aliases, and the specification's
    # own resolve alike, and no step depends on who contributed what.
    assert message.by_tag(5001).as_py() == "T-1"
    assert message.by_name("venuetrade").as_py() == "T-1"
    assert message.by_name("venueticker").as_py() == "AAPL"
    assert message.by_tag(5055).as_py() == "AAPL"
    assert message.by_tag(35).as_py() == "D"
    assert message.by_name("ticker").as_py() == "AAPL"

    # An identifier is exact.
    assert message.by_id(registry.field_by_tag(5001).fix.id).as_py() == "T-1"
    assert message.by_id(registry.field_by_tag(35).fix.id).as_py() == "D"
    assert message.get_by_id(_field("TradeID", "utf8", 5002).fix.id) is None
    # A message root the codec or a caller builds is not a dictionary member.
    assert message.field.fix.branches == []
    assert registry.field_by_tag(5001).fix.has_branch("cme")

    # A message that carries none of the venue's fields still resolves the
    # standard ones and misses the rest.
    plain = Field(
        "Order",
        DataType.from_fields([Field("MsgType", "utf8"), Field("Account", "utf8")]),
        nullable=False,
    )
    standard = FixMsg(plain, {"MsgType": "D", "Account": "A1"}, registry)
    assert standard.by_tag(35).as_py() == "D"
    assert standard.get_by_tag(5001) is None
    assert standard.get_by_name("venuetrade") is None


def test_message_refuses_a_value_its_field_refuses(seed: FixRegistry) -> None:
    root = Field(
        "row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False
    )
    # A text field reads any value that spells text, a number included, so
    # what it refuses is a value with no spelling at all. The dictionary folds
    # its names, so the field the refusal names is `symbol`.
    with pytest.raises(ValueError, match="symbol"):
        FixMsg(root, {"symbol": [1]})
    with pytest.raises(ValueError):
        FixMsg(Field("scalar", "utf8"), {"symbol": "AAPL"})


def test_message_links_the_process_default_when_none_is_named() -> None:
    default = global_registry()
    assert isinstance(default, FixRegistry)
    # Whatever this machine has installed, the two calls answer one registry.
    assert default == global_registry()

    root = Field(
        "row", DataType.from_fields([_field("symbol", "utf8", 55)]), nullable=False
    )
    linked = FixMsg(root, {"symbol": "AAPL"})
    assert linked.registry == default
    # An explicit registry is kept instead.
    explicit = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    assert FixMsg(root, {"symbol": "AAPL"}, explicit).registry == explicit


def test_message_is_hashable_copyable_and_picklable(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)
    same = FixMsg(root, ORDER_VALUE, seed)

    assert message == same
    assert hash(message) == hash(same)
    assert message.stable_hash() == same.stable_hash()
    assert len({message, same}) == 1
    assert message != FixMsg(root, {**ORDER_VALUE, "symbol": "MSFT"}, seed)
    assert message != object()

    assert copy.copy(message) == message
    assert copy.deepcopy(message) == message
    restored = pickle.loads(pickle.dumps(message))
    assert restored == message
    assert restored.registry == seed
    assert restored.by_path("parties[0].partyid").as_py() == "BROKER"

    assert repr(message) == 'FixMsg("NewOrderSingle", 12 values)'
    assert repr(seed) == f"FixRegistry({6241 + CRATED} fields)"
    # A new registry is never empty: it holds the crate's own fields and the
    # two seeded standard clocks.
    assert repr(FixRegistry()) == f"FixRegistry({SEEDED} fields)"


INSTALL_SCRIPT = """
import pathlib
import sys

from yggdryl import DataType, Field
from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

seed = FixRegistry.from_handle(pathlib.Path(sys.argv[1]))
install_global_registry(seed)
assert global_registry() == seed
assert global_registry().field_by_tag(55).name == "symbol"
assert global_registry().field_by_name("SYMBOL").name == "symbol"

root = Field(
    "row",
    DataType.from_fields([global_registry().field_by_tag(55)]),
    nullable=False,
)
assert FixMsg(root, {"symbol": "AAPL"}).registry == seed

try:
    install_global_registry(FixRegistry())
except ValueError as error:
    assert "already resolved" in str(error), error
else:
    raise AssertionError("installing twice must fail")
print("ok")
"""


def test_install_global_registry_wins_before_the_default_resolves() -> None:
    """Process-wide state, so it is driven in a process of its own."""
    result = subprocess.run(
        [sys.executable, "-c", INSTALL_SCRIPT, str(SEED)],
        capture_output=True,
        text=True,
        cwd=REPO,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip().endswith("ok")


def test_scalar_value_and_field_stay_the_native_ones(seed: FixRegistry) -> None:
    message = FixMsg(_order(seed), ORDER_VALUE, seed)

    assert isinstance(message.value, Scalar)
    assert isinstance(message.field, Field)
    assert isinstance(message[55], Scalar)
    assert message.value.kind == "sequence"
    assert message.field.fix.tag is None
    assert message.field.fix.id is None


def test_reader_parses_every_frame_shape_the_core_reads(seed: FixRegistry) -> None:
    """One reader, five entry points, and each is the core's own."""
    reader = _fixed(seed)

    framed = next(reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|"))
    assert framed.by_tag(55).as_py() == "AAPL"
    assert next(reader.parse_line(b"8=FIX.4.4|35=D|55=AAPL|10=0|")).by_tag(55).as_py() == "AAPL"
    assert reader.parse_fix_line(b"8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01").by_tag(
        55
    ).as_py() == "AAPL"
    assert reader.parse_pairs([("55", "AAPL")]).by_tag(55).as_py() == "AAPL"

    # A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
    # whose value packs its members behind the two control bytes ULLINK uses.
    bridge = reader.parse_ullink_line(
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )
    inferred = next(reader.parse_line(
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|"
    ))
    assert inferred == bridge
    assert bridge.by_tag(55).as_py() == "TTF"
    assert bridge.by_tag(38).as_py() == 1200.0
    assert bridge.by_tag(44).as_py() == 41.25
    party = bridge.party("1")
    assert party is not None
    assert party[0] is not None and party[0].as_py() == "BUYSIDE"
    # The counter says two occurrences and one arrived: reported, not repaired.
    assert len(bridge.anomalies()) == 1
    assert "453" in bridge.anomalies()[0]


def test_arrow_reader_uses_separatorless_group_inference(seed: FixRegistry) -> None:
    bridge = (
        b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|"
    )
    parsed = (
        _fixed(seed)
        .parse_text_arrow_reader(pa.table({"body": pa.array([bridge], pa.binary())}))
        .read_all()
    )
    # The counter and the group it plans are two columns, each spelled by the
    # dictionary's folded name: the count itself, and the occurrences beside it.
    assert parsed.column("nopartyids").to_pylist() == [1]
    parties = parsed.column("parties").to_pylist()
    assert len(parties) == 1 and len(parties[0]) == 1
    assert parties[0][0]["partyid"] == "BUYSIDE"
    assert parties[0][0]["partyidsource"] == "D"
    assert parties[0][0]["partyrole"] == 1


def test_reader_takes_the_pins_the_core_takes(seed: FixRegistry) -> None:
    """A version and the spellings that mean nothing was sent."""
    assert FixCodec(seed).registry == seed

    # Tag 32 is `lastshares` at 4.2 and `lastqty` from 4.3 on. A pin settles how
    # a value is read, never what a field is called: the column is the
    # dictionary's own whatever version read the row, and the 4.2 spelling
    # still reaches it as an alias.
    dated = _fixed(seed, version="4.2")
    named = next(dated.parse_line(b"8=FIX.4.4|35=8|32=100|10=0|"))
    assert named.field.index_of("lastqty") is not None
    assert named.get_by_name("lastshares") is not None
    assert named.get_by_name("lastqty") is not None

    # A stated absence produces no field at all.
    silent = _fixed(seed, null_values=["<none>"])
    assert next(silent.parse_line(b"8=FIX.4.4|35=D|55=<none>|10=0|")).get_by_tag(55) is None

    # There is no dialect to pin: the dictionary is one namespace.
    with pytest.raises(TypeError):
        FixCodec(seed, branch="cme")  # type: ignore[call-arg]
    assert not hasattr(FixCodec(seed), "branch")


def test_the_default_sending_time_is_the_clock_undated_intake_takes() -> None:
    """The pin that makes a parse of undated bytes repeat.

    Pinned by ``fixed_intake_clock_settles_native_hard_values_and_replays_exactly``,
    ``message_sending_and_transact_clocks_precede_the_fixed_default`` and
    ``fixed_default_clock_intake_is_exact_and_atomic`` in
    ``rust/tests/fix/content_identity.rs``.
    """
    registry = FixRegistry()
    assert FixCodec(registry).default_sending_time is None
    assert FixCodec(registry, default_sending_time=None).default_sending_time is None
    codec = FixCodec(registry, default_sending_time=CLOCK)
    assert codec.default_sending_time == CLOCK
    # An aware UTC `datetime` is read once into the same nanosecond clock.
    assert FixCodec(registry, default_sending_time=CLOCK_INSTANT).default_sending_time == CLOCK

    # Undated bytes take it, every settled clock follows it, and two parses
    # of the same bytes are one message.
    wire = b"8=FIX.4.4|35=0|10=0|"
    first = next(codec.parse_line(wire))
    second = next(codec.parse_line(wire))
    assert first == second
    assert first.uuid() == second.uuid()
    assert first.digest() == second.digest()
    assert first.updatedat() == CLOCK
    assert first.createdat() == CLOCK
    assert first.by_tag(SNAPSHOTAT_TAG) == CLOCK
    assert first.by_tag(52) == CLOCK
    assert first.by_tag(CODE_TAG).as_py() == ""
    assert first.into_bytes(ord("|")) == wire

    # A message's own clocks precede the pin: SendingTime is the stated one,
    # and TransactTime settles the event, the update and the creation.
    stated = next(
        codec.parse_line(
            b"8=FIX.4.4|35=0|52=19700101-00:00:02.123456789|60=19700101-00:00:03.987654321|10=0|"
        )
    )
    event = Scalar.datetime(3_987_654_321, "ns", "UTC")
    assert stated.by_tag(52) == Scalar.datetime(2_123_456_789, "ns", "UTC")
    assert stated.by_tag(60) == event
    assert stated.by_tag(SNAPSHOTAT_TAG) == event
    assert stated.updatedat() == event
    assert stated.createdat() == event

    # The pin is exact: a clock of another unit, a naive one or text is the
    # core's refusal, located at the option.
    for refused in (
        Scalar.datetime(0, "us", "UTC"),
        Scalar.datetime(0, "ns"),
        "1970-01-01T00:00:00Z",
    ):
        with pytest.raises(ValueError, match="default_sending_time"):
            FixCodec(registry, default_sending_time=refused)


def test_a_reader_fills_what_the_line_implied_and_leaves_the_wire_alone(
    seed: FixRegistry,
) -> None:
    """Enrichment is a call; the rules are the core's."""
    reader = _fixed(seed)

    # A `SecurityID` an ISIN's check digit closes has stated its source, and
    # under that source the crate's `isincode` column and the country its
    # prefix names.
    line = b"8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|"
    filled = reader.enrich_message(next(reader.parse_line(line)))
    assert filled.by_tag(22).as_py() == "4"
    assert filled.by_tag(65013).as_py() == "US0378331005"
    assert filled.by_tag(470).as_py() == "US"
    # An order stating no time in force is a day order.
    assert filled.by_tag(59).as_py() == "0"

    # Unfilled, the line states none of them.
    bare = next(reader.parse_line(line))
    assert bare.get_by_tag(22) is None
    assert bare.get_by_tag(65013) is None
    assert bare.get_by_tag(470) is None

    # Only the row was filled: the wire comes back byte for byte, and a second
    # pass changes nothing.
    assert filled.into_bytes(ord("|")) == line
    assert reader.enrich_message(filled) == filled

    # A value no standard closes answers nothing rather than a guess.
    opaque = reader.enrich_message(
        next(reader.parse_line(b"8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|"))
    )
    assert opaque.get_by_tag(22) is None
    assert opaque.get_by_tag(65013) is None


def test_identifier_membership_and_enriched_altids_are_the_core_answers(seed: FixRegistry) -> None:
    assert seed.msgtype("D").field.fix.identifiers == [
        "clordid", "secondaryclordid", "allocid", "quoteid", "reforderid", "refclordid"
    ]
    assert seed.msgtype("8").field.fix.identifiers == [
        "orderid", "secondaryorderid", "secondaryclordid", "secondaryexecid",
        "clordid", "origclordid", "quoterespid", "listid", "execid", "execrefid",
        "allocid", "reforderid", "refclordid"
    ]
    codec = _fixed(seed)
    wire = b"8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|"
    original = codec.parse_fix_line(wire)
    filled = codec.enrich_message(original)
    expected = {"clordid": "C-001", "execid": "E-09", "orderid": "O-01"}
    assert filled.by_tag(65020).as_py() == expected
    assert list(filled.by_name("AltIds").as_py()) == list(expected)
    assert filled.entries() == original.entries()
    assert filled.into_bytes(124) == wire
    assert filled.digest() == original.digest()
    assert codec.enrich_message(filled) == filled
    schema = fix_schema(seed)
    restored = FixMsg.from_row(schema, filled.into_row(schema), seed)
    assert restored.by_name("altids") == filled.by_name("altids")
    assert restored.entries() == filled.entries()
    stamped = next(codec.lifecycle([filled]))
    assert stamped.by_name("altids").as_py() == expected
    assert stamped.entries() == filled.entries()
    assert stamped.digest() == filled.digest()


@pytest.mark.parametrize("stated", [{}, {"venue": "001"}])
def test_stated_altids_maps_are_preserved_even_when_empty(seed: FixRegistry, stated: dict[str, str]) -> None:
    codec = _fixed(seed)
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C-1|10=0|")
    before = message.entries(), message.into_bytes(124), message.digest()
    message.set("altids", stated)
    filled = codec.enrich_message(message)
    assert filled.by_tag(65020).as_py() == stated
    assert (filled.entries(), filled.into_bytes(124), filled.digest()) == before
    assert codec.enrich_message(filled) == filled


def test_identifier_maps_distinguish_known_empty_unknown_and_nested_messages(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    for code in (b"0", b"D"):
        filled = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=" + code + b"|10=0|"))
        assert filled.by_tag(65020).as_py() == {}
    unknown = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=ZZ|11=C-1|10=0|"))
    assert unknown.get_by_tag(65020) is None
    nested = codec.enrich_message(next(codec.parse_line(
        b"MSGTYPE=E|#LISTID=L-1|#NOORDERS=1|#NOORDERS[0]=CLORDID=C-nested\x04\x03SYMBOL=EXAMPLE"
    )))
    assert nested.by_tag(65020).as_py() == {"listid": "L-1"}


# A FIX 4.2 execution report: a transaction type, a partial fill, a Rule80A
# capacity and two identities the specification later moved into `Parties`.
REPORT = (
    b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|"
    b"14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|"
)


def test_the_enriching_pass_restates_before_it_fills(seed: FixRegistry) -> None:
    """Restatement is the pass's first step; the rules are the dictionary's."""
    codec = _fixed(seed)
    read = next(codec.parse_line(REPORT))
    assert read.by_tag(65001).as_py() == "4.2"
    assert read.by_tag(150).as_py() == "40PARTFILL"
    assert read.get_by_tag(528) is None
    assert read.get_by_tag(453) is None

    latest = codec.enrich_message(read)
    # ExecTransType Cancel wrote ExecType TradeCancel over the retired
    # PartiallyFilled, and the source stays.
    assert latest.by_tag(150).as_py() == "40TRDCXL"
    assert latest.by_tag(20).as_py() == "1"
    # Rule80A A is an agency order.
    assert latest.by_tag(528).as_py() == "A"
    assert latest.by_tag(47).as_py() == "A"
    # ExecBroker and ClientID are two parties, in tag order, counted.
    assert latest.by_tag(453).as_py() == 2
    assert latest.by_path("parties[0].partyid").as_py() == "BRKR"
    assert latest.by_path("parties[0].partyrole").as_py() == 1
    assert latest.by_path("parties[1].partyid").as_py() == "CLIENT1"
    assert latest.by_path("parties[1].partyrole").as_py() == 3
    # The fill under its newest spelling, reachable by the old one too, and
    # reachable is all it is: the registry answers a field for any alias it
    # holds, so an alias is a way of asking rather than a child to store.
    assert latest.by_tag(32).as_py() == 100.0
    assert latest.by_name("LastShares").as_py() == 100.0
    assert [name for name, _ in latest].count("lastqty") == 1
    assert not any(name == "lastshares" for name, _ in latest)
    # The row speaks the dictionary's newest version; the wire still says 4.2.
    assert latest.by_tag(65001).as_py() == "5.0.2"
    assert latest.by_tag(8).as_py() == "FIX.4.2"

    # One pass, and the filling read the restated row: a report stating no
    # time in force is a day order, one fill's average is that fill's price,
    # and what it was worth is the quantity times the price.
    assert latest.by_tag(59).as_py() == "0"
    assert latest.by_tag(6).as_py() == 10.5
    assert latest.by_tag(381).as_py() == 1050.0

    # Only the row was touched: the wire comes back byte for byte, the arrival
    # record and the anomalies are the same, and a second pass changes nothing.
    assert latest.into_bytes(ord("|")) == REPORT
    assert latest.entries() == read.entries()
    assert latest.anomalies() == read.anomalies()
    assert codec.enrich_message(latest) == latest


# A Jolokia answer as a log line writes it: a timestamp and a reader in front
# of the document, the duration the call took behind it. Both are prose.
LOGGED = (
    '2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":'
    '"com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_OrderRouting,'
    'plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"Router_OrderRouting",'
    '"Version":"4.7.0","Category":"Fix BuySide","SenderCompID":"CLI.PROD.TRD",'
    '"TargetCompID":"ST.PROD","BeginString":"FIX.4.4","PrimaryHost":"172.97.127.90",'
    '"CurrentPort":9726,"State":"logged","Type":"I","NeedCFBReload":false,'
    '"cm-extension":"4.7.0","IncomingMsgSeqNum":18336},"status":200} (12 ms)'
).encode()

# A wildcard read: one answer, a plugin per key, each named by its ObjectName.
WILDCARD = (
    '{"request": {"mbean": "com.ullink.ulbridge.sessioninterfaces.plugins:*", "type": "read"},'
    ' "value": {"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_BDG_DMZ_PCO,'
    'plugin-type=FIX,type=ConfigurationPlugin": {"Comment": "", "Category": "InterBridge",'
    ' "Prefix": "", "Name": "ULMSG_BROKER_BDG_DMZ_PCO", "LoadIsolation": 0, "Suffix": "",'
    ' "PriorityLevel": 5, "Version": "2.0.3"},'
    ' "com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,'
    'plugin-type=FIX,type=Plugin": {"Name": "ULMSG_BROKER_TO_DMZ", "Version": "4.7.0"}},'
    ' "status": 200}'
).encode()


@pytest.fixture
def bridge(seed: FixRegistry) -> FixRegistry:
    """The committed dictionary, plus the bridge's own vocabulary."""
    registry = seed
    registry.with_plugin_fields()
    return registry


def test_the_plugin_vocabulary_is_a_caller_s_choice(bridge: FixRegistry) -> None:
    """Registering the plugin fields is the one thing that types a report."""
    fields = fix_plugin_fields()
    assert fields, "the plugin dictionary publishes its own vocabulary"
    assert PLUGIN_DIALECT == "plugin"
    assert all(field.fix.branches == [PLUGIN_DIALECT] for field in fields)
    # The membership folds, like every other name in this crate.
    assert all(field.fix.has_branch("Plugin") for field in fields)
    # Every one of them is in the dictionary that folded them, carrying the
    # membership it declared, and a dictionary that never folded them holds
    # none of them.
    for field in fields:
        held = bridge.field_by_name(field.name)
        assert held.fix.tag == field.fix.tag
        assert held.fix.branches == [PLUGIN_DIALECT]
        assert bridge.field_by_id(field.fix.id) == held
    assert bridge.dialects() == [PLUGIN_DIALECT]
    assert FixRegistry.from_handle(SEED).get_field_by_name(fields[0].name) is None
    assert FixRegistry.from_handle(SEED).dialects() == []


def test_a_bridge_document_is_read_out_of_the_line_that_carries_it(
    bridge: FixRegistry,
) -> None:
    """The reader reads to the document's own close, not to the line's end."""
    # The document is JSON, which is what it is: what makes one a bridge
    # configuration is a shape the codec reads, not a name the classifier
    # gives it.
    assert MimeType.infer_bytes(LOGGED) == MimeType.JSON
    assert FixCodec.infer_msgtype_bytes(LOGGED) == b"Plugin"

    reader = _fixed(bridge)
    message = next(reader.parse_plugin_line(LOGGED))
    # FIX's own names stay FIX's and the bridge's own are the bridge's, both
    # inside the occurrence the document answered for. The registry is one
    # namespace, so the plugin's own `Version` and `State` - not the FIX
    # version a row was read at, nor the order's state - are held under the
    # bridge's `PluginVersion` and `PluginState`.
    assert message.by_path("SenderCompID").as_py() == "CLI.PROD.TRD"
    assert message.by_path("PluginVersion").as_py() == "4.7.0"
    assert message.by_path("PluginState").as_py() == "logged"
    assert message.by_path("Version") == message.by_tag(65001)
    # The registered vocabulary types a port as a number and a flag as a flag.
    assert message.by_path("CurrentPort").as_py() == 9726
    assert message.by_path("NeedCFBReload").as_py() is False
    # A configuration message is the plugin's attributes and nothing the
    # Jolokia answer wrapped them in: `MBean`, `Operation`, `Status` and
    # `Error` - tags 20001 to 20004 - are deleted, and the ObjectName the
    # read named it by stays where it always belonged, on `SessionInterface`.
    for retired in (20001, 20002, 20003, 20004):
        assert message.get_by_tag(retired) is None
    for retired_name in ("MBean", "Operation", "Status", "Error"):
        assert message.get_by_name(retired_name) is None
    assert message.by_name("SessionInterface").as_py().endswith("type=Plugin")
    assert message.by_name("MBeanType").as_py() == "Plugin"
    # The entries are what arrived, and the envelope was never one of them:
    # the re-emission opens on the ObjectName and states none of the four.
    wire = message.into_bytes(ord("|"))
    assert wire.startswith(b"SessionInterface=com.ullink.ulbridge")
    for retired_key in (b"MBean=", b"Operation=", b"Status=", b"Error="):
        assert retired_key not in wire
    # `PLUGIN_TAG_MIN` is 20001 still - the floor of the range this
    # dictionary claims, not the smallest tag it defines, which is 20010.
    assert min(field.fix.tag for field in fix_plugin_fields()) == 20010
    # `parse_line` finds the same document behind the same prose.
    assert next(reader.parse_line(LOGGED)) == message


# A bridge line on one plugin, with no comp ids of its own: what ten million
# lines behind one configuration look like.
def _plugin_row(plugin: str) -> bytes:
    return f"MSGTYPE=8|ACCOUNT=ACCT-000117|PLUGINID={plugin}|".encode()


def test_a_configuration_is_a_message_the_crate_registered(bridge: FixRegistry) -> None:
    """A configuration types as `pluginconfig` and the wire still says no 35."""
    message = next(_fixed(bridge).parse_line(LOGGED))
    # The name the crate registered the code under, and the code itself.
    assert PLUGINCONFIG_CODE_NAME == ("UCFG", "pluginconfig")
    assert message.field.name == PLUGINCONFIG_CODE_NAME[1]
    assert message.by_tag(35).as_py() == PLUGINCONFIG_CODE_NAME[0]

    # Registering it is nobody's choice: a registry that never asked for the
    # plugin fields still holds the message, because a component holds its
    # members by value and the code is the crate's own (decision 19).
    plain = FixRegistry()
    assert plain.msgtype("UCFG").name == "pluginconfig"
    held = plain.definition("components", "pluginconfig")
    assert held.name == fix_plugin_message().name
    assert held.dtype == fix_plugin_message().dtype
    assert held.fix.msgtype == PLUGINCONFIG_CODE_NAME[0]
    assert plain.get_field_by_tag(20010) is None
    assert plain.dialects() == []
    # FIX's own `MsgType` opens the component, the plugin attributes follow,
    # and the three FIX fields a configuration also states close it.
    members = [fix_plugin_message()[at].name for at in range(len(fix_plugin_message()))]
    assert members[0] == "MsgType"
    assert members[1:-3] == [field.name for field in fix_plugin_fields()]
    assert members[-3:] == ["BeginString", "SenderCompID", "TargetCompID"]

    # A built child, not a pair: the document sent no `35=`, so the arrival
    # record holds none and the wire re-emits exactly as it did before the
    # type existed (decision 17 still holds).
    wire = message.into_bytes(ord("|"))
    assert b"35=" not in wire
    assert b"MsgType" not in wire
    assert wire.startswith(b"SessionInterface=com.ullink.ulbridge")


def test_the_enriching_stream_fills_a_row_from_the_configuration_that_named_its_plugin(
    bridge: FixRegistry,
) -> None:
    """A bridge states its two ends once; the lines behind it name only the plugin."""
    codec = _fixed(bridge)

    def one(body: bytes) -> FixMsg:
        return next(codec.parse_line(body))

    named = "Router_OrderRouting"
    config = one(LOGGED)

    # Alone, a row naming a plugin states no comp ids and gains none: there
    # is nothing yet to fill them from.
    (bare,) = codec.enrich_messages([one(_plugin_row(named))])
    assert bare.get_by_tag(49) is None
    assert bare.get_by_tag(56) is None

    # Behind the configuration that named it, the same row takes the
    # session's two ends (decision 19).
    filled = list(codec.enrich_messages([config, one(_plugin_row(named))]))
    assert filled[0].field.name == "pluginconfig"
    assert filled[1].by_tag(49).as_py() == "CLI.PROD.TRD"
    assert filled[1].by_tag(56).as_py() == "ST.PROD"
    # Not the begin string: every built message already fills tag 8 from the
    # version its row was read at, so there is never one absent to fill.
    assert filled[1].by_tag(8) == bare.by_tag(8)

    # A row that stated its own 49 keeps it, which is what makes the pass
    # idempotent; the one it did not state is still filled.
    stated = f"MSGTYPE=8|PLUGINID={named}|SENDERCOMPID=ITS.OWN|".encode()
    held = list(codec.enrich_messages([config, one(stated)]))[1]
    assert held.by_tag(49).as_py() == "ITS.OWN"
    assert held.by_tag(56).as_py() == "ST.PROD"

    # A row naming a plugin no configuration named gains nothing, and so
    # does one naming no plugin at all.
    for untouched in (_plugin_row("Someone_Else"), b"MSGTYPE=8|ACCOUNT=ACCT-000117|"):
        last = list(codec.enrich_messages([config, one(untouched)]))[1]
        assert last.get_by_tag(49) is None
        assert last.get_by_tag(56) is None

    # The memory is the stream's: one message is not a stream, so the door
    # that takes one remembers nothing and fills nothing.
    assert codec.enrich_message(one(_plugin_row(named))).get_by_tag(49) is None


def test_every_plugin_a_document_answers_for_crosses_both_ways(
    bridge: FixRegistry,
) -> None:
    """A wildcard read, a single read, and the message each crosses to."""
    walk = Plugin.from_json_bytes(WILDCARD)
    assert isinstance(walk, Plugins)
    assert iter(walk) is walk
    held = list(walk)
    assert len(held) == 2
    assert held[0].name == "ULMSG_BROKER_BDG_DMZ_PCO"
    assert held[0].mbean_type == "ConfigurationPlugin"
    assert held[0].plugin_type == "FIX"
    assert held[0].category == "InterBridge"
    assert held[0].version == "2.0.3"
    assert held[1].name == "ULMSG_BROKER_TO_DMZ"
    assert held[1].mbean_type == "Plugin"

    # The spelling is folded the way every other name in this crate is, and
    # `in` answers on the same fold as `get`.
    assert held[0].get("priority_level").as_py() == 5
    assert "PriorityLevel" in held[0]
    assert "priority_level" in held[0]
    assert "nothing_stated" not in held[0]
    # `attributes` keeps the document's own spelling; the fold is what `get`
    # and `in` are for.
    assert len(held[0]) == len(held[0].attributes)
    assert held[0].attributes["Version"].as_py() == "2.0.3"

    single = list(Plugin.from_json_bytes(LOGGED))
    assert len(single) == 1
    assert single[0].name == "Router_OrderRouting"
    assert single[0].state == "logged"
    assert single[0].mbean is not None and "Router_OrderRouting" in single[0].mbean

    # A parsed document is the same walk as the bytes it was parsed from, and
    # anything the Scalar boundary reads is a parsed document.
    assert list(Plugin.from_json_scalar(json.loads(WILDCARD))) == held

    # And back to a typed message, and out of one again: the crossing keeps
    # the ObjectName, the attributes and their types.
    reader = _fixed(bridge)
    message = held[0].into_fixmsg(reader)
    assert message.by_path("PriorityLevel").as_py() == 5
    back = Plugin.from_fixmsg(message)
    assert back.mbean == held[0].mbean
    assert back.name == held[0].name
    assert back.version == held[0].version


def test_a_plugin_is_an_immutable_value(bridge: FixRegistry) -> None:
    """Equality, hash, copy and pickle, the way every other value here is."""
    plugin = next(Plugin.from_json_bytes(LOGGED))
    same = next(Plugin.from_json_bytes(LOGGED))
    assert plugin == same
    assert hash(plugin) == hash(same)
    assert plugin.stable_hash() == same.stable_hash()
    assert len({plugin, same}) == 1
    assert plugin != next(Plugin.from_json_bytes(WILDCARD))
    assert plugin != object()

    assert copy.copy(plugin) == plugin
    assert copy.deepcopy(plugin) == plugin
    assert pickle.loads(pickle.dumps(plugin)) == plugin
    assert "Router_OrderRouting" in repr(plugin)

    # Built from the parts a caller has, rather than from a document.
    built = Plugin({"Name": "Local", "Version": "1.0"}, mbean=plugin.mbean)
    assert built.name == "Local"
    assert built.mbean == plugin.mbean
    assert built != plugin
    # A mapping is folded into the record a document would have made, so a
    # hand-built plugin answers the way a read one does.
    assert built.get("name").as_py() == "Local"
    assert built.attributes.keys() == {"Name", "Version"}
    with pytest.raises(TypeError):
        Plugin({1: "not a name"})

    # A body that is not a Jolokia answer names no plugin, and answering
    # none is what it answers: reading is not refusing, so every one of
    # these iterates empty rather than raising - bytes that are not JSON at
    # all included.
    for silent in (
        b"[]",
        b"{}",
        b'{"a":1}',
        b"null",
        b"true",
        b"1",
        b'"text"',
        b"no document here at all",
    ):
        assert list(Plugin.from_json_bytes(silent)) == [], silent


def test_the_fixed_row_is_named_by_fold_and_never_shifts(seed: FixRegistry) -> None:
    """The one shape a whole capture lands in."""
    schema = fix_schema(seed, "FixMessage")
    columns = [child.name for child in schema]
    assert columns[:3] == [
        "beginstring",
        "bodylength",
        "msgtype",
    ], "named by the dictionary's folded names, in message order"
    assert columns[-9:] == [
        "prevupdatedat",
        "prevuuid",
        "createdat",
        "code",
        "snapshotat",
        "sourceurl",
        "msgdirection",
        "nofixentries",
        "fixentries",
    ], "and the one arrival record closes it"
    # The tag stays the identity: each column carries its field's, in order.
    assert fix_schema_tags()[:3] == [8, 9, 35]
    assert [child.fix.tag for child in schema][:3] == [8, 9, 35]

    # A column is found by the name the dictionary spells, never by digits.
    assert schema.index_of("msgtype") == 2
    assert schema.index_of("35") is None
    assert schema.index_of("999999") is None
    assert schema.name == "FixMessage"
    # The crate's own columns are spelled the same way, with the FIX-style
    # spelling kept as the display: twenty-six definitions after the trailer,
    # tags 65001 to 65026 with the `altids` Map at 65020, then FIX's own
    # `msgdirection`, read from the line where the wire states none, then the
    # arrival record under the counter that counts it
    # (``rust/tests/fix/schema.rs``).
    assert len(fix_schema_tags()) == 107
    assert fix_schema_tags()[-28:] == [*range(65001, 65027), 385, 65027]
    assert [child.fix.tag for child in schema][-29:] == [
        *range(65001, 65027),
        385,
        65027,
        None,
    ]
    assert columns.count("altids") == 1
    altids = schema[schema.index_of("altids")]
    assert altids.nullable and altids.fix.counter == 65020
    assert altids.into_arrow().type.equals(pa.map_(pa.string(), pa.string(), keys_sorted=True))
    # The retired columns are gone rather than renamed.
    for retired in ("msghash", "timestamp", "instid", "id", "persistentid", "nounmappedfixentries"):
        assert schema.index_of(retired) is None, retired
    for name, display in (
        ("updatedat", "UpdatedAt"),
        ("sendersessionid", "SenderSessionId"),
        ("instuuid", "InstUuid"),
        ("uuid", "Uuid"),
        ("puuid", "PUuid"),
        ("prevupdatedat", "PrevUpdatedAt"),
        ("prevuuid", "PrevUuid"),
    ):
        assert schema[schema.index_of(name)].display == display, name
    # The three columns a row derives from what the message said are typed
    # as the thing they hold, never as the text a venue spelled it in.
    assert schema[schema.index_of("isincode")].dtype == DataType("isin")
    assert schema[schema.index_of("miccode")].dtype == DataType("mic")
    assert schema[schema.index_of("state")].dtype == DataType("state")
    assert schema[schema.index_of("prevupdatedat")].dtype == schema[schema.index_of("updatedat")].dtype
    assert schema[schema.index_of("prevuuid")].dtype == DataType("fixedbinary(16)")
    assert schema[schema.index_of("prevupdatedat")].nullable
    assert schema[schema.index_of("prevuuid")].nullable
    # BeginString, the partition and the seven members of the settled bundle
    # are declared non-null; every other column is nullable, because a message
    # that carried nothing there answers null rather than shifting its
    # neighbours.
    assert [child.name for child in schema if not child.nullable] == [
        "beginstring",
        "sendingtime",
        "updatedat",
        "timepartition",
        "uuid",
        "puuid",
        "createdat",
        "code",
        "snapshotat",
    ]

    reader = _fixed(seed)
    wire = b"8=FIX.4.4|35=D|11=A|9999=x|VenueOwnThing=y|10=0|"
    message = next(reader.parse_line(wire))
    row = message.into_row(schema).as_py()
    assert len(row) == len(columns)
    assert row[schema.index_of("beginstring")] == "FIX.4.4"
    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("clordid")] == "A"
    assert row[schema.index_of("version")] == "4.4"
    # A message with no clock of its own settles on the codec's default
    # SendingTime, and the partition follows it: never null.
    for clock in ("sendingtime", "updatedat", "createdat", "snapshotat"):
        assert row[schema.index_of(clock)] == CLOCK_INSTANT, clock
    assert row[schema.index_of("timepartition")] == CLOCK_PARTITION
    assert row[schema.index_of("code")] == ""
    for identity in ("uuid", "puuid"):
        held = row[schema.index_of(identity)]
        assert isinstance(held, bytes) and len(held) == 16, identity
    # The message identity leads with `updatedat`'s signed nanoseconds, sign
    # bit flipped, so the bytes order as the instants do.
    ordered = (CLOCK_NS ^ (1 << 63)) & ((1 << 64) - 1)
    assert row[schema.index_of("uuid")][:8] == ordered.to_bytes(8, "big")
    # The projected row's uuid is recomputed over what the row holds; the
    # retained code keeps the chain's puuid.
    assert row[schema.index_of("puuid")] == message.puuid().as_py()
    assert row[schema.index_of("sendersessionid")] is None
    # A derived column a message gives nothing for is null, never a shift.
    assert row[schema.index_of("state")] is None
    assert row[schema.index_of("prevupdatedat")] is None
    assert row[schema.index_of("prevuuid")] is None

    # The arrival record closes the row with everything that arrived, in
    # arrival order: a key no dictionary explains records tag 0 and its raw
    # key (``the_row_stays_lossless_and_says_what_nothing_explained``).
    arrived = reader.arrow_reader(schema, [message]).read_all().column("fixentries").to_pylist()[0]
    assert [entry["tag"] for entry in arrived] == [8, 35, 11, 0, 0, 10]
    assert [entry["key"] for entry in arrived if entry["tag"] == 0] == ["9999", "VenueOwnThing"]
    assert message.into_bytes(ord("|")) == wire


def test_a_captures_own_columns_lead_the_row(seed: FixRegistry) -> None:
    """Where a line was read from is what a monitor orders and joins on."""
    carrier = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", DataType("utf8")),
                Field("body", DataType("binary")),
            ]
        ),
        nullable=False,
    )
    plain = fix_schema(seed, "FixMessage")
    carried = fix_schema_carrying(carrier, plain)

    assert [child.name for child in carried][:2] == ["url", "body"]
    assert len(carried) == len(plain) + 2
    assert carried.index_of("msgtype") == plain.index_of("msgtype") + 2

    # A column no tag names is the capture's, so a row answers null there: the
    # capture fills it, and nothing in the message says what it held.
    message = next(_fixed(seed).parse_line(b"8=FIX.4.4|35=D|10=0|"))
    row = message.into_row(carried).as_py()
    assert row[0] is None
    assert row[carried.index_of("msgtype")] == "D"
    # A required capture column the message cannot fill refuses the projection
    # rather than publishing a null into it.
    strict = fix_schema_carrying(
        Field(
            "line",
            DataType.from_fields([Field("url", DataType("utf8"), nullable=False)]),
            nullable=False,
        ),
        plain,
    )
    with pytest.raises(ValueError, match=r"\$\.url"):
        message.into_row(strict)

    # A capture column whose folded name a FIX column takes is not carried in
    # front: `senderSessionId` and `sendersessionid` are one name, and the FIX column is
    # the one a reader spelling it means.
    named = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", DataType("utf8"), nullable=False),
                Field("senderSessionId", DataType("utf8")),
                Field("body", DataType("binary"), nullable=False),
            ]
        ),
        nullable=False,
    )
    folded = fix_schema_carrying(named, plain)
    names = [child.name for child in folded]
    assert names[:2] == ["url", "body"]
    assert "senderSessionId" not in names
    assert names.count("sendersessionid") == 1
    assert len(folded) == len(plain) + 2


def test_the_crate_fields_declare_their_own_protocols() -> None:
    """The partition says what it derives from; the identities and clocks what they hold.

    The listing is pinned by ``the_crate_carries_fields_of_its_own_from_65000``
    in ``rust/tests/fix/digest.rs``.
    """
    fields = {field.name: field for field in fix_crate_fields()}
    assert len(fields) == CRATED + 1 == 27
    # In tag order, one block from 65001, above every tag FIX or a venue
    # publishes, and none is a dictionary's contribution. The retired 65000
    # is not reused.
    assert list(fields) == [
        "version",
        "symbolticker",
        "updatedat",
        "timepartition",
        "parentclordid",
        "parentorderid",
        "sendersessionid",
        "msgctxid",
        "pluginid",
        "prevpluginid",
        "sendersessionname",
        "targetsessionname",
        "isincode",
        "miccode",
        "state",
        "instuuid",
        "uuid",
        "puuid",
        "targetsessionid",
        "altids",
        "prevupdatedat",
        "prevuuid",
        "createdat",
        "code",
        "snapshotat",
        "sourceurl",
        "nofixentries",
    ]
    assert [field.display for field in fields.values()] == [
        "Version",
        "SymbolTicker",
        "UpdatedAt",
        "TimePartition",
        "ParentClOrdID",
        "ParentOrderID",
        "SenderSessionId",
        "MsgCtxId",
        "PluginId",
        "PrevPluginId",
        "SenderSessionName",
        "TargetSessionName",
        "ISINCode",
        "MICCode",
        "State",
        "InstUuid",
        "Uuid",
        "PUuid",
        "TargetSessionId",
        "AltIds",
        "PrevUpdatedAt",
        "PrevUuid",
        "CreatedAt",
        "Code",
        "SnapshotAt",
        "SourceUrl",
        "NoFixEntries",
    ]
    assert [field.fix.tag for field in fields.values()] == list(range(65001, 65028))
    assert all(field.fix.branches == [] for field in fields.values())
    assert [field.fix.id for field in fields.values()] == [
        _field(name, "utf8", tag).fix.id for name, tag in zip(fields, range(65001, 65028))
    ]
    # The settled bundle's crate members are non-null as fields; every other
    # one is nullable, and every one says what it holds.
    assert [name for name, field in fields.items() if not field.nullable] == [
        "updatedat",
        "uuid",
        "puuid",
        "createdat",
        "code",
        "snapshotat",
    ]
    assert all(field.description is not None for field in fields.values())

    # The clocks are instants in UTC, to the nanosecond; the code is text.
    for name in ("updatedat", "prevupdatedat", "createdat", "snapshotat"):
        assert fields[name].dtype == DataType('datetime64(ns,"UTC")'), name
    assert fields["code"].dtype == DataType("utf8")
    # Where a line was read from is the URL it is, so a row joins on it.
    assert fields["sourceurl"].fix.tag == SOURCEURL_TAG
    assert fields["sourceurl"].dtype == DataType("url")
    # The arrival record is a group, so it has a counter like any other.
    assert fields["nofixentries"].fix.tag == NOFIXENTRIES_TAG
    assert fields["nofixentries"].dtype == DataType("int32")
    # The partition names the column it reads by that column's name.
    held = fields["timepartition"]
    assert held.dtype == DataType('datetime64(ns,"UTC")')
    assert held.metadata["partition:sources"] == '["updatedat"]'
    assert held.metadata["transform:expression"] == "truncate(updatedat, 'hour')"

    # What a bridge's own log states about a line - the session the message
    # itself names, its message context, the plugin that logged it and the one
    # it came through before that, and the two session names the line spells -
    # is text, like the identifiers, and the two session names answer to the
    # spellings a bridge row writes them under.
    for name in (
        "version",
        "symbolticker",
        "parentclordid",
        "parentorderid",
        "sendersessionid",
        "msgctxid",
        "pluginid",
        "prevpluginid",
        "sendersessionname",
        "targetsessionname",
    ):
        assert fields[name].dtype == DataType("utf8"), name
    assert fields["sendersessionname"].fix.aliases == ["ULFromSessionName"]
    assert fields["targetsessionname"].fix.aliases == ["ULToSessionName"]
    # The ISIN, the market and the order's state are typed as the thing they
    # hold.
    assert fields["isincode"].dtype == DataType("isin")
    assert fields["miccode"].dtype == DataType("mic")
    assert fields["state"].dtype == DataType("state")
    # The instrument, the message, the chain and the previous message are
    # sixteen plain bytes - what every lake engine reads as `fixed[16]` -
    # and no alias reaches them.
    for name in ("instuuid", "uuid", "puuid", "prevuuid"):
        assert fields[name].dtype == DataType("fixedbinary(16)"), name
        assert fields[name].fix.aliases == [], name

    # Every registry holds them from construction beside the two seeded
    # standard clocks, and a bridge row spelling `SESSIONID` or
    # `ULFROMSESSIONNAME` reaches them by name. The listing is the very
    # definition a registry answers.
    registry = FixRegistry()
    assert len(registry) == SEEDED == 26
    assert registry.dialects() == []
    for name, field in fields.items():
        if name == "altids":
            assert registry.definition("groups", name) == field
            assert registry.group_by_tag(65020) == field
            assert registry.get_field_by_name(name) is None
            assert registry.get_field_by_id(field.fix.id) is None
            assert registry.get_field_by_tag(65020) is None
            continue
        assert registry.field_by_name(name) == field
        assert registry.field_by_id(field.fix.id) == field
        assert registry.field_by_tag(field.fix.tag) == field
    assert registry.field_by_tag(65007).name == "sendersessionid"
    assert registry.field_by_name("SenderSessionId").name == "sendersessionid"
    assert registry.field_by_name("ULFROMSESSIONNAME").name == "sendersessionname"
    # The seeds are ordinary definitions, typed as the clock the bundle reads.
    for tag, name, display in ((52, "sendingtime", "SendingTime"), (60, "transacttime", "TransactTime")):
        seeded = registry.field_by_tag(tag)
        assert (seeded.name, seeded.display) == (name, display)
        assert seeded.dtype == DataType('datetime64(ns,"UTC")')
    # The retired names reach nothing, and 65000 is no field's tag.
    for retired in ("instid", "id", "persistentid", "timestamp", "msghash"):
        assert registry.get_field_by_name(retired) is None, retired
    assert registry.get_field_by_tag(65000) is None


def _root_names(message: FixMsg) -> list[str]:
    """The names of a message's root children, in the order it declares."""
    return [child.name for child in message.field]


def test_every_built_message_carries_its_version_and_its_settled_bundle(
    seed: FixRegistry,
) -> None:
    """A message the codec built opens with its version and closes with the settled bundle."""
    reader = _fixed(seed)

    # The header in rank order whatever the input order, the body, the
    # version the read used, the trailer, and the bundle the shared finalizer
    # appends (``the_header_orders_first_and_the_trailer_last_whatever_the_input_order``).
    message = next(reader.parse_line(b"8=FIX.4.4|55=AAPL|35=D|9=100|10=000|"))
    assert _root_names(message) == [
        "beginstring",
        "bodylength",
        "msgtype",
        "symbol",
        "version",
        "checksum",
        *BUNDLE,
    ]
    pairs = [(name, value) for name, value in message]
    assert [name for name, _ in pairs] == _root_names(message)
    assert pairs[0][0] == "beginstring" and pairs[0][1].as_py() == "FIX.4.4"
    assert pairs[-1][0] == "sendingtime"
    assert len(message) == 13
    assert message.by_tag(65001).as_py() == "4.4"
    assert message.by_tag(8).as_py() == "FIX.4.4"
    assert message.by_id(seed.field_by_tag(UPDATEDAT_TAG).fix.id) == message.by_name("updatedat")
    assert message.by_tag(UPDATEDAT_TAG) == message.updatedat()

    # A message with no clock of its own settles on the codec's default
    # SendingTime, and every settled clock and the partition follow it.
    assert message.updatedat() == CLOCK
    assert message.updatedat().as_py() == CLOCK_INSTANT
    assert message.createdat() == CLOCK
    assert message.by_tag(SNAPSHOTAT_TAG) == CLOCK
    assert message.by_tag(52) == CLOCK
    assert message.by_tag(CODE_TAG).as_py() == ""
    partition = message.time_partition()
    assert partition is not None and partition.as_py() == CLOCK_PARTITION

    # The bundle is children and never entries: the wire re-emits byte for
    # byte, and nothing the row added is in the arrival record.
    wire = b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=000|"
    order = next(reader.parse_line(wire))
    assert order.into_bytes(ord("|")) == wire
    assert {tag for tag, _, _ in order.entries()} == {8, 35, 11, 55, 54, 38, 10}

    # The message's own clocks: SendingTime(52) is the stated one, and
    # TransactTime(60) outranks it for the event, so the update, the creation
    # and the snapshot are 60's; a sub-second clock still has a partition.
    clocked = next(
        reader.parse_line(
            b"8=FIX.4.4|35=8|52=20260102-09:30:00.500|60=20260102-09:29:59.250|10=0|"
        )
    )
    instant = dt.datetime(2026, 1, 2, 9, 29, 59, 250000, tzinfo=dt.timezone.utc)
    sent = dt.datetime(2026, 1, 2, 9, 30, 0, 500000, tzinfo=dt.timezone.utc)
    assert clocked.by_tag(52).as_py() == sent
    assert clocked.updatedat().as_py() == instant
    assert clocked.createdat().as_py() == instant
    assert clocked.by_tag(SNAPSHOTAT_TAG).as_py() == instant
    partition = clocked.time_partition()
    assert partition is not None
    assert partition.as_py() == dt.datetime(2026, 1, 2, 9, 0, tzinfo=dt.timezone.utc)

    # A message that stated no version is read at one all the same, and the
    # version it was read at is not sent: it is not an entry either.
    stated = reader.parse_pairs([("55", "AAPL")])
    assert _root_names(stated)[0] == "beginstring"
    assert _root_names(stated)[-1] == "sendingtime"
    assert stated.by_tag(8).as_py().startswith("FIX.")
    assert {tag for tag, _, _ in stated.entries()} == {55}
    assert not stated.into_bytes(ord("|")).startswith(b"8=")

    # A bridge frame and a FIXML row are built the same way.
    bridge = reader.parse_ullink_line(b"|#SYMBOL=TTF|#SIDE=1|")
    assert _root_names(bridge)[0] == "beginstring"
    assert _root_names(bridge)[-len(BUNDLE) :] == BUNDLE
    assert bridge.updatedat() == CLOCK


def test_the_settled_bundle_answers_directly_and_refuses_what_would_break_it(
    seed: FixRegistry,
) -> None:
    """``updatedat``, ``createdat``, ``uuid`` and ``puuid`` always answer.

    Pinned by ``rust/tests/fix/content_identity.rs``: code-only ``puuid``,
    content ``uuid`` excluding the creation instant, and atomic refusals of a
    mandatory null, a mandatory removal and a stated identity that disagrees.
    """
    reader = _fixed(seed)
    message = next(reader.parse_line(b"8=FIX.4.4|35=D|11=A1|55=ALPHA|54=1|10=0|"))
    for reader_name, tag in (
        ("updatedat", UPDATEDAT_TAG),
        ("createdat", CREATEDAT_TAG),
        ("uuid", UUID_TAG),
        ("puuid", PUUID_TAG),
    ):
        answered = getattr(message, reader_name)()
        assert isinstance(answered, Scalar)
        assert not answered.is_null(), reader_name
        assert answered == message.by_tag(tag), reader_name
        assert answered == message.by_name(reader_name), reader_name
    assert message.updatedat().dtype == DataType('datetime64(ns,"UTC")')
    assert message.createdat().dtype == DataType('datetime64(ns,"UTC")')
    for identity in (message.uuid(), message.puuid()):
        assert identity.dtype == DataType("fixedbinary(16)")
        held = identity.as_py()
        assert isinstance(held, bytes) and len(held) == 16
    # The unknown code is the empty name, and every message naming no chain
    # hashes that same empty name.
    assert message.puuid() == next(reader.parse_line(b"8=FIX.4.4|35=0|10=0|")).puuid()

    # The creation instant is not content; the settled update instant and
    # the named content are, and `puuid` hashes the code alone.
    changed = copy.copy(message)
    changed.set("createdat", Scalar.datetime(CLOCK_NS - 10, "ns", "UTC"))
    assert changed.uuid() == message.uuid()
    assert changed != message
    changed.set("updatedat", Scalar.datetime(CLOCK_NS + 1, "ns", "UTC"))
    assert changed.uuid() != message.uuid()
    before = changed.uuid()
    changed.set(55, "BETA")
    assert changed.uuid() != before
    assert changed.puuid() == message.puuid()
    changed.set("code", "chain")
    assert changed.puuid() != message.puuid()
    # Writing SendingTime does not reread or reset the settled clocks.
    settled = changed.updatedat()
    changed.set(52, Scalar.datetime(CLOCK_NS + 2, "ns", "UTC"))
    assert changed.updatedat() == settled

    # A mandatory field refuses null and removal, and a stated identity the
    # row does not hash to is refused; each leaves the message as it was.
    for name in BUNDLE:
        untouched = copy.copy(message)
        with pytest.raises(ValueError, match=name):
            untouched.set(name, None)
        assert untouched == message, name
        with pytest.raises(ValueError, match=name):
            untouched.remove(name)
        assert untouched == message, name
    for name in ("uuid", "puuid"):
        untouched = copy.copy(message)
        with pytest.raises(ValueError, match=name):
            untouched.set(name, "00000000-0000-0000-0000-000000000000")
        assert untouched == message, name
    # An ordinary child still leaves, and the content identity moves with it.
    removed = copy.copy(message)
    assert removed.remove(55) == Scalar("ALPHA")
    assert removed.uuid() != message.uuid()
    assert removed.remove("absent") is None

    # Enrichment carries the settled clocks and is idempotent
    # (``enrichment_keeps_settled_clocks_and_arrival_record_and_finalizes_once_per_result``).
    enriched = reader.enrich_message(message)
    assert enriched.updatedat() == message.updatedat()
    assert enriched.createdat() == message.createdat()
    assert enriched.by_tag(52) == message.by_tag(52)
    assert enriched.entries() == message.entries()
    assert enriched.digest() == message.digest()
    assert reader.enrich_message(enriched) == enriched


def test_a_rows_own_columns_feed_the_message(seed: FixRegistry) -> None:
    """A capture's clock is context; its other columns fill the fields their names reach."""
    clock = dt.datetime(2026, 1, 2, 10, 15, 30, 500000, tzinfo=dt.timezone.utc)
    source = pa.table(
        {
            "timestamp": pa.array([clock, None], pa.timestamp("us", tz="UTC")),
            "senderSessionId": pa.array(["e7254b22", None], pa.utf8()),
            "seqNum": pa.array([4507, None], pa.int64()),
            "body": pa.array(
                [
                    b"8=FIX.4.4|35=D|55=AAPL|10=0|",
                    b"8=FIX.4.4|35=0|34=696|52=20260102-09:29:59.250|10=0|",
                ],
                pa.binary(),
            ),
        }
    )
    parsed = _fixed(seed).parse_text_arrow_reader(source).read_all()
    names = parsed.schema.names

    # The capture's own columns lead the row and the fixed columns follow. A
    # capture whose folded name a fixed column takes is not carried in front,
    # it fills that column: `senderSessionId`. No fixed column is named
    # `timestamp` any more, so the capture's clock is carried as context like
    # `seqNum`, which fills `msgseqnum` besides (decision 26).
    assert names[:3] == ["timestamp", "seqNum", "body"]
    assert names[3:6] == ["beginstring", "bodylength", "msgtype"]
    assert "senderSessionId" not in names
    for once in ("timestamp", "sendersessionid", "msgseqnum"):
        assert names.count(once) == 1, once
    assert names[-3:] == ["msgdirection", "nofixentries", "fixentries"]
    assert parsed.column("seqNum").to_pylist() == [4507, None]

    # The capture's clock stamps nothing: the wire's event clock settles the
    # message, else the codec's default SendingTime, and the partition
    # follows. The capture column keeps what the capture said.
    instant = dt.datetime(2026, 1, 2, 9, 29, 59, 250000, tzinfo=dt.timezone.utc)
    assert parsed.column("timestamp").to_pylist() == [clock, None]
    assert parsed.column("updatedat").to_pylist() == [CLOCK_INSTANT, instant]
    assert parsed.column("sendingtime").to_pylist() == [CLOCK_INSTANT, instant]
    assert parsed.column("timepartition").to_pylist() == [
        CLOCK_PARTITION,
        dt.datetime(2026, 1, 2, 9, 0, tzinfo=dt.timezone.utc),
    ]

    # A column spelled `senderSessionId` is the crate's `sendersessionid` under the fold,
    # so it fills that column; the sequence fills `MsgSeqNum` where the frame
    # stated none and never where it did.
    assert parsed.column("sendersessionid").to_pylist() == ["e7254b22", None]
    assert parsed.column("msgseqnum").to_pylist() == [4507, 696]

    # Row-only: what the row filled is never an entry, so the arrival record
    # is exactly what the frame carried.
    entries = parsed.column("fixentries").to_pylist()
    assert {entry["tag"] for entry in entries[0]} == {8, 35, 55, 10}
    assert {entry["tag"] for entry in entries[1]} == {8, 35, 34, 52, 10}


def test_a_rows_pluginid_fills_its_field_and_selects_nothing(
    seed: FixRegistry,
) -> None:
    """A row's `pluginid` fills the crate's field; the dictionary is one namespace."""
    crated = {field.name: field.fix.tag for field in fix_crate_fields()}
    # A venue's field, stamped as the venue's, resolves for every row
    # whatever plugin logged it: membership is provenance, not a namespace.
    seed.insert(_field("VenueTag", "utf8", 5001, branches=["venue"]))
    codec = _fixed(seed)
    body = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark"
    spellings = ["venue", "VNU", "OMS_X1_TradeCapture", None, ""]
    previous = ["ULFilter", None, None, None, None]
    source = pa.table(
        {
            "pluginid": pa.array(spellings, pa.utf8()),
            "prevpluginid": pa.array(previous, pa.utf8()),
            "body": pa.array([body] * len(spellings), pa.binary()),
        }
    )
    parsed = codec.parse_text_arrow_reader(source).read_all()
    names = parsed.schema.names

    # Both columns are fixed columns' names, so neither is carried in front:
    # each fills the column of its own name, exactly as the row spelled it,
    # and only the payload leads.
    assert names[0] == "body"
    assert names[1:4] == ["beginstring", "bodylength", "msgtype"]
    for once in ("pluginid", "prevpluginid"):
        assert names.count(once) == 1, once
    assert parsed.column("pluginid").to_pylist() == spellings
    assert parsed.column("prevpluginid").to_pylist() == previous

    # The plugin selects nothing: the venue's key maps to its field on every
    # row - one whose plugin spells the venue's name, one that spells another
    # plugin, a null, an empty string - and no arrival is left unresolved,
    # which the one arrival record would say with tag 0.
    entries = parsed.column("fixentries").to_pylist()
    for row in range(len(spellings)):
        assert [entry["tag"] for entry in entries[row]] == [35, 11, 5001], row

    # One line read alone answers exactly what the batch did, and a fill is
    # never an entry: neither plugin is one.
    lined = _fixed(seed, capture_names=["pluginid"])
    for spelled in spellings:
        message = next(lined.parse_text_line(TextLine(0, body, [spelled])))
        assert message.by_tag(5001).as_py() == "dark", spelled
        assert message.field.fix.branches == [], spelled
        held = message.get_by_name("pluginid")
        assert (held.as_py() if held is not None else None) == spelled, spelled
        assert all(
            tag not in (crated["pluginid"], crated["prevpluginid"])
            for tag, _, _ in message.entries()
        ), spelled

    # The two session names are only ever what the line itself spells, through
    # the aliases a bridge writes them under: neither the plugin that logged
    # the row nor the direction it moved in fills them, and nothing fills
    # `prevpluginid` but a column of that name.
    spoken = pa.table(
        {
            "msgdirection": pa.array(["Send", "R"], pa.utf8()),
            "pluginid": pa.array(["venue", "vnu"], pa.utf8()),
            "body": pa.array(
                [
                    b"MSGTYPE=D|CLORDID=A",
                    b"|#SYMBOL=TTF|#ULFROMSESSIONNAME=OMS_X1_OrderOut"
                    b"|#ULTOSESSIONNAME=ULMSG_BROKER_BDG_DMZ_CLI|",
                ],
                pa.binary(),
            ),
        }
    )
    spoken = codec.parse_text_arrow_reader(spoken).read_all()
    # A stated `msgdirection` is the row's direction, read as a parameter and
    # stored as the code of the set: FIX's own column carries it and no
    # second column repeats it.
    assert spoken.schema.names[:1] == ["body"]
    assert spoken.schema.names.count("msgdirection") == 1
    assert spoken.column("msgdirection").to_pylist() == ["S", "R"]
    assert spoken.column("sendersessionname").to_pylist() == [None, "OMS_X1_OrderOut"]
    assert spoken.column("targetsessionname").to_pylist() == [None, "ULMSG_BROKER_BDG_DMZ_CLI"]
    assert spoken.column("sendersessionid").to_pylist() == [None, None]
    assert spoken.column("prevpluginid").to_pylist() == [None, None]


# The messages of one order's life, as a venue and its client tell it: the
# order under the client's identifier, its acknowledgement under the venue's,
# half of it done, a replace whose new identifier names the old one, its
# acknowledgement, and the fill under the new identifier alone.
LIFE = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|",
    b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|44=12.6|60=20260102-10:15:32.000|10=0|",
    b"8=FIX.4.4|35=8|41=A1|11=A2|37=O1|17=E3|150=5|39=5|55=AAPL|207=XNAS|15=USD|38=120|14=50|151=70|60=20260102-10:15:32.100|10=0|",
    b"8=FIX.4.4|35=8|11=A2|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=120|14=120|151=0|32=70|31=12.6|60=20260102-10:15:33.000|10=0|",
]


def _identity_bytes(message: FixMsg, tag: int) -> bytes | None:
    """The sixteen bytes one identity column holds, or ``None`` where it is null."""
    held = message.get_by_tag(tag)
    if held is None or held.is_null():
        return None
    value = held.as_py()
    assert isinstance(value, bytes) and len(value) == 16, value
    return value


def _ns(count: int) -> Scalar:
    """One nanosecond UTC instant, the clock every settled field holds."""
    return Scalar.datetime(count, "ns", "UTC")


def _previous(message: FixMsg, expected: FixMsg | None) -> None:
    """That a filled message's previous pair is ``expected``'s own, or null."""
    if expected is None:
        assert message.by_tag(PREVUPDATEDAT_TAG).is_null()
        assert message.by_tag(PREVUUID_TAG).is_null()
        return
    assert message.by_tag(PREVUPDATEDAT_TAG) == expected.updatedat()
    assert message.by_tag(PREVUUID_TAG) == expected.uuid()


def _event(registry: FixRegistry, nanos: int, code: str) -> FixMsg:
    """A message at one instant whose chain its code alone names, as ``lifecycle_grid.rs`` builds one."""
    clock = _ns(nanos)
    root = Field(
        "event",
        DataType.from_fields(
            [
                registry.field_by_tag(tag)
                for tag in (52, UPDATEDAT_TAG, CREATEDAT_TAG, SNAPSHOTAT_TAG, CODE_TAG)
            ]
        ),
        nullable=False,
    )
    return FixMsg(root, [clock, clock, clock, clock, code], registry)


def test_every_message_of_one_order_carries_the_chains_identity_until_it_ends(
    seed: FixRegistry,
) -> None:
    """The lifecycle pass is the core's; Python feeds it one message at a time.

    Pinned by ``every_message_of_one_order_carries_the_chains_identity_until_it_ends``
    and ``instrument_payload_keeps_its_recipe_and_chain_payload_is_only_the_code``
    in ``rust/tests/fix/lifecycle.rs``.
    """
    reader = _fixed(seed)
    life = FixLifecycle(seed)
    assert life.interval_ns == FixLifecycle.DEFAULT_INTERVAL_NS
    stamped: list[FixMsg] = []
    for line in LIFE:
        stamped.append(life.fill(next(reader.parse_line(line))))
        # Alive from the first message to the fill that ends it.
        assert life.alive() == int(len(stamped) < len(LIFE))

    # One instrument, one chain, six messages: the replace's new identifier
    # joined the chain the old one opened.
    instruments = [_identity_bytes(held, INSTUUID_TAG) for held in stamped]
    assert instruments[0] is not None
    assert all(held == instruments[0] for held in instruments)
    chains = [_identity_bytes(held, PUUID_TAG) for held in stamped]
    assert chains[0] is not None
    assert all(held == chains[0] for held in chains)
    # The chain is named by its instrument scope and its first identifier,
    # and `puuid` hashes that name.
    assert stamped[0].by_tag(CODE_TAG).as_py() == f"{instruments[0].hex()}/A1"
    assert all(held.by_tag(CODE_TAG) == stamped[0].by_tag(CODE_TAG) for held in stamped)
    # The first creation instant survives the replacement and the terminal
    # fill: the order's own transaction time.
    assert all(held.createdat() == stamped[0].createdat() for held in stamped)
    assert stamped[0].createdat() == stamped[0].by_tag(60)
    assert stamped[0].by_tag(60).as_py() == dt.datetime(2026, 1, 2, 10, 15, 30, tzinfo=dt.timezone.utc)
    # Every message has its own identity, and identities sort by the grid
    # instant `updatedat` is floored to; `snapshotat` keeps the real one.
    ids = [held.uuid() for held in stamped]
    assert all(_identity_bytes(held, UUID_TAG) is not None for held in stamped)
    for earlier, later in zip(stamped, stamped[1:]):
        assert earlier.uuid() != later.uuid()
        if earlier.updatedat() < later.updatedat():
            assert earlier.uuid() < later.uuid()
    assert stamped[1].updatedat().as_py() == dt.datetime(2026, 1, 2, 10, 15, 30, tzinfo=dt.timezone.utc)
    assert stamped[1].by_tag(SNAPSHOTAT_TAG).as_py() == dt.datetime(
        2026, 1, 2, 10, 15, 30, 250000, tzinfo=dt.timezone.utc
    )
    # Each message carries its predecessor's settled instant and identity.
    _previous(stamped[0], None)
    for earlier, later in zip(stamped, stamped[1:]):
        _previous(later, earlier)
    # A state column holds the ranked spelling, never the wire's code.
    assert stamped[1].by_tag(39).as_py() == "20NEW"
    assert stamped[5].by_tag(39).as_py() == "80FILLED"

    # Nothing here is an entry: the wire re-emits byte for byte.
    for line, message in zip(LIFE, stamped):
        assert message.into_bytes(ord("|")) == line

    # Reusing the code tomorrow is the same chain identity but a fresh live
    # incarnation: its own creation instant, and no previous message.
    tomorrow = LIFE[0].replace(b"20260102", b"20260103")
    again = life.fill(next(reader.parse_line(tomorrow)))
    assert again.puuid() == stamped[0].puuid()
    assert again.createdat() == again.by_tag(60)
    assert again.createdat() != stamped[0].createdat()
    assert again.by_tag(PREVUUID_TAG).is_null()
    assert life.alive() == 1
    life.clear()
    assert life.alive() == 0
    assert life.interval_ns == FixLifecycle.DEFAULT_INTERVAL_NS
    assert repr(life) == "FixLifecycle(0 alive)"
    # The same line at the same instant is the same chain identity, which is
    # what makes two reads of one capture agree.
    replayed = life.fill(next(reader.parse_line(LIFE[0])))
    assert replayed.puuid() == stamped[0].puuid()
    assert replayed.uuid() == ids[0]
    assert replayed.createdat() == stamped[0].createdat()

    # The state moves, so nothing about it hashes.
    with pytest.raises(TypeError):
        hash(life)


def test_a_message_naming_no_order_has_an_id_and_no_chain(seed: FixRegistry) -> None:
    reader = _fixed(seed)
    heartbeat = next(reader.parse_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|"))
    # The codec runs one lifecycle over any iterable, a generator included,
    # and answers the stream lazily.
    stream = reader.lifecycle(held for held in [heartbeat])
    assert isinstance(stream, FixMessages)
    stamped = list(stream)
    assert len(stamped) == 1
    (held,) = stamped
    # Every message has an id; no identifier, no chain name, and the empty
    # name every unnamed message hashes; no instrument, no identity.
    assert _identity_bytes(held, UUID_TAG) is not None
    assert held.by_tag(CODE_TAG).as_py() == ""
    undated = next(reader.parse_line(b"8=FIX.4.4|35=0|10=0|"))
    assert held.puuid() == undated.puuid()
    assert _identity_bytes(held, INSTUUID_TAG) is None
    _previous(held, None)
    # The event clock is the sending time where no transaction time is
    # stated, and the codec's default where the message states no clock.
    assert held.updatedat() == held.by_tag(52)
    (settled,) = list(reader.lifecycle([undated]))
    assert settled.updatedat() == settled.by_tag(52) == CLOCK

    # An element that is not a message is refused where it is met: the first
    # message is answered, the stray line raises in its place.
    mixed = reader.lifecycle([heartbeat, b"8=FIX.4.4|35=0|10=0|"])
    assert next(mixed) == held
    with pytest.raises(TypeError):
        next(mixed)
    assert next(mixed, None) is None

    # The process default is the registry a lifecycle built over nothing uses.
    assert FixLifecycle().alive() == 0


def test_the_instrument_identity_is_the_same_across_spellings_and_venues(
    seed: FixRegistry,
) -> None:
    reader = _fixed(seed)
    life = FixLifecycle(seed)

    def identity(line: bytes) -> bytes | None:
        return _identity_bytes(life.fill(next(reader.parse_line(line))), INSTUUID_TAG)

    # An ISIN outranks a symbol, so the same security under two symbols is
    # one instrument, and case is not a difference.
    by_isin = identity(b"8=FIX.4.4|35=D|11=B1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|10=0|")
    by_isin_again = identity(
        b"8=FIX.4.4|35=D|11=B2|48=us0378331005|22=4|55=APPLE|207=xnas|15=usd|10=0|"
    )
    assert by_isin is not None and by_isin == by_isin_again
    # Another market is another instrument identity.
    elsewhere = identity(b"8=FIX.4.4|35=D|11=B3|48=US0378331005|22=4|55=AAPL|207=XLON|15=USD|10=0|")
    assert by_isin != elsewhere
    # Without an ISIN the symbol stands in, and a stated one wins over a
    # symbol that would say otherwise.
    by_symbol = identity(b"8=FIX.4.4|35=D|11=B4|55=AAPL|207=XNAS|15=USD|10=0|")
    assert by_symbol is not None and by_symbol != by_isin
    # A bridge row names the same facts under its own keys.
    bridged = identity(b"#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B5|")
    assert bridged == by_isin


def test_a_stamped_stream_read_again_keeps_what_it_carries(seed: FixRegistry) -> None:
    reader = _fixed(seed)
    once = list(reader.lifecycle(next(reader.parse_line(line)) for line in LIFE))
    twice = list(reader.lifecycle(once))
    assert len(once) == len(twice) == len(LIFE)
    for first, second in zip(once, twice):
        for tag in (INSTUUID_TAG, UUID_TAG, PUUID_TAG):
            assert _identity_bytes(first, tag) == _identity_bytes(second, tag), tag
        assert first.createdat() == second.createdat()
        assert len(first.entries()) == len(second.entries())
    assert once == twice


def test_a_batch_read_runs_one_lifecycle_over_the_whole_capture(seed: FixRegistry) -> None:
    """A stage is a call: the lifecycle composes over the messages a batch holds."""
    codec = _fixed(seed)
    source = pa.table({"body": pa.array(LIFE, pa.binary())})
    read = codec.parse_text_arrow_reader(source)
    schema = Field.from_arrow_schema(read.schema, "fix")
    parsed = codec.arrow_reader(schema, codec.lifecycle(codec.messages(read))).read_all()
    assert parsed.schema.names == read.schema.names, "the same schema in and out"
    chains = parsed.column("puuid").to_pylist()
    assert len(chains) == len(LIFE)
    assert chains[0] is not None and all(held == chains[0] for held in chains)
    codes = parsed.column("code").to_pylist()
    assert codes[0] != "" and all(held == codes[0] for held in codes)
    created = parsed.column("createdat").to_pylist()
    assert all(held == created[0] for held in created)
    assert len(set(parsed.column("uuid").to_pylist())) == len(LIFE)
    previous = parsed.column("prevuuid").to_pylist()
    assert previous[0] is None and all(held is not None for held in previous[1:])
    # A state column holds the ranked spelling, never the wire's code, as the
    # text its datatype stores.
    assert parsed.column("ordstatus").to_pylist()[1:3] == ["20NEW", "40PARTFILL"]
    # Not filled unless asked: every row names no chain, so every row hashes
    # the one empty name, and none carries a previous message.
    bare = codec.parse_text_arrow_reader(source).read_all()
    assert bare.column("code").to_pylist() == [""] * len(LIFE)
    assert len(set(bare.column("puuid").to_pylist())) == 1
    assert bare.column("puuid").to_pylist()[0] != chains[0]
    assert bare.column("prevuuid").to_pylist() == [None] * len(LIFE)
    # Enrichment is another call over the same stream, and the two compose.
    both = codec.arrow_reader(
        schema, codec.lifecycle(codec.enrich_messages(codec.messages(codec.parse_text_arrow_reader(source))))
    ).read_all()
    assert both.column("puuid").to_pylist() == chains
    assert both.column("state").to_pylist()[1] == "20NEW"


def test_a_stream_of_lines_through_enrichment_and_the_lifecycle_carries_its_chain(
    seed: FixRegistry,
) -> None:
    """Pinned by ``composed_fallible_stages_are_lazy_preserve_errors_and_fuse_exhaustion``."""
    codec = _fixed(seed)
    bodies = [
        b"8=FIX.4.4|35=D|11=A|65024=stream|52=20260102-10:15:30|10=0|",
        b"8=FIX.4.4|35=D|11=A|65024=stream|52=20260102-10:15:31|10=0|",
    ]
    pulled = 0

    def lines() -> Iterator[TextLine]:
        nonlocal pulled
        for index, body in enumerate(bodies):
            pulled += 1
            yield TextLine(index, body)

    pipeline = codec.lifecycle(codec.enrich_messages(codec.parse_text_lines(lines())))
    assert pulled == 0
    first = next(pipeline)
    assert pulled == 1
    # A wire stating the crate's `code` names its chain itself.
    assert first.by_tag(CODE_TAG).as_py() == "stream"
    last = next(pipeline)
    assert pulled == 2
    assert last.by_tag(PREVUUID_TAG) == first.uuid()
    assert last.createdat() == first.createdat()
    assert last.puuid() == first.puuid()
    assert next(pipeline, None) is None
    assert next(pipeline, None) is None


def test_the_lifecycle_cadence_is_positive_atomic_and_kept_by_clear() -> None:
    """Pinned by ``cadence_is_positive_atomic_and_retained_by_clear`` in ``rust/tests/fix/lifecycle_grid.rs``."""
    registry = FixRegistry()
    assert FixLifecycle.DEFAULT_INTERVAL_NS == 1_000_000_000
    life = FixLifecycle(registry)
    assert life.interval_ns == FixLifecycle.DEFAULT_INTERVAL_NS
    for invalid in (0, -1, -(2**63)):
        with pytest.raises(ValueError, match="interval_ns"):
            life.set_interval_ns(invalid)
        assert life.interval_ns == FixLifecycle.DEFAULT_INTERVAL_NS
        assert life.alive() == 0
    life.set_interval_ns(10)
    life.fill(_event(registry, 1, "A"))
    # Repeating the interval changes nothing, even while a chain is live; a
    # different one waits until none is.
    life.set_interval_ns(10)
    for invalid in (0, -1, 20):
        with pytest.raises(ValueError, match="interval_ns"):
            life.set_interval_ns(invalid)
        assert life.interval_ns == 10
        assert life.alive() == 1
    life.clear()
    assert life.interval_ns == 10
    assert life.alive() == 0
    life.set_interval_ns(2**63 - 1)
    assert life.interval_ns == 2**63 - 1
    assert FixLifecycle(registry, interval_ns=10).interval_ns == 10
    with pytest.raises(ValueError, match="interval_ns"):
        FixLifecycle(registry, interval_ns=0)


@pytest.mark.parametrize(
    "time, grid",
    [(-11, -20), (-10, -10), (-1, -10), (0, 0), (9, 0), (10, 10), (11, 10)],
)
def test_a_message_takes_its_epoch_grid_instant_and_keeps_its_real_one(time: int, grid: int) -> None:
    """Pinned by ``epoch_floor_uses_negative_buckets_and_boundary_belongs_to_the_bucket_it_opens``."""
    registry = FixRegistry()
    raw = _event(registry, time, "A")
    filled = FixLifecycle(registry, interval_ns=10).fill(raw)
    assert filled.updatedat() == _ns(grid)
    assert filled.createdat() == _ns(time)
    assert filled.by_tag(SNAPSHOTAT_TAG) == _ns(time)
    # The message handed in is not the one answered.
    assert raw.updatedat() == _ns(time)
    # A snapshot is answered only for an arrival off its grid.
    answered = FixLifecycle(registry, interval_ns=10).snapshot(raw)
    if time == grid:
        assert answered is None
    else:
        assert answered == filled


def test_the_full_and_the_filtered_door_share_one_history_and_consume_aligned_buckets() -> None:
    """Pinned by ``full_and_filtered_doors_share_finalized_history_and_consume_aligned_buckets``."""
    registry = FixRegistry()
    full = FixLifecycle(registry, interval_ns=10)
    filtered = FixLifecycle(registry, interval_ns=10)
    last: FixMsg | None = None
    for time, grid, emit in ((1, 0, True), (7, 0, False), (10, 10, False), (11, 10, False), (21, 20, True)):
        raw = _event(registry, time, "A")
        filled = full.fill(raw)
        _previous(filled, last)
        assert filled.updatedat() == _ns(grid)
        assert filled.createdat() == _ns(1)
        assert filled.by_tag(SNAPSHOTAT_TAG) == _ns(time)
        answered = filtered.snapshot(raw)
        if emit:
            assert answered == filled, time
        else:
            assert answered is None, time
        last = filled
    assert full.alive() == 1
    assert filtered.alive() == 1

    # An already-aligned first arrival emits nothing but consumes its bucket,
    # and still establishes the chain's creation instant.
    aligned = FixLifecycle(registry, interval_ns=10)
    opening = _event(registry, 10, "B")
    opening.set("createdat", _ns(77))
    assert aligned.snapshot(opening) is None
    assert aligned.snapshot(_event(registry, 19, "B")) is None
    after = aligned.snapshot(_event(registry, 21, "B"))
    assert after is not None
    assert after.createdat() == _ns(77)


def test_a_live_chain_keeps_its_first_arrivals_creation_instant() -> None:
    """Pinned by ``creation_is_the_first_arrivals_statement_not_the_minimum_or_grid``."""
    registry = FixRegistry()
    life = FixLifecycle(registry, interval_ns=10)
    other = FixLifecycle(registry, interval_ns=10)
    last: FixMsg | None = None
    for time, stated in ((21, 987), (1, -123), (31, 432)):
        raw = _event(registry, time, "A")
        raw.set("createdat", _ns(stated))
        restated = copy.copy(raw)
        restated.set("createdat", _ns(654))
        comparison = other.fill(restated)
        filled = life.fill(raw)
        assert filled.createdat() == _ns(987)
        assert comparison.createdat() == _ns(654)
        assert filled.updatedat() == _ns(time // 10 * 10)
        assert filled.by_tag(SNAPSHOTAT_TAG) == _ns(time)
        # The creation instant is no part of either identity.
        assert filled.uuid() == comparison.uuid()
        assert filled.puuid() == comparison.puuid()
        _previous(filled, last)
        last = filled


def test_snapshots_replay_exactly_and_an_already_filled_stream_emits_nothing() -> None:
    """Pinned by ``fresh_replay_is_exact_and_preprocessed_snapshot_replay_emits_nothing``."""
    registry = FixRegistry()
    raw = [_event(registry, time, "A") for time in (1, 7, 11, 21)]
    full = FixLifecycle(registry, interval_ns=10)
    filled = [full.fill(message) for message in raw]
    assert all(message.createdat() == _ns(1) for message in filled)
    # A fresh lifecycle replays the raw stream, and the filled one, exactly.
    fresh = FixLifecycle(registry, interval_ns=10)
    assert [fresh.fill(message) for message in raw] == filled
    fresh = FixLifecycle(registry, interval_ns=10)
    assert [fresh.fill(message) for message in filled] == filled
    assert fresh.alive() == full.alive()
    full.clear()
    assert [full.fill(message) for message in raw] == filled

    first = list(FixLifecycle(registry, interval_ns=10).snapshots(raw))
    assert len(first) == 3
    assert list(FixLifecycle(registry, interval_ns=10).snapshots(iter(raw))) == first
    filtered = FixLifecycle(registry, interval_ns=10)
    answered = [filtered.snapshot(message) for message in raw]
    assert [message for message in answered if message is not None] == first
    # Already-filled messages sit on their grid: they rebuild the live state
    # and emit nothing.
    assert list(FixLifecycle(registry, interval_ns=10).snapshots(filled)) == []


def test_a_snapshot_stream_owns_the_state_and_goes_on_past_a_refused_transition() -> None:
    """Pinned by ``snapshot_stream_is_lazy_keeps_per_item_errors_and_fuses_only_exhaustion``
    and ``grid_underflow_refuses_before_opening_attaching_advancing_or_closing``."""
    registry = FixRegistry()
    life = FixLifecycle(registry, interval_ns=10)
    opened = life.fill(_event(registry, 1, "A"))
    assert life.alive() == 1
    pulled = 0

    def source() -> Iterator[FixMsg]:
        nonlocal pulled
        for message in (
            _event(registry, 12, "A"),
            # Floored to the grid, the least instant underflows.
            _event(registry, -(2**63), "A"),
            _event(registry, 21, "A"),
            _event(registry, 29, "A"),
            _event(registry, 31, "A"),
        ):
            pulled += 1
            yield message

    stream = life.snapshots(source())
    assert isinstance(stream, FixMessages)
    # The stream took the live state with it: the lifecycle is left with
    # none, at the same interval, and nothing was pulled yet.
    assert life.alive() == 0
    assert life.interval_ns == 10
    assert pulled == 0
    carried = next(stream)
    assert pulled == 1
    _previous(carried, opened)
    assert carried.createdat() == opened.createdat()
    assert carried.updatedat() == _ns(10)
    # A refused transition raises where it is met, changes no chain, and the
    # stream goes on.
    with pytest.raises(ValueError, match="updatedat"):
        next(stream)
    assert pulled == 2
    after = next(stream)
    assert pulled == 3
    _previous(after, carried)
    assert after.updatedat() == _ns(20)
    # A suppressed arrival is processed, not emitted.
    last = next(stream)
    assert pulled == 5
    assert last.updatedat() == _ns(30)
    assert next(stream, None) is None
    assert next(stream, None) is None

    # A failure of the iterable raises as itself; an item that is not a
    # message is refused where it is met and ends the stream; something that
    # is not iterable is refused before anything is pulled.
    def failing() -> Iterator[FixMsg]:
        yield _event(registry, 1, "B")
        raise RuntimeError("the source broke")

    broken = FixLifecycle(registry, interval_ns=10).snapshots(failing())
    assert next(broken).updatedat() == _ns(0)
    with pytest.raises(RuntimeError, match="the source broke"):
        next(broken)
    mixed = FixLifecycle(registry, interval_ns=10).snapshots([_event(registry, 1, "C"), b"not a message"])
    assert next(mixed).by_tag(CODE_TAG).as_py() == "C"
    with pytest.raises(TypeError):
        next(mixed)
    assert next(mixed, None) is None
    kept = FixLifecycle(registry, interval_ns=10)
    kept.fill(_event(registry, 1, "D"))
    with pytest.raises(TypeError):
        kept.snapshots(42)  # type: ignore[arg-type]
    assert kept.alive() == 1


def test_an_unresolved_arrival_records_tag_zero_and_keeps_its_raw_key(seed: FixRegistry) -> None:
    """One arrival record, with zero reserved for a key nothing resolved.

    Pinned by ``rust/tests/fix/zero_entries.rs``.
    """
    codec = _fixed(seed)
    wire = b"35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|"
    message = next(codec.parse_line(wire))
    entries = message.entries()
    assert [tag for tag, _, _ in entries] == [35, 0, 0, 0, 0, 0, 55, 10]
    unresolved = [
        ("999999", "one"),
        ("0999999", "two"),
        ("OwnThing", "three"),
        ("0", "zero"),
        ("2147483648", "wide"),
    ]
    assert [(key, value) for _, key, value in entries[1:6]] == unresolved
    for key, value in unresolved:
        held = message.get_by_name(key)
        assert held is not None and held.as_py() == value, key
    assert message.by_tag(999_999).as_py() == "one"
    assert message.into_bytes(ord("|")) == wire

    # Neither sign makes a numeric tag of a key a caller split.
    signed = codec.parse_pairs([("-1", "negative"), ("+35", "signed")])
    assert signed.entries() == [(0, "-1", "negative"), (0, "+35", "signed")]
    for key, value in (("-1", "negative"), ("+35", "signed")):
        held = signed.get_by_name(key)
        assert held is not None and held.as_py() == value, key
    assert signed.into_bytes(ord("|")) == b"-1=negative|+35=signed|"

    # An indexed unknown keeps each arrival; its value keeps its shape.
    indexed = codec.parse_pairs([("999999[0]", "first"), ("999999[2]", "third")])
    assert indexed.entries() == [(0, "999999[0]", "first"), (0, "999999[2]", "third")]
    held = indexed.get_by_name("999999")
    assert held is not None and held.as_py() == ["first", None, "third"]

    # An unresolved counter heads what arrived under it, at the top and inside
    # a resolved group, whose resolved member still follows on the wire.
    top = codec.parse_pairs([("999999", "2"), ("999999[0].OwnThing", "a"), ("999999[1].999998", "b")])
    assert top.entries() == [
        (0, "999999", "2"),
        (0, "999999[0].OwnThing", "a"),
        (0, "999999[1].999998", "b"),
    ]
    assert top.into_bytes(ord("|")) == b"999999=2|999999[0].OwnThing=a|999999[1].999998=b|"
    nested = codec.parse_pairs(
        [
            ("NoPartyIDs", "1"),
            ("NoPartyIDs[0].999999", "1"),
            ("NoPartyIDs[0].999999[0].999998", "A"),
            ("NoPartyIDs[0].PartyRole", "3"),
        ]
    )
    assert [(tag, key) for tag, key, _ in nested.entries()] == [
        (453, "NoPartyIDs"),
        (0, "NoPartyIDs[0].999999"),
        (0, "NoPartyIDs[0].999999[0].999998"),
        (452, "NoPartyIDs[0].PartyRole"),
    ]

    # The crate's Map has no numeric scalar wire spelling.
    opaque = next(codec.parse_line(b"35=D|65020=opaque|10=0|"))
    assert opaque.entries()[1] == (0, "65020", "opaque")
    held = opaque.get_by_name("65020")
    assert held is not None and held.as_py() == "opaque"
    assert opaque.get_by_name("altids") is None
    assert opaque.into_bytes(ord("|")) == b"35=D|65020=opaque|10=0|"

    # A resolved key keeps its canonical positive tag however it was spelled.
    registry = copy.copy(seed)
    symbol = registry.field_by_tag(55)
    symbol.fix.tags = [9_000_001]
    symbol.fix.aliases = ["SyntheticSymbol"]
    registry.insert(symbol)
    respelled = _fixed(registry)
    canonical = next(respelled.parse_line(b"35=D|55=SYNTH|10=0|"))
    for key in ("55", "00055", "9000001", "SyntheticSymbol"):
        line = f"35=D|{key}=SYNTH|10=0|".encode()
        read = next(respelled.parse_line(line))
        assert read.entries()[1] == (55, key, "SYNTH"), key
        assert read.digest() == canonical.digest(), key
        assert read.into_bytes(ord("|")) == line, key

    # The digest leaves out a header tag only where a dictionary resolved it:
    # under a bare registry `34` is an unresolved arrival, hashed by its key.
    def digest(dictionary: FixRegistry, sequence: str) -> bytes:
        return _fixed(dictionary).parse_pairs([("34", sequence), ("11", "A")]).digest()

    assert digest(seed, "7") == digest(seed, "8")
    bare = FixRegistry()
    assert digest(bare, "7") != digest(bare, "8")


def test_a_batch_read_lands_at_the_newest_version_when_asked(seed: FixRegistry) -> None:
    """A stage is a call: the enriching pass composes between the parse and the batch."""
    codec = _fixed(seed)
    source = pa.table({"body": pa.array([REPORT], pa.binary())})
    # Enriched as messages, before the row: the pass restates first, and the
    # fixed row has no column for a retired field such as `ExecTransType(20)`,
    # so a row read back would restate without it.
    restated = codec.arrow_reader(
        fix_schema(seed), codec.enrich_messages(codec.parse_lines([REPORT]))
    ).read_all()
    assert restated.column("version").to_pylist() == ["5.0.2"]
    assert restated.column("exectype").to_pylist()[0] == "40TRDCXL"
    assert restated.column("nopartyids").to_pylist() == [2]
    assert restated.column("parties").to_pylist()[0][0]["partyid"] == "BRKR"
    # Unenriched, the row speaks the version it was read at.
    read = codec.parse_text_arrow_reader(source).read_all()
    assert read.column("version").to_pylist() == ["4.2"]
    assert read.column("exectype").to_pylist()[0] == "40PARTFILL"
    assert read.column("nopartyids").to_pylist() == [None]
