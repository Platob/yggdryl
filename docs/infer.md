# Type inference

The Python and Node bindings can read the **runtime type** of a value and build the
matching [typed buffer](buffer.md) for you, so simple code need not name a buffer
class. `yggdryl.infer.buffer(values)` is the entry point.

!!! note "Binding-only convenience"
    Inference lives in the **interpreted bindings only** (`CLAUDE.md` rule 13). The
    Rust core reaches its buffers through explicit generics, so it has no `infer`
    module — the Rust tab below shows the explicit constructor each inferred call
    stands in for. Inference is sugar over those constructors, never a replacement:
    reach for the explicit class when a value is ambiguous (e.g. forcing `int32`) or
    out of range.

## Mapping

The two bindings infer identically, adapting only to each language's types. Note the
one honest asymmetry: JS has no native integer type — a `number` is always a float,
so an integer buffer is inferred only from `bigint` elements.

| Value                                   | Result buffer   |
|-----------------------------------------|-----------------|
| Python `bytes`/`bytearray`, Node `Buffer` | `U8Buffer`      |
| sequence of `bool` / `boolean`          | `BooleanBuffer` |
| Python `int`, Node `bigint` (i64 range) | `I64Buffer`     |
| Python `float`, Node `number`           | `F64Buffer`     |

In Python `bool` is checked before `int` (a `bool` *is* an `int`), so a sequence of
booleans always infers a `BooleanBuffer`.

## Infer a buffer

=== "Python"

    ```python
    from yggdryl.infer import buffer
    from yggdryl.buffer import BooleanBuffer, F64Buffer, I64Buffer, U8Buffer

    assert buffer([10, 20, 30]) == I64Buffer([10, 20, 30])   # int   -> I64Buffer
    assert buffer([1.5, 2.5]) == F64Buffer([1.5, 2.5])       # float -> F64Buffer
    assert buffer([True, False]) == BooleanBuffer([True, False])
    assert buffer(b"\x01\x02\x03") == U8Buffer([1, 2, 3])    # bytes -> U8Buffer
    ```

=== "Node"

    ```js
    const { buffer } = require('yggdryl').infer
    const { I64Buffer, F64Buffer, BooleanBuffer, U8Buffer } = require('yggdryl').buffer

    console.assert(buffer([10n, 20n, 30n]).equals(new I64Buffer([10n, 20n, 30n]))) // bigint -> I64Buffer
    console.assert(buffer([1.5, 2.5]).equals(new F64Buffer([1.5, 2.5])))            // number -> F64Buffer
    console.assert(buffer([true, false]).equals(new BooleanBuffer([true, false])))
    console.assert(buffer(Buffer.from([1, 2, 3])).equals(new U8Buffer([1, 2, 3])))  // Buffer -> U8Buffer
    ```

=== "Rust"

    ```rust
    // The core has no inference: name the buffer type explicitly.
    use yggdryl_core::{BooleanBuffer, F64Buffer, I64Buffer, U8Buffer};

    let _ = I64Buffer::from_slice(&[10, 20, 30]);
    let _ = F64Buffer::from_slice(&[1.5, 2.5]);
    let _ = BooleanBuffer::from_bits(&[true, false]);
    let _ = U8Buffer::from_slice(&[1, 2, 3]);
    ```

## Guided errors

An empty sequence, a mixed sequence, an out-of-`i64`-range integer, or an unsupported
element type raises an error naming the explicit constructor to reach for instead
(`CLAUDE.md` rule 12).

=== "Python"

    ```python
    from yggdryl.infer import buffer

    try:
        buffer([])                       # nothing to infer from
    except ValueError as error:
        assert "empty sequence" in str(error)

    try:
        buffer([2**64])                  # beyond signed 64-bit
    except ValueError as error:
        assert "U64Buffer" in str(error)  # names the explicit constructor
    ```

=== "Node"

    ```js
    const { buffer } = require('yggdryl').infer

    try {
      buffer([]) // nothing to infer from
    } catch (error) {
      console.assert(/empty array/.test(error.message))
    }

    try {
      buffer([2n ** 64n]) // beyond signed 64-bit
    } catch (error) {
      console.assert(/signed 64-bit range/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    // No inference to fail: the core takes the element type from the buffer you name.
    use yggdryl_core::U64Buffer;

    let _ = U64Buffer::from_slice(&[1 << 63]); // fits an explicit unsigned buffer
    ```
