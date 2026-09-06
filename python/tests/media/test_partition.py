"""The one partition renderer, and the option-driven shaping around it."""

from __future__ import annotations

import datetime
import decimal

import pyarrow as pa
import pytest

from yggdryl import Field, RecordOptions, TextOptions
from yggdryl.enums import LEADING_FRAGMENTS
from yggdryl.media import (
    AVRO_MAX_SCHEMA_DEPTH,
    DEFAULT_RECORD_BATCH_ROW_SIZE,
    NULL_PARTITION,
    partition_text,
    with_partitions,
    without_partitions,
)

ROOT = Field("row", "struct<id:int64,year:int32>", nullable=False)


def test_a_partition_directory_name_is_rendered_one_way() -> None:
    assert partition_text(datetime.date(2024, 1, 2)) == "2024-01-02"
    assert partition_text(decimal.Decimal("1.50")) == "1.50"
    assert partition_text(7) == "7"
    assert partition_text("AAPL") == "AAPL"
    assert partition_text(None) == NULL_PARTITION
    assert NULL_PARTITION == "null"

    assert DEFAULT_RECORD_BATCH_ROW_SIZE > 0
    assert AVRO_MAX_SCHEMA_DEPTH > 0
    assert set(LEADING_FRAGMENTS) == {"keep", "drop", "error"}


def test_partition_columns_move_between_the_path_and_the_rows() -> None:
    rows = pa.record_batch({"id": pa.array([1, 2], pa.int64())})

    widened = with_partitions(rows, [("year", "2024")], ROOT)
    assert widened.schema.names == ["id", "year"]
    # The text value is cast to the type the declared schema gives it.
    assert widened.schema.field("year").type == pa.int32()
    assert widened.column("year").to_pylist() == [2024, 2024]

    # Without a schema the value stays the text the path carried.
    untyped = with_partitions(rows, [("year", "2024")])
    assert untyped.column("year").to_pylist() == ["2024", "2024"]

    # The write-side mirror drops what the path already spells out.
    assert without_partitions(widened, [("year", "2024")]).schema.names == ["id"]
    # Rows carrying none of the named columns pass through unchanged.
    assert without_partitions(rows, [("year", "2024")]) == rows
    assert with_partitions(rows, []) == rows


def test_a_reader_is_widened_and_narrowed_without_being_drained() -> None:
    rows = pa.record_batch({"id": pa.array([1, 2], pa.int64())})

    widened = with_partitions(
        pa.RecordBatchReader.from_batches(rows.schema, [rows]),
        [("year", "2024")],
        ROOT,
    )
    # The widened schema is known before the first batch is pulled.
    assert widened.schema.names == ["id", "year"]
    batches = list(widened)
    assert len(batches) == 1
    assert batches[0].column("year").to_pylist() == [2024, 2024]

    with_column = pa.record_batch(
        {"id": pa.array([1], pa.int64()), "year": pa.array([2024], pa.int32())}
    )
    narrowed = without_partitions(
        pa.RecordBatchReader.from_batches(with_column.schema, [with_column]),
        [("year", "2024")],
    )
    assert narrowed.schema.names == ["id"]
    assert list(narrowed)[0].schema.names == ["id"]


def test_a_partition_column_names_the_expression_it_derives_with() -> None:
    column = Field("year", "int32")
    assert column.partition.expression is None

    column.partition.sources = ["event"]
    column.partition.transform = "year"
    expression = column.partition.expression
    assert expression is not None
    assert str(expression) == "year(event)"

    # The identity is a source with no transform.
    identity = Field("symbol", "utf8")
    identity.partition.sources = ["ticker"]
    assert str(identity.partition.expression) == "ticker"

    # The vocabulary belongs to the partition view alone.
    with pytest.raises(TypeError):
        Field("id", "int64").fix.expression


def test_options_shape_a_batch_and_a_reader_the_same_way() -> None:
    options = RecordOptions("application/vnd.apache.arrow.stream")
    options.field = ROOT
    options.select_by_names = ["id"]

    source = pa.record_batch(
        {"id": pa.array([1], pa.int32()), "year": pa.array([2024], pa.int32())}
    )
    cast = options.cast_arrow_batch(source)
    assert cast.schema.names == ["id"]
    assert cast.schema.field("id").type == pa.int64()

    reader = options.cast_arrow_reader(
        pa.RecordBatchReader.from_batches(source.schema, [source])
    )
    assert reader.schema.names == ["id"]
    assert list(reader)[0].column("id").to_pylist() == [1]

    assert options.require_field() == ROOT
    assert options.remove_field() == ROOT
    assert options.field is None
    with pytest.raises(ValueError, match="with_field"):
        options.require_field()


def test_a_limit_is_the_last_transform_and_stops_pulling() -> None:
    options = RecordOptions("application/vnd.apache.arrow.stream")
    options.max_row_size = 3

    batches = [
        pa.record_batch({"id": pa.array([1, 2], pa.int64())}),
        pa.record_batch({"id": pa.array([3, 4], pa.int64())}),
        pa.record_batch({"id": pa.array([5, 6], pa.int64())}),
    ]
    limited = options.limit_arrow_reader(
        pa.RecordBatchReader.from_batches(batches[0].schema, batches)
    )
    assert sum(batch.num_rows for batch in limited) == 3

    # With neither bound set the reader is the one that came in.
    plain = RecordOptions("application/vnd.apache.arrow.stream")
    reader = pa.RecordBatchReader.from_batches(batches[0].schema, batches)
    assert sum(batch.num_rows for batch in plain.limit_arrow_reader(reader)) == 6


def test_filter_partitions_read_as_a_path_query_and_as_a_row_predicate() -> None:
    options = RecordOptions("application/vnd.apache.arrow.stream")
    options.filter_partitions = [("year", "2024")]

    # The path form prunes a listing before anything is opened.
    assert "year" in str(options.partition_filter)
    assert "2024" in str(options.partition_filter)

    # The row form reads the same pair through the schema's own datatypes.
    predicate = options.partition_predicate(ROOT)
    assert "int32" in str(predicate)


def test_text_options_name_the_columns_a_rowheader_captures() -> None:
    options = TextOptions()
    assert options.capture_names == ()

    options.rowheader = r"^(?<stamp>\S+) (?<level>\w+) "
    assert options.capture_names == ("stamp", "level")

    # The captures are the fields between `url`/`rownum` and `body`.
    assert options.field is None
    assert options.write_batch_row_size is None
