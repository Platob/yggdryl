#!/usr/bin/env python3
"""Check the instrument registry's seed, `config/isin/instruments.json`.

The seed is one JSON array of instruments sorted by ISIN, each an object
keyed as the registry's columns are. It is the one file maintained by hand;
the crate embeds a copy of it inside its own package,
`rust/src/isin_registry/seed.json` (`rust/src/isin_registry/seed.rs`), so a
published crate and a source distribution carry it too, and
`rust/tests/isin_registry/seed.rs` pins what that copy holds. This script is
the check run in the change that edits the file, and reports every failure
before it exits 1; `--sync` first writes the copy from the file, byte for
byte.

- the crate's copy is the file's bytes exactly;

- the document is a JSON array of objects holding only the keys `isin`,
  `ticker`, `miccode`, `currency`, `countrycode`, `cficode` and `fisn`,
  `isin`, `ticker`, `currency` and `cficode` required;
- an ISIN is twelve upper-case ASCII letters and digits closing on its
  ISO 6166 check digit, unique, and the array is sorted by it;
- `countrycode` is two upper-case letters, the ISIN's own prefix unless that
  prefix is an agency's (`EU EZ XA XB XC XD XF XK XS XT`), where it may name
  another country or be absent - an index of no one country;
- `currency` is three upper-case letters;
- `miccode` is four upper-case letters or digits that ISO 10383 assigned:
  a row of `rust/src/mic/tables.rs`;
- `cficode` is six upper-case letters in ISO 10962:2021, the edition the
  crate's CFI table reads: an index is referential (`T`), indices (`I`) -
  `TIEXXX` an equity index. The research candidates may spell an index in
  2015's `MRIXXX`, which the 2021 table reads as no classification at all,
  so the seed states the 2021 code and this check refuses `MRIXXX`;
- `fisn` is at most thirty-five printable ASCII characters, upper case, one
  `/` between a non-empty issuer and a non-empty description;
- `ticker` is one to sixty-four characters with no blank at either end.

Usage: python scripts/check_isin_seed.py [--sync] [--seed PATH] [--copy PATH] [--mics PATH]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = ROOT / "config" / "isin" / "instruments.json"
COPY = ROOT / "rust" / "src" / "isin_registry" / "seed.json"
MICS = ROOT / "rust" / "src" / "mic" / "tables.rs"

KEYS = ("isin", "ticker", "miccode", "currency", "countrycode", "cficode", "fisn")
REQUIRED = ("isin", "ticker", "currency", "cficode")
AGENCY_PREFIXES = frozenset(("EU", "EZ", "XA", "XB", "XC", "XD", "XF", "XK", "XS", "XT"))

ISIN_SHAPE = re.compile(r"[A-Z]{2}[A-Z0-9]{9}[0-9]")
COUNTRY_SHAPE = re.compile(r"[A-Z]{2}")
CURRENCY_SHAPE = re.compile(r"[A-Z]{3}")
MIC_SHAPE = re.compile(r"[A-Z0-9]{4}")
CFI_SHAPE = re.compile(r"[A-Z]{6}")
MIC_ROW = re.compile(r'\("([A-Z0-9]{4})", "')

# ISO 10962:2015 spellings the 2021 table reads as no classification, and the
# 2021 code a seed row states instead.
RETIRED_CFI = {"MRIXXX": "TIEXXX"}

MAX_TICKER_WIDTH = 64
FISN_WIDTH = 35


def isin_check_digit(body: str) -> int:
    """The ISO 6166 check digit closing the eleven characters `body`."""
    digits = "".join(str(int(char, 36)) for char in body)
    total = 0
    for at, char in enumerate(reversed(digits)):
        value = int(char)
        if at % 2 == 0:
            value *= 2
            if value > 9:
                value -= 9
        total += value
    return (10 - total % 10) % 10


def assigned_mics(path: Path) -> set[str]:
    """Every MIC the generated ISO 10383 table lists."""
    return set(MIC_ROW.findall(path.read_text(encoding="utf-8")))


def check_isin(isin: object) -> str | None:
    if not isinstance(isin, str) or not ISIN_SHAPE.fullmatch(isin):
        return f"expected twelve upper-case letters and digits, got {isin!r}"
    if isin_check_digit(isin[:11]) != int(isin[11]):
        return f"expected the check digit {isin_check_digit(isin[:11])}, got {isin!r}"
    return None


def check_fisn(fisn: object) -> str | None:
    if not isinstance(fisn, str):
        return f"expected text, got {fisn!r}"
    if len(fisn) > FISN_WIDTH:
        return f"expected at most {FISN_WIDTH} characters, got {len(fisn)}: {fisn!r}"
    if not all(" " <= char <= "~" for char in fisn):
        return f"expected printable ASCII, got {fisn!r}"
    if fisn != fisn.upper():
        return f"expected upper case, got {fisn!r}"
    if fisn.count("/") != 1:
        return f"expected one '/' between the issuer and the description, got {fisn!r}"
    issuer, description = fisn.split("/")
    if not issuer.strip() or not description.strip():
        return f"expected an issuer and a description either side of '/', got {fisn!r}"
    return None


def check_row(at: int, row: object, mics: set[str]) -> list[str]:
    """Every failure of the row at `at`, each naming the row and the key."""
    if not isinstance(row, dict):
        return [f"$[{at}]: expected an object, got {type(row).__name__}"]
    where = f"$[{at}]"
    failures = []
    for key in row:
        if key not in KEYS:
            failures.append(f"{where}.{key}: expected one of {', '.join(KEYS)}")
    for key in REQUIRED:
        if key not in row:
            failures.append(f"{where}.{key}: required")
    isin = row.get("isin")
    if "isin" in row and (failure := check_isin(isin)):
        failures.append(f"{where}.isin: {failure}")
    ticker = row.get("ticker")
    if "ticker" in row and (
        not isinstance(ticker, str)
        or not 1 <= len(ticker) <= MAX_TICKER_WIDTH
        or ticker != ticker.strip()
    ):
        failures.append(f"{where}.ticker: expected 1 to {MAX_TICKER_WIDTH} characters, trimmed, got {ticker!r}")
    country = row.get("countrycode")
    agency = isinstance(isin, str) and isin[:2] in AGENCY_PREFIXES
    if "countrycode" not in row:
        if isinstance(isin, str) and not agency:
            failures.append(
                f"{where}.countrycode: required where the ISIN's prefix {isin[:2]!r} is a country"
            )
    else:
        if not isinstance(country, str) or not COUNTRY_SHAPE.fullmatch(country):
            failures.append(f"{where}.countrycode: expected two upper-case letters, got {country!r}")
        elif isinstance(isin, str) and not agency and country != isin[:2]:
            failures.append(
                f"{where}.countrycode: expected the ISIN's prefix {isin[:2]!r}, got {country!r}"
            )
    currency = row.get("currency")
    if "currency" in row and (not isinstance(currency, str) or not CURRENCY_SHAPE.fullmatch(currency)):
        failures.append(f"{where}.currency: expected three upper-case letters, got {currency!r}")
    mic = row.get("miccode")
    if "miccode" in row:
        if not isinstance(mic, str) or not MIC_SHAPE.fullmatch(mic):
            failures.append(f"{where}.miccode: expected four upper-case letters or digits, got {mic!r}")
        elif mic not in mics:
            failures.append(f"{where}.miccode: expected a MIC ISO 10383 assigned, got {mic!r}")
    cfi = row.get("cficode")
    if "cficode" in row:
        if not isinstance(cfi, str) or not CFI_SHAPE.fullmatch(cfi):
            failures.append(f"{where}.cficode: expected six upper-case letters, got {cfi!r}")
        elif cfi in RETIRED_CFI:
            failures.append(
                f"{where}.cficode: expected ISO 10962:2021, got 2015's {cfi!r}; "
                f"an index is {RETIRED_CFI[cfi]!r}"
            )
    if "fisn" in row and (failure := check_fisn(row["fisn"])):
        failures.append(f"{where}.fisn: {failure}")
    return failures


def check(seed: Path, mics_path: Path) -> list[str]:
    """Every failure of the seed at `seed`."""
    try:
        document = json.loads(seed.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        return [f"{seed}: expected a JSON document, got {error}"]
    if not isinstance(document, list):
        return [f"$: expected a JSON array, got {type(document).__name__}"]
    mics = assigned_mics(mics_path)
    failures = []
    seen: dict[str, int] = {}
    previous = None
    for at, row in enumerate(document):
        failures.extend(check_row(at, row, mics))
        isin = row.get("isin") if isinstance(row, dict) else None
        if not isinstance(isin, str):
            continue
        if isin in seen:
            failures.append(f"$[{at}].isin: {isin} is the ISIN of $[{seen[isin]}] too")
        else:
            seen[isin] = at
        if previous is not None and isin < previous:
            failures.append(f"$[{at}].isin: expected the rows sorted by ISIN, got {isin} after {previous}")
        previous = isin
    return failures


def check_copy(seed: Path, copy: Path) -> list[str]:
    """The failure of a crate copy that is not the seed's bytes exactly."""
    try:
        same = copy.read_bytes() == seed.read_bytes()
    except OSError as error:
        return [f"{copy}: expected the crate's copy of {seed}, got {error}"]
    if not same:
        return [
            f"{copy}: expected the bytes of {seed}, got others; "
            "run python scripts/check_isin_seed.py --sync"
        ]
    return []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seed", type=Path, default=SEED, help="the seed document")
    parser.add_argument("--copy", type=Path, default=COPY, help="the crate's embedded copy")
    parser.add_argument("--mics", type=Path, default=MICS, help="the generated ISO 10383 table")
    parser.add_argument(
        "--sync",
        action="store_true",
        help="write the crate's copy from the seed, byte for byte, before checking",
    )
    arguments = parser.parse_args()
    if arguments.sync:
        arguments.copy.write_bytes(arguments.seed.read_bytes())
    failures = check(arguments.seed, arguments.mics)
    failures.extend(check_copy(arguments.seed, arguments.copy))
    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        print(f"{len(failures)} failure(s) in {arguments.seed}", file=sys.stderr)
        return 1
    count = len(json.loads(arguments.seed.read_text(encoding="utf-8")))
    print(f"{arguments.seed}: {count} instruments")
    return 0


if __name__ == "__main__":
    sys.exit(main())
