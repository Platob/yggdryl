from __future__ import annotations

import dataclasses
import datetime as dt
import enum
import uuid
from decimal import Decimal
from pathlib import Path
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import Record, record


def _deep_schema(*, metadata: dict[bytes, bytes] | None = None) -> pa.Schema:
    item = pa.struct(
        [
            pa.field("label", pa.string(), nullable=False),
            pa.field("scores", pa.list_(pa.field("score", pa.int16(), False))),
        ]
    )
    return pa.schema(
        [
            pa.field("identifier", pa.uint32(), nullable=False),
            pa.field("amount", pa.decimal128(18, 4), nullable=False),
            pa.field("created", pa.timestamp("us", tz="UTC"), nullable=False),
            pa.field("items", pa.list_(pa.field("entry", item, False)), False),
            pa.field("attributes", pa.map_(pa.string(), pa.int64()), False),
        ],
        metadata=metadata,
    )


def _deep_values() -> dict[str, Any]:
    return {
        "identifier": "7",
        "amount": "12.3400",
        "created": "2026-08-15T10:30:00+00:00",
        "items": [{"label": "a", "scores": [1, "2"]}],
        "attributes": {"x": "3"},
    }


def test_arrow_schema_factory_preserves_physical_layout_and_metadata() -> None:
    source = _deep_schema(
        metadata={
            b"owner": b"tests",
            b"python.class": b"ImportedName",
            b"python.module": b"old.module",
        }
    )

    row_type = Record.from_arrow_schema(
        source, class_name="DeepRow", module="tests.generated"
    )

    projected = row_type.into_arrow_schema()
    assert dataclasses.is_dataclass(row_type)
    assert issubclass(row_type, Record)
    assert row_type.__name__ == "DeepRow"
    assert row_type.__module__ == "tests.generated"
    assert projected.equals(source, check_metadata=False)
    assert projected.field("identifier").type == pa.uint32()
    assert projected.field("amount").type == pa.decimal128(18, 4)
    assert projected.metadata == {
        b"owner": b"tests",
        b"python.class": b"DeepRow",
        b"python.kind": b"record",
        b"python.module": b"tests.generated",
        b"python.qualname": b"DeepRow",
    }
    assert row_type.schema_fields()[0] is row_type.schema_fields()[0]
    assert row_type.into_arrow_schema() is projected


def test_arrow_field_factory_preserves_struct_root_and_promotes_scalar() -> None:
    struct = pa.field(
        "physical_root",
        pa.struct([pa.field("value", pa.int8(), False)]),
        nullable=False,
        metadata={b"owner": b"source", b"python.class": b"Old"},
    )
    struct_type = Record.from_arrow_field(
        struct, class_name="StructRow", module="tests.generated"
    )
    projected = struct_type.into_arrow_field()

    assert projected.name == "physical_root"
    assert projected.nullable is False
    assert projected.type.equals(struct.type)
    assert projected.metadata[b"owner"] == b"source"
    assert projected.metadata[b"python.class"] == b"StructRow"

    scalar = pa.field("count", pa.uint8(), False, metadata={b"unit": b"items"})
    count_type = Record.from_arrow_field(scalar, class_name="Count")
    assert count_type.into_arrow_schema().field("count").equals(
        scalar, check_metadata=True
    )
    assert b"unit" not in (count_type.into_arrow_schema().metadata or {})
    assert count_type.from_dict({"count": "4"}).count == 4


@pytest.mark.parametrize(
    "name", ["not valid", "class", "to_dict", "from_arrow", "__yggdryl_schema__"]
)
def test_arrow_factory_rejects_invalid_or_reserved_columns(name: str) -> None:
    with pytest.raises(TypeError, match="identifier|reserved|conflict"):
        Record.from_arrow_schema(pa.schema([pa.field(name, pa.int64())]))


def test_arrow_factory_rejects_non_utf8_schema_metadata() -> None:
    schema = pa.schema([pa.field("value", pa.int64())], metadata={b"\xff": b"x"})
    with pytest.raises(TypeError, match="UTF-8"):
        Record.from_arrow_schema(schema)


def test_from_dicts_is_lazy_one_shot_and_uses_shared_caster() -> None:
    row_type = Record.from_arrow_schema(pa.schema([pa.field("value", pa.int16(), False)]))
    events: list[int] = []

    def rows() -> Any:
        events.append(0)
        yield {"value": "1"}
        events.append(1)
        yield {"value": "bad"}

    converted = row_type.from_dicts(rows())
    assert events == []
    assert next(converted).value == 1
    assert events == [0]
    with pytest.raises(TypeError, match=r"ArrowRecord\[1\]\.value"):
        next(converted)


def test_arrow_batch_table_reader_stream_and_iterable_imports_are_lazy() -> None:
    row_type = Record.from_arrow_schema(_deep_schema(), class_name="DeepRow")
    expected = row_type.from_dict(_deep_values())
    batch = row_type.into_arrow_record_batch([expected])
    table = pa.Table.from_batches([batch])
    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch])

    assert tuple(row_type.from_arrow_record_batch(batch)) == (expected,)
    assert tuple(row_type.from_arrow_table(table)) == (expected,)
    assert tuple(row_type.from_arrow_record_batch_reader(reader)) == (expected,)
    assert tuple(row_type.from_arrow(iter([batch, batch]))) == (expected, expected)

    reader = pa.RecordBatchReader.from_batches(batch.schema, [batch])

    class Stream:
        def __arrow_c_stream__(self, requested_schema: object = None) -> object:
            return reader.__arrow_c_stream__(requested_schema)

    assert tuple(row_type.from_arrow(Stream())) == (expected,)


