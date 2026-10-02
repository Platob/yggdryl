"""Generate the committed FIX dictionary from the published sources.

The crate never fetches anything: it reads the committed output through
``FixRegistry::from_handle``. This script is what writes that output, and it
is the only thing that knows where the specification lives.

Every source URL is pinned to a commit rather than a branch, because a
dictionary regenerated from ``master`` is not reproducible and its diff is
unreviewable. The provenance manifest records both checksums - the bytes read
and the definitions produced - so CI can test for drift, and its second half
needs no network at all.

Orchestra's fields, components and groups each have their own directory of
native Field documents; a message is a component carrying ``FIX:msgtype`` and
is written into ``components/`` beside the others. Only wire
fields have tags. A group references its ordinary int32 counter and contains
a non-null component. Datatypes resolve through the crate's logical-name table.

``codesets/`` is the fourth directory, and it holds vocabularies rather than
fields. A code set is named by the specification - ``SideCodeSet``,
``UnitOfMeasureCodeSet`` - and named from as many fields as draw on it, 165 of
them for one unit set; so its members are written once under
``codesets/<name>.json`` and a field's ``FIX:codeset`` states the name of the set
it reads by. Where Orchestra names no set, or names one whose members differ
between the fields claiming it, the set is named after the field that reads by
it. The dictionary holds each set once and a merge folds two statements of one
set together, so a code named, aliased or documented once is named for every
field that reads it.

The remaining ``FIX:`` properties that hold a document - ``FIX:directions``
and ``FIX:idmap`` - are written as the JSON arrays they are rather than as one
escaped line; the crate restates each as its canonical compact text when it
reads the store back.

The dictionary is one reading of the protocol rather than a history of it: a
field is written under the one name and datatype the newest source gives it,
and every spelling an earlier version used is written beside it in its
``FIX:names`` list, so an old name still reaches the field. What a *value*
was does travel - a code set holds every value an older version declared and
every older spelling of a surviving one, dated. What the specification retired
and what stands in for it, and what a message implies but did not carry, are
not the dictionary's to state: the crate holds both as native rules of its
own, so no field carries a rule.

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
from typing import Any, NamedTuple

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "config" / "fix"

# One classification for every current FIX message type. The source formats
# do not retain a business-area property, so coverage is held against their
# exact wire-code set below rather than inferred from a generated name. A
# type stating many orders, quotes or trades at once - a list, a mass order,
# a cross, a mass quote, a bid list, a match report - files under its batch
# category (`ORDB`, `QUOB`, `TRDB`), which a parse splits into one message
# of the single category per entry; `EXEB` files no standard type. An
# assignment report and a contrary intention report maintain positions, and
# a market definition is market structure, as the standard files them.
MSGCAT_BY_TYPE = {
    code: category
    for category, codes in (
        ("SESS", "0 1 2 3 4 5 A j n BC BD BE BF CB BW BX BY EL EM EN EO EP"),
        ("ORDR", "D F G H 9 AB AC"),
        ("ORDB", "E K L M N q r AF CA BZ DJ DK s t u DS DT"),
        ("QUOT", "6 7 R S Z a AG AH AI AJ CW"),
        ("QUOB", "i b k l m"),
        ("EXEC", "8 BN Q"),
        ("TRAD", "AD AE AQ AR DW DX"),
        ("TRDB", "DC DD"),
        ("BOOK", "V W X Y DO DP DR EQ"),
        ("SECU", "c d e f v w x y z AA BK BP BR CN CO EG ER"),
        ("MKST", "g h BI BJ BS ES BT BU BV"),
        ("ALLO", "J P AS AT BM DU DV"),
        ("POSN", "AL AM AN AO AP BL DL DM DN AW BO"),
        ("SETL", "T AV BQ EC ED EE EF"),
        ("COLL", "AX AY AZ BA BB BG DQ CH CI CJ"),
        ("PRTY", "CF CG CK CX CY DH DI CU CV CZ DA DB"),
        ("RISK", "CL CM CR CS CT DE DF DG"),
        ("PAYM", "DY DZ EA EB"),
        ("CONF", "AK AU BH"),
        ("REGI", "o p"),
        ("STRM", "CC CD CE"),
        ("ACCT", "CQ"),
        ("COMM", "B C"),
        ("CERT", "EH EI EJ EK"),
    )
    for code in codes.split()
}

# The category set itself - every name, its code and its description - is
# `MarketDataKind`'s in `rust/src/marketdatakind.rs`, which renders the
# `msgcatcodeset` document: this table only files each type under a name,
# and a name that enum does not know refuses when the dictionary loads.

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
# datatype outside this set is a hard failure rather than a silent `string`.
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
# is stored. Tags 39 and 150 are the order's state: their two code sets agree
# on every value they share, and the crate's own `state` type reads either.
# Tag 385, `MsgDirection`, is typed as any coded field is - text carrying its
# code set - and the registry reads it.
CODED_TAGS = {54: "side"}

# FIX Latest's order, quote, execution, trade and allocation identifier
# families. Suffixes admit side/leg/ref/orig/affected forms;
# neither a general ID nor an administrative ReportID is an identifier here.
# Meanings: https://fiximate.fixtrading.org/en/FIX.Latest/tag11.html,
# tag19.html, tag1903.html and tag467.html. Source revisions stay pinned above.
IDENTIFIER_FAMILIES = (
    "clordid", "origclordid", "secondaryclordid", "orderid",
    "secondaryorderid", "listid", "quoteid", "quotereqid", "quoterespid",
    "quoteentryid", "execid", "execrefid", "secondaryexecid", "tradeid",
    "secondarytradeid", "tradereportid", "tradereportrefid", "firmtradeid",
    "regulatorytradeid", "allocid", "secondaryallocid", "individualallocid",
)


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


def deprecated_version_of(spelling: str | None, latest_version: str) -> str | None:
    """The version a `deprecated` attribute names, as a version the crate orders.

    Two spellings mean something `version_of` cannot say: `FIXT.1.1` is the
    session layer that shipped with FIX 5.0, so a field deprecated there was
    deprecated at 5.0, and `FIX.Latest` is the document being read, whose own
    version is the date - the extension pack beside it is what makes the
    point finer than the version alone.
    """
    if spelling == "FIXT.1.1":
        return "5.0"
    if spelling in {"FIX.Latest", "Latest"}:
        return latest_version
    return version_of(spelling)


# The QuickFIX versions in publication order: the version after the last one
# naming a field or listing a code is where the dictionary dates its removal.
QUICKFIX_VERSIONS = tuple(source.version for source in SOURCES if source.format == "quickfix")


def version_after(version: str, latest_version: str) -> str:
    """The QuickFIX version following `version`, else the Latest document's own.

    A gap between two listings is not a removal, so callers hand this the last
    version that names the thing; when that is the newest QuickFIX file, the
    next dated point the dictionary has is Latest itself.
    """
    at = QUICKFIX_VERSIONS.index(version)
    if at + 1 < len(QUICKFIX_VERSIONS):
        return QUICKFIX_VERSIONS[at + 1]
    return latest_version


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
    version = protocol_version or version_of(declared.split("_")[0]) or "5.0.2"

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
                    "deprecated": deprecated_version_of(code.get("deprecated"), version),
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
            "deprecated": deprecated_version_of(element.get("deprecated"), version),
            "deprecated_ep": extension_pack(element.get("deprecatedEP")),
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
        "version": version,
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


# The datatype tags that hold an instant, a date or a time of day, and the
# one that holds text: every string is the `string` tag, whatever layout,
# charset or bound it declares. Mirrors `DataTypeId::is_temporal` and
# `DataTypeId::is_string` over the tags `dtype_document` can write; the
# cross-host test asserts the two hosts back-type identically.
def codes_document(codes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """`FIX:codeset`, in the rank the specification gives, `value` leading each record.

    Where a code sits in the list *is* its presentation rank, so the rank is
    the order rather than a key beside it. A code the source ranks keeps that
    rank; one it does not follows every ranked code, in wire-value order, so
    the document is still one text for one set however it was assembled.

    A code's dates are the source's own bookkeeping and are not written: the
    set states one reading of every value it declares and retires none of
    them, so `since`, `ep` and `deprecated` reach this function and stop here.
    """
    order = ["value", "name", "group", "aliases", "doc"]
    unranked = 1 << 30
    by_value = sorted(codes, key=lambda code: (code["value"], code["name"]))
    ranked = sorted(
        enumerate(by_value),
        key=lambda pair: (
            pair[1]["sort"] if pair[1].get("sort") is not None else unranked,
            pair[0],
        ),
    )
    rendered = []
    for _, code in ranked:
        rendered.append({key: code[key] for key in order if code.get(key) not in (None, [], False)})
    return rendered


def camel_case(description: str) -> str:
    """A QuickFIX enum description as a code name: `PARTIAL_FILL` is `PartialFill`."""
    return "".join(part[:1].upper() + part[1:].lower() for part in description.split("_") if part)


# The legacy names the crate's own `state` datatype reads, where the CamelCased
# QuickFIX description would spell them differently. `State::from_spelling`
# knows `PartiallyFilled` and `Filled`; it does not know `PartialFill`.
LEGACY_NAMES = {(150, "1"): "PartiallyFilled", (150, "2"): "Filled"}

# PartyIDSource is copied into every Party family member but reads one shared
# vocabulary. The bridge spelling belongs to that family alone; another
# Proprietary code keeps the standard spelling it declares.
PARTY_ID_SOURCE_ALIASES = {("D", "Proprietary"): ("proprietary/customcode",)}


def party_id_source_aliases(
    name: str, declared: str | None, codes: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Add the bridge spelling to the shared PartyIDSource vocabulary."""
    if not name.endswith("partyidsource") and folded(declared or "") != "partyidsourcecodeset":
        return codes
    for code in codes:
        aliases = PARTY_ID_SOURCE_ALIASES.get((code["value"], code["name"]))
        if aliases is None:
            continue
        current = code.setdefault("aliases", [])
        for alias in aliases:
            if alias not in current:
                current.append(alias)
    return codes

