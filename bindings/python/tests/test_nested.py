"""Tests for the nested typed layer of ``yggdryl.typed``.

Mirrors ``crates/yggdryl-core/src/typed/nested`` on the Python surface: the ``StructSerie``
"table" (heterogeneous, equal-length child columns built ``from_columns``, read back as a copy
via ``column`` / ``column_by_name`` and mutated with ``set_column``, with ``row`` / ``field`` /
``column_names``), the ``ListSerie`` (a variable-length list over a flattened child, ``push`` /
``list``), the ``MapSerie`` (key -> value entries, ``push`` / ``get``), and the value-typed
schema descriptors ``StructField`` / ``ListField`` / ``MapField``.
"""

import pytest

from yggdryl.datatype_id import DataTypeId
from yggdryl.typed import (
    ByteSerie,
    Field,
    ListField,
    ListSerie,
    MapField,
    MapSerie,
    Serie,
    StructField,
    StructSerie,
)


def test_module_surface():
    for cls in (StructSerie, StructField, ListSerie, ListField, MapSerie, MapField):
        assert cls.__module__ == "yggdryl.typed"


# -------------------------------------------------------------------------------------
# StructSerie — the table
# -------------------------------------------------------------------------------------


def test_struct_from_columns():
    ids = Serie.from_values([1, 2, 3], DataTypeId.I64)
    names = ByteSerie.from_values(["a", "b", "c"], DataTypeId.Utf8)
    table = StructSerie.from_columns([ids, names], names=["id", "name"])

    assert table.num_columns() == 2
    assert len(table) == 3
    assert table.column_names() == ["id", "name"]

    name_col = table.column_by_name("name")
    assert isinstance(name_col, ByteSerie)
    assert name_col.to_list() == ["a", "b", "c"]

    id_col = table.column_by_name("id")
    assert isinstance(id_col, Serie)
    assert id_col.to_list() == [1, 2, 3]

    # row(1) marshals the row's child values as a list.
    assert table.row(1) == [2, "b"]

    # column_by_name of a missing column is None; an out-of-range index / row raises.
    assert table.column_by_name("missing") is None
    with pytest.raises(IndexError):
        table.column(5)
    with pytest.raises(IndexError):
        table.row(9)


def test_struct_columns_keep_their_own_names_without_names_arg():
    ids = Serie.from_values([1, 2], DataTypeId.I64).with_name("id")
    table = StructSerie.from_columns([ids])
    assert table.column_names() == ["id"]
    col = table.column(0)
    assert isinstance(col, Serie)
    assert col.to_list() == [1, 2]


def test_struct_set_column():
    table = StructSerie.from_columns(
        [
            Serie.from_values([1, 2, 3], DataTypeId.I64),
            ByteSerie.from_values(["a", "b", "c"], DataTypeId.Utf8),
        ],
        names=["id", "name"],
    )
    table.set_column("id", Serie.from_values([10, 20, 30], DataTypeId.I64))
    assert table.column_by_name("id").to_list() == [10, 20, 30]
    # The untouched column is preserved.
    assert table.column_by_name("name").to_list() == ["a", "b", "c"]

    # A replacement of the wrong length is a guided ValueError.
    with pytest.raises(ValueError):
        table.set_column("id", Serie.from_values([1], DataTypeId.I64))
    # An unknown column name is a guided ValueError.
    with pytest.raises(ValueError):
        table.set_column("nope", Serie.from_values([1, 2, 3], DataTypeId.I64))


def test_nested_struct_column():
    city = ByteSerie.from_values(["nyc", "sf"], DataTypeId.Utf8)
    postcode = Serie.from_values([10001, 94107], DataTypeId.I32)
    address = StructSerie.from_columns([city, postcode], names=["city", "zip"])

    uid = Serie.from_values([1, 2], DataTypeId.I64)
    table = StructSerie.from_columns([uid, address], names=["id", "address"])

    assert table.num_columns() == 2
    assert table.column_names() == ["id", "address"]

    addr_col = table.column_by_name("address")
    assert isinstance(addr_col, StructSerie)
    assert addr_col.column_names() == ["city", "zip"]

    # A nested struct row marshals as a list of its child values.
    row = table.row(0)
    assert row[0] == 1
    assert row[1] == ["nyc", 10001]

    # The schema recurses: the "address" field is itself a StructField.
    schema = table.field()
    assert isinstance(schema, StructField)
    assert schema.names() == ["id", "address"]
    child = schema.field_by_name("address")
    assert isinstance(child, StructField)
    assert child.names() == ["city", "zip"]