def test_fixed_schema_reader_and_table_prepare_recursive_type_plans_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl.records._arrow as arrow_impl

    row_type = Record.from_arrow_schema(_deep_schema(), class_name="PlannedRow")
    value = row_type.from_dict(_deep_values())
    batch = row_type.into_arrow_record_batch([value])
    batches = [batch, batch, batch, batch]
    original = arrow_impl._prepare_type_plan
    calls = 0

    def counted(data_type: object, arrow_type: object) -> object:
        nonlocal calls
        calls += 1
        return original(data_type, arrow_type)

    monkeypatch.setattr(arrow_impl, "_prepare_type_plan", counted)

    assert tuple(row_type.from_arrow_record_batch(batch)) == (value,)
    calls_per_schema = calls
    assert calls_per_schema > len(row_type.schema_fields())

    calls = 0
    reader = pa.RecordBatchReader.from_batches(batch.schema, batches)
    assert tuple(row_type.from_arrow_record_batch_reader(reader)) == (value,) * 4
    assert calls == calls_per_schema

    calls = 0
    table = pa.Table.from_batches(batches)
    assert len(table.to_batches()) == 4
    assert tuple(row_type.from_arrow_table(table)) == (value,) * 4
    assert calls == calls_per_schema


def test_schema_compatibility_ignores_transport_metadata_but_not_layout() -> None:
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int16(), False, metadata={b"a": b"1"})])
    )
    compatible = pa.record_batch(
        [pa.array([1], type=pa.int16())],
        schema=pa.schema(
            [pa.field("value", pa.int16(), False, metadata={b"a": b"2"})],
            metadata={b"transport": b"different"},
        ),
    )
    assert next(row_type.from_arrow_record_batch(compatible)).value == 1

    incompatible = pa.record_batch(
        [pa.array([1], type=pa.int32())],
        schema=pa.schema([pa.field("value", pa.int32(), False)]),
    )
    with pytest.raises(TypeError, match="incompatible Arrow schema"):
        row_type.from_arrow_record_batch(incompatible)


def test_arrow_map_pairs_reject_duplicates_before_shared_casting() -> None:
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("values", pa.map_(pa.string(), pa.int64()), False)])
    )
    duplicate_map = pa.array(
        [[("key", 1), ("key", 2)]], type=pa.map_(pa.string(), pa.int64())
    )
    batch = pa.record_batch([duplicate_map], schema=row_type.into_arrow_schema())

    with pytest.raises(ValueError, match="duplicate Arrow map key"):
        next(row_type.from_arrow_record_batch(batch))

    nested_schema = pa.schema(
        [
            pa.field(
                "payload",
                pa.struct(
                    [pa.field("values", pa.map_(pa.string(), pa.int64()), False)]
                ),
                False,
            )
        ]
    )
    nested_type = Record.from_arrow_schema(nested_schema, class_name="NestedMap")
    nested_batch = pa.record_batch(
        [
            pa.array(
                [{"values": [("key", 1), ("key", 2)]}],
                type=nested_schema.field("payload").type,
            )
        ],
        schema=nested_schema,
    )
    with pytest.raises(
        ValueError,
        match=r"NestedMap\[0\]\.payload\.values.*duplicate Arrow map key",
    ):
        next(nested_type.from_arrow_record_batch(nested_batch))


def test_arrow_outputs_are_bounded_lazy_and_reject_heterogeneous_rows() -> None:
    row_type = Record.from_arrow_schema(pa.schema([pa.field("value", pa.int64(), False)]))
    rows = [row_type(1), row_type(2), row_type(3)]
    batches = row_type.into_arrow_record_batches(iter(rows), batch_size=2)

    first = next(batches)
    assert first.column(0).to_pylist() == [1, 2]
    assert next(batches).column(0).to_pylist() == [3]
    with pytest.raises(StopIteration):
        next(batches)

    batches = row_type.into_arrow_record_batches(
        iter([row_type(1), row_type(2), object()]), batch_size=2
    )
    assert next(batches).num_rows == 2
    with pytest.raises(TypeError, match=r"ArrowRecord\[2\]"):
        next(batches)

    with pytest.raises(TypeError, match="exact ArrowRecord"):
        row_type.into_arrow_record_batch([object()])


def test_arrow_output_deep_lowering_temporal_decimal_uuid_and_path() -> None:
    @dataclasses.dataclass
    class Child:
        label: str

    # These annotations intentionally materialize their exact Arrow physical
    # layout first, then exercise both checked and shallow output lowering.
    schema = pa.schema(
        [
            pa.field("child", pa.struct([pa.field("label", pa.string(), False)]), False),
            pa.field("identifier", pa.string(), False),
            pa.field("path", pa.string(), False),
            pa.field("payload", pa.binary(), False),
            pa.field("amount", pa.decimal128(8, 2), False),
            pa.field("day", pa.date32(), False),
        ]
    )
    row_type = Record.from_arrow_schema(schema, class_name="RichArrowRow")
    value = row_type.from_dict(
        {
            "child": {"label": "x"},
            "identifier": str(uuid.UUID(int=1)),
            "path": str(Path("data/file")),
            "payload": bytearray(b"x"),
            "amount": Decimal("1.20"),
            "day": dt.date(2026, 8, 15),
        }
    )
    for safe in (True, False):
        batch = row_type.into_arrow_record_batch([value], safe=safe)
        assert tuple(row_type.from_arrow_record_batch(batch)) == (value,)


