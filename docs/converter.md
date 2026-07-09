# Converters

A **converter** maps a source representation to a target one and back. The Rust core
exposes a small hierarchy — the FFI-opaque `Converter` (byte-array in/out) and the
typed `TypedConverter<S, T>` (`encode` / `decode`) — with concrete converters for the
identity, numeric casts, flexible string parsing, and byte / UTF-8 codecs.

Because `TypedConverter<S, T>` carries two type parameters it is **Rust-only**; the
Python and Node bindings expose the same behaviour through a **dtype-keyed facade**,
`yggdryl.converter`, that names the primitive at runtime (`"i32"`, `"f64"`, …).

!!! note "Node type limits"
    Scalars follow the usual JS mapping: the small integers and floats are `number`,
    while `i64` / `u64` are `bigint`. Pass a `bigint` when the dtype is `i64` / `u64`.

## Numeric cast

`cast` reinterprets packed little-endian bytes of one dtype as another, element by
element, using a C-style `as` cast (total and allocation-free).

=== "Python"

    ```python
    from yggdryl import converter

    data = (7).to_bytes(4, "little", signed=True)   # one i32
    wide = converter.cast(data, "i32", "i64")        # -> 8 little-endian bytes
    assert wide == (7).to_bytes(8, "little", signed=True)

    # Narrowing follows `as` truncation (258 & 0xFF == 2).
    assert converter.cast((258).to_bytes(4, "little"), "i32", "u8") == bytes([2])
    ```

=== "Node"

    ```js
    const { converter } = require('yggdryl')

    const data = Buffer.alloc(4); data.writeInt32LE(7)
    const wide = converter.cast(data, 'i32', 'i64')  // -> 8 little-endian bytes
    const expected = Buffer.alloc(8); expected.writeBigInt64LE(7n)
    console.assert(wide.equals(expected))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{CastConverter, Converter, PrimitiveType, TypedConverter};

    // Typed:
    assert_eq!(CastConverter::<i32, i64>::new().encode(7).unwrap(), 7_i64);
    // Or dtype-keyed (what the bindings call):
    let wide = PrimitiveType::I32.cast_bytes(PrimitiveType::I64, &7_i32.to_le_bytes());
    assert_eq!(wide.unwrap(), 7_i64.to_le_bytes());
    ```

## Flexible string parsing

`parse` reads a string into the dtype's scalar, trying the fastest format first and
allocating only when a value actually needs it. Integers accept decimal, `0x` / `0o` /
`0b` (any case), `+`/`-` signs, and `_` separators; floats accept decimal and
scientific forms plus `inf` / `nan`. `format` renders the scalar back.

=== "Python"

    ```python
    from yggdryl import converter

    assert converter.parse("42", "i32") == 42
    assert converter.parse("0x2A", "i32") == 42
    assert converter.parse("-1_000", "i64") == -1000
    assert converter.parse("1.5e3", "f64") == 1500.0
    assert converter.format(-7, "i16") == "-7"

    try:
        converter.parse("twelve", "i32")
    except ValueError as error:
        assert "0x-hex" in str(error)     # names the accepted formats
    ```

=== "Node"

    ```js
    const { converter } = require('yggdryl')

    console.assert(converter.parse('0x2A', 'i32') === 42)
    console.assert(converter.parse('-1_000', 'i64') === -1000n) // i64 -> bigint
    console.assert(converter.parse('1.5e3', 'f64') === 1500)
    console.assert(converter.format(-7, 'i16') === '-7')

    try {
      converter.parse('twelve', 'i32')
    } catch (error) {
      console.assert(/0x-hex/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{StringConverter, TypedConverter};

    let ints = StringConverter::<i32>::new();
    assert_eq!(ints.encode("0x2A".to_string()).unwrap(), 42);
    assert_eq!(ints.encode("-1_000".to_string()).unwrap(), -1000);
    assert_eq!(ints.decode(-7).unwrap(), "-7");
    assert!(ints.encode("twelve".to_string()).is_err());
    ```

## UTF-8

`utf8Encode` / `utf8Decode` move a string to and from its UTF-8 bytes, validating on
decode and naming the failing offset on invalid input.

=== "Python"

    ```python
    from yggdryl import converter

    assert converter.utf8_encode("café") == "café".encode()
    assert converter.utf8_decode("café".encode()) == "café"

    try:
        converter.utf8_decode(b"\xff")
    except ValueError as error:
        assert "UTF-8" in str(error)
    ```

=== "Node"

    ```js
    const { converter } = require('yggdryl')

    console.assert(converter.utf8Encode('café').equals(Buffer.from('café', 'utf8')))
    console.assert(converter.utf8Decode(Buffer.from('café', 'utf8')) === 'café')

    try {
      converter.utf8Decode(Buffer.from([0xff]))
    } catch (error) {
      console.assert(/UTF-8/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{TypedConverter, Utf8Converter};

    let codec = Utf8Converter::new();
    assert_eq!(codec.encode("café".to_string()).unwrap(), "café".as_bytes());
    assert_eq!(codec.decode("café".as_bytes().to_vec()).unwrap(), "café");
    assert!(codec.decode(vec![0xFF]).is_err());
    ```

## Rust-only converters

The core also ships [`IdentityConverter<T>`] (pass-through) and
[`BytesConverter<T>`] (a value ↔ its little-endian bytes). These are the typed
building blocks the facade is composed from; the bindings reach the same behaviour
through `cast` / `parse` / `format` and the [typed buffers](buffer.md), so they are
not replicated as separate binding calls.

```rust
use yggdryl_core::{BytesConverter, IdentityConverter, TypedConverter};

assert_eq!(IdentityConverter::<i64>::new().encode(42).unwrap(), 42);
assert_eq!(BytesConverter::<i32>::new().encode(1).unwrap(), vec![1, 0, 0, 0]);
```

[`IdentityConverter<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.IdentityConverter.html
[`BytesConverter<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.BytesConverter.html
