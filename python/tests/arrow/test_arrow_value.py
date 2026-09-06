"""The one boundary between foreign columnar objects and native Arrow values."""

from __future__ import annotations

import pathlib

import numpy as np
import pyarrow as pa
import pytest

from yggdryl import ArrowValue, DataType, Field, IOBase, Scalar
from yggdryl.arrow import SHAPES

pandas = pytest.importorskip("pandas")
polars = pytest.importorskip("polars")


def quotes() -> pa.Table:
    return pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})


def root() -> Field:
    return Field(
        "row",
        "struct<symbol: utf8 not null, size: int64 not null>",
        nullable=False,
    )


class TestShapes:
    def test_every_shape_is_named_in_widening_order(self) -> None:
        assert SHAPES == ("scalar", "array", "batch", "stream")

    def test_a_held_container_keeps_the_length_it_knows(self) -> None:
        batch = ArrowValue.from_py(quotes().to_batches()[0])
        assert batch.shape == "batch"
        assert batch.row_size == 2
        assert batch.column_size == 2
        assert not batch.is_streamed

    def test_a_table_crosses_as_a_stream_because_it_may_hold_many_chunks(self) -> None:
        streamed = ArrowValue.from_py(quotes())
        assert streamed.shape == "stream"
        # A stream states its schema before its first batch, and nothing else.
        assert streamed.row_size is None
        assert streamed.column_size == 2
        assert streamed.is_streamed

    def test_a_column_and_a_pinned_row_are_told_apart(self) -> None:
        assert ArrowValue.from_py(pa.array([1, 2, 3])).shape == "array"
        assert ArrowValue.from_py(pa.scalar(7, pa.int64())).shape == "scalar"
        assert ArrowValue.from_py(pa.scalar(7, pa.int64())).row_size == 1


class TestIngestion:
    def test_a_chunked_column_is_combined_rather_than_truncated(self) -> None:
        chunked = pa.chunked_array([[1, 2], [3]])
        value = ArrowValue.from_py(chunked)
        assert value.shape == "array"
        assert value.row_size == 3

    def test_a_reader_crosses_without_being_pulled(self) -> None:
        value = ArrowValue.from_py(quotes().to_reader())
        assert value.shape == "stream"
        assert value.into_arrow_table().num_rows == 2

    def test_a_pandas_frame_is_converted_by_pandas(self) -> None:
        value = ArrowValue.from_py(quotes().to_pandas())
        assert value.shape == "stream"
        assert value.into_arrow_table().num_rows == 2

    def test_a_polars_frame_and_its_lazy_form_both_name_rows(self) -> None:
        frame = polars.from_arrow(quotes())
        for source in (frame, frame.lazy()):
            value = ArrowValue.from_py(source)
            assert value.shape == "stream"
            assert value.into_arrow_table().num_rows == 2

    def test_a_series_is_one_column_in_either_library(self) -> None:
        for series in (
            pandas.Series([1, 2, 3], name="size"),
            polars.Series("size", [1, 2, 3]),
        ):
            value = ArrowValue.from_py(series)
            assert value.shape == "array"
            assert value.row_size == 3

    def test_a_plain_numpy_array_is_one_column(self) -> None:
        value = ArrowValue.from_py(np.array([1.5, 2.5]))
        assert value.shape == "array"
        assert value.field.dtype == DataType("float64")
        assert value.into_arrow_array().to_pylist() == [1.5, 2.5]

    def test_a_numpy_record_dtype_names_its_members_so_it_is_rows(self) -> None:
        records = np.array(
            [("AAPL", 100), ("MSFT", 250)],
            dtype=[("symbol", "U4"), ("size", "i8")],
        )
        value = ArrowValue.from_py(records)
        assert value.shape == "batch"
        assert value.row_size == 2
        assert value.column_size == 2
        assert value.into_arrow_batch().column_names == ["symbol", "size"]

    def test_more_than_one_numpy_dimension_is_refused_by_name(self) -> None:
        with pytest.raises(TypeError, match="one-dimensional"):
            ArrowValue.from_py(np.zeros((2, 2)))

    def test_a_value_a_scalar_can_hold_is_the_last_resort(self) -> None:
        value = ArrowValue.from_py(7)
        assert value.shape == "scalar"
        assert value.as_py() == 7

    def test_a_native_value_is_taken_rather_than_re_read(self) -> None:
        value = ArrowValue.from_py(quotes().to_batches()[0])
        assert ArrowValue.from_py(value).row_size == 2


