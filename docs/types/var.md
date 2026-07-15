# Typed data — variable-length

`io::var` is the sibling of [`io::fixed`](fixed.md) for types whose values are **not** a fixed
byte width. It ships the two byte families:

- **`Utf8`** — UTF-8 text (every value is validated to be valid UTF-8).
- **`Binary`** — opaque bytes (any bytes, no validation).

Both are stored Arrow-style as an `i32` **offsets** buffer over a contiguous **data** buffer,
plus an optional validity bitmap (absent when there are no nulls). Value `i` is
`data[offsets[i]..offsets[i+1]]` — so an empty value and a *null* value are genuinely different
states.

## One implementation, two kinds

Every var type is generic over a [`VarElement`] marker (`Utf8` / `Binary`), the way the fixed
primitives are generic over a `NativeType`. So one implementation backs both kinds, and each
gets a friendly alias:

| root trait ([`io`](fixed.md)) | var sub-trait | concrete | `Utf8` alias | `Binary` alias |
| --- | --- | --- | --- | --- |
| `DataType` | `VarDataType` | `ByteType<E>` | `Utf8DataType` | `BinaryDataType` |
| `FieldType` | `VarField` | `ByteField<E>` | `Utf8Field` | `BinaryField` |
| `ScalarType` | `VarScalar` | `ByteScalar<E>` | `Utf8Scalar` | `BinaryScalar` |
| `SerieType` | `VarSerie` | `ByteSerie<E>` | `Utf8Serie` | `BinarySerie` |

The **root traits** (`DataType`, `ScalarType`, …) are shared with the fixed family and live at
the [`io`](fixed.md) root; `io::var` only adds the `Var*` sub-traits and the concrete types.
There is no `VarBuffer` — a variable column's storage is the offsets + data inside its
`ByteSerie`; the raw data buffer, when needed on its own, is just [`Bytes`](../guide/io.md).

## Scalars — one nullable value

`Utf8Scalar` / `BinaryScalar` hold one nullable value. A `Utf8` value is validated on every
input path, so `as_str()` never re-checks and never allocates. Every scalar round-trips through
the [`IOCursor`](../guide/io.md) byte codec.

```rust
use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar};
use yggdryl_core::io::{Bytes, IOCursor};

let s = Utf8Scalar::of("héllo");            // multi-byte code points
assert_eq!(s.as_str(), Some("héllo"));
assert!(Utf8Scalar::null().is_null());

// Arbitrary bytes are fine as Binary...
assert!(BinaryScalar::of(&[0xff, 0x00]).value_bytes().is_some());
// ...but rejected as Utf8, with a guided error.
assert!(Utf8Scalar::from_bytes(&[0xff]).is_err());

// Round-trips through any byte sink.
let mut sink = Bytes::new();
s.write_to(&mut sink).unwrap();
sink.rewind();
assert_eq!(Utf8Scalar::read_from(&mut sink).unwrap(), s);
```

## Series — a nullable column

`Utf8Serie` / `BinarySerie` are Arrow-style columns. `get_str` / `get_bytes` hand back a
**zero-copy borrow** into the data buffer. The empty-string-vs-null distinction is preserved
end to end, including across serialization.

```rust
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{Bytes, IOCursor};

let mut col = Utf8Serie::new();
col.push_str(Some("a"));
col.push_str(None);        // null
col.push_str(Some(""));    // present, empty — NOT null
col.push_str(Some("cd"));

assert_eq!(col.len(), 4);
assert_eq!(col.null_count(), 1);
assert_eq!(col.get_str(1), None);       // the null
assert_eq!(col.get_str(2), Some(""));   // the empty string

// Build from a slice of options, and round-trip through a byte sink.
let col = Utf8Serie::from_strs(&[Some("α"), None, Some("ω")]);
let mut sink = Bytes::new();
col.write_to(&mut sink).unwrap();
sink.rewind();
assert_eq!(Utf8Serie::read_from(&mut sink).unwrap(), col);
```

For binary, `from_byte_values` takes `&[Option<&[u8]>]`; the same offsets + validity layout and
round-trip apply.

## In Python and Node

`Utf8Scalar` / `Utf8Serie` and `BinaryScalar` / `BinarySerie` are classes under `yggdryl.types`.
A **UTF-8** value crosses as a `str`; a **binary** value as `bytes` (Python) / a `Buffer` (Node).
A `Scalar` is an **immutable value** (hashable/equatable, pickles/`serializeBytes` through its
byte codec); a `Serie` is a **mutable column** (unhashable) whose per-element `set` may rewrite
trailing offsets. `None` / `null` is a null (distinct from an empty value), and invalid UTF-8 is
a guided error.

