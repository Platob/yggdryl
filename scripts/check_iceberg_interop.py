"""Exchange an Iceberg table with PyIceberg, in both directions.

Self-consistency proves nothing about a table format, so this driver runs the
two halves of a real exchange:

1. ``cargo test --features "parquet iceberg" --test iceberg_interop`` writes a
   partitioned v2 table into ``target/iceberg-interop/from-rust``, appending
   once and then upserting through the plain record surface. PyIceberg then
   opens it as a ``StaticTable``, and this script compares its schema,
   partition spec, snapshot, manifest entries and *rows* against what was
   written - including the entries the merge carried over untouched, which are
   the ones its column statistics said it never had to read.
2. PyIceberg writes the same rows into ``target/iceberg-interop/from-pyiceberg``.
   The same cargo target is run again; its second test opens that table with
   ``yggdryl::iceberg::Table`` and asserts the same rows come back.

The second cargo run is checked for the word ``SKIPPED``: if the external table
is missing, the Rust test says so on stdout rather than passing quietly, and
this script fails.

One deliberate patch is applied to PyIceberg, and only to it. PyIceberg's
``PyArrowFileIO.parse_location`` cannot open a canonical Windows file URI
(``file:///C:/...``): it keeps the leading slash and hands ``/C:/...`` to the
OS. That is a bug in PyIceberg's Windows path handling, not a disagreement
about the table format, and the patch is confined to this script. Everything
else - the metadata JSON, the Avro manifests, the Parquet data files - is read
by unmodified PyIceberg.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import warnings
from pathlib import Path
from urllib.parse import urlparse

warnings.simplefilter("ignore")

REPO = Path(__file__).resolve().parent.parent
INTEROP = REPO / "target" / "iceberg-interop"
FROM_RUST = INTEROP / "from-rust"
FROM_PYICEBERG = INTEROP / "from-pyiceberg"
VENV_PYTHON = REPO / "python" / ".venv" / "Scripts" / "python.exe"
if not VENV_PYTHON.exists():
    VENV_PYTHON = REPO / "python" / ".venv" / "bin" / "python"

# What PyIceberg writes, and what the Rust side reads back from it.
APPENDED = [
    (1, "AAPL", "XNAS"),
    (2, "MSFT", "XNYS"),
    (3, "AAPL", "XNAS"),
    (4, None, None),
]

# What the Rust side leaves after appending those rows and then upserting two
# more on `id`: row 4 is updated in place and row 5 is new.
EXPECTED = [
    (1, "AAPL", "XNAS"),
    (2, "MSFT", "XNYS"),
    (3, "AAPL", "XNAS"),
    (4, "GOOG", None),
    (5, "BP", "XLON"),
]


def patch_pyiceberg_windows_uris() -> None:
    """Teach PyIceberg to open a canonical ``file:///C:/...`` URI."""
    import pyiceberg.io.pyarrow as pyarrow_io

    original = pyarrow_io.PyArrowFileIO.parse_location

    def parse_location(location: str, properties=None):
        uri = urlparse(location)
        if uri.scheme == "file" and not uri.netloc:
            path = uri.path
            if re.match(r"^/[A-Za-z]:", path):
                path = path[1:]
            return "file", "", path
        if properties is None:
            return original(location)
        return original(location, properties)

    pyarrow_io.PyArrowFileIO.parse_location = staticmethod(parse_location)


def file_uri(path: Path) -> str:
    """Return the canonical ``file:`` URI of a local path."""
    return "file:///" + str(path.resolve()).replace("\\", "/").lstrip("/")


def run_cargo() -> str:
    """Run the Rust half of the exchange and return its output."""
    command = [
        "cargo",
        "test",
        "--features",
        "parquet iceberg",
        "--test",
        "iceberg_interop",
        "--",
        "--nocapture",
    ]
    print(f"$ {' '.join(command)}")
    result = subprocess.run(
        command, cwd=REPO / "rust", capture_output=True, text=True, check=False
    )
    output = result.stdout + result.stderr
    print(output.rstrip())
    if result.returncode != 0:
        raise SystemExit("the Rust half of the Iceberg exchange failed")
    return output


