"""The numeric Version boundary and its Scalar/Arrow representations."""

from __future__ import annotations

import copy
import pickle
import typing

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar, Serie, Version, enums, field, json, scalar


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
    assert yggdryl.Version is Version
    assert (parsed.major, parsed.minor, parsed.patch) == parts
    assert parsed == Version(*parts)
    assert str(parsed) == canonical
    assert eval(repr(parsed), {"Version": Version}) == parsed
    assert not hasattr(parsed, "tag")
    assert not hasattr(parsed, "qualifier")


@pytest.mark.parametrize(
    "text",
    [".1", " 1", "-1", "+1", "v1", "256", "999", "1.256", "1.999", "FIX.5.0"],
)
def test_only_the_numeric_components_fail_at_the_native_parser(text):
    with pytest.raises(ValueError, match="version"):
        Version.from_str(text)
    with pytest.raises(ValueError, match="version"):
        DataType("version").scalar(text)


def test_the_empty_text_is_no_version_but_reads_as_null_at_the_datatype():
    # A `Version` is a value, so its own parser refuses the empty text; the
    # datatype door reads an empty text cell entering a non-text column as
    # absence, and a required field is what refuses that.
    with pytest.raises(ValueError, match="version"):
        Version.from_str("")
    assert DataType("version").scalar("").is_null()
    assert Field("release", "version").scalar("").is_null()
    with pytest.raises(ValueError, match="non-nullable field received null"):
        Field("release", "version", nullable=False).scalar("")


@pytest.mark.parametrize(
    ("text", "parts"),
    [
        ("5.0sp250", (5, 0, 250)),
        ("5.0SP250", (5, 0, 250)),
        ("5.0Sp250", (5, 0, 250)),
        ("5.0sP250", (5, 0, 250)),
        ("005.000sp00250", (5, 0, 250)),
        ("5.0SP0", (5, 0, 0)),
        ("5.0sp65535", (5, 0, 65535)),
        ("255.255sp65535", (255, 255, 65535)),
    ],
)
def test_compact_fix_service_packs_are_numeric_patches(text, parts):
    assert Version.from_str(text) == Version(*parts)
    assert str(Version.from_str(text)) == str(Version(*parts))


@pytest.mark.parametrize(
    "text",
    ["1.", "1..2", "1.2.3.4", "1.2.65536", "5.0.SP2", "1.2-rc1", "1.2+meta",
     "1.2.-1", "1.0SP2_EP250", "1.0sp250 "],
)
def test_a_patch_tail_stating_no_number_folds_instead_of_failing(text):
    parsed = Version.from_str(text)
    assert parsed.major == 1 or parsed.major == 5
    # The same tail always reads as the same version.
    assert parsed == Version.from_str(text)
    assert DataType("version").scalar(text).as_py() == parsed


def test_unlike_patch_tails_read_as_unlike_versions():
    assert Version.from_str("1.0-rc1") != Version.from_str("1.0-rc2")
    assert Version.from_str("1.0-rc1") != Version.from_str("1.0")
    assert Version.from_str("1.0-rc1").patch != 0


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
    assert value.stable_hash() == Scalar.from_(value).stable_hash()
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
    field = yggdryl.version("release", nullable=False)
    native = Scalar.from_(value)
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
    field = yggdryl.version("release", nullable=False)
    arrow_field = field.into_arrow()
    assert arrow_field.type == pa.string()
    assert Field.from_arrow(arrow_field) == field
    array = Serie.from_arrow_array(
        pa.array(["005.00.00300", "5.0", "255.255.65535"]), field
    ).into_arrow_array()
    assert array.to_pylist() == ["5.0.300", "5", "255.255.65535"]
    scalar = Scalar.from_(Version(5, 0, 300))
    assert scalar.into_arrow_scalar(field).as_py() == "5.0.300"
    batch = pa.record_batch([array], schema=pa.schema([arrow_field]))
    native = Serie.from_(batch)
    assert native.child("release").as_py() == [Version(5, 0, 300), Version(5), Version(255, 255, 65535)]


def test_the_datatype_identifiers_are_laid_out_by_family():
    # Eighty-five, laid out by family: every identifier sits in its
    # family's range and the list states them in that order, so `url` and
    # `urn` follow `version` in the text family, `sized_utf8` follows
    # `fixed_utf8`, and the geospatial pair closes the list. An identifier is
    # a wire contract laid out by family, so a leaf added later lands beside
    # its family and nothing ever moves.
    assert len(enums.DATA_TYPE_IDS) == 87
    assert "figi" in enums.DATA_TYPE_IDS
    ids = list(enums.DATA_TYPE_IDS)
    assert ids.index("url") == ids.index("version") + 1
    assert ids.index("urn") == ids.index("url") + 1
    assert ids.index("sized_utf8") == ids.index("fixed_utf8") + 1
    assert ids[-2:] == ["geometry", "geography"]
    assert ids[:2] == ["null", "boolean"]


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
    inferred = Scalar.from_([value]).into_struct_field()
    assert inferred.dtype["version"].dtype == DataType("version")
    arrow_root = root.into_arrow_schema()
    assert Field.from_arrow_schema(arrow_root).dtype["version"].dtype == DataType("version")
    generated = root.into_dataclass(name="GeneratedVersion")
    assert typing.get_type_hints(generated)["version"] is Version
    restored = json.loads('{"version":"5.0.300"}', cls=generated)
    assert type(restored.version) is Version
    assert restored.version == value.version
    assert generated.into_field() is root
