"""One Arrow cast compiled from two schemas, reused across every batch."""

from __future__ import annotations

import pyarrow as pa
import pytest

from yggdryl import ArrowCastPlan, DataType, Field
from yggdryl.enums import NULLABILITIES, REPRESENTATIONS

ROOT = Field("row", "struct<id:int64,symbol:utf8>", nullable=False)
SOURCE = pa.schema([pa.field("id", pa.int32()), pa.field("symbol", pa.string())])


def test_a_plan_answers_the_schemas_it_was_compiled_between() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)

    assert plan.field == ROOT
    assert plan.source_schema == SOURCE
    assert plan.schema.field("id").type == pa.int64()
    assert plan.safe
    assert plan.nullability == "default"
    assert plan.representation == "value"
    assert "ArrowCastPlan" in repr(plan)

    # The three cast answers cross explicitly, and each has a vocabulary.
    strict = ArrowCastPlan(SOURCE, ROOT, safe=False, nullability="strict", representation="bits")
    assert not strict.safe
    assert strict.nullability == "strict"
    assert strict.representation == "bits"
    assert set(NULLABILITIES) == {"default", "strict"}
    assert set(REPRESENTATIONS) == {"value", "bits"}

    with pytest.raises(ValueError):
        ArrowCastPlan(SOURCE, ROOT, nullability="lenient")


def test_one_plan_answers_every_batch_of_its_source_schema() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)

    first = plan.apply(
        pa.record_batch(
            {"id": pa.array([1, 2], pa.int32()), "symbol": pa.array(["AAPL", "MSFT"])},
        )
    )
    second = plan.apply(
        pa.record_batch({"id": pa.array([3], pa.int32()), "symbol": pa.array(["AMD"])})
    )

    assert first.schema == plan.schema
    assert second.schema == plan.schema
    assert first.column("id").to_pylist() == [1, 2]
    assert second.column("id").to_pylist() == [3]

    # The same answer the one-shot cast gives, so the plan is only reuse.
    assert first == ROOT.cast_arrow_batch(
        pa.record_batch(
            {"id": pa.array([1, 2], pa.int32()), "symbol": pa.array(["AAPL", "MSFT"])},
        )
    )


def test_an_exact_batch_is_handed_back_as_the_callers_own_object() -> None:
    exact = pa.record_batch(
        {"id": pa.array([1], pa.int64()), "symbol": pa.array(["AAPL"])}
    )
    plan = ArrowCastPlan(exact.schema, ROOT)

    assert plan.apply(exact) is exact


def test_a_plan_refuses_at_compile_time_what_two_schemas_alone_decide() -> None:
    required = Field.from_str(
        'field("row",struct<id:int64,symbol:utf8 not null>,nullable=false)'
    )
    partial = pa.schema([pa.field("id", pa.int32())])

    # `default` writes the target's own default into the column the source
    # cannot fill; `strict` refuses, and refuses before any batch exists.
    assert ArrowCastPlan(partial, required).schema.names == ["id", "symbol"]
    with pytest.raises(ValueError):
        ArrowCastPlan(partial, required, nullability="strict")

    # A column the target does not declare is dropped rather than refused,
    # so a wider source still compiles against a narrower root.
    wider = pa.schema(
        [
            pa.field("id", pa.int32()),
            pa.field("symbol", pa.string()),
            pa.field("venue", pa.string()),
        ]
    )
    assert ArrowCastPlan(wider, ROOT).schema.names == ["id", "symbol"]


def test_preflight_runs_the_plan_over_no_rows() -> None:
    plan = ArrowCastPlan(SOURCE, ROOT)
    assert plan.preflight() is None

    # Everything the values themselves decide is left to `apply`, because no
    # value has been seen yet: a plan over a narrowing conversion compiles and
    # preflights, and the out-of-range value is answered per batch.
    source = pa.schema([pa.field("id", pa.int64()), pa.field("symbol", pa.string())])
    target = Field("row", "struct<id:int8,symbol:utf8>", nullable=False)
    overflowing = pa.record_batch(
        {"id": pa.array([9999], pa.int64()), "symbol": pa.array(["AAPL"])}
    )

    lenient = ArrowCastPlan(source, target, safe=True)
    assert lenient.preflight() is None
    assert lenient.apply(overflowing).column("id").to_pylist() == [None]

    checked = ArrowCastPlan(source, target, safe=False)
    assert checked.preflight() is None
    with pytest.raises(ValueError):
        checked.apply(overflowing)


def test_a_datatype_casts_one_row_the_way_a_field_does() -> None:
    assert DataType("int64").cast_arrow_scalar(pa.array([1], pa.int32())).as_py() == 1
    assert DataType("int64").cast_arrow_scalar(5).as_py() == 5
    assert DataType("utf8").cast_arrow_scalar(pa.array(["AAPL"])).as_py() == "AAPL"

    # A scalar cast takes exactly one row.
    with pytest.raises(ValueError):
        DataType("int64").cast_arrow_scalar(pa.array([1, 2], pa.int32()))


def test_an_empty_text_cell_is_null_before_safe_is_asked() -> None:
    empty = pa.array([""], pa.string())

    # A zero-length text cell entering a column that does not hold text is
    # no value: it is null before any spelling is read, so `safe` never sees
    # it and both policies answer the same null.
    nullable = Field("quantity", "int32")
    assert nullable.cast_arrow_array(empty, safe=False).to_pylist() == [None]
    assert nullable.cast_arrow_array(empty, safe=True).to_pylist() == [None]

    # A required column then answers its nullability, exactly as it does for
    # a null the source carried: the default repairs it, strictness refuses
    # it naming the path and the count.
    required = Field("quantity", "int32", nullable=False)
    assert required.cast_arrow_array(empty).to_pylist() == [0]
    with pytest.raises(
        ValueError, match=r"required Arrow field \$\.quantity holds 1 null values"
    ):
        required.cast_arrow_array(empty, nullability="strict")

    # The same plan compiled between two schemas answers the same way.
    source = pa.schema([pa.field("quantity", pa.string())])
    target = Field("row", "struct<quantity:int32 not null>", nullable=False)
    batch = pa.record_batch({"quantity": empty})
    assert ArrowCastPlan(source, target, safe=False).apply(batch).column(
        "quantity"
    ).to_pylist() == [0]
    with pytest.raises(
        ValueError, match=r"required Arrow field \$\.quantity holds 1 null values"
    ):
        ArrowCastPlan(source, target, nullability="strict").apply(batch)

    # Text is text: into a string column the empty cell is the value it is.
    assert Field("symbol", "utf8").cast_arrow_array(empty).to_pylist() == [""]
    assert Field("symbol", "utf8", nullable=False).cast_arrow_array(
        empty, nullability="strict"
    ).to_pylist() == [""]

    # The scalar door reads the same rule.
    assert DataType("int32").scalar("").is_null()
    assert DataType("utf8").scalar("").as_py() == ""
    with pytest.raises(ValueError, match="non-nullable field received null"):
        Field("quantity", "int32", nullable=False).scalar("")
