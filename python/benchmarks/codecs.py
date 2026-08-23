"""Reproducible Python-boundary codec baselines using only stdlib timing."""

from __future__ import annotations

import argparse
import copy
import gc
import io
import pickle
import statistics
import timeit
from collections.abc import Callable
from decimal import Decimal

from yggdryl import IOBase, Value, avro, codec, json, scalar, toml, yaml


@scalar(frozen=True, slots=True)
class Leg:
    symbol: str
    price: Decimal


@scalar(frozen=True, slots=True)
class Order:
    order_id: int
    legs: tuple[Leg, ...]


VALUE = Order(42, tuple(Leg(f"S{index}", Decimal("12.50")) for index in range(8)))
JSON = json.dumps(VALUE)
TOML = toml.dumps(VALUE)
YAML = yaml.dumps(VALUE)
JSON_TEXT = JSON.decode("utf-8")
TOML_TEXT = TOML.decode("utf-8")
YAML_TEXT = YAML.decode("utf-8")
JSON_READER = io.BytesIO(JSON)
TOML_READER = io.BytesIO(TOML)
YAML_READER = io.BytesIO(YAML)
JSON_WRITER = io.BytesIO()
TOML_WRITER = io.BytesIO()
YAML_WRITER = io.BytesIO()

JSON_HANDLE = IOBase.from_bytes(JSON)
JSON_HANDLE.media_type = "application/json"
JSON_WRITE_HANDLE = IOBase.from_bytes()
JSON_WRITE_HANDLE.media_type = "application/json"

AVRO_SCHEMA_DOCUMENT = {
    "type": "record",
    "name": "quote",
    "fields": [
        {"name": "id", "type": "long"},
        {"name": "symbol", "type": "string"},
    ],
}
AVRO_SCHEMA = avro.Schema(AVRO_SCHEMA_DOCUMENT)
AVRO_SCHEMA_COPY = avro.Schema(AVRO_SCHEMA_DOCUMENT)
AVRO_ROWS = tuple(
    {"id": index, "symbol": f"SYM{index % 20}"} for index in range(32)
)
AVRO_CONTAINER = avro.dumps(AVRO_ROWS, AVRO_SCHEMA)
AVRO_DECODED = avro.loads(AVRO_CONTAINER)
AVRO_SINGLE = avro.dumps_single(AVRO_ROWS[0], AVRO_SCHEMA)


def _deep_fixture() -> object:
    value: object = {"value": 42}
    for index in range(20):
        if index % 2 == 0:
            value = {f"level_{index}": value}
        else:
            value = [value]
    # TOML requires a record at the document root; keep the nested payload
    # identical across all three format measurements beneath one root key.
    return {"root": value}


DEEP = _deep_fixture()
ROWS = tuple({"id": index, "price": Decimal("12.50")} for index in range(32))
JSON_LINES = json.dumps_all(ROWS)
YAML_DOCUMENTS = yaml.dumps_all(ROWS)
JSON_LINES_READER = io.BytesIO(JSON_LINES)
YAML_DOCUMENT_READER = io.BytesIO(YAML_DOCUMENTS)


def _decode_json_reader() -> object:
    JSON_READER.seek(0)
    return json.load(JSON_READER)


def _decode_yaml_reader() -> object:
    YAML_READER.seek(0)
    return yaml.load(YAML_READER)


def _decode_toml_reader() -> object:
    TOML_READER.seek(0)
    return toml.load(TOML_READER)


def _decode_json_lines_reader() -> object:
    JSON_LINES_READER.seek(0)
    return list(json.load_all(JSON_LINES_READER))


def _decode_yaml_document_reader() -> object:
    YAML_DOCUMENT_READER.seek(0)
    return list(yaml.load_all(YAML_DOCUMENT_READER))


def _write_json_stream() -> None:
    JSON_WRITER.seek(0)
    JSON_WRITER.truncate()
    json.dump(VALUE, JSON_WRITER)


def _write_yaml_stream() -> None:
    YAML_WRITER.seek(0)
    YAML_WRITER.truncate()
    yaml.dump(VALUE, YAML_WRITER)


def _write_toml_stream() -> None:
    TOML_WRITER.seek(0)
    TOML_WRITER.truncate()
    toml.dump(VALUE, TOML_WRITER)


def _write_json_handle() -> None:
    JSON_WRITE_HANDLE.write_value(VALUE)


