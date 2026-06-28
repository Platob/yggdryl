"""Tests for the yggdryl Python extension's Serie (Arrow-backed column).

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import copy
import pickle

import pytest

import yggdryl


def test_from_values_infers_type_and_reads():
    s = yggdryl.Serie("n", [1, None, 3])
    assert s.name == "n"
    assert len(s) == 3
    assert s.num_rows == 3
    assert s.null_count == 1
    assert str(s.data_type) == "int64"
    assert s.category == "primitive"
    assert s[0] == 1
    assert s[1] is None
    assert s[-1] == 3            # negative index
    assert s.to_list() == [1, None, 3]
    assert s.is_null(1)
    assert s.is_valid(0)
    with pytest.raises(IndexError):
        _ = s[5]


def test_infers_each_scalar_kind():
    assert str(yggdryl.Serie("b", [True, False]).data_type) == "bool"
    assert str(yggdryl.Serie("f", [1.5, 2.5]).data_type) == "float64"
    assert str(yggdryl.Serie("s", ["a", "b"]).data_type) == "utf8"
    assert yggdryl.Serie("s", ["a", None]).to_list() == ["a", None]
    blob = yggdryl.Serie("raw", [b"xy", None])
    assert blob.to_list()[0] == b"xy"
    # bool is checked before int (Python bool subclasses int), so a bool list with a
    # null stays bool rather than being read as int.
    b = yggdryl.Serie("b", [True, None, False])
    assert str(b.data_type) == "bool"
    assert b.to_list() == [True, None, False]


def test_dtype_argument_casts():
    s = yggdryl.Serie("n", [1, 2, 3], dtype="int32")
    assert str(s.data_type) == "int32"
    # a DataType instance works too
    s2 = yggdryl.Serie("n", [1, 2], dtype=yggdryl.DataType.from_str("float64"))
    assert str(s2.data_type) == "float64"
    assert s2[0] == 1.0
    # all-null needs an explicit dtype
    nulls = yggdryl.Serie("n", [None, None], dtype="int16")
    assert str(nulls.data_type) == "int16"
    assert nulls.null_count == 2
    with pytest.raises(Exception):
        yggdryl.Serie("n", [None, None])


def test_slice_head_resize():
    s = yggdryl.Serie("n", [10, 20, 30, 40])
    assert s.slice(1, 2).to_list() == [20, 30]
    assert s.head(2).to_list() == [10, 20]
    grown = s.resize(6)             # nullable grows with nulls
    assert grown.to_list() == [10, 20, 30, 40, None, None]
    assert s.resize(2).to_list() == [10, 20]


def test_cast_and_categorical():
    s = yggdryl.Serie("n", [1, 2, 3])
    wide = s.cast("float64")
    assert str(wide.data_type) == "float64"
    assert wide[0] == 1.0
    cat = yggdryl.Serie("c", ["a", "b", "a", "a"]).categorical()
    assert cat.is_materialized is False
    assert cat.to_list() == ["a", "b", "a", "a"]
    assert cat.materialize().is_materialized is True
    # dictionary accessors: distinct values stored once, a code per row
    assert cat.category_count == 2
    assert cat.code_at(0) == cat.code_at(2) == cat.code_at(3)
    assert cat.categories().to_list() == ["a", "b"]
    with pytest.raises(TypeError):
        yggdryl.Serie("n", [1, 2]).category_count  # not categorical


def test_cast_to_any_and_null():
    s = yggdryl.Serie("n", [1, 2, 3])
    # cast to `any` is a no-op that keeps the concrete type
    assert str(s.cast("any").data_type) == "int64"
    assert s.cast("any").value_at(0) == 1
    # cast to `null` builds an all-null column (Arrow has no cast-to-null)
    nulled = s.cast("null")
    assert str(nulled.data_type) == "null"
    assert nulled.num_rows == 3 and nulled.null_count == 3
    assert nulled.value_at(0) is None
    # a null column casts back to any type as an all-null fill
    assert nulled.cast("utf8").value_at(0) is None
    # build a null column directly, and round-trip it through the bytes
    direct = yggdryl.Serie("z", [None, None], dtype="null")
    assert str(direct.data_type) == "null"
    assert str(yggdryl.Serie.from_bytes(direct.to_bytes()).data_type) == "null"


def test_lazy_range_and_index():
    r = yggdryl.Serie.range(5)
    assert r.is_materialized is False
    assert r.to_list() == [0, 1, 2, 3, 4]
    r2 = yggdryl.Serie.range(3, start=10, step=5)
    assert r2.to_list() == [10, 15, 20]
    idx = yggdryl.Serie.index(4)
    assert idx.to_list() == [0, 1, 2, 3]
    # index lookups: label <-> position
    assert idx.is_range is True
    assert idx.at(2) == 2
    assert idx.position(3) == 3
    assert idx.contains(3) is True
    assert idx.contains(4) is False
    # a non-range column is not a range, and the index lookups require one
    assert yggdryl.Serie("n", [1, 2]).is_range is False
    assert r2.is_range is False  # start != 0
    assert r2.at(1) == 15
    assert r2.position(20) == 2
    with pytest.raises(TypeError):
        yggdryl.Serie("n", [1, 2]).at(0)  # not an index


def test_list_factory():
    nums = yggdryl.Serie.list("nums", [[1, 2], [], None, [3]])
    assert nums.category == "nested"
    assert nums.num_rows == 4
    assert nums.null_count == 1
    assert nums.value_at(0) == "[1, 2]"
    assert nums.value_at(3) == "[3]"
    assert nums.child(0).name == "item"  # the flattened element column
    # an explicit element dtype casts the elements
    typed = yggdryl.Serie.list("f", [[1], [2, 3]], dtype="float64")
    assert typed.child(0).data_type == yggdryl.DataType.from_str("float64")
    # round-trips losslessly through the column bytes
    assert yggdryl.Serie.from_bytes(nums.to_bytes()).value_at(0) == "[1, 2]"


def test_map_factory():
    m = yggdryl.Serie.map("m", [{"a": 1, "b": 2}, {"c": 3}, None])
    assert m.category == "nested"
    assert m.num_rows == 3
    assert m.null_count == 1
    assert m.value_at(0) == "{a=1, b=2}"
    assert m.value_at(1) == "{c=3}"
    assert yggdryl.Serie.from_bytes(m.to_bytes()).value_at(1) == "{c=3}"


def test_constructor_infers_nested():
    # a list value infers a list column
    a = yggdryl.Serie("a", [[1, 2], [], None, [3]])
    assert a.category == "nested"
    assert str(a.data_type) == "list[item: int64]"
    assert a.value_at(0) == "[1, 2]" and a.value_at(3) == "[3]"

    # a dict value infers a map column
    m = yggdryl.Serie("m", [{"x": 1, "y": 2}, {"z": 3}])
    assert m.category == "nested"
    assert m.value_at(0) == "{x=1, y=2}"

    # nesting composes: a list of dicts is list<map>, list of lists is list<list>
    assert str(yggdryl.Serie("ld", [[{"a": 1}]]).data_type) == "list[item: map[utf8, int64]]"
    assert str(yggdryl.Serie("ll", [[[1], [2]]]).data_type) == "list[item: list[item: int64]]"

    # the leaf dtype is still castable
    floats = yggdryl.Serie("f", [[1], [2, 3]], dtype="float64")
    assert str(floats.child(0).data_type) == "float64"


def test_nested_struct_and_select():
    a = yggdryl.Serie("a", [1, 2])
    b = yggdryl.Serie("b", ["x", "y"])
    rec = yggdryl.Serie.struct("rec", [a, b])
    assert rec.category == "nested"
    assert rec.children()[0].name == "a"
    assert rec.child("b").to_list() == ["x", "y"]
    assert rec.child(0).name == "a"
    assert rec.select("a")[1] == 2
    assert rec.select("missing") is None
    with pytest.raises(Exception):
        rec.select("a.")              # malformed path raises


def test_display_repr():
    s = yggdryl.Serie("n", list(range(100)))
    text = s.display(max_rows=3)
    assert "n: int64" in text
    assert "97 more rows" in text
    assert "Serie('n'" in repr(s)


def test_bytes_roundtrip_and_pickle():
    s = yggdryl.Serie("n", [1, None, 3])
    back = yggdryl.Serie.from_bytes(s.to_bytes())
    assert back.to_list() == [1, None, 3]
    assert bytes(s) == s.to_bytes()
    # pickle / copy round-trip through the IPC bytes (nested too)
    assert pickle.loads(pickle.dumps(s)).to_list() == [1, None, 3]
    rec = yggdryl.Serie.struct("rec", [yggdryl.Serie("a", [1, 2])])
    assert copy.copy(rec).select("a").to_list() == [1, 2]


def test_eq_and_hash():
    a = yggdryl.Serie("n", [1, 2, 3])
    b = yggdryl.Serie("n", [1, 2, 3])
    c = yggdryl.Serie("n", [1, 2, 4])
    assert a == b
    assert a != c
    assert hash(a) == hash(b)
    assert len({a, b}) == 1          # hashable, equal series collapse


def _frame():
    """A 3-row, 2-column frame: id int64, name utf8."""
    return yggdryl.Serie.struct(
        "df",
        [yggdryl.Serie("id", [3, 1, 2]), yggdryl.Serie("name", ["c", "a", "b"])],
    )


def test_frame_shape_and_projection():
    df = _frame()
    assert df.shape == (3, 2)
    assert df.num_columns == 2
    assert df.column_names == ["id", "name"]

    # projection / reorder
    assert df.select_columns(["name"]).column_names == ["name"]
    # add / drop / rename columns (functional)
    flagged = df.with_column(yggdryl.Serie("ok", [True, True, False]))
    assert flagged.column_names == ["id", "name", "ok"]
    assert df.drop_columns(["name"]).column_names == ["id"]
    assert df.rename("id", "key").column_names == ["key", "name"]


def test_frame_rows_filter_sort_stack():
    df = _frame()
    # sort ascending by id
    asc = df.sort_by("id")
    assert asc.to_dicts() == [
        {"id": 1, "name": "a"},
        {"id": 2, "name": "b"},
        {"id": 3, "name": "c"},
    ]
    # filter keeps the masked rows
    even = df.filter([True, False, True])
    assert even.shape == (2, 2)
    # vstack doubles the rows
    assert df.vstack(df).shape == (6, 2)
    # with_row_index prepends a 0..n column
    assert df.with_row_index("i").column_names == ["i", "id", "name"]


def test_frame_row_record_and_dataclass():
    df = _frame()
    record = df.row(1)                       # a Scalar struct
    assert record.to_dict() == {"id": 1, "name": "a"}
    dc = record.as_dataclass("Row")
    assert (dc.id, dc.name) == (1, "a")
    assert type(dc).__name__ == "Row"


def test_frame_select_fields_casts_and_fills():
    df = _frame()
    target = [
        yggdryl.Field("name", yggdryl.DataType("utf8"), True),
        yggdryl.Field("id", yggdryl.DataType("int64"), True),
        yggdryl.Field("score", yggdryl.DataType("float64"), True),
    ]
    projected = df.select_fields(target)
    assert projected.column_names == ["name", "id", "score"]
    assert projected.child("score").value_at(0) is None     # filled with null


def test_frame_arrow_ipc_roundtrip():
    df = _frame()
    ipc = df.to_arrow_ipc()
    back = yggdryl.Serie.from_arrow_ipc("df", ipc)
    assert back.shape == (3, 2)
    assert back.to_dicts() == df.to_dicts()


def test_set_at_and_push():
    s = yggdryl.Serie("n", [1, 2, 3])
    # set_at casts the value to the column type (safe by default) and is functional
    updated = s.set_at(1, yggdryl.Scalar(20))
    assert updated.to_list() == [1, 20, 3]
    assert s.to_list() == [1, 2, 3]
    # writing a typed null
    nulled = s.set_at(0, yggdryl.Scalar.null("int64"))
    assert nulled.to_list() == [None, 2, 3]
    # push appends a row
    assert s.push(yggdryl.Scalar(4)).to_list() == [1, 2, 3, 4]
    # out of bounds raises
    with pytest.raises(Exception):
        s.set_at(9, yggdryl.Scalar(1))


def test_frame_requires_struct():
    s = yggdryl.Serie("n", [1, 2, 3])
    with pytest.raises(Exception):
        _ = s.shape          # not a struct column
