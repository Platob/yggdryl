from __future__ import annotations

import copy
import decimal
import inspect
import pickle
from typing import Optional

import pyarrow as pa
import pytest

from yggdryl import DataType, Field


def test_data_type_infers_native_string_and_arrow_values() -> None:
    expected = DataType("int64")

    assert DataType(expected) == expected
    assert DataType.from_value("int64") == expected
    assert DataType.from_value(pa.int64()) == expected
    assert DataType.from_arrow(pa.int64()) == expected
    assert expected.to_arrow() == pa.int64()
    assert expected.into_arrow() == pa.int64()


def test_data_type_builds_and_casts_exact_arrow_scalars() -> None:
    data_type = DataType("int8")
    exact = pa.scalar(7, type=pa.int8())

    assert data_type.arrow_scalar(exact) is exact
    assert data_type.arrow_scalar(7).equals(exact)
    assert data_type.arrow_scalar(None).equals(pa.scalar(None, type=pa.int8()))
    assert data_type.arrow_scalar(pa.scalar(7, type=pa.int64())).equals(exact)

    with pytest.raises((pa.ArrowInvalid, OverflowError)):
        data_type.arrow_scalar(130)
    assert data_type.arrow_scalar(130, safe=False).as_py() == -126
    assert data_type.arrow_scalar("7", safe=False).equals(exact)

    with pytest.raises(TypeError):
        data_type.arrow_scalar(7, False)  # type: ignore[misc]


def test_data_type_arrow_scalar_handles_nested_dictionary_map_and_run_end() -> None:
    nested = DataType.from_arrow(
        pa.struct([pa.field("items", pa.list_(pa.int8()), nullable=False)])
    )
    nested_scalar = nested.arrow_scalar({"items": [1, 2]}, safe=False)
    assert nested_scalar.type.equals(nested.to_arrow())
    assert nested_scalar.as_py() == {"items": [1, 2]}

    mapping = DataType.from_arrow(pa.map_(pa.string(), pa.int8()))
    map_scalar = mapping.arrow_scalar([("left", 1), ("right", 2)], safe=False)
    assert map_scalar.type.equals(mapping.to_arrow())
    assert map_scalar.as_py() == [("left", 1), ("right", 2)]

    dictionary = DataType.from_arrow(pa.dictionary(pa.int8(), pa.string()))
    dictionary_scalar = dictionary.arrow_scalar("ready", safe=False)
    assert dictionary_scalar.type.equals(dictionary.to_arrow())
    assert dictionary_scalar.as_py() == "ready"

    run_end = DataType.from_arrow(pa.run_end_encoded(pa.int16(), pa.int64()))
    run_end_scalar = run_end.arrow_scalar(42)
    assert run_end_scalar.type.equals(run_end.to_arrow())
    assert run_end_scalar.as_py() == 42


def test_data_type_infers_python_types_without_stringifying_objects() -> None:
    assert DataType(str) == DataType("utf8")
    assert DataType(bool) == DataType("bool")
    assert DataType(int) == DataType("int64")
    assert DataType(float) == DataType("float64")
    assert DataType(bytes) == DataType("binary")
    assert DataType(decimal.Decimal) == DataType("decimal128(38,18)")
    assert DataType(list[str]) == DataType.from_pyhint(list[str])
    assert DataType(Optional[int]) == DataType("int64")

    with pytest.raises(TypeError, match="unsupported Python type hint"):
        DataType(object())


def test_decimal_infers_storage_width_and_integer_like_arguments() -> None:
    class IndexValue:
        def __init__(self, value: int) -> None:
            self.value = value

        def __index__(self) -> int:
            return self.value

    assert DataType.decimal(18) == DataType("decimal128(18,0)")
    assert str(inspect.signature(DataType.decimal)) == "(precision, scale=0)"
    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal("39", "-4") == DataType("decimal256(39,-4)")
    assert DataType.decimal(IndexValue(39), IndexValue(-4)) == DataType(
        "decimal256(39,-4)"
    )
    assert str(DataType.decimal("18", 2)) == "decimal128(18,2)"

    with pytest.raises(TypeError, match="precision.*not bool"):
        DataType.decimal(True, 0)
    with pytest.raises(TypeError, match="scale.*float"):
        DataType.decimal(18, 2.0)
    with pytest.raises(TypeError, match="scale.*NoneType"):
        DataType.decimal(18, None)  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="precision.*base-10 integer string"):
        DataType.decimal("18.0", 2)
    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, "3")
    with pytest.raises(OverflowError, match="precision.*unsigned byte"):
        DataType.decimal(256, 0)
    with pytest.raises(OverflowError, match="precision.*supported integer range"):
        DataType.decimal("9" * 100, 0)
    with pytest.raises(OverflowError, match="scale.*supported integer range"):
        DataType.decimal(18, "-" + "9" * 100)