def _decode_first_avro_block() -> object:
    return next(avro.blocks(AVRO_CONTAINER)).rows()


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    median = statistics.median(samples)
    print(f"{name:28} {median * 1_000_000_000 / iterations:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    gc.disable()
    try:
        bulk = max(1, args.iterations // 100)
        _measure("field class into JSON", lambda: json.dumps(VALUE), args.iterations)
        _measure(
            "field class from JSON",
            lambda: json.loads(JSON, cls=Order),
            args.iterations,
        )
        _measure("field class into TOML", lambda: toml.dumps(VALUE), args.iterations)
        _measure(
            "field class from TOML",
            lambda: toml.loads(TOML, cls=Order),
            args.iterations,
        )
        _measure("field class into YAML", lambda: yaml.dumps(VALUE), args.iterations)
        _measure(
            "field class from YAML",
            lambda: yaml.loads(YAML, cls=Order),
            args.iterations,
        )
        _measure(
            "JSON borrowed str decode",
            lambda: json.loads(JSON_TEXT),
            args.iterations,
        )
        _measure(
            "TOML borrowed str decode",
            lambda: toml.loads(TOML_TEXT),
            args.iterations,
        )
        _measure(
            "YAML borrowed str decode",
            lambda: yaml.loads(YAML_TEXT),
            args.iterations,
        )
        _measure("JSON bytes decode", lambda: json.loads(JSON), args.iterations)
        _measure("TOML bytes decode", lambda: toml.loads(TOML), args.iterations)
        _measure("YAML bytes decode", lambda: yaml.loads(YAML), args.iterations)
        _measure("generic inferred decode", lambda: codec.from_io(JSON), args.iterations)
        _measure(
            "JSON bounded decode",
            lambda: json.loads(
                JSON,
                max_depth=128,
                max_input_bytes=len(JSON),
                max_nodes=1_000_000,
                max_documents=1,
            ),
            args.iterations,
        )
        _measure(
            "JSON exact Value decode",
            lambda: json.loads(JSON, cls=Value),
            args.iterations,
        )
        _measure(
            "TOML exact Value decode",
            lambda: toml.loads(TOML, cls=Value),
            args.iterations,
        )
        _measure(
            "YAML exact Value decode",
            lambda: yaml.loads(YAML, cls=Value),
            args.iterations,
        )
        _measure("JSON reader redirect", _decode_json_reader, args.iterations)
        _measure("TOML reader redirect", _decode_toml_reader, args.iterations)
        _measure("YAML reader redirect", _decode_yaml_reader, args.iterations)
        _measure(
            "JSON buffered bytes dump",
            lambda: json.dumps(VALUE),
            args.iterations,
        )
        _measure(
            "TOML buffered bytes dump",
            lambda: toml.dumps(VALUE),
            args.iterations,
        )
        _measure(
            "YAML buffered bytes dump",
            lambda: yaml.dumps(VALUE),
            args.iterations,
        )
        _measure(
            "generic default JSON dump",
            lambda: codec.into_io(VALUE),
            args.iterations,
        )
        _measure(
            "JSON indent=2 dump",
            lambda: json.dumps(VALUE, indent=2),
            args.iterations,
        )
        _measure(
            "YAML flow dump",
            lambda: yaml.dumps(VALUE, indent=None),
            args.iterations,
        )
        _measure(
            "TOML indent=2 dump",
            lambda: toml.dumps(VALUE, indent=2),
            args.iterations,
        )
        _measure("JSON writer redirect", _write_json_stream, args.iterations)
        _measure("TOML writer redirect", _write_toml_stream, args.iterations)
        _measure("YAML writer redirect", _write_yaml_stream, args.iterations)
        _measure("deep JSON encode", lambda: json.dumps(DEEP), args.iterations)
        _measure("deep TOML encode", lambda: toml.dumps(DEEP), args.iterations)
        _measure("deep YAML encode", lambda: yaml.dumps(DEEP), args.iterations)
        _measure(
            "JSON Lines decode",
            lambda: list(json.loads_all(JSON_LINES)),
            args.iterations,
        )
        _measure(
            "YAML documents decode",
            lambda: list(yaml.loads_all(YAML_DOCUMENTS)),
            args.iterations,
        )
        _measure(
            "JSON Lines reader stream",
            _decode_json_lines_reader,
            args.iterations,
        )
        _measure(
            "YAML reader stream",
            _decode_yaml_document_reader,
            args.iterations,
        )
        _measure("IOBase read Value", JSON_HANDLE.read_value, args.iterations)
        _measure(
            "IOBase read exact Value",
            lambda: JSON_HANDLE.read_value(cls=Value),
            args.iterations,
        )
        _measure("IOBase write Value", _write_json_handle, args.iterations)
        _measure(
            "Avro schema parse",
            lambda: avro.Schema(AVRO_SCHEMA_DOCUMENT),
            args.iterations,
        )
        _measure("Avro canonical form", AVRO_SCHEMA.into_canonical_form, args.iterations)
        _measure("Avro schema document", AVRO_SCHEMA.into_json, args.iterations)
        _measure("Avro schema stable hash", AVRO_SCHEMA.stable_hash, args.iterations)
        _measure("Avro schema hash", lambda: hash(AVRO_SCHEMA), args.iterations)
        _measure("Avro schema equality", lambda: AVRO_SCHEMA == AVRO_SCHEMA_COPY, args.iterations)
        _measure("Avro schema copy", lambda: copy.copy(AVRO_SCHEMA), args.iterations)
        _measure("Avro container encode", lambda: avro.dumps(AVRO_ROWS, AVRO_SCHEMA), bulk)
        _measure("Avro container decode", lambda: avro.loads(AVRO_CONTAINER), bulk)
        _measure("Avro container stable hash", AVRO_DECODED.stable_hash, args.iterations)
        _measure("Avro container hash", lambda: hash(AVRO_DECODED), args.iterations)
        _measure("Avro container pickle", lambda: pickle.dumps(AVRO_DECODED), bulk)
        _measure(
            "Avro block iterator open",
            lambda: avro.blocks(AVRO_CONTAINER),
            bulk,
        )
        _measure("Avro first block decode", _decode_first_avro_block, bulk)
        _measure(
            "Avro single encode",
            lambda: avro.dumps_single(AVRO_ROWS[0], AVRO_SCHEMA),
            args.iterations,
        )
        _measure(
            "Avro single decode",
            lambda: avro.loads_single(AVRO_SINGLE, AVRO_SCHEMA),
            args.iterations,
        )
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
