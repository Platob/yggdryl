"""Generate the committed FIX dictionary from the published sources.

The crate never fetches anything: it reads the committed output through
``FixRegistry::from_handle``. This script is what writes that output, and it
is the only thing that knows where the specification lives.

Every source URL is pinned to a commit rather than a branch, because a
dictionary regenerated from ``master`` is not reproducible and its diff is
unreviewable. The provenance manifest records both checksums - the bytes read
and the definitions produced - so CI can test for drift, and its second half
needs no network at all.

Orchestra's fields, components, groups and messages each have their own
directory of native Field documents. Only wire fields have tags. A group
references its ordinary int32 counter and contains a non-null component.
Each field stores its enum records directly in fix:codes metadata. Datatypes
resolve through the crate's logical-name table.

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
                ("FIX50SP1", "5.0.1"),
                ("FIX50SP2", "5.0.2"),
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
    "mic", "cfi", "side", "utcdate",
    # Orchestra names these beside the ones the table resolves.
    "localmktdatetime", "time", "date",
}

# The tags the standard declares as code sets and the crate types with a
# datatype of their own. Honouring the declaration, not guessing at one: the
# code set stays the field's vocabulary, and the datatype is how a value of it
# is stored. Tag 385 is here because a direction is what every capture in this
# crate carries on its own lines, so it is read far more often than it arrives.
# Tags 39 and 150 are the order's state: their two code sets agree on every
# value they share, and the crate's own `state` type reads either.
CODED_TAGS = {39: "state", 54: "side", 150: "state", 385: "msgdirection"}


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
    service_pack = re.fullmatch(r"(\d+\.\d+)[sS][pP](\d+)", text)
    if service_pack:
        text = f"{service_pack[1]}.{service_pack[2]}"
    version_key(text)
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


def parse_orchestra(data: bytes, protocol_version: str | None = None) -> dict[str, Any]:
    """Every kind one Orchestra file publishes."""
    root = ElementTree.fromstring(data)
    declared = root.get("version", "")
    ep = None
    if "_EP" in declared:
        ep = int(declared.rsplit("_EP", 1)[1])
    elif declared.startswith("EP"):
        ep = int(declared[2:])

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
            "doc": orchestra_documentation(element),
        }
        for element in root.iter(f"{{{NS['fixr']}}}component")
    }

    messages = {
        element.get("msgType", ""): {
            "name": element.get("name", ""),
            "id": int(element.get("id", "0")),
            "members": members_of(element),
            "since": version_of(element.get("added")),
            "doc": orchestra_documentation(element),
        }
        for element in root.iter(f"{{{NS['fixr']}}}message")
    }

    return {
        "version": protocol_version or version_of(declared.split("_")[0]) or "5.0.2",
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
    field declares its FIX type and the grammar answers the column. Registered
    coded types keep their crate datatype; message codes remain plain text.
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


# The datatype tags that hold an instant, a date or a time of day, and the
# ones that hold text. Mirrors `DataTypeId::is_temporal` and
# `DataTypeId::is_string` over the tags `dtype_document` can write; the
# cross-host test asserts the two hosts back-type identically.
TEMPORAL_TAGS = {"date32", "date64", "time32", "time64", "datetime64"}
TEXT_TAGS = {"utf8", "large_utf8", "utf8_view", "ascii", "fixed_ascii"}


def back_type(entries: list[dict[str, Any]]) -> None:
    """Adopt a temporal type backward over the string entries preceding it.

    The Python half of `back_type` in `rust/src/fix/lineage.rs`, with that
    function's reasoning: a field FIX transmitted as text and later declared
    temporal was always carrying an instant, so stating the later type at the
    earlier version is what the crate can consistently parse. Only a string
    yields, and only to a temporal - every other pair is a real constraint the
    later version added.
    """
    for at in range(len(entries) - 1, 0, -1):
        later = entries[at].get("type")
        if not isinstance(later, dict) or later.get("type") not in TEMPORAL_TAGS:
            continue
        earlier = at
        while earlier > 0:
            held = entries[earlier - 1].get("type")
            if not isinstance(held, dict) or held.get("type") not in TEXT_TAGS:
                break
            earlier -= 1
            entries[earlier]["type"] = later


def lineage_document(entries: list[dict[str, Any]]) -> str:
    """`fix:lineage`, keys in the order the reader expects.

    This is the Python half of `FixLineage::render` and makes the same two
    derivations, so a generated dictionary and a hand-built or merged one hold
    one document:

    - the datatype is stored resolved, as the crate's serialized type and
      exactly as the field's own ``dtype`` is stored, so a parameterized type
      carries its parameters and ``char`` and ``String`` are one ``utf8``;
    - an entry stating nothing its predecessor did not is dropped, because a
      dated point repeating what was already true is not a point in a history.

    The oldest entry is never dropped - it is what ``since`` reads - and ``ep``
    is the entry's date rather than one of its statements, so an extension
    pack that changed nothing is exactly the entry worth dropping. Whether a
    field has a lineage at all is decided by the caller, before this: the
    specification dating a field twice is what gives it a history, and
    normalizing how that history is written must not take it away.
    """
    order = ["since", "ep", "name", "type", "deprecated", "removed", "doc"]
    stated = ["name", "deprecated", "removed", "doc"]
    resolved: list[dict[str, Any]] = []
    for entry in entries:
        held = dict(entry)
        if held.get("type") is not None:
            held["type"] = dtype_document(held["type"])
        resolved.append(held)
    back_type(resolved)

    rendered: list[dict[str, Any]] = []
    for held in resolved:
        if rendered:
            kept = rendered[-1]
            if held.get("type") == kept.get("type") and all(
                held.get(key) == kept.get(key) for key in stated
            ):
                continue
        rendered.append({key: held[key] for key in order if held.get(key) not in (None, False)})
    return canonical_json({"entries": rendered})


def codes_document(codes: list[dict[str, Any]]) -> str:
    """`fix:codes`, ordered by wire value with `value` leading each record."""
    order = ["value", "name", "since", "ep", "deprecated", "sort", "group", "aliases", "doc"]
    ordered = sorted(codes, key=lambda code: (code["value"], code["name"]))
    rendered = []
    for code in ordered:
        rendered.append({key: code[key] for key in order if code.get(key) not in (None, [], False)})
    return canonical_json({"codes": rendered})


# Longest first so a longer suffix is never shadowed.
LATIN = (
    ("appendices", "appendix"),
    ("matrices", "matrix"),
    ("vertices", "vertex"),
    ("indices", "index"),
)


def _singularize(stem: str) -> str:
    """The stripped stem as one occurrence, by the first arm that matches."""
    # Arm 1. The only arm that rewrites inside the stem, so the only one that
    # can lose case: the replacement takes the case of the byte it replaces.
    for plural, singular in LATIN:
        if len(stem) >= len(plural) and stem[-len(plural) :].lower() == plural:
            head, tail = stem[: -len(plural)], stem[-len(plural) :]
            replaced = singular[0].upper() + singular[1:] if tail[0].isupper() else singular
            return head + replaced
    # Arm 2. Byte-exact: an uppercase `S` is not a plural marker here.
    if not stem.endswith("s"):
        return stem
    # Arm 3.
    if stem.endswith("ss"):
        return stem
    # Arm 4. The length guard keeps `Ties` from becoming `Ty`.
    if len(stem) > 4 and stem.endswith("ies"):
        return stem[:-3] + "y"
    # Arm 5.
    if stem.endswith("sses"):
        return stem[:-2]
    # Arm 6.
    if stem.endswith("es"):
        prefix = stem[:-2]
        if prefix.endswith(("x", "ch", "sh", "zz")):
            return prefix
    # Arm 7.
    return stem[:-1]


def entry_name(group_name: str) -> str:
    """An entry follows its published collection, including numeric qualifiers."""
    stem = group_name.removesuffix("Grp")
    matched = re.fullmatch(r"(.*?)(\d*)", stem)
    assert matched is not None
    return _singularize(matched[1]) + matched[2]


def build(parsed: dict[str, dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    """Resolve the source graph once into the registry's five categories."""
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
            if (entries[-1]["since"], entries[-1].get("ep")) == (current["since"], current["ep"]):
                # One dated point states one thing. Where the scraped snapshot
                # of a version and the Latest reading of that same version
                # disagree - tag 327 is `HaltReasonInt` in the QuickFIX
                # FIX.5.0SP2 file and `HaltReason` in Orchestra Latest - the
                # higher-priority source replaces the lower rather than
                # standing beside it, which is the same rule a lineage merge
                # follows and the one `FixLineage::render` enforces.
                entries[-1] = current
            else:
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
        code_set_name = field["code_set"] or field["type"] or ""
        if field["code_set"] and code_set_name not in latest["code_sets"]:
            raise ValueError(f"{field['name']}: unresolved code set {code_set_name}")
        if code_set_name in latest["code_sets"]:
            metadata["fix:codes"] = codes_document(latest["code_sets"][code_set_name]["codes"])

        fields.append(
            {
                "name": name,
                "dtype": dtype_document(dtype),
                "nullable": True,
                "metadata": dict(sorted(metadata.items())),
            }
        )
    return build_catalog(latest, fields)


