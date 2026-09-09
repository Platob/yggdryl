"""Record media, encoding options, and table formats.

The capitalized handle classes are the record implementation a composed handle
retains, so ``type(handle)`` names the encoding its rows are read through.
"""

from .._native import (
    Avro,
    Csv,
    Ipc,
    Media,
    MediaType,
    MimeType,
    Parquet,
    RecordOptions,
    Text,
    TextOptions,
)
from . import avro, iceberg

__all__ = [
    "Avro",
    "Csv",
    "Ipc",
    "Media",
    "MediaType",
    "MimeType",
    "Parquet",
    "RecordOptions",
    "Text",
    "TextOptions",
    "avro",
    "iceberg",
]
