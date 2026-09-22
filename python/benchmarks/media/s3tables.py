"""Reproducible baselines for one Amazon S3 Tables table, beside PyIceberg.

S3 Tables addresses a table bucket only by ARN, and its catalog is the Iceberg
REST endpoint the service runs. This crate speaks neither: what it speaks is
the ``s3:`` warehouse location the catalog answers for a table, so a table is
read here the way the ARN page says it is - the catalog says where the table
is, and the object backend reads it there. This benchmark times that read
beside PyIceberg's own, on one table both open.

Run after ``maturin develop --release`` with a table bucket to write into::

    YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:us-east-1:123456789012:bucket/lake \\
        python benchmarks/media/s3tables.py --min-time 0.2 --repeat 5

PyIceberg (``pip install "pyiceberg[pyarrow]==0.11.1" "boto3>=1.34"``) creates
a namespace and one table in that bucket, partitioned by ``symbol``, and
appends the rows through the catalog - the only door a commit to S3 Tables
has. Both readers then open the same table: PyIceberg through the catalog,
this crate through the warehouse ``s3:`` location, with the region the ARN
carries and whatever credentials the catalog vended alongside. The table is
dropped afterwards, whatever happened in between.

Three operations are timed on both sides: opening the table (the REST load
against the metadata read), a full scan to Arrow, and a scan pruned to one of
four partitions. What the two sides read is compared before anything is timed,
because a measurement of two readers reading different tables would say
nothing.

Nothing runs without the ARN or without PyIceberg: the script reports
``SKIPPED`` and names what is missing, rather than passing quietly.
"""

from __future__ import annotations

import argparse
import gc
import os
import platform
import statistics
import time
import timeit
import uuid
from collections.abc import Callable
from typing import Any

import pyarrow as pa

from yggdryl import Arn
from yggdryl.holder import ObjectFolder
from yggdryl.iceberg import Table

