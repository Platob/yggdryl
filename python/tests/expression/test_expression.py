"""Expressions, from Python: one grammar, one meaning, one tree."""

from __future__ import annotations

import copy
import pickle

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Expression, Field, Filter, Plan, Selector, Term
from yggdryl.iceberg import ScanPlan, Table


def trades_schema() -> Field:
    return Field(
        "trades",
        DataType.from_fields(
            [
                Field("ccy", "utf8", True),
                Field("price", "decimal128(9,2)", True),
                Field("size", "int64", True),
            ]
        ),
        False,
    )


# ---------------------------------------------------------------------------
# Terms
# ---------------------------------------------------------------------------


def test_text_parses_and_round_trips() -> None:
    term = Term("ccy = 'EUR' and price > 100")
    assert str(term) == "ccy = 'EUR' and price > 100"
    assert Term(str(term)) == term
    assert term.columns() == ["ccy", "price"]
    assert repr(term) == "Term(\"ccy = 'EUR' and price > 100\")"
    assert copy.copy(term) == term
    assert copy.deepcopy(term) == term
    assert pickle.loads(pickle.dumps(term)) == term
    assert term != str(term)
    assert hash(term) == hash(Term(str(term)))
    assert term.stable_hash() == Term(str(term)).stable_hash()


def test_text_is_never_taken_as_a_literal() -> None:
    # The one failure this layer must not have: a filter that silently matches
    # everything because its text became a string constant.
    with pytest.raises(ValueError):
        Term("ccy = ")
    with pytest.raises(ValueError):
        Filter("ccy = ")


def test_a_document_round_trips() -> None:
    term = Term("size between 1 and 10")
    assert Term.from_json(term.into_json()) == term
    selector = Selector("ccy, price * 2 as doubled")
    assert Selector.from_json(selector.into_json()) == selector
    filter = Filter("size > 1")
    assert Filter.from_json(filter.into_json()) == filter
    plan = Plan("select ccy from t where size > 1 limit 3")
    assert Plan.from_json(plan.into_json()) == plan
    expression = Expression("select ccy; where size > 1")
    assert Expression.from_json(expression.into_json()) == expression


def test_operators_build_the_tree() -> None:
    left = Term("ccy = 'EUR'")
    right = Term("size > 1")
    assert str(left & right) == "ccy = 'EUR' and size > 1"
    assert str(left | right) == "ccy = 'EUR' or size > 1"
    assert str(~left) == "not ccy = 'EUR'"
    # Text is accepted wherever a term is, and parses.
    assert str(left & "size > 1") == "ccy = 'EUR' and size > 1"


def test_arithmetic_builders_preserve_inference_and_operand_order() -> None:
    size = Term.column("size")
    tax = Term.column("tax")

    assert size.add(2) == Term("size + 2")
    assert size.subtract(2) == Term("size - 2")
    assert size.multiply(2) == Term("size * 2")
    assert size.divide(2) == Term("size / 2")
    assert size.remainder(2) == Term("size % 2")
    assert size.negate() == Term("-size")

    assert size + 2 == Term("size + 2")
    assert 2 + size == Term("2 + size")
    assert size - 2 == Term("size - 2")
    assert 2 - size == Term("2 - size")
    assert size * 2 == Term("size * 2")
    assert 2 * size == Term("2 * size")
    assert size / 2 == Term("size / 2")
    assert 2 / size == Term("2 / size")
    assert size % 2 == Term("size % 2")
    assert 2 % size == Term("2 % size")
    assert -size == Term("-size")

    # Strings remain expression text while non-strings become native values.
    assert size + "tax" == size + tax
    assert size + "'fee'" == Term("size + 'fee'")


def test_binding_resolves_and_folds() -> None:
    bound = Term("price > 100 and size is not null").bind(trades_schema())
    assert bound.is_predicate
    assert bound.columns == ["price", "size"]
    # The literal is converted once, into the column's own exact type.
    assert str(bound.term) == "price > decimal128(9,2) '100.00' and size is not null"
    assert "column price" in bound.explain()
    with pytest.raises(TypeError, match="unhashable"):
        hash(bound)


def test_rows_answer_either_spelling() -> None:
    bound = Term("ccy = 'EUR' and size > 1").bind(trades_schema())
    assert bound.matches(["EUR", None, 5])
    assert not bound.matches(["USD", None, 5])
    assert bound.matches({"ccy": "EUR", "size": 5})
    # Unknown is not true: a null size does not pass `size > 1`.
    assert not bound.matches({"ccy": "EUR", "size": None})


