# The typed serialization layer

`typed` grows a **precise element-type system** on the one [`IOBase`](io/memory.md) byte contract:
where `io` moves bytes, `typed` gives them a type. A **`Serie`** is a typed column — many elements of
one data type over a data buffer, plus an optional validity bit buffer for nulls — and it forwards
every read, write, and reduction straight to the byte layer's **vectorized** kernels, so a typed
column is a *zero-overhead* view (a build owns only its data buffer; a reduction allocates nothing).

The layer is built from six small pieces in the Rust core — `DataType`, `Encoder`, `Decoder`,
`Reduce`, `Scalar`, and `Serie` (`Serie: Scalar`) — plus a `Field` (a column's `name` / `type` /
`nullable`, carried in a [`Headers`](headers.md) map). Implementations are split by **length ×
granularity**: `fixedbyte` (integers, floats), `fixedbit` (booleans), and the reserved `varbyte` /
`varbit` (strings, binary). The bindings expose the column surface — a `Serie` and its `Field` —
with the element type inferred from a [`DataTypeId`](https://platob.github.io/yggdryl/).

## Build a column and reduce it

A `Serie` is built from a list of values (or options, for nulls); its aggregations run on the byte
layer's allocation-free vectorized kernels.

=== "Python"

    ```python
    from yggdryl.typed import Serie
    from yggdryl.datatype_id import DataTypeId

    col = Serie.from_values([4, 8, 15, 16, 23, 42], DataTypeId.I64)
    assert col.len() == 6
    assert col.get(0) == 4
    assert col.to_list() == [4, 8, 15, 16, 23, 42]
    assert col.sum() == 108          # vectorized reduction over the data buffer
    assert col.min() == 4 and col.max() == 42
    assert col.mean() == 18.0
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    const col = Serie.fromValues([4n, 8n, 15n, 16n, 23n, 42n], DataTypeId.I64())
    console.assert(col.len() === 6)
    console.assert(col.get(0) === 4n)
    console.assert(col.sum() === 108n)   // vectorized reduction over the data buffer
    console.assert(col.min() === 4n && col.max() === 42n)
    console.assert(col.mean() === 18.0)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{FixedSerie, Scalar};
    use yggdryl_core::typed::fixedbyte::Int64;

    let col = FixedSerie::<Int64>::from_values(&[4, 8, 15, 16, 23, 42]);
    assert_eq!(col.len(), 6);
    assert_eq!(col.get(0), Some(4));
    assert_eq!(col.values(), vec![4, 8, 15, 16, 23, 42]);
    assert_eq!(col.sum().unwrap(), 108i128); // vectorized reduction
    assert_eq!(col.max().unwrap(), Some(42));
    assert_eq!(col.mean().unwrap(), Some(18.0));
    ```

## Nulls — a nullable column

Building from options (with `None` / `null`) creates the validity bitmap; `get` is null-aware and
`null_count` counts the gaps.

=== "Python"

    ```python
    from yggdryl.typed import Serie
    from yggdryl.datatype_id import DataTypeId

    col = Serie.from_options([1, None, 3, None, 5], DataTypeId.I32)
    assert col.len() == 5
    assert col.null_count() == 2
    assert col.get(0) == 1
    assert col.get(1) is None          # the null
    assert col.is_null(1) and col.is_valid(0)
    assert col.to_list() == [1, None, 3, None, 5]
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    const col = Serie.fromOptions([1, null, 3, null, 5], DataTypeId.I32())
    console.assert(col.len() === 5)
    console.assert(col.nullCount() === 2)
    console.assert(col.get(1) === null)   // the null
    console.assert(col.isNull(1) && col.isValid(0))
    console.assert(JSON.stringify(col.toList()) === '[1,null,3,null,5]')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{FixedSerie, Scalar, Serie};
    use yggdryl_core::typed::fixedbyte::Int32;

    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    assert_eq!(col.len(), 5);
    assert_eq!(col.null_count(), 2);
    assert_eq!(col.get(1), None);           // the null
    assert!(col.is_null(1) && col.is_valid(0));
    assert_eq!(col.to_options(), vec![Some(1), None, Some(3), None, Some(5)]);
    ```

## A column's `Field` — its metadata

A `Field` describes a column: its `name`, element type, and nullability — three entries in a
[`Headers`](headers.md) map, so a field serializes and travels like any metadata.

=== "Python"

    ```python
    from yggdryl.typed import Serie, Field
    from yggdryl.datatype_id import DataTypeId

    field = Field("price", DataTypeId.I64, nullable=True)
    assert field.name() == "price"
    assert field.dtype() == DataTypeId.I64
    assert field.nullable()

    col = Serie.from_values([1, 2, 3], DataTypeId.I64).with_name("id")
    assert col.field().name() == "id"
    assert col.field().nullable() is False   # no nulls -> non-nullable
    ```

=== "Node"

    ```javascript
    const { Serie, Field } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    const field = new Field('price', DataTypeId.I64(), true)
    console.assert(field.name() === 'price')
    console.assert(field.dtype().equals(DataTypeId.I64()))
    console.assert(field.nullable())

    const col = Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()).withName('id')
    console.assert(col.field().name() === 'id')
    console.assert(col.field().nullable() === false)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{Field, FixedSerie, HeaderField};
    use yggdryl_core::typed::fixedbyte::Int64;
    use yggdryl_core::datatype_id::DataTypeId;

    let field = HeaderField::new(Some("price"), DataTypeId::I64, true);
    assert_eq!(field.name(), Some("price"));
    assert_eq!(field.data_type_id(), DataTypeId::I64);
    assert!(field.nullable());

    let col = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("id");
    assert_eq!(col.field().name(), Some("id"));
    assert!(!col.field().nullable()); // no nulls -> non-nullable
    ```

## Types & families

| family | types | granularity |
|---|---|---|
| `fixedbyte` | `Int8`…`UInt128`, `Float32`, `Float64` | fixed length, byte-packed |
| `fixedbit` | `Bit` (bool) | fixed length, bit-packed |
| `varbyte` *(reserved)* | `Utf8`, `Binary` | variable length (offsets + data) |
| `varbit` *(reserved)* | bit-lists | variable length, bit-packed |

Booleans do not reduce (`Bit` is not `Reduce`); the numeric types run `sum` / `min` / `max` / `mean`
over the source's vectorized, NaN-safe `Aggregate` kernels. A column is generic over its backing
`IOBase`, so it is in-heap, memory-mapped, or on device memory with no change to its surface — build
a `Serie` from a mapped file and it reads straight from OS pages.
