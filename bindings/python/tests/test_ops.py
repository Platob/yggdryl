"""Tests for the ``yggdryl.types`` Phase 8 surface — vectorized element-wise arithmetic
(``+ - * / %`` and the ``add`` / ``sub`` / ``mul`` / ``div`` / ``rem`` twins, serie×serie and
serie×scalar) plus the reshape coercions (``filter`` / ``fill_null`` / ``to_list`` / ``to_struct`` /
``to_map``), over the core ``dyn AnySerie`` ops.

Arithmetic is on the twelve numeric leaf columns and the three nested columns; the result type
follows the LEFT operand. Reshape is on every column type. Every core error surfaces as a
``ValueError`` with the core's guided text.
"""

import pytest

from yggdryl.decimal import D32Serie
from yggdryl.temporal import Ts64Serie
from yggdryl.types import (
    F64Serie,
    I8Serie,
    I32Serie,
    I64Serie,
    I128Serie,
    ListSerie,
    MapSerie,
    NullSerie,
    StructSerie,
    Utf8Serie,
)


# ---- serie × serie ----------------------------------------------------------------------------


def test_add_same_type():
    out = I32Serie([1, 2, 3]) + I32Serie([10, 20, 30])
    assert isinstance(out, I32Serie)
    assert out.to_options() == [11, 22, 33]


def test_cross_type_result_follows_left():
    # i32 + i64 -> i32 (the right is range-checked into the left's type).
    out = I32Serie([1, 2]) + I64Serie([10, 20])
    assert isinstance(out, I32Serie)
    assert out.to_options() == [11, 22]


def test_cross_type_float_left():
    # f64 + i32 -> f64 (the i32 casts up into f64).
    out = F64Serie([1.5, 2.5]) + I32Serie([1, 2])
    assert isinstance(out, F64Serie)
    assert out.to_options() == [2.5, 4.5]


def test_sub_mul_div_rem_named_and_operators_agree():
    a, b = I32Serie([10, 20, 30]), I32Serie([3, 4, 7])
    assert (a - b).to_options() == a.sub(b).to_options() == [7, 16, 23]
    assert (a * b).to_options() == a.mul(b).to_options() == [30, 80, 210]
    assert (a / b).to_options() == a.div(b).to_options() == [3, 5, 4]  # integer division
    assert (a % b).to_options() == a.rem(b).to_options() == [1, 0, 2]


def test_div_by_zero_is_a_null_cell():
    out = I32Serie([10, 20]).div(I32Serie([2, 0]))
    assert out.to_options() == [5, None]


def test_rem_by_zero_is_a_null_cell():
    out = I32Serie([10, 20]).rem(I32Serie([3, 0]))
    assert out.to_options() == [1, None]


def test_integer_overflow_wraps():
    # 127 + 1 wraps to -128 in i8 (serie and scalar paths agree).
    assert (I8Serie([127]) + I8Serie([1])).to_options() == [-128]
    assert (I8Serie([127]) + 1).to_options() == [-128]


def test_null_propagates():
    out = I32Serie([1, None, 3]) + I32Serie([10, 20, None])
    assert out.to_options() == [11, None, None]


# ---- serie × scalar (broadcast) --------------------------------------------------------------


def test_scalar_broadcast():
    out = I64Serie([1, 2, 3]) + 1
    # i64 crosses as an exact decimal string.
    assert out.to_options() == ["2", "3", "4"]


def test_scalar_broadcast_named():
    out = I32Serie([1, 2, 3]).mul(10)
    assert out.to_options() == [10, 20, 30]


def test_reverse_add_is_commutative():
    # 1 + serie routes through __radd__.
    out = 1 + I32Serie([1, 2, 3])
    assert out.to_options() == [2, 3, 4]


def test_reverse_mul_is_commutative():
    out = 2 * I32Serie([1, 2, 3])
    assert out.to_options() == [2, 4, 6]


def test_float_scalar_broadcast():
    out = F64Serie([1.0, 2.0]) - 0.5
    assert out.to_options() == [0.5, 1.5]


def test_null_scalar_yields_all_null():
    out = I32Serie([1, 2, 3]).add(None)
    assert out.to_options() == [None, None, None]


# ---- guided errors ---------------------------------------------------------------------------


def test_cross_type_out_of_range_right_is_guided():
    # 1000 does not fit i8: the right is range-checked into the left's type.
    with pytest.raises(ValueError):
        I8Serie([1]) + I64Serie([1000])


