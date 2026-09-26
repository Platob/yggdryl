# yggdryl-expressions in Python

`Term`, `Filter`, `Selector`, `Plan`, `Expression`, `Bounds` and `FieldPath`
import from `yggdryl`; the UDF decorators and the `FUNCTIONS` / `COMPARISONS`
/ `VERBS` vocabularies from `yggdryl.expression`. Arrow crosses as `pyarrow`.

## Parse a predicate, bind it once, answer rows

`bind` resolves names and converts each literal into its column's type once;
a row is a sequence in schema order or a mapping of column to value.

```python
from decimal import Decimal

from yggdryl import Field, Filter

schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2),size:bigint>", False)
filter = Filter("ccy = 'EUR' and price > 100")
assert filter.columns() == ["ccy", "price"]

bound = filter.bind(schema)
assert str(bound.term) == "ccy = 'EUR' and price > decimal32(9,2) '100.00'"

assert bound.matches(["EUR", Decimal("150.00"), 5])
assert bound.matches({"ccy": "EUR", "price": Decimal("150.00"), "size": 5})
# A null makes the answer unknown, and unknown does not keep the row.
assert bound.eval({"ccy": "EUR", "price": None, "size": 5}) is None
assert not bound.matches({"ccy": "EUR", "price": None, "size": 5})
```

## Build a term without text, with a parameter

Method builders (`gt`, `eq`, `is_in`, `between`, `is_null`, ...) and the
`&`, `|`, `~`, `+`, `-`, `*`, `/`, `%` operators compose the tree; a
`:name` parameter is supplied once at `bind`.

```python
from yggdryl import Field, Term

schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2),size:bigint>", False)

composed = Term.column("price").gt(100) & Term.column("ccy").eq(Term.literal("EUR"))
assert composed == Term("price > 100 and ccy = 'EUR'")
assert str(Term.column("size") * 2 + 1) == "size * 2 + 1"
assert str(Term.column("size").between(1, 10)) == "size between 1 and 10"
assert str(Term.column("ccy").is_in([Term.literal("EUR"), Term.literal("USD")])) == "ccy in ('EUR', 'USD')"

late = Term("size >= :floor")
assert late.parameters() == ["floor"]
bound = late.bind(schema, {"floor": 10})
assert str(bound.term) == "size >= 10"
assert bound.matches({"size": 11})

assert str(Term("a = 1 or a = 2").simplify()) == "a in (1, 2)"
```

## Filter and project an Arrow batch

`Filter.apply_arrow_batch` keeps rows, and hands the very same batch back when
every row is kept. `Selector` computes, renames, casts and excludes.

```python
import pyarrow as pa

from yggdryl import DataType, Field, Filter, Selector

root = Field("rows", "struct<ccy:utf8,size:int64,secret:utf8>", False)
batch = pa.record_batch(
    {"ccy": ["EUR", "USD", "EUR"], "size": pa.array([5, 5, 0], pa.int64()), "secret": ["a", "b", "c"]}
)

kept = Filter("ccy = 'EUR' and size > 1").apply_arrow_batch(batch)
assert kept.column("ccy").to_pylist() == ["EUR"]
assert Filter.always_true().apply_arrow_batch(batch) is batch

selector = Selector("* exclude (secret), size * 2 as doubled int32")
projected = selector.apply_arrow_batch(batch)
assert projected.schema.names == ["ccy", "size", "doubled"]
assert projected.column("doubled").to_pylist() == [10, 10, 0]
# The published schema is known without data.
assert selector.apply_field(root).dtype["doubled"].dtype == DataType("int32")
assert Selector.all().bind(root).is_identity
```

## Shape a stream with a plan

`apply_arrow_reader` binds once and wraps the `pyarrow.RecordBatchReader`
lazily; `apply_arrow` keeps the input kind (batch, table or reader).

```python
import pyarrow as pa

from yggdryl import Expression, Plan

batch = pa.record_batch({"ccy": ["A", "B", "C", "D"], "size": pa.array([1, 4, 3, None], pa.int64())})

plan = Plan("select ccy, size as quantity where size >= 2 order by size desc limit 1")
reader = plan.apply_arrow_reader(pa.RecordBatchReader.from_batches(batch.schema, [batch]))
assert reader.schema.names == ["ccy", "quantity"]
assert reader.read_all().column("quantity").to_pylist() == [4]
assert isinstance(plan.apply_arrow(pa.Table.from_batches([batch])), pa.Table)

steps = Expression("where size is not null; select upper(ccy) as ccy")
assert steps.kind == "sequence" and len(steps.steps) == 2
assert steps.apply_arrow_batch(batch).column("ccy").to_pylist() == ["A", "B", "C"]
```

## Evaluate native rows

`apply_records(rows, schema)` binds once and streams canonical rows as
dictionaries; without a schema the first record's own datatype is the schema.

