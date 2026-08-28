"""An Iceberg table, built on the handle every other test already uses."""

from __future__ import annotations

import copy
import json
import pathlib
import pickle

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
    Snapshot,
    Table,
    assign_field_ids,
    can_promote,
    schema_from_json,
    schema_to_json,
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
        assert [child.parquet_field_id for child in numbered.data_type] == [1, 2]
        # The input is untouched: the numbered schema is a new value.
        assert SCHEMA.field("id").metadata is None

    def test_numbering_starts_where_it_is_told_to(self) -> None:
        numbered = assign_field_ids(SCHEMA, 10)

        assert [child.parquet_field_id for child in numbered.data_type] == [10, 11]

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
        assert schema.data_type.kind == "struct"
        assert not schema.nullable
        # `required` inverts into nullability, and `id` becomes PARQUET:field_id.
        assert not schema.data_type[0].nullable
        assert schema.data_type[1].nullable
        assert [child.parquet_field_id for child in schema.data_type] == [1, 2]

        assert schema_to_json(schema) == document

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

        ids = [child.parquet_field_id for child in table.schema.data_type]
        assert ids == [1, 2]
        assert [field.name for field in table.spec.fields] == ["venue"]

        table.append(_rows())
        assert table.scan().read_all().num_rows == 3

    def test_the_metadata_document_is_where_a_reader_looks(
        self, table: Table, tmp_path: pathlib.Path
    ) -> None:
        assert table.metadata_file_name.startswith("00001-")
        assert table.metadata_file_name.endswith(".metadata.json")
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
        # A table is a folder, and an in-memory buffer names no folder.
        with pytest.raises(ValueError, match="file URI"):
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
        columns = pa.schema([child.into_arrow() for child in marked.data_type])
        rows = pa.table(
            {"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=columns
        )

        table = catalog.append("nyc.taxis", rows)
        assert "nyc.taxis" in catalog.tables
        assert list(catalog.namespaces) == ["nyc"]
        assert list(catalog.namespaces["nyc"].tables) == ["taxis"]

        # The schema was inferred from the reader and numbered, and the marked
        # column became the identity spec.
        assert [child.parquet_field_id for child in table.schema.data_type] == [1, 2]
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
        with pytest.raises(ValueError, match="reserved .*iceberg:"):
            catalog.update_properties({"iceberg:x": "1"})

        sales = catalog.namespaces.create("sales")
        assert sales.properties == {}
        sales.update_properties({"team": "emea"})
        assert sales.properties == {"team": "emea"}
        assert catalog.namespaces["sales"].properties == {"team": "emea"}
        with pytest.raises(ValueError, match="reserved .*iceberg:"):
            sales.update_properties({"iceberg:x": "1"})


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
        children = list(narrow.schema.data_type)
        assert [child.name for child in children] == ["id", "market", "price"]
        # The widened type reads back, the renamed column keeps its
        # identifier, and the added column is numbered above every identifier
        # the table has ever assigned.
        assert children[0].data_type.id == "int64"
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
        assert [child.name for child in narrow.schema.data_type] == ["id", "venue"]

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

        evolved = narrow.schema.data_type[0]
        assert evolved.nullable
        assert evolved.iceberg["doc"] == "row identifier"

    def test_a_dropped_column_retires_its_identifier(self, narrow: Table) -> None:
        with narrow.update_schema() as update:
            update.drop_column("venue").add_column("", "note: string")

        children = list(narrow.schema.data_type)
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
