#!/usr/bin/env python3
"""Exchange charset tables with Python's ``codecs``, both directions.

A code page is 128 facts and no framing, so self-consistency proves nothing
about it: a table wrong in both directions round trips perfectly. This driver
runs the same facts through the standard library's own registry, which is the
outside implementation those tables were generated from and must still agree
with:

1. ``cargo test --test interop charset::`` writes
   ``target/charset-interop/from-rust.tsv``: one line per charset naming it,
   every byte it assigns in hex, and the text this crate says those bytes are.
2. ``codecs`` decodes the same bytes and asserts the same text, and encodes
   that text back and asserts the same bytes.
3. ``codecs`` writes ``target/charset-interop/from-python.tsv`` from its own
   tables, including text this crate never chose - a full Western sample, the
   C1 range that separates Latin-1 from windows-1252, and an astral scalar for
   the UTF-16 forms.
4. The same cargo target runs again; its reading half decodes and re-encodes
   the external corpus. That half prints ``SKIPPED`` when the file is absent,
   and this driver fails on that word, so a skipped half can never read as a
   pass.

``codecs`` is the standard library, so this adds no dependency to anything.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXCHANGE = ROOT / "rust" / "target" / "charset-interop"

# The charset as this crate spells it, and as Python's registry knows it.
CHARSETS = [
    ("utf-8", "utf-8"),
    ("utf-16le", "utf-16-le"),
    ("utf-16be", "utf-16-be"),
    ("us-ascii", "ascii"),
    ("iso-8859-1", "iso-8859-1"),
    ("iso-8859-2", "iso-8859-2"),
    ("iso-8859-15", "iso-8859-15"),
    ("windows-1250", "cp1250"),
    ("windows-1251", "cp1251"),
    ("windows-1252", "cp1252"),
    ("ibm437", "cp437"),
    ("ibm850", "cp850"),
    ("macintosh", "mac-roman"),
]

PYTHON_NAME = dict(CHARSETS)

# Text Python encodes for the second direction. Every charset is offered every
# sample and keeps the ones it can spell, so a charset is never asked to hold a
# scalar it has no byte for - which would be a different test.
SAMPLES = [
    "symbol,price\nAAPL,187.23\n",
    "Grüße aus Zürich",
    "prix: 12€ — déjà vu",
    "ПРИВЕТ мир",
    "Příliš žluťoučký kůň",
    "\x80\x81\x9d\x9e\x9f",
    "😀 Ω ∞",
]


def run_cargo(allow_skip: bool) -> str:
    """Run the interop target and return its output."""
    result = subprocess.run(
        ["cargo", "test", "--locked", "--test", "interop", "charset::", "--", "--nocapture"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise SystemExit(f"cargo test failed:\n{output}")
    if not allow_skip and "SKIPPED" in output:
        raise SystemExit(f"the reading half skipped, which is not a pass:\n{output}")
    return output


def read_exchange(path: pathlib.Path) -> list[tuple[str, bytes, str]]:
    """Read one exchange file into (charset, bytes, text) rows."""
    rows = []
    # Split on the line feed alone: `splitlines` also breaks on U+0085, which
    # is a perfectly ordinary character in half the charsets exchanged here.
    for line in path.read_text(encoding="utf-8").split("\n"):
        if not line:
            continue
        name, hex_bytes, text = line.split("\t")
        rows.append((name, bytes.fromhex(hex_bytes), json.loads(text)))
    return rows


def check_what_rust_wrote() -> int:
    """Decode and re-encode this crate's corpus with ``codecs``."""
    path = EXCHANGE / "from-rust.tsv"
    if not path.exists():
        raise SystemExit(f"{path} is absent; the writing half did not run")

    rows = read_exchange(path)
    names = [name for name, _, _ in rows]
    if names != [name for name, _ in CHARSETS]:
        raise SystemExit(f"expected {[n for n, _ in CHARSETS]}, got {names}")

    for name, payload, text in rows:
        codec = PYTHON_NAME[name]
        decoded = payload.decode(codec)
        if decoded != text:
            raise SystemExit(
                f"{name}: codecs decoded {decoded!r}, yggdryl said {text!r}"
            )
        encoded = text.encode(codec)
        if encoded != payload:
            raise SystemExit(
                f"{name}: codecs encoded {encoded.hex()}, yggdryl said {payload.hex()}"
            )
    print(f"codecs agrees with {len(rows)} charset tables written by yggdryl")
    return len(rows)


def write_for_rust() -> int:
    """Write a corpus from Python's own tables for the reading half."""
    EXCHANGE.mkdir(parents=True, exist_ok=True)
    lines = []
    for name, codec in CHARSETS:
        # Everything this charset can spell, joined, so one line per charset
        # still carries every sample it holds.
        held = []
        for sample in SAMPLES:
            try:
                sample.encode(codec)
            except UnicodeEncodeError:
                continue
            held.append(sample)
        text = "\n".join(held)
        payload = text.encode(codec)
        if payload.decode(codec) != text:
            raise SystemExit(f"{codec} did not round trip its own sample")
        lines.append(f"{name}\t{payload.hex()}\t{json.dumps(text)}")
    path = EXCHANGE / "from-python.tsv"
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"codecs wrote {len(lines)} charset samples to {path}")
    return len(lines)


def main() -> int:
    # The writing half first, with the external corpus still absent: its
    # reading half is expected to report SKIPPED on this pass.
    run_cargo(allow_skip=True)
    check_what_rust_wrote()
    write_for_rust()
    # And again, with the external corpus present, where a skip is a failure.
    run_cargo(allow_skip=False)
    print("charset interop passed in both directions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
