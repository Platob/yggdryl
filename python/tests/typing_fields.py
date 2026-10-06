"""Static-typing smoke cases checked separately with mypy."""

from __future__ import annotations

from dataclasses import field as dataclass_field
from decimal import Decimal
from typing import Annotated, cast

import pyarrow as pa  # type: ignore[import-untyped]

import yggdryl
from yggdryl import (
    CcyField,
    DataType,
    Field,
    ForexField,
    MarketDataKind,
    MarketDataKindField,
    MarketDataType,
    MarketDataTypeField,
    MediaTypeField,
    MimeTypeField,
    PluginSide,
    PluginSideField,
    ProtocolField,
    PythonMetadata,
    RicField,
    Scalar,
    Side,
    SideField,
    StructField,
    TimeInForce,
    TimeInForceField,
    TimezoneField,
    UrlField,
    UrnField,
    Version,
    VersionField,
    enums,
    field,
    json,
    scalar,
    toml,
    yaml,
)


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
class_root: StructField = TypedOrder.into_field()
same_root: Field = field(TypedOrder)
native_root: Field = field(order)
renamed_root: Field = field(TypedOrder, name="order")
datatype: DataType = DataType.from_pyhint(list[int])
variant_datatype: DataType = DataType.variant(
    [Field("integer", "int64", nullable=False), Field("text", "utf8", nullable=False)]
)
optional: Field = Field.from_pyhint("note", str | None)
native_scalar: Scalar = Scalar.from_(order)
python_value: object = native_scalar.as_py()
arrow_scalar: pa.Scalar = DataType("float32").scalar(1.5).into_arrow_scalar()

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
ccy: CcyField = yggdryl.ccy("currency", nullable=False)
ccy_default_scalar: Scalar = ccy.default_scalar()
instrument: RicField = yggdryl.ric("instrument")
pair: ForexField = yggdryl.forex("pair")
side: SideField = yggdryl.side("side", nullable=False)
side_member: Side = Side.BUYS
category: MarketDataKindField = yggdryl.marketdatakind("marketdatakind")
category_member: MarketDataKind | None = MarketDataKind.from_spelling("ORDR")
typed: MarketDataTypeField = yggdryl.marketdatatype("marketdatatype")
typed_member: MarketDataType | None = MarketDataType.from_fix(40, "2")
typed_tags: tuple[int, ...] = MarketDataType.fix_tags_of("AE", MarketDataKind.TRAD)
standing: TimeInForceField = yggdryl.timeinforce("timeinforce")
standing_member: TimeInForce = TimeInForce.from_fix("1")
role: PluginSideField = yggdryl.pluginside("pluginside")
role_member: PluginSide = PluginSide.from_plugin_type("x.SellSideFIXCPluginCBlock")
version: VersionField = yggdryl.version("version", nullable=False)
version_default_scalar: Scalar = version.default_scalar()
location: UrlField = yggdryl.url("url")
location_dtype: DataType = location.dtype
name: UrnField = yggdryl.urn("urn")
name_dtype: DataType = name.dtype
zone: TimezoneField = yggdryl.timezone("zone")
mime: MimeTypeField = yggdryl.mimetype("mime")
media: MediaTypeField = yggdryl.mediatype("media")
canonical_text_dtypes: tuple[DataType, ...] = (zone.dtype, mime.dtype, media.dtype)
python_view: ProtocolField = root.python
declared: PythonMetadata | None = python_view.class_metadata
declared_module: str = PythonMetadata(__name__, "TypedOrder", "field").module
declared_properties: dict[str, str] = PythonMetadata(
    __name__, "TypedOrder", "field"
).properties
declared_kinds: tuple[str, ...] = enums.PYTHON_KINDS
declared_class_name: str | None = python_view.class_name
declared_import_path: str | None = python_view.import_path


assert payload["order_id"] == 42
assert root is class_root is same_root is native_root
assert datatype.is_nested
assert optional.nullable
assert from_yaml == from_toml == from_json == order
assert dynamic_class.into_field() is imported  # type: ignore[attr-defined]
assert ccy_default_scalar.as_py() == ""
assert version_default_scalar.as_py() == Version(0)
assert location_dtype == DataType("url")
assert name_dtype == DataType("urn")
assert pair.dtype == DataType("forex") and side.dtype == DataType("side")
assert category.dtype == DataType("marketdatakind")
assert side_member.is_bid() and category_member is MarketDataKind.ORDR
assert typed.dtype == DataType("marketdatatype")
assert typed_member is MarketDataType.ORDLIMIT
assert MarketDataType.ORDLIMIT.fix_code == (40, "2")
assert typed_tags == (856, 828, 40)
assert standing.dtype == DataType("timeinforce")
assert standing_member is TimeInForce.GTC
assert role.dtype == DataType("pluginside")
assert role_member is PluginSide.SELL
assert canonical_text_dtypes == (
    DataType("timezone"),
    DataType("mimetype"),
    DataType("mediatype"),
)