def rows_of(table) -> list[tuple]:
    """Read a PyIceberg table as sorted (id, symbol, venue) triples."""
    arrow = table.scan().to_arrow()
    columns = arrow.to_pydict()
    triples = list(
        zip(columns["id"], columns["symbol"], columns["venue"], strict=True)
    )
    return sorted(triples)


def read_with_pyiceberg() -> None:
    """Open the Rust-written table with PyIceberg and check what it holds."""
    from pyiceberg.table import StaticTable

    metadata_dir = FROM_RUST / "metadata"
    hint = (metadata_dir / "version-hint.text").read_text().strip()
    location = file_uri(metadata_dir / f"v{hint}.metadata.json")
    print(f"\n== PyIceberg reads {location}")

    table = StaticTable.from_metadata(location)
    document = json.loads((metadata_dir / f"v{hint}.metadata.json").read_text())

    assert table.metadata.format_version == 2, table.metadata.format_version
    assert document["format-version"] == 2

    names = [field.name for field in table.schema().fields]
    assert names == ["id", "symbol", "venue"], names
    ids = [field.field_id for field in table.schema().fields]
    assert ids == [1, 2, 3], ids
    assert table.schema().fields[0].required, "id is required"
    assert not table.schema().fields[1].required, "symbol is optional"

    spec = table.spec()
    assert len(spec.fields) == 1, spec
    assert spec.fields[0].name == "venue", spec
    assert str(spec.fields[0].transform) == "identity", spec

    snapshot = table.current_snapshot()
    assert snapshot is not None, "the table has a current snapshot"
    # The last commit is the upsert, which Iceberg spells as an overwrite.
    assert snapshot.summary.operation.value == "overwrite", snapshot.summary
    assert len(table.metadata.snapshots) == 2, table.metadata.snapshots

    manifests = snapshot.manifests(table.io)
    assert manifests, "the snapshot names manifests"
    # A manifest list row summarizes the partition values below it, which is
    # what lets a planner skip the whole manifest.
    summarized = [
        summary
        for manifest in manifests
        for summary in (manifest.partitions or [])
    ]
    assert summarized, "the manifest list carries partition field summaries"

    entries = [entry for manifest in manifests for entry in manifest.fetch_manifest_entry(table.io)]
    assert len(entries) == 4, f"one data file per venue, including null: {len(entries)}"
    partitions = sorted(
        str(entry.data_file.partition[0] if entry.data_file.partition else None)
        for entry in entries
    )
    assert partitions == ["None", "XLON", "XNAS", "XNYS"], partitions
    for entry in entries:
        assert entry.data_file.record_count > 0
        assert entry.data_file.value_counts, "per-column value counts survived"
        assert entry.sequence_number is not None, "an entry has a commit order"

    # The two files the merge's statistics excluded were carried over rather
    # than rewritten, and PyIceberg reads them as exactly that.
    existing = [entry for entry in entries if entry.status.value == 0]
    assert len(existing) == 2, [entry.status for entry in entries]
    assert all(entry.sequence_number == 1 for entry in existing), existing

    actual = rows_of(table)
    assert actual == sorted(EXPECTED), f"{actual} != {sorted(EXPECTED)}"
    print(f"   schema, spec, snapshot, {len(entries)} manifest entries and {len(actual)} rows agree")
    print(f"   {len(existing)} of them are carried-over `existing` entries the merge never read")


