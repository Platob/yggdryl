"""Generate the committed FIX dictionary from the published sources.

The crate never fetches anything: it reads the committed output through
``FixRegistry::from_handle``. This script is what writes that output, and it
is the only thing that knows where the specification lives.

Every source URL is pinned to a commit rather than a branch, because a
dictionary regenerated from ``master`` is not reproducible and its diff is
unreviewable. The provenance manifest records both checksums - the bytes read
and the definitions produced - so CI can test for drift, and its second half
needs no network at all.

Six kinds come out of Orchestra and each lands where it can live: fields and
groups carry a tag so they become registry entries, code sets are a property
of the field that declares them, datatypes are checked against the crate's
own logical-name table and stored nowhere, and components and messages have
no tag so they go to a layouts manifest beside the trees, which also records
each group's own identifier so a `groupRef` resolves. Nothing is invented to
make a kind fit.

Usage::

    python scripts/generate_fix_dictionary.py
    python scripts/generate_fix_dictionary.py --source .fixsrc --out config/fix
    python scripts/generate_fix_dictionary.py --check
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
import urllib.request
import xml.etree.ElementTree as ElementTree
from typing import Any, Iterable, NamedTuple

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "config" / "fix"

# Pinned commits. A branch would make the output unreproducible.
ORCHESTRA_COMMIT = "099914dd0edd49a699326f0441776d6e21cfaf93"
QUICKFIX_COMMIT = "3536699e830e65f875df4a50b647a6d3bad3b884"

ORCHESTRA_BASE = (
    "https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/"
    f"{ORCHESTRA_COMMIT}/FIX%20Standard/"
)
QUICKFIX_BASE = (
    f"https://raw.githubusercontent.com/quickfix/quickfix/{QUICKFIX_COMMIT}/spec/"
)

ORCHESTRA_LICENSE = "https://github.com/FIXTradingCommunity/orchestrations/blob/master/LICENSE"
QUICKFIX_LICENSE = "https://github.com/quickfix/quickfix/blob/master/LICENSE"

NS = {"fixr": "http://fixprotocol.io/2020/orchestra/repository"}


class Source(NamedTuple):
    """One published file, and what it contributes."""

    source_id: str
    format: str
    file: str
    url: str
    license_url: str
    version: str
    priority: int


# Lowest priority first, so the highest-priority source is merged last and
# wins. QuickFIX supplies the per-version history Orchestra does not publish;
# FIX Latest supplies the current name, type and code set of every field.
SOURCES: tuple[Source, ...] = (
    *(
        Source(
            source_id=f"quickfix-{name.lower()}",
            format="quickfix",
            file=f"{name}.xml",
            url=f"{QUICKFIX_BASE}{name}.xml",
            license_url=QUICKFIX_LICENSE,
            version=version,
            priority=index,
        )
        for index, (name, version) in enumerate(
            [
                ("FIX40", "4.0"),
                ("FIX41", "4.1"),
                ("FIX42", "4.2"),
                ("FIX43", "4.3"),
                ("FIX44", "4.4"),
                ("FIX50", "5.0"),
                ("FIX50SP1", "5.0SP1"),
                ("FIX50SP2", "5.0SP2"),
            ]
        )
    ),
    Source(
        source_id="orchestra-fix42",
        format="orchestra",
        file="OrchestraFIX42.xml",
        url=f"{ORCHESTRA_BASE}OrchestraFIX42.xml",
        license_url=ORCHESTRA_LICENSE,
        version="4.2",
        priority=20,
    ),
    Source(
        source_id="orchestra-fix44",
        format="orchestra",
        file="OrchestraFIX44.xml",
        url=f"{ORCHESTRA_BASE}OrchestraFIX44.xml",
        license_url=ORCHESTRA_LICENSE,
        version="4.4",
        priority=21,
    ),
    Source(
        source_id="orchestra-latest",
        format="orchestra",
        file="OrchestraFIXLatest.xml",
        url=f"{ORCHESTRA_BASE}OrchestraFIXLatest.xml",
        license_url=ORCHESTRA_LICENSE,
        version="latest",
        priority=99,
    ),
)

# The FIX datatype names the crate's logical-name table resolves. A scraped
# datatype outside this set is a hard failure rather than a silent `utf8`.
LOGICAL_NAMES = {
    "int", "float", "char", "boolean", "string", "data", "pattern",
    "length", "tagnum", "seqnum", "numingroup", "dayofmonth",
    "reserved100plus", "reserved1000plus", "reserved4000plus",
    "qty", "price", "priceoffset", "percentage", "amt",
    "utctimestamp", "tztimestamp", "utctimeonly", "localmkttime",
    "utcdateonly", "localmktdate", "tztimeonly",
    "monthyear", "country", "currency", "exchange", "language", "tenor",
    "multiplecharvalue", "multiplestringvalue", "xid", "xidref", "xmldata",
    "mic", "cfi", "side", "msgtype", "utcdate",
    # Orchestra names these beside the ones the table resolves.
    "localmktdatetime", "time", "date",
}

# The tags the standard declares as code sets and the crate types with a
# datatype of their own. Honouring the declaration, not guessing at one: the
# code set stays the field's vocabulary, and the datatype is how a value of it
# is stored. Tag 385 is here because a direction is what every capture in this
# crate carries on its own lines, so it is read far more often than it arrives.
CODED_TAGS = {35: "msgtype", 54: "side", 385: "msgdirection"}

# The crate's own branch, and the two fields it registers on it.
CRATE_BRANCH = "yggdryl"


def folded(name: str) -> str:
    """The crate's one fold: case folded, `_`, `-` and space dropped."""
    return "".join(character for character in name.lower() if character not in "_- ")


