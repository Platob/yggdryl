# Scalar

A `Scalar` is a single **atomic value** that carries its own
[`DataType`](../schema/datatype.md) and round-trips losslessly to and from an Apache
**Arrow** scalar. It is the value-level companion to the [schema](../schema/datatype.md)
type layer and the [serie](../serie/serie.md) columnar layer — where a `Serie` is a
*column* of values, a `Scalar` is *one* value.

Every variant pins the full type information of its `DataType`: an integer keeps its
width and signedness, a decimal its precision / scale / storage width, a timestamp its
[`TimeUnit`](../core/time.md) and optional [`Timezone`](../core/time.md). So
`data_type()` reconstructs the exact logical type, and — like the schema layer — the
model is **parameterised, not combinatorial** (one integer variant covers every width).

!!! note "Available in all three languages"
    The scalar API is surfaced in **Python** and **Node** as a single `Scalar` class
    (build from a value, read `value` / `data_type`, serialise through `to_str` /
    `to_bytes`) as well as the Rust core. The Rust API additionally exposes the Arrow
    `to_array` / `from_array` conversion and the typed variant enum directly.

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Scalar(42)                       # type inferred: int64
    assert str(s.data_type) == "int64"
    assert s.value == 42
    assert s.to_str() == "42::int64"
    assert yggdryl.Scalar.from_str("42::int64") == s

    n = yggdryl.Scalar.null("float64")           # a typed null
    assert n.is_null and n.value is None

    # lossless Arrow-IPC bytes round-trip (also backs pickle)
    assert yggdryl.Scalar.from_bytes(s.to_bytes()) == s
    ```

=== "Node"

    ```javascript
    const { Scalar } = require('yggdryl')

    const s = new Scalar(42)                       // type inferred: int64
    if (s.dataType.toString() !== 'int64') throw new Error('type')
    if (s.value !== 42) throw new Error('value')
    if (s.toStr() !== '42::int64') throw new Error('str')
    if (!Scalar.fromStr('42::int64').equals(s)) throw new Error('roundtrip')

    const blob = Scalar.binary(Buffer.from('xy')) // bytes via the typed factory
    if (!Scalar.fromBytes(blob.toBytes()).equals(blob)) throw new Error('bytes')
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{DataType, Scalar};

    let s = Scalar::int(42, 64, true);
    assert_eq!(s.data_type(), DataType::int(64, true));
    assert_eq!(s.to_str(), "42::int64");

    // Render as a length-1 Arrow array and read it back.
    let array = s.to_array().unwrap();
    assert_eq!(Scalar::from_array(array.as_ref(), 0).unwrap(), s);
    ```

## Building a scalar

| from | how |
| --- | --- |
| a native value | `Scalar(value[, dtype])` — the type is inferred (`bool` / `int` → int64 / `float` → float64 / `str` → utf8 / `bytes` → binary), or built to an explicit `dtype` |
| a typed null | `Scalar.null(dtype)` — a null still knows its column type |
| a canonical string | `Scalar.from_str("42::int64")` |
| a component map | `Scalar.from_mapping({"type": ..., "value": ...})` |
| Arrow-IPC bytes | `Scalar.from_bytes(...)` |

In **Rust** the typed builders ([`int`](https://docs.rs/yggdryl-scalar), `float`,
`utf8`, `decimal128`, `from_datetime`, …) and `From` impls construct directly; the
public variant enum can also be matched and built by hand.

## Arrow scalar conversion

The headline capability is total conversion with Arrow, in the Rust core:

- `to_array()` renders the value as a **length-1 `ArrayRef`** of its `DataType`'s Arrow
  type;
- `to_arrow_scalar()` wraps that in an `arrow_array::Scalar` — the broadcast marker
  Arrow's compute kernels treat as a single value;
- `Scalar::from_array(array, index)` reads any Arrow array cell back into a `Scalar`,
  and `from_arrow_scalar(...)` reads an `arrow_array::Scalar`.

A null cell (or an out-of-bounds index) reads back as a **typed null** of the array's
type. Logical refinements the Arrow type system does not carry are **normalised** on the
round-trip, exactly as in the [schema layer](../schema/datatype.md)'s Arrow conversion: a
`Json` value reads back as a `Utf8`, a fixed-size string loses its length, a non-UTF-8
charset maps to UTF-8. For a round-trip that keeps the exact logical type, use `to_str`
or the structural serde JSON.

## Reading the value

`value` returns the native value: `None` / `bool` / `int` / `float` / `str` / `bytes`
for the primitive families; the [`Date`](../core/time.md) / `Time` / `DateTime` /
`Duration` types for temporals (an ISO string in Node); a scaled decimal string for
decimals; and a `list` / `dict` (array / object) for the nested list / struct / map
types. The typed accessors `as_bool` / `as_int` / `as_float` / `as_str` / `as_bytes`
return the value only when it is of that kind.

## Serialization

As with every yggdryl value type, a `Scalar` round-trips through:

- a **canonical string** — `to_str` / `from_str` (`"42::int64"`, `"'hi'::utf8"`,
  `"null::int64"`), lossless for the atomic types;
- a **component map** — `to_mapping` / `from_mapping` (`{"type", "value"}`);
- **bytes** — `to_bytes` / `from_bytes`, the lossless Arrow-IPC interchange form the
  bindings' pickle / `toJSON` use;
- and, in Rust under the `serde` / `json` features, **JSON** (with wide integers
  string-encoded so the value is exact).

A `Scalar` is `Hash` + `Eq`, so it can key a map or set: floats hash by a canonical bit
pattern, so `-0.0` equals `0.0` and every `NaN` is equal (and hashes the same).
