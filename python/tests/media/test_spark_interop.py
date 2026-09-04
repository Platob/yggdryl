"""Exchange Iceberg tables with Apache Spark, the reference implementation.

Self-consistency proves nothing about a table format, so this suite runs both
halves of a real exchange over one shared Hadoop (filesystem) warehouse:
tables Spark writes, yggdryl must read; tables yggdryl writes, Spark must
read - and must be able to keep writing to.

The suite is gated: it carries the ``spark_interop`` marker, is deselected
from the default run by ``pyproject.toml``, and skips itself with a message
naming what is missing when Java, ``pyspark``, or the
``iceberg-spark-runtime`` jar is absent. ``scripts/setup_spark_interop.py``
provisions the latter two; run the suite with ``pytest -m spark_interop``.

Where the two implementations can each express a feature, the test drives it
in both directions. Where the reference implementation cannot express one,
the test covers the direction that exists and says so:

- Spark SQL has no ``uuid`` or ``fixed`` DDL type, so those columns are
  written by yggdryl and read back by Spark as the binary values the spec
  encodes them as.
- Spark has no ``time`` column type at all (SPARK-51162 added a TIME type to
  Spark 4.1's engine, but the Iceberg runtime does not map it yet), so
  ``time`` stays a yggdryl-to-yggdryl type and is exercised in the core
  suites instead.
- Iceberg fixes every temporal at microseconds (``timestamp_ns`` exists only
  from format v3), so "all temporal units" here means date, timestamp, and
  timestamptz at microseconds - exactly what the spec's v2 allows.
"""

from __future__ import annotations

import datetime
import decimal
import os
import pathlib
import shutil
import uuid as uuid_module
from typing import Any, Iterator

import pyarrow as pa
import pytest

from yggdryl import MimeType
from yggdryl.media.iceberg import Catalog, IcebergOptions, Table

pytestmark = pytest.mark.spark_interop

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
JAR_DIR = REPO / "python" / ".spark-interop"


def _jar() -> pathlib.Path | None:
    """The iceberg-spark-runtime jar, provisioned or named by environment."""
    named = os.environ.get("YGGDRYL_ICEBERG_SPARK_JAR")
    if named:
        path = pathlib.Path(named)
        return path if path.exists() else None
    if JAR_DIR.is_dir():
        jars = sorted(JAR_DIR.glob("iceberg-spark-runtime-*.jar"))
        if jars:
            return jars[-1]
    return None


def _missing() -> str | None:
    """Why the suite cannot run here, or None when it can."""
    if shutil.which("java") is None:
        return "no `java` on PATH; the Spark interop suite needs a JVM"
    try:
        import pyspark  # noqa: F401
    except ImportError:
        return (
            "pyspark is not installed; run scripts/setup_spark_interop.py "
            "to provision the Spark interop suite"
        )
    if _jar() is None:
        return (
            "no iceberg-spark-runtime jar found; run "
            "scripts/setup_spark_interop.py or set YGGDRYL_ICEBERG_SPARK_JAR"
        )
    return None


_reason = _missing()
if _reason is not None:
    pytest.skip(_reason, allow_module_level=True)


CATALOG = "interop"


@pytest.fixture(scope="module")
def warehouse(tmp_path_factory: pytest.TempPathFactory) -> pathlib.Path:
    """One filesystem warehouse both implementations resolve tables in."""
    return tmp_path_factory.mktemp("iceberg-warehouse")


