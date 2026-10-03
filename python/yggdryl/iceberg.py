"""Apache Iceberg tables, over the handles :mod:`yggdryl` already gives you.

A table is a folder: every metadata document, manifest, and data file below it
is reached through the same ``IOBase`` handle a caller builds for anything else,
so the code that writes a table on disk is the code that will write one to an
object store. A scan hands back a ``pyarrow.RecordBatchReader``, so it stays
lazy on both sides of the boundary; a write takes any shape the record surface
takes, typed against the table's stored schema.

A warehouse folder of tables is an :class:`IcebergCatalog`, a folder under it
an :class:`IcebergNamespace`, and a table in it an :class:`IcebergTable` - the
Iceberg subclasses of :class:`yggdryl.Catalog`, :class:`yggdryl.Namespace` and
:class:`yggdryl.Table`, walked through the views every catalog answers.
"""

from __future__ import annotations

from ._native import (
    Compaction,
    DataFile,
    IcebergCatalog,
    IcebergNamespace,
    IcebergOptions,
    IcebergTable,
    ManifestFile,
    PartitionField,
    PartitionSpec,
    ScanPlan,
    SchemaUpdate,
    Snapshot,
    assign_field_ids,
    can_promote,
    schema_from_json,
    schema_into_json,
)

__all__ = [
    "Compaction",
    "DataFile",
    "IcebergCatalog",
    "IcebergNamespace",
    "IcebergOptions",
    "IcebergTable",
    "ManifestFile",
    "PartitionField",
    "PartitionSpec",
    "ScanPlan",
    "SchemaUpdate",
    "Snapshot",
    "assign_field_ids",
    "can_promote",
    "schema_from_json",
    "schema_into_json",
]