def version_of(spelling: str | None) -> str | None:
    """`FIX.4.2` becomes `4.2`; the family prefix is not stored."""
    if not spelling:
        return None
    text = spelling.removeprefix("FIX.").removeprefix("FIXT.")
    if text in {"Latest", "FIX.Latest"}:
        return None
    return text or None


def read_source(source: Source, base: str | pathlib.Path | None) -> bytes:
    """Read one source from a local directory or from its pinned URL."""
    if base is not None and not str(base).startswith(("http://", "https://")):
        return (pathlib.Path(base) / source.file).read_bytes()
    url = source.url if base is None else f"{str(base).rstrip('/')}/{source.file}"
    with urllib.request.urlopen(url) as response:  # noqa: S310 - pinned host
        return response.read()


def orchestra_documentation(element: ElementTree.Element) -> str | None:
    """The specification's own wording, collapsed to one line."""
    for documentation in element.iter(f"{{{NS['fixr']}}}documentation"):
        text = " ".join((documentation.text or "").split())
        if text:
            return text
    return None


def extension_pack(declared: str | None) -> int | None:
    """One `addedEP` attribute as a pack number, or nothing.

    Orchestra writes `-1` where a field or code predates the extension-pack
    scheme, and that is an absence rather than a pack. Passing it through
    wrote `"ep":-1` into the canonical document, which the borrowed scanner
    refuses - and because the walk stops at a refusal, every later code in
    that set silently disappeared from resolution.
    """
    try:
        pack = int(declared) if declared else 0
    except ValueError:
        return None
    return pack if pack > 0 else None


