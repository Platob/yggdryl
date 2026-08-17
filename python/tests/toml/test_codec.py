from __future__ import annotations

import dataclasses
import datetime as dt
import io
import os
import pathlib
import uuid
from collections import OrderedDict, deque
from decimal import Decimal

import pytest

from yggdryl import DataType, Field, toml
from yggdryl.records import record


@record(frozen=True, slots=True)
class Fill:
    price: Decimal
    observed_at: dt.datetime


@record(frozen=True, slots=True)
class Order:
    order_id: int
    fill: Fill
    labels: tuple[str, ...] = ()


def test_public_surface_is_deliberately_single_document() -> None:
    assert toml.load is toml.loads
    for name in ("dump_all", "dumps_all", "load_all", "loads_all"):
        assert not hasattr(toml, name)


def test_plain_table_and_enveloped_roots_round_trip() -> None:
    table = {"name": "yggdryl", "enabled": True, "count": 42}
    assert toml.loads(b"") == {}
    encoded = toml.dumps(table)
    assert b"$yggdryl" not in encoded
    assert toml.loads(encoded) == table

    for value in (None, 42, [1, 2], {1: "one"}):
        encoded = toml.dumps(value)
        assert b"$yggdryl" in encoded
        assert toml.loads(encoded) == value


def test_reserved_envelope_shaped_mapping_remains_user_data() -> None:
    value = {"$yggdryl": {"version": 1, "type": "null"}}

    assert toml.loads(toml.dumps(value)) == value


def test_exact_scalars_round_trip_through_toml_syntax_or_an_envelope() -> None:
    value = {
        "payload": b"\x00\xff",
        "decimal": Decimal("123.4500"),
        "date": dt.date(2026, 8, 15),
        "time": dt.time(12, 3, 4, 5),
        "datetime": dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        "zoned": dt.datetime(2026, 8, 15, 12, tzinfo=dt.timezone.utc),
        "delta": dt.timedelta(days=-2, seconds=3, microseconds=4),
    }

    encoded = toml.dumps(value)
    restored = toml.loads(encoded)

    assert isinstance(encoded, bytes)
    # A date, a time, and a datetime have TOML syntax of their own and use it,
    # so they come back typed.
    assert {name: restored[name] for name in value if name != "delta"} == {
        name: item for name, item in value.items() if name != "delta"
    }
    assert b'"date" = 2026-08-15\n' in encoded
    assert b'"datetime" = 2026-08-15T12:03:04.000005\n' in encoded
    assert b'"zoned" = 2026-08-15T12:00:00Z\n' in encoded
    # A decimal has no TOML syntax and takes the envelope; a duration spells
    # its classic ISO string and comes back as that string.
    assert b'type = "decimal"' in encoded
    assert b'"delta" = "-PT172796.999996S"\n' in encoded
    assert restored["delta"] == "-PT172796.999996S"


def test_values_without_a_native_shape_lower_and_lose_their_class() -> None:
    value = {
        "huge": 1 << 300,
        "uuid": uuid.UUID("12345678-1234-5678-1234-567812345678"),
        "tuple": (1, "two"),
        "set": {3, 1, 2},
        "frozen": frozenset(("a", "b")),
        "deque": deque((1, 2), maxlen=4),
        "ordered": OrderedDict((("b", 2), ("a", 1))),
        "path": pathlib.PurePosixPath("nested/file.arrow"),
        "datatype": DataType("list<int64>"),
        "field": Field("price", "decimal(18,4)", nullable=False),
    }

    restored = toml.loads(toml.dumps(value))

    assert restored == {
        "huge": str(1 << 300),
        "uuid": "12345678-1234-5678-1234-567812345678",
        "tuple": [1, "two"],
        "set": [1, 2, 3],
        "frozen": ["a", "b"],
        "deque": [1, 2],
        "ordered": {"b": 2, "a": 1},
        "path": "nested/file.arrow",
        "datatype": str(value["datatype"]),
        "field": str(value["field"]),
    }
    # Insertion order is the mapping's, not the alphabet's.
    assert list(restored["ordered"]) == ["b", "a"]


def test_dataclass_reconstruction_requires_an_explicit_target() -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    value = Point(2, 3)
    encoded = toml.dumps(value)

    assert b"python:" not in encoded
    assert toml.loads(encoded) == {"x": 2, "y": 3}
    assert toml.loads(encoded, cls=Point) == value


