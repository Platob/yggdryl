"""Allocation-conscious types, storage, media, and protocols.

The native core reports what it does through `logging`, under this package's
own logger: `yggdryl.media.iceberg.table` and its siblings are the Rust module
paths the work happens in, so `logging.getLogger("yggdryl")` is the one switch.
Debug is an operation starting, info is one done and carries the counts a
monitor watches - a table opened, a scan planned and what its filters pruned,
a snapshot's rows and files, a commit and the version it landed. Nothing is
reported per row, per batch, or per file. A level changed after import reaches
the bridge through `refresh_logging`.
"""

from . import arrow, coding, enums, expression, fix, holder, media, text, txhash, types, uri, xxhash
from ._native import (
    DEFAULT_FETCH_BYTE_SIZE,
    DEFAULT_STREAM_BATCH_SIZE,
    IPC_DICTIONARY_IDS_KEY,
    ArrowValue,
    __version__,
    combined,
    refresh_logging,
)
from .expression import Bound, Bounds, BoundStatement, Expression, Statement
from .holder import IOBase, IOCursor
from .media import MediaType, MimeType, RecordOptions, TextOptions
from .types import (
    ArrowCastPlan,
    AsciiEnum,
    DataType,
    Field,
    ProtocolField,
    PythonMetadata,
    Scalar,
    Timezone,
    Version,
    field,
)
from .types.scalar import scalar
from .uri import Parameters, Uri, Url, Urn

__all__ = [
    "ArrowCastPlan",
    "ArrowValue",
    "AsciiEnum",
    "Bound",
    "Bounds",
    "BoundStatement",
    "DataType",
    "Expression",
    "Field",
    "IOBase",
    "IOCursor",
    "MediaType",
    "MimeType",
    "Parameters",
    "ProtocolField",
    "PythonMetadata",
    "RecordOptions",
    "Scalar",
    "Statement",
    "TextOptions",
    "Timezone",
    "Version",
    "Uri",
    "Url",
    "Urn",
    "__version__",
    "arrow",
    "coding",
    "DEFAULT_FETCH_BYTE_SIZE",
    "DEFAULT_STREAM_BATCH_SIZE",
    "IPC_DICTIONARY_IDS_KEY",
    "combined",
    "enums",
    "expression",
    "field",
    "fix",
    "holder",
    "media",
    "refresh_logging",
    "scalar",
    "text",
    "types",
    "uri",
    "txhash",
    "xxhash",
]