def test_parameters_are_supplied_at_bind() -> None:
    term = Term("size >= :floor")
    assert term.parameters() == ["floor"]
    with pytest.raises(ValueError):
        term.bind(trades_schema())
    bound = term.bind(trades_schema(), {"floor": 10})
    assert str(bound.term) == "size >= 10"
    assert bound.matches({"size": 11})


def test_holder_attributes_are_their_own_question() -> None:
    term = Term("&holder.partition['year'] = '2024' and &holder.size > 0")
    assert term.attributes() == ["partition['year']", "size"]
    assert term.columns() == []
    assert term.has_attributes
    bound = term.bind(Field("holder", DataType.from_fields([]), False))
    assert not bound.reads_rows
    # A predicate over the holder and the rows splits into the half a
    # partition path answers and the half the rows have to.
    mixed = Term("&holder.partition['year'] = '2024' and size > 0").bind(trades_schema())
    answerable, remaining = mixed.partition_split()
    assert isinstance(answerable, Filter)
    assert str(answerable) == "&holder.partition['year'] = '2024'"
    assert str(remaining) == "size > 0"


def test_a_term_composes_without_going_back_through_text() -> None:
    price = Term.column("price")
    symbol = Term.column("symbol")

    assert str(price.eq(100)) == "price = 100"
    assert str(price.ne(100)) == "price <> 100"
    assert str(price.lt(100)) == "price < 100"
    assert str(price.le(100)) == "price <= 100"
    assert str(price.gt(100)) == "price > 100"
    assert str(price.ge(100)) == "price >= 100"
    assert str(price.compare("is distinct from", 100)) == "price is distinct from 100"

    assert str(price.is_in([1, 2, 3])) == "price in (1, 2, 3)"
    assert str(price.between(10, 20)) == "price between 10 and 20"
    assert str(price.is_null()) == "price is null"
    assert str(price.is_not_null()) == "price is not null"
    assert str(symbol.like("'AA%'")) == "symbol like 'AA%'"
    assert str(symbol.ilike("'aa%'")) == "symbol ilike 'aa%'"
    assert str(symbol.glob("'*.parquet'")) == "symbol glob '*.parquet'"
    assert str(price.cast("int64")) == "cast(price as int64)"
    assert str(price.try_cast("int32")) == "try_cast(price as int32)"

    # Composed and parsed reach the same tree.
    assert price.gt(100) == Term("price > 100")
    assert Term.all([price.gt(1), symbol.eq("'AAPL'")]) == Term(
        "price > 1 and symbol = 'AAPL'"
    )

    with pytest.raises(ValueError):
        price.compare("approximately", 1)


def test_the_closed_vocabularies_are_named_rather_than_guessed() -> None:
    from yggdryl.expression import (
        COMPARISONS,
        FUNCTIONS,
        HOLDER_ATTRIBUTES,
        VERBS,
        needs_quoting,
    )

    assert "=" in COMPARISONS
    assert "is distinct from" in COMPARISONS
    assert "year" in FUNCTIONS
    assert len(FUNCTIONS) == 19
    assert "size" in HOLDER_ATTRIBUTES
    assert "url" in HOLDER_ATTRIBUTES
    assert VERBS == ("insert into", "insert overwrite", "upsert into", "delete from")

    assert str(Term.call("year", [Term.column("event")])) == "year(event)"
    with pytest.raises(ValueError):
        Term.call("median", [Term.column("price")])

    assert not needs_quoting("price")
    assert needs_quoting("total price")


def test_a_term_reaches_into_a_nested_value_and_names_a_constant() -> None:
    trade = Term.column("trade")

    assert str(trade.child("leg").at(0)) == "trade.leg[0]"
    assert str(trade.path(["leg", 0])) == "trade.leg[0]"
    assert trade.path(["leg", 0]) == trade.child("leg").at(0)
    # A negative position counts back from the end, and a run is a slice.
    assert str(trade.at(-1)) == "trade[-1]"
    assert str(trade.child("legs").slice(1, 3)) == "trade.legs[1:3]"
    assert str(trade.child("legs").slice(None, -1)) == "trade.legs[:-1]"

    literal = Term.literal(5)
    assert literal.is_literal
    value, dtype = literal.as_literal()
    assert value.as_py() == 5
    assert str(dtype) == "int64"
    assert Term.column("price").as_literal() is None
    assert Term.column("price").as_column() == "price"
    assert literal.as_column() is None

    assert str(Term.typed_literal("int32", 5)) == "int32 '5'"

    assert Term.always_true().is_always_true
    assert Term.always_false().is_always_false
    assert Term.all([]).is_always_true
    assert Term.any([]).is_always_false

    assert Term.column("price").gt(1).node_count() == 3
    assert Term.column("price").check_budget() is None


