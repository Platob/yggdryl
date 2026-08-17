# Arrow interoperability

`yggdryl::arrow` is the boundary where a Field meets Apache Arrow: one-row scalars, projected schemas, and streamed batches.

=== "Rust"

    ```rust
    use yggdryl::arrow::DefaultArrowScalar;
    use yggdryl::{DataType, Field};

    let field = Field::new("symbol", DataType::Utf8, false);
    let scalar = field.default_arrow_scalar()?;

    // A scalar is one Arrow row plus the exact Field that owns it.
    assert_eq!(scalar.field(), &field);
    assert_eq!(scalar.data_type(), &DataType::Utf8);
    assert_eq!(scalar.array().len(), 1);
    assert_eq!(scalar.to_value()?.as_str(), Some(""));
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("symbol", "utf8", nullable=False)
    scalar = field.default_arrow_scalar()

    assert isinstance(scalar, pa.Scalar)
    assert scalar.type == pa.string()
    assert scalar.as_py() == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('symbol', 'utf8', false)
    assert.equal(field.defaultArrowScalar(), '')
    ```

`DefaultArrowScalar` is implemented for both `DataType` and `Field`, and it is how a canonical
default becomes a physical array. The result is a one-row array, not a row object: this crate hands
Arrow exactly two units, a `RecordBatch` and a one-row scalar. The row-to-Arrow conversion layer
that used to sit between them is gone - there is no record type, no row iterator, and no
`read_records`/`write_records` anywhere. Anything wider than one value is a batch, and batches are
read and written through [io.md](io.md), [ipc.md](ipc.md), and [parquet.md](parquet.md).

The three runtimes hand the scalar back in their own Arrow vocabulary. Rust keeps it wrapped in
`ArrowScalar`. Python returns a `pyarrow.Scalar`, imported through the Arrow C Data Interface with
the complete Field, so a registered `ExtensionType` rehydrates rather than collapsing to its
storage type. JavaScript materializes the one-row IPC message with Apache Arrow JS and returns the
value itself, which is why an `int64` arrives as a `BigInt`.

## Nullability picks the default

=== "Rust"

    ```rust
    use yggdryl::arrow::DefaultArrowScalar;
    use yggdryl::{DataType, Field, Value};

    // A bare DataType has no name of its own, so it borrows a required one.
    let scalar = DataType::Int64.default_arrow_scalar()?;
    assert_eq!(scalar.field().name(), "value");
    assert!(!scalar.field().is_nullable());
    assert_eq!(scalar.to_value()?.as_i128(), Some(0));

    // A nullable Field defaults to a logical null and keeps its own identity.
    let optional = Field::new("symbol", DataType::Utf8, true).default_arrow_scalar()?;
    assert!(optional.array().is_null(0));
    assert_eq!(optional.to_value()?, Value::Null);

    // A required Null column has no value it could ever hold.
    let refused = Field::new("never", DataType::Null, false).default_arrow_scalar();
    assert!(refused.is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    assert DataType("int64").default_arrow_scalar().as_py() == 0

    # A field is nullable unless you say otherwise, and its default is null.
    optional = Field("symbol", "utf8").default_arrow_scalar()
    assert not optional.is_valid
    assert optional.as_py() is None

    try:
        Field("never", "null", nullable=False).default_arrow_scalar()
    except ValueError as error:
        assert "logical-null default" in str(error)
    else:
        raise AssertionError("a required null column has no default")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    assert.equal(new DataType('int64').defaultArrowScalar(), 0n)

    // A field is nullable unless you say otherwise, and its default is null.
    assert.equal(new Field('symbol', 'utf8').defaultArrowScalar(), null)

    assert.throws(
      () => new Field('never', 'null', false).defaultArrowScalar(),
      /logical-null default/,
    )
    ```

The two implementations differ in what they are allowed to assume. `DataType` has no name,
nullability, or metadata, so it projects through a synthetic required Field called `value`.
`Field` keeps everything an array datatype cannot carry - the name, the nullability, the
dictionary options, the metadata, and the extension identity - which is why the Field
implementation is the one that can return a logical null.

