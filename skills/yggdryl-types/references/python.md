# yggdryl-types in Python

`from yggdryl import DataType, Field, Scalar` for the three values; every
typed field factory (`int64`, `utf8`, `decimal`, `datetime64`, `serie`,
`struct`, `map_of`, `ccy`, ...) and the `@scalar` decorator and `field()`
converter are importable from `yggdryl` itself. Every factory returns the one
native `Field`, nullable unless `nullable=False`.

## Parse a datatype once and read it back

`DataType(value)` takes a type expression, a Python annotation (`str`,
`list[int]`) or a `pyarrow.DataType`; parse it once and reuse the value.

```python
import pyarrow as pa

from yggdryl import DataType

amount = DataType("numeric(18, 4)")
assert str(amount) == "decimal64(18,4)"          # one canonical spelling
assert amount.id == "decimal64" and amount.kind == "decimal"
assert DataType(str(amount)) == amount           # the text round-trips
assert repr(amount) == 'DataType.from_str("decimal64(18,4)")'

# Every dialect's spelling is the same value; compare values, not text.
assert DataType("bigint") == DataType("int64") == DataType(pa.int64())
assert DataType(str) == DataType("utf8")
assert DataType(list[int]).id == "serie"
assert DataType("list<int64>") == DataType("array<int64>")
assert str(DataType("timestamp")) == "datetime64(us)"

# Refusals name the byte where parsing stopped.
try:
    DataType("large_utf8(64)")
except ValueError as error:
    assert "at byte" in str(error)
else:
    raise AssertionError("a large leaf holds no maximum")
```

## Put the width, unit, scale and zone on the type

The selectors pick a width once: `DataType.decimal(p, s)` the narrowest
backing integer, `DataType.time(unit)` `time32` or `time64`. Strings and bytes
name their leaf; a number on a plain leaf is a maximum.

```python
from yggdryl import DataType, StringParameters

assert DataType.decimal(9, 2) == DataType("decimal32(9,2)")
assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
assert DataType.time("ms") == DataType("time32(ms)")
assert DataType.time("ns") == DataType("time64(ns)")

utc = DataType('datetime64(ns,"UTC")')
assert str(utc.timezone) == "UTC"
assert str(DataType("datetime64(us)").timezone) == "NAIVE"

# The leaf is the whole string declaration: charset, shape, number.
latin = DataType.string(charset="windows-1252", bound=32)
assert str(latin) == "sized_cp1252(32)"
assert latin.string_parameters == StringParameters("string", "windows-1252", 32)
assert DataType("varchar(32)") == DataType("sized_utf8(32)")
assert DataType.fixed_ascii(4).fixed_byte_width == 4
assert str(DataType.bytes(bound=16)) == "sized_binary(16)"
assert DataType.fixed_size_binary(16).fixed_byte_width == 16

# The bare word `decimal` is the fixed leaf, not a precision.
assert DataType("decimal").id == "decimal"
assert DataType("decimal(38,18)") != DataType("decimal")
```

## Build a field and a schema

A schema is a non-null struct `Field`; `validate_struct_root` checks it.
Subscripting reaches a child by name, position or dotted path.

```python
import pytest

import yggdryl
from yggdryl import DataType, Field

trade = yggdryl.struct(
    "trade",
    [
        yggdryl.int64("id", nullable=False),
        Field("price", "decimal(18, 4)", nullable=False),
        yggdryl.struct("venue", [yggdryl.mic("mic")]),
    ],
    nullable=False,
)
trade.validate_struct_root()

assert trade["id"].dtype == DataType("int64")
assert trade[1].name == "price"
assert trade["venue"]["mic"].dtype == DataType("mic")
assert trade.get_field_by_path("venue.mic").name == "mic"
assert trade.index_of("price") == 1
assert [child.name for child in trade] == ["id", "price", "venue"]
assert trade.get_field_by_path("missing") is None

# The same field from text, and a nullable root is not a schema.
assert Field.from_str("id int64 NOT NULL") == trade["id"]
with pytest.raises(ValueError):
    Field("trade", trade.dtype).validate_struct_root()
```

## Declare a schema from a dataclass