def test_a_conditional_is_built_from_its_branches() -> None:
    price = Term.column("price")
    branched = Term.case([(price.gt(100), "'high'")], "'low'")

    assert str(branched) == "case when price > 100 then 'high' else 'low' end"
    # An absent else means null, which is what SQL's CASE means.
    assert "else" not in str(Term.case([(price.gt(100), "'high'")]))


def test_a_simplification_keeps_the_answer_and_drops_nodes() -> None:
    assert str(Term("a = 1 or a = 2").simplify()) == "a in (1, 2)"
    assert str(Filter("not (a = 1 or a = 2)").simplify()) == "not a in (1, 2)"
    assert str(Selector("a as a, b + 0 as b").simplify()) == "a, b + 0 as b"
    assert str(Plan("select a where a = 1 or a = 2").simplify()) == "select a where a in (1, 2)"
    assert Term("a = 1 or a = 2").explain().startswith("or")


def test_a_bound_predicate_answers_a_whole_batch_at_once() -> None:
    root = Field("row", "struct<price:int64,symbol:utf8>", nullable=False)
    bound = Term.column("price").gt(100).bind(root)

    assert bound.schema == root
    assert bound.column_indices == [0]

    batch = pa.record_batch(
        {
            "price": pa.array([50, 150, 250], pa.int64()),
            "symbol": pa.array(["A", "B", "C"]),
        }
    )
    assert bound.filter_mask_arrow_batch(batch).to_pylist() == [False, True, True]
    assert bound.filter_arrow_batch(batch).column("price").to_pylist() == [150, 250]

    # A mask that keeps every row hands the caller's own batch back.
    everything = Term.always_true().bind(root)
    assert everything.filter_arrow_batch(batch) is batch

    values = Term.column("price").bind(root).evaluate_arrow_batch(batch)
    assert values.to_pylist() == [50, 150, 250]

    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch, batch])
    assert sum(part.num_rows for part in bound.filter_arrow_reader(reader)) == 4


def test_statistics_settle_a_container_without_reading_it() -> None:
    from yggdryl import Bounds

    root = Field("row", "struct<price:int64>", nullable=False)
    bound = Term.column("price").gt(100).bind(root)

    # False is a proof: nothing in this container can match.
    below = Bounds(rows=3).with_column("price", 1, 9, 0)
    assert not bound.statistics_prune(below)
    assert bound.statistics_certainty(below) is False

    # Straddling the bound: the statistics do not say, so the rows decide.
    straddling = Bounds(rows=3).with_column("price", 50, 250, 0)
    assert bound.statistics_prune(straddling)
    assert bound.statistics_certainty(straddling) is None

    # Entirely above: every row matches.
    above = Bounds(rows=3).with_column("price", 500, 900, 0)
    assert bound.statistics_prune(above)
    assert bound.statistics_certainty(above) is True

    assert straddling.column("price")[0].as_py() == 50
    assert straddling.column("price")[2] == 0
    assert straddling.column("absent") is None

    # A path's partition values are bounds too: one value, so minimum is maximum.
    partitioned = Bounds.from_partitions(
        Field("row", "struct<year:int32>", nullable=False), [("year", "2024")]
    )
    minimum, maximum, _ = partitioned.column("year")
    assert minimum.as_py() == maximum.as_py() == 2024

    # A holder attribute is a statistic too, the partition one by its column.
    held = Bounds(rows=1).with_attribute("partition", "2024", "2024", 0, key="year")
    minimum, maximum, _ = held.attribute("partition", key="year")
    assert minimum.as_py() == maximum.as_py() == "2024"


# ---------------------------------------------------------------------------
# Clauses
# ---------------------------------------------------------------------------


def rows_schema() -> Field:
    return Field(
        "rows",
        DataType.from_fields(
            [
                Field("ccy", "utf8", True),
                Field("size", "int64", True),
            ]
        ),
        False,
    )


