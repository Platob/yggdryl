# Arrow interoperability

`yggdryl::arrow` is the boundary where a Field meets Apache Arrow: one-row scalars, projected schemas, and streamed batches.

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field};

    let field = Field::new("symbol", DataType::Utf8, false);
    let array = field.default_arrow_array()?;

    // A scalar is one Arrow row; the exact Field beside it says what it means.
    assert_eq!(array.len(), 1);
    assert_eq!(scalar_value(&field, array.as_ref())?.as_str(), Some(""));
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

`default_arrow_array` is implemented on both `DataType` and `Field`, and it is how a canonical
default becomes a physical array. The result is a one-row array, not a row object: this crate hands
Arrow exactly two units, a `RecordBatch` and a one-row array. There is no Python
row-object I/O surface between them. Anything wider than one value is a batch, and batches are
read and written through [io.md](io.md), [ipc.md](ipc.md), and [parquet.md](parquet.md).

The three runtimes hand the scalar back in their own Arrow vocabulary. Rust hands back the bare
one-row `ArrayRef`, with `scalar_value` beside it to decode the row under its exact Field. Python
returns a `pyarrow.Scalar`, imported through the Arrow C Data Interface with the complete Field, so
a registered `ExtensionType` rehydrates rather than collapsing to its storage type. JavaScript
materializes the one-row IPC message with Apache Arrow JS and returns the value itself, which is
why an `int64` arrives as a `BigInt`.

## Nullability picks the default

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar};

    // A bare DataType projects through its own datatype default planner.
    let array = DataType::Int64.default_arrow_array()?;
    let value = Field::new("value", DataType::Int64, false);
    assert_eq!(scalar_value(&value, array.as_ref())?.as_i128(), Some(0));

    // A nullable Field defaults to a logical null under its own identity.
    let optional = Field::new("symbol", DataType::Utf8, true);
    let array = optional.default_arrow_array()?;
    assert!(array.is_null(0));
    assert_eq!(scalar_value(&optional, array.as_ref())?, Scalar::Null);

    // A required Null column has no value it could ever hold.
    let refused = Field::new("never", DataType::Null, false).default_arrow_array();
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

The two methods differ in what they are allowed to assume. `DataType` has no name, nullability, or
metadata, so its default is the datatype's own present value, materialized through a synthetic
required Field. `Field` keeps everything an array datatype cannot carry - the name, the
nullability, the dictionary options, the metadata, and the extension identity - which is why the
Field method is the one that can return a logical null.

## A struct root is one row

=== "Rust"

    ```rust
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar};

    let schema = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])?,
        false,
    );

    let row = scalar_value(&schema, schema.default_arrow_array()?.as_ref())?;
    let values = row.as_sequence().ok_or("a struct row is an ordered sequence")?;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].as_i128(), Some(0));
    assert_eq!(values[1], Scalar::Null);
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
a schema is one default row. A Rust `Scalar` for a struct is positional, which is why the row above
is indexed rather than keyed; Python and JavaScript project the same row through their own struct
scalar, keyed by name.

Everything below is the Rust surface. Python reaches the same materializer through
`DataType.arrow_scalar`, `Field.arrow_scalar`, `Field.cast_arrow_array`, and
`Field.cast_arrow_batch` over PyArrow objects ([../extensions/python.md](extensions/python.md)).
The JavaScript package exposes `defaultArrowScalar` and nothing else from this module; its
`DataType.fromArrow` and `Field.fromArrow` constructors belong to [datatype.md](datatype.md) and
[field.md](field.md) ([../extensions/javascript.md](extensions/javascript.md)).

## One scalar across the array boundary

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array};
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::{DataType, Field, Scalar};

let field = Field::new("id", DataType::Int64, false);
let array = scalar_array(&field, &Scalar::from(7_i64))?;
assert_eq!(array.len(), 1);

// The exact Field decodes the same one-row array back, unchanged.
assert_eq!(scalar_value(&field, array.as_ref())?.as_i128(), Some(7));

// A foreign array has to be one row of the Field's exact physical datatype.
let two: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
assert!(scalar_value(&field, two.as_ref()).is_err());

// And the value has to satisfy the Field, recursively.
assert!(scalar_array(&field, &Scalar::Null).is_err());
```

`scalar_array` and `scalar_value` are the two directions of one boundary: a validated native value
becomes an immutable one-row `ArrayRef`, and a one-row array that came from somewhere else decodes
back to its canonical `Scalar`. The exact `Field` beside them is the authority an array datatype
alone cannot be - it carries the name, the nullability, the dictionary options, the metadata, and
the extension identity - so both directions validate through the same schema-directed walk every
row value takes. The array is a plain `ArrayRef`, so exporting to Arrow never copies buffers.

