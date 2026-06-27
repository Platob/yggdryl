# yggdryl-scalar

An **atomic scalar value** for [yggdryl](https://github.com/Platob/yggdryl): a single,
type-erased value that knows its own [`DataType`](../yggdryl-schema) and round-trips
losslessly to and from an Apache **Arrow** scalar. It is the value-level companion to the
`yggdryl-schema` type layer and the `yggdryl-serie` columnar layer — a `Serie` is a
*column* of values, a `Scalar` is *one* value.

Every variant carries the full type information of its `DataType` (an `Int` keeps its
width and signedness, a `Decimal` its precision / scale / storage width, a `Timestamp`
its `TimeUnit` and optional `Timezone`), so `data_type()` reconstructs the exact logical
type. The model is parameterised, not combinatorial — one `Int` variant covers every
width.

## Arrow scalar conversion — the headline

```rust
use yggdryl_scalar::Scalar;

let value = Scalar::int(42, 64, true);

// Render as a length-1 Arrow array, or an `arrow_array::Scalar` broadcast marker.
let array = value.to_array().unwrap();
let arrow_scalar = value.to_arrow_scalar().unwrap();

// Read any Arrow array cell back into a Scalar.
assert_eq!(Scalar::from_array(array.as_ref(), 0).unwrap(), value);
```

## Serialization

Like every yggdryl value type, a `Scalar` round-trips through a canonical string
(`42::int64`, `'hi'::utf8`, `null::int64`), a component map, bytes (lossless Arrow IPC)
and — under the `serde` / `json` features — JSON, and is `Hash` + `Eq` (floats hash by a
canonical bit pattern).

## Features

| feature | what it adds |
| --- | --- |
| `serde` | structural `Serialize` / `Deserialize` (wide integers are string-encoded so JSON is exact) |
| `json` | `to_json` / `from_json` (implies `serde`) |
| `log` | off-by-default structured logging via the crate-local `log_event!` macro |

Arrow (`arrow-array` / `arrow-schema` / `arrow-buffer` / `arrow-ipc`) is a required
dependency — a scalar *is* a length-1 Arrow array.
