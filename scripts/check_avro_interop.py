#!/usr/bin/env python3
"""Exchange Avro object containers with fastavro, both directions.

Self-consistency proves nothing about an exchange format, so this driver runs
the same data through the reference-quality Python implementation:

1. ``cargo test --test interop avro::`` writes ``target/avro-interop/from-rust.avro``.
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
            "interop",
            "avro::",
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


APACHE_PROBE_MANIFEST = """\
[package]
name = "avro-interop-apache"
version = "0.0.0"
edition = "2021"

# The probe lives under the repository's target directory; an empty workspace
# table keeps cargo from adopting it into the repository workspace.
[workspace]

[dependencies]
apache-avro = "0.17"
"""

APACHE_PROBE_MAIN = """\
//! Round-trip the Rust-written container through the apache-avro crate.

use apache_avro::{Reader, Writer};

fn main() {
    let exchange = std::path::PathBuf::from(std::env::args().nth(1).expect("an exchange dir"));
    let source = std::fs::File::open(exchange.join("from-rust.avro")).expect("the Rust file");
    let reader = Reader::new(source).expect("an apache-avro reader");
    let schema = reader.writer_schema().clone();
    let rows: Vec<apache_avro::types::Value> =
        reader.map(|row| row.expect("a decoded row")).collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let spelled = format!("{:?}", rows[0]);
    for needle in ["AAPL", "Date(19782)", "TimestampMicros(1700000000000000)"] {
        assert!(spelled.contains(needle), "{spelled}");
    }

    let target = std::fs::File::create(exchange.join("from-apache.avro")).expect("a target");
    let mut writer = Writer::new(&schema, target);
    for row in rows {
        writer.append_value_ref(&row).expect("an appended row");
    }
    writer.flush().expect("a flushed container");
    println!("apache-avro: round-tripped the Rust container");
}
"""


def check_with_apache_avro() -> bool:
    """Round-trip through the apache-avro crate in a scratch project.

    The crate is a checking tool of this script only. A build failure - an
    offline runner, a yanked version - is reported and tolerated; a probe
    that *ran* must be read back, which main() enforces by marker.
    """
    probe = EXCHANGE / "apache-probe"
    (probe / "src").mkdir(parents=True, exist_ok=True)
    (probe / "Cargo.toml").write_text(APACHE_PROBE_MANIFEST)
    (probe / "src" / "main.rs").write_text(APACHE_PROBE_MAIN)
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--", str(EXCHANGE)],
        cwd=probe,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        print(f"apache-avro: probe unavailable, skipping ({result.stderr.strip()[-200:]})")
        return False
    print(result.stdout.strip())
    return True


def main() -> None:
    EXCHANGE.mkdir(parents=True, exist_ok=True)
    output = run_cargo(allow_skip=True)
    if "avro-interop: wrote" not in output:
        raise SystemExit(f"the Rust writer never confirmed:\n{output}")
    read_with_fastavro()
    write_with_fastavro()
    if check_with_apache_avro():
        output = run_cargo(allow_skip=False)
        if "avro-interop: read apache" not in output:
            raise SystemExit(f"the Rust reader never confirmed the apache file:\n{output}")
        print("apache-avro: Rust read it back")
    print("avro interop: ok")


if __name__ == "__main__":
    sys.exit(main())