def build_catalog(
    latest: dict[str, Any], fields: list[dict[str, Any]]
) -> dict[str, list[dict[str, Any]]]:
    """Keep wire identity separate from the source's reusable named graph.

    The published collection names are authoritative. Entry names are local
    schema names: Parties becomes Party, NestedParties2 becomes NestedParty2.
    A category suffix resolves collisions with existing protocol vocabulary;
    the source ID disambiguates a remaining generated-name collision.

    References are native null Fields with one typed FIX metadata reference.
    The core resolves them at intake; a tree never duplicates its owners here.
    """
    result = {kind: [] for kind in ("fields", "components", "groups", "messages")}
    result["fields"] = fields
    by_tag = {int(field["metadata"]["fix:tag"]): field for field in fields}
    used = {field["name"] for field in fields}
    if len(used) != len(fields):
        raise ValueError("duplicate folded FIX field name")
    used.update(
        folded(alias)
        for field in fields
        for alias in field.get("metadata", {}).get("fix:aliases", "").split(",")
        if alias
    )
    source_names: dict[tuple[str, int], str] = {}
    definitions: dict[tuple[str, int], dict[str, Any]] = {}
    for category in ("components", "groups", "messages"):
        for source_name, definition in latest[category].items():
            key = (category, definition["id"])
            if key in definitions:
                raise ValueError(f"duplicate {category} id {key[1]}")
            definitions[key] = definition
            source_names[key] = definition.get("name", source_name)

    reserved = {folded(name) for name in source_names.values()} | used
    names: dict[tuple[str, int], str] = {}

    def claim(display: str, suffix: str, identifier: int, original: bool) -> str:
        canonical = folded(display)
        if canonical in used or (not original and canonical in reserved):
            canonical = folded(display + suffix)
            if canonical in used or canonical in reserved:
                canonical += str(identifier)
        if canonical in used or (not original and canonical in reserved):
            raise ValueError(f"unresolvable FIX name collision: {display} ({identifier})")
        if re.fullmatch(r"[a-z0-9][a-z0-9_.-]*", canonical) is None:
            raise ValueError(f"invalid FIX catalog name: {display!r}")
        used.add(canonical)
        return canonical

    suffixes = {"components": "Component", "groups": "Grp", "messages": "Message"}
    for key, display in sorted(source_names.items()):
        names[key] = claim(display, suffixes[key[0]], key[1], original=True)

    entries: dict[int, str] = {}
    entry_displays: dict[int, str] = {}
    for (category, identifier), display in sorted(source_names.items()):
        if category == "groups":
            entry_displays[identifier] = entry_name(display)
            entries[identifier] = claim(entry_displays[identifier], "Component", identifier, original=False)

    def reference(name: str, kind: str, required: bool) -> dict[str, Any]:
        return {
            "name": name,
            "dtype": {"type": "null"},
            "nullable": not required,
            "metadata": {f"fix:{kind}": name},
        }

    def members(owner: tuple[str, int]) -> list[dict[str, Any]]:
        children = []
        child_names = set()
        for member in definitions[owner]["members"]:
            kind, identifier = member["kind"], member["id"]
            required = member["required"]
            if kind == "field":
                if identifier not in by_tag:
                    raise ValueError(f"{source_names[owner]}: unresolved field {identifier}")
                children.append(reference(by_tag[identifier]["name"], "field", required))
            else:
                target = (f"{kind}s", identifier)
                if target not in definitions:
                    raise ValueError(f"{source_names[owner]}: unresolved {kind} {identifier}")
                if kind == "group":
                    counter = definitions[target]["tag"]
                    if counter not in by_tag or by_tag[counter]["dtype"] != {"type": "int32"}:
                        raise ValueError(f"{source_names[target]}: counter {counter} must be an int32 field")
                    children.append(reference(by_tag[counter]["name"], "field", required))
                children.append(reference(names[target], kind, required))
        for child in children:
            if child["name"] in child_names:
                raise ValueError(f"{source_names[owner]}: duplicate member {child['name']}")
            child_names.add(child["name"])
        return children

    # References keep storage compact but must still describe one finite graph.
    visiting: set[tuple[str, int]] = set()
    resolved: set[tuple[str, int]] = set()

    def validate_graph(key: tuple[str, int], depth: int = 0) -> None:
        if key in visiting or depth > 64:
            raise ValueError(f"cyclic or excessive FIX nesting at {source_names[key]}")
        if key in resolved:
            return
        visiting.add(key)
        for member in definitions[key]["members"]:
            if member["kind"] != "field":
                target = (f"{member['kind']}s", member["id"])
                if target not in definitions:
                    raise ValueError(f"{source_names[key]}: unresolved {target}")
                validate_graph(target, depth + 1)
        visiting.remove(key)
        resolved.add(key)

    for key, definition in sorted(definitions.items()):
        validate_graph(key)
        category, identifier = key
        metadata = {"display": source_names[key]}
        if definition.get("doc"):
            metadata["description"] = definition["doc"]
        children = members(key)
        if category == "groups":
            counter = definition["tag"]
            if counter not in by_tag or by_tag[counter]["dtype"] != {"type": "int32"}:
                raise ValueError(f"{source_names[key]}: counter {counter} must be an int32 field")
            entry = {
                "name": entries[identifier],
                "dtype": {"type": "struct", "fields": children},
                "nullable": False,
                "metadata": {"display": entry_displays[identifier]},
            }
            result["components"].append(entry)
            dtype = {"type": "list", "field": reference(entries[identifier], "component", True)}
            metadata["fix:counter"] = str(counter)
            metadata["fix:component"] = entries[identifier]
        else:
            dtype = {"type": "struct", "fields": children}
        if category == "messages":
            metadata["fix:msgtype"] = next(
                wire for wire, held in latest["messages"].items() if held["id"] == identifier
            )
        result[category].append(
            {"name": names[key], "dtype": dtype, "nullable": False, "metadata": dict(sorted(metadata.items()))}
        )

    for category in result:
        result[category].sort(key=lambda field: int(field["metadata"]["fix:tag"]) if category == "fields" else field["name"])
    return result


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
        "LocalMktTime": {"type": "time64", "unit": "nanosecond"},
        "UTCDateOnly": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "UTCDate": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        # A naive zone is the absence of one, which the crate serializes by
        # omitting the key; writing it would be a second spelling of one type.
        "LocalMktDate": {"type": "datetime64", "unit": "nanosecond"},
        "LocalMktDatetime": {"type": "datetime64", "unit": "nanosecond"},
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
        "Date": {"type": "datetime64", "unit": "nanosecond", "timezone": "UTC"},
        "side": {"type": "side"},
        "msgdirection": {"type": "msgdirection"},
        "state": {"type": "state"},
    }
    document = documents.get(name)
    if document is None:
        document = documents.get(name[:1].upper() + name[1:])
    if document is None:
        raise SystemExit(f"no datatype document for {name!r}")
    return document


