#!/usr/bin/env python3
"""Time outside implementations over a manifest this crate wrote.

A performance claim needs a baseline the reader trusts, on the same payload
and wire. `cargo test --test interop iceberg::a_large_manifest` leaves a
deterministic ten-thousand-entry Iceberg manifest under
``target/iceberg-interop/manifest-10k.avro``; this script times fastavro and
PyIceberg's own manifest reader over that exact file and prints entries per
second, so the criterion numbers for `manifest/decode_*/10000` compare
against implementations reading identical bytes. Both packages are checking
tools of this script only - never dependencies of the crate.
"""

from __future__ import annotations

import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MANIFEST = REPO / "target" / "iceberg-interop" / "manifest-10k.avro"
ENTRIES = 10_000
REPEATS = 7


def ensure_fixture() -> None:
    """Have the Rust half (re)write the deterministic manifest."""
    result = subprocess.run(
        [
            "cargo",
            "test",
            "--features",
            "parquet iceberg",
            "--test",
            "interop",
            "iceberg::a_large_manifest",
            "--",
            "--nocapture",
        ],
        cwd=REPO / "rust",
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0 or "wrote manifest-10k.avro" not in (
        result.stdout + result.stderr
    ):
        raise SystemExit(f"the fixture writer failed:\n{result.stdout}{result.stderr}")


def timed(label: str, action) -> None:
    """Report the best-of-`REPEATS` entry rate for one reader."""
    laps = []
    for _ in range(REPEATS):
        started = time.perf_counter()
        count = action()
        laps.append(time.perf_counter() - started)
        assert count == ENTRIES, f"{label} read {count} entries"
    best = min(laps)
    median = statistics.median(laps)
    print(
        f"{label}: {ENTRIES / best:>12,.0f} entries/s best, "
        f"{ENTRIES / median:>12,.0f} median ({best * 1e3:.1f} ms best of {REPEATS})"
    )


def with_fastavro() -> None:
    import fastavro

    def read() -> int:
        with MANIFEST.open("rb") as source:
            return sum(1 for _ in fastavro.reader(source))

    timed(f"fastavro {fastavro.__version__}", read)


def with_pyiceberg() -> None:
    import pyiceberg
    from pyiceberg.io.pyarrow import PyArrowFileIO
    from pyiceberg.manifest import ManifestFile

    io = PyArrowFileIO()
    uri = MANIFEST.resolve().as_uri()
    manifest = ManifestFile.from_args(
        manifest_path=uri,
        manifest_length=MANIFEST.stat().st_size,
        partition_spec_id=0,
        added_snapshot_id=7_001,
    )

    def read() -> int:
        return len(manifest.fetch_manifest_entry(io, discard_deleted=False))

    timed(f"pyiceberg {pyiceberg.__version__}", read)


def with_yggdryl() -> None:
    """Time this crate's release readers over the same file."""
    result = subprocess.run(
        [
            "cargo",
            "test",
            "--release",
            "--features",
            "parquet iceberg",
            "--test",
            "interop",
            "iceberg::times_the_baseline_manifest",
            "--",
            "--nocapture",
        ],
        cwd=REPO / "rust",
        env={**__import__("os").environ, "YGGDRYL_BASELINE_TIMING": "1"},
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    for line in output.splitlines():
        if line.startswith("iceberg-interop: timed"):
            parts = dict(
                item.split("=") for item in line.removeprefix("iceberg-interop: timed ").split()
            )
            for label, value in parts.items():
                milliseconds = float(value.removesuffix("ms"))
                print(
                    f"yggdryl {label} (release): {ENTRIES / (milliseconds / 1e3):>12,.0f} "
                    f"entries/s best ({milliseconds:.1f} ms best of 7)"
                )
            return
    raise SystemExit(f"the Rust timing never reported:\n{output}")


def main() -> None:
    ensure_fixture()
    print(f"file: {MANIFEST} ({MANIFEST.stat().st_size:,} bytes, {ENTRIES:,} entries)")
    print(f"host: {platform.platform()} / {platform.python_implementation()} {platform.python_version()}")
    with_fastavro()
    with_pyiceberg()
    with_yggdryl()


if __name__ == "__main__":
    sys.exit(main())
