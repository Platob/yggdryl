from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from ._native import (
    AvroBlock as _AvroBlock,
    AvroBlockIterator as _AvroBlockIterator,
    AvroContainer as _AvroContainer,
    AvroSchema as _AvroSchema,
)

Block = _AvroBlock
BlockIterator = _AvroBlockIterator
Container = _AvroContainer
Schema = _AvroSchema

Buffer = bytes | bytearray | memoryview
SchemaInput = Schema | str | Buffer | object

def loads(
    data: Buffer,
    *,
    reader_schema: Schema | None = None,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> Container: ...
def blocks(
    data: Buffer,
    *,
    reader_schema: Schema | None = None,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> BlockIterator: ...
def dumps(
    rows: Iterable[object],
    schema: SchemaInput,
    *,
    metadata: Mapping[str, str] | Iterable[tuple[str, str]] | None = None,
) -> bytes: ...
def loads_single(
    data: Buffer,
    schema: SchemaInput,
    *,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
) -> Any: ...
def dumps_single(value: object, schema: SchemaInput) -> bytes: ...

__all__: list[str]