def fold_legacy_codes(
    tag: int,
    codes: list[dict[str, Any]],
    listings: list[tuple[str, list[dict[str, str]]]],
    latest_version: str,
) -> list[dict[str, Any]]:
    """One code set holding every value any version declared for the field.

    `codes` is what Latest states and wins on every value it keeps; `listings`
    are the QuickFIX enum listings, oldest version first. A value Latest kept
    whose older description folds to another spelling gains that spelling as
    an alias; a value Latest dropped becomes a code dated from the first
    version listing it to the version after the last. Names and aliases are
    one folded namespace across the set, because a spelling reaching two codes
    resolves to neither: a generated name a current one already claims takes
    the `Legacy` suffix, and a spelling already reachable is not added twice.
    """
    held = [dict(code) for code in codes]
    by_value = {code["value"]: code for code in held}
    # Latest's own names claim the namespace first, current codes before
    # deprecated ones: two codes whose names fold together - `EURIBOR` beside
    # the `Euribor` it replaced - cannot both be reached by one spelling, so
    # the deprecated one takes the suffix, as a legacy code would.
    spelled: set[str] = set()
    for code in sorted(held, key=lambda code: code.get("deprecated") is not None):
        if folded(code["name"]) in spelled and code.get("deprecated"):
            code["name"] += "Legacy"
        if folded(code["name"]) in spelled:
            raise ValueError(f"tag {tag}: two current codes are spelled {code['name']!r}")
        spelled.add(folded(code["name"]))
    spelled.update(folded(alias) for code in held for alias in code.get("aliases") or [])
    legacy: dict[str, dict[str, Any]] = {}
    for version, listed in listings:
        for entry in listed:
            value = entry["value"].strip()
            if not value:
                continue
            name = LEGACY_NAMES.get((tag, value)) or camel_case(entry["name"])
            current = by_value.get(value)
            if current is not None:
                if folded(name) not in spelled:
                    current.setdefault("aliases", []).append(name)
                    spelled.add(folded(name))
                continue
            dropped = legacy.get(value)
            if dropped is None:
                legacy[value] = {"value": value, "spellings": [name], "since": version, "last": version}
            else:
                dropped["last"] = version
                if name not in dropped["spellings"]:
                    dropped["spellings"].append(name)
    for value, dropped in legacy.items():
        # The newest spelling names the code, as it does for a surviving one.
        spellings = dropped["spellings"]
        name = spellings[-1]
        if folded(name) in spelled:
            name += "Legacy"
        if folded(name) in spelled:
            raise ValueError(f"tag {tag}: legacy code {value!r} has no free name ({name})")
        spelled.add(folded(name))
        aliases = []
        for spelling in spellings[:-1]:
            if folded(spelling) not in spelled:
                aliases.append(spelling)
                spelled.add(folded(spelling))
        held.append(
            {
                "value": value,
                "name": name,
                "since": dropped["since"],
                "deprecated": version_after(dropped["last"], latest_version),
                "aliases": aliases,
            }
        )
    return held


