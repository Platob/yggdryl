"""Static-typing smoke cases checked separately with mypy."""

from __future__ import annotations

from dataclasses import field as dataclass_field
from decimal import Decimal
from typing import Annotated, cast

import pyarrow as pa  # type: ignore[import-untyped]

from yggdryl import (
    DataType,
    Field,
    Scalar,
    field,
    fields,
    json,
    scalar,
    toml,
    yaml,
)
from yggdryl.fields import Ascii24Field, StructField


@scalar(frozen=True, slots=True)
class TypedOrder:
    order_id: int
    price: Annotated[Decimal, ("arrow_type", pa.decimal128(9, 2))] = Decimal(
        "0.00"
    )
    tags: list[str] = dataclass_field(default_factory=list)
    note: str | None = None


order: TypedOrder = json.loads('{"order_id":"42"}', cls=TypedOrder)
same: TypedOrder = json.loads(json.dumps(order), cls=TypedOrder)
payload = cast(dict[str, object], json.loads(json.dumps(same)))
root: Field = field(TypedOrder)
class_root: StructField = TypedOrder.field()
same_root: Field = field(TypedOrder)
native_root: Field = field(order)
renamed_root: Field = field(TypedOrder, name="order")
datatype: DataType = DataType.from_pyhint(list[int])
variant_datatype: DataType = DataType.variant(
    [Field("integer", "int64", nullable=False), Field("text", "utf8", nullable=False)]
)
optional: Field = Field.from_pyhint("note", str | None)
native_scalar: Scalar = Scalar.from_py(order)
python_value: object = native_scalar.as_py()
arrow_value: pa.Scalar = Scalar.float(1.5, 32).into_arrow_scalar()

yaml_payload: bytes = yaml.dumps(order)
from_yaml: TypedOrder = yaml.loads(yaml_payload, cls=TypedOrder)
toml_payload: bytes = toml.dumps(order)
from_toml: TypedOrder = toml.loads(toml_payload, cls=TypedOrder)
json_payload: bytes = json.dumps(order)
from_json: TypedOrder = json.loads(json_payload, cls=TypedOrder)

arrow_schema: pa.Schema = root.into_arrow_schema()
imported: Field = Field.from_arrow_schema(arrow_schema, name=root.name)
dynamic_class: type[object] = imported.into_dataclass(
    name="DynamicTypedOrder"
)
currency: Ascii24Field = fields.ascii24("currency", nullable=False)
currency_code: str | None = currency.default_pyvalue()


assert payload["order_id"] == 42
assert root is class_root is same_root is native_root
assert datatype.is_nested
assert optional.nullable
assert from_yaml == from_toml == from_json == order
assert dynamic_class.field() is imported  # type: ignore[attr-defined]
assert currency_code == ""