def rows_batch(ccy: list[str], size: list[int | None]) -> pa.RecordBatch:
    return pa.record_batch(
        [pa.array(ccy, pa.string()), pa.array(size, pa.int64())],
        names=["ccy", "size"],
    )


def test_a_filter_is_a_where_clause() -> None:
    filter = Filter("where ccy = 'EUR' and size > 1")
    assert str(filter) == "ccy = 'EUR' and size > 1"
    assert Filter(str(filter)) == filter
    assert Filter(Term("ccy = 'EUR' and size > 1")) == filter
    assert filter.term == Term("ccy = 'EUR' and size > 1")
    assert [str(part) for part in filter.conjuncts()] == ["ccy = 'EUR'", "size > 1"]
    assert filter.columns() == ["ccy", "size"]
    assert not filter.is_always_true
    assert Filter.always_true().is_always_true
    assert Filter.all([]).is_always_true
    assert Filter.any([]).is_always_false
    assert str(filter & "size < 9") == "ccy = 'EUR' and size > 1 and size < 9"
    assert str(~filter) == "not (ccy = 'EUR' and size > 1)"
    assert repr(filter) == "Filter(\"ccy = 'EUR' and size > 1\")"
    assert pickle.loads(pickle.dumps(filter)) == filter
    assert copy.deepcopy(filter) == filter
    assert hash(filter) == hash(Filter(str(filter)))
    assert filter.apply_field(rows_schema()) == rows_schema()
    assert filter.bind(rows_schema()).is_predicate
    with pytest.raises(ValueError):
        Filter("size + 1").bind(rows_schema())

    batch = rows_batch(["EUR", "USD", "EUR"], [5, 5, 0])
    assert filter.apply_arrow_batch(batch).column("ccy").to_pylist() == ["EUR"]
    assert Filter.always_true().apply_arrow_batch(batch) is batch
    table = pa.Table.from_batches([batch, batch])
    assert filter.apply_arrow_table(table).num_rows == 2
    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch, batch])
    assert filter.apply_arrow_reader(reader).read_all().num_rows == 2
    assert isinstance(filter.apply_arrow(batch), pa.RecordBatch)
    assert isinstance(filter.apply_arrow(table), pa.Table)
    kept = filter.apply_records([{"ccy": "EUR", "size": 5}, {"ccy": "USD", "size": 5}])
    assert kept.field.name == "row"
    assert list(kept) == [{"ccy": "EUR", "size": 5}]


def test_a_selector_is_a_select_clause() -> None:
    selector = Selector("select ccy, size as quantity, size * 2 as doubled int64")
    assert str(selector) == "ccy, size as quantity, size * 2 as doubled int64"
    assert Selector(str(selector)) == selector
    assert selector.names == ["ccy", "quantity", "doubled"]
    assert selector.projections == ["ccy", "size as quantity", "size * 2 as doubled int64"]
    assert selector.columns() == ["ccy", "size"]
    assert len(selector) == 3
    assert not selector.is_all
    assert not selector.is_columns
    assert Selector.all().is_all
    assert str(Selector.all()) == "*"
    assert str(Selector.all_except(["size"])) == "* exclude (size)"
    assert Selector.all_except(["size"]).excluded == ["size"]
    assert Selector.from_columns(["size", "ccy"]).is_columns
    assert Selector(["ccy", (Term.column("size"), "quantity")]) == Selector("ccy, size as quantity")
    assert Selector.select([Term.column("ccy")]) == Selector("ccy")
    assert Selector(Term.column("ccy")) == Selector("ccy")
    assert str(selector.without_columns(["ccy"])) == "size as quantity, size * 2 as doubled int64"
    assert str(Selector("ccy").with_projection("size")) == "ccy, size"
    assert repr(Selector("ccy")) == 'Selector("ccy")'
    assert pickle.loads(pickle.dumps(selector)) == selector
    assert hash(selector) == hash(Selector(str(selector)))

    published = selector.apply_field(rows_schema())
    assert [field.name for field in published.dtype] == ["ccy", "quantity", "doubled"]
    bound = selector.bind(rows_schema())
    assert bound.schema == rows_schema()
    assert bound.output == published
    assert [str(projection.term) for projection in bound.projections] == ["ccy", "size", "size * 2"]
    assert not bound.is_all
    assert not bound.is_identity
    assert Selector.all().bind(rows_schema()).is_identity
    assert bound.apply_row({"ccy": "EUR", "size": 2}) == {"ccy": "EUR", "quantity": 2, "doubled": 4}
    assert "doubled" in bound.explain()

    batch = rows_batch(["A", "B"], [1, 2])
    projected = selector.apply_arrow_batch(batch)
    assert projected.schema.names == ["ccy", "quantity", "doubled"]
    assert projected.column("doubled").to_pylist() == [2, 4]
    assert bound.apply_arrow_batch(batch).column("quantity").to_pylist() == [1, 2]
    assert Selector.all().apply_arrow_batch(batch) is not None
    table = pa.Table.from_batches([batch, batch])
    assert selector.apply_arrow_table(table).column("doubled").to_pylist() == [2, 4, 2, 4]
    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch, batch])
    assert selector.apply_arrow_reader(reader).schema.names == ["ccy", "quantity", "doubled"]
    assert isinstance(selector.apply_arrow(batch), pa.RecordBatch)
    assert isinstance(selector.apply_arrow(table), pa.Table)
    assert isinstance(
        selector.apply_arrow(pa.RecordBatchReader.from_batches(batch.schema, [batch])),
        pa.RecordBatchReader,
    )
    array = pa.StructArray.from_arrays([batch.column("ccy"), batch.column("size")], names=["ccy", "size"])
    assert selector.apply_arrow_array(array).field("doubled").to_pylist() == [2, 4]
    rows = selector.apply_records([{"ccy": "A", "size": 1}, {"ccy": "B", "size": 2}])
    assert rows.field.dtype["doubled"].dtype == DataType("int64")
    assert rows.collect() == [
        {"ccy": "A", "quantity": 1, "doubled": 2},
        {"ccy": "B", "quantity": 2, "doubled": 4},
    ]


