"""Tests for Io.json() — parsing a handle's bytes as JSON in Rust."""

import pytest

import yggdryl


def test_bytesio_json():
    io = yggdryl.BytesIO(b'{"n": 42, "xs": [1, 2], "ok": true, "nil": null}')
    assert io.json() == {"n": 42, "xs": [1, 2], "ok": True, "nil": None}


def test_localpath_json(tmp_path):
    path = str(tmp_path / "data.json")
    yggdryl.LocalPath(path).write(b'{"a": [1, 2, 3], "b": "x"}')
    assert yggdryl.LocalPath(path).json() == {"a": [1, 2, 3], "b": "x"}


def test_invalid_json_raises():
    with pytest.raises(ValueError):
        yggdryl.BytesIO(b"{not json").json()
