"""The FIX boundary: the typed vocabulary, the registry, and the message.

Every answer here is the core's; what these check is the crossing - the key
coercion, the exception each core failure maps to, the storage locations a
Python caller names, and the Python protocols the wrappers implement. An
identifier crosses as the ``int`` the core derives from a tag and a name, and
a dictionary's contribution is a list of names on the field, so neither has a
class of its own and every refusal is the native one.
"""

from __future__ import annotations

import copy
import datetime as dt
import json
import logging
import pathlib
import pickle
import subprocess
import sys
from typing import Any, Iterable

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase, MimeType, Scalar, TextLine, Url, refresh_logging
from yggdryl.fix import (
    FixCapture,
    FixCodec,
    FixHeader,
    FixMessages,
    FixMsg,
    FixRegistry,
    MarketEventData,
    MsgType,
    fix_cfb_fields,
    fix_crate_fields,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
    global_registry,
    install_global_registry,
)

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"

# What the crate itself adds beside the specification: its own columns in tag
# order from 65003 and the two Map groups.
CRATED = 38
# What ``FixRegistry()`` holds: the crate's own scalar fields, the two seeded
# standard clocks SendingTime (52) and TransactTime (60), and the two Map
# groups. ``len`` counts the groups; iteration walks the scalars alone.
SEEDED = 40
SEEDED_SCALARS = 38

# The one intake clock undated test bytes take, so a parse repeats; replay
# never consults now.
CLOCK_NS = 1_704_190_530_000_000_000
CLOCK = DataType('datetime64(ns,"UTC")').scalar(CLOCK_NS)
CLOCK_INSTANT = dt.datetime(2024, 1, 2, 10, 15, 30, tzinfo=dt.timezone.utc)

# The crate's own tags the message answers as typed facts.
UNIX_TAG = 65003
HASHCODE_TAG = 65017
CROSSHASHCODE_TAG = 65018
IDENTIFIERS_TAG = 65020
PREVUNIX_TAG = 65021
PREVUUID_TAG = 65022
CREATUNIX_TAG = 65023
SNAPUNIX_TAG = 65025
NOFIXENTRIES_TAG = 65027
CURRUUID_TAG = 65039
CROSSUUID_TAG = 65040
SEQNUM_TAG = 65042
PX_TAG = 65043
CROSSCODE_TAG = 65048
METADATA_TAG = 65049
PREVPX_TAG = 65051
PREVQTY_TAG = 65052
TRADABLE_TAG = 65053
SYMBOLTICKER_TAG = 65054


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
    names: Iterable[str] = (),
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
    if names:
        field.fix.names = names
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
    field.fix.names = ["Qty", "Quantity"]
    field.fix.description = "Quantity ordered."

    assert field.fix.tag == 38
    assert field.fix.tags == [1088]
    assert field.fix.names == ["Qty", "Quantity"]
    assert field.fix.description == "Quantity ordered."
    # Ordinary namespaced text, in the one metadata map: a list is the
    # compact JSON array it is.
    assert field.metadata["fix:names"] == '["Qty","Quantity"]'
    assert field.metadata["fix:tags"] == "[1088]"
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
    field.fix.names = ()
    assert field.fix.names == []
    del field.fix["tag"]
    assert field.fix.tag is None

    absent = Field("Symbol", "utf8")
    assert absent.fix.tag is None
    assert absent.fix.tags == []
    assert absent.fix.names == []
    assert absent.fix.description is None


def test_typed_vocabulary_is_only_on_the_fix_view() -> None:
    field = Field("Symbol", "utf8")
    field.fix.tag = 55

    for view, scheme in ((field.http, "http"), (field.iceberg, "iceberg")):
        with pytest.raises(TypeError, match=scheme):
            view.tag
        with pytest.raises(TypeError, match=scheme):
            view.names
        with pytest.raises(TypeError, match=scheme):
            view.branches
        with pytest.raises(TypeError, match=scheme):
            view.id
        with pytest.raises(TypeError, match=scheme):
            view.tag = 55
        with pytest.raises(TypeError, match=scheme):
            view.add_branch("cme")
        with pytest.raises(TypeError, match=scheme):
            view.derivation
    # The mapping protocol still works on every view, including this one.
    assert field.protocol("fix")["tag"] == "55"


def test_a_derivation_crosses_as_canonical_text(seed: FixRegistry) -> None:
    """One term over the message's fields, stored as its canonical spelling."""
    field = Field("leavesqty", "float64")
    field.fix.tag = 151
    assert field.fix.derivation is None

    field.fix.derivation = "orderqty-cumqty"
    assert field.fix.derivation == "orderqty - cumqty"
    assert field.metadata["fix:derivation"] == "orderqty - cumqty"

    with pytest.raises(ValueError):
        field.fix.derivation = "orderqty -"
    assert field.fix.derivation == "orderqty - cumqty"

    # An edited derivation is what a parse fills by: the rules run inside the
    # parse rather than in a pass of their own.
    leaves = seed.get_field_by_tag(151)
    assert leaves is not None
    leaves.fix.derivation = "case when msgtype in ('8', '9') then orderqty * 2 end"
    seed.update(leaves)
    held = _fixed(seed).parse_fix_line(b"8=FIX.4.4|35=8|37=A|38=100|14=0|10=0|")
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
    assert json.loads(field.metadata["fix:directions"]) == rules

    # A codec compiles the rules of the dictionary it is built over, once,
    # and the line door fills the header's direction from them.
    registry = FixRegistry()
    registry.insert(field)
    codec = _fixed(registry)
    assert next(codec.parse_line(b"TX 8=FIX.4.4|35=D|10=0|")).header().msgdirection == "S"
    assert next(codec.parse_line(b"RX 8=FIX.4.4|35=D|10=0|")).header().msgdirection == "R"
    assert next(codec.parse_line(b"sending >> 8=FIX.4.4|35=D|10=0|")).header().msgdirection is None

    # A pattern the regex crate refuses is refused whole, the field unchanged.
    with pytest.raises(ValueError, match="valid byte regex"):
        field.fix.directions = [{"code": "S", "patterns": ["("]}]
    assert field.fix.directions == rules
    field.fix.directions = []
    assert "fix:directions" not in field.metadata


def test_tag_and_identifier_keys_refuse_bool_and_narrowing(seed: FixRegistry) -> None:
    field = Field("Symbol", "utf8")
    with pytest.raises(TypeError, match="not bool"):
        field.fix.tag = True
    with pytest.raises(OverflowError):
        field.fix.tag = 2**31

    with pytest.raises(TypeError, match="not bool"):
        seed.get_field_by_tag(True)
    with pytest.raises(TypeError, match="not bool"):
        seed.field_by_id(True)
    with pytest.raises(OverflowError):
        seed.get_field_by_id(2**31)
    # A name and a path take one argument: there is no dictionary to pin.
    with pytest.raises(TypeError):
        seed.get_field_by_name("Symbol", "")  # type: ignore[call-arg]


def test_id_is_the_tag_under_the_name_and_never_stored() -> None:
    field = _field("Symbol", "utf8", 55)
    assert field.fix.id == _field("symbol", "utf8", 55).fix.id
    assert field.fix.id != _field("Symbol", "utf8", 56).fix.id
    assert "fix:id" not in field.metadata
    assert Field("Symbol", "utf8").fix.id is None


