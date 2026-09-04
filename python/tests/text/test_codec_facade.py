"""Public inferred structured-codec facade tests."""

from __future__ import annotations

import io
import pathlib

import pytest

from yggdryl import Field, Scalar
from yggdryl.text import codec


def test_from_io_infers_content_once_and_keeps_exact_field_casting() -> None:
    exact = codec.from_io(
        b'{"id":7}',
        cls=Scalar,
        field=Field("row", "struct<id: int16>", nullable=False),
        max_depth=2,
        max_input_bytes=8,
        max_nodes=3,
        max_documents=1,
    )
    assert isinstance(exact, Scalar)
    assert exact.at(0) is not None and exact.at(0).kind == "i16"


def test_from_io_and_into_io_infer_declared_path_suffixes(
    tmp_path: pathlib.Path,
) -> None:
    target = tmp_path / "value.yaml"
    assert codec.into_io({"answer": 42}, target, indent=2) is None
    assert codec.from_io(target) == {"answer": 42}

    with pytest.raises(ValueError, match="contradicts"):
        codec.into_io({"answer": 42}, target, format="toml")
    with pytest.raises(ValueError, match="contradicts"):
        codec.from_io(target, format="json")


def test_into_io_defaults_anonymous_output_to_json() -> None:
    assert codec.into_io({"answer": 42}) == b'{"answer":42}'
    assert codec.into_io({"answer": 42}, utf8=True, indent=2) == (
        '{\n  "answer": 42\n}'
    )

    destination = io.BytesIO()
    codec.into_stream({"answer": 42}, destination)
    assert destination.getvalue() == b'{"answer":42}'


def test_explicit_json_lines_redirects_to_collection_and_lazy_stream_paths() -> None:
    rows = ({"id": 1}, {"id": 2})
    encoded = codec.into_io(rows, format="json_lines")
    assert encoded == b'{"id":1}\n{"id":2}\n'
    assert list(codec.from_io(encoded, format="json_lines")) == list(rows)
    assert list(
        codec.from_stream(
            io.BytesIO(encoded), format="json_lines", max_documents=2
        )
    ) == list(rows)

    destination = io.BytesIO()
    codec.into_stream(rows, destination, format="json_lines")
    assert destination.getvalue() == encoded


def test_stream_yaml_is_lazy_multi_document_and_single_formats_are_values() -> None:
    yaml_documents = b"id: 1\n---\nid: 2\n"
    assert list(codec.from_stream(io.BytesIO(yaml_documents), format="yaml")) == [
        {"id": 1},
        {"id": 2},
    ]
    assert codec.from_stream(io.BytesIO(b'{"id":1}'), format="json") == {
        "id": 1
    }


def test_anonymous_stream_inference_is_one_document_and_bounded() -> None:
    assert codec.from_io(io.BytesIO(b"answer = 42\n")) == {"answer": 42}
    with pytest.raises(ValueError, match="exceeds|limit"):
        codec.from_io(io.BytesIO(b'{"answer":42}'), max_input_bytes=2)