## A struct root is one row

=== "Rust"

    ```rust
    use yggdryl::arrow::DefaultArrowScalar;
    use yggdryl::{DataType, Field, Value};

    let schema = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])?,
        false,
    );

    let row = schema.default_arrow_scalar()?.to_value()?;
    assert_eq!(row.len(), 2);
    assert_eq!(row.get(0).and_then(Value::as_i128), Some(0));
    assert_eq!(row.get(1), Some(&Value::Null));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "row",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), Field("symbol", "utf8")]
        ),
        nullable=False,
    )

    scalar = schema.default_arrow_scalar()
    assert scalar.as_py() == {"id": 0, "symbol": None}
    assert scalar["id"].as_py() == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('symbol', 'utf8'),
      ]),
      false,
    )

    const scalar = schema.defaultArrowScalar()
    assert.equal(scalar.id, 0n)
    assert.equal(scalar.symbol, null)
    assert.deepEqual(
      [...scalar],
      [
        ['id', 0n],
        ['symbol', null],
      ],
    )
    ```

A non-null Struct Field is the schema, described in [field.md](field.md), so the default scalar of
a schema is one default row. A Rust `Value` for a struct is positional, which is why the row above
is indexed rather than keyed; Python and JavaScript project the same row through their own struct
scalar, keyed by name.

Everything below is the Rust surface. Python reaches the same materializer through
`DataType.arrow_scalar`, `Field.arrow_scalar`, `Field.cast_arrow_array`, and
`Field.cast_arrow_batch` over PyArrow objects ([../extensions/python.md](extensions/python.md)).
The JavaScript package exposes `defaultArrowScalar` and nothing else from this module; its
`DataType.fromArrow` and `Field.fromArrow` constructors belong to [datatype.md](datatype.md) and
[field.md](field.md) ([../extensions/javascript.md](extensions/javascript.md)).

## ArrowScalar

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array};
use yggdryl::arrow::ArrowScalar;
use yggdryl::{DataType, Field, Value};

let field = Field::new("id", DataType::Int64, false);
let scalar = ArrowScalar::from_value(field.clone(), Value::from(7_i64))?;
assert_eq!(scalar.data_type(), &DataType::Int64);
assert_eq!(scalar.to_value()?.as_i128(), Some(7));

// The parts come apart and go back together unchanged.
let (field, array) = scalar.into_parts();
let rebuilt = ArrowScalar::from_parts(field, array)?;
assert_eq!(rebuilt.into_value()?.as_i128(), Some(7));

// A foreign array has to be one row of the Field's exact physical datatype.
let two: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
assert!(ArrowScalar::from_parts(Field::new("id", DataType::Int64, false), two).is_err());

// And the value has to satisfy the Field, recursively.
assert!(ArrowScalar::from_value(Field::new("id", DataType::Int64, false), Value::Null).is_err());
```

`ArrowScalar` pairs an immutable, shallow-cloneable `ArrayRef` of length one with the exact `Field`
that owns it. `from_value` materializes a validated native value; `from_parts` adopts an array that
came from somewhere else and validates it. `array` borrows, `to_array` shallow-clones, and
`into_array` and `into_parts` consume - so exporting to Arrow never copies buffers.

`from_parts` is the defensive door. It checks the length, then bounds the Field's datatype depth
*before* handing it to Arrow's recursive projection, so a malformed foreign scalar reports a schema
error instead of exhausting the native stack. A non-nullable Field may still hold a logical null,
but only when the decoded value is exactly its datatype's canonical intrinsic default; that narrow
exception is what keeps null-only dictionaries, unions, and run-end encodings closed under
`into_parts` followed by `from_parts`, without admitting an arbitrary selected null.

## StructScalar

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Datum, Int64Array, StringArray, StructArray};
use yggdryl::arrow::{StructScalar, schema_from_field};
use yggdryl::{DataType, Field};

let schema = Field::new(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?,
    false,
);

let projected = schema.to_arrow_schema()?;
let array = StructArray::new(
    projected.fields().clone(),
    vec![
        Arc::new(Int64Array::from(vec![1])) as ArrayRef,
        Arc::new(StringArray::from(vec![Some("AAPL")])) as ArrayRef,
    ],
    None,
);

let row = StructScalar::from_parts(schema, array)?;
assert_eq!(row.field().name(), "row");
assert_eq!(row.schema().field_len(), 2);

// Children come back as zero-copy one-element slices, by position or by name.
let (field, column) = row.entry(0).expect("first column");
assert_eq!(field.name(), "id");
assert_eq!(column.len(), 1);
assert_eq!(row.get_by_name("symbol").map(|column| column.len()), Some(1));

// Arrow's own scalar marker is a shallow clone away.
let marker = row.to_arrow_scalar();
let (inner, is_scalar) = marker.get();
assert!(is_scalar);
assert_eq!(inner.len(), 1);
```

