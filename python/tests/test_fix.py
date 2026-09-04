"""The FIX boundary: the typed vocabulary, the registry, and the message.

Every answer here is the core's; what these check is the crossing - the key
coercion, the exception each core failure maps to, the storage locations a
Python caller names, and the Python protocols the two wrappers implement.
"""

from __future__ import annotations

import copy
import decimal
import pathlib
import pickle
import subprocess
import sys
from typing import Any, Iterable

import pytest

from yggdryl import DataType, Field, IOBase, Scalar, Url
from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
SEED = REPO / "config" / "fix"


def _field(
    name: str,
    dtype: str,
    tag: int,
    *,
    tags: Iterable[int] = (),
    aliases: Iterable[str] = (),
    description: str | None = None,
    nullable: bool = True,
) -> Field:
    """One FIX field, written through the protocol view alone."""
    field = Field(name, dtype, nullable=nullable)
    field.fix.tag = tag
    if tags:
        field.fix.tags = tags
    if aliases:
        field.fix.aliases = aliases
    if description is not None:
        field.fix.description = description
    return field


@pytest.fixture
def seed() -> FixRegistry:
    """The dictionary the repository tracks at ``config/fix``."""
    return FixRegistry.from_handle(SEED)


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
    assert len(field.fix) == 4

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
            view.tag = 55
    # The mapping protocol still works on every view, including this one.
    assert field.protocol("fix")["tag"] == "55"


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


def test_registry_resolves_every_key_the_way_the_core_does(seed: FixRegistry) -> None:
    assert len(seed) == 34
    assert bool(seed)

    assert seed.field_by_tag(55).name == "Symbol"
    assert seed.get_field_by_tag(55) == seed.field_by_tag(55)
    # The alternate tag 20 reaches ExecType, which claims 150 canonically.
    assert seed.field_by_tag(20).name == "ExecType"
    assert seed.field_by_tag(150).name == "ExecType"
    # A name answers the canonical spelling whatever case it was asked in.
    assert seed.field_by_name("symbol").name == "Symbol"
    assert seed.field_by_name("SYMBOL").name == "Symbol"
    assert seed.field_by_name("ticker").name == "Symbol"
    assert seed.field_by_name("clientorderid").name == "ClOrdID"
    # A path reaches a repeating group and one of its members.
    assert seed.field_by_path("NoPartyIDs").fix.tag == 453
    assert seed.field_by_path("NoPartyIDs.PartyID").fix.tag == 448
    assert seed.field_by_path("nopartyids.item.PartyRole").name == "PartyRole"

    # The generic pair answers exactly what the specialized one does.
    for key in (55, "Symbol", "ticker", "NoPartyIDs.PartyID", 20):
        assert seed.get_field(key) == seed[key]
        assert seed.field(key) == seed[key]
        assert key in seed
    assert 9999 not in seed
    assert "Nope" not in seed
    assert seed.get_field(9999) is None
    assert seed.get(9999) is None
    assert seed.get(9999, "fallback") == "fallback"
    assert seed.get(55) == seed[55]


def test_registry_absence_is_a_key_error_carrying_the_core_message(
    seed: FixRegistry,
) -> None:
    # ``KeyError`` renders its argument as a repr, so the native message is
    # read off the argument itself rather than off the rendering.
    with pytest.raises(KeyError) as by_tag:
        seed.field_by_tag(9999)
    assert by_tag.value.args[0] == 'expected a fix field at "tag 9999", got nothing'

    with pytest.raises(KeyError) as by_name:
        seed.field_by_name("Nope")
    assert 'name \\"Nope\\"' in by_name.value.args[0]

    with pytest.raises(KeyError) as by_path:
        seed.field_by_path("Symbol.absent")
    assert 'path \\"Symbol.absent\\"' in by_path.value.args[0]

    with pytest.raises(KeyError):
        seed[9999]
    assert seed.get_field_by_name("Nope") is None
    assert seed.get_field_by_path("Symbol.absent") is None


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


