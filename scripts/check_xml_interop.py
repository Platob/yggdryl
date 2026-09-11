#!/usr/bin/env python3
"""Exchange XML documents with Python's ``xml.etree.ElementTree``, both directions.

Self-consistency proves nothing about an exchange format, so this driver runs
the same document through the standard library's implementation:

1. ``cargo test --test interop xml::`` writes ``target/xml-interop/from-rust.xml``.
2. ``ElementTree`` parses it and asserts the whole infoset this crate claims to
   write - the root element's name, an attribute, a repeated element, an empty
   element, a prefixed name, and the character data that only survives as
   references.
3. ``ElementTree`` writes ``target/xml-interop/from-python.xml`` with the same
   content, laid out and escaped its own way.
4. The same cargo target runs again; its reading half decodes the external
   document and asserts the exchanged value. That half prints ``SKIPPED`` when
   the file is absent, and this driver fails on that word, so a skipped half can
   never read as a pass.

``xml.etree.ElementTree`` is the standard library, so this adds no dependency to
anything.
"""

from __future__ import annotations

import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXCHANGE = REPO / "rust" / "target" / "xml-interop"
NS = "urn:example"


def run_cargo() -> str:
    """Run the interop target and answer its output, failing on SKIPPED."""

    completed = subprocess.run(
        [
            "cargo",
            "test",
            "--locked",
            "-p",
            "yggdryl",
            "--test",
            "interop",
            "xml::",
            "--",
            "--nocapture",
        ],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    output = completed.stdout + completed.stderr
    if completed.returncode != 0:
        sys.exit(f"cargo test failed:\n{output}")
    return output


def read_from_rust() -> None:
    """Assert the document this crate wrote, with the reference parser."""

    path = EXCHANGE / "from-rust.xml"
    root = ET.parse(path).getroot()
    assert root.tag == "trades", root.tag
    assert root.attrib == {"venue": "XPAR"}, root.attrib

    trades = root.findall("trade")
    assert len(trades) == 2, len(trades)
    assert [trade.get("id") for trade in trades] == ["1", "2"]
    assert [trade.findtext("symbol") for trade in trades] == ["AAPL", "MSFT"]
    assert [trade.findtext("size") for trade in trades] == ["100", "250"]

    # The escaped text arrives unescaped, and the empty element carries none.
    assert trades[0].findtext("note") == "a & b <c>"
    note = trades[1].find("note")
    assert note is not None and note.text is None, ET.tostring(trades[1])

    # ElementTree resolves the prefix into the name, which is what a prefix is.
    total = root.find(f"{{{NS}}}total")
    assert total is not None, ET.tostring(root)
    assert total.text == "350", total.text


def write_from_python() -> None:
    """Write the same document with the reference implementation."""

    ET.register_namespace("ns", NS)
    root = ET.Element("trades", {"venue": "XPAR"})
    for identifier, symbol, size, note in (
        ("1", "AAPL", "100", "a & b <c>"),
        ("2", "MSFT", "250", None),
    ):
        trade = ET.SubElement(root, "trade", {"id": identifier})
        ET.SubElement(trade, "note").text = note
        ET.SubElement(trade, "size").text = size
        ET.SubElement(trade, "symbol").text = symbol
    ET.SubElement(root, f"{{{NS}}}total").text = "350"

    EXCHANGE.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(
        EXCHANGE / "from-python.xml", encoding="utf-8", xml_declaration=True
    )


def main() -> None:
    (EXCHANGE / "from-python.xml").unlink(missing_ok=True)

    first = run_cargo()
    if "SKIPPED" not in first:
        sys.exit("the reading half must skip before the external document exists")
    read_from_rust()

    write_from_python()
    second = run_cargo()
    if "SKIPPED" in second:
        sys.exit(f"the reading half skipped with the document present:\n{second}")
    print("xml interop: both directions agree")


if __name__ == "__main__":
    main()
