"""Apache Iceberg tables, over the handles :mod:`yggdryl` already gives you.

A table is a folder: every metadata document, manifest, and data file below it
is reached through the same ``IOBase`` handle a caller builds for anything else,
so the code that writes a table on disk is the code that will write one to an
object store. Rows cross as ``pyarrow.RecordBatchReader`` values, which means a
scan stays lazy on both sides of the boundary.
"""

from __future__ import annotations

from ._native import (
    Catalog,
    Compaction,
    DataFile,
    ManifestFile,
    PartitionField,
    PartitionSpec,
    SchemaUpdate,
    Snapshot,
    Table,
    assign_field_ids,
    can_promote,
    schema_from_json,
    schema_to_json,
)

__all__ = [
    "Catalog",
    "Compaction",
    "DataFile",
    "ManifestFile",
    "PartitionField",
    "PartitionSpec",
    "SchemaUpdate",
    "Snapshot",
    "Table",
    "assign_field_ids",
    "can_promote",
    "schema_from_json",
    "schema_to_json",
]
