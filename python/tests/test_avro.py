from __future__ import annotations

import copy
import pickle

import pytest

from yggdryl import RecordOptions, Value, avro


SCHEMA = {
    "type": "record",
    "name": "trade",
    "doc": "one fill",
    "fields": [
        {"name": "symbol", "type": "string", "field-id": 1},
        {"name": "qty", "type": "int", "field-id": 2},
    ],
}


def test_record_options_expose_validated_avro_block_settings() -> None:
    options = RecordOptions("trades.avro")
    assert options.block_codec == "deflate"
    assert options.sync_marker is None

    options.block_codec = "null"
    options.sync_marker = memoryview(b"0123456789abcdef")
    assert options.block_codec == "null"
    assert options.sync_marker == b"0123456789abcdef"
    options.sync_marker = None
    assert options.sync_marker is None

    with pytest.raises(ValueError, match="brotli"):
        options.block_codec = "brotli"
    with pytest.raises(ValueError, match="exactly 16 bytes"):
        options.sync_marker = b"short"

    ipc = RecordOptions("trades.arrows")
    assert ipc.block_codec is None
    assert ipc.sync_marker is None
    with pytest.raises(ValueError, match="expected Avro options"):
        ipc.block_codec = "null"
    with pytest.raises(ValueError, match="expected Avro options"):
        ipc.sync_marker = None


def test_schema_accepts_natural_value_native_value_text_and_bytes() -> None:
    natural = avro.Schema(SCHEMA)
    native = avro.Schema(Value(SCHEMA))
    text = avro.Schema('{"type":"long"}')
    binary = avro.Schema(b'"long"')

    assert natural.into_json() == SCHEMA
    assert native.fingerprint() == natural.fingerprint()
    assert text.fingerprint() == binary.fingerprint()
    assert natural.kind == "record"
    assert "doc" not in natural.into_canonical_form()
    assert natural.into_json()["fields"][0]["field-id"] == 1

    assert avro.Schema(bytearray(b'"long"')).fingerprint() == text.fingerprint()
    assert avro.Schema(memoryview(b'"long"')).fingerprint() == text.fingerprint()


def test_schema_value_protocols_use_the_complete_native_identity() -> None:
    schema = avro.Schema(SCHEMA)
    same = avro.Schema(SCHEMA)

    assert schema == same
    assert schema != schema.into_json()
    assert hash(schema) == hash(same)
    assert schema.stable_hash() == same.stable_hash()
    assert {schema: "held"}[same] == "held"
    assert copy.copy(schema) == schema
    assert copy.deepcopy(schema) == schema
    assert pickle.loads(pickle.dumps(schema)) == schema
    assert eval(repr(schema), {"AvroSchema": avro.Schema}) == schema

    # Parsing Canonical Form deliberately erases logical annotations, but
    # equality and hashing retain everything that affects schema behavior.
    date = avro.Schema({"type": "int", "logicalType": "date"})
    plain = avro.Schema('"int"')
    assert date.fingerprint() == plain.fingerprint()
    assert date != plain
    assert date.stable_hash() != plain.stable_hash()
    assert date < plain or plain < date

    with pytest.raises(TypeError):
        _ = schema < SCHEMA


def test_object_container_round_trips_metadata_rows_and_writer_schema() -> None:
    rows = [
        {"symbol": "AAPL", "qty": 100},
        {"symbol": "MSFT", "qty": 25},
    ]
    encoded = avro.dumps(rows, SCHEMA, metadata={"source": "python"})
    decoded = avro.loads(encoded)

    assert decoded.schema.fingerprint() == avro.Schema(SCHEMA).fingerprint()
    assert decoded.metadata == {"source": "python"}
    assert decoded.rows == rows
    assert len(decoded) == 2
    assert avro.loads(bytearray(encoded)).rows == rows
    assert avro.loads(memoryview(encoded)).rows == rows


def test_object_container_uses_complete_native_value_protocols() -> None:
    encoded = avro.dumps(
        [{"symbol": "AAPL", "qty": 100}],
        SCHEMA,
        metadata={"source": "python"},
    )
    container = avro.loads(encoded)
    same = avro.loads(encoded)

    assert container == same
    assert container != container.rows
    assert container.stable_hash() == same.stable_hash()
    assert hash(container) == hash(same)
    assert {container: "held"}[same] == "held"
    assert copy.copy(container) == container
    assert copy.deepcopy(container) == container
    assert pickle.loads(pickle.dumps(container)) == container
    assert eval(repr(container), {"AvroContainer": type(container)}) == container
    assert container <= same and container >= same