def write_with_pyiceberg() -> None:
    """Write the same rows with PyIceberg for the Rust side to read back."""
    import pyarrow as pa
    from pyiceberg.catalog.sql import SqlCatalog
    from pyiceberg.transforms import IdentityTransform

    shutil.rmtree(FROM_PYICEBERG, ignore_errors=True)
    warehouse = INTEROP / "py-warehouse"
    shutil.rmtree(warehouse, ignore_errors=True)
    warehouse.mkdir(parents=True, exist_ok=True)

    catalog = SqlCatalog(
        "interop",
        uri=f"sqlite:///{warehouse / 'catalog.db'}",
        warehouse=file_uri(warehouse),
    )
    catalog.create_namespace("ns")

    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("symbol", pa.string()),
            pa.field("venue", pa.string()),
        ]
    )
    print(f"\n== PyIceberg writes {file_uri(FROM_PYICEBERG)}")
    table = catalog.create_table(
        "ns.trades",
        schema=schema,
        location=file_uri(FROM_PYICEBERG),
    )
    # PyIceberg renumbers a schema on create, so the partition source is named
    # rather than numbered here; this also leaves the table with several
    # metadata documents, which is what the Rust reader's fallback has to pick
    # the newest of.
    with table.update_spec() as update:
        update.add_field("venue", IdentityTransform(), "venue")
    table.append(
        pa.table(
            {
                "id": [row[0] for row in APPENDED],
                "symbol": [row[1] for row in APPENDED],
                "venue": [row[2] for row in APPENDED],
            },
            schema=schema,
        )
    )
    files = sorted(
        str(path.relative_to(FROM_PYICEBERG)).replace(os.sep, "/")
        for path in FROM_PYICEBERG.rglob("*")
        if path.is_file()
    )
    print("   wrote:")
    for name in files:
        print(f"     {name}")


def read_versions_with_pyiceberg() -> None:
    """Read the v1 and v3 tables the Rust half wrote."""
    from pyiceberg.table import StaticTable

    for version in (1, 3):
        root = INTEROP / f"from-rust-v{version}"
        hint = (root / "metadata" / "version-hint.text").read_text().strip()
        table = StaticTable.from_metadata(
            file_uri(root / "metadata" / f"v{hint}.metadata.json")
        )
        assert table.metadata.format_version == version, table.metadata.format_version
        rows = sorted(
            (row["id"], row["symbol"], row["venue"])
            for row in table.scan().to_arrow().to_pylist()
        )
        expected = sorted(APPENDED)
        assert rows == expected, f"v{version}: {rows}"
        print(f"pyiceberg: read the Rust v{version} table")


def write_versions_with_pyiceberg() -> list[int]:
    """Write v1 and v3 tables for the Rust reader; report which succeeded.

    PyIceberg's v3 write support is still growing, so a v3 refusal is
    reported and tolerated rather than failed - but a version it *did* write
    must be read back, which main() enforces by marker.
    """
    import pyarrow as pa
    from pyiceberg.catalog.sql import SqlCatalog
    from pyiceberg.transforms import IdentityTransform

    written = []
    for version in (1, 3):
        target = INTEROP / f"from-pyiceberg-v{version}"
        shutil.rmtree(target, ignore_errors=True)
        warehouse = INTEROP / f"py-warehouse-v{version}"
        shutil.rmtree(warehouse, ignore_errors=True)
        warehouse.mkdir(parents=True, exist_ok=True)
        catalog = SqlCatalog(
            f"interop-v{version}",
            uri=f"sqlite:///{warehouse / 'catalog.db'}",
            warehouse=file_uri(warehouse),
        )
        catalog.create_namespace("ns")
        schema = pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("symbol", pa.string()),
                pa.field("venue", pa.string()),
            ]
        )
        try:
            table = catalog.create_table(
                "ns.trades",
                schema=schema,
                location=file_uri(target),
                properties={"format-version": str(version)},
            )
            with table.update_spec() as update:
                update.add_field("venue", IdentityTransform(), "venue")
            table.append(
                pa.table(
                    {
                        "id": [row[0] for row in APPENDED],
                        "symbol": [row[1] for row in APPENDED],
                        "venue": [row[2] for row in APPENDED],
                    },
                    schema=schema,
                )
            )
        except Exception as error:  # noqa: BLE001 - report and move on
            print(f"pyiceberg: cannot write a v{version} table ({error})")
            shutil.rmtree(target, ignore_errors=True)
            continue
        written.append(version)
        print(f"pyiceberg: wrote a v{version} table")
    return written