def test_registry_resolves_every_key_the_way_the_core_does(seed: FixRegistry) -> None:
    # The store's fields, its components and its groups, and the crate's own
    # beside them: a store writes those too, and a loaded dictionary holds
    # the crate's definition rather than the document's.
    assert bool(seed)
    assert len(seed) > len(list(seed)), "len counts the definitions, iteration walks scalars"

    assert seed.field_by_tag(55).name == "symbol"
    assert seed.get_field_by_tag(55) == seed.field_by_tag(55)
    symbol_id = seed.field_by_tag(55).fix.id
    assert symbol_id is not None
    assert seed.field_by_id(symbol_id).name == "symbol"
    # Only the crate's own `state` is typed as the ranked lifecycle
    # vocabulary; the protocol's own code sets keep their base type, and
    # tag 54 is the one FIX field typed as the value it holds.
    assert seed.field_by_tag(54).dtype == DataType("side")
    assert seed.field_by_tag(15).dtype == DataType("currency")
    assert seed.field_by_tag(39).dtype == DataType("utf8")
    assert seed.field_by_tag(40).dtype == DataType("utf8")

    # An alternate tag is not an identity: the one id is the canonical tag's.
    alternate = _field("exectype", "utf8", 150)
    alternate.fix.tags = [20]
    aliased = FixRegistry.from_fields([alternate])
    assert aliased.field_by_tag(20).name == "exectype"
    assert aliased.get_field_by_id(_field("exectype", "utf8", 20).fix.id) is None

    # A name answers the canonical spelling whatever case it was asked in.
    assert seed.field_by_name("SYMBOL").name == "symbol"
    named = FixRegistry.from_fields([_field("symbol", "utf8", 55, names=["ticker"])])
    assert named.field_by_name("ticker").name == "symbol"

    # One namespace: a component and a group answer the name doors too, and
    # a group answers the counter its occurrences are counted by.
    assert seed.field_by_name("parties").fix.counter == 453
    assert seed.field_by_name("party").dtype.is_nested
    assert seed.field_by_counter(453).name == "parties"
    assert seed.get_field_by_counter(9999) is None
    assert seed.field_by_tag(453).dtype == DataType("int32")

    # A path reaches a repeating group and one of its members.
    assert seed.field_by_path("NoPartyIDs").fix.tag == 453
    assert seed.field_by_path("parties.partyid").fix.tag == 448
    assert seed.get_field_by_path("parties.partyid.partyid") is None

    # The generic pair answers exactly what the specialized one does.
    for key in (55, "Symbol", "nopartyids", "parties.partyid"):
        assert seed.get_field(key) == seed[key]
        assert seed.field(key) == seed[key]
        assert key in seed
    assert 9999 not in seed
    assert seed.get_field(9999) is None
    assert seed.get(9999, "fallback") == "fallback"


def test_registry_absence_is_a_key_error_carrying_the_core_message(seed: FixRegistry) -> None:
    with pytest.raises(KeyError) as by_tag:
        seed.field_by_tag(9999)
    assert by_tag.value.args[0] == 'expected a fix field at "tag 9999", got nothing'

    absent_id = _field("TradeID", "utf8", 5001).fix.id
    assert absent_id is not None
    with pytest.raises(KeyError):
        seed.field_by_id(absent_id)
    with pytest.raises(KeyError):
        seed.field_by_name("Nope")
    with pytest.raises(KeyError):
        seed.field_by_counter(9999)
    with pytest.raises(KeyError):
        seed.msgtype("nope")


def test_registry_iterates_lazily_in_ascending_identifier_order() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
            _field("Price", "decimal128(20, 8)", 44),
            _field("Account", "utf8", 1),
        ]
    )
    # Tag-major, then by identifier. The seeded SendingTime (52) and
    # TransactTime (60) walk among the dictionary's own tags, and the crate's
    # own fields close every walk, above any tag a test claims.
    tags = [field.fix.tag for field in registry]
    assert tags[:5] == [1, 44, 52, 55, 60]
    assert tags == sorted(tags)
    assert len(tags) == 4 + SEEDED_SCALARS

    walk = iter(registry)
    assert next(walk).name == "Account"
    # An unfinished walk shares the registry, so a mutation refuses until it
    # is dropped rather than moving the fields under the cursor.
    with pytest.raises(ValueError, match="shared with a message"):
        registry.remove(1)
    del walk
    assert registry.remove(1) is not None


def test_a_new_registry_holds_the_crate_and_the_two_seeded_clocks() -> None:
    registry = FixRegistry()
    assert len(registry) == SEEDED
    assert len(list(registry)) == SEEDED_SCALARS
    assert registry.dialects() == []
    assert repr(registry) == f"FixRegistry({SEEDED} fields)"

    # The seeds are ordinary definitions, typed as the clock the event reads.
    for tag, name, display in ((52, "sendingtime", "SendingTime"), (60, "transacttime", "TransactTime")):
        seeded = registry.field_by_tag(tag)
        assert (seeded.name, seeded.display) == (name, display)
        assert seeded.dtype == DataType('datetime64(ns,"UTC")')

    # The crate's Map groups are groups: filed by their shape, reached by
    # name and by their own tag as a counter, never by the scalar tag door.
    for name, tag in (("identifiers", IDENTIFIERS_TAG), ("metadata", METADATA_TAG)):
        assert registry.field_by_name(name).dtype.is_nested, name
        assert registry.field_by_counter(tag).name == name, name
        assert registry.get_field_by_tag(tag) is None, name

    # The retired spellings reach nothing.
    for retired in ("updatedat", "createdat", "msghash", "msgphash", "altids", "code", "version"):
        assert registry.get_field_by_name(retired) is None, retired


def test_the_crate_fields_declare_their_own_protocols() -> None:
    """Each column says what it derives from and what it holds, on the field."""
    fields = {field.name: field for field in fix_crate_fields()}
    assert len(fields) == CRATED
    # In tag order, one block from 65003, above every tag FIX or a venue
    # publishes: the event's clocks, its identities, the facts a row derives
    # from what the message said, what a capture stated, and the two Maps.
    assert list(fields) == [
        "unix",
        "msgctxid",
        "pluginid",
        "isincode",
        "miccode",
        "state",
        "hashcode",
        "crosshashcode",
        "identifiers",
        "prevunix",
        "prevuuid",
        "creatunix",
        "snapunix",
        "sourceurl",
        "nofixentries",
        "recordedat",
        "expirunix",
        "bidcurrency",
        "askcurrency",
        "msgsessionid",
        "bloombergcode",
        "cusipcode",
        "sedolcode",
        "curruuid",
        "crossuuid",
        "parentuuids",
        "seqnum",
        "px",
        "qty",
        "unit",
        "bidunit",
        "askunit",
        "crosscode",
        "metadata",
        "prevpx",
        "prevqty",
        "tradable",
        "symbolticker",
    ]
    tags = [field.fix.tag for field in fields.values()]
    assert tags == sorted(tags)
    assert tags[0] == UNIX_TAG and tags[-1] == SYMBOLTICKER_TAG
    assert all(field.fix.branches == [] for field in fields.values())
    assert all(field.description is not None for field in fields.values())

    # The columns every message settles are non-null; every other one is
    # nullable, because a message that carried nothing there answers null.
    assert [name for name, field in fields.items() if not field.nullable] == [
        "unix",
        "hashcode",
        "crosshashcode",
        "creatunix",
        "curruuid",
        "crossuuid",
    ]

    # The clocks are instants in UTC, to the nanosecond; the identities are
    # what a lake reads as a UUID and a 64-bit integer; the facts a row
    # derives are typed as the thing they hold.
    for name in ("unix", "prevunix", "creatunix", "snapunix", "recordedat", "expirunix"):
        assert fields[name].dtype == DataType('datetime64(ns,"UTC")'), name
    for name in ("curruuid", "crossuuid", "prevuuid"):
        assert fields[name].dtype == DataType("uuid"), name
    for name in ("hashcode", "crosshashcode", "seqnum"):
        assert fields[name].dtype == DataType("uint64"), name
    assert fields["isincode"].dtype == DataType("isin")
    assert fields["miccode"].dtype == DataType("mic")
    assert fields["state"].dtype == DataType("state")
    assert fields["sourceurl"].dtype == DataType("url")
    assert fields["crosscode"].dtype == DataType("utf8")
    assert fields["nofixentries"].dtype == DataType("int32")
    # How a layout is cut is the target's: no partition column.
    assert all(not field.is_partition for field in fields.values())
    # The two Maps carry the names a message goes by and what a bridge said.
    for name in ("identifiers", "metadata"):
        assert fields[name].into_arrow().type.equals(
            pa.map_(pa.string(), pa.string(), keys_sorted=True)
        ), name