`@scalar` is a `dataclasses.dataclass` that also compiles its annotations into
one cached non-null struct `Field`, returned by `Class.into_field()`;
`field(value)` converts a class, an instance, a native field or a pyarrow
schema. `Annotated` options pin an exact Arrow layout or field property.

```python
import datetime as dt
from decimal import Decimal
from typing import Annotated, Optional

import pyarrow as pa

from yggdryl import DataType, Field, field, scalar


@scalar(frozen=True)
class Leg:
    symbol: str
    qty: int


@scalar(frozen=True)
class Order:
    order_id: int
    at: dt.datetime
    price: Annotated[Decimal, ("arrow_type", pa.decimal128(18, 4)), {"id": 7}]
    legs: list[Leg]
    note: Optional[str] = None


root = Order.into_field()
assert Order.into_field() is root and field(Order) is root   # cached once
assert root.nullable is False and root.name == "Order"
assert [child.name for child in root] == ["order_id", "at", "price", "legs", "note"]
assert root["price"].dtype == DataType("decimal128(18,4)")
assert root["price"].parquet_field_id == 7
assert root["note"].nullable and not root["order_id"].nullable

# An instance is a row under its class's own field.
order = Order(1, dt.datetime(2024, 1, 2, tzinfo=dt.timezone.utc), Decimal("1.5"), [Leg("ABC", 10)])
row = root.scalar(order)
assert row.as_py()[2] == Decimal("1.5000")
assert row.as_py()[3] == [["ABC", 10]]

# One annotation as one field, and a native field back to a dataclass.
price = Field.from_pyhint("price", Annotated[Decimal, ("arrow_type", pa.decimal128(9, 0))])
assert price.dtype == DataType("decimal128(9,0)")
Row = Field("Row", DataType("struct<id:int64 not null,tags:serie<utf8>>"), nullable=False).into_dataclass()
assert Row(id=1, tags=["a"]).tags == ["a"]
```

## Read a value through a column

`Field.scalar` / `DataType.scalar` is the one value door: it narrows to the
declared width, parses text, applies nullability and refuses what the type
cannot hold - there is no host-runtime cast in between.

```python
import pytest

from yggdryl import DataType, Field

qty = Field("qty", "int16", nullable=False)
value = qty.scalar(42)
assert value.kind == "i16" and value.id == "int16"   # narrowed to the column
assert qty.scalar("42") == value                     # text reads under the type

with pytest.raises(ValueError, match="non-nullable field received null"):
    qty.scalar(None)
with pytest.raises(ValueError, match="expected int16"):
    qty.scalar(70_000)                                 # never wraps
with pytest.raises(ValueError, match="expected float64, got i64"):
    DataType("float64").scalar(100)                   # an int is not a float
assert DataType("float64").scalar(100.0).kind == "f64"
assert Field("note", "utf8").scalar(None).is_null()   # nullable by default
```

## Build a row

A row is the ordered sequence of the struct's children. Named input is a
`Scalar.from_struct` record or a dataclass instance - a plain `dict` is a
*map* and a struct refuses it. A child the input does not name takes its
default.

```python
import pytest

import yggdryl
from yggdryl import Scalar

trade = yggdryl.struct(
    "trade",
    [yggdryl.int64("id", nullable=False), yggdryl.utf8("symbol")],
    nullable=False,
)

assert trade.scalar([7, "AAPL"]).as_py() == [7, "AAPL"]
named = trade.scalar(Scalar.from_struct({"symbol": "AAPL", "id": 7}))
assert named.as_py() == [7, "AAPL"]                   # declaration order
assert trade.scalar(Scalar.from_struct({"id": 7})).as_py() == [7, None]

with pytest.raises(ValueError, match="got map"):
    trade.scalar({"id": 7, "symbol": "AAPL"})         # a dict is a map

assert named[0].as_py() == 7 and len(named) == 2
```

## Infer a value and convert it back

`Scalar.from_` reads what a Python value already is - a bare `int` is `i64`,
a `float` `f64` - and `as_py` is the way back. Equality, order and hashing are
by value across widths, and `stable_hash` is the same in every language.