```python
from yggdryl import Expression, Field

root = Field("rows", "struct<ccy:utf8,size:int64>", False)
steps = Expression("where size > 1; select upper(ccy) as ccy")

records = steps.apply_records([{"ccy": "a", "size": 1}, {"ccy": "b", "size": 2}], root)
assert records.field.dtype["ccy"].dtype == root.dtype["ccy"].dtype
assert records.collect() == [{"ccy": "B"}]
```

## Use the bound tiers directly: mask, filter, reader

A `Bound` answers one row, one batch and one stream from the same compiled
tree - the shape for a hand-written batch loop.

```python
import pyarrow as pa

from yggdryl import Field, Term

schema = Field("trades", "struct<ccy:utf8,size:bigint>", False)
bound = Term("ccy = 'EUR' and size > 10").bind(schema)
assert bound.column_indices == [0, 1]

batch = pa.record_batch({"ccy": ["EUR", "USD"], "size": pa.array([25, 25], pa.int64())})
assert bound.evaluate_arrow_batch(batch).to_pylist() == [True, False]
assert bound.filter_mask_arrow_batch(batch).to_pylist() == [True, False]
assert bound.filter_arrow_batch(batch).num_rows == 1

reader = pa.RecordBatchReader.from_batches(batch.schema, [batch, batch])
assert bound.filter_arrow_reader(reader).read_all().num_rows == 2
```

## Skip a file from its statistics

`statistics_prune` answers `False` only when no row can match; anything
unprovable answers `True` and the file is read.

```python
from yggdryl import Bounds, Field, Term

schema = Field("trades", "struct<ccy:utf8,size:bigint>", False)
bounds = Bounds(rows=1_000).with_column("ccy", "EUR", "USD", 0).with_column("size", 1, 99, 4)

assert not Term("size > 1000").bind(schema).statistics_prune(bounds)
assert Term("size > 50").bind(schema).statistics_prune(bounds)
assert Term("size > 1000").bind(schema).statistics_certainty(bounds) is False
assert Term("size is null").bind(schema).statistics_prune(bounds)
```

## Split a predicate into partition and row halves

`Bound.partition_split()` answers `(answerable, remaining)`: the conjuncts a
partition layout settles, and the residual over rows.

```python
from yggdryl import DataType, Field, Term

year = Field("year", "int32", False)
year.set_partition(True)
schema = Field("trades", DataType.from_fields([year, Field("price", "decimal(9,2)", False)]), False)

answerable, remaining = Term("year = 2024 and price > 100").bind(schema).partition_split()
assert str(answerable) == "year = int32 '2024'"
assert str(remaining) == "price > decimal32(9,2) '100.00'"
```

## Push the filter and projection into a read

A record read takes `filter=` and `select=` as option properties (or a
`RecordOptions` carrying them): the media prunes by path, statistics and
columns, and only matching rows reach Python.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import Filter, IOBase

with tempfile.TemporaryDirectory() as root:
    handle = IOBase(pathlib.Path(root) / "trades.parquet")
    handle.overwrite_arrow_table(pa.table({"ccy": ["EUR", "USD", "EUR"], "size": pa.array([1, 5, 9], pa.int64())}))

    table = handle.read_arrow_reader(filter="size > 2", select="ccy, size * 2 as doubled").read_all()
    assert table.to_pylist() == [{"ccy": "USD", "doubled": 10}, {"ccy": "EUR", "doubled": 18}]

    options = handle.record_options()
    options.filter = Filter("ccy = 'EUR'")
    options.select = "size"
    assert str(options.plan) == "select size where ccy = 'EUR'"
    assert handle.read_arrow_reader(options=options).read_all().column("size").to_pylist() == [1, 9]
    assert list(handle.read_records(filter="size < 5")) == [{"ccy": "EUR", "size": 1}]
```

## Run a plan against storage

`execute` reads the `from` target through its holder with the read sections
pushed down; a write verb sends the shaped stream to its target.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import Plan

with tempfile.TemporaryDirectory() as root:
    url = (pathlib.Path(root) / "trades.arrow").as_uri()

    Plan(f"create '{url}' (id int64 not null, name utf8)").execute().read_all()
    rows = pa.record_batch({"id": pa.array([1, 2, 3], pa.int64()), "name": ["a", "b", "c"]})
    Plan(f"insert into '{url}'").apply_arrow_batch(rows)

    read = Plan(f"select name from '{url}' where id > 1 order by id desc")
    assert read.execute().read_all().column("name").to_pylist() == ["c", "b"]

assert str(Plan("merge into t on (id) select id from s")) == "upsert into t by (id) select id from s"
built = Plan().with_select("id, name").with_source("raw").with_filter("id > 1").with_limit(10)
assert str(built) == "select id, name from raw where id > 1 limit 10"
```

## Address a nested value by path

`FieldPath` parses and renders the path grammar; inside a term the same steps
are accessors, plus predicate segments over a serie of structs.