def test_zero_column_records_keep_row_counts_across_arrow() -> None:
    empty_type = Record.from_arrow_schema(pa.schema([]), class_name="Empty")
    empty = empty_type()

    assert empty_type.into_arrow_record_batch([]).num_rows == 0
    batch = empty_type.into_arrow_record_batch([empty, empty])
    assert batch.num_columns == 0
    assert batch.num_rows == 2
    assert tuple(empty_type.from_arrow_record_batch(batch)) == (empty, empty)


def test_batch_size_and_import_options_are_validated_before_sources_are_touched() -> None:
    row_type = Record.from_arrow_schema(pa.schema([pa.field("value", pa.int64())]))
    touched = False

    def source() -> Any:
        nonlocal touched
        touched = True
        yield row_type.into_arrow_record_batch([])

    with pytest.raises(TypeError, match="validate_schema"):
        row_type.from_arrow(source(), validate_schema=1)
    assert touched is False
    with pytest.raises(ValueError, match="positive"):
        row_type.into_arrow_record_batches([], batch_size=0)
    with pytest.raises(TypeError, match="batch_size"):
        row_type.into_arrow_record_batches(source(), batch_size=True)
    assert touched is False


def test_dynamic_schema_handles_nested_all_type_columns_and_dictionary_roundtrip() -> None:
    detail = pa.struct(
        [
            pa.field("score", pa.int16(), False),
            pa.field(
                "labels",
                pa.map_(pa.string(), pa.int8(), keys_sorted=True),
                False,
            ),
        ]
    )
    dictionary = pa.dictionary(pa.int8(), pa.string())
    schema = pa.schema(
        [
            pa.field("small", pa.int8(), False),
            pa.field("amount", pa.decimal256(40, 8), False),
            pa.field("created", pa.timestamp("ns", tz="Europe/Paris"), False),
            pa.field("day", pa.date32(), False),
            pa.field("at", pa.time64("us"), False),
            pa.field("elapsed", pa.duration("us"), False),
            pa.field("details", pa.list_(pa.field("item", detail, False)), False),
            pa.field(
                "lookup",
                pa.map_(pa.string(), detail, keys_sorted=True),
                False,
            ),
            pa.field("code", dictionary, False),
        ]
    )
    code = pa.array(["A"], type=pa.string()).dictionary_encode().cast(dictionary)
    batch = pa.RecordBatch.from_arrays(
        [
            pa.array([7], type=pa.int8()),
            pa.array([Decimal("12345678901234567890.12345678")], type=pa.decimal256(40, 8)),
            pa.array(
                [dt.datetime(2026, 8, 15, 12, tzinfo=dt.timezone(dt.timedelta(hours=2)))],
                type=pa.timestamp("ns", tz="Europe/Paris"),
            ),
            pa.array([dt.date(2026, 8, 15)], type=pa.date32()),
            pa.array([dt.time(12, 34, 56)], type=pa.time64("us")),
            pa.array([dt.timedelta(seconds=3)], type=pa.duration("us")),
            pa.array(
                [[{"score": 2, "labels": [("hot", 1)]}]],
                type=schema.field("details").type,
            ),
            pa.array(
                [[("left", {"score": 3, "labels": [("cold", 2)]})]],
                type=schema.field("lookup").type,
            ),
            code,
        ],
        schema=schema,
    )
    row_type = Record.from_arrow_schema(schema, class_name="AllTypes")

    value = next(row_type.from_arrow_record_batch(batch))
    assert value.small == 7
    assert value.amount == Decimal("12345678901234567890.12345678")
    assert dataclasses.is_dataclass(value.details[0])
    assert value.details[0].labels == {"hot": 1}
    assert value.lookup["left"].score == 3
    assert value.code == "A"

    projected = row_type.into_arrow_record_batch([value])
    assert projected.schema.equals(schema, check_metadata=False)
    assert projected.schema.field("code").type == dictionary
    assert projected.schema.field("lookup").type.keys_sorted is True
    assert tuple(row_type.from_arrow_record_batch(projected)) == (value,)


def test_nullable_dictionary_null_key_roundtrips_through_record() -> None:
    dictionary_type = pa.dictionary(pa.int8(), pa.string())
    schema = pa.schema(
        [pa.field("encoded", dictionary_type, nullable=True)]
    )
    dictionary = pa.DictionaryArray.from_arrays(
        pa.array([None], type=pa.int8()),
        pa.array(["not-null"], type=pa.string()),
    )
    batch = pa.RecordBatch.from_arrays([dictionary], schema=schema)
    row_type = Record.from_arrow_schema(schema, class_name="NullableDictionary")

    row = next(row_type.from_arrow_record_batch(batch))
    projected = row_type.into_arrow_record_batch([row])

    assert row.encoded is None
    assert projected.schema.equals(schema, check_metadata=False)
    assert projected.column("encoded").to_pylist() == [None]
    assert tuple(row_type.from_arrow_record_batch(projected)) == (row,)