=== "Python"

    ```python
    from yggdryl.types import Utf8Scalar, Utf8Serie, BinaryScalar

    s = Utf8Scalar("héllo")
    assert s.value == "héllo" and not s.is_null and s.type_name == "utf8"
    assert Utf8Scalar.deserialize_bytes(s.serialize_bytes()) == s   # byte codec

    col = Utf8Serie(["a", None, "cd"])
    assert len(col) == 3 and col.null_count == 1
    assert col.to_options() == ["a", None, "cd"] and col[0] == "a"
    col.set(1, "longer")                                            # grows -> offsets shift
    assert col.to_options() == ["a", "longer", "cd"]
    assert col.get_scalar(0) == Utf8Scalar("a")

    assert BinaryScalar(bytes([0xff, 0x00])).value == b"\xff\x00"   # any bytes are valid
    ```

=== "Node"

    ```js
    const { Utf8Scalar, Utf8Serie, BinaryScalar } = require('yggdryl').types

    const s = new Utf8Scalar('héllo')
    assert(s.value === 'héllo' && !s.isNull && s.typeName === 'utf8')
    assert(Utf8Scalar.deserializeBytes(s.serializeBytes()).equals(s))   // byte codec

    const col = new Utf8Serie(['a', null, 'cd'])
    assert(col.length === 3 && col.nullCount === 1)
    assert(col.get(0) === 'a')
    col.set(1, 'longer')                                                // grows -> offsets shift
    assert(JSON.stringify(col.toOptions()) === JSON.stringify(['a', 'longer', 'cd']))
    assert(col.getScalar(0).equals(new Utf8Scalar('a')))

    assert(new BinaryScalar(Buffer.from([0xff, 0x00])).value.equals(Buffer.from([0xff, 0x00])))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar, Utf8Serie};

    let s = Utf8Scalar::of("héllo");
    assert_eq!(Utf8Scalar::deserialize_bytes(&s.serialize_bytes()).unwrap(), s);

    let mut col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
    col.set_str(1, Some("longer")).unwrap();
    assert_eq!(col.get_str(0), Some("a"));
    assert_eq!(col.get_scalar(0), Utf8Scalar::of("a"));

    assert!(BinaryScalar::of(&[0xff, 0x00]).value_bytes().is_some());
    ```

## In-place set — the offset rewrite

Like every `Serie`, a variable-length column can overwrite an existing element — `set_str` /
`set_bytes` for one index, and the bulk `set_range` / `set_scalars` / `set_byte_values`. But unlike
the fixed-width columns, a value here can change **length**, so `set` is deliberately **expensive**:
when the new length differs it splices the data buffer and shifts *every* trailing offset (an O(n)
rewrite). A `None` (or empty value) shrinks the slot to zero bytes. This is the price of an
in-place update on the offsets + data layout — for replacing most of a column, build a fresh one.

```rust
use yggdryl_core::io::var::Utf8Serie;

let mut col = Utf8Serie::from_strs(&[Some("a"), Some("bb"), Some("ccc")]);
col.set_str(1, Some("longer")).unwrap(); // grows -> trailing offsets shift up
col.set_str(2, None).unwrap();            // -> null, slot shrinks to empty
assert_eq!(col.to_strs(), [Some("a"), Some("longer"), None]);
// The rewritten offsets stay valid — a serialize/deserialize round-trip reproduces the column.
assert!(col.get_str(1) == Some("longer"));
```

A same-length overwrite skips the offset shift (it patches the bytes in place); both are
allocation-free. See the [access benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/access.md).

## Category drill-down

Like every descriptor, a var type reports a single [`DataTypeCategory`](fixed.md) and answers
the `is_*` predicates by forwarding to it — so it classifies uniformly with the fixed types,
with no `match`:

```rust
use yggdryl_core::io::DataType;
use yggdryl_core::io::var::{ByteType, Utf8};

let dt = ByteType::<Utf8>::new();
assert_eq!(dt.name(), "utf8");
assert!(dt.is_utf8() && dt.is_variable_length());
assert!(!dt.is_fixed_width() && !dt.is_numeric());
assert_eq!(dt.byte_width(), 4);   // the fixed portion is one 32-bit offset
```

## Value semantics + performance

Scalars are `Eq` + `Hash` + serializable (so they work as map keys, in sets, and over a wire),
and columns are `Eq` + serializable. The `get_str` / `get_bytes` / `value_bytes` accessors are
**zero-copy** (a borrow, 0 allocations), and serialization packs the header + all offsets into
one pre-sized buffer so writing a column to a copy-on-write sink reallocates a constant number
of times, not once per element — see the
[benchmark notes](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/var.md).
