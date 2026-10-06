"""The native Version boundary and its Scalar/Arrow representations."""

from __future__ import annotations

import copy
import pickle
import sys
import typing

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar, Serie, Version, enums, field, json, scalar


@pytest.mark.parametrize(
    ("text", "parts", "canonical"),
    [
        ("0", (0, 0, None), "0"),
        ("5.0.0", (5, 0, None), "5"),
        ("05.002.0003", (5, 2, "3"), "5.2.3"),
        ("5.0.300", (5, 0, "300"), "5.0.300"),
        ("5.0.007", (5, 0, "7"), "5.0.7"),
        ("256", (256, 0, None), "256"),
        ("1.256", (1, 256, None), "1.256"),
        ("255.255.65535", (255, 255, "65535"), "255.255.65535"),
        ("65535.65535.65535", (65535, 65535, "65535"), "65535.65535.65535"),
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
    [".1", " 1", "-1", "+1", "v1", "65536", "99999", "1.65536", "FIX.5.0"],
)
def test_only_the_major_and_minor_fail_at_the_native_parser(text):
    with pytest.raises(ValueError, match="version"):
        Version.from_str(text)
    with pytest.raises(ValueError, match="version"):
        DataType("version").scalar(text)


@pytest.mark.parametrize(
    ("text", "component"), [("65536.0", "major"), ("1.65536", "minor")]
)
def test_a_component_past_sixteen_bits_names_its_range(text, component):
    with pytest.raises(ValueError, match=rf"{component} version in 0\.\.=65535"):
        Version.from_str(text)


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
        ("5.0sp250", (5, 0, "250")),
        ("5.0SP250", (5, 0, "250")),
        ("5.0Sp250", (5, 0, "250")),
        ("5.0sP250", (5, 0, "250")),
        ("5.0SP2", (5, 0, "2")),
        ("005.000sp00250", (5, 0, "250")),
        ("5.0SP0", (5, 0, None)),
        ("5.0sp65535", (5, 0, "65535")),
        ("5.0sp65536", (5, 0, "65536")),
        ("255.255sp65535", (255, 255, "65535")),
    ],
)
def test_compact_fix_service_packs_are_numeric_patches(text, parts):
    assert Version.from_str(text) == Version(*parts)
    assert str(Version.from_str(text)) == str(Version(*parts))


@pytest.mark.parametrize(
    ("text", "parts", "canonical"),
    [
        ("1.", (1, 0, None), "1"),
        ("1..2", (1, 0, ".2"), "1.0..2"),
        ("1.x", (1, 0, "x"), "1.0.x"),
        ("1.2.3.4", (1, 2, "3.4"), "1.2.3.4"),
        ("1.2.65536", (1, 2, "65536"), "1.2.65536"),
        ("5.0.SP2", (5, 0, "SP2"), "5.0.SP2"),
        ("1.2-rc1", (1, 2, "-rc1"), "1.2-rc1"),
        ("1.0rc1", (1, 0, "rc1"), "1.0.rc1"),
        ("1.2+meta", (1, 2, "+meta"), "1.2+meta"),
        ("1.2.-1", (1, 2, "-1"), "1.2-1"),
        ("1.2界", (1, 2, "界"), "1.2界"),
        ("1.0SP2_EP250", (1, 0, "SP2_EP250"), "1.0.SP2_EP250"),
        ("1.0sp250 ", (1, 0, "sp250 "), "1.0.sp250 "),
    ],
)
def test_a_patch_tail_stating_no_number_is_the_patch_as_written(text, parts, canonical):
    parsed = Version.from_str(text)
    assert (parsed.major, parsed.minor, parsed.patch) == parts
    assert str(parsed) == canonical
    assert Version.from_str(canonical) == parsed
    assert parsed == Version(*parts)
    assert DataType("version").scalar(text).as_py() == parsed


def test_unlike_patch_tails_read_as_unlike_versions():
    assert Version.from_str("1.0-rc1") != Version.from_str("1.0-rc2")
    assert Version.from_str("1.0-rc1") != Version.from_str("1.0")
    # A dotted `SP` is text; only the compact service pack states a number.
    assert Version.from_str("5.0.SP2") != Version.from_str("5.0SP2")
    # `rc01` and `rc1` order equal by number, so their bytes break the tie.
    assert Version(1, 0, "rc01") != Version(1, 0, "rc1")
    assert Version(1, 0, "rc01") < Version(1, 0, "rc1")


