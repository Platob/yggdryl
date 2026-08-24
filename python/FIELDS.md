# Python field classes

```python
import dataclasses

from yggdryl import field, scalar


@scalar(frozen=True, slots=True)
class Leg:
    symbol: str
    quantity: int


@scalar(frozen=True, slots=True)
class Order:
    order_id: int
    active: bool
    legs: list[Leg]
    note: str | None = None


order = Order(42, True, [Leg("ABC", 10)])

assert dataclasses.is_dataclass(Order)
order_field = Order.field()
assert Order.field() is order_field
assert field(Order) is order_field
assert field(order) is order_field
assert order_field.name == "Order"
assert [child.name for child in order_field] == [
    "order_id",
    "active",
    "legs",
    "note",
]
assert order_field["note"].nullable
```

`@scalar` forwards the standard `dataclasses.dataclass` options and compiles the
class annotations into one non-null native Struct `Field`. The resolved value
is cached once and returned by the `Class.field()` staticmethod. The
decorator does not add conversion, serialization, or Arrow methods to the
dataclass.

Use `Class.field()` for the cached Struct field and `field(value)` for general
conversion. Removed names are listed once in the
[migration notes](../docs/migration.md#field-classes-and-declared-record-shape).

An undecorated subclass inherits its nearest decorated base's cached root.
Apply `@scalar` to the subclass when its annotations should declare a distinct
Struct field.

The canonical cross-runtime signatures and error contract live in
[the core field guide](../docs/field.md#converting-to-one-native-field).
Python accepts a native `Field`, a PyArrow Schema/Field/DataType, or a
dataclass class/instance; `name` uniformly renames the result, including to the
empty string.

Mypy needs the bundled plugin to model the staticmethod that the decorator adds
at runtime. Enable it in the consuming project's configuration:

```toml
[tool.mypy]
plugins = ["yggdryl.mypy"]
```

Checkers without class-decorator hooks can use the equivalently typed
`field(Class)` call.

An existing ordinary dataclass can enter through the same native boundary:

```python
import dataclasses

from yggdryl import field


@dataclasses.dataclass
class Venue:
    mic: str


venue_field = field(Venue)

assert venue_field.name == "Venue"
assert venue_field["mic"].data_type.id == "utf8"
```

Use reserved `Annotated` options when the logical Python type needs an exact
Arrow layout or Field property. Inference still produces native values; an
annotation is never a second schema model.

```python
from decimal import Decimal
from typing import Annotated

import pyarrow as pa

from yggdryl import scalar


@scalar
class Quote:
    price: Annotated[
        Decimal,
        ("arrow_type", pa.decimal128(9, 0)),
        {"nullable": False, "metadata": {"unit": "EUR"}, "id": 7},
    ]


price = Quote.field()["price"]

assert price.into_arrow().type == pa.decimal128(9, 0)
assert price.parquet_field_id == 7
assert price.metadata["unit"] == "EUR"
```

Arrow schemas use the inverse pair on `Field` itself:

```python
import pyarrow as pa

from yggdryl import Field


metric_field = Field.from_arrow_schema(
    pa.schema([
        pa.field("name", pa.string(), nullable=False),
        pa.field("value", pa.float64(), nullable=True),
    ]),
    name="Metric",
)
Metric = metric_field.into_dataclass()

assert Metric.field() is metric_field
assert Metric.field().into_arrow_schema().field("value").nullable
assert Metric(name="latency", value=1.25).value == 1.25
```

The native import preserves physical widths, nested layout, metadata,
extension identity, and dictionary transport state. `into_dataclass()` derives
Python annotations from that exact graph rather than re-inferring the schema.

See the [Field guide](../docs/field.md) and the
[Python boundary guide](../docs/extensions/python.md) for the complete API.
The reproducible decorator benchmarks are:

```console
python benchmarks/fields.py --iterations 100000
python benchmarks/fields_arrow.py --iterations 10000
```

Run those commands from `python` after building the extension with
`maturin develop`.