@pytest.fixture(scope="module")
def spark(warehouse: pathlib.Path) -> Iterator[Any]:
    """A local Spark session with a Hadoop catalog over the warehouse."""
    from pyspark.sql import SparkSession

    session = (
        SparkSession.builder.master("local[2]")
        .appName("yggdryl-iceberg-interop")
        .config("spark.jars", str(_jar()))
        # In local mode the driver JVM is the executor too, and the session
        # extensions load during construction - before spark.jars lands - so
        # the jar goes on the driver's own classpath as well.
        .config("spark.driver.extraClassPath", str(_jar()))
        .config(
            "spark.sql.extensions",
            "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions",
        )
        .config(f"spark.sql.catalog.{CATALOG}", "org.apache.iceberg.spark.SparkCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.type", "hadoop")
        # Spark caches table metadata per identifier and only refreshes it on
        # its own commits; the whole point here is an external writer, so the
        # cache is off and every query sees the current metadata document.
        .config(f"spark.sql.catalog.{CATALOG}.cache-enabled", "false")
        .config(f"spark.sql.catalog.{CATALOG}.warehouse", warehouse.as_uri())
        .config("spark.sql.session.timeZone", "UTC")
        .config("spark.ui.enabled", "false")
        .config("spark.driver.memory", "1g")
        .getOrCreate()
    )
    yield session
    session.stop()


@pytest.fixture(scope="module")
def catalog(warehouse: pathlib.Path) -> Catalog:
    """The yggdryl catalog over the same warehouse folder."""
    return Catalog(warehouse)


def spark_rows(spark: Any, query: str) -> list[tuple[Any, ...]]:
    """Run one Spark query and return its rows as plain sorted tuples."""
    return sorted(tuple(row) for row in spark.sql(query).collect())


def scan_rows(table: Table, *columns: str) -> list[tuple[Any, ...]]:
    """Scan a yggdryl table and return the named columns as sorted tuples."""
    batches = table.scan().read_all()
    names = list(columns) if columns else batches.column_names
    pylists = [batches.column(name).to_pylist() for name in names]
    return sorted(zip(*pylists)) if pylists else []


class TestTableCreationAndFieldIds:
    """Created tables carry the field ids the other side resolves by."""

    def test_spark_creates_yggdryl_opens_with_matching_ids(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.ids.spark_made "
            "(id BIGINT NOT NULL, venue STRING) USING iceberg"
        )
        table = catalog.namespaces["ids"].tables["spark_made"]
        assert [child.parquet_field_id for child in table.schema.dtype] == [1, 2]
        assert not table.schema.dtype["id"].nullable
        assert table.schema.dtype["venue"].nullable

    def test_yggdryl_creates_spark_describes_the_same_shape(
        self, spark: Any, catalog: Catalog
    ) -> None:
        schema = pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("note", pa.string()),
            ]
        )
        catalog.tables.create("ids.ygg_made", schema)
        described = {
            row[0]: row[1]
            for row in spark_rows(spark, f"DESCRIBE TABLE {CATALOG}.ids.ygg_made")
            if row[0] and not row[0].startswith("#")
        }
        assert described["id"] == "bigint"
        assert described["note"] == "string"
        # Spark can immediately write into the yggdryl-created table.
        spark.sql(f"INSERT INTO {CATALOG}.ids.ygg_made VALUES (1, 'from spark')")
        assert scan_rows(catalog.table("ids.ygg_made")) == [(1, "from spark")]