```python
import datetime as dt
from decimal import Decimal

from yggdryl import DataType, Scalar

seven = Scalar.from_(7)
assert (seven.kind, seven.id, seven.family) == ("i64", "int64", "integer")
assert Scalar.from_(1.5).kind == "f64"
assert Scalar.from_(Decimal("10.50")).kind == "d128"
assert Scalar.from_(dt.date(2024, 1, 2)).kind == "date32"
assert Scalar.from_(b"\x01").family == "bytes"

# One number at two widths is one value, one hash, one stable hash.
narrow = DataType("uint8").scalar(7)
assert narrow == seven and hash(narrow) == hash(seven)
assert narrow.stable_hash() == seven.stable_hash()
assert Scalar.from_(1) < Scalar.from_(2)

assert seven.as_py() == 7 and seven.as_int() == 7
assert Scalar.from_([1, None]).as_py() == [1, None]
assert Scalar.from_(42).into_field().name == "value"   # the inferred field
```

## Decimals, durations and checked arithmetic

`Scalar.decimal(coefficient, scale)` and `Scalar.duration(count, unit)` are
the two values the type side cannot state. A decimal column takes a
`Decimal`, text or an `int` - never a `float`. Arithmetic is exact or raises.

```python
from decimal import Decimal

import pytest

from yggdryl import Field, Scalar

price = Scalar.decimal(1050, 2)
assert price.as_py() == Decimal("10.50")
assert (price.unscaled, price.scale) == (1050, 2)
assert price == Scalar.decimal(105, 1)                  # normalized equality

amount = Field("amount", "decimal(10, 2)")
assert amount.scalar("12.5").as_py() == Decimal("12.50")
assert amount.scalar(Decimal("12.5")).as_py() == Decimal("12.50")
with pytest.raises(ValueError, match="got f64"):
    amount.scalar(12.5)                                # a float is inexact

assert (Scalar.decimal(1, 0) / Scalar.decimal(2, 0)) == Scalar.decimal(5, 1)
with pytest.raises(ArithmeticError):
    Scalar.decimal(1, 0) / Scalar.decimal(3, 0)        # inexact: refused
with pytest.raises(ZeroDivisionError):
    Scalar.from_(1) / 0
assert (Scalar.from_(10) / 4).as_py() == 2             # integer division
assert (Scalar.from_("a") + "b").as_py() == "ab"       # text concatenates

assert Scalar.duration(1, "ms").kind == "duration32"   # width from the count
assert Scalar.duration(2**31, "us").kind == "duration64"
```

## Temporal values and zones

A datetime column carries its unit and zone; the value door reads an aware
`datetime`, ISO text, or a count at the column's unit. A naive value in a
zoned column is refused rather than guessed.

```python
import datetime as dt

import pytest

from yggdryl import DataType, Field

at = Field("at", 'datetime64(ns,"UTC")', nullable=False)
utc = dt.timezone.utc
v = at.scalar(dt.datetime(2024, 1, 2, tzinfo=utc))
assert (v.unit, v.zone) == ("ns", "UTC")
assert v.count == 1_704_153_600_000_000_000
assert at.scalar("2024-01-02T00:00:00Z") == v
assert at.scalar(1_704_153_600_000_000_000) == v
assert v.as_py() == dt.datetime(2024, 1, 2, tzinfo=utc)

with pytest.raises(ValueError):
    at.scalar(dt.datetime(2024, 1, 2))                  # naive into zoned

day = DataType("date32").scalar("1970-01-02")
assert day.count == 1 and day.as_py() == dt.date(1970, 1, 2)
assert DataType("time64(us)").scalar(dt.time(1, 2, 3)).as_py() == dt.time(1, 2, 3)
assert DataType("timezone").scalar("Asia/Calcutta").as_py() == "Asia/Kolkata"
```

## Strings, bytes, codes and identifiers

A bound counts stored bytes; a registered code is its own datatype with its
own validity; a UUID column reads every spelling to one value.