def test_a_selector_declares_columns_like_a_create_table() -> None:
    declared = Selector("id int64 not null, name utf8, size * 2 as doubled int32")
    root = Field("rows", "struct<id:int64,name:utf8,size:int64>", nullable=False)
    published = declared.apply_field(root)
    assert published.dtype["id"].dtype == DataType("int64")
    assert not published.dtype["id"].nullable
    assert published.dtype["doubled"].dtype == DataType("int32")

    # A field is a selector, and comes back as the one it was made from,
    # with the nullability every stored column carries spelled out.
    stored = declared.into_field(root)
    assert stored.dtype["doubled"].transform["expression"] == "size * 2"
    assert Selector.from_field(stored) == Selector(
        "id int64 not null, name utf8 null, size * 2 as doubled int32 null"
    )
    assert Selector.from_field(root) == Selector("id int64 null, name utf8 null, size int64 null")

    # A value the declared column cannot hold becomes null, unless the
    # column is required, where it is refused by name.
    batch = pa.record_batch({"n": pa.array([1, 1000], pa.int64())})
    nulled = Selector("n as small int8").apply_arrow_batch(batch)
    assert nulled.column("small").to_pylist() == [1, None]
    with pytest.raises(ValueError, match="small"):
        Selector("n as small int8 not null").apply_arrow_batch(batch)


def test_records_stream_both_ways() -> None:
    from yggdryl import Records

    rows = Selector("size * 10 as size").apply_records(
        [{"size": 1}, {"size": 2}], schema=Field("rows", "struct<size:int64>", nullable=False)
    )
    reader = rows.into_arrow_reader()
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.read_all().column("size").to_pylist() == [10, 20]
    with pytest.raises(ValueError, match="consumed"):
        rows.into_arrow_reader()
    batch = pa.record_batch({"size": pa.array([3, 4], pa.int64())})
    back = Records.from_arrow_reader(pa.RecordBatchReader.from_batches(batch.schema, [batch]))
    assert back.field.name == "row"
    assert list(back) == [{"size": 3}, {"size": 4}]
    with pytest.raises(ValueError, match="schema"):
        Selector("size").apply_records([])


# ---------------------------------------------------------------------------
# Plans and expressions
# ---------------------------------------------------------------------------