def test_registry_takes_every_storage_location(seed: FixRegistry, tmp_path: pathlib.Path) -> None:
    absolute = SEED.resolve()
    for location in (absolute, str(absolute), absolute.as_uri(), Url(absolute), IOBase(absolute)):
        assert FixRegistry.from_handle(location) == seed

    # A folder that is not there loads as a new registry and is not created.
    missing = tmp_path / "missing"
    assert FixRegistry.from_handle(missing) == FixRegistry()
    assert not missing.exists()


def test_a_malformed_native_field_shard_is_located(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "invalid"
    (root / "fields").mkdir(parents=True)
    (root / "fields" / "000000000.json").write_text("not JSON", encoding="utf-8")
    with pytest.raises(ValueError, match="000000000.json"):
        FixRegistry.from_handle(root)


def test_a_store_writes_the_whole_row_and_reads_its_own_dump_back(tmp_path: pathlib.Path) -> None:
    """Shards are nine digits wide, and the crate dumps its own row beside them."""
    registry = FixRegistry.from_fields(
        [_field("MsgType", "utf8", 35), _field("TradeID", "utf8", 5001, branches=["cme"])]
    )
    root = tmp_path / "dictionary"
    registry.write_into(root)

    # One shard arithmetic for every field, nine digits with leading zeros:
    # tag 35 lands in shard 0, tag 5001 in shard 50, the crate's own in 650.
    assert sorted(str(path.relative_to(root)) for path in root.rglob("*.json")) == [
        "components/fixmsg.json",
        "fields/000000000.json",
        "fields/000000050.json",
        "fields/000000650.json",
        "groups/identifiers.json",
        "groups/metadata.json",
    ]
    assert not (root / "branches.json").exists()

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_name("tradeid").fix.branches == ["cme"]
    assert reloaded.dialects() == ["cme"]


def test_registry_insert_update_and_remove_over_fields() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55, names=["Ticker"]),
            _field("Price", "decimal128(20, 8)", 44, names=["Px"]),
        ]
    )
    assert len(registry) == 2 + SEEDED
    assert registry.insert(_field("Side", "utf8", 54)) is None
    assert registry.field_by_tag(54).name == "Side"

    # A key another field holds is refused, naming both; nothing changes.
    with pytest.raises(ValueError, match="held by symbol"):
        registry.insert(_field("SymbolSfx", "utf8", 65, names=["ticker"]))
    assert len(registry) == 3 + SEEDED

    # A merge concatenates the two list properties, incoming first.
    registry.update(_field("SYMBOL", "utf8", 55, tags=[65], names=["Sym"]))
    merged = registry.field_by_tag(65)
    assert merged.name == "symbol"
    assert merged.fix.names == ["Sym", "Ticker"]
    with pytest.raises(ValueError):
        registry.update(_field("symbol", "large_utf8", 55))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    removed = registry.remove("sym")
    assert removed is not None and removed.name == "symbol"
    assert registry.remove(9999) is None

    # `remove` reads an int as a tag, so a field sharing its tag with another
    # leaves only through the identifier `remove_by_id` reads as one.
    registry.insert(_field("Held", "utf8", 77, names=["Beside"]))
    registry.insert(_field("Beside", "utf8", 78, tags=[77]))
    beside = registry.field_by_name("Beside")
    assert registry.remove_by_id(beside.fix.id) is not None
    assert registry.field_by_tag(77).name == "Held"
    assert registry.remove_by_id(beside.fix.id) is None

    # A field with no tag cannot enter at all.
    with pytest.raises(ValueError, match="fix:tag"):
        registry.insert(Field("Untagged", "utf8"))


def test_registry_add_field_answers_whether_the_field_arrived_or_folded() -> None:
    """`add_field` is the one-field verb `add_fields` folds through."""
    registry = FixRegistry.from_fields(
        [
            _field("Symbol", "utf8", 55, tags=[65], names=["Ticker"], description="stored"),
            _field("Price", "float64", 44),
        ]
    )

    incoming = _field(
        "symbol", "utf8", 9001, tags=[66], names=["Sym", "TICKER"], description="incoming"
    )
    assert registry.add_field(incoming) is False
    stored = registry.field_by_tag(55)
    assert stored.name == "Symbol"
    assert stored.fix.tags == [65, 66, 9001]
    assert stored.fix.names == ["Ticker", "Sym"]
    assert stored.description == "incoming"
    for key in (9001, 66, 65, "sym", "TICKER"):
        assert registry.field(key).name == "Symbol", key

    before = registry.into_json()
    assert registry.add_field(incoming) is False
    assert registry.into_json() == before
    assert registry.add_field(_field("Text", "utf8", 58)) is True

    # A datatype that disagrees with the stored field is refused atomically.
    before = registry.into_json()
    with pytest.raises(ValueError, match="utf8"):
        registry.add_field(_field("SYMBOL", "int32", 9002))
    assert registry.into_json() == before
    # One of this crate's own is every dictionary's already.
    assert registry.add_field(fix_crate_fields()[0]) is False
    assert registry.into_json() == before


