# Python records

```python
from yggdryl import Record, record


@record(frozen=True, slots=True)
class Leg(Record):
    symbol: str
    quantity: int


@record(frozen=True, slots=True)
class Order(Record):
    order_id: int
    active: bool
    legs: list[Leg]
    note: str | None = None


order = Order.from_dict(
    {
        "order_id": "42",
        "active": "true",
        "legs": [{"symbol": "ABC", "quantity": "10"}],
    }
)

assert order == Order(42, True, [Leg("ABC", 10)])
assert order.to_dict() == {
    "order_id": 42,
    "active": True,
    "legs": [{"symbol": "ABC", "quantity": 10}],
    "note": None,
}
assert Order.schema_field() is Order.schema_field()
assert Order.schema_fields() is Order.schema_fields()
```

`@record` creates a standard-library dataclass backed by cached native Yggdryl
`Field` values. Safe dictionary conversion validates and casts annotations;
`safe=False` is the explicit shallow path.

Use reserved `Annotated` options when the logical Python type needs an exact
Arrow layout or Field property:

```python
from decimal import Decimal
from typing import Annotated

import pyarrow as pa
from yggdryl import Field

price = Field.from_pyhint(
    "price",
    Annotated[
        Decimal,
        ("arrow_type", pa.decimal128(9, 0)),
        {"nullable": False, "metadata": {"unit": "EUR"}, "id": 7},
    ],
)

assert price.to_arrow().type == pa.decimal128(9, 0)
assert price.id == 7
```

The full guide documents metadata merging, nullability, dictionary options,
ExtensionType preservation, and left-to-right precedence.

```python
import pyarrow as pa
from yggdryl import Record

Metric = Record.from_arrow_schema(
    pa.schema([
        pa.field("name", pa.string(), nullable=False),
        pa.field("value", pa.float64(), nullable=True),
    ]),
    class_name="Metric",
    module=__name__,
)

values = tuple(Metric.from_dicts([
    {"name": "latency", "value": "1.25"},
    {"name": "missing", "value": None},
]))
reader = Metric.into_arrow_record_batch_reader(values, batch_size=1)

assert tuple(Metric.from_arrow(reader)) == values
```

Arrow Field/Schema factories produce real record dataclasses whose cached
native fields remain authoritative. Batch, table, reader, C-stream, and
dictionary iterators all reuse the same safe deep caster.

The complete, example-first guide covers annotation mappings, Arrow identity
metadata, nullability, error policies, ordinary dataclasses, generics, forward
references, cache behavior, and benchmarks:

- [Python records guide](https://platob.github.io/yggdryl/extensions/python/records/)
- [Local MkDocs source](../../docs/extensions/python/records.md)
- [Runnable example](examples/records.py)
- [Arrow/tabular example](examples/records_arrow.py)
- [Benchmark](benchmarks/records.py)
- [Arrow/tabular benchmark](benchmarks/records_arrow.py)

```console
python benchmarks/records.py --iterations 100000
python benchmarks/records_arrow.py --min-time 0.2 --repeat 7
```

Run those commands from `python` after building the extension
with `maturin develop`.