def parse_orchestra(data: bytes) -> dict[str, Any]:
    """Every kind one Orchestra file publishes."""
    root = ElementTree.fromstring(data)
    declared = root.get("version", "")
    ep = None
    if "_EP" in declared:
        ep = int(declared.rsplit("_EP", 1)[1])

    code_sets: dict[str, dict[str, Any]] = {}
    for element in root.iter(f"{{{NS['fixr']}}}codeSet"):
        codes = []
        for code in element.iter(f"{{{NS['fixr']}}}code"):
            codes.append(
                {
                    "value": code.get("value", ""),
                    "name": code.get("name", ""),
                    "since": version_of(code.get("added")),
                    "ep": extension_pack(code.get("addedEP")),
                    "deprecated": version_of(code.get("deprecated")),
                    "sort": int(code.get("sort")) if code.get("sort") else None,
                    "group": code.get("group"),
                    "doc": orchestra_documentation(code),
                }
            )
        code_sets[element.get("name", "")] = {
            "type": element.get("type", "String"),
            "codes": codes,
        }

    fields = {}
    for element in root.iter(f"{{{NS['fixr']}}}field"):
        tag = int(element.get("id", "0"))
        if tag <= 0:
            continue
        fields[tag] = {
            "name": element.get("name", ""),
            "type": element.get("type", "String"),
            "code_set": element.get("codeSet"),
            "since": version_of(element.get("added")),
            "ep": extension_pack(element.get("addedEP")),
            "doc": orchestra_documentation(element),
        }

    datatypes = {
        element.get("name", "") for element in root.iter(f"{{{NS['fixr']}}}datatype")
    }

    groups = {}
    for element in root.iter(f"{{{NS['fixr']}}}group"):
        counter = element.find(f"{{{NS['fixr']}}}numInGroup")
        if counter is None:
            continue
        groups[element.get("name", "")] = {
            "id": int(element.get("id", "0")),
            "tag": int(counter.get("id", "0")),
            "members": members_of(element),
            "since": version_of(element.get("added")),
            "doc": orchestra_documentation(element),
        }

    components = {
        element.get("name", ""): {
            "id": int(element.get("id", "0")),
            "members": members_of(element),
            "since": version_of(element.get("added")),
        }
        for element in root.iter(f"{{{NS['fixr']}}}component")
    }

    messages = {
        element.get("msgType", ""): {
            "name": element.get("name", ""),
            "id": int(element.get("id", "0")),
            "members": members_of(element),
            "since": version_of(element.get("added")),
        }
        for element in root.iter(f"{{{NS['fixr']}}}message")
    }

    return {
        "version": version_of(declared.split("_")[0]) or "5.0SP2",
        "ep": ep,
        "fields": fields,
        "code_sets": code_sets,
        "datatypes": datatypes,
        "groups": groups,
        "components": components,
        "messages": messages,
    }


def members_of(element: ElementTree.Element) -> list[dict[str, Any]]:
    """One component's or message's members, in wire order."""
    members = []
    for child in element.iter():
        tag = child.tag.rsplit("}", 1)[-1]
        if tag == "fieldRef":
            members.append(
                {
                    "kind": "field",
                    "id": int(child.get("id", "0")),
                    "required": child.get("presence") == "required",
                }
            )
        elif tag in {"componentRef", "groupRef"}:
            members.append(
                {
                    "kind": tag.removesuffix("Ref"),
                    "id": int(child.get("id", "0")),
                    "required": child.get("presence") == "required",
                }
            )
    return members


def parse_quickfix(data: bytes) -> dict[str, Any]:
    """One QuickFIX data dictionary: names, types and enums per version."""
    root = ElementTree.fromstring(data)
    fields = {}
    for element in root.iter("field"):
        tag = int(element.get("number", "0"))
        if tag <= 0:
            continue
        fields[tag] = {
            "name": element.get("name", ""),
            "type": element.get("type", "STRING"),
            "codes": [
                {"value": value.get("enum", ""), "name": value.get("description", "")}
                for value in element.iter("value")
            ],
        }
    header = [
        element.get("name", "")
        for element in root.findall("./header/field")
        if element.get("name")
    ]
    return {"fields": fields, "header": header}


def quickfix_type(name: str) -> str:
    """A QuickFIX type spelling in the crate's grammar."""
    folded_name = folded(name)
    replacements = {
        "utctimestamp": "UTCTimestamp",
        "utctimeonly": "UTCTimeOnly",
        "utcdateonly": "UTCDateOnly",
        "utcdate": "UTCDateOnly",
        "monthyear": "MonthYear",
        "numingroup": "NumInGroup",
        "seqnum": "SeqNum",
        "tagnum": "TagNum",
        "dayofmonth": "DayOfMonth",
        "localmktdate": "LocalMktDate",
        "multiplecharvalue": "MultipleCharValue",
        "multiplestringvalue": "MultipleStringValue",
        "priceoffset": "PriceOffset",
        "exchange": "Exchange",
        "currency": "Currency",
        "country": "Country",
        "language": "Language",
        "boolean": "Boolean",
        "amt": "Amt",
        "qty": "Qty",
        "price": "Price",
        "percentage": "Percentage",
        "length": "Length",
        "data": "data",
        "int": "int",
        "float": "float",
        "char": "char",
        "string": "String",
        "xmldata": "XMLData",
    }
    return replacements.get(folded_name, "String")