def test_registry_iterates_lazily_in_canonical_tag_order(seed: FixRegistry) -> None:
    names = [field.name for field in seed]
    assert names[:4] == ["Account", "AvgPx", "BeginString", "BodyLength"]
    assert len(names) == len(seed)

    walk = iter(seed)
    assert next(walk).name == "Account"
    assert next(walk).name == "AvgPx"
    # An unfinished walk shares the registry, so a mutation refuses until it
    # is dropped rather than moving the fields under the cursor.
    with pytest.raises(ValueError, match="shared with a message"):
        seed.remove(1)
    del walk
    assert seed.remove(1) is not None

    tags = [field.fix.tag for field in seed]
    assert tags == sorted(tags)


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

    # A folder that is not there loads as empty and is not created.
    missing = tmp_path / "missing"
    assert not FixRegistry.from_handle(missing)
    assert not missing.exists()


def test_registry_round_trips_through_a_written_folder(
    seed: FixRegistry, tmp_path: pathlib.Path
) -> None:
    root = tmp_path / "dictionary"
    seed.write_into(root)

    shards = sorted(path.name for path in (root / "records").iterdir())
    assert shards == ["0.json", "1.json", "4.json"]
    assert FixRegistry.from_handle(root) == seed

    reloaded = FixRegistry.from_handle(IOBase(root))
    reloaded.remove(453)
    reloaded.remove("PartyID")
    reloaded.remove(447)
    reloaded.remove(452)
    reloaded.write_into(root)
    assert not (root / "records" / "4.json").exists()
    assert len(FixRegistry.from_handle(root)) == len(seed) - 4


def test_registry_insert_update_and_remove(seed: FixRegistry) -> None:
    registry = FixRegistry.from_fields(
        [
            _field("Symbol", "utf8", 55, aliases=["Ticker"]),
            _field("Price", "decimal128(20, 8)", 44, aliases=["Px"]),
        ]
    )
    assert len(registry) == 2
    assert registry.insert(_field("Side", "utf8", 54)) is None
    assert registry.field_by_tag(54).name == "Side"

    # A key another field holds is refused, naming both; nothing changes.
    with pytest.raises(ValueError, match="held by Symbol"):
        registry.insert(_field("SymbolSfx", "utf8", 65, aliases=["ticker"]))
    assert len(registry) == 3

    # A merge concatenates the two list properties, incoming first.
    registry.update(_field("SYMBOL", "utf8", 55, tags=[65], aliases=["Sym"]))
    merged = registry.field_by_tag(65)
    assert merged.name == "SYMBOL"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(_field("Symbol", "large_utf8", 55))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    removed = registry.remove("sym")
    assert removed is not None and removed.name == "SYMBOL"
    assert registry.get_field_by_tag(65) is None
    assert registry.remove(9999) is None

    # A field with no tag cannot enter at all.
    with pytest.raises(ValueError, match="fix:tag"):
        registry.insert(Field("Untagged", "utf8"))
    assert seed.get_field_by_name("Untagged") is None


def test_registry_mutation_refuses_while_something_shares_it(
    seed: FixRegistry,
) -> None:
    root = Field(
        "row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False
    )
    message = FixMsg(root, {"Symbol": "AAPL"}, seed)

    for mutation in (
        lambda: seed.insert(_field("Side", "utf8", 54)),
        lambda: seed.update(_field("Symbol", "utf8", 55)),
        lambda: seed.remove(55),
    ):
        with pytest.raises(ValueError, match="shared with a message"):
            mutation()
    assert message.registry == seed

    # The registry a message shares is still readable, and a copy is writable.
    assert seed.field_by_tag(55).name == "Symbol"
    fresh = FixRegistry.from_handle(SEED)
    assert fresh.remove(55) is not None


def _order(seed: FixRegistry) -> Field:
    """A root that carries a group and one tag no dictionary explains."""
    return Field(
        "NewOrderSingle",
        DataType.from_fields(
            [
                seed.field_by_tag(55),
                seed.field_by_tag(38),
                seed.field_by_name("NoPartyIDs"),
                Field("9999", "utf8"),
            ]
        ),
        nullable=False,
    )


ORDER_VALUE: dict[str, Any] = {
    "Symbol": "AAPL",
    "OrderQty": decimal.Decimal("100"),
    "NoPartyIDs": [{"PartyID": "BROKER", "PartyIDSource": "D", "PartyRole": 1}],
    "9999": "custom",
}


