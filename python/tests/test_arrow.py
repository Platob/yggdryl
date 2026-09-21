"""The Arrow boundary: foreign columnar objects in, native values out.

Both halves are pinned here - ``python/yggdryl/arrow.py``'s caller-facing
surface and the ``python/src/arrow.rs`` conversions under it - because a
caller sees one boundary, not two.
"""

from __future__ import annotations

import pathlib
import threading
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor

import numpy as np
import pyarrow as pa
import pytest

from yggdryl import ArrowScalar, DataType, Field, IOBase, Scalar
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
        batch = ArrowScalar.from_(quotes().to_batches()[0])
        assert batch.shape == "batch"
        assert batch.row_size == 2
        assert batch.column_size == 2
        assert not batch.is_streamed

    def test_a_table_crosses_as_a_stream_because_it_may_hold_many_chunks(self) -> None:
        streamed = ArrowScalar.from_(quotes())
        assert streamed.shape == "stream"
        # A stream states its schema before its first batch, and nothing else.
        assert streamed.row_size is None
        assert streamed.column_size == 2
        assert streamed.is_streamed

    def test_a_column_and_a_pinned_row_are_told_apart(self) -> None:
        assert ArrowScalar.from_(pa.array([1, 2, 3])).shape == "array"
        assert ArrowScalar.from_(pa.scalar(7, pa.int64())).shape == "scalar"
        assert ArrowScalar.from_(pa.scalar(7, pa.int64())).row_size == 1


class TestIngestion:
    def test_a_chunked_column_is_combined_rather_than_truncated(self) -> None:
        chunked = pa.chunked_array([[1, 2], [3]])
        value = ArrowScalar.from_(chunked)
        assert value.shape == "array"
        assert value.row_size == 3

    def test_a_reader_crosses_without_being_pulled(self) -> None:
        value = ArrowScalar.from_(quotes().to_reader())
        assert value.shape == "stream"
        assert value.into_arrow_table().num_rows == 2

    def test_a_pandas_frame_is_converted_by_pandas(self) -> None:
        value = ArrowScalar.from_(quotes().to_pandas())
        assert value.shape == "stream"
        assert value.into_arrow_table().num_rows == 2

    def test_a_polars_frame_and_its_lazy_form_both_name_rows(self) -> None:
        frame = polars.from_arrow(quotes())
        for source in (frame, frame.lazy()):
            value = ArrowScalar.from_(source)
            assert value.shape == "stream"
            assert value.into_arrow_table().num_rows == 2

    def test_a_series_is_one_column_in_either_library(self) -> None:
        for series in (
            pandas.Series([1, 2, 3], name="size"),
            polars.Series("size", [1, 2, 3]),
        ):
            value = ArrowScalar.from_(series)
            assert value.shape == "array"
            assert value.row_size == 3

    def test_a_plain_numpy_array_is_one_column(self) -> None:
        value = ArrowScalar.from_(np.array([1.5, 2.5]))
        assert value.shape == "array"
        assert value.field.dtype == DataType("float64")
        assert value.into_arrow_array().to_pylist() == [1.5, 2.5]

    def test_a_numpy_record_dtype_names_its_members_so_it_is_rows(self) -> None:
        records = np.array(
            [("AAPL", 100), ("MSFT", 250)],
            dtype=[("symbol", "U4"), ("size", "i8")],
        )
        value = ArrowScalar.from_(records)
        assert value.shape == "batch"
        assert value.row_size == 2
        assert value.column_size == 2
        assert value.into_arrow_batch().column_names == ["symbol", "size"]

    def test_more_than_one_numpy_dimension_is_refused_by_name(self) -> None:
        with pytest.raises(TypeError, match="one-dimensional"):
            ArrowScalar.from_(np.zeros((2, 2)))

    def test_a_value_a_scalar_can_hold_is_the_last_resort(self) -> None:
        value = ArrowScalar.from_(7)
        assert value.shape == "scalar"
        assert value.as_py() == 7

    def test_a_native_value_is_taken_rather_than_re_read(self) -> None:
        value = ArrowScalar.from_(quotes().to_batches()[0])
        assert ArrowScalar.from_(value).row_size == 2


