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
    BinarySerie,
    DataType,
    F64Serie,
    Field,
    I8Serie,
    I16Serie,
    I32Serie,
    I64Serie,
    StructField,
    StructSerie,
    U8Serie,
    Utf8Serie,
    column,
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


# ---- StructSerie deep navigation (get / set a cell or sub-column) ---------------------------


def _flat_struct():
    return StructSerie([("id", I32Serie([1, 2, 3])), ("name", Utf8Serie(["ann", None, "cara"]))])


def test_struct_deep_cell_read_by_coords():
    s = _flat_struct()
    # coords are (child_index, cell_index): field 0 (id) row 2, field 1 (name) row 0.
    assert s[0, 2] == 3
    assert s[1, 0] == "ann"
    assert s[1, 1] is None  # a null cell reads as None


def test_struct_deep_cell_read_by_path():
    s = _flat_struct()
    assert s["id[0]"] == 1
    assert s["name[2]"] == "cara"
    # A name-terminal path returns the live sub-column wrapper, not a cell.
    col = s["name"]
    assert isinstance(col, Utf8Serie)
    assert col.get(2) == "cara"


def test_struct_row_as_dict():
    s = _flat_struct()
    assert s[0] == {"id": 1, "name": "ann"}
    assert s[1] == {"id": 2, "name": None}
    assert s[-1] == {"id": 3, "name": "cara"}  # negative-index aware


def test_struct_deep_cell_write_and_readback():
    s = _flat_struct()
    s[0, 1] = 42  # a python int cast into the i32 leaf
    assert s[0, 1] == 42
    s["name[0]"] = "ANN"
    assert s["name[0]"] == "ANN"


def test_struct_deep_cell_set_null():
    s = _flat_struct()
    s[0, 0] = None
    assert s[0, 0] is None


def test_struct_deep_index_error_out_of_range():
    s = _flat_struct()
    with pytest.raises(IndexError):
        _ = s[0, 99]  # cell out of range -> core IndexOutOfBounds
    with pytest.raises(IndexError):
        s[0, 99] = 5


def test_struct_deep_bad_path_value_error():
    s = _flat_struct()
    with pytest.raises(ValueError):
        _ = s["missing[0]"]  # no such child -> core PathError (guided text)


def test_struct_named_navigation_methods():
    s = _flat_struct()
    assert s.num_children == 2
    assert isinstance(s.child_at(0), I32Serie)
    assert isinstance(s.child_named("name"), Utf8Serie)
    assert s.child_named("missing") is None
    assert "id" in s and "missing" not in s
    assert s.get_cell((0, 0)) == 1
    s.set_cell((0, 0), 7)
    assert s.get_cell("id[0]") == 7
    assert isinstance(s.get_column("name"), Utf8Serie)


def test_struct_slice_returns_sub_column():
    s = _flat_struct()
    head = s[0:2]
    assert isinstance(head, StructSerie)
    assert len(head) == 2


def test_struct_nested_deep_cell():
    inner = StructSerie([("x", I32Serie([1, 2])), ("y", I32Serie([3, 4]))])
    outer = StructSerie([("point", inner), ("tag", Utf8Serie(["a", "b"]))])
    # coords descend the schema: field 0 (point), field 0 (x), cell 1.
    assert outer[0, 0, 1] == 2
    assert outer["point.x[1]"] == 2
    assert outer["point.y[0]"] == 3
    outer[0, 1, 0] = 99  # point.y[0]
    assert outer["point.y[0]"] == 99


# ---- Regression: struct row read (null row + nested-struct child) and deep set contracts ----


def test_struct_null_row_reads_as_none():
    # A struct with a NULL top-level row: s[null_row] must be None, not a dict of stale child bytes.
    pa = pytest.importorskip("pyarrow")
    arr = pa.array(
        [{"id": 1, "name": "ann"}, None, {"id": 3, "name": "cara"}],
        type=pa.struct([("id", pa.int32()), ("name", pa.string())]),
    )
    s = StructSerie.from_arrow(arr)
    assert s.null_count == 1
    assert s[1] is None  # the null struct row is None (not a dict of stale child cells)
    # A list-comprehension over the rows preserves the null as None.
    assert [s[i] for i in range(len(s))] == [
        {"id": 1, "name": "ann"},
        None,
        {"id": 3, "name": "cara"},
    ]