`scalar_value` is the defensive door. It checks the length, then bounds the Field's datatype depth
*before* handing it to Arrow's recursive projection, so a malformed foreign scalar reports a schema
error instead of exhausting the native stack. A non-nullable Field may still hold a logical null,
but only when the decoded value is exactly its datatype's canonical intrinsic default; that narrow
exception is what keeps null-only dictionaries, unions, and run-end encodings closed under
`scalar_array` followed by `scalar_value`, without admitting an arbitrary selected null.

## TypedScalar is the scalar without a Field

```rust
use arrow_array::Array;
use yggdryl::generic::Int64Scalar;
use yggdryl::{DataType, TypedScalar, Scalar};

let price = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?;
let array = price.clone().into_arrow_array()?;
assert_eq!(array.len(), 1);
assert_eq!(TypedScalar::from_arrow_array(DataType::Int64, array.as_ref())?, price);

// The marker-narrowed decode checks the datatype at compile time too.
let typed = Int64Scalar::try_from_arrow_array(DataType::Int64, array.as_ref())?;
assert_eq!(typed.value(), &Scalar::I64(7));

// A null projects only when the datatype's own default spells it...
let nothing = TypedScalar::from_parts(DataType::Null, Scalar::Null)?;
assert_eq!(nothing.into_arrow_array()?.logical_null_count(), 1);

// ...an int64 null belongs to a nullable Field, so the pairing refuses it.
let absent = TypedScalar::from_parts(DataType::Int64, Scalar::Null)?;
assert!(absent.into_arrow_array().is_err());
```

A caller holding a [`TypedScalar`](generic.md) - one value and one datatype, with no Field around
them - projects the same boundary directly: `into_arrow_array` materializes one row, and
`from_arrow_array` (or the marker-narrowed `try_from_arrow_array`) decodes row zero of a one-row
array into a validated pairing. The projection runs through a synthetic non-nullable Field over the
pairing's datatype, with the same canonical-default exception the foreign-array door makes, so
null-only datatypes stay closed under the round trip while a plain null remains the business of the
nullable Field that would hold it.

## StructScalar

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Datum, Int64Array, StringArray, StructArray};
use yggdryl::arrow::StructScalar;
use yggdryl::{DataType, Field};

let schema = Field::new(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?,
    false,
);

let projected = schema.clone().into_arrow_schema()?;
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
assert_eq!(row.field().field_len(), 2);

// Children come back as zero-copy one-element slices, by position or by name.
let (field, column) = row.entry(0).expect("first column");
assert_eq!(field.name(), "id");
assert_eq!(column.len(), 1);
assert_eq!(row.get_by_name("symbol").map(|column| column.len()), Some(1));

// Arrow's own scalar marker is a shallow clone away.
let marker = row.into_arrow_scalar();
let (inner, is_scalar) = marker.get();
assert!(is_scalar);
assert_eq!(inner.len(), 1);
```

`StructScalar` is the same idea one level up: one present Arrow struct row paired with the root
Field it satisfies. `from_parts` refuses
anything that is not exactly one row, refuses a null root - a native `Scalar` cannot represent one -
and refuses a struct array whose columns are not exactly the ones the Field declares, naming both
schemas in the error.

`get`, `get_by_name`, and `entry` slice rather than copy, so reading one column out of a row costs
a slice and an `Arc` bump. `into_arrow_scalar` and `into_arrow_scalar` produce Arrow's `Scalar`
marker, the `Datum` its kernels expect when one side of a binary operation is a single value.

## Projecting a schema

```rust
use arrow_schema::{Schema, ffi::FFI_ArrowSchema};
use yggdryl::{DataType, Field};

let mut symbol = DataType::dictionary(DataType::Int16, DataType::Utf8)?
    .nullable_field("symbol");
symbol.set_dictionary_options(-7, true)?;

let schema = Field::from_parts(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        symbol,
    ])?,
    false,
    [("owner", "trading")],
)?;

// Root metadata becomes Arrow schema metadata, and comes back.
let projected = schema.clone().into_arrow_exchange_schema()?;
assert_eq!(projected.fields().len(), 2);
assert_eq!(projected.metadata().get("owner").map(String::as_str), Some("trading"));
assert_eq!(Field::from_arrow_schema("row", &projected)?, schema);