`StructScalar` is the same idea one level up: one present Arrow struct row paired with the root
Field it satisfies - `schema` and `field` are two names for that one accessor. `from_parts` refuses
anything that is not exactly one row, refuses a null root - a native `Value` cannot represent one -
and refuses a struct array whose columns are not exactly the ones the Field declares, naming both
schemas in the error.

`get`, `get_by_name`, and `entry` slice rather than copy, so reading one column out of a row costs
a slice and an `Arc` bump. `to_arrow_scalar` and `into_arrow_scalar` produce Arrow's `Scalar`
marker, the `Datum` its kernels expect when one side of a binary operation is a single value.

## Projecting a schema

```rust
use yggdryl::arrow::{record_schema_from_arrow, record_schema_to_arrow, schema_from_field};
use yggdryl::{DataType, Field};

let schema = Field::from_parts(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?,
    false,
    [("owner", "trading")],
)?;

// Root metadata becomes Arrow schema metadata, and comes back.
let projected = record_schema_to_arrow(&schema)?;
assert_eq!(projected.fields().len(), 2);
assert_eq!(projected.metadata().get("owner").map(String::as_str), Some("trading"));
assert_eq!(record_schema_from_arrow("row", &projected)?, schema);

// schema_from_field is the same projection, already behind an Arc.
assert_eq!(schema.to_arrow_schema()?.as_ref(), &projected);

// A root that is not a non-null Struct is refused, not coerced.
assert!(record_schema_to_arrow(&Field::new("row", DataType::Int64, false)).is_err());
assert!(record_schema_to_arrow(&schema.with_nullable(true)).is_err());
```

`schema_from_field` is the one place a root Field becomes an `arrow_schema::Schema`, so every
encoding sees the same field identifiers and the same root metadata. It validates that the root is
bounded, non-nullable, and a Struct before projecting; `record_schema_to_arrow` is the owned form
of it and `record_schema_from_arrow` is its inverse, taking the name the Arrow schema does not
carry. Per-field projection - `Field::to_arrow`, `Field::from_arrow`, `DataType::to_arrow` - lives
in [field.md](field.md) and [datatype.md](datatype.md).

## Streaming batches

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
use yggdryl::io::Buffer;
use yggdryl::ipc::{self, IpcOptions};
use yggdryl::{DataType, Field};

let schema = Field::new(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?,
    false,
);
let projected = schema.to_arrow_schema()?;

