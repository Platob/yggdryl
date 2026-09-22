"""An Iceberg table, built on the handle every other test already uses.

Filtered reads, scoped writes and the maintenance a table needs to stay one.
The exchange with Apache Spark is its own target, `test_spark_interop.py`,
because the module-level skip it needs for a missing JVM would take this
suite with it.
"""

from __future__ import annotations

import copy
import json
import pathlib
import pickle
import time

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase, MimeType
from yggdryl.iceberg import (
    Catalog,
    Compaction,
    DataFile,
    IcebergOptions,
    ManifestFile,
    PartitionField,
    PartitionSpec,
    ScanPlan,
    Snapshot,
    Table,
    assign_field_ids,
    can_promote,
    schema_from_json,
    schema_into_json,
)

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)

NARROW = pa.schema(
    [
        pa.field("id", pa.int32(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def _rows(start: int = 1) -> pa.RecordBatch:
    """Three rows across two venues and the absence of one."""
    return pa.record_batch(
        {"id": [start, start + 1, start + 2], "venue": ["XNAS", "XNYS", None]},
        schema=SCHEMA,
    )


@pytest.fixture
def numbered() -> object:
    """The shared schema, with the field identifiers Iceberg resolves by."""
    return assign_field_ids(SCHEMA)


@pytest.fixture
def table(tmp_path: pathlib.Path, numbered: object) -> Table:
    """A partitioned table with nothing written to it yet."""
    return Table.create(IOBase(tmp_path / "trades"), numbered, ["venue"])


@pytest.fixture
def narrow(tmp_path: pathlib.Path) -> Table:
    """An unpartitioned table holding one row under a 32-bit id."""
    table = Table.create(IOBase(tmp_path / "narrow"), assign_field_ids(NARROW))
    table.append(pa.record_batch({"id": [1], "venue": ["XNAS"]}, schema=NARROW))
    return table


class TestSchemasCarryIdentifiers:
    """Iceberg resolves a column by identifier, not by position."""

    def test_numbering_a_pyarrow_schema_returns_a_native_root(self) -> None:
        numbered = assign_field_ids(SCHEMA)

        assert numbered.name == "row"
        assert [child.parquet_field_id for child in numbered.dtype] == [1, 2]
        # The input is untouched: the numbered schema is a new value.
        assert SCHEMA.field("id").metadata is None

    def test_numbering_starts_where_it_is_told_to(self) -> None:
        numbered = assign_field_ids(SCHEMA, 10)

        assert [child.parquet_field_id for child in numbered.dtype] == [10, 11]

    def test_a_root_that_is_not_a_non_null_struct_is_refused(
        self, tmp_path: pathlib.Path
    ) -> None:
        with pytest.raises(ValueError):
            Table.create(IOBase(tmp_path / "scalar"), "row:int64 not null")

    def test_a_schema_document_round_trips(self) -> None:
        document = {
            "type": "struct",
            "schema-id": 0,
            "fields": [
                {"id": 1, "name": "id", "required": True, "type": "long"},
                {"id": 2, "name": "venue", "required": False, "type": "string"},
            ],
        }

        schema = schema_from_json("row", document)
        assert schema.dtype.kind == "nested"
        assert not schema.nullable
        # `required` inverts into nullability, and `id` becomes PARQUET:field_id.
        assert not schema.dtype[0].nullable
        assert schema.dtype[1].nullable
        assert [child.parquet_field_id for child in schema.dtype] == [1, 2]

        assert schema_into_json(schema) == document

    def test_a_document_that_is_not_a_schema_is_refused(self) -> None:
        with pytest.raises(ValueError):
            schema_from_json("row", {"type": "long"})


class TestCreatingAndOpening:
    """A table is a folder, and it is found without a catalog."""

    def test_a_new_table_has_a_schema_and_no_snapshot(self, table: Table) -> None:
        assert table.format_version == 2
        assert table.version == 1
        assert table.current_snapshot is None
        assert table.schemas != []
        assert [field.name for field in table.spec.fields] == ["venue"]
        assert table.spec.fields[0].transform == "identity"

        # An empty table reads as no rows rather than as a failure.
        assert table.scan().read_all().num_rows == 0

    def test_immutable_metadata_views_use_complete_native_scalar_protocols(
        self, table: Table
    ) -> None:
        table.append(_rows())
        spec = table.spec
        field = spec.fields[0]
        snapshot = table.current_snapshot
        assert snapshot is not None
        manifest = table.manifests()[0]
        data_file, file_spec = table.data_files()[0]

        assert snapshot.encryption_key_id is None
        assert snapshot.first_row_id is None
        assert snapshot.added_rows is None
        assert snapshot.manifests is None
        assert manifest.content == "data"
        assert manifest.min_sequence_number == manifest.sequence_number
        assert isinstance(manifest.partitions, tuple)
        assert manifest.key_metadata is None
        assert manifest.first_row_id is None
        assert data_file.key_metadata is None
        assert data_file.equality_ids is None
        assert data_file.first_row_id is None
        assert data_file.referenced_data_file is None
        assert data_file.content_offset is None
        assert data_file.content_size_in_bytes is None
        assert data_file.nan_value_counts == {}
        assert data_file.mime_type == MimeType.PARQUET

        enriched_snapshot = Snapshot.from_json(
            {
                "snapshot-id": 9,
                "sequence-number": 3,
                "timestamp-ms": 1_000,
                "manifest-list": "file:///metadata/snap.avro",
                "summary": {"operation": "append"},
                "schema-id": 0,
                "key-id": "kms-key",
                "first-row-id": 40,
                "added-rows": 2,
            }
        )
        assert enriched_snapshot.encryption_key_id == "kms-key"
        assert enriched_snapshot.first_row_id == 40
        assert enriched_snapshot.added_rows == 2
        assert pickle.loads(pickle.dumps(enriched_snapshot)) == enriched_snapshot

        v1_snapshot = Snapshot.from_json(
            {
                "snapshot-id": 7,
                "timestamp-ms": 900,
                "manifests": ["file:///metadata/a.avro", "file:///metadata/b.avro"],
                "summary": {"operation": "append"},
                "schema-id": 0,
            }
        )
        assert v1_snapshot.manifest_list == ""
        assert v1_snapshot.manifests == (
            "file:///metadata/a.avro",
            "file:///metadata/b.avro",
        )
        assert v1_snapshot.into_json(1)["manifests"] == list(v1_snapshot.manifests)
        assert eval(repr(v1_snapshot), {"Snapshot": Snapshot}) == v1_snapshot
        assert pickle.loads(pickle.dumps(v1_snapshot)) == v1_snapshot

        rebuild, (state,) = data_file.__reduce__()
        state.update(
            key_metadata=b"key",
            nan_value_counts=[(2, 1)],
            equality_ids=[1, 2],
            first_row_id=40,
            referenced_data_file="file:///data/base.parquet",
            content_offset=8,
            content_size_in_bytes=16,
        )
        enriched_file = rebuild(state)
        assert enriched_file.key_metadata == b"key"
        assert enriched_file.nan_value_counts == {2: 1}
        assert enriched_file.equality_ids == [1, 2]
        assert enriched_file.first_row_id == 40
        assert enriched_file.referenced_data_file == "file:///data/base.parquet"
        assert enriched_file.content_offset == 8
        assert enriched_file.content_size_in_bytes == 16
        assert pickle.loads(pickle.dumps(enriched_file)) == enriched_file

        rebuild_manifest, (manifest_state,) = manifest.__reduce__()
        manifest_state["key_metadata"] = b"manifest-key"
        manifest_state["partitions"] = ((True, False, b"a", b"z"),)
        manifest_state["first_row_id"] = 40
        for count_name in (
            "added_files_count",
            "existing_files_count",
            "deleted_files_count",
            "added_rows_count",
            "existing_rows_count",
            "deleted_rows_count",
        ):
            manifest_state[count_name] = None
        enriched_manifest = rebuild_manifest(manifest_state)
        assert enriched_manifest.key_metadata == b"manifest-key"
        assert enriched_manifest.partitions == ((True, False, b"a", b"z"),)
        assert enriched_manifest.first_row_id == 40
        assert enriched_manifest.added_files_count is None
        assert enriched_manifest.existing_files_count is None
        assert enriched_manifest.deleted_files_count is None
        assert enriched_manifest.added_rows_count is None
        assert enriched_manifest.existing_rows_count is None
        assert enriched_manifest.deleted_rows_count is None
        assert pickle.loads(pickle.dumps(enriched_manifest)) == enriched_manifest

        values = [spec, field, snapshot, manifest, data_file]
        namespaces = {
            "PartitionSpec": PartitionSpec,
            "PartitionField": PartitionField,
            "Snapshot": Snapshot,
            "ManifestFile": ManifestFile,
            "DataFile": DataFile,
        }
        for value in values:
            copied = copy.copy(value)
            deep = copy.deepcopy(value)
            restored = pickle.loads(pickle.dumps(value))
            represented = eval(repr(value), namespaces)
            assert copied == value
            assert deep == value
            assert restored == value
            assert represented == value
            assert copied.stable_hash() == value.stable_hash()
            assert hash(copied) == hash(value)
            assert {value: "held"}[copied] == "held"
            assert value <= copied and value >= copied
            assert value != object()

        assert file_spec == spec
        assert PartitionSpec.from_json(spec.into_json()) == spec
        assert PartitionField.from_json(field.into_json()) == field
        assert Snapshot.from_json(snapshot.into_json()) == snapshot

        unknown = PartitionField.from_json(
            {
                "name": "venue_opaque",
                "transform": "unknown",
                "source-id": 2,
                "field-id": 1001,
            }
        )
        assert unknown.transform == "unknown"
        assert PartitionField.from_json(
            {
                "name": "venue_bucket",
                "transform": "bucket[4294967295]",
                "source-id": 2,
                "field-id": 1002,
            }
        ).transform == "bucket[4294967295]"

    def test_create_numbers_a_plain_pyarrow_schema_itself(
        self, tmp_path: pathlib.Path
    ) -> None:
        """A schema without ids is numbered at create, partitioning included."""
        table = Table.create(IOBase(tmp_path / "plain"), SCHEMA, ["venue"])

        ids = [child.parquet_field_id for child in table.schema.dtype]
        assert ids == [1, 2]
        assert [field.name for field in table.spec.fields] == ["venue"]

        table.append(_rows())
        assert table.scan().read_all().num_rows == 3

    def test_the_metadata_document_is_where_a_reader_looks(
        self, table: Table, tmp_path: pathlib.Path
    ) -> None:
        assert table.metadata_file_name == "v1.metadata.json"
        assert table.metadata_location.endswith(f"metadata/{table.metadata_file_name}")

        metadata = IOBase(tmp_path / "trades" / "metadata")
        assert {entry.name for entry in metadata} == {
            table.metadata_file_name,
            "version-hint.text",
        }
        assert metadata.joinpath("version-hint.text").read_text() == "1"

    def test_open_finds_the_current_document(
        self, table: Table, tmp_path: pathlib.Path
    ) -> None:
        table.append(_rows())

        reopened = Table.open(IOBase(tmp_path / "trades"))
        assert reopened.version == table.version
        assert reopened.table_uuid == table.table_uuid
        assert reopened.scan().read_all().num_rows == 3

    def test_open_or_create_does_not_write_over_a_table(
        self, table: Table, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        table.append(_rows())

        same = Table.open_or_create(IOBase(tmp_path / "trades"), numbered, ["venue"])
        assert same.scan().read_all().num_rows == 3

    def test_a_buffer_is_not_a_table(self, numbered: object) -> None:
        # A table is a folder, and an in-memory buffer names no folder: its
        # address is an identity, and no backend holds a `mem:` location.
        with pytest.raises(ValueError, match='"mem" does not support'):
            Table.create(IOBase.from_bytes(), numbered)


class TestCommits:
    """Each commit writes data files, a manifest, a list, and a document."""

    def test_appending_keeps_what_is_already_stored(self, table: Table) -> None:
        table.append(_rows())
        table.append(_rows(4))

        assert table.scan().read_all().num_rows == 6
        assert table.version == 3
        assert len(table.snapshots) == 2
        assert table.current_snapshot is not None
        assert table.current_snapshot.operation == "append"
        assert (
            table.current_snapshot.parent_snapshot_id == table.snapshots[0].snapshot_id
        )

    def test_overwriting_replaces_every_row(self, table: Table) -> None:
        table.append(_rows())
        table.overwrite(_rows(10))

        rows = table.scan().read_all()
        assert rows.column("id").to_pylist() == [10, 11, 12]
        assert table.current_snapshot is not None
        assert table.current_snapshot.operation == "overwrite"
        # The previous snapshot is retained, which is what makes this reversible.
        assert len(table.snapshots) == 2

    def test_a_commit_takes_the_rows_the_record_surface_takes(
        self, table: Table
    ) -> None:
        # The same inference point `append_records` uses, with the table's
        # stored schema as the declared field - so plain rows need no Arrow
        # holder and no schema of their own.
        table.append([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": None}])
        table.overwrite_where(None, [{"id": 3, "venue": "XLON"}])

        rows = table.scan().read_all()
        assert rows.column("id").to_pylist() == [3]

        import pandas
        import polars

        table.append(pandas.DataFrame({"id": [4], "venue": ["XPAR"]}))
        table.append(polars.DataFrame({"id": [5], "venue": ["XAMS"]}).lazy())
        assert table.scan().read_all().column("id").to_pylist() == [3, 4, 5]

        with pytest.raises(TypeError, match="expected rows"):
            table.append(12)

        # A table that declared its schema can be emptied by writing no rows,
        # which is how JavaScript already spelled the same delete.
        table.overwrite([])
        assert table.scan().read_all().num_rows == 0

    def test_a_named_write_types_rows_against_the_table_it_lands_in(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")

        # Nothing declares a schema on the first write, so the rows do.
        created = catalog.append("nyc.trades", [{"id": 1, "venue": "XNAS"}])
        assert created.scan().read_all().num_rows == 1

        # The second write types against the schema the first one created,
        # through either spelling of the same view.
        catalog.append("nyc.trades", [{"id": 2, "venue": "XNYS"}])
        catalog.tables.append("nyc.trades", [{"id": 3, "venue": None}])
        assert catalog.table("nyc.trades").scan().read_all().column(
            "id"
        ).to_pylist() == [1, 2, 3]

    def test_a_commit_takes_anything_pyarrow_streams(self, table: Table) -> None:
        table.append(pa.Table.from_batches([_rows()]))
        table.append(pa.RecordBatchReader.from_batches(SCHEMA, [_rows(4)]))

        assert table.scan().read_all().num_rows == 6


class TestPartitioning:
    """The manifest is the authority on a partition value; the path is layout."""

    def test_one_file_per_partition_lands_in_a_named_directory(
        self, table: Table
    ) -> None:
        table.append(_rows())

        files = table.data_files()
        assert len(files) == 3
        assert sorted(file.partition[0] for file, _ in files if file.partition[0]) == [
            "XNAS",
            "XNYS",
        ]
        assert [spec.fields[0].name for _, spec in files] == ["venue"] * 3
        assert all(file.mime_type == MimeType.PARQUET for file, _ in files)
        assert {file.record_count for file, _ in files} == {1}

    def test_a_null_partition_is_the_absence_and_not_the_word(
        self, table: Table
    ) -> None:
        table.append(_rows())

        absent = [file for file, _ in table.data_files() if file.partition[0] is None]
        assert len(absent) == 1
        # The directory spells it `null`, and only the manifest can say which.
        assert "venue=null" in absent[0].path

        rows = table.scan().read_all()
        assert rows.column("venue").to_pylist() == ["XNAS", "XNYS", None]

    def test_a_data_file_is_a_child_of_the_table(
        self, table: Table, tmp_path: pathlib.Path
    ) -> None:
        table.append(_rows())
        file, _ = table.data_files()[0]

        assert file.path.startswith(table.location)
        assert file.file_size_in_bytes > 0
        assert file.value_counts != {}
        assert file.content == 0, "rows, not deletes"

    def test_a_bound_travels_as_the_encoded_value(self, table: Table) -> None:
        table.append(_rows())
        file, _ = table.data_files()[0]

        # A bound is the encoded value Iceberg stores, keyed by field id, which
        # is what lets a planner skip a file without opening it.
        assert isinstance(file.lower_bounds[1], bytes)
        assert file.lower_bounds[1] == file.upper_bounds[1]
        assert file.null_value_counts[1] == 0

    def test_the_manifest_describes_what_the_commit_added(self, table: Table) -> None:
        table.append(_rows())

        manifests = table.manifests()
        assert len(manifests) == 1
        assert manifests[0].is_data()
        assert manifests[0].added_files_count == 3
        assert manifests[0].added_rows_count == 3
        assert manifests[0].path.endswith(".avro")

    def test_an_unpartitioned_table_writes_one_file(
        self, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        table = Table.create(IOBase(tmp_path / "flat"), numbered)

        assert table.spec.is_unpartitioned()
        table.append(_rows())
        assert len(table.data_files()) == 1

    def test_a_spec_may_be_built_before_the_table(self, numbered: object) -> None:
        spec = PartitionSpec.identity(numbered, ["venue"], spec_id=0)

        assert len(spec) == 1
        assert spec.fields[0].source_id == 2
        assert spec.fields[0].field_id == 1000
        assert not spec.is_unpartitioned()


class TestScans:
    """A scan pushes columns down to each file and casts to the scan root."""

    def test_a_projected_scan_reads_the_columns_it_names(self, table: Table) -> None:
        table.append(_rows())
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        projected = table.scan(wanted).read_all()
        assert projected.column_names == ["id"]
        assert projected.num_rows == 3

    def test_an_evolved_schema_reads_as_one_shape(self, table: Table) -> None:
        table.append(_rows())

        widened = assign_field_ids(
            pa.schema(
                [
                    pa.field("id", pa.int64(), nullable=False),
                    pa.field("venue", pa.string()),
                    pa.field("price", pa.float64()),
                ]
            )
        )
        schema_id = table.evolve_schema(widened)

        assert schema_id == 1
        assert len(table.schemas) == 2
        rows = table.scan().read_all()
        # The files predate the column, so it reads as null rather than failing.
        assert rows.column_names == ["id", "venue", "price"]
        assert rows.column("price").to_pylist() == [None, None, None]

        table.append(
            pa.record_batch(
                {"id": [4], "venue": ["XNAS"], "price": [1.5]},
                schema=pa.schema(
                    [
                        pa.field("id", pa.int64(), nullable=False),
                        pa.field("venue", pa.string()),
                        pa.field("price", pa.float64()),
                    ]
                ),
            )
        )
        assert table.scan().read_all().column("price").to_pylist() == [
            None,
            None,
            None,
            1.5,
        ]

    def test_a_scan_is_a_reader_that_knows_its_schema_first(
        self, table: Table
    ) -> None:
        table.append(_rows())

        scan = table.scan()
        assert isinstance(scan, pa.RecordBatchReader)
        assert scan.schema.names == ["id", "venue"]
        assert scan.read_next_batch().num_rows == 1


class TestCatalog:
    """A catalog is a warehouse folder, and a dotted name is nested folders."""

    def test_the_views_chain_a_catalog_to_namespaces_to_tables(
        self, tmp_path: pathlib.Path
    ) -> None:
        import pyarrow as pa

        catalog = Catalog(tmp_path / "warehouse")

        # The views are lazy: constructing them touches nothing, and an empty
        # warehouse answers empty rather than failing.
        assert len(catalog.namespaces) == 0
        assert "analytics" not in catalog.namespaces
        assert not (tmp_path / "warehouse").exists()

        analytics = catalog.namespaces.open_or_create("analytics")
        assert analytics.name == "analytics"
        assert "analytics" in catalog.namespaces
        assert list(catalog.namespaces) == ["analytics"]
        assert len(catalog.namespaces) == 1

        # open_or_create gets or creates; doing it again is the same table.
        schema = Field(
            "row",
            DataType.from_fields(
                [Field("id", "int64", nullable=False), Field("venue", "string")]
            ),
            nullable=False,
        )
        first = analytics.tables.open_or_create("trades", schema)
        same = analytics.tables.open_or_create("trades", schema)
        assert same.table_uuid == first.table_uuid
        assert "trades" in analytics.tables
        assert list(analytics.tables) == ["trades"]
        assert len(analytics.tables) == 1

        # Indexing opens the table; a missing one is a KeyError, as a map
        # spells absence - carrying the native message unchanged.
        table = catalog.namespaces["analytics"].tables["trades"]
        table.append(pa.table({"id": [1, 2], "venue": ["XNAS", None]}))
        chained = catalog.namespaces["analytics"].tables["trades"]
        assert chained.scan().read_all().num_rows == 2
        with pytest.raises(KeyError, match="expected a table at .*absent.*, got nothing"):
            catalog.namespaces["analytics"].tables["absent"]
        with pytest.raises(
            KeyError, match="expected a namespace at .*missing.*, got nothing"
        ):
            catalog.namespaces["missing"]

        # The write conveniences on the view create on first write, from the
        # rows' own schema, and two views observe each other's writes.
        tables = analytics.tables
        tables.overwrite("quotes", pa.table({"symbol": ["AAPL"], "price": [12.5]}))
        assert sorted(analytics.tables) == ["quotes", "trades"]
        assert sorted(tables) == ["quotes", "trades"]

    def test_namespaces_cascade_and_create_is_strict(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")

        nyc = catalog.namespaces.create("nyc")
        yellow = nyc.namespaces.create("yellow")
        assert yellow.name == "nyc.yellow"
        assert list(nyc.namespaces) == ["yellow"]

        # Creating what exists is refused by name; a table is not a namespace.
        with pytest.raises(ValueError, match="expected to create a namespace"):
            catalog.namespaces.create("nyc")
        yellow.tables.create("taxis", SCHEMA)
        assert "taxis" not in yellow.namespaces
        assert catalog.namespaces["nyc"].namespaces["yellow"].tables[
            "taxis"
        ].scan().read_all().num_rows == 0

    def test_a_pyarrow_append_creates_a_partitioned_table_on_first_write(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")
        assert catalog.warehouse.name == "warehouse"
        assert list(catalog.namespaces) == []
        assert "nyc.taxis" not in catalog.tables

        # The schema's own marks say which columns the layout spells out, and
        # they ride the Arrow fields' metadata into the very first append.
        marked = Field(
            "row",
            DataType.from_fields(
                [
                    Field("id", "int64", nullable=False),
                    Field("venue", "string"),
                ]
            ),
            nullable=False,
        ).with_partition_fields(["venue"])
        columns = pa.schema([child.into_arrow() for child in marked.dtype])
        rows = pa.table(
            {"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=columns
        )

        table = catalog.append("nyc.taxis", rows)
        assert "nyc.taxis" in catalog.tables
        assert list(catalog.namespaces) == ["nyc"]
        assert list(catalog.namespaces["nyc"].tables) == ["taxis"]

        # The schema was inferred from the reader and numbered, and the marked
        # column became the identity spec.
        assert [child.parquet_field_id for child in table.schema.dtype] == [1, 2]
        assert [field.name for field in table.spec.fields] == ["venue"]
        assert table.spec.fields[0].transform == "identity"
        assert table.scan().read_all().num_rows == 3

        # Appending again through the catalog keeps what is stored, and the
        # name opens the same table it created.
        assert catalog.append("nyc.taxis", rows).scan().read_all().num_rows == 6
        assert catalog.table("nyc.taxis").table_uuid == table.table_uuid

    def test_tables_create_takes_an_iterable_of_fields(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(IOBase(tmp_path / "warehouse"))

        table = catalog.tables.create(
            "ns.trades", [Field("id", "int64", nullable=False)]
        )
        assert list(catalog.namespace("ns").tables) == ["trades"]
        assert table.spec.is_unpartitioned()

        with pytest.raises(ValueError, match="expected to create a table"):
            catalog.tables.create("ns.trades", [Field("id", "int64", nullable=False)])
        # An existing table is opened as it is; the schema describes only the
        # table the call would create.
        same = catalog.tables.open_or_create("ns.trades", SCHEMA)
        assert same.table_uuid == table.table_uuid

    def test_overwrite_replaces_and_a_missing_table_is_named(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")

        catalog.overwrite("flat", pa.Table.from_batches([_rows()]))
        replaced = catalog.overwrite("flat", pa.Table.from_batches([_rows(10)]))
        assert replaced.scan().read_all().column("id").to_pylist() == [10, 11, 12]
        # The previous snapshot is retained, which is what makes it reversible.
        assert len(replaced.snapshots) == 2

        with pytest.raises(ValueError, match="expected a table"):
            catalog.table("absent")
        with pytest.raises(ValueError, match="path separators"):
            catalog.tables.create("a/b", SCHEMA)

    def test_a_dotted_create_into_an_empty_warehouse_is_one_call(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")

        # The namespace view exists before its folder does, so the chain
        # writes into an empty warehouse: the table's first metadata document
        # is what brings every ancestor namespace into being.
        created = catalog.namespace("sales.eu").tables.create("orders", SCHEMA)

        # The same table, every spelling: the catalog's dotted entry point,
        # the root tables view, and the strict indexed cascade.
        assert catalog.table("sales.eu.orders").table_uuid == created.table_uuid
        assert catalog.tables["sales.eu.orders"].table_uuid == created.table_uuid
        assert "sales.eu.orders" in catalog.tables
        chained = catalog.namespaces["sales.eu"].tables["orders"]
        assert chained.table_uuid == created.table_uuid

        # The root tables view lists tables directly under the warehouse, so
        # a table two namespaces down is reached by name, not by listing.
        assert list(catalog.tables) == []

    def test_the_views_speak_every_mapping_spelling(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")
        sales = catalog.namespaces.create("sales")
        sales.tables.create("orders", SCHEMA)
        sales.tables.create("returns", SCHEMA)
        sales.namespaces.create("eu")

        # keys, values, items, iteration, membership, and len - the same
        # mapping dunders Field metadata answers.
        assert list(catalog.namespaces.keys()) == ["sales"]
        assert [view.name for view in catalog.namespaces.values()] == ["sales"]
        assert [
            (name, view.name) for name, view in catalog.namespaces.items()
        ] == [("sales", "sales")]
        assert "sales" in catalog.namespaces
        assert len(catalog.namespaces) == 1

        assert list(sales.tables.keys()) == ["orders", "returns"]
        assert [table.location for table in sales.tables.values()] == [
            sales.tables["orders"].location,
            sales.tables["returns"].location,
        ]
        assert [name for name, _ in sales.tables.items()] == ["orders", "returns"]
        assert "orders" in sales.tables
        assert len(sales.tables) == 2

    def test_values_opens_one_table_per_next(self, tmp_path: pathlib.Path) -> None:
        catalog = Catalog(tmp_path / "warehouse")
        catalog.tables.create("ns.aaa", SCHEMA)
        # A sibling that lists as a table but cannot open: its current
        # metadata document is not table metadata at all.
        poisoned = tmp_path / "warehouse" / "ns" / "zzz" / "metadata"
        poisoned.mkdir(parents=True)
        (poisoned / "v1.metadata.json").write_bytes(b"{}")

        # values() is lazy: taking the first value opens exactly that table,
        # so the poisoned sibling is never touched - draining raises at it.
        values = catalog.namespace("ns").tables.values()
        assert next(values).root.name == "aaa"
        with pytest.raises((ValueError, KeyError)):
            list(values)
        items = catalog.namespace("ns").tables.items()
        name, table = next(items)
        assert name == "aaa"
        assert table.scan().read_all().num_rows == 0

    def test_catalog_and_namespace_carry_properties(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")

        # Absent means empty, and a call given nothing writes nothing.
        assert catalog.properties == {}
        catalog.update_properties()
        assert not (tmp_path / "warehouse").exists()

        catalog.update_properties({"owner": "finance"})
        assert catalog.properties == {"owner": "finance"}
        catalog.update_properties({"region": "eu"}, ["owner"])
        assert catalog.properties == {"region": "eu"}

        # The reserved prefix is refused with the core's own message.
        with pytest.raises(ValueError, match="reserved .*ICEBERG:"):
            catalog.update_properties({"ICEBERG:x": "1"})

        sales = catalog.namespaces.create("sales")
        assert sales.properties == {}
        sales.update_properties({"team": "emea"})
        assert sales.properties == {"team": "emea"}
        assert catalog.namespaces["sales"].properties == {"team": "emea"}
        with pytest.raises(ValueError, match="reserved .*ICEBERG:"):
            sales.update_properties({"ICEBERG:x": "1"})


class TestTimeTravel:
    """Every retained snapshot is a complete table, read by ordinary scans."""

    def test_scan_at_reads_the_snapshot_an_overwrite_replaced(
        self, table: Table
    ) -> None:
        table.append(_rows())
        assert table.current_snapshot is not None
        first = table.current_snapshot.snapshot_id
        table.overwrite(_rows(10))

        assert table.scan().read_all().column("id").to_pylist() == [10, 11, 12]
        old = table.scan_at(first).read_all()
        assert old.column("id").to_pylist() == [1, 2, 3]

        # Filters take the same (column, value) pairs a lake read takes, and
        # the schema keeps the columns it names.
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])
        filtered = table.scan_at(
            first, filters={"venue": "XNAS"}, schema=wanted
        ).read_all()
        assert filtered.column_names == ["id"]
        assert filtered.column("id").to_pylist() == [1]

    def test_a_snapshot_the_table_does_not_retain_is_named(
        self, table: Table
    ) -> None:
        table.append(_rows())

        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            table.scan_at(999)

    def test_a_v1_direct_manifest_snapshot_stays_readable(
        self, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        location = tmp_path / "v1"
        table = Table.create(IOBase(location), numbered, format_version=1)
        table.append(_rows())
        snapshot = table.current_snapshot
        assert snapshot is not None
        direct = [manifest.path for manifest in table.manifests()]

        metadata_path = location / "metadata" / table.metadata_file_name
        document = json.loads(metadata_path.read_text(encoding="utf-8"))
        document["snapshots"][0]["manifests"] = direct
        del document["snapshots"][0]["manifest-list"]
        metadata_path.write_text(json.dumps(document), encoding="utf-8")

        reopened = Table.open(IOBase(location))
        v1 = reopened.current_snapshot
        assert v1 is not None
        assert v1.manifest_list == ""
        assert v1.manifests == tuple(direct)
        assert reopened.scan().read_all().num_rows == 3

        reopened.append(_rows(10))
        assert reopened.scan().read_all().num_rows == 6
        assert reopened.scan_at(snapshot.snapshot_id).read_all().column(
            "id"
        ).to_pylist() == [1, 2, 3]
        retained = next(
            item for item in reopened.snapshots if item.snapshot_id == snapshot.snapshot_id
        )
        assert retained.manifests == tuple(direct)

    def test_snapshot_by_ref_follows_main(self, table: Table) -> None:
        table.append(_rows())
        current = table.current_snapshot
        assert current is not None

        assert table.snapshot_by_ref("main").snapshot_id == current.snapshot_id

    def test_a_ref_the_table_does_not_have_names_the_refs_it_has(
        self, table: Table
    ) -> None:
        table.append(_rows())

        with pytest.raises(ValueError, match=r"got \"nightly\"; it has \[main\]"):
            table.snapshot_by_ref("nightly")


class TestSchemaUpdates:
    """A column change is a new schema, recorded first and committed once."""

    def test_a_with_block_commits_the_recorded_operations_as_one_document(
        self, narrow: Table
    ) -> None:
        first = narrow.current_snapshot
        assert first is not None
        before = narrow.version

        with narrow.update_schema() as update:
            update.add_column("", "price: float64").update_type(
                "id", "int64"
            ).rename_column("venue", "market")

        # One metadata document, however many operations were recorded.
        assert narrow.version == before + 1
        children = list(narrow.schema.dtype)
        assert [child.name for child in children] == ["id", "market", "price"]
        # The widened type reads back, the renamed column keeps its
        # identifier, and the added column is numbered above every identifier
        # the table has ever assigned.
        assert children[0].dtype.id == "int64"
        assert [child.parquet_field_id for child in children] == [1, 2, 3]
        assert children[2].nullable

        rows = narrow.scan().read_all()
        assert rows.schema.field("id").type == pa.int64()
        assert rows.column("id").to_pylist() == [1]
        assert rows.column("price").to_pylist() == [None]
        # The pre-rename snapshot reads as the schema it was written under, so
        # the stored value is a time travel away under its pre-rename name.
        assert narrow.scan_at(first.snapshot_id).read_all().column(
            "venue"
        ).to_pylist() == ["XNAS"]

        # A row appended after the evolution carries the new shape.
        narrow.append(
            pa.record_batch(
                {"id": [2], "market": ["XNYS"], "price": [1.5]},
                schema=pa.schema(
                    [
                        pa.field("id", pa.int64(), nullable=False),
                        pa.field("market", pa.string()),
                        pa.field("price", pa.float64()),
                    ]
                ),
            )
        )
        assert narrow.scan().read_all().column("market").to_pylist()[-1] == "XNYS"

    def test_an_exception_discards_the_update(self, narrow: Table) -> None:
        before = narrow.version

        with pytest.raises(RuntimeError, match="stop"):
            with narrow.update_schema() as update:
                update.add_column("", "price: float64")
                raise RuntimeError("stop")

        assert narrow.version == before
        assert [child.name for child in narrow.schema.dtype] == ["id", "venue"]

    def test_an_update_that_records_nothing_commits_nothing(
        self, narrow: Table
    ) -> None:
        before = narrow.version

        with narrow.update_schema():
            pass

        assert narrow.version == before

    def test_an_illegal_promotion_is_refused_naming_both_sides(
        self, narrow: Table
    ) -> None:
        before = narrow.version

        with pytest.raises(
            ValueError, match="expected an Iceberg-legal promotion, got int32 to int16"
        ):
            with narrow.update_schema() as update:
                update.update_type("id", "int16")

        assert narrow.version == before

    def test_docs_and_nullability_evolve_too(self, narrow: Table) -> None:
        with narrow.update_schema() as update:
            update.update_doc("id", "row identifier").make_nullable("id")

        evolved = narrow.schema.dtype[0]
        assert evolved.nullable
        assert evolved.iceberg["doc"] == "row identifier"

    def test_a_dropped_column_retires_its_identifier(self, narrow: Table) -> None:
        with narrow.update_schema() as update:
            update.drop_column("venue").add_column("", "note: string")

        children = list(narrow.schema.dtype)
        assert [child.name for child in children] == ["id", "note"]
        # The added column is numbered above the dropped one, never as it.
        assert children[1].parquet_field_id == 3

    def test_a_spent_update_is_refused(self, narrow: Table) -> None:
        update = narrow.update_schema()
        update.add_column("", "price: float64")
        update.commit()

        with pytest.raises(ValueError, match="already committed or discarded"):
            update.commit()
        with pytest.raises(ValueError, match="already committed or discarded"):
            update.drop_column("price")


class TestProperties:
    """A property change is a metadata-only commit, and a no-op is free."""

    def test_update_properties_round_trips_and_reaches_the_write_target(
        self, table: Table
    ) -> None:
        assert table.target_file_size == 512 * 1024 * 1024
        before = table.version

        table.update_properties({"write.target-file-size-bytes": "1048576"})
        assert table.version == before + 1
        assert table.properties["write.target-file-size-bytes"] == "1048576"
        assert table.target_file_size == 1048576

        # A sequence of pairs spells the same thing, and updates land before
        # removes inside the one commit.
        table.update_properties(
            [("commit.retry.num-retries", "4")],
            ["write.target-file-size-bytes"],
        )
        assert table.version == before + 2
        assert "write.target-file-size-bytes" not in table.properties
        assert table.properties["commit.retry.num-retries"] == "4"
        assert table.target_file_size == 512 * 1024 * 1024

    def test_a_call_given_nothing_commits_nothing(self, table: Table) -> None:
        before = table.version

        table.update_properties()
        table.update_properties({}, [])

        assert table.version == before


class TestCompaction:
    """Compaction merges undersized files and reports what it rewrote."""

    def test_compact_merges_the_small_files_of_a_partition(
        self, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        table = Table.create(IOBase(tmp_path / "flat"), numbered)
        for start in (1, 4, 7):
            table.append(_rows(start))

        files = table.inspect_files().read_all()
        assert files.num_rows == 3
        recorded = sum(files.column("file_size_in_bytes").to_pylist())

        result = table.compact()
        assert isinstance(result, Compaction)
        assert result.files_before == 3
        assert result.files_after == 1
        assert result.bytes_rewritten == recorded
        same = copy.copy(result)
        assert same == result
        assert same.stable_hash() == result.stable_hash()
        assert hash(same) == hash(result)
        assert pickle.loads(pickle.dumps(result)) == result
        assert eval(repr(result), {"Compaction": Compaction}) == result
        assert result <= same and result >= same

        assert table.inspect_files().read_all().num_rows == 1
        assert table.scan().read_all().num_rows == 9
        assert table.current_snapshot is not None
        assert table.current_snapshot.operation == "replace"

        # The pre-compaction snapshot still reads exactly what it always read.
        previous = table.snapshots[-2]
        assert table.scan_at(previous.snapshot_id).read_all().num_rows == 9

    def test_a_table_with_nothing_to_do_commits_nothing(
        self, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        table = Table.create(IOBase(tmp_path / "flat"), numbered)
        table.append(_rows())
        version = table.version

        result = table.compact()

        assert (result.files_before, result.files_after, result.bytes_rewritten) == (
            0,
            0,
            0,
        )
        assert table.version == version


class TestInspection:
    """The table's own record renders as record batches."""

    def test_the_inspection_readers_use_pyiceberg_column_names(
        self, table: Table
    ) -> None:
        table.append(_rows())
        table.overwrite(_rows(10))

        history = table.inspect_history().read_all()
        assert history.column_names == [
            "made_current_at",
            "snapshot_id",
            "parent_id",
            "is_current_ancestor",
        ]
        assert history.num_rows == 2
        assert history.column("is_current_ancestor").to_pylist() == [True, True]

        snapshots = table.inspect_snapshots().read_all()
        assert snapshots.column_names == [
            "committed_at",
            "snapshot_id",
            "parent_id",
            "operation",
            "manifest_list",
            "summary",
        ]
        assert snapshots.column("operation").to_pylist() == ["append", "overwrite"]

        files = table.inspect_files().read_all()
        assert files.column_names == [
            "file_path",
            "file_format",
            "spec_id",
            "partition",
            "record_count",
            "file_size_in_bytes",
        ]
        assert files.num_rows == 3
        assert sorted(files.column("partition").to_pylist()) == [
            "venue=XNAS",
            "venue=XNYS",
            "venue=null",
        ]


class TestPromotions:
    """`can_promote` is the one place the legal type promotions live."""

    def test_the_legal_promotions_pass(self) -> None:
        assert can_promote("int32", "int64") is None
        assert can_promote(pa.float32(), pa.float64()) is None
        assert can_promote("decimal128(10, 2)", "decimal128(20, 2)") is None
        assert can_promote(DataType("int32"), DataType("int32")) is None

    def test_everything_else_is_refused_naming_both_sides(self) -> None:
        with pytest.raises(
            ValueError, match="expected an Iceberg-legal promotion, got int64 to int32"
        ):
            can_promote("int64", "int32")
        with pytest.raises(ValueError, match="promotion"):
            can_promote("decimal128(10, 2)", "decimal128(10, 3)")
        with pytest.raises(ValueError, match="promotion"):
            can_promote(pa.int32(), pa.string())


class TestIcebergOptions:
    """Iceberg configuration crosses the boundary through one options value."""

    def test_the_options_value_records_only_what_was_set(self) -> None:
        options = IcebergOptions()
        assert options.commit_retries == 4
        assert options.commit_total_timeout_ms == 1_800_000
        assert options.target_file_size == 512 * 1024 * 1024
        assert options.data_mime_type == MimeType.PARQUET

        options = IcebergOptions(
            commit_retries=2, commit_total_timeout_ms=500, data_mime_type="avro"
        )
        assert options.commit_retries == 2
        assert options.commit_total_timeout_ms == 500
        assert options.data_mime_type == MimeType.AVRO
        options.target_file_size = 1024
        assert options.target_file_size == 1024
        with pytest.raises(TypeError, match="commit_retres"):
            IcebergOptions(commit_retres=2)

        # The write parallelism defaults to the read parallelism, resolves on
        # its own once set, and refuses zero naming its key.
        assert options.write_parallelism == options.read_parallelism
        options.read_parallelism = 3
        assert options.write_parallelism == 3
        options.write_parallelism = 5
        assert options.write_parallelism == 5
        assert IcebergOptions(write_parallelism=2).write_parallelism == 2
        with pytest.raises(ValueError, match=r"write\.parallelism"):
            IcebergOptions(write_parallelism=0)
        with pytest.raises(ValueError, match=r"write\.parallelism"):
            options.write_parallelism = 0
        assert options.write_parallelism == 5

        # The staging folder is unset until a layer speaks, reads back as the
        # text it was given, takes a path as well as a URL, and refuses a
        # remote folder naming its key.
        assert options.write_staging is None
        options.write_staging = "off"
        assert options.write_staging == "off"
        options.write_staging = pathlib.Path(__file__).resolve().parent / "stage"
        assert options.write_staging.startswith("file:")
        assert IcebergOptions(write_staging="off").write_staging == "off"
        with pytest.raises(ValueError, match=r"write\.staging"):
            IcebergOptions(write_staging="s3://trades/stage")
        with pytest.raises(ValueError, match=r"write\.staging"):
            options.write_staging = "s3://trades/stage"
        assert options.write_staging.startswith("file:")
        with pytest.raises(TypeError, match="write_staging"):
            options.write_staging = 7

    def test_puffin_is_a_native_format_but_not_a_table_data_writer(
        self, table: Table
    ) -> None:
        options = IcebergOptions(data_mime_type=MimeType.PUFFIN)
        assert options.data_mime_type == MimeType.PUFFIN
        with pytest.raises(
            ValueError,
            match=r"write\.format\.default.*application/vnd\.apache\.puffin",
        ):
            table.append(_rows(), options=options)
        assert table.current_snapshot is None

    def test_only_iceberg_data_mime_types_are_accepted_atomically(self) -> None:
        options = IcebergOptions(data_mime_type=MimeType.AVRO)
        with pytest.raises(ValueError, match=r"write\.format\.default.*application/json"):
            options.data_mime_type = MimeType.JSON
        assert options.data_mime_type == MimeType.AVRO
        with pytest.raises(TypeError, match="MimeType or MIME/extension string"):
            options.data_mime_type = object()
        assert options.data_mime_type == MimeType.AVRO

    def test_native_identity_hash_locks_every_setter_and_copies_unlock(self) -> None:
        options = IcebergOptions(commit_retries=4, data_mime_type="parquet")
        same = IcebergOptions(commit_retries=4, data_mime_type=MimeType.PARQUET)
        unset = IcebergOptions()

        # Explicit defaults and an unset option resolve to the same getters but
        # are different values because only the former shadows table properties.
        assert options.commit_retries == unset.commit_retries == 4
        assert options != unset
        assert options == same
        assert options.stable_hash() == same.stable_hash()
        assert hash(options) == hash(same)
        assert {options: "held"}[same] == "held"
        assert options <= same and options >= same

        for name, value in [
            ("commit_retries", 3),
            ("commit_min_backoff_ms", 1),
            ("commit_max_backoff_ms", 2),
            ("commit_total_timeout_ms", 3),
            ("target_file_size", 1024),
            ("read_parallelism", 1),
            ("read_parallel_min_files", 1),
            ("read_parallel_min_file_size", 1),
            ("write_parallelism", 1),
            ("write_staging", "off"),
            ("compact_after_commits", 1),
            ("data_mime_type", "avro"),
        ]:
            with pytest.raises(TypeError, match="hashed IcebergOptions"):
                setattr(options, name, value)

        for unlocked in [
            copy.copy(options),
            copy.deepcopy(options),
            pickle.loads(pickle.dumps(options)),
            eval(repr(options), {"IcebergOptions": IcebergOptions}),
        ]:
            assert unlocked == options
            unlocked.commit_retries = 2
            assert unlocked.commit_retries == 2

    def test_append_takes_one_explicit_options_value(self, table: Table) -> None:
        options = IcebergOptions(
            target_file_size=1024, commit_retries=1, data_mime_type="avro"
        )
        table.append(_rows(), options=options)
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.AVRO}
        assert options.data_mime_type == MimeType.AVRO

    def test_an_avro_append_scans_back_and_mixes_with_parquet(
        self, table: Table
    ) -> None:
        table.append(_rows(), options=IcebergOptions(data_mime_type="avro"))
        table.append(_rows(10))
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.AVRO, MimeType.PARQUET}
        got = table.scan().read_all().sort_by("id")
        assert got.column("id").to_pylist() == [1, 2, 3, 10, 11, 12]

    def test_the_record_options_type_is_refused_by_name(
        self, table: Table
    ) -> None:
        from yggdryl import RecordOptions

        with pytest.raises(TypeError, match="expected IcebergOptions"):
            table.append(_rows(), options=RecordOptions("application/vnd.apache.parquet"))

    def test_an_unknown_keyword_is_a_typeerror_naming_it(
        self, table: Table
    ) -> None:
        with pytest.raises(
            TypeError, match=r"append\(\) got an unexpected keyword argument"
        ):
            table.append(_rows(), data_fromat="avro")
        with pytest.raises(TypeError, match="parallelism"):
            table.scan(parallelism=2)

    def test_set_options_stores_a_handle_wide_override(
        self, table: Table
    ) -> None:
        table.set_options(IcebergOptions(data_mime_type="avro"))
        table.append(_rows())
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.AVRO}
        assert table.options().data_mime_type == MimeType.AVRO
        # A per-call options value wins for one call without disturbing the
        # stored override.
        table.append(
            _rows(10), options=IcebergOptions(data_mime_type="parquet")
        )
        assert {file.mime_type for file, _ in table.data_files()} == {
            MimeType.AVRO,
            MimeType.PARQUET,
        }
        assert table.options().data_mime_type == MimeType.AVRO

    def test_the_property_layer_sets_the_format_per_table(
        self, table: Table
    ) -> None:
        table.update_properties({"write.format.default": "avro"})
        table.append(_rows())
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.AVRO}
        # An unencodable format is a typed error naming the key, up front.
        table.update_properties({"write.format.default": "orc"})
        with pytest.raises(ValueError, match="write.format.default"):
            table.append(_rows(10))

    def test_the_catalog_write_paths_take_the_same_options_value(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")
        table = catalog.append(
            "sales.orders",
            _rows(),
            options=IcebergOptions(data_mime_type="avro"),
        )
        formats = {file.mime_type for file, _ in table.data_files()}
        assert formats == {MimeType.AVRO}

        tables = catalog.namespaces["sales"].tables
        table = tables.append(
            "orders", _rows(10), options=IcebergOptions(target_file_size=1024)
        )
        assert table.scan().read_all().num_rows == 6
        table = tables.overwrite(
            "orders", _rows(), options=IcebergOptions(data_mime_type="avro")
        )
        assert table.scan().read_all().num_rows == 3

SCHEMA_planning = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def _rows_planning(start: int = 1) -> pa.RecordBatch:
    """Three rows across two venues and the absence of one."""
    return pa.record_batch(
        {"id": [start, start + 1, start + 2], "venue": ["XNAS", "XNYS", None]},
        schema=SCHEMA_planning,
    )


def _row(key: int, venue: str | None) -> pa.RecordBatch:
    """One row, for the writes that are about a single key."""
    return pa.record_batch({"id": [key], "venue": [venue]}, schema=SCHEMA_planning)


@pytest.fixture
def numbered_planning() -> object:
    """The shared schema, with the field identifiers Iceberg resolves by."""
    return assign_field_ids(SCHEMA_planning)


@pytest.fixture
def table_planning(tmp_path: pathlib.Path, numbered_planning: object) -> Table:
    """A partitioned table with nothing written to it yet."""
    return Table.create(IOBase(tmp_path / "trades"), numbered_planning, ["venue"])


@pytest.fixture
def filled(table_planning: Table) -> Table:
    """Two commits over the same three partitions: six files, two manifests.

    Two commits rather than one is what makes the manifest counts mean
    something - a table whose whole history is one manifest cannot show that a
    manifest was skipped.
    """
    table_planning.append(_rows_planning())
    table_planning.append(_rows_planning(4))
    return table_planning


class TestFilteredScans:
    """A filter is answered by the plan for a partition column, by rows for the rest."""

    def test_a_filtered_scan_reads_only_the_matching_partition(
        self, filled: Table
    ) -> None:
        rows = filled.scan_where({"venue": "XNAS"}).read_all()

        assert rows.column("venue").to_pylist() == ["XNAS", "XNAS"]
        assert sorted(rows.column("id").to_pylist()) == [1, 4]
        # The plan and the read agree, which is what says the rows were skipped
        # rather than read and discarded.
        assert filled.plan({"venue": "XNAS"}).files_planned == 2

    def test_a_filter_naming_a_column_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        # A misspelled column must not read as "matches nothing": an empty
        # answer to a typo is the failure this refusal exists to catch.
        with pytest.raises(ValueError, match='got "market"'):
            filled.scan_where({"market": "XNAS"})

    def test_the_absence_of_a_partition_value_is_spelled_null(
        self, filled: Table
    ) -> None:
        rows = filled.scan_where({"venue": "null"}).read_all()

        assert rows.column("venue").to_pylist() == [None, None]
        assert sorted(rows.column("id").to_pylist()) == [3, 6]

    def test_a_filter_on_a_column_no_partition_carries_still_selects_rows(
        self, filled: Table
    ) -> None:
        # `id` is not a partition column, so statistics can only bound a file;
        # the rows that come back must still be exactly the matching rows.
        rows = filled.scan_where({"id": "5"}).read_all()

        assert rows.to_pydict() == {"id": [5], "venue": ["XNYS"]}

    def test_a_projection_rides_alongside_the_filter(self, filled: Table) -> None:
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        rows = filled.scan_where([("venue", "XNYS")], wanted).read_all()

        assert rows.column_names == ["id"]
        assert sorted(rows.column("id").to_pylist()) == [2, 5]

    def test_filtering_by_nothing_reads_the_whole_table(self, filled: Table) -> None:
        # The filters are optional, so `scan_where()` has to be `scan()` rather
        # than an accidental empty read.
        assert filled.scan_where().read_all().num_rows == 6
        assert filled.scan_where({}).read_all().num_rows == 6


class TestRefScans:
    """A branch or tag is read as the snapshot it names, not as the present."""

    def test_a_branch_reads_the_snapshot_it_names_rather_than_the_current_one(
        self, table_planning: Table
    ) -> None:
        table_planning.append(_rows_planning())
        assert table_planning.current_snapshot is not None
        first = table_planning.current_snapshot.snapshot_id
        table_planning.create_branch("nightly", first)
        table_planning.append(_rows_planning(4))

        assert table_planning.scan().read_all().num_rows == 6
        assert table_planning.scan_ref("nightly").read_all().num_rows == 3
        # The filters mean on a ref what they mean on the present.
        pinned = table_planning.scan_ref("nightly", {"venue": "XNAS"}).read_all()
        assert pinned.to_pydict() == {"id": [1], "venue": ["XNAS"]}

    def test_a_ref_the_table_does_not_carry_names_the_refs_it_does(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match=r'got "nightly"; it has \[main\]'):
            filled.scan_ref("nightly")


class TestPlanning:
    """A plan is what the metadata decided, before a single row was read."""

    def test_a_plan_accounts_for_every_live_file_it_did_not_read(
        self, filled: Table
    ) -> None:
        total = len(filled.data_files())
        assert total == 6

        plan = filled.plan({"venue": "XNAS"})

        assert isinstance(plan, ScanPlan)
        # Nothing may be quietly dropped: every live file is either planned or
        # explicitly skipped, which is the arithmetic that makes "it skipped
        # four files" a claim rather than a hope.
        assert plan.files_planned + plan.files_skipped == total
        assert (plan.files_planned, plan.files_skipped) == (2, 4)
        assert plan.manifests_read == 2

    def test_a_plan_reports_the_rows_the_equivalent_scan_yields(
        self, filled: Table
    ) -> None:
        # An identity partition filter is settled by the plan alone - every row
        # of a matching file holds the value - so the counted rows and the read
        # rows have to be the same number, without a row being read to find out.
        for filters in ({}, {"venue": "XNAS"}, {"venue": "XNYS"}, {"venue": "null"}):
            assert (
                filled.plan(filters).record_count
                == filled.scan_where(filters).read_all().num_rows
            )

    def test_a_plan_counts_the_rows_of_the_files_and_not_of_the_answer(
        self, tmp_path: pathlib.Path, numbered_planning: object
    ) -> None:
        flat = Table.create(IOBase(tmp_path / "flat"), numbered_planning)
        flat.append(_rows_planning())

        plan = flat.plan({"id": "1"})

        # `id` carries no partition, so statistics bound a file rather than
        # select a row: the plan counts the whole file it could not exclude,
        # and the scan then filters the rows inside it. Reading `record_count`
        # as "the answer's size" would be wrong here, and that is the point.
        assert plan.files_planned == 1
        assert plan.record_count == 3
        assert flat.scan_where({"id": "1"}).read_all().num_rows == 1

    def test_a_filter_matching_no_partition_opens_no_manifest_at_all(
        self, filled: Table
    ) -> None:
        plan = filled.plan({"venue": "XLON"})

        # The manifest-list summaries settle an identity partition filter, so
        # the cheapest level answered it: nothing was opened to find out.
        assert plan.files_planned == 0
        assert plan.record_count == 0
        assert plan.manifests_read == 0
        assert plan.manifests_skipped == 2

    def test_a_filter_no_row_can_satisfy_still_reports_the_manifests_it_read(
        self, filled: Table
    ) -> None:
        plan = filled.plan({"id": "999"})

        # `id` is not partitioned, so no summary could exclude a manifest: both
        # had to be opened, and the column statistics inside them are what
        # excluded every file.
        assert plan.files_planned == 0
        assert plan.files_skipped == 6
        assert plan.manifests_read == 2
        assert plan.manifests_skipped == 0
        assert filled.scan_where({"id": "999"}).read_all().num_rows == 0

    def test_planning_an_earlier_snapshot_reports_that_snapshot(
        self, filled: Table
    ) -> None:
        first = filled.snapshots[0].snapshot_id

        earlier = filled.plan_at(first)
        now = filled.plan()

        assert (earlier.record_count, earlier.files_planned) == (3, 3)
        assert (now.record_count, now.files_planned) == (6, 6)
        # History is planned by the same rules the present is planned by.
        assert filled.plan_at(first, {"venue": "XNAS"}).files_planned == 1

    def test_planning_a_snapshot_the_table_does_not_retain_is_refused(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            filled.plan_at(999)

    def test_an_empty_table_plans_nothing_rather_than_failing(
        self, table_planning: Table
    ) -> None:
        # A table with no snapshot has no manifests, which is an answer and not
        # an error - the same way an empty scan reads as no rows.
        plan = table_planning.plan()

        assert (plan.record_count, plan.files_planned, plan.files_skipped) == (0, 0, 0)
        assert (plan.manifests_read, plan.manifests_skipped) == (0, 0)

    def test_a_plan_refuses_an_undeclared_filter_column_as_a_scan_does(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match='got "market"'):
            filled.plan({"market": "XNAS"})
        with pytest.raises(ValueError, match='got "market"'):
            filled.plan_at(filled.snapshots[0].snapshot_id, {"market": "XNAS"})


class TestOverwritingAPartition:
    """Replacing one partition is the whole point: every other file is carried."""

    def test_replacing_one_partition_leaves_every_other_partition_untouched(
        self, filled: Table
    ) -> None:
        untouched = filled.scan_where({"venue": "XNYS"}).read_all().to_pydict()
        absent = filled.scan_where({"venue": "null"}).read_all().to_pydict()

        filled.overwrite_where({"venue": "XNAS"}, _row(100, "XNAS"))

        assert filled.scan_where({"venue": "XNAS"}).read_all().to_pydict() == {
            "id": [100],
            "venue": ["XNAS"],
        }
        # The rows nobody named must come back byte for byte the same, in the
        # same order: a carried file is carried, not rewritten.
        assert filled.scan_where({"venue": "XNYS"}).read_all().to_pydict() == untouched
        assert filled.scan_where({"venue": "null"}).read_all().to_pydict() == absent
        assert sorted(filled.scan().read_all().column("id").to_pylist()) == [
            2,
            3,
            5,
            6,
            100,
        ]

    def test_the_replaced_snapshot_is_retained_and_still_reads_the_old_rows(
        self, filled: Table
    ) -> None:
        assert filled.current_snapshot is not None
        before = filled.current_snapshot.snapshot_id

        filled.overwrite_where({"venue": "XNAS"}, _row(100, "XNAS"))

        assert filled.current_snapshot is not None
        assert filled.current_snapshot.operation == "overwrite"
        # Retention is what makes the replacement reversible.
        assert len(filled.snapshots) == 3
        old = filled.scan_at(before).read_all()
        assert sorted(old.column("id").to_pylist()) == [1, 2, 3, 4, 5, 6]

    def test_an_undeclared_filter_column_is_refused_before_anything_is_written(
        self, filled: Table
    ) -> None:
        before = filled.version

        with pytest.raises(ValueError, match='got "market"'):
            filled.overwrite_where({"market": "XNAS"}, _row(100, "XNAS"))

        # A refused write must be a write that never happened.
        assert filled.version == before
        assert filled.scan().read_all().num_rows == 6

    def test_overwriting_by_no_filter_replaces_every_row(self, filled: Table) -> None:
        filled.overwrite_where({}, _row(100, "XNAS"))

        assert filled.scan().read_all().to_pydict() == {
            "id": [100],
            "venue": ["XNAS"],
        }


class TestMerging:
    """A merge is the upsert: a stored key is updated, an unknown one appended."""

    def test_a_merge_updates_a_stored_key_and_appends_an_unknown_one(
        self, table_planning: Table
    ) -> None:
        table_planning.append(_rows_planning())

        table_planning.merge(
            pa.record_batch(
                {"id": [2, 9], "venue": ["XPAR", "XLON"]}, schema=SCHEMA_planning
            ),
            ["id"],
        )

        rows = table_planning.scan().read_all().sort_by("id").to_pydict()
        # A merge joins on the partition columns before the caller's key, so a
        # row only ever updates a row of its own partition: the stored 2 sits
        # under `XNYS` and the arriving 2 names `XPAR`, a partition of its own,
        # so both stand. 9 was stored nowhere, so it arrived.
        assert rows == {
            "id": [1, 2, 2, 3, 9],
            "venue": ["XNAS", "XNYS", "XPAR", None, "XLON"],
        }

    def test_a_merge_scoped_to_one_partition_leaves_the_others_as_they_were(
        self, filled: Table
    ) -> None:
        others = sorted(
            filled.scan_where({"venue": "XNAS"}).read_all().column("id").to_pylist()
        )

        filled.merge_where(
            {"venue": "XNYS"},
            pa.record_batch({"id": [2, 8], "venue": ["XNYS", "XNYS"]}, schema=SCHEMA_planning),
            ["id"],
        )

        # 2 was already in the scoped partition and was updated in place; 8 was
        # not stored anywhere and was appended.
        assert sorted(
            filled.scan_where({"venue": "XNYS"}).read_all().column("id").to_pylist()
        ) == [2, 5, 8]
        # The partitions the filter excluded were never even read.
        assert (
            sorted(
                filled.scan_where({"venue": "XNAS"}).read_all().column("id").to_pylist()
            )
            == others
        )
        assert filled.scan().read_all().num_rows == 7

    def test_merging_on_no_column_at_all_is_an_overwrite(self, table_planning: Table) -> None:
        table_planning.append(_rows_planning())

        # Every row would match every row, so the only honest reading of "no
        # match key" is a replacement.
        table_planning.merge(_rows_planning(10), [])

        assert table_planning.scan().read_all().column("id").to_pylist() == [10, 11, 12]
        assert table_planning.current_snapshot is not None
        assert table_planning.current_snapshot.operation == "overwrite"

    def test_a_match_key_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        before = filled.version

        with pytest.raises(ValueError, match='got "market"'):
            filled.merge(_row(100, "XNAS"), ["market"])

        assert filled.version == before
        assert filled.scan().read_all().num_rows == 6

    def test_one_string_is_the_selector_text(self, filled: Table) -> None:
        # "id" is selector text, so one string names one match key rather
        # than reading as the characters it is made of.
        before = filled.version
        filled.merge(_row(100, "XNAS"), "id")
        assert filled.version == before + 1
        assert filled.scan().read_all().num_rows == 7

    def test_a_value_the_column_cannot_read_is_refused_under_a_strict_cast(
        self, filled: Table
    ) -> None:
        before = filled.version
        text = pa.schema(
            [
                pa.field("id", pa.string(), nullable=False),
                pa.field("venue", pa.string()),
            ]
        )
        unreadable = pa.record_batch({"id": ["nine"], "venue": ["XNAS"]}, schema=text)

        with pytest.raises(ValueError, match="Cast error"):
            filled.merge(unreadable, ["id"], safe=False)

        assert filled.version == before

    def test_a_merge_filter_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match='got "market"'):
            filled.merge_where({"market": "XNAS"}, _row(100, "XNAS"), ["id"])


class TestExpiringSnapshots:
    """Expiry drops what retention no longer names, and nothing else."""

    def test_defaults_retain_override_and_explicit_ids(self, filled: Table) -> None:
        first = filled.snapshots[0].snapshot_id
        assert filled.current_snapshot is not None
        current = filled.current_snapshot.snapshot_id
        before = filled.version

        # Fresh snapshots survive the default five-day cutoff. A retain
        # override also protects both snapshots from a future cutoff.
        assert filled.expire_snapshots() == []
        assert filled.expire_snapshots(int(time.time() * 1000) + 60_000, 2) == []
        assert filled.expire_snapshots(0, snapshot_ids=[999]) == []
        assert filled.version == before

        with pytest.raises(ValueError, match="retain_last.*at least 1"):
            filled.expire_snapshots(retain_last=0)
        with pytest.raises(ValueError, match="cannot expire current snapshot"):
            filled.expire_snapshots(snapshot_ids=[current])

        # Explicit ids join age selection, so an old cutoff does not stop this
        # known, unprotected ancestor from being removed.
        assert filled.expire_snapshots(0, snapshot_ids=[first]) == [first]
        assert filled.scan().read_all().num_rows == 6

    def test_a_cutoff_older_than_everything_expires_nothing_and_spends_no_version(
        self, filled: Table
    ) -> None:
        before = filled.version
        retained = [snapshot.snapshot_id for snapshot in filled.snapshots]

        assert filled.expire_snapshots(0) == []

        # An expiry with nothing to do must not write a metadata document: the
        # check runs on a copy first, so a no-op costs no version.
        assert filled.version == before
        assert [snapshot.snapshot_id for snapshot in filled.snapshots] == retained

    def test_a_cutoff_past_an_early_snapshot_expires_it_and_the_current_survives(
        self, table_planning: Table
    ) -> None:
        for start in (1, 4, 7):
            table_planning.append(_rows_planning(start))
        early = [snapshot.snapshot_id for snapshot in table_planning.snapshots[:2]]
        assert table_planning.current_snapshot is not None
        current = table_planning.current_snapshot.snapshot_id
        before = table_planning.version

        expired = table_planning.expire_snapshots(int(time.time() * 1000) + 60_000)

        assert sorted(expired) == sorted(early)
        assert table_planning.version == before + 1
        # The current snapshot is always retained, whatever the cutoff says,
        # and it is still a complete table.
        assert [snapshot.snapshot_id for snapshot in table_planning.snapshots] == [current]
        assert table_planning.current_snapshot is not None
        assert table_planning.current_snapshot.snapshot_id == current
        assert table_planning.scan().read_all().num_rows == 9

    def test_an_expired_snapshot_is_no_longer_one_the_table_will_read(
        self, table_planning: Table
    ) -> None:
        table_planning.append(_rows_planning())
        table_planning.append(_rows_planning(4))
        first = table_planning.snapshots[0].snapshot_id

        assert table_planning.expire_snapshots(int(time.time() * 1000) + 60_000) == [first]

        # Time travel to a dropped snapshot must be refused rather than read
        # from files that happen to still be on disk.
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            table_planning.scan_at(first)
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            table_planning.plan_at(first)

    def test_a_tagged_snapshot_survives_a_cutoff_that_would_reach_it(
        self, table_planning: Table
    ) -> None:
        table_planning.append(_rows_planning())
        first = table_planning.snapshots[0].snapshot_id
        table_planning.create_tag("release", first)
        table_planning.append(_rows_planning(4))

        assert table_planning.expire_snapshots(int(time.time() * 1000) + 60_000) == []

        # A ref anchors its target: retention is honored before the age cutoff.
        assert table_planning.scan_at(first).read_all().num_rows == 3


class TestFastForward:
    """A branch moves only forward, which is why it cannot lose history."""

    def test_a_branch_moves_to_a_descendant_snapshot(self, table_planning: Table) -> None:
        table_planning.append(_rows_planning())
        assert table_planning.current_snapshot is not None
        first = table_planning.current_snapshot.snapshot_id
        table_planning.create_branch("nightly", first)
        table_planning.append(_rows_planning(4))
        assert table_planning.current_snapshot is not None
        second = table_planning.current_snapshot.snapshot_id

        table_planning.fast_forward("nightly", second)

        assert table_planning.snapshot_by_ref("nightly").snapshot_id == second
        assert table_planning.scan_ref("nightly").read_all().num_rows == 6

    def test_a_target_that_is_not_a_descendant_is_refused_naming_both_ends(
        self, table_planning: Table
    ) -> None:
        table_planning.append(_rows_planning())
        assert table_planning.current_snapshot is not None
        first = table_planning.current_snapshot.snapshot_id
        table_planning.append(_rows_planning(4))
        assert table_planning.current_snapshot is not None
        second = table_planning.current_snapshot.snapshot_id
        table_planning.create_branch("nightly", second)

        # Moving the branch back to its head's parent would silently drop the
        # commits between them, which is the one thing a fast-forward promises
        # it cannot do.
        with pytest.raises(ValueError, match=f"expected {first} to descend from"):
            table_planning.fast_forward("nightly", first)

        assert table_planning.snapshot_by_ref("nightly").snapshot_id == second

    def test_moving_a_branch_the_table_does_not_have_is_refused(
        self, filled: Table
    ) -> None:
        assert filled.current_snapshot is not None

        with pytest.raises(ValueError, match='expected a branch named "nightly"'):
            filled.fast_forward("nightly", filled.current_snapshot.snapshot_id)

    def test_a_target_the_table_does_not_retain_is_refused(self, table_planning: Table) -> None:
        table_planning.append(_rows_planning())

        with pytest.raises(ValueError, match="unknown snapshot id 999"):
            table_planning.fast_forward("main", 999)


class TestManifestsOfASnapshot:
    """A snapshot is named by identifier, because the table owns what it retains."""

    def test_the_manifests_of_a_retained_snapshot_are_that_snapshots(
        self, filled: Table
    ) -> None:
        first = filled.snapshots[0].snapshot_id

        earlier = filled.manifests_at(first)
        current = filled.manifests()

        # The second commit carries the first commit's manifest forward, so the
        # earlier snapshot is a strict subset - and every manifest it names was
        # added by it.
        assert len(earlier) == 1
        assert len(current) == 2
        assert [manifest.added_snapshot_id for manifest in earlier] == [first]
        assert {manifest.path for manifest in earlier} <= {
            manifest.path for manifest in current
        }
        assert earlier[0].added_files_count == 3
        assert earlier[0].added_rows_count == 3

    def test_an_id_the_table_does_not_retain_is_refused_naming_the_ids_it_does(
        self, filled: Table
    ) -> None:
        retained = ", ".join(
            str(snapshot.snapshot_id) for snapshot in filled.snapshots
        )

        with pytest.raises(
            ValueError,
            match=f"expected a retained snapshot id, got 999; the table retains "
            rf"\[{retained}\]",
        ):
            filled.manifests_at(999)
