"""The numeric Version boundary and its Scalar/Arrow representations."""

from __future__ import annotations

import copy
import pickle
import typing

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, Version, enums, field, scalar, types
from yggdryl.text import json


@pytest.mark.parametrize(
    ("text", "parts", "canonical"),
    [
        ("0", (0, 0, 0), "0"),
        ("5.0.0", (5, 0, 0), "5"),
        ("05.002.0003", (5, 2, 3), "5.2.3"),
        ("5.0.300", (5, 0, 300), "5.0.300"),
        ("255.255.65535", (255, 255, 65535), "255.255.65535"),
    ],
)
def test_native_parts_and_canonical_numeric_text(text, parts, canonical):
    parsed = Version.from_str(text)
    assert type(parsed) is Version
    assert types.Version is Version
    assert (parsed.major, parsed.minor, parsed.patch) == parts
    assert parsed == Version(*parts)
    assert str(parsed) == canonical
    assert eval(repr(parsed), {"Version": Version}) == parsed
    assert not hasattr(parsed, "tag")
    assert not hasattr(parsed, "qualifier")


@pytest.mark.parametrize(
    "text",
    ["", "1.", ".1", "1..2", "1.2.3.4", "256", "1.256", "1.2.65536",
     "5.0SP2", "5.0.SP2", "1.2-rc1", "1.2+meta", "FIX.5.0", "-1", "1.2.-1", " 1"],
)
def test_invalid_or_qualified_versions_fail_at_the_native_parser(text):
    with pytest.raises(ValueError, match="version"):
        Version.from_str(text)
    with pytest.raises(ValueError, match="version"):
        DataType("version").scalar(text)


@pytest.mark.parametrize("parts", [(-1,), (256,), (1, -1), (1, 256), (1, 2, -1), (1, 2, 65536)])
def test_constructor_enforces_each_numeric_width(parts):
    with pytest.raises(OverflowError):
        Version(*parts)


@pytest.mark.parametrize("parts", [(1.5,), (1, 2.5), (1, 2, 3.5), ("1",)])
def test_constructor_requires_integer_parts(parts):
    with pytest.raises(TypeError):
        Version(*parts)


def test_order_hash_copy_pickle_and_readonly_parts():
    value = Version(5, 0, 300)
    assert Version(5, 0, 2) < Version(5, 0, 10) < value < Version(5, 1)
    assert Version(5) == Version.from_str("5.0.0")
    assert value != "5.0.300"
    assert hash(Version(5)) == hash(Version.from_str("5.0.0"))
    assert {Version(5), Version.from_str("5.0")} == {Version(5)}
    assert value.stable_hash() == Scalar.from_py(value).stable_hash()
    for clone in (copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))):
        assert type(clone) is Version
        assert clone == value
        assert hash(clone) == hash(value)
        assert (clone.major, clone.minor, clone.patch) == (5, 0, 300)
    for name in ("major", "minor", "patch", "tag"):
        with pytest.raises(AttributeError):
            setattr(value, name, 1)
    with pytest.raises(TypeError):
        value < "5.0.300"


def test_scalar_fields_and_annotations_preserve_native_version_identity():
    value = Version(5, 0, 300)
    field = types.version("release", nullable=False)
    native = Scalar.from_py(value)
    assert native.id == "version"
    assert type(native.as_py()) is Version
    assert native.as_py() == value
    assert field.scalar(value) == native == field.scalar("5.0.300")
    assert field.default_scalar().as_py() == Version(0)
    assert field.default_pyhint() is Version
    assert DataType.from_pyhint(Version) == DataType("version")
    assert Field.from_pyhint("release", Version).dtype == DataType("version")
    assert json.dumps(value) == b'"5.0.300"'
    restored = json.loads('"5.0.300"', field=field, cls=Scalar)
    assert restored == native
    assert restored.as_py() == value
    assert pickle.loads(pickle.dumps(native)) == native


def test_arrow_keeps_string_storage_and_declared_field_restores_version():
    field = types.version("release", nullable=False)
    arrow_field = field.into_arrow()
    assert arrow_field.type == pa.string()
    assert Field.from_arrow(arrow_field) == field
    array = field.cast_arrow_array(pa.array(["005.00.00300", "5.0", "255.255.65535"]))
    assert array.to_pylist() == ["5.0.300", "5", "255.255.65535"]
    scalar = Scalar.from_py(Version(5, 0, 300))
    assert scalar.into_arrow_scalar(field).as_py() == "5.0.300"
    batch = pa.record_batch([array], schema=pa.schema([arrow_field]))
    native = Scalar.from_arrow_batch(batch)
    assert [row[0].as_py() for row in native] == [Version(5, 0, 300), Version(5), Version(255, 255, 65535)]


def test_retired_msgtype_datatype_is_absent_and_url_keeps_its_new_index():
    assert not hasattr(types, "msgtype")
    assert not hasattr(types, "MsgTypeField")
    assert "msgtype" not in enums.DATA_TYPE_IDS
    assert len(enums.DATA_TYPE_IDS) == 60
    assert enums.DATA_TYPE_IDS.index("url") == 59
    with pytest.raises(ValueError):
        DataType("msgtype")
    with pytest.raises(ValueError):
        Field("code", "msgtype")
    assert DataType("utf8").scalar("UConfigurationPlugin").as_py() == "UConfigurationPlugin"


def test_version_annotations_and_generated_dataclasses_keep_the_native_type():
    @scalar(frozen=True)
    class Release:
        version: Version

    assert field(Version).dtype == DataType("version")
    assert field(Version(5, 0, 300), name="release").dtype == DataType("version")
    root = field(Release)
    assert root.dtype["version"].dtype == DataType("version")
    value = Release(Version(5, 0, 300))
    decoded = json.loads('{"version":"5.0.300"}', cls=Release)
    assert decoded == value
    assert type(decoded.version) is Version
    assert json.dumps(decoded) == b'{"version":"5.0.300"}'
    inferred = Scalar.from_py([value]).into_struct_field()
    assert inferred.dtype["version"].dtype == DataType("version")
    arrow_root = root.into_arrow_schema()
    assert Field.from_arrow_schema(arrow_root).dtype["version"].dtype == DataType("version")
    generated = root.into_dataclass(name="GeneratedVersion")
    assert typing.get_type_hints(generated)["version"] is Version
    restored = json.loads('{"version":"5.0.300"}', cls=generated)
    assert type(restored.version) is Version
    assert restored.version == value.version
    assert generated.field() is root
