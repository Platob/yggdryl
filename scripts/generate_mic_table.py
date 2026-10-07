#!/usr/bin/env python3
"""Generate ``rust/src/mic/tables.rs`` from the ISO 10383 market identifier list.

A market identifier code names a venue, and three facts about it decide what a
capture's market means: the operating MIC it trades under, whether it is a
segment of that market, and the country it is in. ISO 10383's registration
authority publishes every code ever assigned as one CSV file, and transcribing
three thousand rows by hand is how a wrong market reaches a released crate, so
this driver reads that file and writes the Rust source ``Mic::operating``,
``Mic::is_segment`` and ``Mic::country`` binary-search.

Every row is kept whatever its status: a capture names venues that have since
expired, and an expired code still says which market it was.

The file's header spellings vary across releases, so each column is resolved
once, case-insensitively: a header that is exactly the name, else the one
header that contains it. A column no header names, or two headers name, fails
the run. So does a row that breaks the registry's own shape:

* a MIC or operating MIC that is not four of ``[A-Z0-9]``, or a country that
  is not two upper-case letters (empty is kept: a row stating none);
* an operating row (``OPRT``) whose operating MIC is not itself, or a segment
  (``SGMT``) whose operating MIC is;
* an operating MIC no row lists, or a chain of them that never reaches an
  operating row;
* one MIC on two rows that disagree.

A segment whose operating MIC has since become a segment of another market -
an expired row the registry never re-pointed - is written under the operating
MIC that chain ends at, so every operating MIC in the table is an operating
row of its own.

Run with ``--check`` to fetch the file again and fail when the committed table
is stale instead of rewriting it; ``--source`` reads a saved copy - a path or
a URL - so the script also runs offline. The table states the newest date the
file's rows record, never the day it was fetched, so ``--check`` passes for as
long as the file is the same release.
"""

from __future__ import annotations

import argparse
import csv
import io
import pathlib
import re
import sys
import urllib.request

# The official CSV, as the ISO 10383 registration authority publishes it.
SOURCE = "https://www.iso20022.org/sites/default/files/ISO10383_MIC/ISO10383_MIC.csv"

TARGET = pathlib.Path(__file__).resolve().parent.parent / "rust" / "src" / "mic" / "tables.rs"

USER_AGENT = "yggdryl-generate-mic-table"

# Each column the table reads, and the header spellings that name it.
COLUMNS: dict[str, tuple[str, ...]] = {
    "mic": ("MIC",),
    "operating": ("OPERATING MIC",),
    "kind": ("OPRT/SGMT", "O/S"),
    "country": ("ISO COUNTRY CODE",),
    "status": ("STATUS",),
    "updated": ("LAST UPDATE DATE",),
}

CODE = re.compile(r"[A-Z0-9]{4}")
COUNTRY = re.compile(r"[A-Z]{2}")


def fetch(source: str) -> bytes:
    """Return the bytes of the CSV: a URL fetched, else a saved file read."""
    try:
        if source.startswith(("https://", "http://")):
            request = urllib.request.Request(source, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=120) as response:
                return response.read()
        return pathlib.Path(source).read_bytes()
    except OSError as error:
        raise SystemExit(f"cannot read the ISO 10383 list from {source}: {error}") from error


def resolve(header: list[str]) -> dict[str, int]:
    """Return the position of every column ``COLUMNS`` names in ``header``."""
    spelled = [" ".join(name.split()).upper() for name in header]
    positions: dict[str, int] = {}
    for column, spellings in COLUMNS.items():
        exact = [at for at, name in enumerate(spelled) if name in spellings]
        found = exact or [
            at for at, name in enumerate(spelled) if any(spelling in name for spelling in spellings)
        ]
        if len(found) != 1:
            named = ", ".join(repr(header[at]) for at in found) or "no header"
            raise SystemExit(f"the {' or '.join(spellings)} column: expected one header, found {named}")
        positions[column] = found[0]
    return positions