def test_struct_row_nested_struct_child_reads_as_nested_dict():
    # A cell that is itself a struct nests as a dict, instead of raising ValueError.
    s = StructSerie([("p", StructSerie([("x", I32Serie([1]))])), ("t", I32Serie([9]))])
    assert s[0] == {"p": {"x": 1}, "t": 9}


def test_struct_deep_set_writes_into_currently_null_cell():
    # Writing a real value INTO a currently-null cell must succeed — the type comes from the
    # column, not the (null) cell. Mirrors the Node test 'setAt writes into a currently-null cell'.
    s = StructSerie([("id", I32Serie([1, 2, 3])), ("name", Utf8Serie(["ann", None, "cara"]))])
    assert s["name[1]"] is None  # cell 1 is currently null
    s["name[1]"] = "x"
    assert s["name[1]"] == "x"


def test_struct_deep_set_narrow_overflow_and_fraction_rejected():
    # An out-of-range / fractional value into a narrow int leaf raises the guided error from
    # PyNative::from_py, and leaves the column unchanged.
    s = StructSerie([("n", U8Serie([1, 2, 3]))])
    with pytest.raises((OverflowError, ValueError)):
        s[0, 0] = 300  # out of u8 range
    assert s[0, 0] == 1  # unchanged
    with pytest.raises((TypeError, ValueError, OverflowError)):
        s[0, 0] = 2.5  # a fractional value into an int leaf
    assert s[0, 0] == 1  # still unchanged


def test_struct_deep_decimal_leaf_guided_error():
    # A decimal leaf has no native cross-language scalar form, so a deep get/set is a guided error
    # pointing to get_column — decimal LEAF cells stay erroring even after struct cells became dicts.
    from yggdryl.decimal import D64Serie

    s = StructSerie([("v", D64Serie(10, 2, ["1.00", "2.00"]))])
    with pytest.raises(ValueError) as read_exc:
        _ = s[0, 0]  # deep read of a decimal leaf
    assert "get_column" in str(read_exc.value)
    with pytest.raises(ValueError) as write_exc:
        s[0, 0] = "3.00"  # deep write of a decimal leaf
    message = str(write_exc.value)
    assert "not supported" in message and "concrete" in message


# ---- The generic inference factory: yggdryl.types.column(values, dtype=None) ---------------


def test_column_infers_smallest_signed_int():
    col = column([1, 2, 3])
    assert isinstance(col, I8Serie)
    assert col.get(0) == 1


def test_column_infers_wider_int_by_range():
    assert isinstance(column([1, 2, 300]), I16Serie)
    assert isinstance(column([1, 2, 100000]), I32Serie)


def test_column_infers_float_when_any_float():
    col = column([1, 2.5, 3])
    assert isinstance(col, F64Serie)
    assert col.get(1) == 2.5


def test_column_infers_str_and_bytes():
    assert isinstance(column(["a", "b"]), Utf8Serie)
    assert isinstance(column([b"x", b"y"]), BinarySerie)


def test_column_none_is_nullable():
    col = column([1, None, 3])
    assert col.get(1) is None
    assert col.null_count == 1


def test_column_empty_defaults_to_i64():
    assert isinstance(column([]), I64Serie)


def test_column_explicit_dtype_by_name_and_datatype():
    assert isinstance(column([1, 2, 3], dtype="i64"), I64Serie)
    assert isinstance(column([1, 2, 3], dtype=DataType.i32()), I32Serie)


def test_column_ambiguous_mix_raises_guided():
    with pytest.raises(ValueError) as exc:
        column([1, "two"])
    message = str(exc.value)
    assert "int" in message and "str" in message


def test_column_unsupported_element_raises():
    with pytest.raises(ValueError):
        column([{"a": 1}])  # a dict element has no inferrable leaf type
