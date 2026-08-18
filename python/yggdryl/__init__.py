"""Allocation-conscious schema, record, and resource identifier values.

The ``gzip``, ``zlib``, and ``zstd`` submodules carry the standard library's
names on purpose: they hold the same wire formats behind the same
``loads``/``dumps`` spelling, so ``from yggdryl import gzip`` changes the engine
underneath and nothing else.
"""

from ._native import (
    DataType,
    Field,
    IOBase,
    MediaType,
    MimeType,
    ProtocolMetadata,
    RecordOptions,
    Timezone,
    Uri,
    Url,
    Urn,
    __version__,
    schema_from_pattern,
)
from . import fields, gzip, iceberg, json, toml, yaml, zlib, zstd
from .records import Record, from_dict, record, schema_field, schema_fields, to_dict

__all__ = [
    "DataType",
    "Field",
    "IOBase",
    "MediaType",
    "MimeType",
    "ProtocolMetadata",
    "Record",
    "RecordOptions",
    "Timezone",
    "Uri",
    "Url",
    "Urn",
    "__version__",
    "from_dict",
    "fields",
    "gzip",
    "iceberg",
    "json",
    "record",
    "schema_field",
    "schema_fields",
    "schema_from_pattern",
    "toml",
    "to_dict",
    "yaml",
    "zlib",
    "zstd",
]
