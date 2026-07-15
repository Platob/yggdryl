"""Tests for the ``yggdryl.types`` schema layer: ``DataType`` and ``Field`` (whose metadata is
the centralized ``yggdryl.io.Headers`` map)."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.io import Headers
from yggdryl.types import DataType, Field


def test_module_surface():
    for cls in (DataType, Field):
        assert cls.__module__ == "yggdryl.types"
        assert hasattr(yggdryl.types, cls.__name__)


# ---------------------------------------------------------------------------------------
# DataType
# ---------------------------------------------------------------------------------------


def test_named_factories_name_and_width():
    assert (DataType.u8().name, DataType.u8().byte_width) == ("u8", 1)
    assert (DataType.i256().name, DataType.i256().byte_width) == ("i256", 32)
    assert (DataType.f16().name, DataType.f16().byte_width) == ("f16", 2)
    assert (DataType.utf8().name, DataType.utf8().byte_width) == ("utf8", 4)
    assert (DataType.large_binary().name, DataType.large_binary().byte_width) == ("large_binary", 8)


def test_by_name_covers_all_and_rejects_unknown():
    for name in ("u96", "i128", "f64", "binary", "large_utf8", "null"):
        assert DataType.by_name(name).name == name
    with pytest.raises(ValueError, match="unknown data type"):
        DataType.by_name("nonesuch")


def test_fixed_size_takes_runtime_width():
    fb = DataType.fixed_binary(16)
    assert fb.name == "fixed_binary" and fb.byte_width == 16
    assert fb.is_binary() and fb.is_fixed_width() and not fb.is_variable_length()

    fu = DataType.fixed_utf8(4)
    assert fu.name == "fixed_utf8" and fu.byte_width == 4
    assert fu.is_utf8() and fu.is_fixed_width()
    # Same Arrow width, different logical type -> not equal.
    assert fb != DataType.fixed_binary(8)
    assert fu != fb


def test_category_drill_down():
    # (integer, unsigned, signed_int, signed, floating, numeric, utf8, binary, fixed, variable)
    def row(dt):
        return (
            dt.is_integer(), dt.is_unsigned_integer(), dt.is_signed_integer(), dt.is_signed(),
            dt.is_floating(), dt.is_numeric(), dt.is_utf8(), dt.is_binary(),
            dt.is_fixed_width(), dt.is_variable_length(),
        )

    assert row(DataType.u32()) == (True, True, False, False, False, True, False, False, True, False)
    assert row(DataType.i32()) == (True, False, True, True, False, True, False, False, True, False)
    assert row(DataType.f64()) == (False, False, False, True, True, True, False, False, True, False)
    assert row(DataType.utf8()) == (False, False, False, False, False, False, True, False, False, True)
    assert row(DataType.binary()) == (False, False, False, False, False, False, False, True, False, True)
    # A fixed-size byte type is BOTH fixed-width AND binary/utf8.
    assert DataType.fixed_utf8(3).is_fixed_width() and DataType.fixed_utf8(3).is_utf8()


def test_category_string():
    assert DataType.u8().category == "unsigned_integer"
    assert DataType.i8().category == "signed_integer"
    assert DataType.f32().category == "float"
    assert DataType.utf8().category == "utf8"
    assert DataType.binary().category == "binary"
    assert DataType.null().category == "null"


def test_data_type_equality_and_hash():
    assert DataType.i64() == DataType.i64()
    assert DataType.i64() != DataType.u64()
    assert hash(DataType.i64()) == hash(DataType.i64())
    # Usable as a dict key.
    seen = {DataType.i64(): "a", DataType.i64(): "b", DataType.utf8(): "c"}
    assert len(seen) == 2
    assert repr(DataType.fixed_binary(16)) == "DataType(fixed_binary[16])"
    assert repr(DataType.i32()) == "DataType(i32)"


# ---------------------------------------------------------------------------------------
# Field
# ---------------------------------------------------------------------------------------


def test_field_construction_and_properties():
    f = Field("id", DataType.i64())  # nullable defaults to True
    assert f.name == "id"
    assert f.type_name == "i64"
    assert f.byte_width == 8
    assert f.nullable is True
    assert f.data_type == DataType.i64()
    assert f.is_integer() and f.is_signed()
    assert len(f.metadata) == 0

    strict = Field("id", DataType.i64(), nullable=False)
    assert strict.nullable is False


def test_field_metadata_from_dict_and_object():
    a = Field("t", DataType.f64(), metadata={"unit": "seconds"})
    b = Field("t", DataType.f64(), metadata=Headers({"unit": "seconds"}))
    assert a.metadata.get("unit") == "seconds"
    assert a == b  # metadata is part of the value

    with pytest.raises(ValueError, match="Headers or a dict"):
        Field("t", DataType.f64(), metadata=123)


def test_field_metadata_builders_are_non_mutating():
    base = Field("t", DataType.utf8())
    tagged = base.with_metadata_entry("charset", "utf8").with_metadata_entry("lang", "en")
    assert tagged.metadata.items() == [("charset", "utf8"), ("lang", "en")]
    assert len(base.metadata) == 0  # base untouched

    replaced = base.with_metadata({"only": "this"})
    assert replaced.metadata.items() == [("only", "this")]

    # The metadata accessor returns a copy — mutating it does not affect the field.
    meta = tagged.metadata
    meta["extra"] = "x"
    assert "extra" not in tagged.metadata


def test_field_equality_hash_and_copy():
    a = Field("x", DataType.utf8(), metadata={"k": "v"})
    b = Field("x", DataType.utf8(), metadata={"k": "v"})
    assert a == b
    assert hash(a) == hash(b)
    assert a != Field("x", DataType.utf8())  # different metadata
    assert a != Field("y", DataType.utf8(), metadata={"k": "v"})

    dup = a.copy()
    assert dup == a
    schema = {a: "col-a"}  # hashable -> usable as a dict/set key
    assert schema[b] == "col-a"


# ---------------------------------------------------------------------------------------
# Dunders: pickle + copy
# ---------------------------------------------------------------------------------------


@pytest.mark.parametrize(
    "obj",
    [
        DataType.i32(),
        DataType.u256(),
        DataType.fixed_binary(16),
        DataType.fixed_utf8(4),
        Field("id", DataType.i64(), False),
        Field("t", DataType.f64(), True, {"unit": "seconds", "scale": "1"}),
    ],
)
def test_pickle_round_trip(obj):
    assert pickle.loads(pickle.dumps(obj)) == obj


def test_copy_and_deepcopy():
    field = Field("t", DataType.utf8(), metadata={"k": "v"})
    assert copy.copy(field) == field
    assert copy.deepcopy(field) == field
    # The copy is an independent value.
    dup = copy.deepcopy(field).with_metadata_entry("k2", "v2")
    assert "k2" not in field.metadata
