"""Allocation-conscious schema, codec, and resource identifier values.

The ``gzip``, ``zlib``, and ``zstd`` submodules carry the standard library's
names on purpose: they hold the same wire formats behind the same
``loads``/``dumps`` spelling, so ``from yggdryl import gzip`` changes the engine
underneath and nothing else.
"""

from ._native import (
    Bound,
    BoundStatement,
    DataType,
    Expression,
    Field,
    IOBase,
    MediaType,
    MimeType,
    ProtocolMetadata,
    RecordOptions,
    Statement,
    Timezone,
    Uri,
    Url,
    Urn,
    Value,
    __version__,
    combined,
    field_from_pattern,
)
from . import avro, codec, enums, fields, gzip, iceberg, json, toml, yaml, zlib, zstd
from .fields import field
from .fields._classes import scalar

__all__ = [
    "Bound",
    "BoundStatement",
    "DataType",
    "Expression",
    "Field",
    "IOBase",
    "MediaType",
    "MimeType",
    "ProtocolMetadata",
    "RecordOptions",
    "Statement",
    "Timezone",
    "Uri",
    "Url",
    "Urn",
    "Value",
    "__version__",
    "combined",
    "codec",
    "avro",
    "enums",
    "field",
    "scalar",
    "fields",
    "gzip",
    "iceberg",
    "json",
    "field_from_pattern",
    "toml",
    "yaml",
    "zlib",
    "zstd",
]