def test_non_numeric_operand_is_guided():
    # A non-numeric right column on a numeric op is a guided ValueError.
    with pytest.raises(ValueError):
        I64Serie([1]).add(Utf8Serie(["a"]))


def test_non_numeric_columns_have_no_arithmetic():
    # Arithmetic is scoped to the twelve numeric + three nested columns; var/decimal/temporal/null
    # columns deliberately expose no `add` (they would only ever error).
    assert not hasattr(Utf8Serie(["a"]), "add")
    assert not hasattr(D32Serie(5, 2, ["1.00"]), "add")


def test_length_mismatch_is_guided():
    with pytest.raises(ValueError):
        I32Serie([1, 2]).add(I32Serie([1, 2, 3]))


# ---- nested arithmetic -----------------------------------------------------------------------


def test_struct_arithmetic_is_field_wise():
    a = StructSerie([("x", I32Serie([1, 2])), ("y", I32Serie([10, 20]))])
    b = StructSerie([("x", I32Serie([3, 4])), ("y", I32Serie([30, 40]))])
    out = a + b
    assert isinstance(out, StructSerie)
    assert out.column_named("x").to_options() == [4, 6]
    assert out.column_named("y").to_options() == [40, 60]


def test_struct_scalar_broadcast_into_leaves():
    a = StructSerie([("x", I32Serie([1, 2])), ("y", I32Serie([10, 20]))])
    out = a.add(1)
    assert out.column_named("x").to_options() == [2, 3]
    assert out.column_named("y").to_options() == [11, 21]


def test_list_arithmetic_is_element_wise():
    left = I32Serie([1, 2, 3]).to_list()
    right = I32Serie([10, 20, 30]).to_list()
    out = left + right
    assert isinstance(out, ListSerie)
    assert out.values.to_options() == [11, 22, 33]


# ---- filter ----------------------------------------------------------------------------------


def test_filter_keeps_the_true_rows():
    out = I32Serie([1, 2, 3, 4]).filter([True, False, True, False])
    assert isinstance(out, I32Serie)
    assert out.to_options() == [1, 3]


def test_filter_length_mismatch_is_guided():
    with pytest.raises(ValueError):
        I32Serie([1, 2, 3, 4]).filter([True, False])


def test_filter_preserves_nulls():
    out = I32Serie([1, None, 3]).filter([True, True, False])
    assert out.to_options() == [1, None]


def test_filter_on_var_column():
    out = Utf8Serie(["a", "b", "c"]).filter([False, True, True])
    assert isinstance(out, Utf8Serie)
    assert out.to_options() == ["b", "c"]


# ---- fill_null -------------------------------------------------------------------------------


def test_fill_null_fills_with_a_native():
    out = I32Serie([1, None, 3]).fill_null(0)
    assert out.to_options() == [1, 0, 3]


def test_fill_null_none_is_a_no_op():
    out = I32Serie([1, None, 3]).fill_null(None)
    assert out.to_options() == [1, None, 3]


def test_fill_null_on_var_column():
    out = Utf8Serie(["a", None, "c"]).fill_null("z")
    assert out.to_options() == ["a", "z", "c"]


def test_fill_null_decimal_with_matching_carrier():
    # A decimal column fills from a single-element decimal Serie carrying the same scale.
    out = D32Serie(5, 2, ["1.00", None]).fill_null(D32Serie(5, 2, ["9.99"]))
    assert out.to_options() == ["1.00", "9.99"]


def test_fill_null_decimal_scale_mismatch_is_guided():
    col = D32Serie(5, 2, ["1.00", None])
    carrier = D32Serie(5, 1, ["9.9"])  # scale 1 != column scale 2
    with pytest.raises(ValueError):
        col.fill_null(carrier)


def test_fill_null_temporal_unit_mismatch_is_guided():
    col = Ts64Serie("us", "naive", ["2021-01-01T00:00:00", None])
    carrier = Ts64Serie("s", "naive", ["2021-01-01T00:00:00"])  # unit s != column us
    with pytest.raises(ValueError):
        col.fill_null(carrier)


def test_fill_null_on_null_column_only_accepts_none():
    # A null column has no element type: filling with a null is the identity.
    out = NullSerie(3).fill_null(None)
    assert isinstance(out, NullSerie)
    assert len(out) == 3
    with pytest.raises(ValueError):
        NullSerie(3).fill_null(0)


# ---- to_list / to_struct / to_map ------------------------------------------------------------


