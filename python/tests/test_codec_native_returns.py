"""Exact native-Value returns from every structured decode shape."""

from __future__ import annotations

import io
import pathlib

import pytest

from yggdryl import Field, IOBase, Value, json, toml, yaml
from yggdryl import _codec


@pytest.mark.parametrize("format", [json, yaml])
def test_cls_value_preserves_the_core_scalar(format: object) -> None:
    native = format.loads("1", cls=Value)  # type: ignore[attr-defined]

    assert isinstance(native, Value)
    assert native.kind == "u64"
    assert format.loads("1") == 1  # type: ignore[attr-defined]


def test_cls_value_preserves_typed_toml_root_canonicalization() -> None:
    field = Field(
        "row",
        "struct<value: float16 not null>",
        False,
    )

    native = toml.loads("value = 1.5", field=field, cls=Value)

    assert isinstance(native, Value)
    assert native.kind == "sequence"
    assert toml.loads("value = 1.5", field=field) == {"value": 1.5}


@pytest.mark.parametrize(
    ("format", "content"),
    [
        (json, "1.5\n2.5\n"),
        (yaml, "1\n---\n2\n"),
    ],
)
def test_all_and_lazy_streams_return_native_values(format: object, content: str) -> None:
    eager = list(  # type: ignore[attr-defined]
        format.loads_all(content, cls=Value)
    )
    lazy = list(  # type: ignore[attr-defined]
        format.load_all(io.StringIO(content), cls=Value)
    )

    expected_kind = "f64" if format is json else "u64"
    assert [value.kind for value in eager] == [expected_kind, expected_kind]
    assert [value.kind for value in lazy] == [expected_kind, expected_kind]
    assert all(isinstance(value, Value) for value in [*eager, *lazy])


@pytest.mark.parametrize("source", [b"1", "1"])
def test_inferred_decode_returns_the_native_value_directly(source: bytes | str) -> None:
    native = _codec._decode_inferred(source, cls=Value)

    assert isinstance(native, Value)
    assert native.kind == "u64"


def test_single_reader_and_path_return_native_values(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.json"
    path.write_bytes(b"1")

    reader_value = json.loads(io.BytesIO(b"1"), cls=Value)
    path_value = json.loads(path, cls=Value)

    assert reader_value.kind == "u64"
    assert path_value.kind == "u64"


def test_iobase_read_value_selects_exact_value_without_changing_default() -> None:
    handle = IOBase.from_bytes(b"1")
    handle.media_type = "application/json"

    assert handle.read_value() == 1
    native = handle.read_value(cls=Value)
    assert isinstance(native, Value)
    assert native.kind == "u64"

    with pytest.raises(TypeError, match="cls must be Value or None"):
        handle.read_value(cls=float)