def version_key(version: str) -> tuple[int, int, int]:
    """The core's major:u8, minor:u8, patch:u16 numeric ordering."""
    matched = re.fullmatch(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?", version)
    if matched is None:
        raise ValueError(f"invalid numeric FIX version {version!r}")
    major, minor, patch = (int(part or 0) for part in matched.groups())
    if major > 255 or minor > 255 or patch > 65535:
        raise ValueError(f"FIX version exceeds major:u8, minor:u8, patch:u16: {version!r}")
    return major, minor, patch


def render_tree(catalog: dict[str, list[dict[str, Any]]]) -> dict[str, str]:
    """Render native Field documents with compact references between owners."""
    shards: dict[int, list[dict[str, Any]]] = {}
    for field in catalog["fields"]:
        tag = int(field["metadata"]["fix:tag"])
        shards.setdefault(tag // 100, []).append(field)
    documents: dict[str, Any] = {
        f"fields/{shard}.json": held for shard, held in sorted(shards.items())
    }
    for category in ("components", "groups", "messages"):
        for field in catalog[category]:
            documents[f"{category}/{field['name']}.json"] = field
    return {
        name: json.dumps(document, indent=2, ensure_ascii=False) + "\n"
        for name, document in sorted(documents.items())
    }


def write_tree(out: pathlib.Path, documents: dict[str, str]) -> dict[str, str]:
    """Replace generated files only; every target stays under the output root."""
    out = out.resolve()
    for tree in ("fields", "components", "groups", "messages", "primitive", "nested"):
        for stale in sorted((out / tree).glob("*.json")):
            relative = stale.resolve().relative_to(out).as_posix()
            if relative not in documents:
                stale.unlink()
        if tree in {"primitive", "nested"}:
            try:
                (out / tree).rmdir()
            except FileNotFoundError:
                pass
    (out / "layouts.json").unlink(missing_ok=True)
    written = {}
    for name, text in documents.items():
        path = out / name
        path.resolve().relative_to(out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")
        written[name] = hashlib.sha256(text.encode()).hexdigest()
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
            parse_orchestra(data, source.version if source.version != "latest" else "5.0.2")
            if source.format == "orchestra" else parse_quickfix(data)
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

    catalog = build(parsed)
    documents = render_tree(catalog)
    written = {name: hashlib.sha256(text.encode()).hexdigest() for name, text in documents.items()}
    manifest = {
        "version": latest["version"],
        "ep": latest["ep"],
        "sources": provenance,
        "definitions": written,
    }
    if arguments.check:
        failures = []
        for name, expected in documents.items():
            try:
                actual = (out / name).read_text(encoding="utf-8")
            except FileNotFoundError:
                failures.append(f"missing {name}")
                continue
            if actual != expected:
                failures.append(f"changed {name}")
        expected_names = set(documents)
        for category in (*catalog, "primitive", "nested"):
            for path in (out / category).glob("*.json"):
                name = path.relative_to(out).as_posix()
                if name not in expected_names:
                    failures.append(f"unexpected {name}")
        if (out / "layouts.json").exists():
            failures.append("retired layouts.json exists")
        try:
            actual_manifest = json.loads((out / "provenance.json").read_text(encoding="utf-8"))
        except FileNotFoundError:
            actual_manifest = None
        if actual_manifest != manifest:
            failures.append("changed provenance.json")
        if failures:
            print("\n".join(failures), file=sys.stderr)
            return 1
        print(f"verified {len(documents)} documents; " + ", ".join(f"{len(held)} {kind}" for kind, held in catalog.items()))
        return 0
    write_tree(out, documents)

    (out / "provenance.json").write_text(
        json.dumps(
            manifest,
            indent=2,
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    write_constants(latest, parsed)
    print(
        f"wrote {len(written)} documents (" + ", ".join(f"{len(held)} {kind}" for kind, held in catalog.items()) + ") "
        f"at FIX {latest['version']} EP{latest['ep']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
