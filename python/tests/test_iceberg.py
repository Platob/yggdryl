"""An Iceberg table, built on the handle every other test already uses."""

from __future__ import annotations

import pathlib

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase
from yggdryl.iceberg import (
    Catalog,
    Compaction,
    IcebergOptions,
    PartitionSpec,
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
        assert table.metadata_file_name == "v1.metadata.json"
        assert table.metadata_location.endswith("metadata/v1.metadata.json")

        metadata = IOBase(tmp_path / "trades" / "metadata")
        assert {entry.name for entry in metadata} == {
            "v1.metadata.json",
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
        assert all(file.file_format == "PARQUET" for file, _ in files)
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
        # spells absence - the error names what is missing.
        table = catalog.namespaces["analytics"].tables["trades"]
        table.append(pa.table({"id": [1, 2], "venue": ["XNAS", None]}))
        chained = catalog.namespaces["analytics"].tables["trades"]
        assert chained.scan().read_all().num_rows == 2
        with pytest.raises(KeyError, match="absent"):
            catalog.namespaces["analytics"].tables["absent"]
        with pytest.raises(KeyError, match="missing"):
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
        with pytest.raises(ValueError, match="expected no namespace"):
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
        assert catalog.list_namespaces() == []
        assert not catalog.has_table("nyc.taxis")

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
        columns = pa.schema([child.to_arrow() for child in marked.data_type])
        rows = pa.table(
            {"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=columns
        )

        table = catalog.append("nyc.taxis", rows)
        assert catalog.has_table("nyc.taxis")
        assert catalog.list_namespaces() == ["nyc"]
        assert catalog.list_tables("nyc") == ["nyc.taxis"]

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

    def test_create_table_takes_an_iterable_of_fields(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(IOBase(tmp_path / "warehouse"))

        table = catalog.create_table(
            "ns.trades", [Field("id", "int64", nullable=False)]
        )
        assert catalog.list_tables("ns") == ["ns.trades"]
        assert table.spec.is_unpartitioned()

        with pytest.raises(ValueError, match="expected no table"):
            catalog.create_table("ns.trades", [Field("id", "int64", nullable=False)])
        # An existing table is opened as it is; the schema describes only the
        # table the call would create.
        same = catalog.open_or_create_table("ns.trades", SCHEMA)
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
            catalog.create_table("a/b", SCHEMA)


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
        # the stored value is a time travel away under its old name.
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
    """Iceberg keeps its own options type, with the flattened-kwargs rule."""

    def test_the_options_value_records_only_what_was_set(self) -> None:
        options = IcebergOptions()
        assert options.commit_retries == 4
        assert options.target_file_size == 512 * 1024 * 1024
        assert options.data_format == "PARQUET"

        options = IcebergOptions(commit_retries=2, data_format="avro")
        assert options.commit_retries == 2
        assert options.data_format == "AVRO"
        options.target_file_size = 1024
        assert options.target_file_size == 1024
        with pytest.raises(TypeError, match="commit_retres"):
            IcebergOptions(commit_retres=2)

    def test_append_takes_the_option_fields_as_keywords(
        self, table: Table
    ) -> None:
        table.append(_rows(), target_file_size=1024, commit_retries=1)
        assert table.scan().read_all().num_rows == 3

    def test_a_keyword_overrides_the_same_field_on_the_options(
        self, table: Table
    ) -> None:
        options = IcebergOptions(data_format="parquet")
        table.append(_rows(), options=options, data_format="avro")
        # The keyword won: the data files are Avro, and the manifest says so.
        formats = {file.file_format for file, _ in table.data_files()}
        assert formats == {"AVRO"}
        # The caller's options object was never touched.
        assert options.data_format == "PARQUET"

    def test_an_avro_append_scans_back_and_mixes_with_parquet(
        self, table: Table
    ) -> None:
        table.append(_rows(), data_format="avro")
        table.append(_rows(10))
        formats = {file.file_format for file, _ in table.data_files()}
        assert formats == {"AVRO", "PARQUET"}
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
        table.set_options(data_format="avro")
        table.append(_rows())
        formats = {file.file_format for file, _ in table.data_files()}
        assert formats == {"AVRO"}
        assert table.options().data_format == "AVRO"
        # A per-call keyword still wins for its one call, without disturbing
        # the stored override.
        table.append(_rows(10), data_format="parquet")
        assert sorted(
            {file.file_format for file, _ in table.data_files()}
        ) == ["AVRO", "PARQUET"]
        assert table.options().data_format == "AVRO"

    def test_the_property_layer_sets_the_format_per_table(
        self, table: Table
    ) -> None:
        table.update_properties({"write.format.default": "avro"})
        table.append(_rows())
        formats = {file.file_format for file, _ in table.data_files()}
        assert formats == {"AVRO"}
        # An unencodable format is a typed error naming the key, up front.
        table.update_properties({"write.format.default": "orc"})
        with pytest.raises(ValueError, match="write.format.default"):
            table.append(_rows(10))

    def test_the_catalog_write_paths_take_the_same_keywords(
        self, tmp_path: pathlib.Path
    ) -> None:
        catalog = Catalog(tmp_path / "warehouse")
        table = catalog.append("sales.orders", _rows(), data_format="avro")
        formats = {file.file_format for file, _ in table.data_files()}
        assert formats == {"AVRO"}

        tables = catalog.namespaces["sales"].tables
        table = tables.append("orders", _rows(10), target_file_size=1024)
        assert table.scan().read_all().num_rows == 6
        table = tables.overwrite("orders", _rows(), data_format="avro")
        assert table.scan().read_all().num_rows == 3