def test_an_integer_patch_is_its_digits_and_zero_is_no_patch():
    assert Version(5, 0, 300) == Version(5, 0, "300") == Version(5, 0, "0300")
    assert Version(5, 0, 300).patch == "300"
    for none in (0, "0", "000", "", None):
        assert Version(5, 0, none) == Version(5)
        assert Version(5, 0, none).patch is None
    assert Version(1, 2, 2**100).patch == str(2**100)
    assert Version(1, patch="-rc1") == Version.from_str("1.0-rc1")
    # The constructor takes the patch as written: `sp` is the grammar's.
    assert Version(5, 0, "sp2").patch == "sp2"
    assert str(Version(5, 0, "sp2")) == "5.0.sp2"
    assert Version(5, 0, "sp2") != Version.from_str("5.0sp2")


def test_constructor_accepts_every_sixteen_bit_component():
    assert (Version(65535, 65535).major, Version(65535, 65535).minor) == (65535, 65535)


@pytest.mark.parametrize(
    "parts", [(-1,), (65536,), (1, -1), (1, 65536), (1, 2, -1), (1, 2, -(2**70))]
)
def test_constructor_enforces_each_numeric_width(parts):
    with pytest.raises(OverflowError):
        Version(*parts)


def test_a_negative_patch_is_refused_by_name():
    with pytest.raises(OverflowError, match="patch"):
        Version(1, 2, -1)


def test_an_int_patch_past_the_interpreter_digit_limit_is_refused_by_name():
    # CPython 3.11 and the 3.10 security releases bound int-to-text; older
    # interpreters render any length.
    limit = getattr(sys, "get_int_max_str_digits", lambda: 0)()
    if limit == 0:
        pytest.skip("this interpreter renders an int of any length")
    assert Version(1, 2, 10 ** (limit - 1)).patch == "1" + "0" * (limit - 1)
    with pytest.raises(ValueError, match="patch"):
        Version(1, 2, 10**limit)


@pytest.mark.parametrize("parts", [(1.5,), (1, 2.5), ("1",)])
def test_constructor_requires_integer_components(parts):
    with pytest.raises(TypeError):
        Version(*parts)


@pytest.mark.parametrize("patch", [2.5, b"3", [3], object()])
def test_a_patch_is_an_int_a_str_or_none(patch):
    with pytest.raises(TypeError, match="patch"):
        Version(1, 2, patch)


def test_patches_order_naturally_after_no_patch():
    ordered = [
        Version(1, 0),
        Version(1, 0, "-rc1"),
        Version(1, 0, "-rc2"),
        Version(1, 0, "-rc10"),
        Version(1, 1),
        Version(5, 0, 2),
        Version(5, 0, "2.1"),
        Version(5, 0, 10),
        Version(5, 1),
    ]
    assert sorted(reversed(ordered)) == ordered
    assert all(left < right for left, right in zip(ordered, ordered[1:]))


@pytest.mark.parametrize(
    ("value", "text"),
    [
        (Version(5), "Version(5, 0)"),
        (Version(5, 2), "Version(5, 2)"),
        (Version(5, 0, 300), "Version(5, 0, '300')"),
        (Version(1, 0, "-it's"), 'Version(1, 0, "-it\'s")'),
    ],
)
def test_repr_evaluates_back_to_the_value(value, text):
    assert repr(value) == text
    assert eval(repr(value), {"Version": Version}) == value


def test_repr_quotes_any_patch_text():
    value = Version(1, 0, "-a'b\"c\\d")
    restored = eval(repr(value), {"Version": Version})
    assert restored == value
    assert restored.patch == "-a'b\"c\\d"


@pytest.mark.parametrize(
    ("value", "parts"),
    [
        (Version(5, 0, 300), (5, 0, "300")),
        (Version(1, 0, "-rc1"), (1, 0, "-rc1")),
        (Version(5), (5, 0, None)),
    ],
)
def test_order_hash_copy_pickle_and_readonly_parts(value, parts):
    assert value.__reduce__() == (Version, parts)
    assert value.stable_hash() == Scalar.from_(value).stable_hash()
    for clone in (copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))):
        assert type(clone) is Version
        assert clone == value
        assert hash(clone) == hash(value)
        assert (clone.major, clone.minor, clone.patch) == parts
    for name in ("major", "minor", "patch", "tag"):
        with pytest.raises(AttributeError):
            setattr(value, name, 1)
    assert value != str(value)
    with pytest.raises(TypeError):
        value < str(value)