ARN_VARIABLE = "YGGDRYL_S3TABLES_ARN"
NAMESPACE = "yggdryl_bench"
ROW_COUNT = 65_536
BATCH_SIZE = 8_192
SYMBOLS = ("AAPL", "MSFT", "GOOG", "BP")
PRUNED_SYMBOL = "AAPL"
PRUNED_ROWS = ROW_COUNT // len(SYMBOLS)

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ]
)
# Each batch holds one symbol, so the identity partition on `symbol` lands
# the rows in four data files and a pruned scan reads exactly one of them.
BATCHES = tuple(
    pa.record_batch(
        {
            "id": list(range(start, start + BATCH_SIZE)),
            "symbol": [SYMBOLS[(start // BATCH_SIZE) % len(SYMBOLS)]] * BATCH_SIZE,
            "price": [float(start)] * BATCH_SIZE,
        },
        schema=SCHEMA,
    )
    for start in range(0, ROW_COUNT, BATCH_SIZE)
)
TABLE = pa.Table.from_batches(BATCHES, schema=SCHEMA)

# The names PyIceberg keeps a vended credential under. `ObjectFolder` reads
# them as they are spelled, so the catalog's answer is handed over whole
# rather than translated.
VENDED = (
    "s3.access-key-id",
    "s3.secret-access-key",
    "s3.session-token",
    "s3.region",
    "s3.endpoint",
)


def _skip(reason: str) -> int:
    """Report the benchmark as skipped, naming what is missing."""
    print(f"s3tables-benchmark: SKIPPED ({reason})")
    return 0


def _table_bucket(value: str) -> Arn:
    """The table bucket the environment names, refused when it names less."""
    try:
        bucket = Arn(value)
    except ValueError as error:
        raise SystemExit(f"{ARN_VARIABLE} is not an ARN: {error}") from error
    if bucket.service != "s3tables" or bucket.bucket is None:
        raise SystemExit(f"{ARN_VARIABLE} must name an S3 Tables table bucket: {value}")
    if bucket.table is not None:
        raise SystemExit(
            f"{ARN_VARIABLE} names a table; the benchmark creates its own, "
            f"so name the bucket: {value}"
        )
    if bucket.region is None:
        raise SystemExit(f"{ARN_VARIABLE} carries no region: {value}")
    return bucket


def _catalog(bucket: Arn) -> Any:
    """PyIceberg's REST catalog over the S3 Tables endpoint of the ARN's region."""
    from pyiceberg.catalog import load_catalog

    return load_catalog(
        "s3tables",
        **{
            "type": "rest",
            "uri": f"https://s3tables.{bucket.region}.amazonaws.com/iceberg",
            "warehouse": str(bucket),
            "rest.sigv4-enabled": "true",
            "rest.signing-name": "s3tables",
            "rest.signing-region": bucket.region,
        },
    )


def _create(catalog: Any, name: str) -> Any:
    """Create the table this run fills, and nothing else.

    Creating is separated from filling so the drop can guard every commit
    that follows: the table exists the moment this returns, and a partition
    or an append that fails afterwards must not leave one behind in somebody
    else's billed bucket.
    """
    catalog.create_namespace_if_not_exists(NAMESPACE)
    return catalog.create_table((NAMESPACE, name), schema=SCHEMA)


def _fill(table: Any) -> float:
    """Partition the table and append the rows, timing the append.

    A commit to S3 Tables has no door but the catalog, so this is the one
    write the benchmark reports, and it is reported as PyIceberg's alone.
    """
    from pyiceberg.transforms import IdentityTransform

    with table.update_spec() as update:
        update.add_field("symbol", IdentityTransform(), "symbol")
    started = time.perf_counter()
    table.append(TABLE)
    return time.perf_counter() - started


def _root(table: Any, bucket: Arn) -> tuple[ObjectFolder, str]:
    """The warehouse folder this crate reads the table from, and how it signs.

    The region is the ARN's. The credentials are the ones the catalog vended
    with the table when it did, spelled exactly as PyIceberg keeps them; when
    it vended none, the AWS chain answers on the first request.
    """
    options: dict[str, str] = {"region": str(bucket.region)}
    vended = {
        name: str(value)
        for name, value in table.io.properties.items()
        if name in VENDED and value is not None
    }
    options.update(vended)
    signing = "credentials vended by the catalog" if vended else "the AWS chain"
    return ObjectFolder(table.location(), options=options), signing


def _open_pyiceberg(catalog: Any, name: str) -> Callable[[], object]:
    return lambda: catalog.load_table((NAMESPACE, name)).metadata_location


def _open_yggdryl(root: ObjectFolder) -> Callable[[], object]:
    return lambda: Table.open(root).metadata_location


def _scan_pyiceberg(table: Any) -> Callable[[], object]:
    return lambda: table.scan().to_arrow().num_rows


def _scan_yggdryl(table: Table) -> Callable[[], object]:
    return lambda: table.scan().read_all().num_rows


def _pruned_pyiceberg(table: Any) -> Callable[[], object]:
    predicate = f"symbol == '{PRUNED_SYMBOL}'"
    return lambda: table.scan(row_filter=predicate).to_arrow().num_rows


def _pruned_yggdryl(table: Table) -> Callable[[], object]:
    return lambda: table.scan_where({"symbol": PRUNED_SYMBOL}).read_all().num_rows


def _document(location: str) -> str:
    """The metadata document a location names, free of how the folder is spelled."""
    return location.rsplit("/", 1)[-1]


def _verify(theirs: Any, ours: Table) -> None:
    """Prove both sides read the same table before timing either."""
    if _document(ours.metadata_location) != _document(theirs.metadata_location):
        raise AssertionError(
            "the two readers opened different documents: "
            f"{ours.metadata_location} != {theirs.metadata_location}"
        )
    their_rows = theirs.scan().to_arrow()
    our_rows = ours.scan().read_all()
    if their_rows.num_rows != ROW_COUNT or our_rows.num_rows != ROW_COUNT:
        raise AssertionError(
            f"full scans read {their_rows.num_rows} and {our_rows.num_rows} rows, "
            f"not {ROW_COUNT}"
        )
    their_ids = sorted(their_rows.column("id").to_pylist())
    our_ids = sorted(our_rows.column("id").to_pylist())
    if their_ids != our_ids:
        raise AssertionError("the two readers disagree on which rows the table holds")
    pruned = (
        _pruned_pyiceberg(theirs)(),
        _pruned_yggdryl(ours)(),
    )
    if pruned != (PRUNED_ROWS, PRUNED_ROWS):
        raise AssertionError(f"pruned scans read {pruned} rows, not {PRUNED_ROWS}")
    plan = ours.plan({"symbol": PRUNED_SYMBOL})
    print(
        f"both read {ROW_COUNT:,} rows, {PRUNED_ROWS:,} of them for {PRUNED_SYMBOL}; "
        f"the pruned plan reads {plan.files_planned} of "
        f"{plan.files_planned + plan.files_skipped} files"
    )


def _measure(
    operation: Callable[[], object], *, minimum_seconds: float, repeat: int
) -> tuple[float, float, int]:
    operation()
    number = 1
    while number < 4_096:
        if timeit.timeit(operation, number=number) >= minimum_seconds:
            break
        number *= 2
    gc.collect()
    samples = timeit.repeat(operation, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def _drop(catalog: Any, name: str) -> None:
    """Drop the table, then the namespace when nothing else is left in it."""
    catalog.drop_table((NAMESPACE, name))
    try:
        catalog.drop_namespace(NAMESPACE)
    except Exception as error:  # noqa: BLE001 - somebody else's tables stay
        print(f"namespace {NAMESPACE} kept: {error}")


def main() -> int:
    """Time one S3 Tables table through both readers, then drop it."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    named = os.environ.get(ARN_VARIABLE)
    if not named:
        return _skip(f"set {ARN_VARIABLE} to a table bucket ARN")
    try:
        import pyiceberg
        import boto3  # noqa: F401 - what PyIceberg's sigv4 adapter signs with
    except ImportError as error:
        return _skip(
            f"{error}; install "
            '"pyiceberg[pyarrow]==0.11.1" "boto3>=1.34" to run this benchmark'
        )

    bucket = _table_bucket(named)
    name = f"t_{uuid.uuid4().hex[:8]}"
    print(
        f"Python {platform.python_version()}, PyArrow {pa.__version__}, "
        f"PyIceberg {pyiceberg.__version__}; {ROW_COUNT:,} rows, {len(SCHEMA)} "
        f"columns, {len(SYMBOLS)} partitions"
    )
    print(
        f"table bucket {bucket.bucket} in {bucket.region} ({bucket.locator()}); "
        f"table {NAMESPACE}.{name}"
    )

    catalog = _catalog(bucket)
    theirs = _create(catalog, name)
    try:
        appended = _fill(theirs)
        print(f"pyiceberg appended {ROW_COUNT:,} rows through the catalog in {appended:.3f} s")
        root, signing = _root(theirs, bucket)
        print(f"yggdryl reads {theirs.location()} with {signing}")
        ours = Table.open(root)
        _verify(theirs, ours)

        print()
        print(f"{'benchmark':32} {'pyiceberg':>14} {'yggdryl':>14} {'ratio':>8}")
        print("-" * 72)
        gc.disable()
        try:
            measured = [
                (
                    "open the table",
                    _open_pyiceberg(catalog, name),
                    _open_yggdryl(root),
                ),
                (
                    f"full scan, {len(SYMBOLS)} files",
                    _scan_pyiceberg(theirs),
                    _scan_yggdryl(ours),
                ),
                (
                    f"pruned scan, 1 file of {len(SYMBOLS)}",
                    _pruned_pyiceberg(theirs),
                    _pruned_yggdryl(ours),
                ),
            ]
            for label, their_operation, our_operation in measured:
                their_median, _, their_iterations = _measure(
                    their_operation,
                    minimum_seconds=arguments.min_time,
                    repeat=arguments.repeat,
                )
                our_median, _, our_iterations = _measure(
                    our_operation,
                    minimum_seconds=arguments.min_time,
                    repeat=arguments.repeat,
                )
                print(
                    f"{label:32} "
                    f"{their_median * 1_000:11.3f} ms "
                    f"{our_median * 1_000:11.3f} ms "
                    f"{their_median / our_median:7.2f}x "
                    f"({their_iterations}/{our_iterations} iterations)"
                )
        finally:
            gc.enable()
        print()
        print("medians; ratio = PyIceberg / yggdryl, so above 1 is in this crate's favor")
    finally:
        _drop(catalog, name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