def test_nested_arrow_names_are_validated_and_generated_identities_do_not_collide() -> None:
    invalid = pa.schema(
        [
            pa.field(
                "items",
                pa.list_(
                    pa.field(
                        "item",
                        pa.struct([pa.field("bad-name", pa.int64())]),
                    )
                ),
            )
        ]
    )
    with pytest.raises(TypeError, match="bad-name.*identifier"):
        Record.from_arrow_schema(invalid)

    reserved = pa.schema(
        [pa.field("payload", pa.struct([pa.field("from_dict", pa.int64())]))]
    )
    with pytest.raises(TypeError, match="from_dict.*reserved"):
        Record.from_arrow_schema(reserved)

    siblings = pa.schema(
        [
            pa.field("foo_bar", pa.struct([pa.field("value", pa.int8())]), False),
            pa.field("fooBar", pa.struct([pa.field("value", pa.int8())]), False),
        ]
    )
    sibling_type = Record.from_arrow_schema(siblings, class_name="Siblings")
    left = sibling_type.__annotations__["foo_bar"]
    right = sibling_type.__annotations__["fooBar"]
    assert left is not right
    assert left.__qualname__ != right.__qualname__

    with pytest.raises((TypeError, ValueError), match="duplicate.*(?:column|field)"):
        Record.from_arrow_schema(
            pa.schema([pa.field("value", pa.int8()), pa.field("value", pa.int16())])
        )


def test_validate_schema_false_casts_rows_but_extension_identity_is_semantic() -> None:
    expected = pa.schema([pa.field("value", pa.int8(), False)])
    row_type = Record.from_arrow_schema(expected, class_name="Narrow")
    wider = pa.record_batch(
        [pa.array([7], type=pa.int16())],
        schema=pa.schema([pa.field("value", pa.int16(), False)]),
    )
    assert next(
        row_type.from_arrow_record_batch(wider, validate_schema=False)
    ).value == 7

    extension_a = pa.schema(
        [
            pa.field(
                "value",
                pa.int64(),
                metadata={b"ARROW:extension:name": b"tests.a"},
            )
        ]
    )
    extension_type = Record.from_arrow_schema(extension_a, class_name="Extension")
    extension_b = pa.record_batch(
        [pa.array([1], type=pa.int64())],
        schema=pa.schema(
            [
                pa.field(
                    "value",
                    pa.int64(),
                    metadata={b"ARROW:extension:name": b"tests.b"},
                )
            ]
        ),
    )
    with pytest.raises(TypeError, match="incompatible Arrow schema"):
        extension_type.from_arrow_record_batch(extension_b)


def test_arrow_input_errors_use_global_row_indices_across_batches() -> None:
    class Side(enum.Enum):
        BUY = "buy"

    @record
    class SideRow(Record):
        side: Side

    schema = SideRow.into_arrow_schema()
    first = pa.record_batch([pa.array(["buy"])], schema=schema)
    second = pa.record_batch([pa.array(["bad"])], schema=schema)
    rows = SideRow.from_arrow(iter([first, second]))

    assert next(rows) == SideRow(Side.BUY)
    with pytest.raises(TypeError, match=r"SideRow\[1\]\.side"):
        next(rows)

    reader = pa.RecordBatchReader.from_batches(schema, [first, second])
    reader_rows = SideRow.from_arrow_record_batch_reader(reader)
    assert next(reader_rows) == SideRow(Side.BUY)
    with pytest.raises(TypeError, match=r"SideRow\[1\]\.side"):
        next(reader_rows)


def test_c_stream_options_fail_before_exporter_is_invoked() -> None:
    row_type = Record.from_arrow_schema(pa.schema([pa.field("value", pa.int64())]))

    class Stream:
        touched = False

        def __arrow_c_stream__(self, requested_schema: object = None) -> object:
            self.touched = True
            raise AssertionError("stream must not be requested")

    stream = Stream()
    with pytest.raises(TypeError, match="safe"):
        row_type.from_arrow(stream, safe=1)
    assert stream.touched is False


def test_zero_column_external_batch_and_output_iterators_remain_lazy() -> None:
    empty_type = Record.from_arrow_schema(pa.schema([]), class_name="EmptyExternal")
    external = pa.RecordBatch.from_struct_array(
        pa.array([{}, {}, {}], type=pa.struct([]))
    )
    assert len(tuple(empty_type.from_arrow_record_batch(external))) == 3

    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int64(), False)])
    )
    consumed: list[int] = []

    def rows() -> Any:
        for index in range(3):
            consumed.append(index)
            yield row_type(index)

    batches = row_type.into_arrow_record_batches(rows(), batch_size=1)
    assert consumed == []
    assert next(batches).column(0).to_pylist() == [0]
    assert consumed == [0]
    assert next(batches).column(0).to_pylist() == [1]
    assert consumed == [0, 1]


def test_safe_false_lowers_declared_python_shapes_without_casting() -> None:
    class Mode(enum.Enum):
        FAST = "fast"

    @dataclasses.dataclass
    class Child:
        count: int

    @record
    class NativeShapes(Record):
        child: Child
        mode: Mode
        identifier: uuid.UUID
        path: Path
        payload: bytes
        labels: set[str]

    identifier = uuid.UUID(int=7)
    value = NativeShapes(
        Child(3),
        Mode.FAST,
        identifier,
        Path("nested/file"),
        memoryview(b"payload"),  # type: ignore[arg-type]
        {"a", "b"},
    )
    batch = NativeShapes.into_arrow_record_batch([value], safe=False)
    raw = batch.to_pylist()[0]
    assert raw["child"] == {"count": 3}
    assert raw["mode"] == "fast"
    assert raw["identifier"] == str(identifier)
    assert raw["path"] == "nested/file"
    assert raw["payload"] == b"payload"
    assert set(raw["labels"]) == {"a", "b"}


