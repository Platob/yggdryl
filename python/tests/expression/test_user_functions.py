"""A Python function is an expression function once it is registered."""

from __future__ import annotations

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Filter, Selector, Term
from yggdryl.expression import (
    UserFunction,
    unregister_user_function,
    user_defined_filter,
    user_defined_function,
    user_function_signature,
    user_functions,
)

ROWS = Field("rows", "struct<size: int64, ccy: utf8>", nullable=False)


def batch() -> pa.RecordBatch:
    return pa.record_batch(
        {"size": pa.array([1, None, 3], pa.int64()), "ccy": ["a", "b", "c"]}
    )


def test_a_decorated_function_is_registered_with_the_signature_its_hints_spell() -> None:
    @user_defined_function
    def double(value: int) -> int:
        return value * 2

    assert isinstance(double, UserFunction)
    assert double(4) == 8
    assert double.qualified == "py.double"
    assert "py.double" in user_functions()
    signature = double.signature
    assert signature.name == "py.double"
    assert [field.name for field in signature.dtype] == ["value"]
    assert signature.dtype["value"].dtype == DataType("int64")
    assert not signature.dtype["value"].nullable
    assert signature.metadata["function:returns"] == "int64 not null"
    assert user_function_signature("py.double") == signature

    # The call is a term like any other, and the two tiers agree.
    term = double.term("size")
    assert str(term) == "py.double(size)"
    assert Term("py.double(size)").field(ROWS).dtype == DataType("int64")
    selector = Selector("ccy, py.double(size) as doubled")
    projected = selector.apply_arrow_batch(batch())
    assert projected.column("doubled").to_pylist() == [2, None, 6]
    rows = selector.apply_records([{"size": 2, "ccy": "EUR"}], ROWS)
    assert rows.collect() == [{"ccy": "EUR", "doubled": 4}]
    assert double.unregister()
    assert "py.double" not in user_functions()


def test_defaults_fill_in_and_a_stored_column_derives_by_function() -> None:
    @user_defined_function(namespace="test")
    def add(value: int, amount: int = 1) -> int:
        return value + amount

    assert add.signature.dtype["amount"].metadata["function:default"] == "1"
    assert Selector("test.add(size) as next, test.add(size, 10) as later").apply_arrow_batch(
        batch()
    ).to_pydict() == {"next": [2, None, 4], "later": [11, None, 13]}

    stored = Selector("ccy, test.add(size) as next").into_field(ROWS)
    assert stored.dtype["next"].transform["function"] == "test.add"
    assert stored.dtype["next"].transform["sources"] == '["size"]'
    assert str(Selector.from_field(stored)) == "ccy utf8 null, test.add(size) as next int64 null"
    with pytest.raises(ValueError, match="test.add"):
        Term("test.add()").field(ROWS)
    add.unregister()


def test_a_vectorized_function_takes_whole_columns() -> None:
    @user_defined_function(vectorized=True, returns="utf8")
    def shout(text: str) -> str:
        import pyarrow.compute as pc

        return pc.utf8_upper(text)

    assert Selector("py.shout(ccy) as loud").apply_arrow_batch(batch()).column(
        "loud"
    ).to_pylist() == ["A", "B", "C"]
    shout.unregister()


def test_a_filter_function_keeps_the_rows_it_answers_true_for() -> None:
    @user_defined_filter
    def big(size: int | None) -> bool | None:
        return None if size is None else size > 1

    kept = big.where("size").apply_arrow_batch(batch())
    assert kept.column("ccy").to_pylist() == ["c"]
    assert Filter("py.big(size)").apply_arrow_batch(batch()).num_rows == 1
    assert Filter("py.big(size)").apply_records([{"size": 5, "ccy": "x"}], ROWS).collect() == [
        {"size": 5, "ccy": "x"}
    ]
    big.unregister()


def test_an_unregistered_function_is_refused_by_name() -> None:
    term = Term("nobody.knows(size)")
    with pytest.raises(ValueError, match="nobody.knows"):
        term.field(ROWS)
    with pytest.raises(ValueError, match="lives"):
        Term("9lives.f(size)")
    assert unregister_user_function("nobody.knows") is False

    with pytest.raises(TypeError, match="type hint"):

        @user_defined_function
        def untyped(value):  # type: ignore[no-untyped-def]
            return value
