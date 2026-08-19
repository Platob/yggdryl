# yggdryl Python bindings

```python
import os
from decimal import Decimal

import pyarrow as pa

from yggdryl import DataType, Field, MediaType, MimeType, Uri

price = DataType.decimal("18", 4)
clock = DataType.time("microseconds")
text = DataType(str)
field = Field("price", price, nullable=False)
field.set_table_name("bars")
field.set_parquet_field_id(17)
field.set_location("s3://warehouse/bars/data.arrow")
field.set_property("postgres", "type", "numeric(18,4)")
scalar = field.arrow_scalar(Decimal("12.5000"))
file = Uri.from_path(r"C:\data\prices.arrow")
encoded = Uri("https://example.test/prices.csv.gz")

assert str(price) == "decimal128(18,4)"
assert str(clock) == "time64(us)"
assert text == DataType("utf8")
assert field.to_arrow().name == "price"
assert field.parquet_field_id == 17
assert field["PARQUET:field_id"] == "17"
assert field.location.scheme == "s3"
assert field.get_property("postgres", "type") == "numeric(18,4)"
assert scalar == pa.scalar(Decimal("12.5000"), type=pa.decimal128(18, 4))
assert str(file) == "file:///C:/data/prices.arrow"
assert os.fspath(file) == "C:/data/prices.arrow"
assert encoded.media_type.base == MimeType("text/csv")
encoded.set_media_type(
    MediaType.from_parts(MimeType("application/json"), [MimeType("application/gzip")])
)
assert encoded.file_name == "prices.json.gz"
```

The package provides native Python views over Yggdryl's `DataType`, `Field`,
`MimeType`, `MediaType`, `Uri`, `Url`, and `Urn`, plus dataclass-compatible records. Parsing,
validation, Arrow conversion, ordering, stable hashing, and path normalization
remain owned by Rust. Python 3.10 and PyArrow 18 are the minimum supported
versions; 15 supplied the generic Arrow C-stream reader boundary, and 18 is
where the run-end-encoded map and extension-type scalar paths became correct.
`DataType.arrow_scalar(value, *, safe=True)` builds a Scalar of the projected
physical type. `Field.arrow_scalar` adds top-level nullability enforcement and returns
an already matching Scalar by identity; `safe=False` explicitly requests
PyArrow's unsafe cast rules. Conversion from arbitrary Python objects into a
PyArrow Scalar is Python-specific; native scalar validation and materialization
are shared through the default-enabled core `yggdryl::arrow` runtime.

```python
from yggdryl import Field, MediaType, MimeType

media = MediaType.from_parts(MimeType.CSV, [MimeType.GZIP, MimeType.ZSTD])
field = Field(
    "payload",
    "binary",
    metadata={"HTTPS:Content-Type": "text/csv; charset=utf-8"},
)
field.set_media_type(media)

assert field.mime_type == MimeType.CSV
assert field.content_encoding == "gzip, zstd"
assert field.get("HTTPS:CONTENT-TYPE") == "text/csv"
```

`MimeType` exposes the complete native known vocabulary as immutable class
constants and accepts custom validated MIME names. `MediaType()` defaults to
octet-stream, consumes iterable encodings once, and returns a detached tuple
from `encodings`; it is mutable and intentionally unhashable. Field HTTP and
HTTPS input shares canonical lowercase `http:*` Arrow metadata. Raw header
properties retain parameters, while typed MIME/media, unsigned content length,
and absolute HTTP Location accessors validate without a binding-side parser.
Pair media updates are atomic and cache-aware.

Precise Arrow layout can stay beside the logical Python annotation. Reserved
`Annotated` options work as `(key, value)` pairs or entries in one mapping:

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
assert price.parquet_field_id == 7
```

```python
from yggdryl import Field, fields
from yggdryl.fields import Int32Field, ListField

trade_id: Int32Field = fields.int32("trade_id", nullable=False)
tags: ListField[str] = fields.list("tags", fields.utf8("item"))

