"""Tests for the ``yggdryl.types`` Phase 9 mutation surface — nested **child-column** replace /
add (``set_child_at`` / ``set_child_by`` and their ``__setitem__`` twins), the length-preserving
**slice assignment** on every column (``set_slice`` and ``serie[a:b] = other``), and the
**cast-anything** arithmetic operands (the core coerces any convertible right operand).

Every method is a thin delegate to the erased ``dyn AnySerie`` core surface; every core guided
failure surfaces as a ``ValueError`` (a non-slice leaf assignment key is a ``TypeError``).
"""

import pytest

from yggdryl.decimal import D32Serie
from yggdryl.temporal import Ts64Serie
from yggdryl.types import (
    I32Serie,
    I64Serie,
    ListSerie,
    MapSerie,
    StructSerie,
    U8Serie,
    Utf8Serie,
)


# ---- set_child on a STRUCT column -------------------------------------------------------------


def test_struct_set_child_at_replaces_column_by_index():
    st = StructSerie([("id", I64Serie([1, 2, 3])), ("name", Utf8Serie(["a", "b", "c"]))])
    st.set_child_at(0, I32Serie([10, 20, 30]))
    # The slot's data + type change, its schema name is preserved.
    assert st.column(0).to_options() == [10, 20, 30]
    assert st.field(0).name == "id"
    assert st.field(0).type_name == "i32"


def test_struct_set_child_by_adds_or_replaces_column_dict_like():
    st = StructSerie([("id", I64Serie([1, 2, 3]))])
    # A brand-new name is an add.
    st.set_child_by("score", U8Serie([7, 8, 9]))
    assert st.num_columns == 2
    assert st.column_named("score").to_options() == [7, 8, 9]
    # An existing name is a replace.
    st.set_child_by("score", U8Serie([1, 1, 1]))
    assert st.num_columns == 2
    assert st.column_named("score").to_options() == [1, 1, 1]


def test_struct_setitem_int_serie_is_set_child_at():
    st = StructSerie([("id", I64Serie([1, 2])), ("name", Utf8Serie(["a", "b"]))])
    st[0] = I32Serie([9, 9])
    assert st.column(0).to_options() == [9, 9]
    assert st.field(0).type_name == "i32"


def test_struct_setitem_str_serie_is_set_child_by():
    st = StructSerie([("id", I64Serie([1, 2]))])
    st["tag"] = Utf8Serie(["x", "y"])
    assert st.num_columns == 2
    assert st.column_named("tag").to_options() == ["x", "y"]


def test_struct_setitem_scalar_value_stays_deep_cell_set():
    # A scalar / coords key is the EXISTING deep-cell set, untouched by the Serie-value dispatch.
    st = StructSerie([("id", I32Serie([1, 2, 3]))])
    st[0, 0] = 99
    assert st.column(0).to_options() == [99, 2, 3]
    # A None writes a null cell.
    st[0, 1] = None
    assert st.column(0).to_options() == [99, None, 3]


# ---- set_child on a LIST column ---------------------------------------------------------------


def test_list_set_child_replaces_item_by_index_and_name():
    lst = ListSerie(I32Serie([10, 20, 30, 40]), [0, 3, 3, 4])
    lst.set_child_at(0, I32Serie([1, 2, 3, 4]))
    assert lst.values.to_options() == [1, 2, 3, 4]
    lst.set_child_by("item", I32Serie([5, 6, 7, 8]))
    assert lst.values.to_options() == [5, 6, 7, 8]


def test_list_set_child_bad_index_is_guided_error():
    lst = ListSerie(I32Serie([10, 20, 30, 40]), [0, 3, 3, 4])
    with pytest.raises(ValueError, match="single child at index 0"):
        lst.set_child_at(1, I32Serie([1, 2, 3, 4]))


# ---- set_child on a MAP column ----------------------------------------------------------------


def test_map_set_child_replaces_keys_and_values():
    mp = MapSerie(Utf8Serie(["a", "b", "c"]), I32Serie([1, 2, 3]), [0, 2, 3])
    mp.set_child_by("value", I32Serie([10, 20, 30]))
    assert mp.values.to_options() == [10, 20, 30]
    mp.set_child_at(0, Utf8Serie(["x", "y", "z"]))
    assert mp.keys.to_options() == ["x", "y", "z"]
    # Index 1 is the value column too.
    mp.set_child_at(1, I32Serie([7, 8, 9]))
    assert mp.values.to_options() == [7, 8, 9]


def test_map_set_child_bad_name_is_guided_error():
    mp = MapSerie(Utf8Serie(["a", "b", "c"]), I64Serie([1, 2, 3]), [0, 2, 3])
    with pytest.raises(ValueError, match='"key" child and a "value" child'):
        mp.set_child_by("nope", I64Serie([1, 2, 3]))


# ---- set_child error paths --------------------------------------------------------------------


