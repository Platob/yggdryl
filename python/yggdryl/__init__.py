"""Allocation-conscious schema, codec, and resource identifier values.

The ``gzip``, ``zlib``, and ``zstd`` submodules carry the standard library's
names on purpose: they hold the same wire formats behind the same
``loads``/``dumps`` spelling, so ``from yggdryl import gzip`` changes the engine
underneath and nothing else.
"""

from ._native import (
    AsciiDictionary,
    Bound,
    BoundStatement,
    DataType,
    Expression,
    Field,
    IOBase,
    MediaType,
    MimeType,
    ProtocolField,
    RecordOptions,
    Statement,
    TextOptions,
    Timezone,
    Uri,
    Url,
    Urn,
    __version__,
    combined,
)
from . import (
    avro,
    codec,
    enums,
    fields,
    fix,
    gzip,
    iceberg,
    json,
    toml,
    xxhash,
    yaml,
    zlib,
    zstd,
)
from .fields import field
from .scalar import Scalar, scalar

__all__ = [
    "AsciiDictionary",
    "Bound",
    "BoundStatement",
    "DataType",
    "Expression",
    "Field",
    "IOBase",
    "MediaType",
    "MimeType",
    "ProtocolField",
    "RecordOptions",
    "Statement",
    "TextOptions",
    "Timezone",
    "Uri",
    "Url",
    "Urn",
    "Scalar",
    "__version__",
    "combined",
    "codec",
    "avro",
    "enums",
    "field",
    "scalar",
    "fields",
    "fix",
    "gzip",
    "iceberg",
    "json",
    "toml",
    "yaml",
    "zlib",
    "xxhash",
    "zstd",
]