def test_to_list_lifts_each_row_to_a_singleton():
    out = I32Serie([1, 2, 3]).to_list()
    assert isinstance(out, ListSerie)
    assert len(out) == 3
    assert out.values.to_options() == [1, 2, 3]


def test_to_struct_wraps_in_a_one_field_struct():
    out = I32Serie([1, 2, 3]).to_struct("n")
    assert isinstance(out, StructSerie)
    assert out.num_columns == 1
    assert out.column_named("n").to_options() == [1, 2, 3]


def test_to_struct_defaults_the_field_name():
    out = I32Serie([1, 2]).to_struct()
    assert out.column_named("value").to_options() == [1, 2]


def test_to_map_from_two_column_struct():
    st = StructSerie([("k", I32Serie([1, 2])), ("v", I32Serie([10, 20]))])
    out = st.to_map()
    assert isinstance(out, MapSerie)
    assert len(out) == 2


def test_to_list_is_idempotent_on_a_list():
    once = I32Serie([1, 2]).to_list()
    twice = once.to_list()
    assert isinstance(twice, ListSerie)
    assert len(twice) == 2


def test_reshape_on_temporal_roundtrips_the_wrapper():
    # A temporal column reshapes and rewraps to its concrete wrapper (filter path).
    col = Ts64Serie("us", "naive", ["2021-01-01T00:00:00", None, "2021-01-03T00:00:00"])
    out = col.filter([True, False, True])
    assert isinstance(out, Ts64Serie)
    assert len(out) == 2


# ---- CROSS-BINDING PARITY --------------------------------------------------------------------
#
# The SAME cases as the Node suite's parity block (bindings/node/test/ops.test.js), asserting the
# SAME outcome. An arithmetic SCALAR operand is coerced to the LEFT column's element type: an
# integer column requires wholeness (a fractional operand is a guided error) and accepts a whole int
# or an integer string; a float column accepts any numeric string; a nested column infers an i128 /
# f64 broadcast; and a real-but-non-castable Serie right operand surfaces the core's guided error.


def test_parity_int_column_scalar_operand_acceptance():
    # A fractional operand into an integer column is a guided error (wholeness required). In JS
    # `2.0 === 2`, so only the genuinely fractional 2.5 / "2.5" are shared error cases; a Python
    # `float` (2.0) is additionally rejected here, since a float is never an integer operand.
    with pytest.raises(ValueError):
        I32Serie([1, 2, 3]) + 2.5
    with pytest.raises(ValueError):
        I32Serie([1, 2, 3]) + 2.0
    with pytest.raises(ValueError):
        I32Serie([1, 2, 3]) + "2.5"
    # A whole int, or an integer-valued numeric string, is accepted (range-checked into the column).
    assert (I32Serie([1, 2, 3]) + 2).to_options() == [3, 4, 5]
    assert (I32Serie([1, 2, 3]) + "5").to_options() == [6, 7, 8]
    # A whole scalar into a wide i64 column keeps working (i64 crosses as a decimal string).
    assert (I64Serie([1]) + 1).to_options() == ["2"]


def test_parity_float_column_accepts_numeric_string():
    assert (F64Serie([1.0]) + "2.5").to_options() == [3.5]


def test_parity_nested_broadcast_marshals_a_whole_int_at_i128():
    st = StructSerie([("big", I128Serie(["1", "2"]))])
    expected = [str(10**30 + 1), str(10**30 + 2)]
    # A whole int well beyond i64 marshals at i128, so it fits the i128 leaf (i64 would overflow).
    assert st.add(10**30).column_named("big").to_options() == expected
    # The integer-string form is accepted identically.
    assert st.add(str(10**30)).column_named("big").to_options() == expected
    # A value beyond i128::MAX errors in both bindings.
    with pytest.raises(ValueError):
        st.add(10**40)


def test_parity_uncastable_serie_right_operand_is_core_guided():
    # The core now coerces ANY convertible Serie right operand (a numeric utf8 / decimal / temporal
    # / wide-int column casts into the left type), so only a genuinely non-convertible value errors.
    # A non-numeric utf8 operand surfaces the core's guided parse error — identical text in Node.
    with pytest.raises(ValueError, match="cannot parse"):
        I64Serie([1]).add(Utf8Serie(["a"]))
    # A struct column can never be a leaf-column operand — its own guided error.
    with pytest.raises(ValueError, match="cannot use a struct column"):
        I64Serie([1]).add(StructSerie([("x", I64Serie([1]))]))
