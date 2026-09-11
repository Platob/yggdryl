#!/usr/bin/env python3
"""Generate ``rust/src/charset/tables.rs`` from Python's own codec registry.

A single-byte charset is 128 facts - one Unicode scalar per byte above ASCII -
and transcribing them by hand is how a wrong scalar reaches a released crate.
The standard library already ships every one of these tables, so this driver
reads them there and writes the Rust source the crate compiles.

Each charset becomes one ``SingleByte`` value carrying four projections of the
same 128 facts:

* ``scalars`` - the Unicode scalar per byte ``0x80..=0xFF``; ``U+FFFD`` marks a
  byte the charset leaves unassigned.
* ``encoded`` and ``widths`` - those scalars pre-encoded as UTF-8, so decoding
  a high byte is a three-byte copy rather than a call into ``encode_utf8``. A
  width of zero marks the same unassigned byte ``scalars`` marks with
  ``U+FFFD``.
* ``reverse`` - ``(scalar, byte)`` pairs ordered by scalar, which is what
  encoding binary-searches.

Run with ``--check`` to fail when the committed file is stale instead of
rewriting it; that is the mode the validation gate runs.
"""

from __future__ import annotations

import argparse
import codecs
import pathlib
import sys

# The generated constant, the Python codec it is read from, and the canonical
# charset name the crate spells it with. Order decides the order of the file.
CHARSETS = [
    ("LATIN1", "iso-8859-1", "iso-8859-1"),
    ("LATIN2", "iso-8859-2", "iso-8859-2"),
    ("LATIN9", "iso-8859-15", "iso-8859-15"),
    ("CP1250", "cp1250", "windows-1250"),
    ("CP1251", "cp1251", "windows-1251"),
    ("CP1252", "cp1252", "windows-1252"),
    ("CP437", "cp437", "ibm437"),
    ("CP850", "cp850", "ibm850"),
    ("MAC_ROMAN", "mac-roman", "macintosh"),
]

TARGET = pathlib.Path(__file__).resolve().parent.parent / "rust" / "src" / "charset" / "tables.rs"

UNASSIGNED = "�"


def read_table(codec: str) -> list[str | None]:
    """Return the scalar each byte ``0x80..=0xFF`` decodes to, or ``None``."""
    scalars: list[str | None] = []
    for byte in range(0x80, 0x100):
        try:
            decoded = bytes([byte]).decode(codec)
        except UnicodeDecodeError:
            scalars.append(None)
            continue
        if len(decoded) != 1:
            raise SystemExit(f"{codec}: byte {byte:#04x} decodes to {len(decoded)} scalars")
        scalars.append(decoded)
    return scalars


def assert_ascii_identity(codec: str) -> None:
    """Refuse a charset whose low half is not US-ASCII."""
    low = bytes(range(0, 0x80))
    if low.decode(codec) != low.decode("ascii"):
        raise SystemExit(f"{codec}: bytes 0x00..0x7F are not US-ASCII")


def rust_char(scalar: str | None) -> str:
    return f"'\\u{{{ord(scalar if scalar is not None else UNASSIGNED):04X}}}'"


def rows(items: list[str], per_row: int, indent: str) -> str:
    lines = []
    for start in range(0, len(items), per_row):
        lines.append(indent + " ".join(items[start : start + per_row]))
    return "\n".join(lines)


def render(constant: str, codec: str, name: str) -> str:
    assert_ascii_identity(codec)
    scalars = read_table(codec)

    scalar_items = [f"{rust_char(scalar)}," for scalar in scalars]

    encoded_items = []
    width_items = []
    for scalar in scalars:
        if scalar is None:
            encoded_items.append("[0, 0, 0],")
            width_items.append("0,")
            continue
        utf8 = scalar.encode("utf-8")
        if len(utf8) > 3:
            raise SystemExit(f"{codec}: {scalar!r} needs four UTF-8 bytes")
        padded = utf8 + bytes(3 - len(utf8))
        encoded_items.append("[" + ", ".join(f"{byte:#04x}" for byte in padded) + "],")
        width_items.append(f"{len(utf8)},")

    pairs = sorted(
        (ord(scalar), 0x80 + index)
        for index, scalar in enumerate(scalars)
        if scalar is not None
    )
    reverse_items = [f"('\\u{{{point:04X}}}', {byte:#04x})," for point, byte in pairs]

    unassigned = sum(1 for scalar in scalars if scalar is None)
    note = (
        f"All 128 high bytes are assigned."
        if unassigned == 0
        else f"{unassigned} high bytes are unassigned and refuse to decode."
    )

    return f"""/// The `{name}` table. {note}
///
/// The rows are the table's own shape - eight scalars, four encodings, sixteen
/// widths - so a byte is found by counting rows rather than by scrolling.
#[rustfmt::skip]
pub(super) static {constant}: SingleByte = SingleByte {{
    name: "{name}",
    scalars: [
{rows(scalar_items, 8, "        ")}
    ],
    encoded: [
{rows(encoded_items, 4, "        ")}
    ],
    widths: [
{rows(width_items, 16, "        ")}
    ],
    reverse: &[
{rows(reverse_items, 4, "        ")}
    ],
}};
"""


def build() -> str:
    header = '''//! Single-byte charset tables, generated from Python's codec registry.
//!
//! Regenerate with `python scripts/generate_charset_tables.py`; the same
//! script run with `--check` fails when this file is stale. Nothing here is
//! edited by hand: a wrong scalar is a silently corrupted column, and the
//! standard library already owns every one of these mappings.
//!
//! Every charset below agrees with US-ASCII on bytes `0x00..=0x7F`, which is
//! what lets [`super::single_byte`] borrow an all-ASCII input instead of
//! transcoding it.

use super::single_byte::SingleByte;
'''
    blocks = [render(constant, codec, name) for constant, codec, name in CHARSETS]
    return header + "\n" + "\n".join(blocks)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when the committed table file is stale instead of rewriting it",
    )
    arguments = parser.parse_args()

    for _, codec, _ in CHARSETS:
        codecs.lookup(codec)

    generated = build()
    if arguments.check:
        current = TARGET.read_text(encoding="utf-8") if TARGET.exists() else ""
        if current != generated:
            print(f"{TARGET} is stale; run python scripts/generate_charset_tables.py")
            return 1
        print(f"{TARGET} is current")
        return 0

    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(generated, encoding="utf-8")
    print(f"wrote {TARGET} ({len(CHARSETS)} charsets)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