```python
from yggdryl import Field, FieldPath, Term

path = FieldPath("line[-1].price as last_price")
assert len(path) == 3 and path.column_name == "last_price"
assert str(path.parent()) == "line[-1]"
assert str(FieldPath("line").join(0)) == "line[0]"
assert len(FieldPath('"a.b"')) == 1 and len(FieldPath("a.b")) == 2

root = Field("orders", "struct<line:serie<struct<ccy:utf8,price:int64>>>", False)
row = {"line": [{"ccy": "EUR", "price": 10}, {"ccy": "USD", "price": 12}, {"ccy": "EUR", "price": 14}]}
assert Term("line[-1].price").bind(root).eval(row) == 14
assert Term("line[ccy = 'EUR'][0].price").bind(root).eval(row) == 10
assert str(Term.column("line").at(-1).child("price")) == "line[-1].price"
```

## Register a user-defined function

`@user_defined_function` reads the signature off the annotations and
registers `namespace.name`; the function stays callable. `vectorized=True`
takes and answers `pyarrow` arrays, one call per batch.

```python
import pyarrow as pa
import pyarrow.compute as pc

from yggdryl import Field, Selector, Term
from yggdryl.expression import user_defined_filter, user_defined_function


@user_defined_function(namespace="skills")
def triple(value: int) -> int:
    return value * 3


@user_defined_function(namespace="skills", vectorized=True, returns="utf8")
def shout(text: str) -> str:
    return pc.utf8_upper(text)


@user_defined_filter(namespace="skills")
def big(size: int | None) -> bool | None:
    return None if size is None else size > 1


rows = Field("rows", "struct<size: int64, ccy: utf8>", nullable=False)
batch = pa.record_batch({"size": pa.array([1, None, 3], pa.int64()), "ccy": ["a", "b", "c"]})

assert triple(4) == 12
assert str(triple.term("size")) == "skills.triple(size)"
projected = Selector("skills.triple(size) as tripled, skills.shout(ccy) as loud").apply_arrow_batch(batch)
assert projected.column("tripled").to_pylist() == [3, None, 9]
assert projected.column("loud").to_pylist() == ["A", "B", "C"]
assert big.where("size").apply_arrow_batch(batch).column("ccy").to_pylist() == ["c"]

stored = Selector("skills.triple(size) as tripled").into_field(rows)
assert stored.dtype["tripled"].transform["function"] == "skills.triple"
assert stored.dtype["tripled"].transform["sources"] == '["size"]'

for function in (triple, shout, big):
    assert function.unregister()
try:
    Term("skills.triple(size)").field(rows)
    raise AssertionError("an unregistered function is refused")
except ValueError as error:
    assert "skills.triple" in str(error)
```

## Keep a derivation on the schema

`into_field` writes a selector as the declaration it is, each computed column
carrying `TRANSFORM:` metadata; `Selector.from_field` reads it back.

```python
from yggdryl import Field, Selector

root = Field("rows", "struct<ccy:utf8,size:int64>", False)
stored = Selector("ccy, size * 2 as doubled int32").into_field(root)
assert stored.dtype["doubled"].transform["expression"] == "size * 2"
assert Selector.from_field(stored) == Selector("ccy utf8 null, size * 2 as doubled int32 null")
```

## Read the plan, the text and the document

Every layer prints canonical text that re-parses to the same tree, has a JSON
document, pickles, and draws its bound plan with `explain`.

```python
import pickle

from yggdryl import Expression, Field, Plan, Selector, Term
from yggdryl.expression import FUNCTIONS, VERBS

schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2)>", False)
drawn = Term("price > 100").bind(schema).explain()
assert drawn.startswith(">") and "column price" in drawn

plan = Plan("select ccy from t where price > 0 limit 10 offset 5")
assert Plan(str(plan)) == plan and pickle.loads(pickle.dumps(plan)) == plan
assert plan.read_columns() == ["price", "ccy"]
assert Plan("select * exclude (price)").read_columns() is None
assert Selector.from_json(Selector.all().into_json()) == Selector.all()

assert "coalesce" in FUNCTIONS and "upsert into" in VERBS
try:
    Expression("price > 1")
    raise AssertionError("an Expression names its clause")
except ValueError as error:
    assert "expected `select`, `where`" in str(error)
```

## Gotchas in Python

- A `str` handed to a comparison builder is term **text**:
  `Term.column("ccy").eq("EUR")` compares two columns. Pass
  `Term.literal("EUR")` or `"'EUR'"`. Numbers are literals.
- `>`, `<`, `==` on a `Term` are Python comparisons (identity and total
  order), not builders - use `.gt()`, `.eq()`. `&`, `|`, `~` and arithmetic
  operators do build terms.
- A Python `float` against a `decimal(9,2)` column is a different number:
  pass `decimal.Decimal`.
- `Filter.apply_arrow_batch` and `Selector.apply_arrow_batch` bind per call;
  in a loop use `apply_arrow_reader` once, or hold `bind(...)`.
- `Bounds` and statistics pruning are Python and Rust only.
- `Plan.execute()` and `apply_arrow_reader` answer a one-shot
  `pyarrow.RecordBatchReader`; drain it once.