# ---- Identifier maps: which key of which map a field names a message by ----
#
# A message goes by the names its fields state - the order's own
# identifiers, its quote's, its execution's - and which field states which
# key is the crate's reading, not the specification's. Each entry is written
# onto its field as a ``FIX:idmap`` document; the crate's own bridge fields
# carry theirs in the crate dump. ``follow`` marks an identifier an
# operation that follows another carries forward, and ``role`` the
# PartyRole(452) of the Parties occurrence whose PartyID(448) states the key.
# A key is the folded word of its identifier type - lower-case letters and
# digits. A message's parties are its ``partyids`` and its regulatory trade
# identifiers are ``identifiers``, both read by the crate natively rather
# than stated on a field.


def idmap(map_name: str, key: str, *, follow: bool = False, role: str | None = None) -> dict[str, Any]:
    """One identifier-map entry, its keys in the order the crate reads them."""
    entry: dict[str, Any] = {"map": map_name, "key": key}
    if follow:
        entry["follow"] = True
    if role is not None:
        entry["role"] = role
    return entry


IDMAP_SOURCES: tuple[tuple[int, list[dict[str, Any]]], ...] = (
    (11, [idmap("identifiers", "clordid")]),
    (17, [idmap("identifiers", "execid")]),
    (37, [idmap("identifiers", "orderid", follow=True)]),
    (41, [idmap("identifiers", "origclordid")]),
    (117, [idmap("identifiers", "quoteid")]),
    (131, [idmap("identifiers", "quotereqid")]),
    (198, [idmap("identifiers", "secondaryorderid", follow=True)]),
    (262, [idmap("identifiers", "mdreqid")]),
    (526, [idmap("identifiers", "secondaryclordid")]),
    (527, [idmap("identifiers", "secondaryexecid")]),
    (793, [idmap("identifiers", "secondaryallocid")]),
    (880, [idmap("identifiers", "trdmatchid")]),
    (989, [idmap("identifiers", "secondaryindividualallocid")]),
    (1003, [idmap("identifiers", "tradeid")]),
    (1040, [idmap("identifiers", "secondarytradeid")]),
    (1042, [idmap("identifiers", "secondaryfirmtradeid")]),
    (1751, [idmap("identifiers", "secondaryquoteid")]),
)

# The parents of the identifier a field states are what the dictionary's
# own field names say of one another: a field named ``parent`` or ``orig``
# before another identifier field's name is listed among that field's
# ``FIX:parents``, nearest first - a ``parent`` type before an ``orig`` one.
# OrigClOrdID(41) makes ClOrdID(11)'s parents ``origclordid``, the client
# order identifier a cancel/replace replaced; ClOrdID takes no ``parent``
# field, its previous value being FIX's own OrigClOrdID. The crate reads the
# same rule off any dictionary it loads (``FixRegistry::parent_sources``),
# and a test holds the two to one answer over this dictionary.
PARENT_PREFIXES = ("parent", "orig")


def parent_of(name: str, names: set[str]) -> tuple[str, int] | None:
    """The identifier field ``name`` is a parent of by name, and the place
    it takes among that field's parents; ``None`` where it is none."""
    for rank, prefix in enumerate(PARENT_PREFIXES):
        if not name.startswith(prefix):
            continue
        base = name[len(prefix):]
        # ``origin...`` and ``original...`` are words of their own.
        if prefix == "orig" and base.startswith("in"):
            return None
        if base == "clordid":
            return (base, 0) if prefix == "orig" else None
        if not base.endswith("id") or parent_of(base, names) is not None:
            return None
        return (base, rank) if base in names else None
    return None


def attach_parents(catalog: dict[str, list[dict[str, Any]]]) -> None:
    """Write onto every identifier field the parents the dictionary's own
    field names say it has, as ``FIX:parents``, each type once; and where a
    field is its base's one parent, name it by the other prefix too - an
    ``orig`` field also ``parent``, a ``parent`` field also ``orig`` - since
    with one parent the two spellings are one field: OrigClOrdID(41) is also
    ``parentclordid``, ParentAllocID(1593) also ``origallocid``. A spelling
    another field holds, as its name or an alias, is never taken."""
    by_name = {field["name"]: field for field in catalog["fields"]}
    names = set(by_name)
    parents: dict[str, list[tuple[int, str]]] = {}
    for name in sorted(names):
        found = parent_of(name, names)
        if found is not None:
            base, rank = found
            parents.setdefault(base, []).append((rank, name))
    # ClOrdID's one parent is OrigClOrdID by FIX's own rule, never read off a
    # ``parent`` name, so the swap reads the parent fields themselves.
    held_names = names | {
        alias for field in catalog["fields"] for alias in field["metadata"].get("FIX:names", [])
    }
    for base, held in parents.items():
        metadata = by_name[base]["metadata"]
        metadata["FIX:parents"] = [name for _, name in sorted(held)]
        by_name[base]["metadata"] = dict(sorted(metadata.items()))
        if len(held) != 1:
            continue
        parent = held[0][1]
        prefix = next(prefix for prefix in PARENT_PREFIXES if parent.startswith(prefix))
        other = next(other for other in PARENT_PREFIXES if other != prefix)
        swapped = other + parent[len(prefix):]
        if swapped in held_names:
            continue
        aliases = by_name[parent]["metadata"]
        aliases["FIX:names"] = aliases.get("FIX:names", []) + [swapped]
        by_name[parent]["metadata"] = dict(sorted(aliases.items()))
        held_names.add(swapped)

# Spellings a bridge writes for a field that no FIX version ever wrote, each
# an alias ranked after every spelling a version did: OrderID(37) arrives as a
# bridge's market or OMS dealer order identifier, Account(1) as its OMS
# dealer account, ClOrdID(11) as a trader's own client order identifier,
# CFICode(461) as the detailed classification a bridge states beside a coarse
# one, SecondaryClOrdID(526) as the one an exchange uses and Username(553) as
# the OMS user.
CRATE_NAMES: tuple[tuple[int, list[str]], ...] = (
    (1, ["omsdealeraccount"]),
    (11, ["ultraderclordid"]),
    (37, ["marketorderid", "omsdealerorderid"]),
    (461, ["detailedcficode"]),
    (526, ["exchangeclientorderid"]),
    (553, ["omsuserid"]),
)