class TestDeclaredField:
    def test_the_declared_field_casts_in_rust(self) -> None:
        value = ArrowValue.from_py(pa.array([1, 2, 3]), "price: float64 not null")
        assert value.field.dtype == DataType("float64")
        assert value.into_arrow_array().type == pa.float64()

    def test_a_declared_root_reorders_and_retypes_a_table(self) -> None:
        declared = Field(
            "row",
            "struct<size: decimal128(12, 2) not null, symbol: utf8 not null>",
            nullable=False,
        )
        value = ArrowValue.from_py(quotes(), declared)
        assert value.into_arrow_table().column_names == ["size", "symbol"]

    def test_safe_decides_whether_a_failed_conversion_is_null_or_an_error(self) -> None:
        text = pa.array(["not a number"])
        # `safe` is Arrow's own answer: a supported conversion that fails
        # becomes null when it is true, and an error when it is false.
        nulled = ArrowValue.from_py(text, "size: int64", safe=True)
        assert nulled.into_arrow_array().to_pylist() == [None]

        with pytest.raises(ValueError, match="Cannot cast"):
            ArrowValue.from_py(text, "size: int64", safe=False)

    def test_cast_keeps_the_shape_it_was_given(self) -> None:
        value = ArrowValue.from_py(pa.array([1, 2, 3])).cast("price: float64 not null")
        assert value.shape == "array"
        assert value.row_size == 3


class TestCrossings:
    def test_a_held_shape_shares_its_buffers_back_and_stays_readable(self) -> None:
        held = ArrowValue.from_py(quotes().to_batches()[0])
        assert held.into_arrow_batch().num_rows == 2
        assert held.into_pandas().shape == (2, 2)
        assert held.into_polars().height == 2
        assert held.row_size == 2
        assert not held.is_consumed

    def test_a_stream_crosses_once_and_says_so(self) -> None:
        streamed = ArrowValue.from_py(quotes())
        assert streamed.into_arrow_table().num_rows == 2
        assert streamed.is_consumed
        with pytest.raises(ValueError, match="crosses once"):
            streamed.into_arrow_table()

    def test_every_arrow_export_answers_its_own_pyarrow_class(self) -> None:
        batch = quotes().to_batches()[0]
        assert isinstance(ArrowValue.from_py(batch).into_arrow_batch(), pa.RecordBatch)
        assert isinstance(ArrowValue.from_py(batch).into_arrow_table(), pa.Table)
        assert isinstance(
            ArrowValue.from_py(batch).into_arrow_reader(), pa.RecordBatchReader
        )
        assert isinstance(ArrowValue.from_py(pa.array([1])).into_arrow_array(), pa.Array)
        assert isinstance(
            ArrowValue.from_py(pa.scalar(1, pa.int64())).into_arrow_scalar(), pa.Scalar
        )

    def test_only_a_one_row_value_becomes_an_arrow_scalar(self) -> None:
        with pytest.raises(ValueError, match="one row"):
            ArrowValue.from_py(pa.array([1, 2, 3])).into_arrow_scalar()

    def test_numpy_takes_a_column_back(self) -> None:
        assert list(ArrowValue.from_py(pa.array([1, 2, 3])).into_numpy()) == [1, 2, 3]

    def test_the_native_value_model_is_one_call_away(self) -> None:
        value = ArrowValue.from_py(quotes().to_batches()[0], root())
        assert value.into_scalar() == Scalar.from_py(
            [["AAPL", 100], ["MSFT", 250]]
        )

    def test_as_py_names_what_the_field_types(self) -> None:
        # A canonical row is positional; the Field is what puts the names back.
        rows = ArrowValue.from_py(quotes().to_batches()[0], root()).as_py()
        assert rows == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]
        assert ArrowValue.from_py(pa.array([1, 2])).as_py() == [1, 2]
        assert ArrowValue.from_py(pa.scalar(7, pa.int64())).as_py() == 7


class TestHandles:
    @pytest.mark.parametrize(
        "name", ["quotes.json", "quotes.jsonl", "quotes.yaml", "quotes.toml"]
    )
    def test_every_structured_format_round_trips_a_table(
        self, tmp_path: pathlib.Path, name: str
    ) -> None:
        handle = IOBase(tmp_path / name)
        handle.write_arrow_value(quotes())

        read = handle.read_arrow_value(root())
        assert read.shape == "batch"
        assert read.as_py() == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]

    def test_a_record_encoding_answers_its_stream(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow_value(quotes())

        read = handle.read_arrow_value()
        assert read.shape == "stream"
        assert read.into_arrow_table().num_rows == 2

    def test_a_document_is_written_whole_so_only_an_overwrite_applies(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.json")
        with pytest.raises(ValueError, match="overwrite"):
            handle.write_arrow_value(quotes(), "append")

    def test_a_record_encoding_appends(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow_value(quotes())
        handle.write_arrow_value(quotes(), "append")
        assert handle.read_arrow_value().into_arrow_table().num_rows == 4

    def test_a_frame_reaches_the_same_publication_path(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.jsonl")
        handle.write_arrow_value(quotes().to_pandas())
        assert handle.read_arrow_value(root()).row_size == 2