def parse(payload: bytes) -> tuple[str, dict[str, tuple[str, str, str]]]:
    """Return the newest date a row records and every row as ``mic -> (kind, operating, country)``."""
    try:
        text = payload.decode("utf-8-sig")
    except UnicodeDecodeError as error:
        raise SystemExit(f"the ISO 10383 list is not UTF-8: {error}") from error
    reader = csv.reader(io.StringIO(text, newline=""))
    header = next(reader, None)
    if header is None:
        raise SystemExit("the ISO 10383 list is empty")
    at = resolve(header)

    rows: dict[str, tuple[str, str, str]] = {}
    newest = ""
    for line, record in enumerate(reader, start=2):
        if not any(cell.strip() for cell in record):
            continue
        if len(record) < len(header):
            raise SystemExit(f"line {line}: {len(record)} cells under a {len(header)}-column header")
        mic, operating, kind, country, status, updated = (
            record[at[column]].strip() for column in COLUMNS
        )
        if not CODE.fullmatch(mic):
            raise SystemExit(f"line {line}: MIC {mic!r} is not four of [A-Z0-9]")
        if not CODE.fullmatch(operating):
            raise SystemExit(f"line {line}: {mic}'s operating MIC {operating!r} is not four of [A-Z0-9]")
        if country and not COUNTRY.fullmatch(country):
            raise SystemExit(f"line {line}: {mic}'s country {country!r} is not two upper-case letters")
        kind = {"O": "OPRT", "S": "SGMT"}.get(kind.upper(), kind.upper())
        if kind not in ("OPRT", "SGMT"):
            raise SystemExit(f"line {line}: {mic} is neither OPRT nor SGMT but {kind!r}")
        if (kind == "OPRT") != (operating == mic):
            raise SystemExit(f"line {line}: {mic} is {kind} under the operating MIC {operating}")
        if re.fullmatch(r"\d{8}", updated):
            newest = max(newest, updated)
        row = (kind, operating, country)
        held = rows.setdefault(mic, row)
        if held != row:
            raise SystemExit(
                f"line {line}: {mic} ({status or 'no status'}) is stated twice, as {held} and as {row}"
            )
    if not rows:
        raise SystemExit("the ISO 10383 list holds no row")
    if not newest:
        raise SystemExit("no row records a LAST UPDATE DATE")
    return f"{newest[:4]}-{newest[4:6]}-{newest[6:]}", rows


def operating_of(mic: str, rows: dict[str, tuple[str, str, str]]) -> str:
    """Return the operating row ``mic``'s chain of operating MICs ends at."""
    seen: list[str] = []
    current = mic
    while True:
        if current not in rows:
            raise SystemExit(f"{seen[-1]} names the operating MIC {current}, which no row lists")
        kind, operating, _ = rows[current]
        if kind == "OPRT":
            return current
        if current in seen:
            raise SystemExit(f"operating MICs loop: {' -> '.join([*seen, current])}")
        seen.append(current)
        current = operating


def rows_of(items: list[str], per_row: int, indent: str) -> str:
    lines = []
    for start in range(0, len(items), per_row):
        lines.append(indent + " ".join(items[start : start + per_row]))
    return "\n".join(lines)


def render(newest: str, rows: dict[str, tuple[str, str, str]]) -> str:
    items = [
        f'("{mic}", "{operating_of(mic, rows)}", "{rows[mic][2]}"),' for mic in sorted(rows)
    ]
    return f"""//! Every ISO 10383 market identifier code, generated from the registry's CSV.
//!
//! Regenerate with `python scripts/generate_mic_table.py`; the same script
//! run with `--check` fetches the file again and fails when this table is
//! stale. Nothing here is edited by hand: a wrong operating MIC or country is
//! a capture filed under the wrong market.
//!
//! Source: <{SOURCE}>,
//! its rows dated up to {newest}.
//!
//! Every row is kept whatever its status, because a capture names venues that
//! have since expired. A segment whose operating MIC has since become a
//! segment itself is written under the operating MIC that chain ends at, so
//! every operating MIC below is a row naming itself.

/// `(mic, operating_mic, country)`, sorted by the MIC: what
/// [`Mic::operating`](crate::Mic::operating), [`Mic::is_segment`](crate::Mic::is_segment)
/// and [`Mic::country`](crate::Mic::country) binary-search. An operating MIC
/// names itself, a segment its market; `ZZ` is ISO 10383's own statement of
/// no single country, and an empty country a row stating none.
#[rustfmt::skip]
pub(crate) static MICS: [(&str, &str, &str); {len(items)}] = [
{rows_of(items, 4, "    ")}
];
"""


def build(source: str) -> tuple[str, int]:
    newest, rows = parse(fetch(source))
    return render(newest, rows), len(rows)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when the committed table file is stale instead of rewriting it",
    )
    parser.add_argument(
        "--source",
        default=SOURCE,
        help="the ISO 10383 CSV to read, a URL or a saved file (default: the official list)",
    )
    arguments = parser.parse_args()

    generated, count = build(arguments.source)
    if arguments.check:
        current = TARGET.read_text(encoding="utf-8") if TARGET.exists() else ""
        if current != generated:
            print(f"{TARGET} is stale; run python scripts/generate_mic_table.py")
            return 1
        print(f"{TARGET} is current ({count} codes)")
        return 0

    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(generated, encoding="utf-8", newline="\n")
    print(f"wrote {TARGET} ({count} codes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