class TestDeclaredField:
    def test_the_declared_field_casts_in_rust(self) -> None:
        value = ArrowScalar.from_(pa.array([1, 2, 3]), "price: float64 not null")
        assert value.field.dtype == DataType("float64")
        assert value.into_arrow_array().type == pa.float64()

    def test_a_declared_root_reorders_and_retypes_a_table(self) -> None:
        declared = Field(
            "row",
            "struct<size: decimal128(12, 2) not null, symbol: utf8 not null>",
            nullable=False,
        )
        value = ArrowScalar.from_(quotes(), declared)
        assert value.into_arrow_table().column_names == ["size", "symbol"]

    def test_safe_decides_whether_a_failed_conversion_is_null_or_an_error(self) -> None:
        text = pa.array(["not a number"])
        # `safe` is Arrow's own answer: a supported conversion that fails
        # becomes null when it is true, and an error when it is false.
        nulled = ArrowScalar.from_(text, "size: int64", safe=True)
        assert nulled.into_arrow_array().to_pylist() == [None]

        with pytest.raises(ValueError, match="Cannot cast"):
            ArrowScalar.from_(text, "size: int64", safe=False)

    def test_cast_keeps_the_shape_it_was_given(self) -> None:
        value = ArrowScalar.from_(pa.array([1, 2, 3])).cast("price: float64 not null")
        assert value.shape == "array"
        assert value.row_size == 3


class TestCrossings:
    def test_a_held_shape_shares_its_buffers_back_and_stays_readable(self) -> None:
        held = ArrowScalar.from_(quotes().to_batches()[0])
        assert held.into_arrow_batch().num_rows == 2
        assert held.into_pandas().shape == (2, 2)
        assert held.into_polars().height == 2
        assert held.row_size == 2
        assert not held.is_consumed

    def test_a_stream_crosses_once_and_says_so(self) -> None:
        streamed = ArrowScalar.from_(quotes())
        assert streamed.into_arrow_table().num_rows == 2
        assert streamed.is_consumed
        with pytest.raises(ValueError, match="crosses once"):
            streamed.into_arrow_table()

    def test_every_arrow_export_answers_its_own_pyarrow_class(self) -> None:
        batch = quotes().to_batches()[0]
        assert isinstance(ArrowScalar.from_(batch).into_arrow_batch(), pa.RecordBatch)
        assert isinstance(ArrowScalar.from_(batch).into_arrow_table(), pa.Table)
        assert isinstance(
            ArrowScalar.from_(batch).into_arrow_reader(), pa.RecordBatchReader
        )
        assert isinstance(ArrowScalar.from_(pa.array([1])).into_arrow_array(), pa.Array)
        assert isinstance(
            ArrowScalar.from_(pa.scalar(1, pa.int64())).into_arrow_scalar(), pa.Scalar
        )

    def test_only_a_one_row_value_becomes_an_arrow_scalar(self) -> None:
        with pytest.raises(ValueError, match="one row"):
            ArrowScalar.from_(pa.array([1, 2, 3])).into_arrow_scalar()

    def test_numpy_takes_every_shape_back(self) -> None:
        assert list(ArrowScalar.from_(pa.array([1, 2, 3])).into_numpy()) == [1, 2, 3]
        assert ArrowScalar.from_(pa.scalar(7, pa.int64())).into_numpy().tolist() == [7]

        # NumPy has no counterpart for Arrow's null mask or its nested
        # layouts, so both are allowed to copy rather than being refused.
        nulled = ArrowScalar.from_(pa.array([1, None, 3])).into_numpy()
        assert np.isnan(nulled[1])

        # A struct column has no NumPy layout, so PyArrow's own conversion
        # answers an object array of mappings rather than a record array.
        rows = ArrowScalar.from_(quotes().to_batches()[0]).into_numpy()
        assert rows[0] == {"symbol": "AAPL", "size": 100}

    def test_the_native_value_model_is_one_call_away(self) -> None:
        value = ArrowScalar.from_(quotes().to_batches()[0], root())
        assert value.into_scalar() == Scalar.from_(
            [["AAPL", 100], ["MSFT", 250]]
        )

    def test_as_py_names_what_the_field_types(self) -> None:
        # A canonical row is positional; the Field is what puts the names back.
        rows = ArrowScalar.from_(quotes().to_batches()[0], root()).as_py()
        assert rows == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]
        assert ArrowScalar.from_(pa.array([1, 2])).as_py() == [1, 2]
        assert ArrowScalar.from_(pa.scalar(7, pa.int64())).as_py() == 7


