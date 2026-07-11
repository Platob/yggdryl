# Typed buffers

Where [`ByteBuffer`](io.md) is untyped byte storage, the **buffer** layer
(`yggdryl-buffer`) is one immutable, cheaply-shared, contiguous buffer per native
primitive — `I8Buffer` … `I64Buffer`, `U8Buffer` … `U64Buffer`, `F32Buffer`,
`F64Buffer` — plus the bit-packed `BooleanBuffer`. Each is a value type: it clones by
sharing its allocation, round-trips through little-endian bytes, and compares and
hashes by content. A buffer also carries optional headers and hands out the matching
typed [field](field.md) (see [Field & headers](#field-and-headers) below).

!!! note "Node type limits"
    Node omits `U64Buffer` (napi has no native `u64` scalar — use `I64Buffer` or raw
    bytes), and `F32Buffer` marshals its values over an `f64` boundary. `I64Buffer`
    values and the `length` / `len()` counts marshal as JS `BigInt` (wrap with
    `Number(...)`), exactly as on the [IO cursor](io.md).

## Construct and access

=== "Python"

    ```python
    from yggdryl.buffer import I32Buffer

    buf = I32Buffer([10, 20, 30])
    assert len(buf) == 3
    assert buf.get(1) == 20
    assert buf.get(3) is None          # out of bounds
    assert buf.to_list() == [10, 20, 30]
    ```

=== "Node"

    ```js
    const { I32Buffer } = require('yggdryl').buffer

    const buf = new I32Buffer([10, 20, 30])
    console.assert(Number(buf.length) === 3)
    console.assert(buf.get(1) === 20)
    console.assert(buf.get(3) === null)
    console.assert(buf.toArray().join() === '10,20,30')
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::I32Buffer;

    let buf = I32Buffer::from_slice(&[10, 20, 30]);
    assert_eq!(buf.len(), 3);
    assert_eq!(buf.get(1), Some(20));
    assert_eq!(buf.get(3), None);
    assert_eq!(buf.as_slice(), &[10, 20, 30]); // aligned, zero-copy
    ```

## Serialize to and from bytes

`serialize_bytes` emits the values' little-endian bytes; `deserialize_bytes`
validates the length against the element width and is its exact inverse — a length
that is not a whole number of values is rejected with an actionable message.

=== "Python"

    ```python
    from yggdryl.buffer import I32Buffer

    buf = I32Buffer([1, -2, 3])
    data = buf.serialize_bytes()                 # 12 little-endian bytes
    assert I32Buffer.deserialize_bytes(data) == buf

    try:
        I32Buffer.deserialize_bytes(bytes(6))    # 6 is not a multiple of 4
    except ValueError as error:
        assert "multiple of 4" in str(error)
    ```

=== "Node"

    ```js
    const { I32Buffer } = require('yggdryl').buffer

    const buf = new I32Buffer([1, -2, 3])
    const data = buf.serializeBytes()
    console.assert(I32Buffer.deserializeBytes(data).equals(buf))

    try {
      I32Buffer.deserializeBytes(Buffer.alloc(6)) // 6 is not a multiple of 4
    } catch (error) {
      console.assert(/multiple of 4/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::{BufferError, I32Buffer};

    let buf = I32Buffer::from_slice(&[1, -2, 3]);
    let data = buf.serialize_bytes(); // 12 little-endian bytes
    assert_eq!(I32Buffer::deserialize_bytes(&data).unwrap(), buf);

    assert!(matches!(
        I32Buffer::deserialize_bytes(&[0; 6]),   // 6 is not a multiple of 4
        Err(BufferError::InvalidByteLength { width: 4, .. })
    ));
    ```

## Value semantics (bitwise, so floats work)

Two buffers are equal **iff** their `serialize_bytes` are equal, and equal buffers
hash equal — comparison is by *byte content*, so the float buffers compare bitwise
(a `NaN` equals an identical `NaN`; `+0.0` and `-0.0` differ).

=== "Python"

    ```python
    import math
    from yggdryl.buffer import F64Buffer

    assert F64Buffer([math.nan]) == F64Buffer([math.nan])
    assert F64Buffer([0.0]) != F64Buffer([-0.0])
    assert len({F64Buffer([1.0]), F64Buffer([1.0])}) == 1
    ```

=== "Node"

    ```js
    const { F64Buffer } = require('yggdryl').buffer

    console.assert(new F64Buffer([NaN]).equals(new F64Buffer([NaN])))
    console.assert(!new F64Buffer([0]).equals(new F64Buffer([-0])))
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::F64Buffer;

    assert_eq!(F64Buffer::from_slice(&[f64::NAN]), F64Buffer::from_slice(&[f64::NAN]));
    assert_ne!(F64Buffer::from_slice(&[0.0]), F64Buffer::from_slice(&[-0.0]));
    ```

## Bit-packed booleans

`BooleanBuffer` packs one bit per value, LSB-first (8 values per byte), like an
Arrow validity bitmap. Unused high bits of the final byte are kept zero, so the
packed bytes are canonical.

=== "Python"

    ```python
    from yggdryl.buffer import BooleanBuffer

    bits = BooleanBuffer([True, False, True, True])   # 0b1101, LSB-first
    assert bits.get(2) is True
    assert bits.count_set_bits() == 3
    assert bits.as_bytes() == b"\x0d"
    assert BooleanBuffer.deserialize_bytes(bits.serialize_bytes()) == bits
    # only the low 3 bits of 0xFF are read for a 3-bit buffer
    assert BooleanBuffer.from_bytes(b"\xff", 3) == BooleanBuffer([True, True, True])
    ```

=== "Node"

    ```js
    const { BooleanBuffer } = require('yggdryl').buffer

    const bits = new BooleanBuffer([true, false, true, true]) // 0b1101
    console.assert(bits.get(2) === true)
    console.assert(Number(bits.countSetBits()) === 3)
    console.assert(bits.asBytes().equals(Buffer.from([0x0d])))
    console.assert(
      BooleanBuffer.fromBytes(Buffer.from([0xff]), 3)
        .equals(new BooleanBuffer([true, true, true])),
    )
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::BooleanBuffer;

    let bits = BooleanBuffer::from_bits(&[true, false, true, true]); // 0b1101
    assert_eq!(bits.get(2), Some(true));
    assert_eq!(bits.count_set_bits(), 3);
    assert_eq!(bits.as_bytes(), &[0x0d]);
    assert_eq!(
        BooleanBuffer::from_bytes(&[0xff], 3).unwrap(),
        BooleanBuffer::from_bits(&[true, true, true])
    );
    ```

## Bridge to positioned IO

`byte_cursor()` opens a [`ByteCursor`](io.md) over the values' little-endian bytes,
and `to_byte_buffer` / `from_byte_buffer` convert to and from a `ByteBuffer`.

=== "Python"

    ```python
    from yggdryl.buffer import I64Buffer
    from yggdryl.io import Whence

    cursor = I64Buffer([7, 8, 9]).byte_cursor()
    assert cursor.pread_i64_array(3, Whence.Start) == [7, 8, 9]

    buf = I64Buffer([7, 8, 9])
    assert I64Buffer.from_byte_buffer(buf.to_byte_buffer()) == buf
    ```

=== "Node"

    ```js
    const { I64Buffer } = require('yggdryl').buffer
    const { Whence } = require('yggdryl').io

    const cursor = new I64Buffer([7, 8, 9]).byteCursor()
    console.assert(cursor.preadI64Array(3, Whence.Start).map(Number).join() === '7,8,9')

    const buf = new I64Buffer([7, 8, 9])
    console.assert(I64Buffer.fromByteBuffer(buf.toByteBuffer()).equals(buf))
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::I64Buffer;
    use yggdryl_core::{IOBase, Whence};

    let mut cursor = I64Buffer::from_slice(&[7, 8, 9]).byte_cursor();
    assert_eq!(cursor.pread_i64_array(3, Whence::Start).unwrap(), [7, 8, 9]);

    let buf = I64Buffer::from_slice(&[7, 8, 9]);
    assert_eq!(I64Buffer::from_byte_buffer(&buf.to_byte_buffer()).unwrap(), buf);
    ```

## Field and headers

A buffer carries optional **headers** (a bytes→bytes map) and hands out the matching
typed [field](field.md) via `field(name, nullable)` — `I64Buffer` → `I64Field`,
`BooleanBuffer` → `BooleanField`, and so on. The headers is carried into that field.

!!! note "Idioms & Arrow"
    Headers marshals as a Python `dict[bytes, bytes]` and, in Node, an
    `Array<{key: Buffer, value: Buffer}>` (JS cannot key a map by bytes). It is an
    annotation only — it does **not** affect the buffer's byte-content equality, and it
    is **not** carried into Arrow's `Field` (arbitrary bytes are not valid UTF-8), so it
    stays yggdryl-side.

=== "Python"

    ```python
    from yggdryl.buffer import I64Buffer

    buf = I64Buffer([1, 2, 3]).with_headers({b"unit": b"ms"})
    assert buf.headers == {b"unit": b"ms"}

    field = buf.field("ts", True)              # an I64Field
    assert field.name == "ts" and field.nullable is True
    assert field.headers == {b"unit": b"ms"}

    assert buf == I64Buffer([1, 2, 3])         # headers is not part of byte identity
    ```

=== "Node"

    ```js
    const { I64Buffer } = require('yggdryl').buffer

    const entries = [{ key: Buffer.from('unit'), value: Buffer.from('ms') }]
    const buf = new I64Buffer([1, 2, 3]).withHeaders(entries)

    const field = buf.field('ts', true)        // an I64Field
    console.assert(field.name === 'ts' && field.nullable === true)
    console.assert(field.headers[0].value.equals(Buffer.from('ms')))

    console.assert(new I64Buffer([1, 2, 3]).equals(buf)) // headers off byte identity
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::I64Buffer;
    use yggdryl_field::Field;
    use yggdryl_http::{Headers, HeadersBased};

    let buf = I64Buffer::from_slice(&[1, 2, 3])
        .with_headers(Headers::from_pairs([(b"unit".to_vec(), b"ms".to_vec())]));

    let field = buf.field("ts", true); // an I64Field
    assert_eq!(field.name(), "ts");
    assert_eq!(field.headers().unwrap().get(b"unit"), Some(b"ms".as_slice()));

    assert_eq!(buf, I64Buffer::from_slice(&[1, 2, 3])); // headers off byte identity
    ```

## Zero-copy Arrow (Rust)

A numeric buffer **is** an Apache Arrow `ScalarBuffer<T>` (the core is Arrow-backed),
so it wraps one **zero-copy** — sharing the allocation — and emits one back;
`BooleanBuffer` does the same with an Arrow `BooleanBuffer`. This is Rust-only: an
`arrow_buffer` value does not cross the FFI boundary, so the bindings do not expose it
(the same choice as [`ByteBuffer`](io.md)).

```rust
use yggdryl_buffer::I64Buffer;
use yggdryl_core::arrow_buffer::ScalarBuffer; // re-exported at the matching version

let scalar = ScalarBuffer::<i64>::from(vec![1, 2, 3]);
let buffer = I64Buffer::from_arrow(scalar);   // shares the allocation
assert_eq!(buffer.as_slice(), &[1, 2, 3]);

let out = buffer.to_arrow();                   // zero-copy back (an Arc bump)
assert_eq!(out.as_ref(), &[1, 2, 3]);
```

## Benchmarks

Buffer construction, byte round-trips, and Arrow interop have throughput benchmarks
in all three surfaces (`cargo bench -p yggdryl-buffer`; `bindings/*/…/buffer.*`). See
the [report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-buffer/buffer/primitive_buffer.md).

[`I64Buffer`]: https://docs.rs/yggdryl-buffer/latest/yggdryl_buffer/struct.I64Buffer.html
[`BooleanBuffer`]: https://docs.rs/yggdryl-buffer/latest/yggdryl_buffer/struct.BooleanBuffer.html
