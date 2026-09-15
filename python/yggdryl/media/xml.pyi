from yggdryl._native import Field, Scalar

__all__ = [
    "dumps",
    "loads",
    "loads_with_field",
    "schema",
    "schema_dumps",
]

def loads(
    data: str | bytes | bytearray | memoryview,
    *,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> object: ...
def dumps(value: object, name: str, *, indent: int | None = None) -> bytes: ...
def loads_with_field(
    data: str | bytes | bytearray | memoryview,
    field: object,
    *,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> Scalar: ...
def schema(
    data: str | bytes | bytearray | memoryview,
    *,
    root: str | None = None,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> Field: ...
def schema_dumps(field: object, *, indent: int | None = None) -> bytes: ...
