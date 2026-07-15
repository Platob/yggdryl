"""Tests for the ``yggdryl.types`` nested layer: ``StructField`` (the centralized struct schema)
and ``StructSerie`` (a nullable struct column of heterogeneous child columns), over
``yggdryl_core::io::nested``.

A ``StructField`` is a value type (hashable, pickles). A ``StructSerie`` holds the crate's existing
``Serie`` columns as children and serializes to the same canonical bytes in every language.
"""

import copy
import pickle

import pytest

from yggdryl.types import (
    DataType,
    Field,
    I64Serie,
    StructField,
    StructSerie,
    U8Serie,
    Utf8Serie,
)


def _table():
    ids = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])
    return StructSerie([("id", ids), ("name", names)])


# ---- StructField ---------------------------------------------------------------------------


def test_struct_field_shape():
    schema = StructField(
        "person",
        [
            Field("id", DataType.i64(), False),
            Field("name", DataType.utf8(), True),
        ],
        True,
    )
    assert schema.name == "person"
    assert schema.type_name == "struct"
    assert schema.nullable
    assert schema.num_fields == 2
    assert len(schema) == 2
    assert schema.index_of("name") == 1
    assert schema.field(1).name == "name"
    assert schema.field_named("id").name == "id"
    assert schema.field_named("missing") is None
    assert [f.name for f in schema.fields()] == ["id", "name"]


def test_struct_field_nests():
    inner = StructField("point", [Field("x", DataType.f64(), False)], False)
    outer = StructField("shape", [inner], True)
    assert outer.num_fields == 1
    recovered = outer.field(0)
    assert isinstance(recovered, StructField)
    assert recovered.name == "point"


def test_struct_field_builders_are_immutable():
    base = StructField("s", [Field("a", DataType.i32(), True)], True)
    renamed = base.with_name("t").with_nullable(False)
    assert base.name == "s" and base.nullable
    assert renamed.name == "t" and not renamed.nullable
    grown = base.with_field(Field("b", DataType.utf8(), True))
    assert base.num_fields == 1 and grown.num_fields == 2


def test_struct_field_value_semantics():
    a = StructField("s", [Field("a", DataType.i32(), True)], True)
    b = StructField("s", [Field("a", DataType.i32(), True)], True)
    assert a == b
    assert hash(a) == hash(b)
    assert {a, b} == {a}  # hashable, so it works as a set/dict key
    assert StructField.deserialize_bytes(a.serialize_bytes()) == a
    assert pickle.loads(pickle.dumps(a)) == a
    assert copy.deepcopy(a) == a


# ---- StructSerie ---------------------------------------------------------------------------


def test_struct_serie_build_and_navigate():
    table = _table()
    assert len(table) == 3
    assert table.num_columns == 2
    assert table.field(1).name == "name"
    ids = table.column(0)
    assert isinstance(ids, I64Serie)
    assert ids.get(0) == "1"  # 64-bit values cross as decimal strings (JS/Python parity)
    names = table.column_named("name")
    assert isinstance(names, Utf8Serie)
    assert names.get(0) == "ann"
    assert names.get(1) is None
    assert table.column_named("missing") is None


def test_struct_serie_columns_list():
    table = _table()
    cols = table.columns()
    assert [type(c).__name__ for c in cols] == ["I64Serie", "Utf8Serie"]


def test_struct_serie_mismatched_lengths_raise():
    with pytest.raises(ValueError):
        StructSerie([("a", I64Serie([1, 2])), ("b", U8Serie([1]))])


def test_struct_serie_not_a_column_raises():
    with pytest.raises(ValueError):
        StructSerie([("a", 123)])


def test_struct_serie_serialize_round_trip():
    table = _table()
    assert StructSerie.deserialize_bytes(table.serialize_bytes()) == table


def test_struct_serie_nested():
    inner = StructSerie([("x", I64Serie([1, 2])), ("y", U8Serie([3, 4]))])
    outer = StructSerie([("point", inner), ("tag", Utf8Serie(["a", "b"]))])
    assert outer.num_columns == 2
    recovered = outer.column(0)
    assert isinstance(recovered, StructSerie)
    assert recovered.column_named("x").get(1) == "2"  # I64Serie: decimal-string values
    assert StructSerie.deserialize_bytes(outer.serialize_bytes()) == outer


def test_struct_serie_value_semantics():
    a = _table()
    b = _table()
    assert a == b
    assert a.copy() == a
    assert copy.deepcopy(a) == a
    assert pickle.loads(pickle.dumps(a)) == a


def test_struct_serie_repr_and_bool():
    table = _table()
    assert "StructSerie(len=3" in repr(table)
    assert bool(table) is True
    assert bool(StructSerie([])) is False


def test_to_field_nullability_reflects_struct_rows_not_child_nulls():
    # _table() has a null in the `name` *column*, but no null struct *rows*, so the struct field
    # it names is non-nullable (top-level validity, not child validity, drives it).
    schema = _table().to_field("person")
    assert isinstance(schema, StructField)
    assert schema.name == "person"
    assert not schema.nullable


# ---- Arrow C Data Interface (PyCapsule) bridge to pyarrow -----------------------------------


def test_pyarrow_c_data_interface_round_trip():
    pa = pytest.importorskip("pyarrow")
    from yggdryl.types import I32Serie

    table = StructSerie([("id", I32Serie([1, 2, 3])), ("name", Utf8Serie(["ann", None, "cara"]))])

    # Export zero-copy to pyarrow via the Arrow PyCapsule interface.
    arr = pa.array(table)  # -> a StructArray, imported through __arrow_c_array__
    assert len(arr) == 3
    assert [arr.type.field(i).name for i in range(arr.type.num_fields)] == ["id", "name"]
    assert arr.field(0).to_pylist() == [1, 2, 3]
    assert arr.field(1).to_pylist() == ["ann", None, "cara"]

    # Import it back, zero-copy — the inverse direction.
    assert StructSerie.from_arrow(arr) == table

    # And through a RecordBatch (which also exposes the C Data Interface).
    batch = pa.RecordBatch.from_struct_array(arr)
    assert batch.num_rows == 3
    assert StructSerie.from_arrow(batch) == table


def test_arrow_c_schema_capsule_is_exposed():
    # The schema capsule is produced independently of pyarrow being installed.
    cap = _table().__arrow_c_schema__()
    assert type(cap).__name__ == "PyCapsule"
