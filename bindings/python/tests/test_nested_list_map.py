"""Tests for the ``yggdryl.types`` nested LIST and MAP families — ``ListField`` / ``ListSerie`` and
``MapField`` / ``MapSerie`` — plus the self-describing ``StructSerie.from_series`` builder, over
``yggdryl_core::io::nested``.

``ListSerie`` is ``i32`` offsets over one flattened child column; ``MapSerie`` is the optimized alias
of ``List<Struct<{key, value}>>``. Both serialize to the same canonical bytes in every language and
bridge zero-copy to pyarrow's ``ListArray`` / ``MapArray`` via the Arrow C Data Interface.
"""

import copy
import pickle

import pytest

from yggdryl.types import (
    DataType,
    Field,
    I32Serie,
    I64Serie,
    ListField,
    ListSerie,
    MapField,
    MapSerie,
    StructField,
    StructSerie,
    Utf8Serie,
)


def _list_i32():
    # 3 rows over the flat child [10, 20, 30, 40]: [10, 20, 30], [], [40].
    return ListSerie(I32Serie([10, 20, 30, 40]), [0, 3, 3, 4])


def _map_utf8_i64():
    # 2 rows over 3 entries: {"a"->1, "b"->2}, {"c"->3}.
    keys = Utf8Serie(["a", "b", "c"])
    values = I64Serie([1, 2, 3])
    return MapSerie(keys, values, [0, 2, 3])


# ---- ListField -----------------------------------------------------------------------------


def test_list_field_shape():
    schema = ListField("scores", Field("item", DataType.i32(), True), True)
    assert schema.name == "scores"
    assert schema.type_name == "list"
    assert schema.nullable
    assert schema.data_type.name == "list"
    assert schema.item.name == "item"


def test_list_field_nests_a_struct_item():
    item = StructField("point", [Field("x", DataType.f64(), False)], False)
    outer = ListField("shapes", item, True)
    recovered = outer.item
    assert isinstance(recovered, StructField)
    assert recovered.name == "point"


def test_list_field_builders_are_immutable():
    base = ListField("l", Field("item", DataType.i32(), True), True)
    renamed = base.with_name("m").with_nullable(False)
    assert base.name == "l" and base.nullable
    assert renamed.name == "m" and not renamed.nullable
    reitem = base.with_item(Field("item", DataType.utf8(), True))
    assert base.item.data_type == DataType.i32()
    assert reitem.item.data_type == DataType.utf8()


def test_list_field_value_semantics():
    a = ListField("l", Field("item", DataType.i32(), True), True)
    b = ListField("l", Field("item", DataType.i32(), True), True)
    assert a == b
    assert hash(a) == hash(b)
    assert {a, b} == {a}
    assert ListField.deserialize_bytes(a.serialize_bytes()) == a
    assert pickle.loads(pickle.dumps(a)) == a
    assert copy.deepcopy(a) == a


# ---- ListSerie -----------------------------------------------------------------------------


def test_list_serie_build_and_navigate():
    col = _list_i32()
    assert len(col) == 3
    assert col.null_count == 0
    assert not col.has_nulls
    assert col.offsets == [0, 3, 3, 4]
    assert col.data_type.name == "list"
    # The flattened child column, rewrapped to its concrete Serie.
    child = col.values
    assert isinstance(child, I32Serie)
    assert child.get(3) == 40
    # Row access: each row is its element sub-Serie.
    row0 = col.get(0)
    assert isinstance(row0, I32Serie)
    assert [row0.get(i) for i in range(len(row0))] == [10, 20, 30]
    assert len(col.get(1)) == 0  # the empty row
    assert [col.get(2).get(i) for i in range(len(col.get(2)))] == [40]


def test_list_serie_null_rows():
    col = ListSerie(I32Serie([1, 2, 3]), [0, 2, 2, 3], present=[True, False, True])
    assert col.null_count == 1
    assert col.has_nulls
    assert col.get(1) is None  # a null list row
    assert col.get(0) is not None


def test_list_serie_row_out_of_range_raises():
    with pytest.raises(IndexError):
        _list_i32().get(5)


def test_list_serie_item_field_and_to_field():
    col = _list_i32()
    assert col.item_field.name == "item"
    assert col.item_field.data_type == DataType.i32()
    schema = col.to_field("scores")
    assert isinstance(schema, ListField)
    assert schema.name == "scores"
    assert not schema.nullable  # no null rows


def test_list_serie_serialize_round_trip():
    col = _list_i32()
    assert ListSerie.deserialize_bytes(col.serialize_bytes()) == col


def test_list_serie_value_semantics():
    a = _list_i32()
    b = _list_i32()
    assert a == b
    assert a.copy() == a
    assert copy.deepcopy(a) == a
    assert pickle.loads(pickle.dumps(a)) == a


def test_list_serie_repr_and_bool():
    col = _list_i32()
    assert "ListSerie(len=3" in repr(col)
    assert bool(col) is True
    assert bool(ListSerie(I32Serie([]), [0])) is False


def test_list_serie_nested_in_list():
    inner = ListSerie(I32Serie([1, 2, 3, 4]), [0, 2, 4])  # 2 rows
    outer = ListSerie(inner, [0, 1, 2])  # a List<List<i32>>: 2 rows, one inner list each
    assert len(outer) == 2
    recovered = outer.get(0)
    assert isinstance(recovered, ListSerie)
    assert ListSerie.deserialize_bytes(outer.serialize_bytes()) == outer


# ---- MapField ------------------------------------------------------------------------------