assert type(trade_id) is Field
assert tags.data_type.kind == "list"
assert trade_id.show_diff(trade_id) == "✓ equal"
```

The categorized `yggdryl.fields` package covers every native datatype variant.
Its aliases preserve kind/value information for static typing while factories
return the same generic native `Field`. `equals(..., with_metadata=False)` can
ignore metadata recursively; `show_diffs` returns readable UTF-8 difference
lines and `show_diff` joins them.

```python
from decimal import Decimal

from yggdryl.records import record

@record
class Order:
    order_id: int
    price: Decimal

order = Order(42, Decimal("12.50"))
payload = order.into_toml()  # UTF-8 bytes with a collision-safe envelope
assert Order.from_toml(payload) == order
```

`yggdryl.json`, `yggdryl.toml`, and `yggdryl.yaml` expose byte-first `dumps`/`loads`
plus path and typed text/binary file-object `dump`/`load`. JSON Lines and YAML
also expose document-stream `dump_all`/`load_all`; TOML is deliberately one
document. String content uses the borrowed native text path; paths and named or
explicitly formatted I/O redirect to the native reader/writer paths without
staging a whole encoded document.
A document carries shapes, never class names: a `set` reads back as a list and a
`uuid.UUID` as its text, and a class is constructed only through an explicit
target such as `cls=`.

```python
import pyarrow as pa

from yggdryl import Record

Trade = Record.from_arrow_schema(
    pa.schema([
        pa.field("trade_id", pa.uint32(), nullable=False),
        pa.field("tags", pa.list_(pa.string()), nullable=False),
    ]),
    class_name="Trade",
    module=__name__,
)

records = tuple(Trade.from_dicts([
    {"trade_id": "1", "tags": ["new"]},
    {"trade_id": "2", "tags": ["filled"]},
]))
batch = Trade.into_arrow_record_batch(records)

assert tuple(Trade.from_arrow_record_batch(batch)) == records
assert Trade.into_arrow_schema().field("trade_id").type == pa.uint32()
```

Dynamic records import Arrow Fields/Schemas through native `Field` and
`DataType` values, preserving exact physical widths and nested layout.
`from_arrow` lazily accepts batches, tables, readers, C-stream exporters, and
batch iterables; bounded output is available through
`into_arrow_record_batches` and `into_arrow_record_batch_reader`.

`DataType.cast_arrow_array` and `Field.cast_arrow_array` use the native Arrow
kernel plan; their `cast_arrow_batch` forms reconcile Struct columns by
ASCII-case-insensitive name, target order, and Field null/default policy.
`yggdryl.Tabular` is the open resource protocol, while the growable in-memory
`ArrowTable` is convenient for tests and local batch work. `TabularMixin`
derives RecordBatchReader, non-consuming Dataset getters, positional
append/upsert, atomic overwrite, and cached Record conveniences from a concrete
adapter's private cursor-read, snapshot, and overwrite-all hooks. See the
[Python tabular guide](https://platob.github.io/yggdryl/extensions/python/tabular/)
and the reproducible [ArrowTable benchmark](benchmarks/tabular.py).

`DataType(value)` infers native wrappers, datatype expression strings,
PyArrow datatypes, and Python annotations such as `str` or `list[int]`.
`DataType.decimal(precision, scale=0)` also accepts integer-like objects and
base-10 numeric strings, selecting Decimal128 through precision 38 and
Decimal256 through precision 76.
`DataType.time(unit)` accepts the shared native unit aliases and selects
Time32 for seconds/milliseconds or Time64 for microseconds/nanoseconds.
Bare `DataType.variant()` is the self-describing semi-structured Variant
datatype; `DataType.variant(fields)` stays the dense-union sugar building the
canonical dense Union with sequential native type IDs - the parenthesis
disambiguates. `fields.variant` builds the bare Variant field,
`fields.dense_union` the union one, and explicit `fields.union` remains
available for custom IDs or sparse layout.

Start with the [Python guide](https://platob.github.io/yggdryl/extensions/python/)
and the [records guide](RECORDS.md).