def test_a_plan_carries_the_whole_read() -> None:
    plan = Plan(
        "select ccy, size as quantity from lake.trades where size >= :floor "
        "order by size desc nulls first, ccy limit 2 offset 1"
    )
    assert str(plan) == (
        "select ccy, size as quantity from lake.trades where size >= :floor "
        "order by size desc nulls first, ccy limit 2 offset 1"
    )
    assert plan.selector == Selector("ccy, size as quantity")
    assert plan.source == "lake.trades"
    assert plan.source_plan is None
    assert plan.filter == Filter("size >= :floor")
    term, direction, nulls = plan.ordering[0]
    assert (str(term), direction, nulls) == ("size", "desc", "first")
    assert plan.ordering[1][1:] == ("asc", "last")
    assert plan.limit == 2
    assert plan.offset == 1
    assert plan.verb is None
    assert plan.schema is None
    assert plan.columns() == ["size", "ccy"]
    assert plan.read_columns() == ["size", "ccy"]
    assert plan.parameters() == ["floor"]
    assert not plan.is_empty
    assert not plan.is_identity
    assert Plan().is_empty
    assert Plan("select *").is_identity
    assert str(plan.read_sections()) == (
        "select ccy, size as quantity where size >= :floor "
        "order by size desc nulls first, ccy limit 2 offset 1"
    )
    assert repr(Plan("select a")) == 'Plan("select a")'
    assert Plan(str(plan)) == plan
    assert pickle.loads(pickle.dumps(plan)) == plan
    assert copy.deepcopy(plan) == plan
    assert hash(plan) == hash(Plan(str(plan)))
    assert plan.stable_hash() == Plan(str(plan)).stable_hash()
    assert plan != str(plan)
    other = Plan("select ccy limit 1")
    assert plan < other or other < plan
    assert "order by" in plan.explain()


def test_a_plan_is_built_section_by_section() -> None:
    price = Term.column("price")
    plan = (
        Plan()
        .with_create("id int64 not null, price float64", "trades")
        .with_write("merge into", "'file:///lake/trades.parquet'", ["id"])
        .with_select([price, (Term.column("symbol"), "sym")])
        .with_source("raw")
        .with_filter(price.gt(100))
        .with_ordering([(price, "desc", "last"), "sym"])
        .with_limit(10)
        .with_offset(2)
    )
    assert str(plan) == (
        "create trades (id int64 not null, price float64) "
        "upsert into 'file:///lake/trades.parquet' by (id) "
        "select price, symbol as sym from raw where price > 100 "
        "order by price desc, sym limit 10 offset 2"
    )
    assert plan.create_target == "trades"
    assert plan.root_name == "trades"
    assert plan.schema == Selector("id int64 not null, price float64")
    assert plan.verb == "upsert into"
    assert plan.write_target == "'file:///lake/trades.parquet'"
    assert plan.merge_by == Selector("id")
    assert plan.field().name == "trades"
    assert plan.field().dtype["price"].dtype == DataType("float64")
    assert plan == Plan(str(plan))
    assert Plan().with_merge_by("id").verb == "upsert into"
    assert Plan().with_write("append").verb == "insert into"
    assert Plan().with_write("replace into", "t").verb == "insert overwrite"
    assert str(Plan().with_write("delete", "t").with_filter("id = 1")) == "delete from t where id = 1"
    nested = Plan("select a from t").with_source(Plan("select a, b from u where b > 1"))
    assert str(nested) == "select a from (select a, b from u where b > 1)"
    assert nested.source_plan == Plan("select a, b from u where b > 1")
    with pytest.raises(ValueError):
        Plan().with_write("sideways")
    with pytest.raises(ValueError):
        Plan().with_ordering([(price, "sideways")])


def test_a_field_is_a_plan_and_a_plan_is_a_field() -> None:
    root = Field(
        "trades",
        DataType.from_fields([Field("id", "int64", False), Field("ccy", "utf8", True)]),
        False,
        metadata={"comment": "eu desk"},
    )
    plan = Plan.from_field(root)
    assert str(plan) == "create trades (id int64 not null, ccy utf8 null) with (comment = 'eu desk')"
    assert plan.root_metadata == {"comment": "eu desk"}
    assert plan.field() == root
    assert Plan(root) == plan
    assert Plan("select a from t").field() is None
    published = Plan("select id, ccy as currency where id > 1").field_from(root)
    assert [field.name for field in published.dtype] == ["id", "currency"]


