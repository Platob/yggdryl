# The typed serialization layer

`typed` grows a **precise element-type system** on the one [`IOBase`](io/memory.md) byte contract:
where `io` moves bytes, `typed` gives them a type. A **`Serie`** is a typed column — many elements of
one data type over a data buffer, plus an optional validity bit buffer for nulls — and it forwards
every read, write, and reduction straight to the byte layer's **vectorized** kernels, so a typed
column is a *zero-overhead* view (a build owns only its data buffer; a reduction allocates nothing).

The layer is built from six small pieces in the Rust core — `DataType`, `Encoder`, `Decoder`,
`Reduce`, `Scalar`, and `Serie` (`Serie: Scalar`) — plus a `Field` (a column's `name` / `type` /
`nullable`, carried in a [`Headers`](headers.md) map). Implementations are split by **length ×
granularity**: `fixedbyte` (integers, floats, decimals, and the fixed-size `FixedBinary` /
`FixedUtf8`), `fixedbit` (booleans), `varbyte` (the variable-length `Binary` / `Utf8`), and the
reserved `varbit` (bit-lists). The bindings expose the column surface — a numeric `Serie`, a byte
`ByteSerie`, and their `Field` — with the element type inferred from a
[`DataTypeId`](https://platob.github.io/yggdryl/).

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

## Editing a column — set, ranges, slices

A column is mutable in place: `set(index, value)` replaces one element (re-validating a null slot),
`set_null(index)` nulls one, `set_range(start, values)` bulk-replaces a window from a list (or
`set_range_serie` from another column), and `slice(start, len)` returns a fresh sub-column. Each
bounds-checks and reports a guided error; the **`*_checked` twin skips the bounds check** for a
caller that has already validated the index (the fast path).

=== "Python"

    ```python
    from yggdryl.typed import Serie
    from yggdryl.datatype_id import DataTypeId

    col = Serie.from_values([10, 20, 30, 40, 50], DataTypeId.I32)
    col.set(1, 99)                       # replace element 1 in place
    col.set_null(2)                      # null the element at 2
    col.set_range(3, [7, 8])             # bulk-replace a window from a list
    assert col.to_list() == [10, 99, None, 7, 8]

    window = col.slice(0, 2)             # a fresh sub-column
    assert window.to_list() == [10, 99]

    col.set_checked(0, 1)                # fast path: caller guarantees 0 is in range
    assert col.get(0) == 1
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    const col = Serie.fromValues([10, 20, 30, 40, 50], DataTypeId.I32())
    col.set(1, 99)                       // replace element 1 in place
    col.setNull(2)                       // null the element at 2
    col.setRange(3, [7, 8])              // bulk-replace a window from a list
    console.assert(JSON.stringify(col.toList()) === '[10,99,null,7,8]')

    const window = col.slice(0, 2)       // a fresh sub-column
    console.assert(window.len() === 2)
    col.setChecked(0, 1)                 // fast path: caller guarantees 0 is in range
    console.assert(col.get(0) === 1)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{FixedSerie, Scalar, Serie};
    use yggdryl_core::typed::fixedbyte::Int32;

    let mut col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40, 50]);
    col.set(1, 99).unwrap();             // replace element 1 in place
    col.set_null(2).unwrap();            // null the element at 2
    col.set_range(3, &[7, 8]).unwrap();  // bulk-replace a window
    assert_eq!(col.to_options(), vec![Some(10), Some(99), None, Some(7), Some(8)]);

    let window = col.slice(0, 2);        // a fresh sub-column
    assert_eq!(window.values(), vec![10, 99]);
    col.set_checked(0, 1);               // fast path: caller guarantees 0 is in range
    assert_eq!(col.get(0), Some(1));
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

## Fixed-point decimals

A **decimal** stores a signed *unscaled integer* plus a **precision** (max significant digits) and
**scale** (decimal places) in its `Field` metadata — the value is `unscaled × 10^-scale`. Four
widths back the four native integers: `Decimal32`/`Decimal64`/`Decimal128` over `i32`/`i64`/`i128`,
and `Decimal256` over a 256-bit `I256` (Rust has no `i256`). The shared `Decimal` trait gives each a
max precision and a scale-aware format, so `to_decimal_string` places the decimal point for you.

=== "Python"

    ```python
    from yggdryl.typed import Serie
    from yggdryl.datatype_id import DataTypeId

    # Money as Decimal128 scale 2: the stored value is the unscaled integer.
    col = Serie.from_values([12345, 5, -5], DataTypeId.Decimal128).with_precision_scale(10, 2)
    assert col.get(0) == 12345                       # raw unscaled value
    assert col.to_decimal_string(0) == "123.45"      # scale-aware string
    assert col.to_decimal_string(1) == "0.05"
    assert col.field().precision() == 10 and col.field().scale() == 2
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    // Money as Decimal128 scale 2: the stored value is the unscaled integer.
    const col = Serie.fromValues([12345n, 5n, -5n], DataTypeId.Decimal128()).withPrecisionScale(10, 2)
    console.assert(col.get(0) === 12345n)              // raw unscaled value
    console.assert(col.toDecimalString(0) === '123.45') // scale-aware string
    console.assert(col.field().precision() === 10 && col.field().scale() === 2)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{Decimal, FixedSerie, Scalar};
    use yggdryl_core::typed::fixedbyte::{Decimal128, I256, Decimal256};

    // Money as Decimal128 scale 2: the stored value is the unscaled integer.
    let col = FixedSerie::<Decimal128>::from_values(&[12345, 5, -5])
        .with_precision_scale(10, 2);
    assert_eq!(col.get(0), Some(12345i128));            // raw unscaled value
    assert_eq!(col.to_decimal_string(0).as_deref(), Some("123.45"));
    assert_eq!(col.field().precision(), Some(10));

    // The 256-bit width uses the native I256 (up to 76 digits).
    assert_eq!(Decimal128::format(-5, 2), "-0.05");
    let wide = FixedSerie::<Decimal256>::from_values(&[I256::from_i128(1)]);
    assert_eq!(wide.get(0), Some(I256::from_i128(1)));
    ```

## Variable-length & fixed-size byte columns

A **byte column** stores raw bytes or UTF-8 text instead of fixed-width numbers. Two layouts share
one `VarType` descriptor: **variable-length** `Binary` / `Utf8` (an `i32` offsets buffer + a data
buffer — element *i* is `data[offsets[i]..offsets[i+1]]`, so each element sizes itself) and
**fixed-size** `FixedBinary` / `FixedUtf8` (a single data buffer at a per-column byte `width` — a
shorter value is zero-padded, a longer one truncated; the width lives in the `Field` metadata). The
bindings expose both through one `ByteSerie` class (the numeric `Serie` stays as is); a binary
element crosses as `bytes` / a `Buffer`, a UTF-8 element as `str` / a `string`.

=== "Python"

    ```python
    from yggdryl.typed import ByteSerie
    from yggdryl.datatype_id import DataTypeId

    # Variable-length UTF-8: each element sizes itself.
    words = ByteSerie.from_values(["héllo", "世界"], DataTypeId.Utf8)
    assert words.len() == 2
    assert words.get(0) == "héllo"
    assert words.width() is None                 # variable-length: no fixed width

    # Fixed-size binary, width 4: short values zero-pad, long ones truncate.
    codes = ByteSerie.from_options([b"\x01\x02", None, b"ABCDE"], DataTypeId.FixedBinary, width=4)
    assert codes.width() == 4
    assert codes.field().byte_width() == 4
    assert codes.get(0) == b"\x01\x02\x00\x00"   # zero-padded to 4
    assert codes.get(1) is None                  # the null
    assert codes.get(2) == b"ABCD"               # truncated to 4
    ```

=== "Node"

    ```javascript
    const { ByteSerie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    // Variable-length UTF-8: each element sizes itself.
    const words = ByteSerie.fromValues(['héllo', '世界'], DataTypeId.Utf8())
    console.assert(words.len() === 2)
    console.assert(words.get(0) === 'héllo')
    console.assert(words.width() === null)          // variable-length: no fixed width

    // Fixed-size binary, width 4: short values zero-pad, long ones truncate.
    const codes = ByteSerie.fromOptions(
      [Buffer.from([1, 2]), null, Buffer.from('ABCDE')], DataTypeId.FixedBinary(), 4)
    console.assert(codes.width() === 4)
    console.assert(codes.field().byteWidth() === 4)
    console.assert(codes.get(0).equals(Buffer.from([1, 2, 0, 0])))  // zero-padded to 4
    console.assert(codes.get(1) === null)           // the null
    console.assert(codes.get(2).equals(Buffer.from('ABCD')))        // truncated to 4
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{FixedBinary, FixedSizeSerie, Scalar, Utf8, VarSerie};

    // Variable-length UTF-8: element i is data[offsets[i]..offsets[i + 1]].
    let words = VarSerie::<Utf8>::from_values(&["héllo".to_string(), "世界".to_string()]);
    assert_eq!(words.len(), 2);
    assert_eq!(words.get(0).as_deref(), Some("héllo"));
    assert!(words.field().byte_width().is_none()); // variable-length: no fixed width

    // Fixed-size binary, width 4: short values zero-pad, long ones truncate.
    let codes =
        FixedSizeSerie::<FixedBinary>::from_options(4, &[Some(vec![1, 2]), None, Some(b"ABCDE".to_vec())]);
    assert_eq!(codes.width(), 4);
    assert_eq!(codes.get(0), Some(vec![1, 2, 0, 0])); // zero-padded
    assert_eq!(codes.get(1), None);                   // the null
    assert_eq!(codes.get(2), Some(b"ABCD".to_vec()));  // truncated
    ```

## More aggregations — statistics for every type

Beyond `sum` / `min` / `max` / `mean`, a numeric column reduces with `var` (population variance),
`std`, `median`, and `count_ge(threshold)` — each a single streamed, allocation-free pass (a
`median` is the one exception: it sorts a copy). And a **universal** set works on *every* column,
numbers, booleans, and byte/string alike: `count` / `valid_count`, `n_unique` (distinct non-null
values), `first_value` / `last_value`, and — for any orderable element — `min_value` / `max_value`
(the lexicographic min/max of a `utf8` or `binary` column).

=== "Python"

    ```python
    from yggdryl.typed import Serie, ByteSerie
    from yggdryl.datatype_id import DataTypeId

    col = Serie.from_values([2, 4, 4, 4, 5, 5, 7, 9], DataTypeId.I64)
    assert col.mean() == 5.0
    assert col.var() == 4.0           # population variance; std == 2.0
    assert col.std() == 2.0
    assert col.median() == 4.5        # even count -> mean of the middle two
    assert col.count_ge(5) == 4       # how many elements are >= 5
    assert col.n_unique() == 5        # {2, 4, 5, 7, 9}

    # The universal set also runs on a string column — min/max are lexicographic.
    names = ByteSerie.from_values(["banana", "apple", "cherry", "apple"], DataTypeId.Utf8)
    assert names.min_value() == "apple" and names.max_value() == "cherry"
    assert names.n_unique() == 3
    assert names.first_value() == "banana" and names.count() == 4
    ```

=== "Node"

    ```javascript
    const { Serie, ByteSerie } = require('yggdryl').typed
    const { DataTypeId } = require('yggdryl').datatype_id

    const col = Serie.fromValues([2n, 4n, 4n, 4n, 5n, 5n, 7n, 9n], DataTypeId.I64())
    console.assert(col.var() === 4.0 && col.std() === 2.0) // population variance
    console.assert(col.median() === 4.5)
    console.assert(col.countGe(5n) === 4)
    console.assert(col.nUnique() === 5)

    const names = ByteSerie.fromValues(['banana', 'apple', 'cherry', 'apple'], DataTypeId.Utf8())
    console.assert(names.minValue() === 'apple' && names.maxValue() === 'cherry')
    console.assert(names.nUnique() === 3 && names.count() === 4)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::typed::{FixedSerie, Serie, Utf8, VarSerie};
    use yggdryl_core::typed::fixedbyte::Int64;

    let col = FixedSerie::<Int64>::from_values(&[2, 4, 4, 4, 5, 5, 7, 9]);
    assert_eq!(col.var().unwrap(), Some(4.0)); // population variance
    assert_eq!(col.std().unwrap(), Some(2.0));
    assert_eq!(col.median().unwrap(), Some(4.5));
    assert_eq!(col.count_ge(5).unwrap(), 4);
    assert_eq!(col.n_unique(), 5); // Serie default (Value: Eq + Hash)

    // The universal set runs on a string column — min_value/max_value are lexicographic (Value: Ord).
    let names = VarSerie::<Utf8>::from_values(
        &["banana", "apple", "cherry", "apple"].map(str::to_string),
    );
    assert_eq!(names.min_value().as_deref(), Some("apple"));
    assert_eq!(names.max_value().as_deref(), Some("cherry"));
    assert_eq!(names.n_unique(), 3);
    ```

## Types & families

| family | types | granularity |
|---|---|---|
| `fixedbyte` | `Int8`…`UInt128`, `Float32`, `Float64`, `Decimal32`…`Decimal256`, `FixedBinary`, `FixedUtf8` | fixed length, byte-packed |
| `fixedbit` | `Bit` (bool) | fixed length, bit-packed |
| `varbyte` | `Binary`, `Utf8` | variable length (offsets + data) |
| `varbit` *(reserved)* | bit-lists | variable length, bit-packed |

A **decimal** carries precision/scale in its `Field`; `Decimal256` uses the native 256-bit `I256`. A
**fixed-size** `FixedBinary` / `FixedUtf8` carries its byte `width` in its `Field`; a **variable-length**
`Binary` / `Utf8` sizes each element through its offsets buffer.

Booleans do not reduce (`Bit` is not `Reduce`); the numeric types run `sum` / `min` / `max` / `mean`
over the source's vectorized, NaN-safe `Aggregate` kernels. A column is generic over its backing
`IOBase`, so it is in-heap, memory-mapped, or on device memory with no change to its surface — build
a `Serie` from a mapped file and it reads straight from OS pages.
