"""Reproducible Python-boundary codec baselines using only stdlib timing."""

from __future__ import annotations

import argparse
import gc
import io
import statistics
import timeit
from collections.abc import Callable
from decimal import Decimal

from yggdryl import json, toml, yaml
from yggdryl.records import record


@record(frozen=True, slots=True)
class Leg:
    symbol: str
    price: Decimal


@record(frozen=True, slots=True)
class Order:
    order_id: int
    legs: tuple[Leg, ...]


VALUE = Order(42, tuple(Leg(f"S{index}", Decimal("12.50")) for index in range(8)))
JSON = VALUE.into_json()
TOML = VALUE.into_toml()
YAML = VALUE.into_yaml()
JSON_TEXT = JSON.decode("utf-8")
TOML_TEXT = TOML.decode("utf-8")
YAML_TEXT = YAML.decode("utf-8")
JSON_READER = io.BytesIO(JSON)
TOML_READER = io.BytesIO(TOML)
YAML_READER = io.BytesIO(YAML)
JSON_WRITER = io.BytesIO()
TOML_WRITER = io.BytesIO()
YAML_WRITER = io.BytesIO()


def _deep_fixture() -> object:
    value: object = {"value": 42}
    for index in range(20):
        if index % 2 == 0:
            value = {f"level_{index}": value}
        else:
            value = [value]
    return value


DEEP = _deep_fixture()
ROWS = tuple({"id": index, "price": Decimal("12.50")} for index in range(32))
JSON_LINES = json.dumps_all(ROWS)
YAML_DOCUMENTS = yaml.dumps_all(ROWS)


def _decode_json_reader() -> object:
    JSON_READER.seek(0)
    return json.load(JSON_READER)


def _decode_yaml_reader() -> object:
    YAML_READER.seek(0)
    return yaml.load(YAML_READER)


def _decode_toml_reader() -> object:
    TOML_READER.seek(0)
    return toml.load(TOML_READER)


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
        _measure("record into JSON", VALUE.into_json, args.iterations)
        _measure("record from JSON", lambda: Order.from_json(JSON), args.iterations)
        _measure("record into TOML", VALUE.into_toml, args.iterations)
        _measure("record from TOML", lambda: Order.from_toml(TOML), args.iterations)
        _measure("record into YAML", VALUE.into_yaml, args.iterations)
        _measure("record from YAML", lambda: Order.from_yaml(YAML), args.iterations)
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
            "YAML stream decode",
            lambda: list(yaml.loads_all(YAML_DOCUMENTS)),
            args.iterations,
        )
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