// Arrow's C schema has no dictionary-ID slot.  This is the same boundary
// PyArrow uses: the ID becomes zero, while the sidecar metadata survives.
assert_eq!(
    projected
        .metadata()
        .get("yggdryl:ipc:dictionary-ids")
        .map(String::as_str),
    Some("v1;1=-7"),
);
let ffi = FFI_ArrowSchema::try_from(&projected)?;
let crossed = Schema::try_from(&ffi)?;
#[allow(deprecated)]
{
    assert_eq!(crossed.field(1).dict_id(), Some(0));
}
let restored = Field::from_arrow_schema("row", &crossed)?;
assert_eq!(restored, schema);
assert!(!restored.has_metadata("yggdryl:ipc:dictionary-ids"));

// Field::into_arrow_schema is the shared in-process projection. It retains the ID
// on Arrow's Field directly and needs no transport sidecar.
let in_process = schema.clone().into_arrow_schema()?;
#[allow(deprecated)]
{
    assert_eq!(in_process.field(1).dict_id(), Some(-7));
}

// A root that is not a non-null Struct is refused, not coerced.
assert!(Field::new("row", DataType::Int64, false)
    .into_arrow_exchange_schema()
    .is_err());
assert!(schema.with_nullable(true).into_arrow_exchange_schema().is_err());
```

Rust's `Field::into_arrow_schema` is the shared in-process projection, so every encoding sees the
same field identifiers and root metadata. It validates that the root is bounded, non-nullable, and
a Struct. `Field::into_arrow_exchange_schema` is the exchange-safe owned projection used when a
schema crosses the Arrow C Data Interface; for every non-zero dictionary ID
it adds the reserved
`yggdryl:ipc:dictionary-ids` schema-metadata entry, keyed by deterministic numeric field paths.
The Arrow C Data Interface carries dictionary ordering but not those IDs; PyArrow crosses that
interface, preserves the metadata entry, and resets the direct ID. `Field::from_arrow_schema` - and
Python's `Field.from_arrow_schema()` - validates the entry, restores every nested ID, and strips it
before constructing root Field metadata. A malformed entry, a path to a non-dictionary field, a
conflict with a non-zero Arrow ID, or caller-owned root metadata under the reserved key is refused.
Per-field projection - `Field::into_arrow`, `Field::from_arrow`, `DataType::into_arrow` - lives in
[field.md](field.md) and [datatype.md](datatype.md).

### What schema projection costs

The `field` Criterion target measures the three Struct-root methods above over
the same nested field. Construction of the fixture stays outside the timer:

```console
cargo bench -p yggdryl --bench field --all-features -- arrow/struct_field --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```

One local Windows x86_64 release run (Criterion point estimates; regenerate on
the deployment host):

| operation | estimate | 95% interval |
| --- | ---: | ---: |
| `Field::into_arrow_schema` | 1.48 us | 1.14-2.27 us |
| `Field::into_arrow_exchange_schema` | 1.81 us | 1.71-1.95 us |
| `Field::from_arrow_schema` | 2.59 us | 2.37-2.81 us |

The exchange sidecar and its validation remain microsecond-scale even for the
nested dictionary fixture. These numbers price schema projection only; no
record batch is allocated or crossed.

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
let projected = schema.into_arrow_schema()?;

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
ipc::overwrite_arrow_reader(
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
and `IOMedia::read_arrow_reader` - and the explicit
`overwrite_arrow_reader`, `append_arrow_reader`, and `merge_arrow_reader` paths consume one.
Their canonical signatures and intent validation are documented in
[io.md](io.md#canonical-record-write-signatures). Passing a reader rather than a `Vec` leaves the
decision of how much to hold in memory with the caller, and the box owns whatever it reads from, so
it outlives the call that produced it and can be sent to another thread.

`arrow::batch_reader(schema, batches)` is the constructor for the write side. It takes any
`IntoIterator` of owned batches - a `Vec`, an array, a lazily-computed iterator - under a schema the
reader reports before yielding anything, so a generator is encoded as it produces rather than
materialized first.

## Combining two readers

`combined` chains two readers onto the root their two schemas merge into - the case where the schemas
differ and no target is known in advance. `combined_as` is the same chaining onto a root the caller
already holds. Both are **fully lazy**: a reader answers its schema without pulling a batch, so the
merge costs no rows, neither side is drained to inspect it, and nothing is collected.

The merge rules are the contract, not an accident:

- **Columns unite by name**, ASCII case-insensitively, the way column names already resolve wherever
  a cast or a selection matches them. Left's columns keep left's order; columns only in right are
  appended after, in right's order.
- **A column in both must reconcile to one datatype**, and differing datatypes are **refused**,
  naming both sides. Refusing is the honest default: a silent widening is how a decimal quietly
  becomes a float.
- **A column present in only one side becomes nullable**, necessarily - the other side's rows have no
  value for it and the cast fills null. This holds even when the column is non-nullable on its own
  side, so a caller expecting their non-null declaration to survive a merge reads it here rather than
  discovering it.
- **Metadata and field ids: left's are kept**, and a conflicting `PARQUET:field_id` is refused rather
  than silently reassigned - Iceberg cares about field identity, and a reassigned id corrupts a
  table's schema evolution.
- **The root name is left's**, and the merged root is a bounded, non-nullable Struct. Because the
  merge never widens a datatype, every column stays exactly what one of the two sides declared, so a
  merged reader is appendable to an Iceberg table wherever both inputs were.

Every cast goes through the one definition - `arrow::cast_reader` - which short-circuits a side that
is already the declared shape rather than rebuilding arrays it would hand back unchanged.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::DataType;

    let left_root = DataType::from_fields([DataType::Int64.nullable_field("id")])?
        .required_field("row");
    let right_root = DataType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");

    let left_schema = left_root.into_arrow_schema()?;
    let right_schema = right_root.into_arrow_schema()?;
    let left = yggdryl::arrow::batch_reader(
        Arc::clone(&left_schema),
        [RecordBatch::try_new(left_schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])?],
    );
    let right = yggdryl::arrow::batch_reader(
        Arc::clone(&right_schema),
        [RecordBatch::try_new(
            right_schema,
            vec![
                Arc::new(Int64Array::from(vec![2_i64])),
                Arc::new(StringArray::from(vec!["XPAR"])),
            ],
        )?],
    );

    let joined = yggdryl::arrow::combined(left, right)?;
    assert_eq!(joined.schema().fields().len(), 2);
    // The left's rows carry no `venue`, so they read null for it.
    let batches: Vec<_> = joined.collect::<Result<_, _>>()?;
    assert!(batches[0].column(1).is_null(0));
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import combined

    left = pa.table({"id": [1]})
    right = pa.table({"id": [2], "venue": ["XPAR"]})

    joined = combined(left, right).read_all()
    assert joined.column_names == ["id", "venue"]
    assert joined.num_rows == 2
    # The left's row has no `venue`, so it reads null.
    assert joined.column("venue").to_pylist() == [None, "XPAR"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { BatchReader, DataType, Field } = require('yggdryl')
    const { tableFromArrays, tableToIPC } = require('apache-arrow')

    const left = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([1n]) }))
    const right = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([2n]) }))

    const joined = left.combined(right)
    assert.equal(joined.field.dtype.length, 1)
    assert.equal(joined.intoTable().numRows, 2)
    ```


