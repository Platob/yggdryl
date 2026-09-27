"""What every record medium shares: the handle it composes, and its settings.

The capitalized handle classes are the record implementation a composed handle
retains, so ``type(handle)`` names the encoding its rows are read through. Each
medium itself is a module of its own beside this one - :mod:`yggdryl.avro` and
:mod:`yggdryl.iceberg` - as the crate gives every implementation a root file.
"""

from .._native import (
    DEFAULT_RECORD_BATCH_ROW_SIZE,
    NULL_PARTITION,
    Avro,
    Ipc,
    Media,
    Parquet,
    RecordOptions,
    Text,
    Xmla,
    partition_text,
    with_partitions,
    without_partitions,
)

__all__ = [
    "DEFAULT_RECORD_BATCH_ROW_SIZE",
    "NULL_PARTITION",
    "Avro",
    "Ipc",
    "Media",
    "Parquet",
    "RecordOptions",
    "Text",
    "Xmla",
    "partition_text",
    "with_partitions",
    "without_partitions",
]
