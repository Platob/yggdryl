#!/usr/bin/env python3
"""Exchange XML documents and XML Schemas with Python's own XML stack.

Self-consistency proves nothing about an exchange format, so this driver runs
the same rows through the implementation every Python program already has:

1. ``cargo test --test interop xml::`` writes ``target/xml-interop/from-rust.xml``
   and the ``from-rust.xsd`` that declares its rows.
2. ``xml.etree.ElementTree`` reads the document and asserts every row - the
   attributes as attributes, the repeats as repeats, the escaping decoded.
3. ``xmlschema`` compiles the schema and validates each row element against it,
   which is the claim that matters: the schema written beside a document
   describes that document, to an outside processor rather than to us.
4. ElementTree writes ``from-python.xml`` and a hand-written ``from-python.xsd``
   with the same rows.
5. The same cargo target runs again; its reading half decodes the external
   document under the external schema and asserts the rows. That half prints
   ``SKIPPED`` when the files are absent, and this driver fails on that word,
   so a skipped half can never read as a pass.

``xmlschema`` is a checking tool of this script only - never a dependency of
the crate.
"""

from __future__ import annotations

import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import xmlschema

REPO = Path(__file__).resolve().parent.parent
EXCHANGE = REPO / "rust" / "target" / "xml-interop"

# The rows both sides write and both sides read: two attributes, three child
# elements with one absent on a row, and one child that repeats.
ROWS = [
    {
        "id": "1",
        "Ccy": "EUR",
        "symbol": "AAPL",
        "quantity": "100",
        "note": "a < b & c",
        "tag": ["eu", "cash"],
    },
    {
        "id": "2",
        "Ccy": "USD",
        "symbol": "MSFT",
        "quantity": "25",
        "note": "line\rbreak",
        "tag": ["us"],
    },
    {
        "id": "3",
        "Ccy": "GBP",
        "symbol": "VOD.L",
        "quantity": "7",
        "note": None,
        "tag": ["gb"],
    },
]

# A carriage return is the one character a parser rewrites before a reader sees
# it, so it travels as a reference or not at all. ElementTree does not write
# one, so the direction it writes carries the plain text instead.
PYTHON_ROWS = [dict(row, note=row["note"] and row["note"].replace("\r", " ")) for row in ROWS]

SCHEMA = """<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="Trade">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="symbol" type="xs:string"/>
        <xs:element name="quantity" type="xs:long"/>
        <xs:element name="note" type="xs:string" minOccurs="0"/>
        <xs:element name="tag" type="xs:string" maxOccurs="unbounded"/>
      </xs:sequence>
      <xs:attribute name="id" type="xs:int" use="required"/>
      <xs:attribute name="Ccy" type="xs:string" use="required"/>
    </xs:complexType>
  </xs:element>
</xs:schema>
"""


def run_cargo(*, allow_skip: bool) -> None:
    """Run the Rust half, refusing a skip once the external files exist."""

    completed = subprocess.run(
        [
            "cargo",
            "test",
            "--locked",
            "--manifest-path",
            str(REPO / "rust" / "Cargo.toml"),
            "--test",
            "interop",
            "xml::",
            "--",
            "--nocapture",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    if completed.returncode != 0:
        raise SystemExit(f"the Rust half failed with {completed.returncode}")
    if not allow_skip and "SKIPPED" in completed.stdout:
        raise SystemExit("the Rust reading half skipped rather than reading")


def rows_of(root: ET.Element) -> list[dict[str, object]]:
    """Read every row element out of a document, as plain Python data."""

    rows: list[dict[str, object]] = []
    for element in root.findall("Trade"):
        row: dict[str, object] = dict(element.attrib)
        for child in element:
            if child.tag == "tag":
                row.setdefault("tag", []).append(child.text or "")
            else:
                row[child.tag] = child.text or ""
        rows.append(row)
    return rows


def read_with_elementtree(path: Path) -> None:
    """Assert the document this crate wrote, through the reference parser."""

    root = ET.parse(path).getroot()
    found = rows_of(root)
    if len(found) != len(ROWS):
        raise SystemExit(f"expected {len(ROWS)} rows in {path}, got {len(found)}")
    for expected, actual in zip(ROWS, found, strict=True):
        for name in ("id", "Ccy"):
            if actual.get(name) != expected[name]:
                raise SystemExit(f"{name}: {actual.get(name)!r} != {expected[name]!r}")
        for name in ("symbol", "quantity"):
            if actual.get(name) != expected[name]:
                raise SystemExit(f"{name}: {actual.get(name)!r} != {expected[name]!r}")
        if actual.get("note") != expected["note"]:
            raise SystemExit(f"note: {actual.get('note')!r} != {expected['note']!r}")
        if actual.get("tag") != expected["tag"]:
            raise SystemExit(f"tag: {actual.get('tag')!r} != {expected['tag']!r}")


def validate_with_xmlschema(document: Path, schema: Path) -> None:
    """Assert the schema this crate wrote describes the document beside it."""

    compiled = xmlschema.XMLSchema(str(schema))
    root = ET.parse(document).getroot()
    elements = root.findall("Trade")
    if not elements:
        raise SystemExit(f"no row elements to validate in {document}")
    for element in elements:
        compiled.validate(element)


def write_with_elementtree(path: Path) -> None:
    """Write the same rows with the reference implementation."""

    root = ET.Element("rows")
    for row in PYTHON_ROWS:
        trade = ET.SubElement(root, "Trade", {"id": row["id"], "Ccy": row["Ccy"]})
        ET.SubElement(trade, "symbol").text = row["symbol"]
        ET.SubElement(trade, "quantity").text = row["quantity"]
        if row["note"] is not None:
            ET.SubElement(trade, "note").text = row["note"]
        for tag in row["tag"]:
            ET.SubElement(trade, "tag").text = tag
    ET.ElementTree(root).write(path, encoding="UTF-8", xml_declaration=True)


def main() -> int:
    EXCHANGE.mkdir(parents=True, exist_ok=True)
    for stale in ("from-rust.xml", "from-rust.xsd", "from-python.xml", "from-python.xsd"):
        (EXCHANGE / stale).unlink(missing_ok=True)

    # The first run has nothing external to read, so its reading half skips.
    run_cargo(allow_skip=True)
    read_with_elementtree(EXCHANGE / "from-rust.xml")
    print("ElementTree read the document yggdryl wrote")

    validate_with_xmlschema(EXCHANGE / "from-rust.xml", EXCHANGE / "from-rust.xsd")
    print("xmlschema validated it against the schema yggdryl wrote beside it")

    write_with_elementtree(EXCHANGE / "from-python.xml")
    (EXCHANGE / "from-python.xsd").write_text(SCHEMA, encoding="utf-8")
    run_cargo(allow_skip=False)
    print("yggdryl read the document and the schema Python wrote")
    return 0


if __name__ == "__main__":
    sys.exit(main())