def test_equal_versions_hash_equal_and_unlike_patches_do_not():
    assert Version(5) == Version.from_str("5.0.0")
    assert hash(Version(5)) == hash(Version.from_str("5.0.0"))
    assert {Version(5), Version.from_str("5.0"), Version(5, 0, 0)} == {Version(5)}
    assert Version(1, 0, "-rc1").stable_hash() != Version(1, 0, "-rc2").stable_hash()


def test_scalar_fields_and_annotations_preserve_native_version_identity():
    value = Version(5, 0, 300)
    field = yggdryl.version("release", nullable=False)
    native = Scalar.from_(value)
    assert native.id == "version"
    assert type(native.as_py()) is Version
    assert native.as_py() == value
    assert field.scalar(value) == native == field.scalar("5.0.300")
    assert field.default_scalar().as_py() == Version(0)
    assert field.default_scalar().as_py().patch is None
    assert field.default_pyhint() is Version
    assert DataType.from_pyhint(Version) == DataType("version")
    assert Field.from_pyhint("release", Version).dtype == DataType("version")
    assert json.dumps(value) == b'"5.0.300"'
    assert json.dumps(Version(1, 0, "-rc1")) == b'"1.0-rc1"'
    restored = json.loads('"5.0.300"', field=field, cls=Scalar)
    assert restored == native
    assert restored.as_py() == value
    assert pickle.loads(pickle.dumps(native)) == native
    qualified = Scalar.from_(Version(1, 0, "rc1"))
    assert pickle.loads(pickle.dumps(qualified)).as_py() == Version(1, 0, "rc1")


def test_arrow_keeps_string_storage_and_declared_field_restores_version():
    field = yggdryl.version("release", nullable=False)
    arrow_field = field.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.version"
    assert Field.from_arrow(arrow_field) == field
    array = Serie.from_arrow_array(
        pa.array(["005.00.00300", "5.0", "255.255.65535", "65535.65535.65535", "1.0rc1"]),
        field,
    ).into_arrow_array()
    assert array.storage.to_pylist() == [
        "5.0.300",
        "5",
        "255.255.65535",
        "65535.65535.65535",
        "1.0.rc1",
    ]
    scalar = Scalar.from_(Version(5, 0, 300))
    assert scalar.into_arrow_scalar(field).as_py() == Version(5, 0, 300)
    assert scalar.into_arrow_scalar(field).value.as_py() == "5.0.300"
    batch = pa.record_batch([array], schema=pa.schema([arrow_field]))
    native = Serie.from_(batch)
    assert native.child("release").as_py() == [
        Version(5, 0, 300),
        Version(5),
        Version(255, 255, 65535),
        Version(65535, 65535, 65535),
        Version(1, 0, "rc1"),
    ]


def test_the_datatype_identifiers_are_laid_out_by_family():
    # Ninety-six, laid out by family: every identifier sits in its
    # family's range and the list states them in that order, so `url` and
    # `urn` follow `version` in the text family, `sized_utf8` follows
    # `fixed_utf8`, and the geospatial pair closes the list. An identifier is
    # a wire contract laid out by family, so a leaf added later lands beside
    # its family and nothing ever moves.
    assert len(enums.DATA_TYPE_IDS) == 96
    assert "figi" in enums.DATA_TYPE_IDS
    ids = list(enums.DATA_TYPE_IDS)
    # The code family's newest identifiers follow the last one before them.
    assert ids.index("ric") == ids.index("unit") + 1
    assert ids.index("forex") == ids.index("ric") + 1
    # The five reference-data codes took the text family's unused tail, so
    # they open the code family's range, after `mediatype` and before
    # `country`.
    assert ids[ids.index("mediatype") + 1 : ids.index("country")] == [
        "lei",
        "bic",
        "elf",
        "dti",
        "fisn",
    ]
    assert ids.index("url") == ids.index("version") + 1
    assert ids.index("urn") == ids.index("url") + 1
    assert ids.index("sized_utf8") == ids.index("fixed_utf8") + 1
    # The enum family closes the list after the geospatial pair, and
    # `timeinforce`, an enum since it left the code family, lands last in it.
    assert ids[-7:] == [
        "geometry",
        "geography",
        "state",
        "marketdatakind",
        "side",
        "marketdatatype",
        "timeinforce",
    ]
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
