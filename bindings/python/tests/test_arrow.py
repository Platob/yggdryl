"""Tests for the Arrow C Data Interface bridge of ``yggdryl.typed``.

The typed carriers implement the standard Arrow **PyCapsule protocol**, so a real pyarrow build
imports them zero-copy with no pyarrow dependency in the Rust extension:

- ``StructSerie`` exposes ``__arrow_c_schema__`` / ``__arrow_c_array__`` (``pa.record_batch`` /
  ``pa.table`` / ``pa.schema``) and a ``StructSerie.from_arrow`` importer;
- the leaf ``Serie`` / ``ByteSerie`` expose ``__arrow_c_array__`` (``pa.array``).

The suite ``importorskip``s pyarrow so it passes cleanly where a wheel is unavailable.
"""

import pytest

pa = pytest.importorskip("pyarrow")

from yggdryl.datatype_id import DataTypeId
from yggdryl.typed import ByteSerie, Serie, StructSerie


def _person_table():
    """A two-column ``StructSerie``: an ``i64`` ``id`` and a ``utf8`` ``name``."""
    ids = Serie.from_values([1, 2, 3], DataTypeId.I64)
    names = ByteSerie.from_values(["ada", "bo", "cy"], DataTypeId.Utf8)
    return StructSerie.from_columns([ids, names], names=["id", "name"])


# -------------------------------------------------------------------------------------
# Export: StructSerie -> pyarrow (__arrow_c_array__ / __arrow_c_schema__)
# -------------------------------------------------------------------------------------


def test_record_batch_export():
    batch = pa.record_batch(_person_table())
    assert batch.num_columns == 2
    assert batch.num_rows == 3
    assert batch.schema.names == ["id", "name"]
    assert batch.column("id").to_pylist() == [1, 2, 3]
    assert batch.column("name").to_pylist() == ["ada", "bo", "cy"]
    assert batch.schema.field("id").type == pa.int64()
    assert batch.schema.field("name").type == pa.utf8()


def test_table_export():
    table = pa.table(_person_table())
    assert table.num_columns == 2
    assert table.column_names == ["id", "name"]
    assert table.column("name").to_pylist() == ["ada", "bo", "cy"]


def test_schema_export():
    schema = pa.schema(_person_table())
    assert schema.names == ["id", "name"]
    assert schema.field("id").type == pa.int64()
    assert schema.field("name").type == pa.utf8()


# -------------------------------------------------------------------------------------
# Import: pyarrow -> StructSerie (from_arrow)
# -------------------------------------------------------------------------------------


def test_round_trip_from_record_batch():
    batch = pa.record_batch(_person_table())
    back = StructSerie.from_arrow(batch)
    assert back.num_columns() == 2
    assert back.column_names() == ["id", "name"]
    assert back.column_by_name("id").to_list() == [1, 2, 3]
    assert back.column_by_name("name").to_list() == ["ada", "bo", "cy"]
    assert back.row(1) == [2, "bo"]


def test_from_struct_array():
    struct_array = pa.StructArray.from_arrays(
        [pa.array([1, 2, 3], pa.int64()), pa.array(["ada", "bo", "cy"], pa.utf8())],
        names=["id", "name"],
    )
    back = StructSerie.from_arrow(struct_array)
    assert back.column_names() == ["id", "name"]
    assert back.column_by_name("name").to_list() == ["ada", "bo", "cy"]


def test_from_arrow_rejects_non_arrow():
    with pytest.raises(TypeError):
        StructSerie.from_arrow(object())


# -------------------------------------------------------------------------------------
# Leaf columns: Serie / ByteSerie -> pyarrow (pa.array)
# -------------------------------------------------------------------------------------


def test_leaf_serie_array_export():
    arr = pa.array(Serie.from_values([10, 20, 30], DataTypeId.I64))
    assert arr.type == pa.int64()
    assert arr.to_pylist() == [10, 20, 30]


def test_leaf_byte_serie_array_export():
    arr = pa.array(ByteSerie.from_values(["x", "yy", "zzz"], DataTypeId.Utf8))
    assert arr.type == pa.utf8()
    assert arr.to_pylist() == ["x", "yy", "zzz"]


def test_leaf_serie_with_nulls_round_trips():
    arr = pa.array(Serie.from_options([1, None, 3], DataTypeId.I64))
    assert arr.to_pylist() == [1, None, 3]
    assert arr.null_count == 1