def attach_identifier_maps(
    catalog: dict[str, list[dict[str, Any]]],
    code_values: dict[int, set[str]],
) -> None:
    """Write the identifier-map and crate-name tables onto their fields,
    refusing an entry that does not resolve: every tag is a field, a key is
    one to 64 lower-case letters or digits, the map is ``identifiers``, a
    ``role`` sits on PartyID(448) and is a PartyRole(452) code, and a crate
    name is not already one of the field's."""
    by_tag = {int(field["metadata"]["FIX:tag"]): field for field in catalog["fields"]}
    for tag, entries in IDMAP_SOURCES:
        if tag not in by_tag:
            raise ValueError(f"idmap for unknown tag {tag}")
        for entry in entries:
            where = f"idmap of tag {tag}"
            if entry["map"] != "identifiers":
                raise ValueError(f"{where}: unknown map {entry['map']!r}")
            if not re.fullmatch(r"[a-z0-9]{1,64}", entry["key"]):
                raise ValueError(f"{where}: key {entry['key']!r} is not lower-case letters or digits")
            if "role" in entry:
                if tag != 448:
                    raise ValueError(f"{where}: a role sits on PartyID(448) alone")
                if entry["role"] not in code_values.get(452, set()):
                    raise ValueError(f"{where}: {entry['role']!r} is not a PartyRole code")
        metadata = by_tag[tag]["metadata"]
        metadata["FIX:idmap"] = entries
        by_tag[tag]["metadata"] = dict(sorted(metadata.items()))
    for tag, names in CRATE_NAMES:
        if tag not in by_tag:
            raise ValueError(f"crate names for unknown tag {tag}")
        metadata = by_tag[tag]["metadata"]
        held = metadata.get("FIX:names", [])
        for name in names:
            if name in held or name == by_tag[tag]["name"]:
                raise ValueError(f"crate name {name!r} of tag {tag} is already one of its names")
        metadata["FIX:names"] = held + names
        by_tag[tag]["metadata"] = dict(sorted(metadata.items()))


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


def _collection_plural(name: str) -> bool:
    """Whether a published spelling visibly names several occurrences."""
    matched = re.fullmatch(r"(.*?)(\d*)", name)
    assert matched is not None
    stem = matched[1]
    if _singularize(stem) != stem:
        return True
    # FIX's LinesOfText is a plural collection whose plural marker precedes
    # the qualifier rather than ending the spelling.
    return re.search(r"sOf[A-Z]", stem) is not None


def resolved_group_displays(
    latest: dict[str, Any], fields: list[dict[str, Any]]
) -> dict[tuple[str, int], str]:
    """Resolve standard groups to a semantic plural or an explicit ``Grp``.

    A published plural is the strongest statement. Otherwise the group's
    ``No...`` counter may name the collection, but only when one group claims
    that free spelling. Shared and occupied spellings retain the published
    group identity; ``claim`` adds ``Grp`` where a scalar owns its bare name.
    """
    scalar_names = {field["name"] for field in fields}
    scalar_names.update(
        folded(name)
        for field in fields
        for name in field.get("metadata", {}).get("FIX:names", [])
    )
    definitions: dict[tuple[str, int], dict[str, Any]] = {}
    source_names: dict[tuple[str, int], str] = {}
    for category in ("components", "groups", "messages"):
        for source_name, definition in latest[category].items():
            key = (category, definition["id"])
            definitions[key] = definition
            source_names[key] = definition.get("name", source_name)
    reserved = {folded(name) for name in source_names.values()} | scalar_names
    by_tag = {int(field["metadata"]["FIX:tag"]): field for field in fields}
    targets = {
        key
        for key, display in source_names.items()
        if key[0] == "groups"
        and (display.endswith("Grp") or folded(display) in scalar_names)
    }

    first: dict[tuple[str, int], str] = {}
    for key in targets:
        display = source_names[key]
        if display == "SecAltIDGrp":
            first[key] = "SecAltIDs"
            continue
        if display.endswith("Grp"):
            stem = display.removesuffix("Grp")
            if _collection_plural(stem):
                first[key] = stem
    first_counts: dict[str, int] = {}
    for display in first.values():
        spelling = folded(display)
        first_counts[spelling] = first_counts.get(spelling, 0) + 1
    resolved = {
        key: display
        for key, display in first.items()
        if first_counts[folded(display)] == 1 and folded(display) not in reserved
    }

    second: dict[tuple[str, int], str] = {}
    for key in targets - resolved.keys():
        counter = definitions[key]["tag"]
        field = by_tag.get(counter)
        if field is None:
            continue
        display = field.get("metadata", {}).get("display", field["name"])
        if not display.startswith("No"):
            continue
        stem = display.removeprefix("No")
        if _collection_plural(stem):
            second[key] = stem
    second_counts: dict[str, int] = {}
    for display in second.values():
        spelling = folded(display)
        second_counts[spelling] = second_counts.get(spelling, 0) + 1
    claimed = {folded(display) for display in resolved.values()}
    for key, display in second.items():
        spelling = folded(display)
        if second_counts[spelling] == 1 and spelling not in reserved and spelling not in claimed:
            resolved[key] = display
            claimed.add(spelling)

    return {
        key: resolved.get(key, display)
        for key, display in source_names.items()
        if key[0] == "groups"
    }


