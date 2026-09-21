"""Apache Iceberg tables, over the handles :mod:`yggdryl` already gives you.

A table is a folder: every metadata document, manifest, and data file below it
is reached through the same ``IOBase`` handle a caller builds for anything else,
so the code that writes a table on disk is the code that will write one to an
object store. A scan hands back a ``pyarrow.RecordBatchReader``, so it stays
lazy on both sides of the boundary; a write takes any shape the record surface
takes, typed against the table's stored schema.
"""

from __future__ import annotations

from ._native import (
    Catalog,
    Namespace,
    Namespaces,
    Compaction,
    DataFile,
    IcebergOptions,
    ManifestFile,
    PartitionField,
    PartitionSpec,
    ScanPlan,
    SchemaUpdate,
    Snapshot,
    Table,
    Tables,
    assign_field_ids,
    can_promote,
    schema_from_json,
    schema_into_json,
)

__all__ = [
    "Catalog",
    "Namespace",
    "Namespaces",
    "Compaction",
    "DataFile",
    "IcebergOptions",
    "ManifestFile",
    "PartitionField",
    "PartitionSpec",
    "ScanPlan",
    "SchemaUpdate",
    "Snapshot",
    "Table",
    "Tables",
    "assign_field_ids",
    "can_promote",
    "schema_from_json",
    "schema_into_json",
]