class TestPrimitiveTypes:
    """Every primitive both implementations can express, with nulls."""

    def test_spark_writes_the_sql_primitives_yggdryl_reads_them(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.types.primitives ("
            "  id BIGINT NOT NULL,"
            "  b BOOLEAN, i INT, l BIGINT, f FLOAT, d DOUBLE,"
            "  dec9 DECIMAL(9, 2), dec38 DECIMAL(38, 10),"
            "  dt DATE, ts TIMESTAMP_NTZ, tstz TIMESTAMP,"
            "  s STRING, bin BINARY"
            ") USING iceberg"
        )
        spark.sql(
            f"INSERT INTO {CATALOG}.types.primitives VALUES "
            "(1, true, 7, 70, 1.5, 2.5, 12345.67, 1234567890.1234567890, "
            " DATE '2024-02-29', TIMESTAMP_NTZ '2024-02-29 12:34:56.789012', "
            " TIMESTAMP '2024-02-29 12:34:56.789012+00:00', 'text', X'DEADBEEF'), "
            "(2, null, null, null, null, null, null, null, "
            " null, null, null, null, null)"
        )

        table = catalog.table("types.primitives")
        rows = table.scan().read_all().sort_by("id")
        assert rows.num_rows == 2
        assert rows.column("b").to_pylist() == [True, None]
        assert rows.column("i").to_pylist() == [7, None]
        assert rows.column("l").to_pylist() == [70, None]
        assert rows.column("f").to_pylist() == [1.5, None]
        assert rows.column("d").to_pylist() == [2.5, None]
        assert rows.column("dec9").to_pylist() == [
            decimal.Decimal("12345.67"),
            None,
        ]
        assert rows.column("dec38").to_pylist() == [
            decimal.Decimal("1234567890.1234567890"),
            None,
        ]
        assert rows.column("dt").to_pylist() == [datetime.date(2024, 2, 29), None]
        assert rows.column("ts").to_pylist() == [
            datetime.datetime(2024, 2, 29, 12, 34, 56, 789012),
            None,
        ]
        tstz = rows.column("tstz").to_pylist()
        assert tstz[0] is not None and tstz[0].utcoffset() is not None
        assert tstz[0].astimezone(datetime.timezone.utc).replace(tzinfo=None) == (
            datetime.datetime(2024, 2, 29, 12, 34, 56, 789012)
        )
        assert rows.column("s").to_pylist() == ["text", None]
        assert rows.column("bin").to_pylist() == [b"\xde\xad\xbe\xef", None]

    def test_yggdryl_writes_the_same_primitives_spark_reads_them(
        self, spark: Any, catalog: Catalog
    ) -> None:
        rows = pa.table(
            {
                "id": pa.array([1, 2], pa.int64()),
                "b": pa.array([False, None], pa.bool_()),
                "i": pa.array([-7, None], pa.int32()),
                "l": pa.array([-70, None], pa.int64()),
                "f": pa.array([0.5, None], pa.float32()),
                "d": pa.array([-2.5, None], pa.float64()),
                "dec": pa.array(
                    [decimal.Decimal("99.99"), None], pa.decimal128(9, 2)
                ),
                "dt": pa.array([datetime.date(1969, 12, 31), None], pa.date32()),
                "ts": pa.array(
                    [datetime.datetime(2001, 2, 3, 4, 5, 6, 7), None],
                    pa.timestamp("us"),
                ),
                "tstz": pa.array(
                    [datetime.datetime(2001, 2, 3, 4, 5, 6, 7), None],
                    pa.timestamp("us", tz="UTC"),
                ),
                "s": pa.array(["ygg", None], pa.string()),
                "bin": pa.array([b"\x01\x02", None], pa.binary()),
            }
        )
        catalog.append("types.from_ygg", rows)

        got = spark_rows(
            spark,
            f"SELECT id, b, i, l, f, d, dec, dt, ts, s, bin "
            f"FROM {CATALOG}.types.from_ygg ORDER BY id",
        )
        assert got[0] == (
            1,
            False,
            -7,
            -70,
            0.5,
            -2.5,
            decimal.Decimal("99.99"),
            datetime.date(1969, 12, 31),
            datetime.datetime(2001, 2, 3, 4, 5, 6, 7),
            "ygg",
            b"\x01\x02",
        )
        assert got[1] == (2,) + (None,) * 10
        (tstz,) = spark.sql(
            f"SELECT tstz FROM {CATALOG}.types.from_ygg WHERE id = 1"
        ).collect()[0]
        assert tstz == datetime.datetime(2001, 2, 3, 4, 5, 6, 7)

    def test_fixed_width_binary_written_by_yggdryl_reads_in_spark(
        self, spark: Any, catalog: Catalog
    ) -> None:
        # Spark SQL cannot declare a fixed column, so the table comes from
        # this side; Spark reads the fixed(16) values as their exact bytes.
        value = uuid_module.UUID("12345678-1234-5678-1234-567812345678").bytes
        rows = pa.table(
            {
                "id": pa.array([1], pa.int64()),
                "fixed16": pa.array([value], pa.binary(16)),
            }
        )
        catalog.append("types.fixed_from_ygg", rows)
        got = spark_rows(spark, f"SELECT id, fixed16 FROM {CATALOG}.types.fixed_from_ygg")
        assert got == [(1, value)]


class TestNestedTypes:
    """Struct, list, and map round-trip in both directions, nulls included."""

    def test_spark_writes_nested_yggdryl_reads(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.nested.spark_made ("
            "  id BIGINT NOT NULL,"
            "  who STRUCT<name: STRING, age: INT>,"
            "  tags ARRAY<STRING>,"
            "  scores MAP<STRING, INT>"
            ") USING iceberg"
        )
        spark.sql(
            f"INSERT INTO {CATALOG}.nested.spark_made VALUES "
            "(1, named_struct('name', 'ada', 'age', 36), array('x', 'y'), "
            " map('a', 1, 'b', 2)), "
            "(2, null, null, null)"
        )

        rows = catalog.table("nested.spark_made").scan().read_all().sort_by("id")
        assert rows.column("who").to_pylist() == [
            {"name": "ada", "age": 36},
            None,
        ]
        assert rows.column("tags").to_pylist() == [["x", "y"], None]
        maps = rows.column("scores").to_pylist()
        assert sorted(maps[0]) == [("a", 1), ("b", 2)]
        assert maps[1] is None

    def test_yggdryl_writes_nested_spark_reads(
        self, spark: Any, catalog: Catalog
    ) -> None:
        rows = pa.table(
            {
                "id": pa.array([1, 2], pa.int64()),
                "who": pa.array(
                    [{"name": "grace", "age": 85}, None],
                    pa.struct([("name", pa.string()), ("age", pa.int32())]),
                ),
                "tags": pa.array([["p", "q"], None], pa.list_(pa.string())),
                "scores": pa.array(
                    [[("a", 1)], None], pa.map_(pa.string(), pa.int32())
                ),
            }
        )
        catalog.append("nested.from_ygg", rows)

        got = spark_rows(
            spark,
            f"SELECT id, who.name, who.age, tags, scores['a'] "
            f"FROM {CATALOG}.nested.from_ygg ORDER BY id",
        )
        assert got == [
            (1, "grace", 85, ["p", "q"], 1),
            (2, None, None, None, None),
        ]


class TestPartitioning:
    """Identity and transform partitioning, in both directions."""

    def test_spark_transform_partitions_scan_in_yggdryl(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.parts.transformed "
            "(id BIGINT NOT NULL, venue STRING, at TIMESTAMP) USING iceberg "
            "PARTITIONED BY (venue, days(at), bucket(4, id))"
        )
        spark.sql(
            f"INSERT INTO {CATALOG}.parts.transformed VALUES "
            "(1, 'XNAS', TIMESTAMP '2024-01-01 01:00:00+00:00'), "
            "(2, 'XNYS', TIMESTAMP '2024-01-02 02:00:00+00:00'), "
            "(3, null,   TIMESTAMP '2024-01-02 03:00:00+00:00')"
        )

        table = catalog.table("parts.transformed")
        assert [field.transform for field in table.spec.fields] == [
            "identity",
            "day",
            "bucket[4]",
        ]
        assert sorted(
            row[0] for row in scan_rows(table, "id")
        ) == [1, 2, 3]
        # A filter on the identity partition is answered by the plan; a null
        # partition value round-trips as the absence it is.
        venues = (
            table.scan()
            .read_all()
            .sort_by("id")
            .column("venue")
            .to_pylist()
        )
        assert venues == ["XNAS", "XNYS", None]

    def test_yggdryl_identity_partitions_prune_in_spark(
        self, spark: Any, catalog: Catalog
    ) -> None:
        marked = pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field(
                    "venue",
                    pa.string(),
                    metadata={b"field:partition": b"true"},
                ),
            ]
        )
        rows = pa.table(
            {"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=marked
        )
        catalog.append("parts.from_ygg", rows)

        assert spark_rows(
            spark, f"SELECT id FROM {CATALOG}.parts.from_ygg WHERE venue = 'XNAS'"
        ) == [(1,)]
        assert spark_rows(
            spark,
            f"SELECT id FROM {CATALOG}.parts.from_ygg WHERE venue IS NULL",
        ) == [(3,)]
        # Spark keeps writing into the partitioned layout yggdryl laid out.
        spark.sql(f"INSERT INTO {CATALOG}.parts.from_ygg VALUES (4, 'XNAS')")
        assert sorted(
            row[0]
            for row in scan_rows(catalog.table("parts.from_ygg"), "id")
        ) == [1, 2, 3, 4]


class TestSnapshotsAndTimeTravel:
    """Appends, overwrites, snapshot history, time travel, and refs."""

    def test_yggdryl_history_time_travels_in_spark(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "history.from_ygg", pa.table({"id": pa.array([1], pa.int64())})
        )
        first = table.current_snapshot.snapshot_id
        table.append(pa.table({"id": pa.array([2], pa.int64())}))
        table.overwrite(pa.table({"id": pa.array([9], pa.int64())}))

        assert spark_rows(spark, f"SELECT id FROM {CATALOG}.history.from_ygg") == [(9,)]
        assert spark_rows(
            spark,
            f"SELECT id FROM {CATALOG}.history.from_ygg VERSION AS OF {first}",
        ) == [(1,)]
        # The snapshots metadata table decodes yggdryl's manifest lists.
        operations = [
            row[0]
            for row in spark_rows(
                spark,
                f"SELECT operation FROM {CATALOG}.history.from_ygg.snapshots",
            )
        ]
        assert sorted(operations) == ["append", "append", "overwrite"]

    def test_spark_history_time_travels_in_yggdryl(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.history.from_spark (id BIGINT) USING iceberg"
        )
        spark.sql(f"INSERT INTO {CATALOG}.history.from_spark VALUES (1)")
        spark.sql(f"INSERT INTO {CATALOG}.history.from_spark VALUES (2)")
        spark.sql(
            f"INSERT OVERWRITE {CATALOG}.history.from_spark VALUES (9)"
        )

        table = catalog.table("history.from_spark")
        snapshots = table.snapshots
        assert len(snapshots) == 3
        assert scan_rows(table, "id") == [(9,)]
        first = snapshots[0].snapshot_id
        at_first = table.scan_at(first).read_all()
        assert at_first.column("id").to_pylist() == [1]

    def test_refs_written_by_either_side_resolve_in_the_other(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "history.refs", pa.table({"id": pa.array([1], pa.int64())})
        )
        tagged = table.current_snapshot.snapshot_id
        table.append(pa.table({"id": pa.array([2], pa.int64())}))

        # A tag yggdryl writes resolves by name in Spark's time travel.
        table.create_tag("ygg-v1", tagged)
        assert spark_rows(
            spark,
            f"SELECT id FROM {CATALOG}.history.refs VERSION AS OF 'ygg-v1'",
        ) == [(1,)]

        # A tag Spark writes resolves by name in yggdryl.
        spark.sql(
            f"ALTER TABLE {CATALOG}.history.refs CREATE TAG `spark-v1` "
            f"AS OF VERSION {tagged}"
        )
        reopened = catalog.table("history.refs")
        assert reopened.snapshot_by_ref("spark-v1").snapshot_id == tagged
        assert reopened.snapshot_by_ref("ygg-v1").snapshot_id == tagged


class TestSchemaEvolution:
    """Add, drop, rename, promote, and doc, read back across implementations."""

    def test_yggdryl_evolution_reads_back_in_spark(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "evolve.from_ygg",
            pa.table(
                {
                    "id": pa.array([1], pa.int32()),
                    "old_name": pa.array(["a"], pa.string()),
                    "dropped": pa.array([True], pa.bool_()),
                }
            ),
        )
        with table.update_schema() as update:
            update.add_column("", "note:string")
            update.drop_column("dropped")
            update.rename_column("old_name", "new_name")
            update.update_type("id", "int64")
            update.update_doc("new_name", "renamed by yggdryl")

        table.append(
            pa.table(
                {
                    "id": pa.array([2], pa.int64()),
                    "new_name": pa.array(["b"], pa.string()),
                    "note": pa.array(["fresh"], pa.string()),
                }
            )
        )

        got = spark_rows(
            spark,
            f"SELECT id, new_name, note FROM {CATALOG}.evolve.from_ygg ORDER BY id",
        )
        # The pre-evolution file contributes under the new name and a null
        # for the added column; the doc survives as the column comment.
        assert got == [(1, "a", None), (2, "b", "fresh")]
        comments = {
            row[0]: row[2]
            for row in spark_rows(spark, f"DESCRIBE TABLE {CATALOG}.evolve.from_ygg")
            if row[0] and not row[0].startswith("#")
        }
        assert comments["new_name"] == "renamed by yggdryl"

    def test_spark_evolution_reads_back_in_yggdryl(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.evolve.from_spark "
            "(id INT, old_name STRING) USING iceberg"
        )
        spark.sql(f"INSERT INTO {CATALOG}.evolve.from_spark VALUES (1, 'a')")
        spark.sql(
            f"ALTER TABLE {CATALOG}.evolve.from_spark ADD COLUMN note STRING"
        )
        spark.sql(
            f"ALTER TABLE {CATALOG}.evolve.from_spark RENAME COLUMN old_name TO new_name"
        )
        spark.sql(
            f"ALTER TABLE {CATALOG}.evolve.from_spark ALTER COLUMN id TYPE BIGINT"
        )
        spark.sql(
            f"INSERT INTO {CATALOG}.evolve.from_spark VALUES (2, 'b', 'fresh')"
        )

        table = catalog.table("evolve.from_spark")
        rows = table.scan().read_all().sort_by("id")
        assert rows.column("id").to_pylist() == [1, 2]
        assert rows.column("id").type == pa.int64()
        assert rows.column("new_name").to_pylist() == ["a", "b"]
        assert rows.column("note").to_pylist() == [None, "fresh"]


class TestPropertiesAndFormats:
    """Table properties, the data format key, and mixed-format tables."""

    def test_yggdryl_properties_show_in_spark_and_back(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "props.shared", pa.table({"id": pa.array([1], pa.int64())})
        )
        # `owner` would be a poor probe: Spark's SparkCatalog reserves it as
        # catalog ownership and filters it from SHOW TBLPROPERTIES, so a
        # free-form key stands in for the round trip.
        table.update_properties(
            {"commit.retry.num-retries": "7", "written-by": "yggdryl"}
        )

        shown = dict(
            spark_rows(spark, f"SHOW TBLPROPERTIES {CATALOG}.props.shared")
        )
        assert shown["commit.retry.num-retries"] == "7"
        assert shown["written-by"] == "yggdryl"

        spark.sql(
            f"ALTER TABLE {CATALOG}.props.shared "
            "SET TBLPROPERTIES ('write.format.default' = 'avro')"
        )
        reopened = catalog.table("props.shared")
        assert reopened.options().data_mime_type == MimeType.AVRO
        reopened.append(pa.table({"id": pa.array([2], pa.int64())}))
        formats = {file.mime_type for file, _ in reopened.data_files()}
        assert formats == {MimeType.PARQUET, MimeType.AVRO}
        assert spark_rows(
            spark, f"SELECT id FROM {CATALOG}.props.shared ORDER BY id"
        ) == [(1,), (2,)]

    def test_spark_reads_a_mixed_format_table_yggdryl_wrote(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "props.mixed", pa.table({"id": pa.array([1], pa.int64())})
        )
        table.append(
            pa.table({"id": pa.array([2], pa.int64())}),
            options=IcebergOptions(data_mime_type="avro"),
        )
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.PARQUET, MimeType.AVRO}

        got = spark_rows(
            spark, f"SELECT id FROM {CATALOG}.props.mixed ORDER BY id"
        )
        assert got == [(1,), (2,)]
        by_format = spark_rows(
            spark,
            f"SELECT file_format, record_count FROM {CATALOG}.props.mixed.files",
        )
        assert sorted(row[0] for row in by_format) == ["AVRO", "PARQUET"]

    def test_yggdryl_reads_avro_files_spark_wrote(
        self, spark: Any, catalog: Catalog
    ) -> None:
        spark.sql(
            f"CREATE TABLE {CATALOG}.props.spark_avro (id BIGINT, s STRING) "
            "USING iceberg TBLPROPERTIES ('write.format.default' = 'avro')"
        )
        spark.sql(
            f"INSERT INTO {CATALOG}.props.spark_avro VALUES (1, 'a'), (2, null)"
        )
        table = catalog.table("props.spark_avro")
        assert {file.mime_type for file, _ in table.data_files()} == {MimeType.AVRO}
        assert scan_rows(table) == [(1, "a"), (2, None)]


