import pyarrow  # type: ignore[import-untyped]

from ._native import (
    DataType,
    FieldLike,
    TxHash as TxHash,
    TxHasher as TxHasher,
    UnixLike,
    txh3 as txh3,
    txh128 as txh128,
    txh32 as txh32,
    txh64 as txh64,
)

DEFAULT_UNIT: str
UNIX_WIDTH: int

def digest(data: str | bytes | bytearray | memoryview, unix: UnixLike, algorithm: str) -> TxHash: ...
def row_txhashes(
    batch: pyarrow.RecordBatch,
    times: pyarrow.Array,
    unit: str = "us",
    algorithm: str = "xxh3-64",
) -> pyarrow.Array: ...
def column_txhashes(
    times: pyarrow.Array,
    array: pyarrow.Array,
    field: FieldLike,
    unit: str = "us",
    algorithm: str = "xxh3-64",
) -> pyarrow.Array: ...
def compose(
    times: pyarrow.Array,
    digests: pyarrow.Array,
    unit: str = "us",
    algorithm: str = "xxh3-64",
) -> pyarrow.Array: ...
def decompose(
    array: pyarrow.Array, unit: str = "us", algorithm: str = "xxh3-64"
) -> tuple[pyarrow.Array, pyarrow.Array]: ...
def unix_array(array: pyarrow.Array, unit: str = "us") -> pyarrow.Array: ...
def unix_now(unit: str = "us") -> int: ...
def unix_of(value: UnixLike, unit: str = "us") -> int: ...
def restate_unix(count: int, from_unit: str, into_unit: str) -> int: ...
def width(algorithm: str) -> int: ...
def dtype(algorithm: str) -> DataType: ...

__all__ = [
    "DEFAULT_UNIT",
    "UNIX_WIDTH",
    "TxHash",
    "TxHasher",
    "column_txhashes",
    "compose",
    "decompose",
    "digest",
    "dtype",
    "restate_unix",
    "row_txhashes",
    "txh3",
    "txh128",
    "txh32",
    "txh64",
    "unix_array",
    "unix_now",
    "unix_of",
    "width",
]