def test_union_output_is_rejected_before_consuming_records() -> None:
    @record
    class Choice(Record):
        value: int | str

    consumed = False

    def rows() -> Any:
        nonlocal consumed
        consumed = True
        yield Choice(1)

    with pytest.raises(TypeError, match=r"Choice\.value.*union"):
        Choice.into_arrow_record_batch(rows())
    assert consumed is False


def test_output_requires_the_exact_receiving_record_class() -> None:
    schema = pa.schema([pa.field("value", pa.int64(), False)])
    first_type = Record.from_arrow_schema(schema, class_name="First")
    second_type = Record.from_arrow_schema(schema, class_name="Second")

    with pytest.raises(TypeError, match=r"First\[1\].*exact First"):
        first_type.into_arrow_record_batch([first_type(1), second_type(2)])


@pytest.mark.parametrize(
    ("target", "source", "value"),
    [
        (pa.int8(), pa.int64(), 1_000),
        (pa.uint8(), pa.int64(), -1),
        (pa.binary(2), pa.binary(), b"long"),
        (pa.decimal128(3, 0), pa.int64(), 12_345),
    ],
)
def test_validate_schema_false_safe_cast_enforces_physical_constraints(
    target: pa.DataType, source: pa.DataType, value: object
) -> None:
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", target, False)]), class_name="Constrained"
    )
    batch = pa.record_batch(
        [pa.array([value], type=source)],
        schema=pa.schema([pa.field("value", source, False)]),
    )

    with pytest.raises(TypeError, match=r"Constrained\[0\]\.value"):
        next(
            row_type.from_arrow_record_batch(
                batch, safe=True, validate_schema=False
            )
        )


def test_validate_schema_false_cast_is_lazy_and_respects_default_policy() -> None:
    @record
    class Defaulted(Record):
        value: int = 9

    source_schema = pa.schema([pa.field("value", pa.string(), False)])
    batch = pa.record_batch(
        [pa.array(["7", "bad"], type=pa.string())], schema=source_schema
    )

    raising = Defaulted.from_arrow_record_batch(
        batch, validate_schema=False, errors="raise"
    )
    assert next(raising) == Defaulted(7)
    with pytest.raises(TypeError, match=r"Defaulted\[1\]\.value"):
        next(raising)

    defaulting = Defaulted.from_arrow_record_batch(
        batch, validate_schema=False, errors="default"
    )
    assert tuple(defaulting) == (Defaulted(7), Defaulted(9))


def test_arrow_scalar_conversion_is_cached_once_per_primitive_cell(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl.records._arrow as adapter

    row_type = Record.from_arrow_schema(
        pa.schema(
            [pa.field("left", pa.int64(), False), pa.field("right", pa.string(), False)]
        ),
        class_name="CachedCells",
    )
    batch = pa.record_batch(
        [pa.array([1]), pa.array(["x"])], schema=row_type.into_arrow_schema()
    )
    original = adapter._arrow_scalar_value
    calls = 0

    def counted(scalar: object, plan: object, *, path: str) -> object:
        nonlocal calls
        calls += 1
        return original(scalar, plan, path=path)

    monkeypatch.setattr(adapter, "_arrow_scalar_value", counted)
    assert next(row_type.from_arrow_record_batch(batch)) == row_type(1, "x")
    assert calls == 2


def test_record_output_scalar_retry_uses_native_field_only_on_mismatch(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl.records._arrow as adapter

    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int8(), False)]),
        class_name="ScalarRetry",
    )
    original = adapter._field_arrow_scalar
    calls: list[tuple[object, bool]] = []
    scalar_views = 0

    def counted(field: object, value: object, *, safe: bool) -> object:
        calls.append((value, safe))
        return original(field, value, safe=safe)

    original_view = adapter._scalar_as_py_for_validation

    def counted_view(value: object) -> object:
        nonlocal scalar_views
        scalar_views += 1
        return original_view(value)

    monkeypatch.setattr(adapter, "_field_arrow_scalar", counted)
    monkeypatch.setattr(
        adapter, "_scalar_as_py_for_validation", counted_view
    )
    exact = pa.scalar(7, type=pa.int8())
    exact_batch = row_type.into_arrow_record_batch(
        [row_type(exact)], safe=False
    )
    assert exact_batch.column(0)[0].as_py() == 7
    assert calls == []
    assert scalar_views == 0

    mismatch = pa.scalar(8, type=pa.int64())
    mismatch_batch = row_type.into_arrow_record_batch(
        [row_type(mismatch)], safe=False
    )
    assert mismatch_batch.column(0)[0].as_py() == 8
    assert calls == [(mismatch, True)]
    assert scalar_views == 0

    calls.clear()
    mixed = row_type.into_arrow_record_batch(
        [row_type(exact), row_type(mismatch), row_type(exact)], safe=False
    )
    assert mixed.column(0).to_pylist() == [7, 8, 7]
    assert calls == [(mismatch, True)]
    assert scalar_views == 0


