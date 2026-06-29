"""Smoke test for the yggdryl Python binding submodules."""

import yggdryl
from yggdryl import core, schema
from yggdryl.schema import DataTypeId


def test_core_version():
    assert isinstance(core.version(), str)
    assert core.version()


def test_schema_data_type_id():
    assert schema.DataTypeId is DataTypeId
    assert DataTypeId.Int32 != DataTypeId.Utf8
    # hashable → usable as a dict key
    assert {DataTypeId.Int32: 1}[DataTypeId.Int32] == 1
