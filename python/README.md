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
assert field.into_arrow().name == "price"
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
`MimeType`, `MediaType`, `Uri`, `Url`, and `Urn`, plus the `@scalar` decorator
for defining schemas with standard-library dataclasses. Parsing,
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
from `encodings`; it is mutable until first hashed. Hashing locks that wrapper,
and a copy is an independent mutable value. URI, URL, and URN values use the
same rule. Field HTTP and
HTTPS input shares canonical lowercase `http:*` Arrow metadata. Raw header
properties retain parameters, while typed MIME/media, unsigned content length,
and absolute HTTP Location accessors validate without a binding-side parser.
Pair media updates are atomic and cache-aware. Hashing a `Field` likewise
locks every equality-affecting field and metadata mutation on that wrapper;
copies and pickle round-trips are independent values.

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

assert price.into_arrow().type == pa.decimal128(9, 0)
assert price.parquet_field_id == 7
```

```python
from yggdryl import Field, fields
from yggdryl.fields import Int32Field, ListField

trade_id: Int32Field = fields.int32("trade_id", nullable=False)
tags: ListField[str] = fields.list("tags", fields.utf8("item"))

assert type(trade_id) is Field
assert tags.dtype.kind == "list"
assert trade_id.show_diff(trade_id) == "✓ equal"
```

The categorized `yggdryl.fields` package covers every native datatype variant.
Its aliases preserve kind/value information for static typing while factories
return the same generic native `Field`. `equals(..., with_metadata=False)` can
ignore metadata recursively; `show_diffs` returns readable UTF-8 difference
lines and `show_diff` joins them.

```python
from decimal import Decimal

from yggdryl import scalar, toml

@scalar
class Order:
    order_id: int
    price: Decimal

order = Order(42, Decimal("12.50"))
payload = toml.dumps(order)  # ordinary UTF-8 TOML bytes

assert Order.field().name == "Order"
assert toml.loads(payload, cls=Order) == order
```

`yggdryl.json`, `yggdryl.toml`, and `yggdryl.yaml` expose byte-first `dumps`/`loads`
plus declared `os.PathLike` and typed text/binary file-object `dump`/`load`.
A source `str` is always document content; use `pathlib.Path` to name a source
location. String destinations remain paths because output has no content/path
ambiguity. JSON Lines and YAML
also expose document-stream `dump_all`/`load_all`; TOML is deliberately one
document. String content uses the borrowed native text path; paths and named or
explicitly formatted I/O redirect to the native reader/writer paths without
staging a whole encoded document. For `load_all`, Python supplies only its
`read(size)`/`readline(size)` protocol: the owning Rust iterator decides JSON
Lines and YAML document boundaries, enforces cumulative limits, and reports
core byte offsets. It reads lazily under one bounded parser window, leaves the
caller-owned stream open, and is fused after its first read or parse failure.
A document carries shapes, never class names: a `set` reads back as a list and a
`uuid.UUID` as its text, and a class is constructed only through an explicit
target such as `cls=`.

```python
import pyarrow as pa

from yggdryl import Field, field

trade_field = Field.from_arrow_schema(
    pa.schema([
        pa.field("trade_id", pa.uint32(), nullable=False),
        pa.field("tags", pa.list_(pa.string()), nullable=False),
    ]),
    name="Trade",
)
Trade = trade_field.into_dataclass()

trade = Trade(trade_id=1, tags=["new"])

assert field(Trade) is trade_field
assert Trade.field() is trade_field
assert Trade.field().into_arrow_schema().field("trade_id").type == pa.uint32()
```

Arrow schemas import through the native `Field`, preserving exact physical
widths, metadata, dictionary state, and nested layout. `into_dataclass()`
builds a normal dataclass whose cached static `field()` result is that
native shape; it does not inject row conversion, codec, or Arrow methods into
the class.

Use cached `Class.field()` for decorated dataclasses and `field(value)` for the
general conversion funnel.

`DataType.cast_arrow_array` and `Field.cast_arrow_array` use the native Arrow
kernel plan; their `cast_arrow_batch` forms reconcile Struct columns by
ASCII-case-insensitive name, target order, and Field null/default policy.
Readers and writers continue to exchange `pyarrow.RecordBatchReader` values;
class declarations remain schema definitions rather than a second row I/O
surface.

Record writes name both their Python representation and intent:
`overwrite_arrow_reader`, `append_arrow_table`, `merge_arrow_batch`,
and the corresponding `*_records`, pandas, and polars adapters all redirect to
the same streamed Rust pipeline. Every call takes one keyword-only `options=`
value. Set `options.commit_row_size = N` to publish each complete group of `N`
incoming rows plus the final remainder; leave it as `None` for one publication
after successful end of input. A later failure leaves completed groups visible
by design, and zero is rejected before a one-shot Python input is inspected.

For configured intent, the same representations expose `write_*` with the
canonical `(input, mode, *, options=None)` order. `mode` is required and is one
of `"overwrite"`, `"append"`, or `"merge"`; it is validated before the input
is exported or iterated. `is_io()` reports the general byte/record capability,
while lazy `row_size` and `column_size` read whole-media dimensions from native
format metadata and cache them only while the handle is open.

`DataType(value)` infers native wrappers, datatype expression strings,
PyArrow datatypes, and Python annotations such as `str` or `list[int]`.
`DataType.decimal(precision, scale=0)` also accepts integer-like objects and
base-10 numeric strings, selecting Decimal128 through precision 38 and
Decimal256 through precision 76.
`DataType.time(unit)` accepts the shared native unit aliases and selects
Time32 for seconds/milliseconds or Time64 for microseconds/nanoseconds.
`Scalar` exposes checked native arithmetic through `add`, `subtract`,
`multiply`, `divide`, `remainder`, `negate`, and `absolute`, plus Python's
matching operators (including reflected operators and `abs`). Python-native
operands are inferred once at the boundary; overflow, zero division, inexact
decimal division, and invalid operand kinds retain distinct Python errors.
`Expression` uses the same arithmetic names and operators to build native
trees. Strings remain expression grammar (`"price"` names a column and
`"'fee'"` is a literal); every non-string Python operand is inferred as one
native `Scalar`, and reflected operators preserve left-to-right order.
Avro schemas compare, order, and hash by their complete retained native schema
identity; `fingerprint()` remains the separate Parsing Canonical Form identity
that intentionally omits annotations. Decoded Avro containers and immutable
Iceberg metadata values use the core's complete equality, ordering, and stable
hash identities, with exact copy, repr, and pickle round-trips. Iceberg
`ScanPlan` is a self-contained five-count report, so it supports the same value
protocols without retaining a table or executable scan. `IcebergOptions` is
mutable until first hashed; hashing locks every setter, while copied and
unpickled options are independent unlocked values. Lazy Avro blocks, bound
expressions/statements, and generic `RecordOptions` deliberately remain
unhashable because the core defines no complete canonical value identity for
their operational state.
Bare `DataType.variant()` is the self-describing semi-structured Variant
datatype; `DataType.variant(fields)` stays the dense-union sugar building the
canonical dense Union with sequential native type IDs - the parenthesis
disambiguates. `fields.variant` builds the bare Variant field,
`fields.dense_union` the union one, and explicit `fields.union` remains
available for custom IDs or sparse layout.

Start with the [Python guide](https://platob.github.io/yggdryl/extensions/python/)
and the [field classes guide](FIELDS.md).