def test_record_shallow_output_rejects_every_nonnullable_null_shape() -> None:
    required = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int8(), False)]),
        class_name="RequiredScalar",
    )
    for value in (
        None,
        pa.scalar(None, type=pa.int8()),
        pa.scalar(None, type=pa.int64()),
    ):
        with pytest.raises(
            ValueError, match=r"RequiredScalar\[0\].*value.*not nullable"
        ):
            required.into_arrow_record_batch(
                [required(value)], safe=False
            )

    nullable = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int8(), True)]),
        class_name="NullableScalar",
    )
    batch = nullable.into_arrow_record_batch(
        [nullable(None), nullable(pa.scalar(None, type=pa.int8()))],
        safe=False,
    )
    assert batch.column(0).null_count == 2


@pytest.mark.parametrize(
    ("field", "value", "path"),
    (
        (
            pa.field(
                "payload",
                pa.struct([pa.field("child", pa.int8(), False)]),
                False,
            ),
            {"child": None},
            r"payload\.child",
        ),
        (
            pa.field(
                "payload",
                pa.list_(pa.field("item", pa.int8(), False)),
                False,
            ),
            [None],
            r"payload\[0\]",
        ),
        (
            pa.field(
                "payload",
                pa.large_list(pa.field("item", pa.int8(), False)),
                False,
            ),
            [None],
            r"payload\[0\]",
        ),
        (
            pa.field(
                "payload",
                pa.list_(pa.field("item", pa.int8(), False), 1),
                False,
            ),
            [None],
            r"payload\[0\]",
        ),
        (
            pa.field(
                "payload",
                pa.map_(
                    pa.string(),
                    pa.field("value", pa.int8(), nullable=False),
                ),
                False,
            ),
            [("key", None)],
            r"payload\['key'\]",
        ),
        (
            pa.field(
                "payload",
                pa.list_(
                    pa.field(
                        "item",
                        pa.struct([pa.field("leaf", pa.int8(), False)]),
                        False,
                    )
                ),
                False,
            ),
            [{"leaf": None}],
            r"payload\[0\]\.leaf",
        ),
    ),
)
def test_record_shallow_output_enforces_recursive_arrow_nullability(
    field: pa.Field, value: object, path: str
) -> None:
    row_type = Record.from_arrow_schema(
        pa.schema([field]), class_name="NestedRequired"
    )
    with pytest.raises(ValueError, match=path + ".*not nullable"):
        row_type.into_arrow_record_batch(
            [row_type(value)], safe=False
        )


def test_record_shallow_output_allows_nullable_nested_values() -> None:
    schema = pa.schema(
        [
            pa.field(
                "struct_value",
                pa.struct([pa.field("child", pa.int8(), True)]),
                False,
            ),
            pa.field(
                "list_value",
                pa.list_(pa.field("item", pa.int8(), True)),
                False,
            ),
            pa.field(
                "map_value",
                pa.map_(
                    pa.string(),
                    pa.field("value", pa.int8(), nullable=True),
                ),
                False,
            ),
        ]
    )
    row_type = Record.from_arrow_schema(schema, class_name="NestedNullable")
    batch = row_type.into_arrow_record_batch(
        [row_type({"child": None}, [None], [("key", None)])],
        safe=False,
    )
    assert batch.num_rows == 1


@pytest.mark.parametrize(
    ("data_type", "value"),
    (
        (
            pa.struct([pa.field("child", pa.int8(), False)]),
            {"child": None},
        ),
        (
            pa.list_(pa.field("item", pa.int8(), False)),
            [None],
        ),
        (
            pa.map_(
                pa.string(),
                pa.field("value", pa.int8(), nullable=False),
            ),
            [("key", None)],
        ),
    ),
)
def test_record_shallow_output_checks_nested_nulls_in_exact_scalars(
    data_type: pa.DataType, value: object
) -> None:
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("payload", data_type, False)]),
        class_name="NestedScalarRequired",
    )
    scalar = pa.scalar(value, type=data_type)
    with pytest.raises(ValueError, match="not nullable"):
        row_type.into_arrow_record_batch(
            [row_type(scalar)], safe=False
        )


def test_dictionary_union_output_rejects_before_consuming_rows() -> None:
    union_type = pa.union(
        [pa.field("number", pa.int8()), pa.field("text", pa.string())],
        mode="dense",
    )
    row_type = Record.from_arrow_schema(
        pa.schema(
            [
                pa.field(
                    "value",
                    pa.dictionary(pa.int8(), union_type),
                    nullable=False,
                )
            ]
        ),
        class_name="DictionaryUnion",
    )
    consumed = False

    def rows() -> object:
        nonlocal consumed
        consumed = True
        yield row_type(1)

    with pytest.raises(TypeError, match="union arrays"):
        row_type.into_arrow_record_batch(rows())
    assert consumed is False


