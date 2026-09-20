"""Allocation-conscious types, storage, media, and protocols.

The native core reports what it does through `logging`, under this package's
own logger: `yggdryl.iceberg.table` and its siblings are the Rust module
paths the work happens in, so `logging.getLogger("yggdryl")` is the one switch.
Debug is an operation starting, info is one done and carries the counts a
monitor watches - a table opened, a scan planned and what its filters pruned,
a snapshot's rows and files, a commit and the version it landed. Nothing is
reported per row, per batch, or per file. A level changed after import reaches
the bridge through `refresh_logging`.
"""

from . import (
    arrow,
    charset,
    coding,
    enums,
    expression,
    fix,
    hashing,
    holder,
    market,
    media,
    text,
    types,
    uri,
)
from ._native import (
    DEFAULT_FETCH_BYTE_SIZE,
    DEFAULT_STREAM_BATCH_SIZE,
    IPC_DICTIONARY_IDS_KEY,
    ArrowScalar,
    __version__,
    combined,
    refresh_logging,
)
from .expression import (
    Bound,
    Bounds,
    BoundSelector,
    Expression,
    Filter,
    Plan,
    Records,
    Selector,
    Term,
)
from .holder import IOBase, IOCursor
from .media import (
    MediaType,
    MimeType,
    RecordOptions,
    TextEntries,
    TextEntry,
    TextLine,
    TextLines,
    TextOptions,
)
from .types import (
    ArrowCastPlan,
    BytesParameters,
    DataType,
    Field,
    FieldPath,
    ProtocolField,
    PythonMetadata,
    Scalar,
    StringEnum,
    StringParameters,
    Timezone,
    Version,
    field,
)
from .types.scalar import scalar
from .uri import Parameters, Uri, Url, Urn

__all__ = [
    "ArrowCastPlan",
    "ArrowScalar",
    "BytesParameters",
    "Bound",
    "Bounds",
    "BoundSelector",
    "DataType",
    "Expression",
    "Filter",
    "Field",
    "FieldPath",
    "IOBase",
    "IOCursor",
    "MediaType",
    "MimeType",
    "Parameters",
    "Plan",
    "ProtocolField",
    "PythonMetadata",
    "RecordOptions",
    "Records",
    "Scalar",
    "Selector",
    "TextEntries",
    "TextEntry",
    "TextLine",
    "TextLines",
    "TextOptions",
    "Term",
    "StringEnum",
    "StringParameters",
    "Timezone",
    "Version",
    "Uri",
    "Url",
    "Urn",
    "__version__",
    "arrow",
    "charset",
    "coding",
    "DEFAULT_FETCH_BYTE_SIZE",
    "DEFAULT_STREAM_BATCH_SIZE",
    "IPC_DICTIONARY_IDS_KEY",
    "combined",
    "enums",
    "expression",
    "field",
    "fix",
    "hashing",
    "holder",
    "market",
    "media",
    "refresh_logging",
    "scalar",
    "text",
    "types",
    "uri",
]