def test_a_cblock_reads_in_whole_and_stamps_its_dialect(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "bloomberg.cfb"
    path.write_text(CBLOCK, encoding="utf-8")

    fields = fix_cfb_fields(path, "bloomberg")
    assert [field.name for field in fields] == ["symbol", "excludeddealers"]
    assert fields[0].description == "Ticker symbol."
    assert fields[0].fix.branches == ["bloomberg"]

    # Every location the registry takes, the vocabulary takes.
    for location in (path, str(path), path.as_uri(), Url(path), IOBase(path)):
        assert [field.name for field in fix_cfb_fields(location, "bloomberg")] == [
            "symbol",
            "excludeddealers",
        ]

    # The registry form is the same file read whole: the same vocabulary,
    # plus the message roots its grammar bindings describe.
    registry, roots = FixRegistry.from_cfb_file(path, dialect="Bloomberg")
    assert [root.name for root in roots] == ["7"]
    message = registry.get_msgtype("7")
    assert message is not None and message.value == "7"
    assert message.field.fix.branches == ["bloomberg"]
    assert registry.dialects() == ["bloomberg"]

    unstamped, _ = FixRegistry.from_cfb_file(path)
    assert unstamped.dialects() == []

    # The vocabulary folds into a dictionary that already exists.
    dictionary = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    assert dictionary.add_fields(fix_cfb_fields(path, "bloomberg")) == (1, 1)
    assert dictionary.field_by_tag(55).fix.branches == ["bloomberg"]

    # A stem or a dialect that carries a comma is refused rather than stored.
    with pytest.raises(ValueError, match="fix:branches"):
        fix_cfb_fields(path, "b,loomberg")


def test_a_cblock_warns_about_the_declaration_it_dropped(
    tmp_path: pathlib.Path, caplog: pytest.LogCaptureFixture
) -> None:
    broken = tmp_path / "bloomberg.cfb"
    broken.write_text(
        CBLOCK.replace('name="55" alt="Symbol" type="string"', 'name="55" alt="Symbol" type="decimal"'),
        encoding="utf-8",
    )
    caplog.set_level(logging.WARNING)
    refresh_logging()
    FixRegistry.from_cfb_file(broken, "bloomberg")
    warnings = [
        record.getMessage()
        for record in caplog.records
        if record.name.startswith("yggdryl") and record.levelno == logging.WARNING
    ]
    assert warnings, "the native reader reported nothing"
    assert "invalid cfb expression at byte" in warnings[0]

    # The tag went; every other declaration the file made stands.
    assert [field.name for field in fix_cfb_fields(broken)] == ["excludeddealers"]

    # A document that stops with an element open is refused through both
    # doors, with one sentence.
    truncated = tmp_path / "truncated.cfb"
    truncated.write_text(CBLOCK.replace("</vocabulary>", ""), encoding="utf-8")
    with pytest.raises(ValueError) as refused:
        FixRegistry.from_cfb_file(truncated, "bloomberg")
    with pytest.raises(ValueError) as also:
        fix_cfb_fields(truncated)
    assert str(also.value) == str(refused.value)


def test_registry_mutation_refuses_while_something_shares_it(seed: FixRegistry) -> None:
    root = Field("row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False)
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
    fresh = copy.copy(seed)
    fresh.insert(_field("LocalValue", "utf8", 9999))
    assert fresh.remove(9999) is not None

    # A message type view shares it too.
    registry = FixRegistry.from_fields([_field("MsgType", "utf8", 35)])
    order = Field("NewOrderSingle", DataType.from_fields([]), nullable=False)
    order.fix.msgtype = "D"
    registry.insert(order)
    view = registry.msgtype("D")
    with pytest.raises(ValueError, match="shared"):
        registry.insert(_field("Symbol", "utf8", 55))
    del view
    assert registry.insert(_field("Symbol", "utf8", 55)) is None


def _order(seed: FixRegistry) -> Field:
    """A root carrying a group, a tag no dictionary explains, and its clock.

    A message built by hand reads UTC now for a SendingTime it does not
    state, so the root states one and two builds of it are one message.
    """
    return Field(
        "NewOrderSingle",
        DataType.from_fields(
            [
                seed.field_by_tag(55),
                seed.field_by_tag(38),
                seed.field_by_name("NoPartyIDs"),
                seed.field_by_name("Parties"),
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
    "parties": [{"partyid": "BROKER", "partyidsource": "D", "partyrole": 1}],
    "9999": "custom",
    "sendingtime": CLOCK,
}


def test_message_resolves_through_the_registry_it_carries(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)

    # A child stating a typed fact fills the holder that owns it and leaves
    # the row, so the root keeps only what the message states of its own.
    assert [child.name for child in message.field] == [
        "symbol",
        "nopartyids",
        "parties",
        "9999",
    ]
    assert message.registry == seed
    assert len(message) == 4
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
    with pytest.raises(KeyError) as by_path:
        message.by_path("Parties.PartyID")
    assert "path Parties.PartyID" in by_path.value.args[0]
    with pytest.raises(TypeError, match="not bool"):
        message[True]
    with pytest.raises(TypeError):
        message.by_id("55")  # type: ignore[arg-type]
    with pytest.raises(OverflowError):
        message.get_by_id(2**31)

    # The mapping input became the ordered row the message declares.
    pairs = [(name, value) for name, value in message]
    assert [name for name, _ in pairs] == [child.name for child in message.field]
    assert pairs[0][0] == "symbol" and pairs[0][1].as_py() == "AAPL"

    # A native Scalar names the same content under the message's own root.
    # The typed facts are not in it, so the rebuild settles its own clocks
    # and states the content the first one states.
    rebuilt = FixMsg(message.field, message.value, seed)
    assert rebuilt.field == message.field
    assert rebuilt.value == message.value
    assert rebuilt.entries() == message.entries()


def test_a_message_holds_its_typed_facts_beside_its_row(seed: FixRegistry) -> None:
    """The three holders and the two extras, each answering its own facts."""
    codec = _fixed(seed)
    wire = (
        b"8=FIX.4.4|35=D|49=SENDER|56=TARGET|34=7|52=20240102-10:15:30|"
        b"11=A1|55=AAPL|54=1|15=USD|38=100|44=10.5|58=note|60=20240102-10:15:31|10=0|"
    )
    message = codec.parse_fix_line(wire)

    header = message.header()
    assert isinstance(header, FixHeader)
    assert header.beginstring == "FIX.4.4"
    assert header.msgtype == "D"
    assert header.sendercompid == "SENDER"
    assert header.targetcompid == "TARGET"
    assert header.msgseqnum == 7
    assert header.sendingtime == CLOCK_NS
    assert header.stated_sendingtime
    assert header.possdupflag is None
    assert header.msgdirection is None
    assert header == message.header()
    assert hash(header) == hash(message.header())
    assert copy.copy(header) == header

    capture = message.capture()
    assert isinstance(capture, FixCapture)
    assert capture.sourceurl is None
    assert capture.recordedat == CLOCK_NS
    assert capture.pluginid is None
    assert capture.msgctxid is None
    assert capture.msgsessionid is None

    event = message.event()
    assert isinstance(event, MarketEventData)
    # The instant is TransactTime where the message states one.
    assert event.unix == CLOCK_NS + 1_000_000_000
    assert event.creatunix == event.unix
    assert event.px.as_py() == 10.5
    assert event.qty.as_py() == 100
    assert event.currency.as_py() == "USD"
    assert event.side.as_py() == "BUY"
    assert event.crosscode == "A1"
    assert event.identifiers == {"clordid": "A1"}
    assert event.seqnum == 0
    assert event.prevuuid is None and event.prevunix is None and event.snapunix is None
    assert event.state.as_py() == "00UNKNOWN"
    assert event.isincode is None
    assert event == message.event()
    assert hash(event) == hash(message.event())

    # The facts a consumer reads most are the message's own properties, and
    # they answer what the event answers.
    assert message.unix == event.unix
    assert message.curruuid == event.curruuid
    assert message.crossuuid == event.crossuuid
    assert message.crosscode == event.crosscode
    assert message.hashcode == event.hashcode
    assert message.crosshashcode == event.crosshashcode
    assert message.identifiers == event.identifiers
    assert message.parentuuids == event.parentuuids == []
    assert message.state == event.state
    assert message.seqnum == event.seqnum
    assert message.prevuuid is None
    assert message.px == event.px
    assert message.qty == event.qty
    assert message.side == event.side
    assert message.currency == event.currency
    # The identities are uuid scalars, the codes uint64.
    assert message.curruuid.dtype == DataType("uuid")
    assert isinstance(message.hashcode, int) and message.hashcode != 0
    # The cross identity derives from the cross code, so it is not the own one.
    assert message.crossuuid != message.curruuid
    assert message.crosshashcode != 0

    # Tag 58 and a bridge's namespaced keys are the two extras.
    assert message.text == "note"
    assert message.metadata == {}
    bridged = codec.parse_ullink_line(b"|#SYMBOL=TTF|#TECH.CLIENTID=MCFP2|")
    assert bridged.metadata == {"tech.clientid": "MCFP2"}
    assert bridged.text is None

    # A typed tag answers the holder, typed as its column is; every other
    # key reaches the row.
    assert message.by_tag(35).as_py() == "D"
    assert message.by_tag(52) == CLOCK
    assert message.by_tag(54).as_py() == "BUY"
    assert message.by_tag(58).as_py() == "note"
    assert message.by_tag(HASHCODE_TAG).as_py() == message.hashcode
    assert message.by_tag(CURRUUID_TAG) == message.curruuid
    assert message.by_tag(UNIX_TAG).as_py() == dt.datetime.fromtimestamp(
        (CLOCK_NS + 1_000_000_000) / 1e9, dt.timezone.utc
    )
    assert message.by_tag(CROSSCODE_TAG).as_py() == "A1"
    assert message.by_name("crosscode").as_py() == "A1"
    assert message.get_by_tag(SNAPUNIX_TAG) is None
    # None of them is a row child: the content row holds the rest.
    assert [child.name for child in message.field] == [
        "clordid",
        "symbol",
        "transacttime",
        "checksum",
        "settlcurrency",
        "currencycodesource",
    ]


def test_a_message_settles_its_identity_from_what_it_states(seed: FixRegistry) -> None:
    """The cross code, the hash code and the two identities, settled once."""
    codec = _fixed(seed)
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")

    # The cross code is the first stated of the cross tags, and the cross
    # identity and the cross hash code follow it.
    assert order.crosscode == "A1"
    replaced = codec.parse_fix_line(
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|52=20240102-10:15:30|10=0|"
    )
    # 37, then 11, then 41: a replace states its own ClOrdID first.
    assert replaced.crosscode == "A2"
    reported = codec.parse_fix_line(
        b"8=FIX.4.4|35=8|37=O1|11=A1|55=AAPL|52=20240102-10:15:30|10=0|"
    )
    assert reported.crosscode == "O1"

    # A message naming no cross code takes its own identity as the cross one.
    plain = codec.parse_fix_line(b"8=FIX.4.4|35=0|52=20240102-10:15:30|10=0|")
    assert plain.crosscode == ""
    assert plain.crosshashcode == 0
    assert plain.crossuuid == plain.curruuid

    # Two reads of one line are one message, identities included.
    again = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")
    assert again == order
    assert again.curruuid == order.curruuid
    assert again.hashcode == order.hashcode
    assert again.digest() == order.digest()

    # A write settles it again: the content moves, so the hash code and the
    # own identity move with it while the cross identity stands.
    written = copy.copy(order)
    written.set(55, "MSFT")
    assert written.hashcode != order.hashcode
    assert written.curruuid != order.curruuid
    assert written.crossuuid == order.crossuuid
    # Writing the cross code moves the cross identity too.
    written.set("crosscode", "X-1")
    assert written.crosscode == "X-1"
    assert written.crossuuid != order.crossuuid


def test_a_write_reaches_the_holder_or_the_row_by_the_key_it_resolves(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")
    message = copy.copy(order)

    # A row key types the value through the dictionary's field and keeps the
    # child where it stands; an absent one is appended.
    at = message.field.index_of("symbol")
    message.set(55, "MSFT")
    assert message.field.index_of("symbol") == at
    assert message.by_tag(55).as_py() == "MSFT"
    message.set(1, "ACC-1")
    assert message.by_tag(1).as_py() == "ACC-1"
    assert [child.name for child in message.field][-1] == "account"

    # A typed key writes its holder and never the row. `OrderQty` is one:
    # the quantity the message is about, which the crate's `qty` owns.
    before = len(message)
    message.set(38, 100.0)
    assert message.qty.as_py() == 100
    assert message.by_tag(38).as_py() == 100.0
    assert len(message) == before
    message.set(54, "2")
    assert message.side.as_py() == "SELL"
    assert message.by_tag(54).as_py() == "SELL"
    assert len(message) == before
    message.set(PX_TAG, "12.5")
    assert message.px.as_py() == 12.5
    assert len(message) == before
    message.set(58, "note")
    assert message.text == "note"
    message.set("metadata", {"tech.clientid": "A"})
    assert message.metadata == {"tech.clientid": "A"}
    message.set("identifiers", {"clordid": "C-2"})
    assert message.identifiers == {"clordid": "C-2"}

    # `None` clears a typed fact and is stored as a stated null in the row.
    assert message.remove(54) is not None
    assert message.side.as_py() == "UNKNOWN"
    assert message.remove(54) is None
    message.set(58, None)
    assert message.text is None
    message.set(55, None)
    held = message.get_by_tag(55)
    assert held is not None and held.is_null()

    # A name the dictionary does not know still reaches a child spelled that
    # way, and a bare unknown tag appends a text child named by its decimal.
    message.set(9999, "custom")
    assert message.by_tag(9999).as_py() == "custom"
    assert [child.name for child in message.field][-1] == "9999"
    with pytest.raises(KeyError, match="nosuchfield"):
        message.set("nosuchfield", "y")

    # A removal answers the value and the other tags still reach theirs.
    count = len(message)
    assert message.remove(9999) == Scalar.from_("custom")
    assert len(message) == count - 1
    assert message.remove("nosuchfield") is None
    assert message.by_tag(11).as_py() == "A1"

    # A hashed message is frozen; a copy takes writes again.
    hashed = copy.copy(order)
    held_set = {hashed}
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        hashed.set(55, "MSFT")
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        hashed.remove(55)
    assert hashed in held_set
    written = copy.copy(hashed)
    written.set(55, "MSFT")
    assert written.by_tag(55).as_py() == "MSFT"


def test_the_entries_are_the_row_read_as_a_tree(seed: FixRegistry) -> None:
    """One tuple per stated child, a group's occurrences nested under it."""
    codec = _fixed(seed)
    wire = b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|10=0|"
    message = codec.parse_fix_line(wire)

    assert message.entries() == [
        (11, "clordid", "A1", []),
        (55, "symbol", "AAPL", []),
        (
            453,
            "parties",
            "1",
            [
                (
                    0,
                    "party",
                    None,
                    [
                        (448, "partyid", "BRK", []),
                        (447, "partyidsource", "D", []),
                        (452, "partyrole", "1", []),
                    ],
                )
            ],
        ),
        (0, "9999", "x", []),
        (10, "checksum", "0", []),
    ]
    # The group's counter is the group entry's own value, so the counter
    # child beside it states nothing the entries do not already.
    assert [tag for tag, _, _, _ in message.entries()].count(453) == 1
    # A typed fact is not an entry: the holders answer those.
    assert all(tag not in (8, 35, 52, 54) for tag, _, _, _ in message.entries())

    # The wire puts the header and the event's own tags in front of them, and
    # a derived value the dictionary licensed rides with the row.
    assert message.into_bytes(124) == (
        b"8=FIX.4.4|35=D|59=0|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|10=0|"
    )
    assert message.into_text("|") == message.into_bytes(124).decode()
    assert len(message.digest()) == 16
    assert message.digest() == copy.copy(message).digest()

    # A key no dictionary explains keeps tag 0 and its own spelling, and the
    # wire re-emits it as it arrived.
    unresolved = _fixed(seed, exclude_msgtypes=[]).parse_pairs(
        [("999999", "one"), ("OwnThing", "two")]
    )
    assert unresolved.entries() == [(0, "999999", "one", []), (0, "ownthing", "two", [])]
    assert unresolved.by_tag(999_999).as_py() == "one"
    assert unresolved.get_by_name("OwnThing") is not None
    # The beginstring the parse read the frame at heads the wire whatever
    # the frame itself stated.
    assert unresolved.into_bytes(124).endswith(b"999999=one|ownthing=two|")


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
    assert restored.header() == message.header()
    assert restored.event() == message.event()
    assert restored.by_path("parties[0].partyid").as_py() == "BROKER"

    assert repr(message) == 'FixMsg("NewOrderSingle", 4 values)'


def test_message_refuses_a_value_its_field_refuses(seed: FixRegistry) -> None:
    root = Field("row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False)
    with pytest.raises(ValueError, match="symbol"):
        FixMsg(root, {"symbol": [1]}, seed)
    with pytest.raises(ValueError):
        FixMsg(Field("scalar", "utf8"), {"symbol": "AAPL"}, seed)


INSTALL_SCRIPT = """
import pathlib
import sys

from yggdryl import DataType, Field
from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

seed = FixRegistry.from_handle(pathlib.Path(sys.argv[1]))
install_global_registry(seed)
assert global_registry() == seed
assert global_registry().field_by_tag(55).name == "symbol"

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


def test_message_links_the_process_default_when_none_is_named() -> None:
    default = global_registry()
    assert isinstance(default, FixRegistry)
    assert default == global_registry()

    root = Field("row", DataType.from_fields([_field("symbol", "utf8", 55)]), nullable=False)
    assert FixMsg(root, {"symbol": "AAPL"}).registry == default
    explicit = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    assert FixMsg(root, {"symbol": "AAPL"}, explicit).registry == explicit


def test_reader_parses_every_frame_shape_the_core_reads(seed: FixRegistry) -> None:
    """One reader, five entry points, and each is the core's own."""
    # A bridge row and a FIXML order state no message type, so this reader is
    # told to read the untyped row the default refusals drop.
    reader = _fixed(seed, exclude_msgtypes=[])

    framed = next(reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|"))
    assert framed.by_tag(55).as_py() == "AAPL"
    assert next(reader.parse_line(b"8=FIX.4.4|35=D|55=AAPL|10=0|")).by_tag(55).as_py() == "AAPL"
    assert reader.parse_fix_line(b"8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01").by_tag(55).as_py() == "AAPL"
    assert reader.parse_pairs([("55", "AAPL")]).by_tag(55).as_py() == "AAPL"
    assert reader.parse_fixml_line(b"<Order ClOrdID='XML-1'/>").by_tag(11).as_py() == "XML-1"

    # A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
    # whose value packs its members behind the two control bytes ULLINK uses.
    bridge = reader.parse_ullink_line(
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )
    inferred = next(
        reader.parse_line(
            b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=1"
            b"|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|"
        )
    )
    assert inferred == bridge
    assert bridge.by_tag(55).as_py() == "TTF"
    assert bridge.px.as_py() == 41.25
    assert bridge.qty.as_py() == 1200
    assert bridge.side.as_py() == "BUY"
    assert bridge.by_path("parties[0].partyid").as_py() == "BUYSIDE"

    # A line carrying two frames is two messages; a sentence is none.
    two = list(reader.parse_line(b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O|10=002|"))
    assert [held.header().msgtype for held in two] == ["D", "8"]
    assert list(reader.parse_line(b"no level printed by this plugin")) == []

    # A JSON document is one unknown message stating nothing at all.
    (document,) = list(reader.parse_line(b'{"a":1}'))
    assert MimeType.infer_bytes(b'{"a":1}') == MimeType.JSON
    assert FixCodec.infer_msgtype_bytes(b'{"a":1}') is None
    assert document.field.name == "unknown"
    assert document.entries() == []
    assert document.header().msgtype == ""


def test_the_default_sending_time_is_the_clock_undated_intake_takes(seed: FixRegistry) -> None:
    """The pin that makes a parse of undated bytes repeat."""
    registry = FixRegistry()
    assert FixCodec(registry).default_sending_time is None
    assert FixCodec(registry, default_sending_time=None).default_sending_time is None
    # A `Heartbeat` is what the default refuses, and this case is about the
    # clock an undated message takes, so this codec reads every type.
    codec = FixCodec(registry, default_sending_time=CLOCK, exclude_msgtypes=[])
    assert codec.default_sending_time == CLOCK
    # An aware UTC `datetime` is read once into the same nanosecond clock.
    assert FixCodec(registry, default_sending_time=CLOCK_INSTANT).default_sending_time == CLOCK

    wire = b"8=FIX.4.4|35=0|10=0|"
    first = next(codec.parse_line(wire))
    second = next(codec.parse_line(wire))
    assert first == second
    assert first.hashcode == second.hashcode
    assert first.digest() == second.digest()
    assert first.header().sendingtime == CLOCK_NS
    assert not first.header().stated_sendingtime
    assert first.unix == CLOCK_NS
    assert first.event().creatunix == CLOCK_NS
    # A settled clock is not the message's own, so the wire does not state it.
    assert first.into_bytes(ord("|")) == wire

    # A message's own clocks precede the pin: SendingTime is the stated one,
    # and TransactTime settles the event and the creation.
    stated = next(
        codec.parse_line(
            b"8=FIX.4.4|35=0|52=19700101-00:00:02.123456789|60=19700101-00:00:03.987654321|10=0|"
        )
    )
    assert stated.by_tag(52) == DataType('datetime64(ns,"UTC")').scalar(2_123_456_789)
    assert stated.header().sendingtime == 2_123_456_789
    assert stated.header().stated_sendingtime
    assert stated.unix == 3_987_654_321
    assert stated.event().creatunix == 3_987_654_321
    # `OrigSendingTime(122)` is when a resent message came into being, and a
    # TransactTime stating only a day names no instant, so the sending clock
    # stands in. Both are read off the dictionary's own fields.
    dictionary = _fixed(seed, exclude_msgtypes=[])
    resent = next(dictionary.parse_line(b"8=FIX.4.4|35=0|122=19700101-00:00:01|10=0|"))
    assert resent.event().creatunix == 1_000_000_000
    day = next(dictionary.parse_line(b"8=FIX.4.4|35=D|11=A|60=20260814|10=0|"))
    assert day.unix == CLOCK_NS

    # The pin is exact: another unit, a naive clock or text is refused.
    for refused in (DataType('datetime64(us,"UTC")').scalar(0), DataType("datetime64(ns)").scalar(0), "1970-01-01T00:00:00Z"):
        with pytest.raises(ValueError, match="default_sending_time"):
            FixCodec(registry, default_sending_time=refused)


def test_a_parse_fills_what_the_line_implied_and_leaves_the_wire_alone(seed: FixRegistry) -> None:
    """The rules run inside the parse: there is no enriching step of its own."""
    reader = _fixed(seed)

    # A `SecurityID` an ISIN's check digit closes has stated its source, and
    # under that source the crate's `isincode` fact and the country.
    line = b"8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|"
    filled = next(reader.parse_line(line))
    assert filled.by_tag(22).as_py() == "4"
    assert filled.event().isincode is not None
    assert filled.event().isincode.as_py() == "US0378331005"
    assert filled.by_tag(470).as_py() == "US"
    # An order stating no time in force is a day order.
    assert filled.by_tag(59).as_py() == "0"
    # The wire is the message as it now stands: what the line carried, then
    # the pairs the dictionary derived from it.
    assert filled.into_bytes(ord("|")) == (
        b"8=FIX.4.4|35=D|59=0|11=A|48=US0378331005|10=0|22=4|470=US|"
    )

    # A value no standard closes answers nothing rather than a guess.
    opaque = next(reader.parse_line(b"8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|"))
    assert opaque.get_by_tag(22) is None
    assert opaque.event().isincode is None

    # The message component's identifiers fill the event's own map.
    report = reader.parse_fix_line(b"8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|")
    assert report.identifiers == {"clordid": "C-001", "execid": "E-09", "orderid": "O-01"}
    assert report.by_tag(IDENTIFIERS_TAG).as_py() == report.identifiers
    # An unknown message type declares none.
    unknown = reader.parse_fix_line(b"8=FIX.4.4|35=ZZ|11=C-1|10=0|")
    assert unknown.identifiers == {}
    assert unknown.header().msgtype == "ZZ"


REPORT = (
    b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|"
    b"14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|"
)


def test_a_parse_restates_deprecated_fields_to_their_latest_aliases(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    latest = next(codec.parse_line(REPORT))

    # ExecType PartiallyFilled is restated as Trade; Rule80A A is an agency
    # order; ExecBroker and ClientID are two parties, in tag order, counted.
    assert latest.by_tag(150).as_py() == "F"
    assert latest.by_tag(528).as_py() == "A"
    assert latest.by_tag(453).as_py() == 2
    assert latest.by_path("parties[0].partyid").as_py() == "BRKR"
    assert latest.by_path("parties[1].partyid").as_py() == "CLIENT1"
    # The fill under its newest spelling, reachable by the old one too.
    assert latest.by_tag(32).as_py() == 100.0
    assert latest.by_name("LastShares").as_py() == 100.0
    # `LastQty` is the quantity the event last traded, so it is a typed fact
    # and no longer a child of the content row.
    assert [name for name, _ in latest].count("lastqty") == 0
    assert latest.lastqty is not None and latest.lastqty.as_py() == 100
    # The state the event reached, as the crate's ranked spelling.
    assert latest.state.as_py() == "40PARTFILL"
    # And the rules the dictionary licenses: one fill's average is its price.
    assert latest.by_tag(6).as_py() == 10.5
    # The version it was read at is the header's, not a column.
    assert latest.header().beginstring == "FIX.4.2"


# The messages of one order's life: the order, its acknowledgement, a partial
# fill, a replace whose new identifier names the old one, its acknowledgement
# and the fill under the new identifier alone.
LIFE = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|",
]


def test_the_lifecycle_states_each_message_as_the_one_it_follows(seed: FixRegistry) -> None:
    """The one walk, over any iterable, lazily."""
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(LIFE))
    # A parse chains nothing: every message stands alone.
    assert all(held.seqnum == 0 and held.prevuuid is None for held in parsed)

    stream = codec.lifecycle(parsed)
    assert isinstance(stream, FixMessages)
    walked = list(stream)
    assert len(walked) == len(LIFE)

    # One chain: the acknowledgement and the fill follow the order under the
    # cross identity its client identifier derives.
    assert len({held.crossuuid.as_py() for held in walked}) == 1
    assert [held.seqnum for held in walked] == [0, 1, 2]
    assert walked[0].prevuuid is None
    for earlier, later in zip(walked, walked[1:]):
        assert later.prevuuid == earlier.curruuid
        assert later.event().prevunix == earlier.unix
        assert earlier.curruuid in later.parentuuids
    # The lifecycle's own creation instant is carried forward.
    assert {held.event().creatunix for held in walked} == {walked[0].event().creatunix}
    # The state moves with the messages.
    assert [held.state.as_py() for held in walked] == ["00UNKNOWN", "20NEW", "40PARTFILL"]

    # The walk states what a message follows and touches the content of
    # none of them: the row and the entries are the parse's own. The wire is
    # not, and deliberately - a message that named no side is about the side
    # the order it follows named, and re-emits it.
    for before, after in zip(parsed, walked):
        assert after.entries() == before.entries()
        assert after.value == before.value

    # A second walk over a walked stream changes nothing.
    assert list(codec.lifecycle(walked)) == walked

    # The walk sorts by instant, so a stream that arrived out of order is
    # chained in the order the messages happened in.
    reordered = list(codec.lifecycle([parsed[2], parsed[0], parsed[1]]))
    assert [held.unix for held in reordered] == sorted(held.unix for held in reordered)
    assert [held.seqnum for held in reordered] == [0, 1, 2]

    # A message naming no chain has an identity and no predecessor. A
    # `Heartbeat` is what the default refuses, so this reader is told to
    # read it.
    session = _fixed(seed, exclude_msgtypes=[])
    heartbeat = next(session.parse_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|"))
    (held,) = list(session.lifecycle(item for item in [heartbeat]))
    assert held.crosscode == ""
    assert held.crossuuid == held.curruuid
    assert held.prevuuid is None

    # An item that is not a message is refused where it is met.
    mixed = session.lifecycle([heartbeat, b"8=FIX.4.4|35=0|10=0|"])
    assert next(mixed) == held
    with pytest.raises(TypeError):
        next(mixed)
    assert next(mixed, None) is None


def test_the_fixed_row_is_named_by_fold_and_never_shifts(seed: FixRegistry) -> None:
    """The one shape a whole capture lands in."""
    schema = fix_schema(seed, "FixMessage")
    columns = [child.name for child in schema]
    assert schema.name == "FixMessage"
    # Every tag the listing names is some column's identity, and a group's
    # counter names the group beside it, so there are columns the listing
    # does not repeat.
    held = {child.fix.tag for child in schema} | {child.fix.counter for child in schema}
    assert set(fix_schema_tags()) <= held
    assert len(columns) >= len(fix_schema_tags())

    # The crate's own clocks open the row - a table is read by time - and
    # the one arrival record closes it under the counter that counts it.
    assert columns[0] == "unix"
    assert columns[-2:] == ["nofixentries", "fixentries"]
    assert [child.fix.tag for child in schema][-2:] == [NOFIXENTRIES_TAG, None]
    assert columns.count("msgdirection") == 1

    # A column is found by the name the dictionary spells, never by digits.
    assert schema.index_of("msgtype") is not None
    assert schema.index_of("35") is None
    # The tag stays the identity: each column carries its field's.
    at = schema.index_of("msgtype")
    assert fix_schema_tags()[at] == 35
    assert schema[at].fix.tag == 35

    # The columns every message settles are the non-null ones; which band
    # each falls in is the core's to order.
    assert {child.name for child in schema if not child.nullable} == {
        "unix",
        "creatunix",
        "hashcode",
        "crosshashcode",
        "curruuid",
        "crossuuid",
        "beginstring",
    }
    # The facts a row derives are typed as the thing they hold.
    assert schema[schema.index_of("isincode")].dtype == DataType("isin")
    assert schema[schema.index_of("miccode")].dtype == DataType("mic")
    assert schema[schema.index_of("state")].dtype == DataType("state")
    assert schema[schema.index_of("curruuid")].dtype == DataType("uuid")
    assert schema[schema.index_of("identifiers")].fix.counter == IDENTIFIERS_TAG

    # The retired columns are gone rather than renamed.
    for retired in ("updatedat", "createdat", "msghash", "msgphash", "altids", "code", "version"):
        assert schema.index_of(retired) is None, retired

    reader = _fixed(seed)
    wire = b"8=FIX.4.4|35=D|11=A|52=20240102-10:15:30|9999=x|VenueOwnThing=y|10=0|"
    message = next(reader.parse_line(wire))
    row = message.into_row(schema).as_py()
    assert len(row) == len(columns)
    assert row[schema.index_of("beginstring")] == "FIX.4.4"
    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("clordid")] == "A"
    assert row[schema.index_of("unix")] == CLOCK_INSTANT
    assert row[schema.index_of("creatunix")] == CLOCK_INSTANT
    assert row[schema.index_of("sendingtime")] == CLOCK_INSTANT
    assert row[schema.index_of("crosscode")] == "A"
    assert row[schema.index_of("hashcode")] == message.hashcode
    assert str(row[schema.index_of("curruuid")]) == message.curruuid.as_py()
    assert row[schema.index_of("identifiers")] == {"clordid": "A"}
    # A read is not a snapshot, and a fact the message gave nothing for is
    # null rather than a shift.
    assert row[schema.index_of("snapunix")] is None
    assert row[schema.index_of("state")] is None
    assert row[schema.index_of("prevunix")] is None
    assert row[schema.index_of("msgsessionid")] is None
    assert row[schema.index_of("nofixentries")] == len(message.entries())

    # The record closes the row with everything that arrived, a key no
    # dictionary explained under tag 0 and its own name.
    # A row cell is the ordered sequence its Struct declares: the tag, the
    # canonical name, the value, and the entries nested under it.
    arrived = row[schema.index_of("fixentries")]
    assert [entry[0] for entry in arrived] == [11, 0, 0, 10]
    assert [entry[1] for entry in arrived if entry[0] == 0] == ["9999", "venueownthing"]
    assert arrived == [list(entry) for entry in message.entries()]


def test_a_captures_own_columns_lead_the_row(seed: FixRegistry) -> None:
    """Where a line was read from is what a monitor orders and joins on."""
    carrier = Field(
        "line",
        DataType.from_fields([Field("url", DataType("utf8")), Field("body", DataType("binary"))]),
        nullable=False,
    )
    plain = fix_schema(seed, "FixMessage")
    carried = fix_schema_carrying(carrier, plain)

    assert [child.name for child in carried][:2] == ["url", "body"]
    assert len(carried) == len(plain) + 2
    assert carried.index_of("msgtype") == plain.index_of("msgtype") + 2

    # A column no tag names is the capture's, so a row answers null there.
    message = next(_fixed(seed).parse_line(b"8=FIX.4.4|35=D|10=0|"))
    row = message.into_row(carried).as_py()
    assert row[0] is None
    assert row[carried.index_of("msgtype")] == "D"

    # A required capture column the message cannot fill refuses the
    # projection rather than publishing a null into it.
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
    # front: `msgSessionId` and `msgsessionid` are one name.
    named = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", DataType("utf8"), nullable=False),
                Field("msgSessionId", DataType("utf8")),
                Field("body", DataType("binary"), nullable=False),
            ]
        ),
        nullable=False,
    )
    folded = fix_schema_carrying(named, plain)
    names = [child.name for child in folded]
    assert names[:2] == ["url", "body"]
    assert "msgSessionId" not in names
    assert names.count("msgsessionid") == 1
    assert len(folded) == len(plain) + 2


def test_a_rows_own_columns_feed_the_message(seed: FixRegistry) -> None:
    """A capture's clock is context; its other columns fill what they name."""
    clock = dt.datetime(2026, 1, 2, 10, 15, 30, 500000, tzinfo=dt.timezone.utc)
    source = pa.table(
        {
            "timestamp": pa.array([clock, None], pa.timestamp("us", tz="UTC")),
            "msgsessionid": pa.array(["e7254b22", None], pa.utf8()),
            "msgseqnum": pa.array([4507, None], pa.int64()),
            "body": pa.array(
                [
                    b"8=FIX.4.4|35=D|55=AAPL|10=0|",
                    b"8=FIX.4.4|35=0|34=696|52=20260102-09:29:59.250|10=0|",
                ],
                pa.binary(),
            ),
        }
    )
    # The second line is a `Heartbeat`, which the default refuses, and this
    # case is about what a capture's own columns fill: read every type.
    parsed = _fixed(seed, exclude_msgtypes=[]).parse_text_arrow_reader(source).read_all()
    names = parsed.schema.names

    # A capture column whose folded name a fixed column takes fills that
    # column instead of riding in front; no fixed column is named
    # `timestamp`, so the capture's clock is carried as context.
    assert names[:2] == ["timestamp", "body"]
    for once in ("timestamp", "msgsessionid", "msgseqnum"):
        assert names.count(once) == 1, once
    assert names[-2:] == ["nofixentries", "fixentries"]

    # The capture's clock stamps nothing: the wire's own clock settles the
    # message, else the codec's default SendingTime.
    instant = dt.datetime(2026, 1, 2, 9, 29, 59, 250000, tzinfo=dt.timezone.utc)
    assert parsed.column("timestamp").to_pylist() == [clock, None]
    assert parsed.column("unix").to_pylist() == [CLOCK_INSTANT, instant]
    # Only a stated SendingTime lands in its column: the first row settled
    # on the codec's default and states none of its own.
    assert parsed.column("sendingtime").to_pylist() == [None, instant]
    assert parsed.column("snapunix").to_pylist() == [None, None]
    # A column spelled for a field fills it where the frame stated none.
    assert parsed.column("msgsessionid").to_pylist() == ["e7254b22", None]
    assert parsed.column("msgseqnum").to_pylist() == [4507, 696]
    # Row-only: what the row filled is never an entry.
    assert [[entry["tag"] for entry in row] for row in parsed.column("fixentries").to_pylist()] == [
        [55, 10],
        [10],
    ]


def test_a_rows_pluginid_fills_its_field_and_selects_nothing(seed: FixRegistry) -> None:
    """A row's `pluginid` fills the capture's own fact; one namespace."""
    seed.insert(_field("VenueTag", "utf8", 5001, branches=["venue"]))
    codec = _fixed(seed)
    body = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark"
    spellings = ["venue", "OMS_X1_TradeCapture", None]
    source = pa.table(
        {
            "pluginid": pa.array(spellings, pa.utf8()),
            "body": pa.array([body] * len(spellings), pa.binary()),
        }
    )
    parsed = codec.parse_text_arrow_reader(source).read_all()
    assert parsed.schema.names[0] == "body"
    assert parsed.schema.names.count("pluginid") == 1
    assert parsed.column("pluginid").to_pylist() == spellings
    # The plugin selects nothing: the venue's key maps to its field on every
    # row, so no arrival is left unresolved.
    for row in parsed.column("fixentries").to_pylist():
        assert [entry["tag"] for entry in row] == [11, 5001]

    # One line read alone answers exactly what the batch did, and the plugin
    # is a fact about the capture rather than an entry.
    lined = _fixed(seed, capture_names=["pluginid"])
    for spelled in spellings:
        message = next(lined.parse_text_line(TextLine(0, body, [spelled])))
        assert message.by_tag(5001).as_py() == "dark", spelled
        assert message.capture().pluginid == spelled, spelled
        assert all(tag != 65009 for tag, _, _, _ in message.entries()), spelled

    # A stated `msgdirection` column is the row's direction, read once and
    # carried by the header.
    # The second row is a bridge frame stating no type, which the default
    # refuses, and this case is about the direction column.
    spoken = (
        _fixed(seed, capture_names=["pluginid"], exclude_msgtypes=[])
        .parse_text_arrow_reader(
            pa.table(
                {
                    "msgdirection": pa.array(["Send", "R"], pa.utf8()),
                    "body": pa.array([b"MSGTYPE=D|CLORDID=A", b"|#SYMBOL=TTF|"], pa.binary()),
                }
            )
        )
        .read_all()
    )
    assert spoken.schema.names.count("msgdirection") == 1
    assert spoken.column("msgdirection").to_pylist() == ["S", "R"]