def test_init_false_state_roundtrips_and_required_initvar_fails_preflight() -> None:
    @record(frozen=True)
    class StoredState(Record):
        value: int
        derived: int = dataclasses.field(init=False, default=5)

    stored = StoredState(1)
    object.__setattr__(stored, "derived", 9)
    batch = StoredState.into_arrow_record_batch([stored])
    assert tuple(StoredState.from_arrow_record_batch(batch)) == (stored,)

    @record
    class RequiredInit(Record):
        value: int
        seed: dataclasses.InitVar[int]

    required = RequiredInit(1, 2)
    batch = RequiredInit.into_arrow_record_batch([required])
    with pytest.raises(TypeError, match="required InitVar.*seed"):
        RequiredInit.from_arrow_record_batch(batch)

    @record
    class DefaultedInit(Record):
        value: int
        seed: dataclasses.InitVar[int] = 0

    defaulted = DefaultedInit(3)
    batch = DefaultedInit.into_arrow_record_batch([defaulted])
    assert tuple(DefaultedInit.from_arrow_record_batch(batch)) == (defaulted,)


def test_month_day_nano_and_run_end_encoded_map_roundtrip() -> None:
    interval_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.month_day_nano_interval(), False)]),
        class_name="IntervalRow",
    )
    interval_batch = pa.record_batch(
        [
            pa.array(
                [pa.MonthDayNano((1, 2, 3))],
                type=pa.month_day_nano_interval(),
            )
        ],
        schema=interval_type.into_arrow_schema(),
    )
    interval = next(interval_type.from_arrow_record_batch(interval_batch))
    for safe in (True, False):
        projected = interval_type.into_arrow_record_batch([interval], safe=safe)
        assert tuple(interval_type.from_arrow_record_batch(projected)) == (interval,)

    encoded_type = pa.run_end_encoded(
        pa.int16(), pa.map_(pa.string(), pa.int64())
    )
    encoded = pa.RunEndEncodedArray.from_arrays(
        pa.array([2, 3], type=pa.int16()),
        pa.array(
            [[("a", 1)], [("b", 2)]], type=pa.map_(pa.string(), pa.int64())
        ),
        type=encoded_type,
    )
    encoded_record = Record.from_arrow_schema(
        pa.schema([pa.field("value", encoded_type, False)]),
        class_name="EncodedRow",
    )
    source = pa.record_batch([encoded], schema=encoded_record.into_arrow_schema())
    rows = tuple(encoded_record.from_arrow_record_batch(source))
    projected = encoded_record.into_arrow_record_batch(rows)
    assert tuple(encoded_record.from_arrow_record_batch(projected)) == rows


def test_nested_run_end_encoded_map_roundtrips_inside_struct() -> None:
    encoded_type = pa.run_end_encoded(
        pa.int16(), pa.map_(pa.string(), pa.int64())
    )
    nested_type = pa.struct([pa.field("encoded", encoded_type, False)])
    encoded = pa.RunEndEncodedArray.from_arrays(
        pa.array([2, 3], type=pa.int16()),
        pa.array(
            [[("a", 1)], [("b", 2)]], type=pa.map_(pa.string(), pa.int64())
        ),
        type=encoded_type,
    )
    nested = pa.StructArray.from_arrays([encoded], type=nested_type)
    schema = pa.schema([pa.field("payload", nested_type, False)])
    row_type = Record.from_arrow_schema(schema, class_name="NestedEncoded")
    source = pa.record_batch([nested], schema=schema)

    rows = tuple(row_type.from_arrow_record_batch(source))
    for safe in (True, False):
        projected = row_type.into_arrow_record_batch(rows, safe=safe)
        assert projected.schema.equals(schema, check_metadata=False)
        assert tuple(row_type.from_arrow_record_batch(projected)) == rows


def test_nested_map_keys_use_association_lists_and_reject_duplicates() -> None:
    key_type = pa.struct([pa.field("part", pa.int64(), False)])
    map_type = pa.map_(key_type, pa.string())
    schema = pa.schema([pa.field("values", map_type, False)])
    row_type = Record.from_arrow_schema(schema, class_name="NestedKeys")
    source = pa.record_batch(
        [
            pa.array(
                [[({"part": 1}, "one"), ({"part": 2}, "two")]],
                type=map_type,
            )
        ],
        schema=schema,
    )

    row = next(row_type.from_arrow_record_batch(source))
    assert isinstance(row.values, list)
    assert [key.part for key, _ in row.values] == [1, 2]
    projected = row_type.into_arrow_record_batch([row])
    assert tuple(row_type.from_arrow_record_batch(projected)) == (row,)

    duplicate = pa.record_batch(
        [
            pa.array(
                [[({"part": 1}, "one"), ({"part": 1}, "again")]],
                type=map_type,
            )
        ],
        schema=schema,
    )
    with pytest.raises(ValueError, match="duplicate Arrow map key"):
        next(row_type.from_arrow_record_batch(duplicate))


def test_safe_false_map_association_pairs_preserve_tuples_and_uniqueness() -> None:
    map_type = pa.map_(pa.string(), pa.int64())
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("values", map_type, False)]), class_name="PairMap"
    )

    row = row_type([("a", 1)])
    batch = row_type.into_arrow_record_batch([row], safe=False)
    assert batch.column(0)[0].as_py() == [("a", 1)]

    with pytest.raises(ValueError, match="duplicate Arrow map key"):
        row_type.into_arrow_record_batch(
            [row_type([("a", 1), ("a", 2)])], safe=False
        )


