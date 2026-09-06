"""The FIX Arrow reader stays lazy and keeps capture provenance."""

from __future__ import annotations

import pathlib
from collections.abc import Iterator

import pyarrow as pa
import pytest

from yggdryl import fix
from yggdryl.fix import FixRegistry, parse_arrow_reader

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"


@pytest.fixture
def registry() -> FixRegistry:
    """The repository's small FIX dictionary."""
    return FixRegistry.from_handle(SEED)


def _schema(*extra: pa.Field, body_type: pa.DataType = pa.binary()) -> pa.Schema:
    return pa.schema(
        [
            pa.field(
                "url",
                pa.string(),
                nullable=False,
                metadata={b"source": b"capture"},
            ),
            pa.field("rownum", pa.int64(), nullable=False),
            pa.field("branch", pa.string(), nullable=False),
            pa.field("body", body_type, nullable=False),
            pa.field("fixbranch", pa.string(), nullable=True),
            *extra,
        ],
        metadata={b"python.class": b"Message"},
    )


def _batch(schema: pa.Schema, rownum: int) -> pa.RecordBatch:
    body: str | bytes = f"8=FIX.4.4|35=D|11=ORDER-{rownum}|55=AAPL|10=0|"
    if pa.types.is_binary(schema.field("body").type):
        body = body.encode()
    return pa.RecordBatch.from_arrays(
        [
            pa.array(["file:///capture.log"]),
            pa.array([rownum], type=pa.int64()),
            pa.array(["provenance"]),
            pa.array([body], type=schema.field("body").type),
            pa.array([None], type=pa.string()),
        ],
        schema=schema,
    )


def test_arrow_parse_is_lazy_bounded_and_source_first(registry: FixRegistry) -> None:
    schema = _schema()
    pulled: list[int] = []

    def batches() -> Iterator[pa.RecordBatch]:
        for rownum in (1, 2):
            pulled.append(rownum)
            yield _batch(schema, rownum)

    source = pa.RecordBatchReader.from_batches(schema, batches())
    parsed = parse_arrow_reader(
        source,
        registry,
        batch_row_size=1,
        max_row_size=1,
    )

    assert pulled == [], "construction reads the schema, not a source batch"
    assert parsed.schema.names[:5] == [
        "url",
        "rownum",
        "branch",
        "body",
        "fixbranch",
    ]
    assert "35" in parsed.schema.names
    assert parsed.schema.field("url").metadata == {b"source": b"capture"}
    assert b"python.class" not in (parsed.schema.metadata or {})

    batch = parsed.read_next_batch()
    assert pulled == [1]
    assert batch.num_rows == 1
    assert batch.column("rownum").to_pylist() == [1]
    assert batch.column("branch").to_pylist() == ["provenance"]
    assert batch.column("35").null_count == 0
    with pytest.raises(StopIteration):
        parsed.read_next_batch()
    assert pulled == [1], "the result row bound stops before the next source batch"


def test_source_collision_and_zero_batch_bound_fail_before_pull(
    registry: FixRegistry,
) -> None:
    pulled = False

    def batches() -> Iterator[pa.RecordBatch]:
        nonlocal pulled
        pulled = True
        yield _batch(_schema(), 1)

    collision = _schema(pa.field("EnTrIeS", pa.string()))
    source = pa.RecordBatchReader.from_batches(collision, batches())
    with pytest.raises(ValueError, match='duplicate "EnTrIeS"'):
        parse_arrow_reader(source, registry)
    assert not pulled

    source = pa.RecordBatchReader.from_batches(_schema(), batches())
    with pytest.raises(ValueError, match="positive row count"):
        parse_arrow_reader(source, registry, batch_row_size=0)
    assert not pulled


def test_registry_may_use_the_process_default() -> None:
    assert "parse_arrow_reader" in fix.__all__
    source = pa.RecordBatchReader.from_batches(_schema(), [])
    parsed = parse_arrow_reader(source, registry=None)
    assert parsed.schema.names[:5] == [
        "url",
        "rownum",
        "branch",
        "body",
        "fixbranch",
    ]


def test_utf8_payload_is_accepted_too(registry: FixRegistry) -> None:
    schema = _schema(body_type=pa.string())
    source = pa.RecordBatchReader.from_batches(schema, [_batch(schema, 1)])
    parsed = parse_arrow_reader(source, registry)
    assert parsed.read_next_batch().column("35").null_count == 0