def test_map_field_shape():
    schema = MapField(
        "counts",
        Field("key", DataType.utf8(), False),
        Field("value", DataType.i64(), True),
        True,
        False,
    )
    assert schema.name == "counts"
    assert schema.type_name == "map"
    assert schema.nullable
    assert not schema.keys_sorted
    assert schema.data_type.name == "map"
    assert schema.key.name == "key"
    assert schema.value.name == "value"


def test_map_field_value_semantics():
    a = MapField("m", Field("key", DataType.utf8(), False), Field("value", DataType.i64(), True))
    b = MapField("m", Field("key", DataType.utf8(), False), Field("value", DataType.i64(), True))
    assert a == b
    assert hash(a) == hash(b)
    assert {a, b} == {a}
    assert MapField.deserialize_bytes(a.serialize_bytes()) == a
    assert pickle.loads(pickle.dumps(a)) == a
    assert copy.deepcopy(a) == a


# ---- MapSerie ------------------------------------------------------------------------------


def test_map_serie_build_and_navigate():
    col = _map_utf8_i64()
    assert len(col) == 2
    assert col.null_count == 0
    assert not col.keys_sorted
    assert col.offsets == [0, 2, 3]
    assert col.data_type.name == "map"
    keys = col.keys
    values = col.values
    assert isinstance(keys, Utf8Serie)
    assert isinstance(values, I64Serie)
    assert keys.get(0) == "a"
    assert values.get(0) == "1"  # i64 crosses as a decimal string


def test_map_serie_get_value():
    col = _map_utf8_i64()
    # The probe is a single-element Serie of the key type; the result is a one-element value Serie.
    hit = col.get_value(0, Utf8Serie(["b"]))
    assert isinstance(hit, I64Serie)
    assert hit.get(0) == "2"
    # A key absent from the row -> None.
    assert col.get_value(0, Utf8Serie(["c"])) is None
    # Present in the other row.
    assert col.get_value(1, Utf8Serie(["c"])).get(0) == "3"


def test_map_serie_row_and_fields():
    col = _map_utf8_i64()
    row0 = col.get(0)
    assert isinstance(row0, StructSerie)  # the row's [key, value] entries
    assert len(row0) == 2
    assert col.key_field.name == "key"
    assert col.value_field.name == "value"
    schema = col.to_field("counts")
    assert isinstance(schema, MapField)
    assert schema.name == "counts"


def test_map_serie_null_rows_and_out_of_range():
    keys = Utf8Serie(["a", "b"])
    values = I64Serie([1, 2])
    col = MapSerie(keys, values, [0, 1, 2], present=[True, False])
    assert col.null_count == 1
    assert col.get(1) is None
    assert col.get_value(1, Utf8Serie(["b"])) is None  # a null map row
    with pytest.raises(IndexError):
        col.get(9)


def test_map_serie_rejects_null_key():
    with pytest.raises(ValueError):
        MapSerie(Utf8Serie(["a", None]), I64Serie([1, 2]), [0, 2])


def test_map_serie_serialize_round_trip():
    col = _map_utf8_i64()
    assert MapSerie.deserialize_bytes(col.serialize_bytes()) == col


def test_map_serie_value_semantics():
    a = _map_utf8_i64()
    b = _map_utf8_i64()
    assert a == b
    assert a.copy() == a
    assert copy.deepcopy(a) == a
    assert pickle.loads(pickle.dumps(a)) == a


def test_map_serie_repr():
    assert "MapSerie(len=2" in repr(_map_utf8_i64())


# ---- StructSerie.from_series (the self-describing builder) ----------------------------------


def test_struct_from_series_matches_constructor():
    ids = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])
    built = StructSerie.from_series([("id", ids), ("name", names)])
    assert isinstance(built, StructSerie)
    assert built.num_columns == 2
    assert built.field(1).name == "name"
    # Functionally identical to the constructor — byte-for-byte the same frame.
    assert built == StructSerie([("id", ids), ("name", names)])
    assert built.serialize_bytes() == StructSerie([("id", ids), ("name", names)]).serialize_bytes()


def test_struct_from_series_mismatched_lengths_raise():
    with pytest.raises(ValueError):
        StructSerie.from_series([("a", I64Serie([1, 2])), ("b", I32Serie([1]))])


# ---- Arrow C Data Interface (PyCapsule) bridge to pyarrow -----------------------------------


def test_pyarrow_list_round_trip():
    pa = pytest.importorskip("pyarrow")
    col = _list_i32()

    arr = pa.array(col)  # -> a ListArray, imported through __arrow_c_array__
    # A list<int32> — the item field is non-nullable because the child column holds no nulls.
    assert pa.types.is_list(arr.type)
    assert arr.type.value_type == pa.int32()
    assert arr.to_pylist() == [[10, 20, 30], [], [40]]

    # Import it back, zero-copy — the inverse direction.
    assert ListSerie.from_arrow(arr) == col


def test_pyarrow_map_round_trip():
    pa = pytest.importorskip("pyarrow")
    col = _map_utf8_i64()

    arr = pa.array(col)  # -> a MapArray, imported through __arrow_c_array__
    assert isinstance(arr, pa.MapArray)
    assert arr.to_pylist() == [[("a", 1), ("b", 2)], [("c", 3)]]

    # Import it back, zero-copy — the inverse direction.
    assert MapSerie.from_arrow(arr) == col


def test_arrow_c_schema_capsule_is_exposed():
    # The schema capsule is produced independently of pyarrow being installed.
    for col in (_list_i32(), _map_utf8_i64()):
        cap = col.__arrow_c_schema__()
        assert type(cap).__name__ == "PyCapsule"