def write_deletes_with_pyiceberg() -> None:
    """Write a v2 table with a merge-on-read delete, when PyIceberg can.

    PyIceberg through at least 0.11 falls back to copy-on-write ("Merge on
    read is not yet supported") and writes no position or equality delete
    file, so there is nothing external to read our delete subtraction
    against. That half prints SKIPPED and proves nothing rather than passing
    quietly; when a PyIceberg that writes delete files lands in the venv,
    this same code exercises the exchange with no edits.
    """
    import pyarrow as pa
    from pyiceberg.catalog.sql import SqlCatalog
    from pyiceberg.manifest import ManifestContent

    target = INTEROP / "from-pyiceberg-deletes"
    shutil.rmtree(target, ignore_errors=True)
    warehouse = INTEROP / "py-warehouse-deletes"
    shutil.rmtree(warehouse, ignore_errors=True)
    warehouse.mkdir(parents=True, exist_ok=True)
    catalog = SqlCatalog(
        "interop-deletes",
        uri=f"sqlite:///{warehouse / 'catalog.db'}",
        warehouse=file_uri(warehouse),
    )
    catalog.create_namespace("ns")
    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("symbol", pa.string()),
            pa.field("venue", pa.string()),
        ]
    )
    table = catalog.create_table(
        "ns.trades",
        schema=schema,
        location=file_uri(target),
        properties={"format-version": "2", "write.delete.mode": "merge-on-read"},
    )
    table.append(
        pa.table(
            {
                "id": [row[0] for row in APPENDED],
                "symbol": [row[1] for row in APPENDED],
                "venue": [row[2] for row in APPENDED],
            },
            schema=schema,
        )
    )
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        table.delete("id = 2")

    manifests = table.current_snapshot().manifests(table.io)
    if not any(manifest.content == ManifestContent.DELETES for manifest in manifests):
        print(
            "pyiceberg: SKIPPED the delete half; this PyIceberg version deletes by "
            "copy-on-write and wrote no position or equality delete file"
        )
        shutil.rmtree(target, ignore_errors=True)
        return

    import yggdryl.iceberg

    ours = yggdryl.iceberg.Table.open(str(target))
    rows = sorted(
        (row["id"], row["symbol"], row["venue"])
        for row in ours.scan().read_all().to_pylist()
    )
    expected = sorted(row for row in APPENDED if row[0] != 2)
    assert rows == expected, f"deletes: {rows} != {expected}"
    print("pyiceberg: wrote a merge-on-read delete, and the Rust reader subtracted it")


def main() -> int:
    if VENV_PYTHON.exists() and Path(sys.executable).resolve() != VENV_PYTHON.resolve():
        print(f"re-running under {VENV_PYTHON}")
        return subprocess.call([str(VENV_PYTHON), __file__, *sys.argv[1:]])

    patch_pyiceberg_windows_uris()

    print("== Rust writes its half")
    first = run_cargo()
    if "iceberg-interop: wrote" not in first:
        raise SystemExit("the Rust half did not report writing a table")
    for name in ("from-rust-v1", "from-rust-v3"):
        if f"iceberg-interop: wrote {name}" not in first:
            raise SystemExit(f"the Rust half did not report writing {name}")

    read_with_pyiceberg()
    read_versions_with_pyiceberg()
    write_with_pyiceberg()
    versions = write_versions_with_pyiceberg()
    write_deletes_with_pyiceberg()

    print("\n== Rust reads the PyIceberg half")
    second = run_cargo()
    if "SKIPPED" in second:
        raise SystemExit("the Rust side skipped the external table; nothing was cross-validated")
    if "iceberg-interop: read" not in second:
        raise SystemExit("the Rust side did not report reading the external table")
    for version in versions:
        if f"iceberg-interop: read from-pyiceberg-v{version}" not in second:
            raise SystemExit(
                f"the Rust side did not read the v{version} table PyIceberg wrote"
            )

    print("\nBoth directions agree.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
