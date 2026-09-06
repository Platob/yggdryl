"""Record media, encoding options, and table formats.

The capitalized handle classes are the record implementation a composed handle
retains, so ``type(handle)`` names the encoding its rows are read through.
"""

from .._native import (
    AVRO_MAX_SCHEMA_DEPTH,
    Avro,
    DEFAULT_RECORD_BATCH_ROW_SIZE,
    Ipc,
    Media,
    MediaType,
    MimeType,
    NULL_PARTITION,
    Parquet,
    RecordOptions,
    Text,
    TextOptions,
    partition_text,
    with_partitions,
    without_partitions,
)
from . import avro, iceberg

__all__ = [
    "AVRO_MAX_SCHEMA_DEPTH",
    "Avro",
    "DEFAULT_RECORD_BATCH_ROW_SIZE",
    "Ipc",
    "Media",
    "MediaType",
    "MimeType",
    "NULL_PARTITION",
    "Parquet",
    "RecordOptions",
    "Text",
    "TextOptions",
    "avro",
    "iceberg",
    "partition_text",
    "with_partitions",
    "without_partitions",
]