def test_extension_registry_and_non_utf8_transport_metadata_are_compatible() -> None:
    class AuditExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.records.audit-extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> AuditExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = AuditExtension()
    pa.register_extension_type(extension)
    try:
        extension_array = pa.ExtensionArray.from_storage(
            extension, pa.array([3], type=pa.int32())
        )
        source_schema = pa.schema([pa.field("value", extension, False)])
        row_type = Record.from_arrow_schema(
            source_schema, class_name="ExtensionRegistry"
        )
        source = pa.record_batch([extension_array], schema=source_schema)
    finally:
        pa.unregister_extension_type(extension.extension_name)

    assert next(row_type.from_arrow_record_batch(source)).value == 3

    ordinary = Record.from_arrow_schema(
        pa.schema([pa.field("value", pa.int64(), False)]),
        class_name="TransportMetadata",
    )
    non_utf8 = pa.record_batch(
        [pa.array([4], type=pa.int64())],
        schema=pa.schema(
            [pa.field("value", pa.int64(), False, metadata={b"transport": b"\xff"})],
            metadata={b"transport": b"\xff"},
        ),
    )
    assert next(ordinary.from_arrow_record_batch(non_utf8)).value == 4


def test_record_output_rejects_cross_extension_scalar_relabeling(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl.records._arrow as adapter

    class FirstExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.records.scalar-first")

        def __arrow_ext_serialize__(self) -> bytes:
            return b""

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> FirstExtension:
            return cls()

    class SecondExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.records.scalar-second")

        def __arrow_ext_serialize__(self) -> bytes:
            return b""

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> SecondExtension:
            return cls()

    first = FirstExtension()
    second = SecondExtension()
    pa.register_extension_type(first)
    pa.register_extension_type(second)
    try:
        row_type = Record.from_arrow_schema(
            pa.schema([pa.field("value", first, False)]),
            class_name="ExtensionScalarOutput",
        )
        original = adapter._field_arrow_scalar
        calls: list[object] = []

        def counted(field: object, value: object, *, safe: bool) -> object:
            calls.append(value)
            return original(field, value, safe=safe)

        monkeypatch.setattr(adapter, "_field_arrow_scalar", counted)
        exact = pa.scalar(3, type=first)
        batch = row_type.into_arrow_record_batch(
            [row_type(exact)], safe=False
        )
        assert batch.column(0)[0].type.equals(first)
        assert batch.column(0)[0].as_py() == 3
        assert calls == []

        storage = pa.scalar(4, type=pa.int32())
        storage_batch = row_type.into_arrow_record_batch(
            [row_type(storage)], safe=False
        )
        assert storage_batch.column(0)[0].type.equals(first)
        assert storage_batch.column(0)[0].as_py() == 4
        assert calls == [storage]

        calls.clear()
        mismatch = pa.scalar(5, type=second)
        with pytest.raises(
            TypeError,
            match=r"ExtensionScalarOutput\[0\].*value.*safely cast",
        ):
            row_type.into_arrow_record_batch(
                [row_type(mismatch)], safe=False
            )
        assert calls == [mismatch]
    finally:
        pa.unregister_extension_type(second.extension_name)
        pa.unregister_extension_type(first.extension_name)


def test_extension_storage_with_nested_run_end_encoding_roundtrips() -> None:
    encoded_type = pa.run_end_encoded(pa.int16(), pa.int64())
    storage_type = pa.struct([pa.field("encoded", encoded_type, False)])

    class EncodedExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(storage_type, "tests.records.encoded-extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b""

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage: pa.DataType, serialized: bytes
        ) -> EncodedExtension:
            assert storage.equals(storage_type)
            assert serialized == b""
            return cls()

    extension = EncodedExtension()
    pa.register_extension_type(extension)
    try:
        encoded = pa.RunEndEncodedArray.from_arrays(
            pa.array([2], type=pa.int16()),
            pa.array([5], type=pa.int64()),
            type=encoded_type,
        )
        storage = pa.StructArray.from_arrays([encoded], type=storage_type)
        values = pa.ExtensionArray.from_storage(extension, storage)
        schema = pa.schema([pa.field("value", extension, False)])
        row_type = Record.from_arrow_schema(schema, class_name="ExtensionEncoded")
        source = pa.record_batch([values], schema=schema)
        rows = tuple(row_type.from_arrow_record_batch(source))

        for safe in (True, False):
            projected = row_type.into_arrow_record_batch(rows, safe=safe)
            assert tuple(row_type.from_arrow_record_batch(projected)) == rows
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_run_end_encoded_output_respects_narrow_index_capacity() -> None:
    encoded_type = pa.run_end_encoded(pa.int16(), pa.int64())
    row_type = Record.from_arrow_schema(
        pa.schema([pa.field("value", encoded_type, False)]),
        class_name="NarrowRunEnds",
    )
    rows = [row_type(index) for index in range(32_768)]

    batches = tuple(row_type.into_arrow_record_batches(rows))
    assert [batch.num_rows for batch in batches] == [32_767, 1]
    assert row_type.into_arrow_table(rows).column(0).num_chunks == 2
    with pytest.raises(ValueError, match="at most 32767 rows"):
        row_type.into_arrow_record_batch(rows)

    nested_type = pa.list_(pa.field("item", encoded_type, False))
    nested_record = Record.from_arrow_schema(
        pa.schema([pa.field("values", nested_type, False)]),
        class_name="NestedRunEndsCapacity",
    )
    with pytest.raises(TypeError, match=r"NestedRunEndsCapacity\[0\]\.values"):
        nested_record.into_arrow_record_batch(
            [nested_record(list(range(32_768)))], safe=False
        )