def test_struct_length_mismatch():
    a = Serie.from_values([1, 2, 3], DataTypeId.I64)
    b = Serie.from_values([1, 2], DataTypeId.I64)
    with pytest.raises(ValueError):
        StructSerie.from_columns([a, b])
    # A names list of the wrong length is also a guided ValueError.
    with pytest.raises(ValueError):
        StructSerie.from_columns([a], names=["x", "y"])


def test_struct_rejects_non_column():
    with pytest.raises(TypeError):
        StructSerie.from_columns([123])


# -------------------------------------------------------------------------------------
# ListSerie — the variable-length list column
# -------------------------------------------------------------------------------------


def test_list_serie():
    child = Serie.from_values([1, 2, 3, 4, 5], DataTypeId.I64)
    lst = ListSerie(child, name="nums")
    lst.push(2)  # [1, 2]
    lst.push(0)  # []
    lst.push(3)  # [3, 4, 5]

    assert len(lst) == 3
    assert lst.list(0) == [1, 2]
    assert lst.list(1) == []  # a valid empty list
    assert lst.list(2) == [3, 4, 5]
    assert lst.list(99) is None  # out of range

    lst.push_null()
    assert len(lst) == 4
    assert lst.list(3) is None  # a null list (distinct from the empty [])
    assert lst.null_count() == 1

    values = lst.values()
    assert isinstance(values, Serie)
    assert values.to_list() == [1, 2, 3, 4, 5]

    field = lst.field()
    assert isinstance(field, ListField)
    assert field.name() == "nums"


# -------------------------------------------------------------------------------------
# MapSerie — the map column
# -------------------------------------------------------------------------------------


def test_map_serie():
    keys = ByteSerie.from_values(["a", "b", "c"], DataTypeId.Utf8)
    values = Serie.from_values([1, 2, 3], DataTypeId.I32)
    m = MapSerie(keys, values, name="m")
    m.push(2)  # {"a": 1, "b": 2}
    m.push(1)  # {"c": 3}

    assert len(m) == 2
    assert m.get(0) == {"a": 1, "b": 2}
    assert m.get(1) == {"c": 3}
    assert m.get(99) is None

    m.push_null()
    assert m.get(2) is None
    assert m.null_count() == 1

    assert isinstance(m.keys(), ByteSerie)
    assert m.keys().to_list() == ["a", "b", "c"]
    assert isinstance(m.values(), Serie)
    assert m.values().to_list() == [1, 2, 3]

    field = m.field()
    assert isinstance(field, MapField)


def test_map_nullable_key_error():
    keys = Serie.from_options([1, None, 3], DataTypeId.I32)  # a nullable key column
    values = Serie.from_values([1, 2, 3], DataTypeId.I32)
    with pytest.raises(ValueError):
        MapSerie(keys, values)


# -------------------------------------------------------------------------------------
# Schema descriptors — StructField / ListField / MapField
# -------------------------------------------------------------------------------------


def test_struct_field():
    schema = StructField(
        "person",
        [Field("id", DataTypeId.I64), Field("name", DataTypeId.Utf8, nullable=True)],
    )
    assert schema.name() == "person"
    assert schema.num_fields() == 2
    assert schema.names() == ["id", "name"]

    got = schema.field_by_name("name")
    assert isinstance(got, Field)
    assert got.dtype() == DataTypeId.Utf8
    assert got.nullable() is True

    assert schema.field(0).name() == "id"
    assert schema.field(9) is None

    same = StructField(
        "person",
        [Field("id", DataTypeId.I64), Field("name", DataTypeId.Utf8, nullable=True)],
    )
    assert schema == same
    assert hash(schema) == hash(same)
    assert {schema, same} == {schema}

    other = StructField("person", [Field("id", DataTypeId.I64)])
    assert schema != other


def test_empty_struct_field():
    schema = StructField("empty")
    assert schema.num_fields() == 0
    assert schema.names() == []
    assert schema.name() == "empty"


def test_list_and_map_field():
    item = Field("item", DataTypeId.I64, nullable=True)
    lf = ListField(item, name="scores")
    assert lf.name() == "scores"
    got_item = lf.item()
    assert isinstance(got_item, Field)
    assert got_item.dtype() == DataTypeId.I64

    lf_same = ListField(Field("item", DataTypeId.I64, nullable=True), name="scores")
    assert lf == lf_same
    assert hash(lf) == hash(lf_same)

    key = Field("key", DataTypeId.Utf8)
    value = Field("value", DataTypeId.I32, nullable=True)
    mf = MapField(key, value, name="prices", keys_sorted=True)
    assert mf.name() == "prices"
    assert mf.keys_sorted() is True
    assert isinstance(mf.key(), Field)
    assert mf.value().dtype() == DataTypeId.I32
