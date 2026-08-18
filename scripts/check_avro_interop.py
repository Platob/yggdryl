#!/usr/bin/env python3
"""Exchange Avro object containers with fastavro, both directions.

Self-consistency proves nothing about an exchange format, so this driver runs
the same data through the reference-quality Python implementation:

1. ``cargo test --test avro_interop`` writes ``target/avro-interop/from-rust.avro``.
2. fastavro reads it and asserts every row, logical types included.
3. fastavro writes ``target/avro-interop/from-fastavro.avro`` with the same
   rows, once per codec it shares with the Rust reader.
4. The same cargo target runs again; its reading half decodes the external
   file and asserts the rows. That half prints ``SKIPPED`` when the file is
   absent, and this driver fails on that word, so a skipped half can never
   read as a pass.

fastavro is a checking tool of this script only - never a dependency of the
crate.
"""

from __future__ import annotations

import datetime
import decimal
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXCHANGE = REPO / "rust" / "target" / "avro-interop"

SCHEMA = {
    "type": "record",
    "name": "trade",
    "fields": [
        {"name": "symbol", "type": "string"},
        {"name": "quantity", "type": "long"},
        {"name": "price", "type": ["null", "double"], "default": None},
        {"name": "day", "type": {"type": "int", "logicalType": "date"}},
        {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
        {
            "name": "cost",
            "type": {
                "type": "bytes",
                "logicalType": "decimal",
                "precision": 10,
                "scale": 2,
            },
        },
        {"name": "tags", "type": {"type": "array", "items": "string"}},
        {
            "name": "extra",
            "type": {
                "type": "record",
                "name": "extra",
                "fields": [{"name": "flag", "type": "boolean"}],
            },
        },
    ],
}

UTC = datetime.timezone.utc
EPOCH = datetime.date(1970, 1, 1)

EXPECTED = [
    {
        "symbol": "AAPL",
        "quantity": 100,
        "price": 187.5,
        "day": EPOCH + datetime.timedelta(days=19_782),
        "at": datetime.datetime.fromtimestamp(1_700_000_000, tz=UTC),
        "cost": decimal.Decimal("187.50"),
        "tags": ["tech", "large"],
        "extra": {"flag": True},
    },
    {
        "symbol": "MSFT",
        "quantity": -25,
        "price": None,
        "day": EPOCH + datetime.timedelta(days=-3_652),
        "at": datetime.datetime.fromtimestamp(-1, tz=UTC),
        "cost": decimal.Decimal("-0.99"),
        "tags": [],
        "extra": {"flag": False},
    },
]


def run_cargo(allow_skip: bool) -> str:
    """Run the Rust half and hand back its combined output.

    The first run happens before fastavro has written anything, so its
    reading half legitimately skips; every later run must not.
    """
    result = subprocess.run(
        [
            "cargo",
            "test",
            "--test",
            "avro_interop",
            "--",
            "--nocapture",
        ],
        cwd=REPO / "rust",
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise SystemExit(f"cargo test failed:\n{output}")
    if not allow_skip and "SKIPPED" in output:
        raise SystemExit(f"the Rust half skipped an exchange:\n{output}")
    return output


def read_with_fastavro() -> None:
    """Read the Rust-written container and assert every row."""
    import fastavro

    path = EXCHANGE / "from-rust.avro"
    with path.open("rb") as source:
        reader = fastavro.reader(source)
        rows = list(reader)
        assert reader.metadata.get("exchange") == "yggdryl", reader.metadata
    assert rows == EXPECTED, f"fastavro read different rows:\n{rows}"
    print("fastavro: read the Rust container")


def write_with_fastavro() -> None:
    """Write the same rows back for the Rust reader, per shared codec."""
    import fastavro

    for codec in ("null", "deflate"):
        path = EXCHANGE / "from-fastavro.avro"
        with path.open("wb") as target:
            fastavro.writer(target, SCHEMA, EXPECTED, codec=codec)
        output = run_cargo(allow_skip=False)
        if "avro-interop: read" not in output:
            raise SystemExit(f"the Rust reader never confirmed ({codec}):\n{output}")
        print(f"fastavro: wrote with {codec}, Rust read it back")


def main() -> None:
    EXCHANGE.mkdir(parents=True, exist_ok=True)
    output = run_cargo(allow_skip=True)
    if "avro-interop: wrote" not in output:
        raise SystemExit(f"the Rust writer never confirmed:\n{output}")
    read_with_fastavro()
    write_with_fastavro()
    print("avro interop: ok")


if __name__ == "__main__":
    sys.exit(main())