def build(
    parsed: dict[str, dict[str, Any]],
) -> tuple[dict[str, list[dict[str, Any]]], dict[str, list[dict[str, Any]]]]:
    """Resolve the source graph once into the catalog and the code sets.

    A code set is a vocabulary rather than a property of one field: the
    specification names it - ``SideCodeSet``, ``UnitOfMeasureCodeSet`` - and
    names it from as many fields as draw on it. So the members are written
    once under ``codesets/<name>.json`` and a field's ``FIX:codeset`` states
    which set it reads by. The two answers are separate because a code set is
    not a catalog category: it holds no ``Field`` document and resolves no
    reference to one.
    """
    latest = parsed["orchestra-latest"]

    # Per-tag history, oldest first, from the versions QuickFIX publishes:
    # the name and type at each version, the versions naming the tag at all,
    # and the enum listing each version states.
    history: dict[int, list[dict[str, Any]]] = {}
    named: dict[int, list[str]] = {}
    listings: dict[int, list[tuple[str, list[dict[str, str]]]]] = {}
    for source in SOURCES:
        if source.format != "quickfix":
            continue
        held = parsed[source.source_id]
        for tag, field in held["fields"].items():
            named.setdefault(tag, []).append(source.version)
            listings.setdefault(tag, []).append((source.version, field["codes"]))
            entry = {
                "since": source.version,
                "name": folded(field["name"]),
                "type": quickfix_type(field["type"]),
            }
            entries = history.setdefault(tag, [])
            if entries and entries[-1]["name"] == entry["name"] and entries[-1]["type"] == entry["type"]:
                continue
            entries.append(entry)

    # The wire values every field's set holds at any version: what an
    # identifier map's party role is checked against.
    code_values: dict[int, set[str]] = {}
    # Every named set, by the name a field's `FIX:codeset` states.
    code_sets: dict[str, list[dict[str, Any]]] = {}

    def name_codes(tag: int, name: str, declared: str | None, document: list[dict[str, Any]]) -> str:
        """The name this set is filed under, claimed once and never shared.

        The specification's own name leads, folded. Fields are walked in
        ascending tag order, so the lowest tag holding a set claims its
        spec name - and a second field whose *document* differs, which is
        what per-tag legacy folding produces for fifteen of the shipped
        sets, takes a name of its own rather than overwriting the first.
        A tag the specification names no set for is filed under the field
        that reads by it. Two sets are never merged by name alone: a name
        is the identity, and equal members are what let one be shared.
        """
        for candidate in [folded(declared) if declared else None, f"{name}codeset", f"{name}{tag}codeset"]:
            if candidate is None:
                continue
            if re.fullmatch(r"[a-z0-9][a-z0-9_.-]*", candidate) is None:
                continue
            held = code_sets.get(candidate)
            if held is None:
                code_sets[candidate] = document
                return candidate
            if held == document:
                return candidate
        raise ValueError(f"tag {tag}: no free code set name for {declared or name!r}")

    def coded(
        tag: int,
        name: str,
        codes: list[dict[str, Any]],
        declared: str | None = None,
    ) -> str | None:
        """The name of the set this field reads by, or nothing for no set.

        The members - legacy values folded in - are filed under that name,
        and what the field carries is the name alone.
        """
        folded_codes = fold_legacy_codes(tag, codes, listings.get(tag, []), latest["version"])
        folded_codes = party_id_source_aliases(name, declared, folded_codes)
        if not folded_codes:
            return None
        code_values[tag] = {code["value"] for code in folded_codes}
        return name_codes(tag, name, declared, codes_document(folded_codes))

    fields: list[dict[str, Any]] = []
    # A tag some version declared and Latest no longer does was removed: its
    # history ends with a removed entry dated at the version after the last
    # one naming it, and the field is what that last version said it was.
    for tag in sorted(set(history) - set(latest["fields"])):
        entries = [dict(entry) for entry in history[tag]]
        current = entries[-1]
        name, fix_type = current["name"], current["type"]
        display = next(
            parsed[source.source_id]["fields"][tag]["name"]
            for source in reversed(SOURCES)
            if source.format == "quickfix" and tag in parsed[source.source_id]["fields"]
        )
        metadata = {"FIX:tag": str(tag), "display": display}
        names = [entry["name"] for entry in entries if entry.get("name") not in (None, name)]
        if names:
            metadata["FIX:names"] = list(dict.fromkeys(names))
        codes = coded(tag, name, [])
        if codes is not None:
            metadata["FIX:codeset"] = codes
        fields.append(
            {
                "name": name,
                "dtype": dtype_document(fix_type),
                "nullable": True,
                "metadata": dict(sorted(metadata.items())),
            }
        )

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
                # standing beside it, which is the rule a field merge follows.
                entries[-1] = current
            else:
                entries.append(current)
        metadata: dict[str, str] = {"FIX:tag": str(tag)}
        if field["name"] != name:
            metadata["display"] = field["name"]
        if field["doc"]:
            metadata["description"] = field["doc"]
        # A field the newest source deprecates is one it removed: the
        # dictionary keeps it so an old message still resolves, and says so,
        # so a reader restates it under what replaced it and keeps no value
        # of its own for it.
        if field.get("deprecated"):
            metadata["FIX:deprecated"] = field["deprecated"]
        # Every spelling an earlier version gave this tag is an alternate
        # name of the one the dictionary holds it under, and the store holds
        # them as the JSON array they are. A field no version spelled
        # otherwise states none, because the only entry names the field.
        names = []
        for entry in entries:
            spelling = entry.get("name")
            if spelling and spelling != name and spelling not in names:
                names.append(spelling)
        if names:
            metadata["FIX:names"] = names
        code_set_name = field["code_set"] or field["type"] or ""
        if field["code_set"] and code_set_name not in latest["code_sets"]:
            raise ValueError(f"{field['name']}: unresolved code set {code_set_name}")
        if code_set_name in latest["code_sets"]:
            held = latest["code_sets"][code_set_name]
            codes = coded(tag, name, held["codes"], code_set_name)
            if codes is not None:
                metadata["FIX:codeset"] = codes
        elif listings.get(tag):
            # Latest declares no set, so every value an older version listed
            # is a legacy code: the set is what those versions said, and it
            # is named after the field that reads by it.
            codes = coded(tag, name, [])
            if codes is not None:
                metadata["FIX:codeset"] = codes

        fields.append(
            {
                "name": name,
                "dtype": dtype_document(dtype),
                "nullable": True,
                "metadata": dict(sorted(metadata.items())),
            }
        )
    catalog = build_catalog(latest, fields)
    attach_identifier_maps(catalog, code_values)
    attach_parents(catalog)
    return catalog, code_sets


