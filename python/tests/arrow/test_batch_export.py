"""Exact-schema batch exports share buffers and keep readers lazy across threads."""

from __future__ import annotations

import threading
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor

import pyarrow as pa
import pytest

from yggdryl import ArrowScalar, DataType, Field


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