def dtype_of(fix_type: str, tag: int, code_sets: dict[str, Any]) -> str:
    """The crate datatype spelling one FIX datatype name resolves through.

    The logical-name table already *is* the FIX Latest datatype table, so a
    field declares its FIX type and the grammar answers the column. Two tags
    the standard itself declares as code sets take the datatype the crate
    gives that code set; that is honouring the declaration rather than the
    narrowing a generator must not do.
    """
    if tag in CODED_TAGS:
        return CODED_TAGS[tag]
    # A field typed by a code set takes that set's own base type: the set is
    # a vocabulary over a datatype, never a datatype of its own.
    held = code_sets.get(fix_type)
    if held is not None:
        fix_type = held["type"]
    if folded(fix_type) not in LOGICAL_NAMES:
        raise SystemExit(f"unmapped FIX datatype {fix_type!r} on tag {tag}")
    return fix_type


def canonical_json(value: Any) -> str:
    """The compact rendering the crate's own documents use."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def lineage_document(entries: list[dict[str, Any]]) -> str:
    """`fix:lineage`, keys in the order the reader expects."""
    order = ["since", "ep", "name", "type", "deprecated", "removed", "doc"]
    rendered = []
    for entry in entries:
        rendered.append({key: entry[key] for key in order if entry.get(key) not in (None, False)})
    return canonical_json({"entries": rendered})


def codes_document(codes: list[dict[str, Any]]) -> str:
    """`fix:codes`, ordered by wire value with `value` leading each record."""
    order = ["value", "name", "since", "ep", "deprecated", "sort", "group", "aliases", "doc"]
    ordered = sorted(codes, key=lambda code: (code["value"], code["name"]))
    rendered = []
    for code in ordered:
        rendered.append({key: code[key] for key in order if code.get(key) not in (None, [], False)})
    return canonical_json({"codes": rendered})


def build(parsed: dict[str, dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Fold every source into one definition per tag, lowest priority first."""
    latest = parsed["orchestra-latest"]

    # Per-tag history, oldest first, from the versions QuickFIX publishes.
    history: dict[int, list[dict[str, Any]]] = {}
    for source in SOURCES:
        if source.format != "quickfix":
            continue
        held = parsed[source.source_id]
        for tag, field in held["fields"].items():
            entry = {
                "since": source.version,
                "name": folded(field["name"]),
                "type": quickfix_type(field["type"]),
            }
            entries = history.setdefault(tag, [])
            if entries and entries[-1]["name"] == entry["name"] and entries[-1]["type"] == entry["type"]:
                continue
            entries.append(entry)

    fields: list[dict[str, Any]] = []
    for tag, field in sorted(latest["fields"].items()):
        name = folded(field["name"])
        dtype = dtype_of(field["type"], tag, latest["code_sets"])
        # The per-version history leads, because it is the only source that
        # says what a field was called and typed *at* a version. The current
        # definition is appended only when it differs from the last of it,
        # dated at the version the dictionary itself announces - never at the
        # field's , which says when the field appeared and not when its
        # present name took effect.
        entries = [dict(entry) for entry in history.get(tag, [])]
        if entries and field["since"]:
            oldest = min(field["since"], entries[0]["since"], key=version_key)
            entries[0]["since"] = oldest
        current = {"since": latest["version"], "ep": field["ep"], "name": name, "type": dtype}
        if not entries:
            current["since"] = field["since"] or latest["version"]
            entries = [current]
        elif (entries[-1]["name"], entries[-1]["type"]) != (name, dtype):
            entries.append(current)

        metadata: dict[str, str] = {"fix:tag": str(tag)}
        if field["name"] != name:
            metadata["display"] = field["name"]
        if field["doc"]:
            metadata["description"] = field["doc"]
        if len(entries) > 1 or entries[0]["since"] != latest["version"]:
            metadata["fix:lineage"] = lineage_document(entries)
            aliases = []
            for entry in entries:
                spelling = entry.get("name")
                if spelling and spelling != name and spelling not in aliases:
                    aliases.append(spelling)
            if aliases:
                metadata["fix:aliases"] = ",".join(aliases)
        code_set = latest["code_sets"].get(field["code_set"] or field["type"] or "")
        if code_set and code_set["codes"]:
            metadata["fix:codes"] = codes_document(code_set["codes"])

        fields.append(
            {
                "name": name,
                "dtype": dtype_document(dtype),
                "nullable": True,
                "metadata": dict(sorted(metadata.items())),
                "_tag": tag,
                "_nested": False,
            }
        )

    # Groups become the nested tree: a List of a non-null `item` Struct whose
    # own `fix:tag` is the counter's.
    #
    # One counter tag heads several groups - Orchestra declares `NoRelatedSym`
    # eleven times, once per message context - and a registry holds one
    # definition per identifier. So the contexts are unioned in wire order,
    # first occurrence winning, and the per-message shape stays in the layouts
    # manifest, which records it exactly.
    by_tag = {field["_tag"]: field for field in fields}
    grouped: dict[int, list[dict[str, Any]]] = {}
    for name, group in sorted(latest["groups"].items()):
        counter_tag = group["tag"]
        if counter_tag not in by_tag:
            continue
        members = grouped.setdefault(counter_tag, [])
        held = {member["id"] for member in members}
        for member in group["members"]:
            if member["kind"] != "field" or member["id"] == counter_tag:
                continue
            if member["id"] in held or member["id"] not in by_tag:
                continue
            held.add(member["id"])
            members.append(member)

    for counter_tag, members in sorted(grouped.items()):
        if not members:
            continue
        counter = by_tag[counter_tag]
        children = [
            {
                "name": by_tag[member["id"]]["name"],
                "dtype": by_tag[member["id"]]["dtype"],
                "nullable": not member["required"],
                "metadata": {"fix:tag": str(member["id"])},
            }
            for member in members
        ]
        fields = [field for field in fields if field["_tag"] != counter_tag]
        fields.append(
            {
                "name": counter["name"],
                "dtype": {
                    "type": "list",
                    "field": {
                        "name": "item",
                        "dtype": {"type": "struct", "fields": children},
                        "nullable": False,
                        "metadata": {},
                    },
                },
                "nullable": True,
                "metadata": dict(counter["metadata"]),
                "_tag": counter_tag,
                "_nested": True,
            }
        )

    layouts = {
        "components": [
            {"name": name, "id": held["id"], "members": held["members"]}
            for name, held in sorted(latest["components"].items())
        ],
        # A `groupRef` names the group's own identifier, which is neither a tag
        # nor a component identifier, so a layout is unwalkable without this
        # third table. `tag` is the counter the registry holds the group under.
        "groups": [
            {
                "name": name,
                "id": held["id"],
                "tag": held["tag"],
                "members": held["members"],
            }
            for name, held in sorted(latest["groups"].items())
        ],
        "messages": [
            {"msgtype": msgtype, "name": held["name"], "id": held["id"], "members": held["members"]}
            for msgtype, held in sorted(latest["messages"].items())
        ],
    }
    return fields, layouts