let batch = |ids: Vec<i64>, symbols: Vec<Option<&str>>| {
    RecordBatch::try_new(
        Arc::clone(&projected),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
};

let mut handle = Buffer::new();
let options = IpcOptions::new();
ipc::write_batch_reader(
    &mut handle,
    yggdryl::arrow::batch_reader(
        Arc::clone(&projected),
        [batch(vec![1, 2], vec![Some("AAPL"), None])?, batch(vec![3], vec![None])?],
    ),
    &options,
)?;

// A BatchReader knows its schema before it yields anything.
let reader = ipc::read_batch_reader(&handle, None, &options)?;
assert_eq!(reader.schema().as_ref(), projected.as_ref());

let mut rows = 0;
for batch in reader {
    rows += batch?.num_rows();
}
assert_eq!(rows, 3);
```

`BatchReader` is `Box<dyn arrow_array::RecordBatchReader + Send>` - a schema plus an iterator of
`Result<RecordBatch>`. It is the only shape a record read or write has: every read path returns it -
`ipc::read_batch_reader`, `parquet::read_batch_reader` under the non-default `parquet` feature,
`Media::read_batch_reader`, and `IOBase::read_arrow_batch_reader` - and every write path consumes one.
Passing a reader rather than a `Vec` leaves the decision of how much to hold in memory with the
caller, and the box owns whatever it reads from, so it outlives the call that produced it and can
be sent to another thread.

`arrow::batch_reader(schema, batches)` is the constructor for the write side. It takes any
`IntoIterator` of owned batches - a `Vec`, an array, a lazily-computed iterator - under a schema the
reader reports before yielding anything, so a generator is encoded as it produces rather than
materialized first.

## Materialization budgets

```rust
use yggdryl::arrow::ArrowScalar;
use yggdryl::{DataType, Field, Value};

// One logical null, one million and one mandatory physical child slots.
let items = Field::new(
    "items",
    DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 1_000_001)?,
    true,
);
let message = ArrowScalar::from_value(items, Value::Null).unwrap_err().to_string();
assert!(message.contains("expanded slots"), "{message}");
assert!(message.contains("expected at most 1000000"), "{message}");
assert!(message.contains("got 1000001"), "{message}");

// Fixed width is counted across siblings, not per column.
let wide = DataType::from_fields([
    Field::new("left", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
    Field::new("right", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
])?;
let message = ArrowScalar::from_value(Field::new("wide", wide, true), Value::Null)
    .unwrap_err()
    .to_string();
assert!(message.contains("fixed bytes"), "{message}");
assert!(message.contains("expected at most 67108864"), "{message}");
```

A composite Arrow layout can turn one logical null into an enormous number of mandatory physical
child slots, so materialization runs against two running totals: one million expanded slots and 64
MiB of fixed buffer bytes. Both are checked *before* the allocation, and exceeding either produces
`Error::PhysicalLimit`, which names what was counted, the limit, and the count reached. Allocations
that the allocator itself refuses become `Error::Allocation`, carrying the `TryReserveError`. A
size that cannot even be expressed - an overflowing `rows * width` - fails the same way rather than
wrapping.

The totals cover what the logical value tree does not own: validity bitmaps, offset buffers, union
type-id and offset buffers, and the values a dictionary or run-end wrapper hides behind its keys.
A sparse union pays for every child at once, because sparse children are all full length.

```rust
use yggdryl::arrow::ArrowScalar;
use yggdryl::{DataType, Field, UnionMode, Value};

// The inactive branch is far past the byte budget, and is never visited.
let dense = DataType::union(
    [
        (0, Field::new("selected", DataType::Int32, false)),
        (
            1,
            Field::new(
                "inactive",
                DataType::fixed_size_binary(64 * 1024 * 1024 + 1)?,
                false,
            ),
        ),
    ],
    UnionMode::Dense,
)?;

let chosen = Value::from_sequence([Value::from(0_i8), Value::from(11_i32)]);
let scalar = ArrowScalar::from_value(Field::new("choice", dense, false), chosen.clone())?;
assert_eq!(scalar.to_value()?, chosen);
```

The budget is charged for what is actually built. A dense union allocates only the selected member,
so an inactive branch that could never fit is not an error - it is not visited. Reservations for a
phase whose allocation cannot outlive it are released when the phase ends, so a deep tree is not
charged twice for the same temporary buffer. The same accounting runs behind `ArrowCast`, so a
batch cast that would have to invent an oversized default column is refused before it allocates;
casting itself is documented in [field.md](field.md).

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/arrow-rust.ipynb){ download },
[Python](notebooks/arrow-python.ipynb){ download },
[JavaScript](notebooks/arrow-javascript.ipynb){ download }.

<!-- /notebooks -->