def test_a_plan_shapes_a_stream_in_section_order() -> None:
    plan = Plan("select ccy, size as quantity where size >= 2 order by size desc limit 2")
    first = rows_batch(["A", "B", "C", "D"], [1, 4, 3, None])
    second = rows_batch(["E", "F"], [5, 6])

    projected_batch = plan.apply_arrow_batch(first)
    assert isinstance(projected_batch, pa.RecordBatch)
    assert projected_batch.schema.names == ["ccy", "quantity"]
    assert projected_batch.column("quantity").to_pylist() == [4, 3]
    assert isinstance(plan.apply_arrow(first), pa.RecordBatch)

    table = pa.Table.from_batches([first, second])
    projected_table = plan.apply_arrow_table(table)
    assert isinstance(projected_table, pa.Table)
    assert projected_table.column("quantity").to_pylist() == [6, 5]
    assert isinstance(plan.apply_arrow(table), pa.Table)

    reader = pa.RecordBatchReader.from_batches(first.schema, [first, second])
    projected_reader = plan.apply_arrow_reader(reader)
    assert isinstance(projected_reader, pa.RecordBatchReader)
    assert projected_reader.read_all().column("quantity").to_pylist() == [6, 5]

    # A key orders by a column the projection drops.
    dropped = Plan("select ccy where size is not null order by size desc")
    assert dropped.apply_arrow_batch(first).column("ccy").to_pylist() == ["B", "C", "A"]
    rows = Plan("select upper(ccy) as ccy order by size desc limit 1").apply_records(
        [{"ccy": "a", "size": 1}, {"ccy": "b", "size": 2}]
    )
    assert rows.collect() == [{"ccy": "B"}]

    with pytest.raises(TypeError, match="Table"):
        plan.apply_arrow_table(first)


def test_a_plan_runs_a_store_end_to_end(tmp_path) -> None:
    url = (tmp_path / "trades.arrow").as_uri()
    created = Plan(f"create '{url}' (id int64 not null, name utf8)").execute()
    assert created.read_all().num_rows == 0
    Plan(f"insert into '{url}'").apply_arrow_batch(
        pa.record_batch({"id": pa.array([1, 2, 3], pa.int64()), "name": pa.array(["a", "b", "c"])})
    )
    read = Plan(f"select name from '{url}' where id > 1 order by id desc").execute()
    assert isinstance(read, pa.RecordBatchReader)
    assert read.read_all().column("name").to_pylist() == ["c", "b"]
    Plan(f"upsert into '{url}' by (id)").apply_arrow_batch(
        pa.record_batch({"id": pa.array([2, 4], pa.int64()), "name": pa.array(["B", "d"])})
    )
    Plan(f"delete from '{url}' where id = 1").execute().read_all()
    stored = Plan(f"select * from '{url}' order by id").execute().read_all()
    assert stored.column("id").to_pylist() == [2, 3, 4]
    assert stored.column("name").to_pylist() == ["B", "c", "d"]
    # A sequence runs its steps in order over one stream.
    sequence = Expression("where id > 2; select name")
    assert sequence.apply_arrow_table(stored).column("name").to_pylist() == ["c", "d"]


def test_an_expression_is_whichever_clause_its_text_is() -> None:
    selector = Expression("select a, b")
    assert selector.kind == "select"
    assert selector.is_selector
    assert selector.as_selector() == Selector("a, b")
    assert selector.as_filter() is None
    filter = Expression("where a > 1")
    assert filter.kind == "where"
    assert filter.as_filter() == Filter("a > 1")
    plan = Expression("select a from t where a > 1")
    assert plan.kind == "plan"
    assert plan.as_plan() == Plan("select a from t where a > 1")
    sequence = Expression("select a; where a > 1")
    assert sequence.kind == "sequence"
    assert [step.kind for step in sequence.steps] == ["select", "where"]
    assert str(sequence) == "select a; where a > 1"
    assert Expression.sequence([Expression("select a"), filter]) == sequence
    assert Expression.sequence(["select a", "where a > 1"]) == sequence
    assert Expression.select("a, b") == selector
    assert Expression.filter(Term("a > 1")) == filter
    assert Expression.plan(Plan("select a, b")) == selector
    assert Expression(Plan("where a > 1")) == filter
    assert Expression("select *").is_identity
    assert selector.columns() == ["a", "b"]
    assert str(Expression("where a = 1 or a = 2").simplify()) == "where a in (1, 2)"
    assert Expression.from_json(sequence.into_json()) == sequence
    assert pickle.loads(pickle.dumps(sequence)) == sequence
    assert hash(sequence) == hash(Expression(str(sequence)))
    assert sequence.explain().startswith("sequence")
    root = rows_schema()
    assert [field.name for field in Expression("select size").apply_field(root).dtype] == ["size"]
    # Text has to say which clause it is.
    with pytest.raises(ValueError, match="select"):
        Expression("a > 1")


# ---------------------------------------------------------------------------
# Holders and tables
# ---------------------------------------------------------------------------