class TestCompaction:
    """A compacted table keeps its rows and stays readable by Spark."""

    def test_a_yggdryl_compaction_reads_back_in_spark(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "compact.t", pa.table({"id": pa.array([1], pa.int64())})
        )
        for start in (2, 3, 4):
            table.append(pa.table({"id": pa.array([start], pa.int64())}))

        result = table.compact()
        assert result.files_before >= 2
        assert result.files_after < result.files_before

        assert spark_rows(
            spark, f"SELECT id FROM {CATALOG}.compact.t ORDER BY id"
        ) == [(1,), (2,), (3,), (4,)]
        operations = [
            row[0]
            for row in spark_rows(
                spark, f"SELECT operation FROM {CATALOG}.compact.t.snapshots"
            )
        ]
        assert "replace" in operations


class TestManifestsAndStatistics:
    """Spark's metadata tables decode yggdryl's manifests and statistics."""

    def test_the_files_metadata_table_reads_yggdryl_statistics(
        self, spark: Any, catalog: Catalog
    ) -> None:
        rows = pa.table(
            {
                "id": pa.array([5, 1, 9, None], pa.int64()),
                "s": pa.array(["m", "a", "z", None], pa.string()),
            }
        )
        catalog.append("stats.t", rows)

        [(count, nulls, lower, upper)] = spark_rows(
            spark,
            "SELECT record_count, "
            "  readable_metrics.id.null_value_count, "
            "  readable_metrics.id.lower_bound, "
            "  readable_metrics.id.upper_bound "
            f"FROM {CATALOG}.stats.t.files",
        )
        assert count == 4
        assert nulls == 1
        assert lower == 1
        assert upper == 9

        # The same numbers through yggdryl's own view of the manifest.
        [(file, _spec)] = catalog.table("stats.t").data_files()
        assert file.record_count == 4
        field_id = 1
        assert file.null_value_counts[field_id] == 1

    def test_the_manifests_and_snapshots_tables_decode(
        self, spark: Any, catalog: Catalog
    ) -> None:
        table = catalog.append(
            "stats.manifests", pa.table({"id": pa.array([1], pa.int64())})
        )
        table.append(pa.table({"id": pa.array([2], pa.int64())}))

        manifests = spark_rows(
            spark,
            f"SELECT added_data_files_count FROM {CATALOG}.stats.manifests.manifests",
        )
        assert [row[0] for row in manifests] == [1, 1]
        snapshots = spark_rows(
            spark,
            f"SELECT summary['added-records'] FROM {CATALOG}.stats.manifests.snapshots",
        )
        assert sorted(row[0] for row in snapshots) == ["1", "1"]


class TestSparkKeepsWriting:
    """A yggdryl table is not a dead end: Spark appends, then yggdryl does."""

    def test_the_two_implementations_alternate_writes(
        self, spark: Any, catalog: Catalog
    ) -> None:
        catalog.append(
            "alternate.t", pa.table({"id": pa.array([1], pa.int64())})
        )
        spark.sql(f"INSERT INTO {CATALOG}.alternate.t VALUES (2)")
        table = catalog.table("alternate.t")
        table.append(pa.table({"id": pa.array([3], pa.int64())}))
        spark.sql(f"INSERT INTO {CATALOG}.alternate.t VALUES (4)")

        assert spark_rows(
            spark, f"SELECT id FROM {CATALOG}.alternate.t ORDER BY id"
        ) == [(1,), (2,), (3,), (4,)]
        assert scan_rows(catalog.table("alternate.t"), "id") == [
            (1,),
            (2,),
            (3,),
            (4,),
        ]