def test_message_resolves_through_the_registry_it_carries(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)

    assert message.field == root
    assert message.registry == seed
    assert len(message) == 4
    assert message.by_tag(55).as_py() == "AAPL"
    assert message.by_name("ticker").as_py() == "AAPL"
    assert message.by_tag(38).as_py() == decimal.Decimal("100")
    assert message.by_path("NoPartyIDs.0.PartyID").as_py() == "BROKER"
    # An unknown tag is retained under its rendered name, never dropped.
    assert message.by_tag(9999).as_py() == "custom"

    assert message[55] == message.by_tag(55)
    assert message["ticker"] == message.by_tag(55)
    assert message.get(55) == message.by_tag(55)
    assert message.get(1234) is None
    assert message.get(1234, "fallback") == "fallback"
    assert message.get_by_name("nope") is None
    assert message.get_by_path("NoPartyIDs.PartyID") is None
    with pytest.raises(KeyError) as by_tag:
        message.by_tag(1234)
    assert by_tag.value.args[0] == 'expected a fix value at "tag 1234", got nothing'
    with pytest.raises(KeyError) as by_name:
        message.by_name("nope")
    assert 'name \\"nope\\"' in by_name.value.args[0]
    with pytest.raises(KeyError) as by_path:
        message.by_path("NoPartyIDs.PartyID")
    assert 'path \\"NoPartyIDs.PartyID\\"' in by_path.value.args[0]
    with pytest.raises(TypeError, match="not bool"):
        message[True]

    # The mapping input became the ordered row the root declares.
    pairs = [(name, value.as_py()) for name, value in message]
    assert [name for name, _ in pairs] == [child.name for child in root]
    assert pairs[0] == ("Symbol", "AAPL")

    # A native Scalar names the same row.
    assert FixMsg(root, message.value, seed) == message


def test_message_refuses_a_value_its_field_refuses(seed: FixRegistry) -> None:
    root = Field(
        "row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False
    )
    with pytest.raises(ValueError, match="Symbol"):
        FixMsg(root, {"Symbol": 5})
    with pytest.raises(ValueError):
        FixMsg(Field("scalar", "utf8"), {"Symbol": "AAPL"})


def test_message_links_the_process_default_when_none_is_named() -> None:
    default = global_registry()
    assert isinstance(default, FixRegistry)
    # Whatever this machine has installed, the two calls answer one registry.
    assert default == global_registry()

    root = Field(
        "row", DataType.from_fields([_field("Symbol", "utf8", 55)]), nullable=False
    )
    linked = FixMsg(root, {"Symbol": "AAPL"})
    assert linked.registry == default
    # An explicit registry is kept instead.
    explicit = FixRegistry.from_fields([_field("Symbol", "utf8", 55)])
    assert FixMsg(root, {"Symbol": "AAPL"}, explicit).registry == explicit


def test_message_is_hashable_copyable_and_picklable(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)
    same = FixMsg(root, ORDER_VALUE, seed)

    assert message == same
    assert hash(message) == hash(same)
    assert message.stable_hash() == same.stable_hash()
    assert len({message, same}) == 1
    assert message != FixMsg(root, {**ORDER_VALUE, "Symbol": "MSFT"}, seed)
    assert message != object()

    assert copy.copy(message) == message
    assert copy.deepcopy(message) == message
    restored = pickle.loads(pickle.dumps(message))
    assert restored == message
    assert restored.registry == seed
    assert restored.by_path("NoPartyIDs.0.PartyID").as_py() == "BROKER"

    assert repr(message) == 'FixMsg("NewOrderSingle", 4 values)'
    assert repr(seed) == "FixRegistry(34 fields)"
    assert repr(FixRegistry()) == "FixRegistry(0 fields)"


INSTALL_SCRIPT = """
import pathlib
import sys

from yggdryl import DataType, Field
from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

seed = FixRegistry.from_handle(pathlib.Path(sys.argv[1]))
install_global_registry(seed)
assert global_registry() == seed
assert global_registry().field_by_tag(55).name == "Symbol"

root = Field(
    "row",
    DataType.from_fields([global_registry().field_by_tag(55)]),
    nullable=False,
)
assert FixMsg(root, {"Symbol": "AAPL"}).registry == seed

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