def test_a_lake_is_filtered_by_the_same_predicate(tmp_path) -> None:
    lake = tmp_path / "lake"
    for year in ("2024", "2025"):
        part = lake / f"year={year}"
        part.mkdir(parents=True)
        (part / "part-0.parquet").write_bytes(b"")
    handle = yggdryl.IOBase(lake)

    # A listing yields whatever the predicate does not rule out, containers
    # included; nothing under `year=2025` survives.
    matched = list(handle.children_matching("&holder.partition['year'] = '2024'"))
    assert matched
    assert all("year=2024" in str(entry.url) for entry in matched)
    assert len(list(handle.children_matching(Filter("&holder.partition['year'] = '2024'")))) == len(matched)

    # The pair spelling selects the leaves rather than the directories, and it
    # selects the same ones.
    pairs = list(handle.children_where({"year": "2024"}))
    assert len(pairs) == 1
    assert str(pairs[0].url).endswith("year=2024/part-0.parquet")


def trades_table(root) -> Table:
    """A venue-partitioned table whose XLON partition holds two rows.

    One commit is one manifest, so three commits give the manifest list rows to
    prune. The fourth adds a second row to a partition that already exists, which
    is what lets a predicate over the rows discriminate *within* a surviving file
    rather than being answered by the partition alone.
    """
    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("venue", pa.string()),
        ]
    )
    table = Table.create(yggdryl.IOBase(root / "trades"), schema, ["venue"])
    for identifier, venue in ((1, "XNAS"), (2, "XNYS"), (3, "XLON"), (4, "XLON")):
        batch = pa.record_batch({"id": [identifier], "venue": [venue]}, schema=schema)
        table.append(batch)
    return table


def test_a_partitioned_table_prunes_manifests_before_a_byte_is_read(tmp_path) -> None:
    table = trades_table(tmp_path)

    # The baseline: a predicate no summary can settle opens every manifest, so
    # the numbers below are pruning rather than a constant.
    whole = table.plan_matching("id >= 1")
    assert whole["manifests_read"] == 4
    assert whole["manifests_skipped"] == 0
    assert whole["tasks"] == 4

    # A manifest-list summary bounds each manifest's partition values, so a
    # question about the file is settled without opening the Avro.
    held = table.plan_matching("&holder.partition['venue'] = 'XNYS'")
    assert held["manifests_skipped"] == 3
    assert held["manifests_read"] == 1
    assert held["tasks"] == 1
    assert held["record_count"] == 1


def test_one_predicate_mixes_the_file_and_the_rows(tmp_path) -> None:
    table = trades_table(tmp_path)

    # Both halves are load-bearing: the holder conjunct leaves the two XLON rows
    # and the row conjunct keeps one of them, so neither can be dropped without
    # changing the answer.
    mixed = "id >= 4 and &holder.partition['venue'] = 'XLON'"
    rows = table.scan_matching(mixed).read_all()
    assert rows.column("id").to_pylist() == [4]
    assert table.scan_matching(
        Filter("&holder.partition['venue'] = 'XLON'")
    ).read_all().column("id").to_pylist() == [3, 4]
    assert table.scan_matching("id >= 4").read_all().column("id").to_pylist() == [4]


def test_the_pair_spelling_and_the_expression_spelling_are_one_plan(tmp_path) -> None:
    table = trades_table(tmp_path)

    by_pair = table.plan([("venue", "XLON")])
    by_text = table.plan_matching("venue = 'XLON'")
    assert by_pair.files_planned == by_text["tasks"] == 2
    assert by_pair.manifests_skipped == by_text["manifests_skipped"] == 2
    assert by_pair.record_count == by_text["record_count"] == 2
    same = table.plan([("venue", "XLON")])
    assert same == by_pair
    assert hash(same) == hash(by_pair)
    assert same.stable_hash() == by_pair.stable_hash()
    assert {by_pair: "planned"}[same] == "planned"
    assert copy.copy(by_pair) == by_pair
    assert copy.deepcopy(by_pair) == by_pair
    assert pickle.loads(pickle.dumps(by_pair)) == by_pair
    assert eval(repr(by_pair), {"ScanPlan": ScanPlan}) == by_pair
    assert by_pair != by_text
    assert (
        table.scan_where({"venue": "XLON"}).read_all().column("id").to_pylist()
        == table.scan_matching("venue = 'XLON'").read_all().column("id").to_pylist()
        == [3, 4]
    )