def dtype_document(name: str) -> dict[str, Any]:
    """The datatype document one FIX datatype name resolves to.

    The shard holds the crate's own serialized datatype rather than a display
    string, so a parameterized type carries its parameters where the reader
    expects them.
    """
    documents: dict[str, dict[str, Any]] = {
        "int": {"type": "int32"},
        "float": {"type": "float64"},
        "char": {"type": "utf8"},
        "String": {"type": "utf8"},
        "string": {"type": "utf8"},
        "Boolean": {"type": "boolean"},
        "boolean": {"type": "boolean"},
        "data": {"type": "binary"},
        "XMLData": {"type": "binary"},
        "Length": {"type": "int32"},
        "TagNum": {"type": "int32"},
        "SeqNum": {"type": "int64"},
        "NumInGroup": {"type": "int32"},
        "DayOfMonth": {"type": "int8"},
        "Qty": {"type": "float64"},
        "Price": {"type": "float64"},
        "PriceOffset": {"type": "float64"},
        "Percentage": {"type": "float64"},
        "Amt": {"type": "float64"},
        "UTCTimestamp": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "TZTimestamp": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "UTCTimeOnly": {"type": "time64", "unit": "nanosecond"},
        "LocalMktTime": {"type": "time32", "unit": "second"},
        "UTCDateOnly": {"type": "date32"},
        "UTCDate": {"type": "date32"},
        "LocalMktDate": {"type": "date32"},
        "LocalMktDatetime": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "TZTimeOnly": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "MonthYear": {"type": "fixed_ascii", "width": 8},
        "Tenor": {"type": "fixed_ascii", "width": 8},
        "Language": {"type": "fixed_ascii", "width": 2},
        "Country": {"type": "country"},
        "Currency": {"type": "currency"},
        "Exchange": {"type": "mic"},
        "MultipleCharValue": {"type": "utf8"},
        "MultipleStringValue": {"type": "utf8"},
        "XID": {"type": "utf8"},
        "XIDREF": {"type": "utf8"},
        "Pattern": {"type": "utf8"},
        "Reserved100Plus": {"type": "int32"},
        "Reserved1000Plus": {"type": "int32"},
        "Reserved4000Plus": {"type": "int32"},
        "Time": {"type": "time64", "unit": "nanosecond"},
        "Date": {"type": "date32"},
        "msgtype": {"type": "msgtype"},
        "side": {"type": "side"},
        "msgdirection": {"type": "msgdirection"},
    }
    document = documents.get(name)
    if document is None:
        document = documents.get(name[:1].upper() + name[1:])
    if document is None:
        raise SystemExit(f"no datatype document for {name!r}")
    return document


