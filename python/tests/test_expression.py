"""The filter, from Python: one grammar, one meaning, one tree."""

from __future__ import annotations

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Expression, Field, Statement
from yggdryl.iceberg import Table


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


def test_text_parses_and_round_trips() -> None:
    filter = Expression("ccy = 'EUR' and price > 100")
    assert str(filter) == "ccy = 'EUR' and price > 100"
    assert Expression(str(filter)) == filter
    assert filter.columns() == ["ccy", "price"]
    assert repr(filter) == "Expression(\"ccy = 'EUR' and price > 100\")"


def test_text_is_never_taken_as_a_literal() -> None:
    # The one failure this layer must not have: a filter that silently matches
    # everything because its text became a string constant.
    with pytest.raises(ValueError):
        Expression("ccy = ")


def test_a_document_round_trips() -> None:
    filter = Expression("size between 1 and 10")
    assert Expression.from_json(filter.to_json()) == filter


def test_operators_build_the_tree() -> None:
    left = Expression("ccy = 'EUR'")
    right = Expression("size > 1")
    assert str(left & right) == "ccy = 'EUR' and size > 1"
    assert str(left | right) == "ccy = 'EUR' or size > 1"
    assert str(~left) == "not ccy = 'EUR'"
    # Text is accepted wherever an expression is, and parses.
    assert str(left & "size > 1") == "ccy = 'EUR' and size > 1"


def test_binding_resolves_and_folds() -> None:
    bound = Expression("price > 100 and size is not null").bind(trades_schema())
    assert bound.is_predicate
    assert bound.columns == ["price", "size"]
    # The literal is converted once, into the column's own exact type.
    assert str(bound.expression) == "price > decimal128(9,2) '100.00' and size is not null"


def test_rows_answer_either_spelling() -> None:
    bound = Expression("ccy = 'EUR' and size > 1").bind(trades_schema())
    assert bound.matches(["EUR", None, 5])
    assert not bound.matches(["USD", None, 5])
    assert bound.matches({"ccy": "EUR", "size": 5})
    # Unknown is not true: a null size does not pass `size > 1`.
    assert not bound.matches({"ccy": "EUR", "size": None})


def test_parameters_are_supplied_at_bind() -> None:
    filter = Expression("size >= :floor")
    assert filter.parameters() == ["floor"]
    with pytest.raises(ValueError):
        filter.bind(trades_schema())
    bound = filter.bind(trades_schema(), {"floor": 10})
    assert str(bound.expression) == "size >= 10"
    assert bound.matches({"size": 11})


def test_holder_attributes_are_their_own_question() -> None:
    filter = Expression("&holder.partition['year'] = '2024' and &holder.size > 0")
    assert filter.attributes() == ["partition['year']", "size"]
    assert filter.columns() == []
    bound = filter.bind(Field("holder", DataType.from_fields([]), False))
    assert not bound.reads_rows


def test_a_statement_carries_the_whole_read() -> None:
    statement = Statement("select ccy, price as amount where ccy = 'EUR' limit 10")
    assert statement.projections == ["ccy", "amount"]
    assert str(statement.predicate) == "ccy = 'EUR'"
    assert statement.limit == 10
    assert str(Statement(str(statement))) == str(statement)
    assert Statement.from_json(statement.to_json()).to_json() == statement.to_json()


def test_a_lake_is_filtered_by_the_same_predicate(tmp_path) -> None:
    lake = tmp_path / "lake"
    for year in ("2024", "2025"):
        part = lake / f"year={year}"
        part.mkdir(parents=True)
        (part / "part-0.parquet").write_bytes(b"")
    handle = yggdryl.IOBase(lake)

    # A listing yields whatever the predicate does not rule out, containers
    # included; nothing under `year=2025` survives.
    matched = handle.children_matching("&holder.partition['year'] = '2024'")
    assert matched
    assert all("year=2024" in str(entry.url) for entry in matched)

    # The pair spelling selects the leaves rather than the directories, and it
    # selects the same ones.
    pairs = handle.children_where({"year": "2024"})
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
        "&holder.partition['venue'] = 'XLON'"
    ).read_all().column("id").to_pylist() == [3, 4]
    assert table.scan_matching("id >= 4").read_all().column("id").to_pylist() == [4]


def test_the_pair_spelling_and_the_expression_spelling_are_one_plan(tmp_path) -> None:
    table = trades_table(tmp_path)

    by_pair = table.plan([("venue", "XLON")])
    by_text = table.plan_matching("venue = 'XLON'")
    assert by_pair.files_planned == by_text["tasks"] == 2
    assert by_pair.manifests_skipped == by_text["manifests_skipped"] == 2
    assert by_pair.record_count == by_text["record_count"] == 2
    assert (
        table.scan_where({"venue": "XLON"}).read_all().column("id").to_pylist()
        == table.scan_matching("venue = 'XLON'").read_all().column("id").to_pylist()
        == [3, 4]
    )
