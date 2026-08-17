"""Static-typing smoke cases; checked separately with mypy/pyright."""

from __future__ import annotations

from collections.abc import Iterator
from decimal import Decimal
from typing import Annotated

import pyarrow as pa  # type: ignore[import-untyped]

from yggdryl import DataType, Field, Record, record
from yggdryl.records import Format, field, from_dict, schema_field, to_dict


@record(frozen=True, slots=True)
class TypedOrder(Record):
    order_id: int
    price: Annotated[Decimal, ("arrow_type", pa.decimal128(9, 2))] = Decimal(
        "0.00"
    )
    tags: list[str] = field(default_factory=list)
    note: str | None = None


order: TypedOrder = TypedOrder.from_dict({"order_id": "42"})
same: TypedOrder = from_dict(TypedOrder, order.to_dict())
payload: dict[str, object] = to_dict(same)
root: Field = TypedOrder.schema_field()
same_root: Field = schema_field(TypedOrder)
datatype: DataType = DataType.from_pyhint(list[int])
variant_datatype: DataType = DataType.variant(
    [Field("integer", "int64", nullable=False), Field("text", "utf8", nullable=False)]
)
inferred_text: DataType = DataType(str)
decimal_type: DataType = DataType.decimal("18", 4)
time_type: DataType = DataType.time("microseconds")
optional: Field = Field.from_pyhint("note", str | None)
codec_format: Format = "yml"
yaml_payload = order.into_(format=codec_format)
assert isinstance(yaml_payload, bytes)
from_yaml_alias: TypedOrder = TypedOrder.from_(
    yaml_payload, format=codec_format
)
toml_payload: bytes = order.into_toml()
from_toml: TypedOrder = TypedOrder.from_toml(toml_payload)
toml_format: Format = "application/toml"
toml_alias_payload = order.into_(format=toml_format)
assert isinstance(toml_alias_payload, bytes)
from_toml_alias: TypedOrder = TypedOrder.from_(
    toml_alias_payload, format=toml_format
)
toml_extension: Format = ".toml"
from_toml_extension: TypedOrder = TypedOrder.from_(
    toml_alias_payload, format=toml_extension
)
dict_rows: Iterator[TypedOrder] = TypedOrder.from_dicts(
    [{"order_id": "43"}]
)
arrow_field: object = TypedOrder.into_arrow_field()
arrow_schema: object = TypedOrder.into_arrow_schema()
arrow_batch: object = TypedOrder.into_arrow_record_batch([order])
arrow_batches: Iterator[object] = TypedOrder.into_arrow_record_batches(
    [order], batch_size=1
)
arrow_table: object = TypedOrder.into_arrow_table([order])
arrow_reader: object = TypedOrder.into_arrow_record_batch_reader(
    [order], batch_size=1
)
arrow_rows: Iterator[TypedOrder] = TypedOrder.from_arrow(arrow_batch)
dynamic_record: type[Record] = Record.from_arrow_schema(
    TypedOrder.into_arrow_schema(), class_name="DynamicTypedOrder"
)


assert payload["order_id"] == 42
assert root == same_root
assert datatype.is_nested
assert optional.nullable
assert from_yaml_alias == order
assert from_toml == order
assert from_toml_alias == order
assert from_toml_extension == order
assert next(dict_rows).order_id == 43
assert arrow_field is TypedOrder.into_arrow_field()
assert arrow_schema is TypedOrder.into_arrow_schema()
assert next(arrow_rows) == order
assert next(arrow_batches).num_rows == 1  # type: ignore[attr-defined]
assert isinstance(next(dynamic_record.from_arrow(arrow_table)), Record)
assert arrow_reader is not None