## Materialization budgets

```rust
use yggdryl::arrow::scalar_array;
use yggdryl::{DataType, Field, Scalar};

// One logical null, one million and one mandatory physical child slots.
let items = Field::new(
    "items",
    DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 1_000_001)?,
    true,
);
let message = scalar_array(&items, &Scalar::Null).unwrap_err().to_string();
assert!(message.contains("expanded slots"), "{message}");
assert!(message.contains("expected at most 1000000"), "{message}");
assert!(message.contains("got 1000001"), "{message}");

// Fixed width is counted across siblings, not per column.
let wide = DataType::from_fields([
    Field::new("left", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
    Field::new("right", DataType::fixed_size_binary(40 * 1024 * 1024)?, false),
])?;
let message = scalar_array(&Field::new("wide", wide, true), &Scalar::Null)
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
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::{DataType, Field, UnionMode, Scalar};

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

let choice = Field::new("choice", dense, false);
let chosen = Scalar::from_sequence([Scalar::from(0_i8), Scalar::from(11_i32)]);
let array = scalar_array(&choice, &chosen)?;
assert_eq!(scalar_value(&choice, array.as_ref())?, chosen);
```

The budget is charged for what is actually built. A dense union allocates only the selected member,
so an inactive branch that could never fit is not an error - it is not visited. Reservations for a
phase whose allocation cannot outlive it are released when the phase ends, so a deep tree is not
charged twice for the same temporary buffer. The same accounting runs behind `ArrowCast`, so a
batch cast that would have to invent an oversized default column is refused before it allocates;
casting itself is documented in [field.md](field.md).