def test_time_infers_storage_width_from_native_unit_aliases() -> None:
    assert str(inspect.signature(DataType.time)) == "(unit)"
    assert DataType.time("s") == DataType("time32(s)")
    assert DataType.time("milli seconds") == DataType("time32(ms)")
    assert DataType.time("µs") == DataType("time64(us)")
    assert DataType.time("NANO-SECONDS") == DataType("time64(ns)")

    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    with pytest.raises(ValueError, match="unknown temporal resolution"):
        DataType.time("fortnight")
    with pytest.raises(TypeError):
        DataType.time(1)  # type: ignore[arg-type]


def test_data_type_string_json_order_hash_and_pickle_protocols() -> None:
    value = DataType.from_arrow(pa.decimal128(18, 4))

    assert DataType.from_str(str(value)) == value
    assert DataType.from_json(value.to_json()) == value
    assert DataType.from_json(value.into_json()) == value
    assert eval(repr(value), {"DataType": DataType}) == value
    assert copy.copy(value) == value
    assert pickle.loads(pickle.dumps(value)) == value
    assert hash(value) == hash(DataType(value))
    assert DataType("int32") < DataType("int64") or DataType("int64") < DataType("int32")


def test_data_type_is_a_read_only_nested_field_collection() -> None:
    arrow_type = pa.struct(
        [
            pa.field("symbol", pa.string(), nullable=False),
            pa.field("levels", pa.list_(pa.float64())),
        ]
    )
    value = DataType.from_value(arrow_type)

    assert len(value) == 2
    assert [field.name for field in value] == ["symbol", "levels"]
    assert value[0].name == "symbol"
    assert value[-1].name == "levels"
    assert value["levels"].data_type.to_arrow() == pa.list_(pa.float64())
    assert 0 in value
    assert "symbol" in value
    assert value["symbol"] in value
    with pytest.raises(IndexError):
        _ = value[2]
    with pytest.raises(KeyError):
        _ = value["missing"]


def test_data_type_from_fields_builds_exact_native_struct() -> None:
    fields = (
        Field("small", DataType("uint8"), nullable=False, metadata={"unit": "items"}),
        Field("wide", DataType.decimal(39, 4), nullable=True),
    )

    consumed: list[str] = []

    def one_shot() -> object:
        for field in fields:
            consumed.append(field.name)
            yield field

    value = DataType.from_fields(one_shot())

    assert value.kind == "struct"
    assert consumed == ["small", "wide"]
    assert tuple(value) == fields
    arrow = value.to_arrow()
    assert arrow.equals(pa.struct([field.to_arrow() for field in fields]))
    assert arrow.field(0).equals(
        pa.field(
            "small",
            pa.uint8(),
            nullable=False,
            metadata={b"unit": b"items"},
        ),
        check_metadata=True,
    )

    empty = DataType.from_fields(iter(()))
    assert empty.id == "struct"
    assert len(empty) == 0
    assert empty.to_arrow() == pa.struct([])

    with pytest.raises(TypeError, match="field at index 1"):
        DataType.from_fields([fields[0], object()])
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.from_fields([fields[0], fields[0]])


def test_data_type_variant_assigns_dense_type_ids_in_member_order() -> None:
    members = (
        Field(
            "count",
            "int64",
            nullable=False,
            metadata={"source": "integer"},
        ),
        Field("label", "utf8", nullable=False),
        Field("missing", "null", nullable=True),
    )
    consumed: list[str] = []

    def one_shot() -> object:
        for member in members:
            consumed.append(member.name)
            yield member

    variant = DataType.variant(one_shot())
    arrow = variant.to_arrow()

    assert str(inspect.signature(DataType.variant)) == "(fields)"
    assert consumed == ["count", "label", "missing"]
    assert variant.id == "union"
    assert tuple(variant) == members
    assert arrow.mode == "dense"
    assert tuple(arrow.type_codes) == (0, 1, 2)
    assert arrow.field(0).metadata == {b"source": b"integer"}
    assert variant == DataType._union(enumerate(members), "dense")

    with pytest.raises(TypeError, match="field at index 1"):
        DataType.variant([members[0], object()])
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.variant([members[0], members[0]])
    with pytest.raises(ValueError):
        DataType.variant(
            Field(f"member_{index}", "null") for index in range(129)
        )


def test_data_type_arrow_roundtrip_preserves_nested_map_and_dictionary_flags() -> None:
    sorted_map = pa.map_(pa.string(), pa.int64(), keys_sorted=True)
    arrow_type = pa.struct(
        [
            pa.field(
                "items",
                pa.list_(
                    pa.field(
                        "entry",
                        pa.struct([pa.field("lookup", sorted_map, False)]),
                        False,
                    )
                ),
                False,
            ),
            pa.field(
                "codes",
                pa.dictionary(pa.int16(), pa.string(), ordered=True),
                False,
            ),
        ]
    )

    projected = DataType.from_arrow(arrow_type).to_arrow()
    nested_map = projected.field("items").type.value_type.field("lookup").type
    assert nested_map.keys_sorted is True
    assert projected.field("codes").type.ordered is True