def test_operational_avro_blocks_are_intentionally_unhashable() -> None:
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100}], SCHEMA)
    blocks = avro.blocks(encoded)
    block = next(blocks)

    with pytest.raises(TypeError, match="unhashable"):
        hash(blocks)
    with pytest.raises(TypeError, match="unhashable"):
        hash(block)


def test_block_iterator_stays_lazy_and_fuses_after_exhaustion() -> None:
    rows = [
        {"symbol": "AAPL", "qty": 100},
        {"symbol": "MSFT", "qty": 25},
    ]
    encoded = avro.dumps(rows, SCHEMA, metadata={"source": "python"})
    blocks = avro.blocks(encoded)

    assert blocks.schema.kind == "record"
    assert blocks.metadata == {"source": "python"}
    assert blocks.get("source") == "python"
    block = next(blocks)
    assert block.count == 2
    assert block.size > 0
    assert block.rows() == rows
    with pytest.raises(StopIteration):
        next(blocks)
    with pytest.raises(StopIteration):
        next(blocks)


def test_block_rows_can_apply_one_reader_schema() -> None:
    reader = avro.Schema(
        {
            "type": "record",
            "name": "trade",
            "fields": [
                {"name": "quantity", "aliases": ["qty"], "type": "long"},
                {"name": "note", "type": "string", "default": "none"},
            ],
        }
    )
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100}], SCHEMA)

    block = next(avro.blocks(encoded, reader_schema=reader))
    assert block.rows() == [
        {"note": "none", "quantity": 100}
    ]


def test_block_limits_fail_at_iteration_and_fuse_the_iterator() -> None:
    encoded = avro.dumps(
        [
            {"symbol": "AAPL", "qty": 100},
            {"symbol": "MSFT", "qty": 25},
            {"symbol": "NVDA", "qty": 10},
        ],
        SCHEMA,
    )
    stream = avro.blocks(encoded, max_nodes=2)

    with pytest.raises(ValueError, match="at most 2 rows"):
        next(stream)
    with pytest.raises(StopIteration):
        next(stream)
    with pytest.raises(ValueError, match="at most 1"):
        avro.blocks(encoded, max_input_bytes=1)


def test_reader_schema_resolves_aliases_promotions_and_defaults() -> None:
    reader = avro.Schema(
        {
            "type": "record",
            "name": "trade",
            "fields": [
                {"name": "quantity", "aliases": ["qty"], "type": "long"},
                {"name": "note", "type": "string", "default": "none"},
            ],
        }
    )
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100}], SCHEMA)

    assert avro.loads(encoded, reader_schema=reader).rows == [
        {"note": "none", "quantity": 100}
    ]


def test_single_object_round_trips_and_checks_the_fingerprint() -> None:
    schema = avro.Schema(SCHEMA)
    row = {"symbol": "AAPL", "qty": 100}
    encoded = avro.dumps_single(row, schema)

    assert encoded[:2] == b"\xc3\x01"
    assert avro.loads_single(encoded, schema) == row
    with pytest.raises(ValueError, match="fingerprint"):
        avro.loads_single(encoded, avro.Schema('"long"'))


def test_invalid_schema_and_container_bytes_keep_native_diagnostics() -> None:
    with pytest.raises(ValueError, match="json"):
        avro.Schema("not json")
    with pytest.raises(ValueError, match="Avro"):
        avro.loads(b"not an object container")


def test_decode_limits_redirect_to_the_native_budget() -> None:
    nested = {"type": "array", "items": {"type": "array", "items": "long"}}
    with pytest.raises(ValueError, match="at most 1 levels deep"):
        avro.Schema(nested, max_depth=1)
    with pytest.raises(ValueError, match="at most 1 levels deep"):
        avro.Schema.from_value(nested, max_depth=1)

    rows = [
        {"symbol": "AAPL", "qty": 100},
        {"symbol": "MSFT", "qty": 25},
    ]
    encoded = avro.dumps(rows, SCHEMA)
    with pytest.raises(ValueError, match="container of at most"):
        avro.loads(encoded, max_input_bytes=len(encoded) - 1)
    with pytest.raises(ValueError, match="at most 1 rows"):
        avro.loads(encoded, max_nodes=1)

    schema = avro.Schema(SCHEMA)
    framed = avro.dumps_single(rows[0], schema)
    with pytest.raises(ValueError, match="single-object datum of at most"):
        avro.loads_single(framed, schema, max_input_bytes=len(framed) - 1)