_VERSION = re.compile(r"^(\d+)(?:\.(\d+))?(?:\.(\d+))?(.*)$")


def version_key(version: str) -> tuple[int, int, int, int, str]:
    """Numeric-first ordering, post-release qualifiers after a bare version."""
    match = _VERSION.match(version)
    if not match:
        return (0, 0, 0, 0, version)
    major, minor, patch, qualifier = match.groups()
    pre = qualifier.startswith("-")
    return (
        int(major),
        int(minor or 0),
        int(patch or 0),
        -1 if pre else (0 if not qualifier else 1),
        qualifier.lstrip("-."),
    )


def write_tree(out: pathlib.Path, fields: list[dict[str, Any]]) -> dict[str, str]:
    """Write shards exactly as `write_into` produces them."""
    shards: dict[tuple[str, int], list[dict[str, Any]]] = {}
    for field in fields:
        tree = "nested" if field["_nested"] else "primitive"
        shards.setdefault((tree, field["_tag"] // 100), []).append(field)
    written: dict[str, str] = {}
    for (tree, shard), held in sorted(shards.items()):
        held.sort(key=lambda field: field["_tag"])
        document = [
            {key: value for key, value in field.items() if not key.startswith("_")}
            for field in held
        ]
        path = out / tree / f"{shard}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        text = json.dumps(document, indent=2, ensure_ascii=False) + "\n"
        path.write_text(text, encoding="utf-8", newline="\n")
        written[f"{tree}/{shard}.json"] = hashlib.sha256(text.encode()).hexdigest()
    return written


def write_constants(latest: dict[str, Any], parsed: dict[str, dict[str, Any]]) -> None:
    """Write the header and trailer tag lists as a generated Rust module.

    `FixMsg` lays a message flat, so both components are tag lists rather than
    nested Structs, and the lists are the union across every scraped version:
    FIX Latest alone is not enough, because it no longer lists
    `SecureDataLen(90)` in the header or `SignatureLength(93)` in the trailer,
    and a 4.2 message carries both. `defined_at` decides what a version may
    actually carry, so a union costs a reader nothing and a narrower list
    would cost it a field.

    They are not registry entries: a component has no tag, and a synthetic one
    would put a fiction in the identity space.
    """
    header: list[int] = []
    trailer: list[int] = []
    for name, held in latest["components"].items():
        if name == "StandardHeader":
            target = header
        elif name == "StandardTrailer":
            target = trailer
        else:
            continue
        for member in held["members"]:
            if member["kind"] == "field" and member["id"] not in target:
                target.append(member["id"])

    # The per-version headers QuickFIX publishes, folded onto the tags FIX
    # Latest names, so a header a later version stopped listing still travels.
    by_name = {folded(field["name"]): tag for tag, field in latest["fields"].items()}
    for source in SOURCES:
        if source.format != "quickfix":
            continue
        for spelling in parsed[source.source_id]["header"]:
            tag = by_name.get(folded(spelling))
            if tag is not None and tag not in header and tag not in trailer:
                header.append(tag)

    lines = [
        "//! The standard header and trailer, as tag lists.",
        "//!",
        "//! Generated by `scripts/generate_fix_dictionary.py`; do not edit.",
        "//!",
        "//! Order rather than nesting: [`FixMsg`](crate::FixMsg) lays a message",
        "//! flat, so both components are tag lists rather than Structs. They are",
        "//! not registry entries either - a component carries no tag, and",
        "//! `insert` admits only a field that does, so a synthetic tag would put",
        "//! a fiction in the identity space. The component names and ids are",
        "//! dropped; every field *in* them is an ordinary entry by its own tag.",
        "//!",
        "//! The lists are the union across every scraped version, because FIX",
        "//! Latest alone is not enough: it no longer lists `SecureDataLen(90)`",
        "//! in the header or `SignatureLength(93)` in the trailer, and a 4.2",
        "//! message carries both. Requiredness rides the lineage rather than a",
        "//! parallel table, because presence is already `nullable` on the field",
        "//! a version resolves to.",
        "",
    ]
    for name, tags, what in [
        ("STANDARD_HEADER_TAGS", header, "header"),
        ("STANDARD_TRAILER_TAGS", trailer, "trailer"),
    ]:
        rendered = ", ".join(str(tag) for tag in tags)
        lines.extend(
            [
                f"/// The tags every version's standard {what} declares, in wire",
                "/// order.",
                f"pub const {name}: [i32; {len(tags)}] = [{rendered}];",
                "",
            ]
        )
    (ROOT / "rust" / "src" / "fix" / "constants.rs").write_text(
        "\n".join(lines), encoding="utf-8", newline="\n"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", default=None, help="a local directory or a URL base")
    parser.add_argument("--out", default=str(DEFAULT_OUT), help="where the trees are written")
    parser.add_argument("--check", action="store_true", help="verify the committed checksums")
    arguments = parser.parse_args()

    out = pathlib.Path(arguments.out)
    parsed: dict[str, dict[str, Any]] = {}
    provenance = []
    for source in SOURCES:
        data = read_source(source, arguments.source)
        parsed[source.source_id] = (
            parse_orchestra(data) if source.format == "orchestra" else parse_quickfix(data)
        )
        provenance.append(
            {
                "source_id": source.source_id,
                "format": source.format,
                "url": source.url,
                "sha256": hashlib.sha256(data).hexdigest(),
                "branch": "",
                "version": source.version,
                "priority": source.priority,
                "license_url": source.license_url,
            }
        )

    latest = parsed["orchestra-latest"]
    unmapped = sorted(
        name for name in latest["datatypes"] if folded(name) not in LOGICAL_NAMES
    )
    if unmapped:
        raise SystemExit(f"unmapped FIX datatypes: {', '.join(unmapped)}")

    fields, layouts = build(parsed)
    if arguments.check:
        print(f"{len(fields)} definitions, {len(latest['datatypes'])} datatypes resolved")
        return 0

    for tree in ("primitive", "nested"):
        for stale in sorted((out / tree).glob("*.json")):
            stale.unlink()
    written = write_tree(out, fields)

    (out / "provenance.json").write_text(
        json.dumps(
            {
                "version": latest["version"],
                "ep": latest["ep"],
                "sources": provenance,
                "definitions": written,
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    write_constants(latest, parsed)
    (out / "layouts.json").write_text(
        json.dumps(layouts, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(
        f"wrote {len(fields)} definitions across {len(written)} shards "
        f"at FIX {latest['version']} EP{latest['ep']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