def build_catalog(
    latest: dict[str, Any], fields: list[dict[str, Any]]
) -> dict[str, list[dict[str, Any]]]:
    """Keep wire identity separate from the source's reusable named graph.

    A unique published or counter plural names a collection; otherwise the
    published group spelling keeps an explicit ``Grp``. Entry names stay
    local to the published spelling: Parties becomes Party, NestedParties2
    becomes NestedParty2.
    A category suffix resolves collisions with existing protocol vocabulary;
    the source ID disambiguates a remaining generated-name collision.

    References are native null Fields with one typed FIX metadata reference.
    The core resolves them at intake; a tree never duplicates its owners here.
    """
    result = {kind: [] for kind in ("fields", "components", "groups", "messages")}
    result["fields"] = fields
    by_tag = {int(field["metadata"]["FIX:tag"]): field for field in fields}
    used = {field["name"] for field in fields}
    if len(used) != len(fields):
        raise ValueError("duplicate folded FIX field name")
    used.update(
        folded(name)
        for field in fields
        for name in field.get("metadata", {}).get("FIX:names", [])
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
    group_displays = resolved_group_displays(latest, fields)

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
        resolved = group_displays.get(key, display)
        names[key] = claim(resolved, suffixes[key[0]], key[1], original=True)
        if key[0] == "groups" and names[key].endswith("grp") and not resolved.endswith("Grp"):
            group_displays[key] = resolved + "Grp"

    entries: dict[int, str] = {}
    entry_displays: dict[int, str] = {}
    for (category, identifier), display in sorted(source_names.items()):
        if category == "groups":
            entry_displays[identifier] = entry_name(display)
            entries[identifier] = claim(entry_displays[identifier], "Component", identifier, original=False)

    def reference(name: str, kind: str, required: bool, tag: int | None = None) -> dict[str, Any]:
        # A field reference carries the field's tag beside its name, so a
        # reader resolves it by its identity - the pair - and never by a
        # spelling alone.
        metadata = {f"FIX:{kind}": name}
        if tag is not None:
            metadata["FIX:tag"] = str(tag)
        return {
            "name": name,
            "dtype": {"type": "null"},
            "nullable": not required,
            "metadata": dict(sorted(metadata.items())),
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
                children.append(reference(by_tag[identifier]["name"], "field", required, identifier))
            else:
                target = (f"{kind}s", identifier)
                if target not in definitions:
                    raise ValueError(f"{source_names[owner]}: unresolved {kind} {identifier}")
                if kind == "group":
                    counter = definitions[target]["tag"]
                    if counter not in by_tag or by_tag[counter]["dtype"] != {"type": "int32"}:
                        raise ValueError(f"{source_names[target]}: counter {counter} must be an int32 field")
                    children.append(reference(by_tag[counter]["name"], "field", required, counter))
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
        metadata = {"display": group_displays.get(key, source_names[key])}
        if definition.get("doc"):
            metadata["description"] = definition["doc"]
        children = members(key)
        identifiers = ",".join(
            child["name"] for child in children
            if "FIX:field" in child["metadata"]
            and child["name"].endswith(IDENTIFIER_FAMILIES)
        )
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
            if identifiers:
                entry["metadata"]["FIX:identifiers"] = identifiers
            result["components"].append(entry)
            dtype = {"type": "list", "field": reference(entries[identifier], "component", True)}
            metadata["FIX:counter"] = str(counter)
            metadata["FIX:component"] = entries[identifier]
        else:
            dtype = {"type": "struct", "fields": children}
            if identifiers:
                metadata["FIX:identifiers"] = identifiers
        if category == "messages":
            wire = next(
                wire for wire, held in latest["messages"].items() if held["id"] == identifier
            )
            metadata["FIX:msgtype"] = wire
            metadata["FIX:msgcat"] = MSGCAT_BY_TYPE[wire]
        result[category].append(
            {"name": names[key], "dtype": dtype, "nullable": False, "metadata": dict(sorted(metadata.items()))}
        )

    for category in result:
        result[category].sort(key=lambda field: int(field["metadata"]["FIX:tag"]) if category == "fields" else field["name"])
    return result


# The exact number every FIX quantity, price, price offset and amount is
# typed as: `decimal128(38, 18)`, the crate's own decimal width.
DECIMAL_DOCUMENT: dict[str, Any] = {"type": "decimal128", "precision": 38, "scale": 18}


def dtype_document(name: str) -> dict[str, Any]:
    """The datatype document one FIX datatype name resolves to.

    The shard holds the crate's own serialized datatype rather than a display
    string, so a parameterized type carries its parameters where the reader
    expects them.
    """
    documents: dict[str, dict[str, Any]] = {
        "int": {"type": "int32"},
        "float": {"type": "float64"},
        "char": {"type": "string"},
        "String": {"type": "string"},
        "string": {"type": "string"},
        "Boolean": {"type": "boolean"},
        "boolean": {"type": "boolean"},
        "data": {"type": "binary"},
        "XMLData": {"type": "binary"},
        "Length": {"type": "int32"},
        "TagNum": {"type": "int32"},
        "SeqNum": {"type": "int64"},
        "NumInGroup": {"type": "int32"},
        "DayOfMonth": {"type": "int8"},
        # A quantity, a price, a price offset and an amount are exact: the
        # crate keeps them at `decimal128(38, 18)`, the width its own price
        # and quantity columns already hold, so a wire that stated `12.5`
        # never reads back as `12.499999`. A percentage and a float stay
        # what FIX calls them, a floating count.
        "Qty": DECIMAL_DOCUMENT,
        "Price": DECIMAL_DOCUMENT,
        "PriceOffset": DECIMAL_DOCUMENT,
        "Percentage": {"type": "float64"},
        "Amt": DECIMAL_DOCUMENT,
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
        "MonthYear": {"type": "string", "layout": "fixed_ascii", "fixed": 8},
        "Tenor": {"type": "string", "layout": "fixed_ascii", "fixed": 8},
        "Language": {"type": "string", "layout": "fixed_ascii", "fixed": 2},
        "Country": {"type": "country"},
        "Currency": {"type": "ccy"},
        "Exchange": {"type": "mic"},
        "MultipleCharValue": {"type": "string"},
        "MultipleStringValue": {"type": "string"},
        "XID": {"type": "string"},
        "XIDREF": {"type": "string"},
        "Pattern": {"type": "string"},
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


# The block a named definition's derived tag is taken from, and the XXH32 the
# core derives it with. Kept in step with `FixId::DEFINITION_TAG_MIN` and
# `FixId::DEFINITION_TAG_MAX` in rust/src/fix/mod.rs: a document this writes is
# loaded by that core, and a tag outside the block is refused.
DEFINITION_TAG_MIN = 100_000
DEFINITION_TAG_MAX = 1_100_000

_MASK = 0xFFFF_FFFF
_PRIME32 = (2654435761, 2246822519, 3266489917, 668265263, 374761393)


def _rotl32(value: int, count: int) -> int:
    return ((value << count) | (value >> (32 - count))) & _MASK


def _xxh32_round(acc: int, lane: int) -> int:
    return (_rotl32((acc + lane * _PRIME32[1]) & _MASK, 13) * _PRIME32[0]) & _MASK


def xxh32(data: bytes) -> int:
    """XXH32 of `data` at seed zero.

    Spelled here rather than imported: this script is the dictionary's own
    build step and runs on the standard library alone. The core's
    `yggdryl::xxhash::Xxh32` is the same function, which is what lets a tag
    derived here equal the tag the core would derive for the same name.
    """
    one, two, three, four, five = _PRIME32
    size = len(data)
    at = 0
    if size >= 16:
        first, second = (one + two) & _MASK, two
        third, fourth = 0, (-one) & _MASK
        while at + 16 <= size:
            first = _xxh32_round(first, int.from_bytes(data[at : at + 4], "little"))
            second = _xxh32_round(second, int.from_bytes(data[at + 4 : at + 8], "little"))
            third = _xxh32_round(third, int.from_bytes(data[at + 8 : at + 12], "little"))
            fourth = _xxh32_round(fourth, int.from_bytes(data[at + 12 : at + 16], "little"))
            at += 16
        held = (
            _rotl32(first, 1) + _rotl32(second, 7) + _rotl32(third, 12) + _rotl32(fourth, 18)
        ) & _MASK
    else:
        held = five
    held = (held + size) & _MASK
    while at + 4 <= size:
        held = (held + int.from_bytes(data[at : at + 4], "little") * three) & _MASK
        held = (_rotl32(held, 17) * four) & _MASK
        at += 4
    while at < size:
        held = (held + data[at] * five) & _MASK
        held = (_rotl32(held, 11) * one) & _MASK
        at += 1
    held ^= held >> 15
    held = (held * two) & _MASK
    held ^= held >> 13
    held = (held * three) & _MASK
    held ^= held >> 16
    return held


def assign_definition_tags(catalog: dict[str, list[dict[str, Any]]]) -> None:
    """Give every component, group and message a tag of its own.

    Only wire fields have a tag the specification publishes; a named definition
    has none, so one is derived from its name into a block nothing else claims.
    XXH32 of the name places it, and a slot already taken is stepped past,
    wrapping, so a name is always registrable. The core derives the same tag
    the same way, and keeps a tag a document already states rather than
    deriving a second one - so writing them here is what makes the identity the
    dictionary's rather than each reader's.

    Assignment walks components, then groups, then messages, each by name,
    so the tag a definition gets depends on the dictionary and not on the
    order a source file happened to list it in. Messages are written into
    ``components/`` but are still assigned last: the order is
    what places every tag the dictionary already states, and a message that
    moved folders keeps the tag it stated.
    """
    span = DEFINITION_TAG_MAX - DEFINITION_TAG_MIN
    taken = {
        int(field["metadata"]["FIX:tag"])
        for field in catalog["fields"]
        if "FIX:tag" in field.get("metadata", {})
    }
    for category in ("components", "groups", "messages"):
        for field in sorted(catalog[category], key=lambda held: held["name"]):
            start = xxh32(field["name"].encode()) % span
            for step in range(span):
                tag = DEFINITION_TAG_MIN + (start + step) % span
                if tag not in taken:
                    break
            else:
                raise ValueError(f"no free derived tag for {field['name']!r}")
            taken.add(tag)
            metadata = field.setdefault("metadata", {})
            metadata["FIX:tag"] = str(tag)
            field["metadata"] = dict(sorted(metadata.items()))


def render_tree(
    catalog: dict[str, list[dict[str, Any]]],
    code_sets: dict[str, list[dict[str, Any]]],
) -> dict[str, str]:
    """Render native Field documents, and the code sets they read by.

    One `path -> text` map for the whole store, which is also what the
    provenance manifest hashes: a document that is not here is not written,
    not checksummed and not swept.
    """
    shards: dict[int, list[dict[str, Any]]] = {}
    for field in catalog["fields"]:
        tag = int(field["metadata"]["FIX:tag"])
        shards.setdefault(tag // 100, []).append(field)
    # Nine digits with leading zeros, so the shards list in tag order
    # wherever they are listed.
    documents: dict[str, Any] = {
        f"fields/{shard:09}.json": held for shard, held in sorted(shards.items())
    }
    # A message is a component carrying `FIX:msgtype`: its document lives in
    # `components/` beside every other component.
    for category, folder in (("components", "components"), ("groups", "groups"), ("messages", "components")):
        for field in catalog[category]:
            path = f"{folder}/{field['name']}.json"
            if path in documents:
                raise ValueError(f"a message and a component share one document: {path}")
            documents[path] = field
    # A code set is a vocabulary rather than a definition: it holds no tag,
    # no datatype and no reference, so it lives in a folder of its own and
    # states the name it is filed under.
    for name, codes in code_sets.items():
        path = f"codesets/{name}.json"
        if path in documents:
            raise ValueError(f"two code sets share one document: {path}")
        documents[path] = {"name": name, "codes": codes}
    return {
        name: json.dumps(document, indent=2, ensure_ascii=False) + "\n"
        for name, document in sorted(documents.items())
    }


def summary(
    catalog: dict[str, list[dict[str, Any]]],
    code_sets: dict[str, list[dict[str, Any]]],
) -> str:
    """The counts as the store holds them: messages among the components."""
    components = len(catalog["components"]) + len(catalog["messages"])
    return (
        f"{len(catalog['fields'])} fields, {components} components "
        f"({len(catalog['messages'])} of them messages), {len(catalog['groups'])} groups, "
        f"{len(code_sets)} code sets"
    )


# The first tag of the crate's own block: `CRATE_TAG_MIN` in the Rust core.
CRATE_TAG_MIN = 65_000

# The named documents the crate defines and a store dump writes: the fixed
# row, its message-category vocabulary, and the two Map groups whose keys are
# the crate's own vocabulary.
CRATE_DOCUMENTS = frozenset(
    {
        "codesets/msgcatcodeset.json",
        "components/fixmsg.json",
        "groups/identifiers.json",
        "groups/metadata.json",
    }
)


def crate_owned(name: str) -> bool:
    """Whether a document under the output root is the crate's own dump.

    The crate's own documents - its field shard, code set, two Map groups,
    and fixed row ``components/fixmsg.json`` - are written by
    ``FixRegistry::write_into`` and pinned by the Rust store tests; this
    generator neither writes nor checks them, and never removes them.
    """
    if name in CRATE_DOCUMENTS:
        return True
    held = re.fullmatch(r"fields/(\d{9})\.json", name)
    return held is not None and int(held.group(1)) >= CRATE_TAG_MIN // 100


def write_tree(out: pathlib.Path, documents: dict[str, str]) -> dict[str, str]:
    """Replace generated files only; every target stays under the output root."""
    out = out.resolve()
    for tree in ("codesets", "fields", "components", "groups", "messages", "primitive", "nested"):
        for stale in sorted((out / tree).glob("*.json")):
            relative = stale.resolve().relative_to(out).as_posix()
            if relative not in documents and not crate_owned(relative):
                stale.unlink()
        # The retired trees, `messages/` among them: a message document lives
        # in `components/`.
        if tree in {"messages", "primitive", "nested"}:
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


def render_constants(
    latest: dict[str, Any],
    parsed: dict[str, dict[str, Any]],
    catalog: dict[str, list[dict[str, Any]]],
) -> str:
    """Render the protocol tag lists and the standard group names as Rust constants.

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
    group_names = []
    groups_by_identity: dict[tuple[str, str], list[dict[str, Any]]] = {}
    components = {field["name"]: field for field in catalog["components"]}
    for field in catalog["groups"]:
        identity = (
            field["metadata"]["FIX:counter"],
            field["metadata"]["display"],
        )
        groups_by_identity.setdefault(identity, []).append(field)
    displays = resolved_group_displays(latest, catalog["fields"])
    sources = set()
    for source_name, definition in latest["groups"].items():
        source = definition.get("name", source_name)
        source_folded = folded(source)
        if source_folded in sources:
            raise ValueError(f"duplicate folded FIX group display {source!r}")
        sources.add(source_folded)
        display = displays[("groups", definition["id"])]
        matches = groups_by_identity.get((str(definition["tag"]), display), [])
        if len(matches) != 1:
            raise ValueError(
                f"{source}: expected one resolved group for counter {definition['tag']} "
                f"and display {display!r}, got {len(matches)}"
            )
        group = matches[0]
        component_name = group["metadata"]["FIX:component"]
        component = components[component_name]
        group_names.append(
            (
                source_folded,
                group["name"],
                group["metadata"]["display"],
                component_name,
                component["metadata"]["display"],
            )
        )

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
        "//! Generated FIX protocol constants.",
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
        "//! message carries both. Requiredness is `nullable` on the field the",
        "//! dictionary holds, rather than a parallel table.",
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
                "#[rustfmt::skip]",
                f"pub const {name}: [i32; {len(tags)}] = [{rendered}];",
                "",
            ]
        )
    lines.extend(
        [
            "/// Resolve one published standard group spelling to the group's",
            "/// canonical/display names and its occurrence component's names.",
            "/// Custom grammars fall back to counter-based inference.",
            "#[rustfmt::skip]",
            "pub(super) fn shipped_group_names(",
            "    name: &str,",
            ") -> Option<(&'static str, &'static str, &'static str, &'static str)> {",
            "    match name {",
        ]
    )
    lines.extend(
        "        "
        + json.dumps(source, ensure_ascii=False)
        + " => Some(("
        + ", ".join(json.dumps(value, ensure_ascii=False) for value in values)
        + ")),"
        for source, *values in sorted(group_names)
    )
    lines.extend(
        [
            "        _ => None,",
            "    }",
            "}",
            "",
            "/// The generated category of one standard FIX message type.",
            "pub(super) fn msgcat_of(msgtype: &str) -> Option<&'static str> {",
            "    match msgtype {",
        ]
    )
    lines.extend(
        f'        "{msgtype}" => Some("{category}"),'
        for msgtype, category in sorted(MSGCAT_BY_TYPE.items())
    )
    lines.extend(["        _ => None,", "    }", "}", ""])
    return "\n".join(lines)


def write_constants(
    latest: dict[str, Any],
    parsed: dict[str, dict[str, Any]],
    catalog: dict[str, list[dict[str, Any]]],
) -> None:
    """Write the generated Rust constants after every dictionary write."""
    (ROOT / "rust" / "src" / "fix" / "constants.rs").write_text(
        render_constants(latest, parsed, catalog), encoding="utf-8", newline="\n"
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

    # Exhaustiveness belongs to the complete upstream repository; the graph
    # builder also reads deliberately partial fixture repositories.
    message_types = set(latest["messages"])
    if message_types != set(MSGCAT_BY_TYPE):
        missing = ", ".join(sorted(message_types - set(MSGCAT_BY_TYPE)))
        extra = ", ".join(sorted(set(MSGCAT_BY_TYPE) - message_types))
        raise ValueError(f"message category coverage differs: missing={missing}; extra={extra}")
    catalog, code_sets = build(parsed)
    assign_definition_tags(catalog)
    documents = render_tree(catalog, code_sets)
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
        for category in ("codesets", "fields", "components", "groups", "messages", "primitive", "nested"):
            for path in (out / category).glob("*.json"):
                name = path.relative_to(out).as_posix()
                if name not in expected_names and not crate_owned(name):
                    failures.append(f"unexpected {name}")
        if (out / "layouts.json").exists():
            failures.append("retired layouts.json exists")
        try:
            actual_manifest = json.loads((out / "provenance.json").read_text(encoding="utf-8"))
        except FileNotFoundError:
            actual_manifest = None
        if actual_manifest != manifest:
            failures.append("changed provenance.json")
        constants = ROOT / "rust" / "src" / "fix" / "constants.rs"
        try:
            actual_constants = constants.read_text(encoding="utf-8")
        except FileNotFoundError:
            actual_constants = None
        if actual_constants != render_constants(latest, parsed, catalog):
            failures.append("changed rust/src/fix/constants.rs")
        if failures:
            print("\n".join(failures), file=sys.stderr)
            return 1
        print(f"verified {len(documents)} documents; " + summary(catalog, code_sets))
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
    write_constants(latest, parsed, catalog)
    print(
        f"wrote {len(written)} documents ({summary(catalog, code_sets)})"
        f" at FIX {latest['version']} EP{latest['ep']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