def test_native_toml_temporals_decode_as_python_temporals() -> None:
    source = b"""
offset = 1979-05-27T07:32:00Z
local_datetime = 1979-05-27T07:32:00
local_date = 1979-05-27
local_time = 07:32:00
"""

    decoded = toml.loads(source)

    assert decoded == {
        "offset": dt.datetime(1979, 5, 27, 7, 32, tzinfo=dt.timezone.utc),
        "local_datetime": dt.datetime(1979, 5, 27, 7, 32),
        "local_date": dt.date(1979, 5, 27),
        "local_time": dt.time(7, 32),
    }
    # Each one writes back as the same TOML syntax it was read from.
    assert toml.dumps(decoded).split(b"\n")[:4] == [
        b'"offset" = 1979-05-27T07:32:00Z',
        b'"local_datetime" = 1979-05-27T07:32:00',
        b'"local_date" = 1979-05-27',
        b'"local_time" = 07:32:00',
    ]


def test_nested_record_uses_shared_safe_caster() -> None:
    value = Order(
        7,
        Fill(Decimal("12.50"), dt.datetime(2026, 8, 15, 8)),
        ("urgent", "auction"),
    )

    encoded = value.into_toml()

    assert isinstance(encoded, bytes)
    assert Order.from_toml(encoded) == value
    assert Order.from_(encoded, format="toml") == value
    shallow = Order.from_toml(encoded, safe=False)
    assert shallow.order_id == value.order_id
    assert shallow.fill == {
        "price": value.fill.price,
        "observed_at": value.fill.observed_at,
    }