```python
import uuid

import pytest

from yggdryl import DataType, Scalar

bounded = DataType("sized_ascii(4)")
assert bounded.scalar("USD").as_py() == "USD"
assert bounded.scalar("USD").dtype == bounded          # the value keeps its leaf
with pytest.raises(ValueError, match="at most 4 bytes"):
    bounded.scalar("EURO!")
with pytest.raises(ValueError):
    DataType("sized_utf8(4)").scalar("Grüß")           # six bytes of UTF-8

ccy = DataType("ccy")
assert (ccy.kind, ccy.code_width, ccy.string_parameters) == ("code", 3, None)
assert ccy.scalar("USD").kind == "ccy"
assert ccy.scalar("USD") != Scalar.from_("USD")        # a code is not a string
with pytest.raises(ValueError, match="check digit"):
    DataType("isin").scalar("US0378331006")
assert DataType("isin").scalar("US0378331005").as_py() == "US0378331005"

text = "01912d68-783e-7c9a-b1f2-0123456789ab"
column = DataType("uuid")
assert column.scalar(uuid.UUID(text)) == column.scalar(text.upper())
assert Scalar.from_(uuid.UUID(text)).kind == "string"  # undeclared: text
assert DataType("binary(2)").scalar(b"\x01\x02").as_py() == b"\x01\x02"
```

## Nested values: serie, map, union, dictionary

Each nested factory takes its child fields; a union value is `[type_id,
payload]` and a bare payload enters the one member that accepts it.

```python
import yggdryl

levels = yggdryl.serie("levels", yggdryl.float64("item"))
assert levels.dtype.id == "serie" and levels.dtype.kind == "nested"
assert levels.scalar([1.5, 2.5]).as_py() == [1.5, 2.5]

lookup = yggdryl.map_of("lookup", "utf8", "int64")
assert lookup.scalar({"a": 1, "b": 2}).as_py() == {"a": 1, "b": 2}
assert [key.as_py() for key in lookup.scalar({"a": 1})] == ["a"]  # walks keys

payload = yggdryl.dense_union("p", [yggdryl.int64("n", nullable=False), yggdryl.utf8("t")])
assert payload.scalar(7).as_py() == [0, 7]
assert payload.scalar("hi").as_py() == [1, "hi"]
assert payload.scalar([1, "hi"]).as_py() == [1, "hi"]

codes = yggdryl.dictionary("codes", "int16", "utf8")
assert codes.scalar("AAPL").dtype.id == "utf8"         # the decoded value
assert yggdryl.fixed_size_serie("xy", yggdryl.float64("item"), 2).dtype.id == "fixed_size_serie"
```

## Metadata and protocol properties

Metadata is `<SCHEME>:<property>` text on the field; typed accessors and
protocol views read and write that one map. `field["x"]` is a child;
`field.metadata["x"]` is a key.

```python
from yggdryl import Field

price = Field("price", "decimal(18, 4)", nullable=False, metadata={"source": "feed"})
price.set_parquet_field_id(17)
price.set_comment("closing price")
price.set_location("s3://warehouse/bars/data.arrow")
price.set_property("postgres", "type", "numeric(18,4)")
price.iceberg["doc"] = "closing price"

assert price.parquet_field_id == 17
assert price.metadata["PARQUET:field_id"] == "17"
assert price.comment == "closing price"
assert price.location.scheme == "s3"
assert price.get_property("postgres", "type") == "numeric(18,4)"
assert price.metadata["ICEBERG:doc"] == "closing price"
assert price.metadata["source"] == "feed"

row = Field(
    "row",
    "struct<year:int32 not null,venue:utf8 not null,px:float64>",
    nullable=False,
).with_partition_fields(["year", "venue"])
assert row.partition_field_names == ["year", "venue"]
assert row["year"].metadata["FIELD:partition"] == "true"
```

## Compare, diff and merge schemas

`equals` answers yes or no, `show_diffs` why; `merge_with` is the one
promotion table (widening by default).