def test_set_child_length_mismatch_is_valueerror():
    st = StructSerie([("id", I64Serie([1, 2, 3]))])
    with pytest.raises(ValueError, match="length"):
        st.set_child_at(0, I32Serie([1, 2]))


def test_set_child_non_serie_value_is_valueerror():
    st = StructSerie([("id", I64Serie([1, 2, 3]))])
    with pytest.raises(ValueError, match="expected a yggdryl column"):
        st.set_child_at(0, 123)


def test_leaf_has_no_set_child():
    # set_child is a nested-only capability — a leaf column does not advertise it.
    assert not hasattr(I64Serie([1, 2, 3]), "set_child_at")
    assert not hasattr(I64Serie([1, 2, 3]), "set_child_by")


# ---- slice assignment on a leaf column --------------------------------------------------------


def test_set_slice_overwrites_range_and_preserves_length():
    s = I32Serie([0, 0, 0, 0, 0])
    s.set_slice(1, I32Serie([7, 8]))
    assert s.to_options() == [0, 7, 8, 0, 0]
    assert len(s) == 5


def test_slice_setitem_overwrites_range_and_preserves_length():
    s = I32Serie([0, 0, 0, 0, 0])
    s[1:3] = I32Serie([7, 8])
    assert s.to_options() == [0, 7, 8, 0, 0]
    assert len(s) == 5


def test_slice_setitem_nulls_pass_through():
    s = I32Serie([0, 0, 0])
    s[0:2] = I32Serie([9, None])
    assert s.to_options() == [9, None, 0]


def test_slice_setitem_length_mismatch_is_valueerror():
    s = I32Serie([0, 0, 0])
    with pytest.raises(ValueError, match="length-preserving"):
        s[0:2] = I32Serie([1, 2, 3])


def test_slice_setitem_step_must_be_one():
    s = I32Serie([0, 0, 0, 0])
    with pytest.raises(ValueError, match="step of 1"):
        s[0:4:2] = I32Serie([1, 2])


def test_set_slice_out_of_range_is_valueerror():
    s = I32Serie([0, 0, 0, 0, 0])
    with pytest.raises(ValueError, match="out of bounds"):
        s.set_slice(4, I32Serie([1, 2]))


def test_set_slice_incompatible_type_is_valueerror():
    s = I32Serie([0, 0, 0])
    with pytest.raises(ValueError, match="must match the leaf column"):
        s.set_slice(0, Utf8Serie(["a", "b", "c"]))


def test_leaf_int_key_setitem_is_typeerror():
    # A leaf column supports slice assignment only; an int key is a guided TypeError.
    s = I32Serie([0, 0, 0])
    with pytest.raises(TypeError, match="only slice assignment"):
        s[0] = I32Serie([1])


def test_set_slice_on_var_and_decimal_and_temporal_columns():
    # slice assignment reaches every column family via the shared macro.
    u = Utf8Serie(["a", "b", "c"])
    u[1:3] = Utf8Serie(["B", "C"])
    assert u.to_options() == ["a", "B", "C"]

    d = D32Serie(5, 2, ["1.00", "2.00", "3.00"])
    d.set_slice(0, D32Serie(5, 2, ["9.99"]))
    assert d[0] == "9.99"

    t = Ts64Serie("s", "naive", ["2021-01-01T00:00:00", "2021-01-02T00:00:00"])
    t.set_slice(0, Ts64Serie("s", "naive", ["2020-01-01T00:00:00"]))
    assert len(t) == 2


def test_nested_set_slice_is_guided_error():
    # A whole-row range overwrite on a nested column is unsupported (it would resize the child).
    st = StructSerie([("id", I64Serie([1, 2, 3]))])
    with pytest.raises(ValueError, match="nested"):
        st.set_slice(0, StructSerie([("id", I64Serie([9]))]))


# ---- cast-anything arithmetic operands --------------------------------------------------------


def test_add_coerces_utf8_serie_of_numeric_strings():
    # An i64 column marshals as an exact decimal STRING, so the result reads back as strings.
    out = I64Serie([1, 2, 3]).add(Utf8Serie(["5", "6", "7"]))
    assert isinstance(out, I64Serie)
    assert out.to_options() == ["6", "8", "10"]


def test_add_coerces_decimal_serie_operand():
    out = I64Serie([10, 20]).add(D32Serie(5, 2, ["1.00", "2.00"]))
    assert isinstance(out, I64Serie)
    assert out.to_options() == ["11", "22"]


def test_add_coerces_temporal_serie_operand():
    out = I64Serie([10, 20]).add(
        Ts64Serie("s", "naive", ["1970-01-01T00:00:01", "1970-01-01T00:00:02"])
    )
    assert isinstance(out, I64Serie)
    assert out.to_options() == ["11", "22"]


def test_add_non_numeric_string_operand_is_valueerror():
    with pytest.raises(ValueError, match="parse"):
        I64Serie([1, 2, 3]).add(Utf8Serie(["a", "b", "c"]))