def test_path_content_and_text_binary_sources(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "value.toml"
    path.write_bytes(b"value = 42\n")

    assert toml.loads(path) == {"value": 42}
    assert toml.loads(str(path)) == {"value": 42}
    assert toml.loads("value = 43") == {"value": 43}
    assert toml.loads(bytearray(b"value = 43\n")) == {"value": 43}
    assert toml.loads(memoryview(b"value = 43\n")) == {"value": 43}
    binary = io.BytesIO(b"value = 44\n")
    text = io.StringIO("value = 45\n")
    assert toml.load(binary) == {"value": 44}
    assert toml.load(text) == {"value": 45}
    assert not binary.closed
    assert not text.closed


def test_pathlike_is_always_a_path_and_bytes_pathlike_is_supported(
    tmp_path: pathlib.Path,
) -> None:
    missing = tmp_path / "missing.toml"
    with pytest.raises(FileNotFoundError):
        toml.loads(missing)

    class BytesPath(os.PathLike[bytes]):
        def __init__(self, value: bytes) -> None:
            self.value = value

        def __fspath__(self) -> bytes:
            return self.value

    destination = BytesPath(os.fsencode(tmp_path / "bytes-path.toml"))
    toml.dump({"value": 46}, destination)
    assert toml.load(destination) == {"value": 46}


def test_nonexistent_string_is_content_not_a_path() -> None:
    with pytest.raises(ValueError) as caught:
        toml.loads("missing/value.toml")

    assert not isinstance(caught.value, FileNotFoundError)


def test_dump_supports_partial_binary_and_text_writers() -> None:
    class PartialBinaryWriter:
        def __init__(self) -> None:
            self.parts: list[bytes] = []

        def write(self, value: bytes) -> int:
            count = min(3, len(value))
            self.parts.append(value[:count])
            return count

    class PartialTextWriter:
        def __init__(self) -> None:
            self.parts: list[str] = []
            self.binary_attempts = 0

        def write(self, value: bytes | str) -> int:
            if isinstance(value, bytes):
                self.binary_attempts += 1
                raise TypeError("text only")
            count = min(3, len(value))
            self.parts.append(value[:count])
            return count

    value = {"label": "\u0065\u0301\U0001F642" * 5_000}
    binary = PartialBinaryWriter()
    text = PartialTextWriter()
    toml.dump(value, binary)
    toml.dump(value, text)

    assert toml.loads(b"".join(binary.parts)) == value
    assert toml.loads("".join(text.parts)) == value
    assert text.binary_attempts == 1


def test_writer_error_after_progress_is_preserved_without_text_retry() -> None:
    class FailingWriter:
        def __init__(self) -> None:
            self.calls: list[bytes | str] = []

        def write(self, value: bytes | str) -> int:
            self.calls.append(value)
            if len(self.calls) == 1:
                assert isinstance(value, bytes)
                return min(2, len(value))
            raise TypeError("deliberate failure")

    destination = FailingWriter()
    with pytest.raises(TypeError, match="deliberate failure"):
        toml.dump({"value": 42}, destination)
    assert destination.calls
    assert all(isinstance(value, bytes) for value in destination.calls)


def test_conversion_error_does_not_truncate_existing_path(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "existing.toml"
    path.write_bytes(b"keep me")
    cyclic: list[object] = []
    cyclic.append(cyclic)

    with pytest.raises(ValueError, match="cyclic"):
        toml.dump(cyclic, path)

    assert path.read_bytes() == b"keep me"


def test_toml_wire_depth_error_does_not_truncate_existing_path(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "existing.toml"
    path.write_bytes(b"keep me")
    value: object = None
    for index in range(32):
        value = {index: value}

    with pytest.raises(ValueError, match="hard limit"):
        toml.dump(value, path)

    assert path.read_bytes() == b"keep me"


def test_str_and_io_redirect_without_python_byte_staging(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    def staged_read_forbidden(source: object) -> bytes:
        raise AssertionError(f"unexpected Python staging for {source!r}")

    monkeypatch.setattr(codec, "read_bytes", staged_read_forbidden)
    label = "caf\u0065\u0301"
    document = f'label = "{label}"'
    assert toml.loads(document) == {"label": label}
    assert toml.load(io.StringIO(document)) == {
        "label": label
    }
    assert toml.load(io.BytesIO(document.encode())) == {
        "label": label
    }


def test_reader_rejects_oversized_return_before_copying() -> None:
    class OversizedReader:
        def read(self, size: int) -> bytes:
            return b" " * (size + 1)

    with pytest.raises(OSError, match="more data than requested"):
        toml.load(OversizedReader())


def test_reader_error_is_preserved() -> None:
    class FailingReader:
        def read(self, size: int) -> bytes:
            assert size > 0
            raise RuntimeError("deliberate read failure")

    with pytest.raises(RuntimeError, match="deliberate read failure"):
        toml.load(FailingReader())


def test_invalid_duplicate_and_trailing_documents_are_rejected() -> None:
    invalid = (
        b"value = [1,\n",
        b"value = 1\nvalue = 2\n",
        b"value = 1\n---\nvalue = 2\n",
    )
    for source in invalid:
        with pytest.raises(ValueError, match="toml"):
            toml.loads(source)


def test_deep_nested_table_round_trip() -> None:
    value: dict[str, object] = {"value": 42}
    for index in range(24):
        value = {f"level_{index}": value}

    assert toml.loads(toml.dumps(value)) == value


def test_generic_record_path_inference_and_application_alias(
    tmp_path: pathlib.Path,
) -> None:
    value = Order(8, Fill(Decimal("1.25"), dt.datetime(2026, 8, 15)))
    path = tmp_path / "order.toml"

    assert value.into_(path) is None
    assert Order.from_(path) == value
    encoded = value.into_(format="application/toml")
    assert isinstance(encoded, bytes)
    assert Order.from_(encoded, format="application/toml") == value
    assert Order.from_(encoded, format=".toml") == value


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (b"", "yaml"),
        (b"  # YAML comment only\n", "yaml"),
        (b"[1, 2]", "json"),
        (b"key = 1\n", "toml"),
        (b"[table]\nkey = 1\n", "toml"),
        (b"key: 1\n", "yaml"),
    ],
)
def test_generic_content_inference_is_stable(
    source: bytes, expected: str
) -> None:
    from yggdryl._codec import infer_format

    assert infer_format(source) == expected


def test_generic_record_inference_parses_retained_content_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    value = Order(9, Fill(Decimal("3.50"), dt.datetime(2026, 8, 15)))

    import yggdryl._codec as codec

    def legacy_two_pass_route_forbidden(*args: object) -> object:
        raise AssertionError(f"legacy inference/decode route called with {args!r}")

    monkeypatch.setattr(
        codec._native, "_codec_infer", legacy_two_pass_route_forbidden
    )
    monkeypatch.setattr(
        codec._native, "_codec_decode", legacy_two_pass_route_forbidden
    )
    monkeypatch.setattr(
        codec._native, "_codec_infer_text", legacy_two_pass_route_forbidden
    )
    monkeypatch.setattr(
        codec._native, "_codec_decode_text", legacy_two_pass_route_forbidden
    )

    class ReadOnce:
        def __init__(self, payload: bytes) -> None:
            self.payload = payload
            self.calls = 0

        def read(self, size: int) -> bytes:
            assert size > 0
            self.calls += 1
            if self.calls == 1:
                return self.payload
            if self.calls == 2:
                return b""
            raise AssertionError("anonymous stream was read again")

    source = ReadOnce(value.into_toml())
    assert Order.from_(source) == value
    assert source.calls == 2
    assert Order.from_(value.into_toml()) == value
    assert Order.from_(value.into_toml().decode("utf-8")) == value