class TestHandles:
    @pytest.mark.parametrize(
        "name", ["quotes.json", "quotes.jsonl", "quotes.yaml", "quotes.toml"]
    )
    def test_every_structured_format_round_trips_a_table(
        self, tmp_path: pathlib.Path, name: str
    ) -> None:
        handle = IOBase(tmp_path / name)
        handle.write_arrow(quotes())

        read = handle.read_arrow(field=root())
        assert read.shape == "batch"
        assert read.as_py() == [
            {"symbol": "AAPL", "size": 100},
            {"symbol": "MSFT", "size": 250},
        ]

    def test_a_record_encoding_answers_its_stream(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow(quotes())

        read = handle.read_arrow()
        assert read.shape == "stream"
        assert read.into_arrow_table().num_rows == 2

    def test_a_document_is_written_whole_so_only_an_overwrite_applies(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.json")
        with pytest.raises(ValueError, match="overwrite"):
            handle.write_arrow(quotes(), "append")

    def test_a_record_encoding_appends(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "quotes.arrows")
        handle.write_arrow(quotes())
        handle.write_arrow(quotes(), "append")
        assert handle.read_arrow().into_arrow_table().num_rows == 4

    def test_a_frame_reaches_the_same_publication_path(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "quotes.jsonl")
        handle.write_arrow(quotes().to_pandas())
        assert handle.read_arrow(field=root()).row_size == 2


def declared_batch(keys_sorted: bool) -> tuple[Field, pa.RecordBatch]:
    mapping = pa.map_(pa.string(), pa.string(), keys_sorted=keys_sorted)
    lookup = pa.field("lookup", mapping, metadata={b"owner": b"lookup"})
    nested = pa.field("nested", pa.struct([lookup]), metadata={b"owner": b"nested"})
    history = pa.field("history", pa.list_(lookup), metadata={b"owner": b"history"})
    category = Field("category", "dictionary(int16,utf8)")
    category.set_dictionary_options(29, True)
    category.metadata["owner"] = "category"
    root = Field(
        "row",
        DataType.from_fields(
            [Field.from_arrow(field) for field in [lookup, nested, history]] + [category]
        ),
        nullable=False,
    )
    root.metadata["owner"] = "root"
    schema = root.into_arrow_schema()
    values = [None, [], [("first", "one"), ("second", None)]]
    arrays = [
        pa.array(values, type=mapping),
        pa.array([{"lookup": value} for value in values], type=nested.type),
        pa.array([[value] for value in values], type=history.type),
        pa.DictionaryArray.from_arrays(
            pa.array([0, None, 1], type=pa.int16()),
            pa.array(["one", "two"]),
            ordered=True,
        ),
    ]
    return root, pa.RecordBatch.from_arrays(arrays, schema=schema)


def buffer_locations(array: pa.Array) -> list[tuple[int, int] | None]:
    return [
        None if buffer is None else (buffer.address, buffer.size)
        for buffer in array.buffers()
    ]


@pytest.mark.parametrize("whole_schema", [False, True])
def test_schema_capsule_import_calls_the_exporter_once_without_python_field_attributes(
    whole_schema: bool,
) -> None:
    root, batch = declared_batch(True)
    foreign = batch.schema if whole_schema else batch.schema.field("nested")
    calls = 0

    class SchemaExporter:
        def __arrow_c_schema__(self) -> object:
            nonlocal calls
            calls += 1
            return foreign.__arrow_c_schema__()

    source = SchemaExporter()
    assert not hasattr(source, "type")
    if whole_schema:
        imported = Field.from_arrow_schema(source, name=root.name)
        assert imported == root
        assert imported.into_arrow_schema().equals(batch.schema, check_metadata=True)
    else:
        imported = Field.from_arrow(source)
        assert imported == root.dtype["nested"]
        assert imported.into_arrow().equals(foreign, check_metadata=True)
    assert calls == 1


@pytest.mark.parametrize("keys_sorted", [False, True])
@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_preserve_nested_map_flags_metadata_and_shared_buffers(
    keys_sorted: bool, streamed: bool
) -> None:
    root, source = declared_batch(keys_sorted)
    held = ArrowScalar.from_(source)
    if streamed:
        reader = held.into_arrow_reader()
        assert reader.schema.equals(source.schema, check_metadata=True)
        result = reader.read_next_batch()
        with pytest.raises(StopIteration):
            reader.read_next_batch()
    else:
        result = held.into_arrow_batch()

    assert result.num_rows == source.num_rows == 3
    assert result.schema.equals(source.schema, check_metadata=True)
    assert result.equals(source, check_metadata=True)
    restored = Field.from_arrow_schema(result.schema, name=root.name)
    assert restored == root
    assert restored.dtype["category"].dictionary_id == 29
    assert restored.dtype["category"].dictionary_is_ordered is True
    assert restored.metadata["owner"] == "root"
    for mapping in [
        result.schema.field("lookup").type,
        result.schema.field("nested").type.field("lookup").type,
        result.schema.field("history").type.value_type,
    ]:
        assert mapping.keys_sorted is keys_sorted
        assert not mapping.key_field.nullable
        assert mapping.item_field.nullable
    for original, exported in zip(source.columns, result.columns):
        assert buffer_locations(exported) == buffer_locations(original)
    assert buffer_locations(result.column(3).dictionary) == buffer_locations(
        source.column(3).dictionary
    )


@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_keep_nonzero_row_counts_with_no_columns(streamed: bool) -> None:
    empty = pa.array([{}, {}, {}], type=pa.struct([]))
    source = pa.RecordBatch.from_struct_array(empty).replace_schema_metadata(
        {b"owner": b"empty"}
    )
    held = ArrowScalar.from_(source)
    result = (
        held.into_arrow_reader().read_next_batch()
        if streamed
        else held.into_arrow_batch()
    )
    assert result.num_columns == 0
    assert result.num_rows == 3
    assert result.schema.equals(source.schema, check_metadata=True)


def test_reader_export_pulls_one_batch_at_a_time_and_fuses_after_a_source_error() -> None:
    source = pa.record_batch({"value": [1, 2, 3]})
    pulled: list[str] = []

    def batches() -> Iterator[pa.RecordBatch]:
        pulled.append("first")
        yield source
        pulled.append("failure")
        raise ValueError("batch source refused")

    incoming = pa.RecordBatchReader.from_batches(source.schema, batches())
    held = ArrowScalar.from_(incoming)
    reader = held.into_arrow_reader()
    assert pulled == []
    assert reader.schema.equals(source.schema, check_metadata=True)
    assert pulled == []
    first = reader.read_next_batch()
    assert pulled == ["first"]
    assert first.equals(source)
    assert buffer_locations(first.column(0)) == buffer_locations(source.column(0))
    with pytest.raises(pa.ArrowInvalid, match="batch source refused"):
        reader.read_next_batch()
    assert pulled == ["first", "failure"]
    for _ in range(2):
        with pytest.raises(StopIteration):
            reader.read_next_batch()
    assert pulled == ["first", "failure"]


def test_reader_export_can_be_consumed_by_a_python_worker_thread() -> None:
    _, source = declared_batch(True)
    constructed_on = threading.get_ident()
    pulled_on: list[int] = []

    def batches() -> Iterator[pa.RecordBatch]:
        for _ in range(2):
            pulled_on.append(threading.get_ident())
            yield source

    incoming = pa.RecordBatchReader.from_batches(source.schema, batches())
    reader = ArrowScalar.from_(incoming).into_arrow_reader()
    assert pulled_on == []
    with ThreadPoolExecutor(max_workers=1) as pool:
        exported = pool.submit(lambda: list(reader)).result(timeout=10)
    assert len(exported) == len(pulled_on) == 2
    assert all(thread != constructed_on for thread in pulled_on)
    for result in exported:
        assert result.equals(source, check_metadata=True)
        for original, column in zip(source.columns, result.columns):
            assert buffer_locations(column) == buffer_locations(original)


@pytest.mark.parametrize("streamed", [False, True])
def test_batch_exports_rehydrate_registered_extension_identity_without_copying(
    streamed: bool,
) -> None:
    class BatchExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.arrow.batch-export")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> BatchExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = BatchExtension()
    pa.register_extension_type(extension)
    try:
        storage = pa.array([1, None, 3], type=pa.int32())
        array = pa.ExtensionArray.from_storage(extension, storage)
        schema = pa.schema(
            [pa.field("payload", extension, metadata={b"owner": b"payload"})],
            metadata={b"owner": b"extension"},
        )
        source = pa.RecordBatch.from_arrays([array], schema=schema)
        held = ArrowScalar.from_(source)
        if streamed:
            reader = held.into_arrow_reader()
            assert reader.schema.equals(schema, check_metadata=True)
            result = reader.read_next_batch()
        else:
            result = held.into_arrow_batch()
        assert result.equals(source, check_metadata=True)
        assert result.schema.equals(schema, check_metadata=True)
        assert result.column(0).type == extension
        assert result.column(0).type.__arrow_ext_serialize__() == b"v1"
        assert buffer_locations(result.column(0).storage) == buffer_locations(storage)
    finally:
        pa.unregister_extension_type(extension.extension_name)


@pytest.mark.parametrize("keys_sorted", [False, True])
@pytest.mark.parametrize("scalar", [False, True])
def test_nonexact_datatype_cast_exports_nested_map_flags_and_metadata(
    keys_sorted: bool, scalar: bool
) -> None:
    def nested(value_type: pa.DataType) -> pa.DataType:
        return pa.struct(
            [
                pa.field(
                    "lookup",
                    pa.map_(pa.string(), value_type, keys_sorted=keys_sorted),
                    metadata={b"owner": b"lookup"},
                )
            ]
        )

    source = pa.array(
        [{"lookup": [("first", 1), ("second", None)]}, {"lookup": None}],
        type=nested(pa.int16()),
    )
    expected_type = nested(pa.int64())
    dtype = DataType.from_arrow(expected_type)
    if scalar:
        original = source[0]
        result = dtype.cast_arrow_scalar(original)
        assert result.as_py() == original.as_py()
    else:
        original = source
        result = dtype.cast_arrow_array(original)
        assert result.to_pylist() == original.to_pylist()
    assert result is not original
    assert result.type.equals(expected_type, check_metadata=True)
    mapping = result.type.field("lookup")
    assert mapping.metadata == {b"owner": b"lookup"}
    assert mapping.type.keys_sorted is keys_sorted
    assert not mapping.type.key_field.nullable
    assert mapping.type.item_field.nullable


@pytest.mark.parametrize("keys_sorted", [False, True])
def test_exact_map_batch_casts_return_the_original_python_object(keys_sorted: bool) -> None:
    _, batch = declared_batch(keys_sorted)
    source = batch.select(["lookup", "nested", "history"]).replace_schema_metadata(None)
    field = Field.from_arrow_schema(source.schema, name="row")
    assert field.cast_arrow_batch(source) is source
    assert field.dtype.cast_arrow_batch(source) is source