```python
import pytest

from yggdryl import DataType, Field

left = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
right = Field("price", "float64", metadata={"venue": "XNAS"})
assert not left.equals(right)
assert left.equals(Field("price", "float64", nullable=False), with_metadata=False)
assert list(left.show_diffs(right)) == [
    "≠ $.nullable: false → true",
    '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
]

merged = DataType("struct<id:int32 not null,venue:utf8>").merge_with(
    DataType("struct<id:int64 not null,px:float64>")
)
assert merged["id"].dtype == DataType("int64")
assert [child.name for child in merged] == ["id", "venue", "px"]
assert merged["px"].nullable                           # one-sided -> nullable
assert DataType("int32").merge_with("int64", upscale=False) == DataType("int32")
with pytest.raises(ValueError):
    DataType("decimal(10,2)").merge_with("float64")    # exact vs approximate
```

## Serialize a schema, move a value as bytes

A schema is one structural document under JSON, YAML, TOML or a `dict`. A
value is one self-describing byte stream (what `pickle` carries), keeping its
exact leaf.

```python
import pickle

from yggdryl import DataType, Field, Scalar

field = Field("price", "decimal(9, 2)", nullable=False, metadata={"venue": "XPAR"})
assert Field.from_json(field.into_json()) == field
assert Field.from_yaml(field.into_yaml()) == field
assert Field.from_toml(field.into_toml()) == field
assert field.into_dict()["name"] == "price"
assert DataType.from_dict(DataType("uuid").into_dict()) == DataType("uuid")

value = DataType("int32").scalar(7)
data = value.into_value_bytes()
assert Scalar.from_value_bytes(data) == value and Scalar.from_value_bytes(data).kind == "i32"
row = Scalar.from_struct({"symbol": "AAPL", "sizes": [100, None]})
assert pickle.loads(pickle.dumps(row)) == row
```

## Cross an Arrow schema

A `Field` imports and exports pyarrow fields and schemas losslessly,
extension identity included; a bare `pa.DataType` has no metadata, so import
the field to keep a code, a UUID or a fixed decimal.

```python
import pyarrow as pa

import yggdryl
from yggdryl import DataType, Field

schema = pa.schema([
    pa.field("id", pa.uint32(), nullable=False),
    pa.field("tags", pa.list_(pa.string())),
])
root = Field.from_arrow_schema(schema, name="Trade")
assert root.nullable is False and root["id"].dtype == DataType("uint32")
assert root.into_arrow_schema().field("id").type == pa.uint32()

venue = yggdryl.mic("venue", nullable=False)
arrow_field = venue.into_arrow()
assert arrow_field.type == pa.string()
assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
assert Field.from_arrow(arrow_field) == venue           # identity kept
assert DataType.from_arrow(arrow_field.type) == DataType("utf8")  # lost

assert DataType("datetime64(us,UTC)").into_arrow() == pa.timestamp("us", tz="UTC")
assert Field("px", "float64").arrow_scalar(1.5) == pa.scalar(1.5)

# Engine rewrites and canonical defaults come from the same core.
assert DataType("uint8").into_scheme_compat("spark") == DataType("int16")
assert DataType("utf8").default_scalar().as_py() == ""
assert DataType("struct<id:int32 not null,note:utf8>").default_scalar().as_py() == [0, None]
```

## Gotchas in Python

- A `dict` is a map; a struct row is a list, a dataclass instance or
  `Scalar.from_struct({...})`.
- `DataType("float64").scalar(100)` and a `float` into a decimal column are
  refused: pass `100.0`, and `Decimal`/`str`/`int` for decimals.
- `Scalar.from_(uuid.UUID(...))` is text; declare `uuid`. A bare `int` infers
  as `i64` - name the width on the type to store `int16`.
- `field["x"]` is a child; metadata is `field.metadata["x"]`. Assigning
  `field["new"] = Field(...)` appends a child.
- No `DataType.decimal128` and no `DataType.index_of`: use
  `DataType.decimal(p, s)` / `yggdryl.decimal128(name, p, s)` and
  `Field.index_of`.
- The first `hash()` locks a `Field`/`DataType` wrapper against mutation;
  `copy.copy` unlocks, `stable_hash()` never locks.
- Mypy needs `plugins = ["yggdryl.mypy"]` to see the `into_field()` that
  `@scalar` installs; `field(Class)` is the checker-neutral spelling.
- `FieldScalar`, `FieldRecord`, `DataTypeKind` ranges and the `wkb` reader are
  Rust only; a geospatial value crosses as WKB `bytes`.
