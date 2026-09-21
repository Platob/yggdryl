# Strings & bytes

Two families whose value is a run of bytes: one string family in eighteen leaves, one byte family in six, and one rule holding both - the leaf is the whole declaration.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::String(StringType)` and `DataType::Bytes(BytesType)`, and the values `Str` and `Bytes` they store |
| Constructors | `DataType::string` and `DataType::bytes` take the whole declaration; one constructor per leaf picks it once - `utf8`, `large_utf8`, `utf8_view`, `large_utf8_view`, `fixed_utf8(n)`, `sized_utf8(n)`, the same six under `ascii` and `cp1252`, and `binary`, `large_binary`, `binary_view`, `large_binary_view`, `fixed_binary(n)`, `sized_binary(n)` |
| Validates | At construction, the number: a leaf that *is* a number does not stand without one, the other four of each shape refuse one, and zero is refused. At the value door, the charset and the bound |
| Lazy | Nothing - a leaf is a copy value with no registry, no child and no deferred parse |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field and shared by its clones |
| Reads back | `string_parameters`, `bytes_parameters`, `charset`, `fixed_byte_width`, `is_string` |
| Kinds | `DataTypeKind::Text` for every string leaf, `DataTypeKind::Bytes` for every byte leaf; ids `0x51`-`0x62` and `0x41`-`0x46`, laid out by family because `as_u8` is a wire contract |
| Refuses | A number beside a leaf that carries none, a leaf that is its number stated without one, a charset with no leaf, a charset beside a charset-named spelling, and a value the leaf's charset or bound cannot hold |
| Bindings | `Str` and `Bytes` are Rust only: Python and JavaScript read a value as a [`Scalar`](../scalar.md) and the declaration back as the frozen `StringParameters` / `BytesParameters` |

## Pages

| page | owns |
| --- | --- |
| [String](string.md) | The eighteen leaves, their spellings and their number rule, `Str`, the charsets, `StringEnum`, the Arrow document, the casts and the regex-capture schema |
| [Bytes](bytes.md) | The six leaves, `Bytes`, the Arrow storage a payload rides, and the bounded-column cast |

Canonical text values are not strings: each parses, canonicalizes and orders itself, so each is its own datatype rather than prose that happens to look like one - [Version](../version.md), [MIME and media types](../mediatype.md), [time zones](../temporal/timezone.md), the `url` and `urn` leaves a column of locations and names declares ([URI](../../uri/index.md#as-a-column)), and the twelve [registered codes](../codes/index.md), which are an identity over a published registry rather than a repertoire. A [UUID](../uuid.md) and a [geospatial](../geospatial/index.md) value are bytes with an identity, so they answer no `bytes_parameters`, exactly as a code answers no `string_parameters`.

## The eighteen string leaves

A string column is one of eighteen leaves: six shapes in each of the three
[charsets](../../charset/index.md) that have a datatype - UTF-8, US-ASCII and
windows-1252. The leaf is the whole declaration. It says the charset its bytes
are written in, the shape Arrow lays them out in, and - on the two numbered
shapes - what its number means: `fixed_*(n)` is an exact width, NUL-padded,
and `sized_*(n)` a maximum, so neither stands without a number and the other
four refuse one. A leaf's canonical name is its `DataTypeId`, and its number
sits beside it in the table.

| shape | number | UTF-8 | US-ASCII | windows-1252 | Arrow storage (text / binary) |
| --- | --- | --- | --- | --- | --- |
| 32-bit offsets | none | `utf8` (`0x51`) | `ascii` (`0x57`) | `cp1252` (`0x5d`) | `Utf8` / `Binary` |
| 64-bit offsets | none | `large_utf8` (`0x52`) | `large_ascii` (`0x58`) | `large_cp1252` (`0x5e`) | `LargeUtf8` / `LargeBinary` |
| view | none | `utf8_view` (`0x53`) | `ascii_view` (`0x59`) | `cp1252_view` (`0x5f`) | `Utf8View` / `BinaryView` |
| view, 64-bit offsets | none | `large_utf8_view` (`0x54`) | `large_ascii_view` (`0x5a`) | `large_cp1252_view` (`0x60`) | `Utf8View` / `BinaryView` |
| fixed width | the exact width, required | `fixed_utf8(n)` (`0x55`) | `fixed_ascii(n)` (`0x5b`) | `fixed_cp1252(n)` (`0x61`) | `FixedSizeBinary(n)` |
| bounded | the maximum, required | `sized_utf8(n)` (`0x56`) | `sized_ascii(n)` (`0x5c`) | `sized_cp1252(n)` (`0x62`) | `Utf8` / `Binary` |

## The six byte leaves

A byte column is one of six leaves, and the leaf is the whole declaration in
the same way. Four of them stand alone; the other two *are* a number -
`fixed_binary(n)` is an exact width and `sized_binary(n)` a maximum. Bytes are
never padded, so a fixed value is exactly its width.

| shape | leaf | number | Arrow storage |
| --- | --- | --- | --- |
| 32-bit offsets | `binary` (`0x41`) | none | `Binary` |
| 64-bit offsets | `large_binary` (`0x42`) | none | `LargeBinary` |
| view | `binary_view` (`0x43`) | none | `BinaryView` |
| view, 64-bit offsets | `large_binary_view` (`0x44`) | none | `BinaryView` |
| fixed width | `fixed_binary(n)` (`0x45`) | the exact width, required | `FixedSizeBinary(n)` |
| bounded | `sized_binary(n)` (`0x46`) | the maximum, required | `Binary` |

## What the two families share

One declaration read back one way. `utf8(32)` and `binary(16)` are the sized
leaf written short, because plain storage is exactly what a bounded column
fills; a large or a viewed leaf says so rather than silently becoming
something narrower.

=== "Rust"

    ```rust
    use yggdryl::{BytesType, StringType};
    use yggdryl::{Charset, DataType, DataTypeKind};

    // A string reads back as its leaf: charset, shape and number.
    let latin = DataType::from_str("string(windows-1252,32)")?;
    assert_eq!(latin, DataType::sized_cp1252(32)?);
    assert_eq!(latin.to_string(), "sized_cp1252(32)");
    assert_eq!(latin.string_parameters(), Some(StringType::SizedCp1252String(32)));
    assert_eq!(latin.charset(), Some(Charset::Cp1252));
    assert_eq!(latin.kind(), DataTypeKind::Text);

    // A byte column reads back the same way, with no charset to answer.
    let bounded = DataType::from_str("varbinary(16)")?;
    assert_eq!(bounded.to_string(), "sized_binary(16)");
    assert_eq!(bounded.bytes_parameters(), Some(BytesType::SizedBinary(16)));
    assert_eq!(bounded.kind(), DataTypeKind::Bytes);
    assert_eq!(bounded.charset(), None);

    // The number is the leaf: a width on a fixed one, a maximum on a sized one.
    assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    assert_eq!(DataType::fixed_binary(16)?.fixed_byte_width(), Some(16));
    assert_eq!(DataType::ascii().fixed_byte_width(), None);
    assert!(DataType::from_str("large_utf8(64)").is_err());
    assert!(DataType::from_str("large_binary(16)").is_err());

    // Neither family answers for the other.
    assert!(DataType::binary().string_parameters().is_none());
    assert!(DataType::utf8().bytes_parameters().is_none());
    ```

=== "Python"

    ```python
    from yggdryl import BytesParameters, DataType, StringParameters

    # A string reads back as its leaf: charset, shape and number.
    latin = DataType.string(charset="windows-1252", bound=32)
    assert str(latin) == "sized_cp1252(32)"
    assert latin == DataType("sized_cp1252(32)")
    assert latin.string_parameters == StringParameters("string", "windows-1252", 32)
    assert latin.string_parameters.layout == "sized_cp1252"
    assert latin.charset == "windows-1252"
    assert latin.kind == "text"

    # A byte column reads back the same way, with no charset to answer.
    bounded = DataType("varbinary(16)")
    assert str(bounded) == "sized_binary(16)"
    assert bounded.bytes_parameters == BytesParameters("sized_binary", 16)
    assert bounded.kind == "bytes"

    # The number is the leaf: a width on a fixed one, a maximum on a sized one.
    assert DataType.fixed_ascii(4).fixed_byte_width == 4
    assert DataType.fixed_size_binary(16).fixed_byte_width == 16
    assert DataType.ascii().fixed_byte_width is None

    # Neither family answers for the other.
    assert DataType.binary().string_parameters is None
    assert DataType.utf8().bytes_parameters is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // A string reads back as its leaf: charset, shape and number.
    const latin = DataType.string({ charset: 'windows-1252', max: 32 })
    assert.equal(latin.toString(), 'sized_cp1252(32)')
    assert.deepEqual(latin.stringParameters, {
      layout: 'sized_cp1252',
      charset: 'windows-1252',
      bound: 32,
      max: 32,
    })
    assert.equal(latin.charset, 'windows-1252')
    assert.equal(latin.kind, 'text')

    // A byte column reads back the same way, with no charset to answer.
    const bounded = DataType.from('varbinary(16)')
    assert.equal(bounded.toString(), 'sized_binary(16)')
    assert.deepEqual(bounded.bytesParameters, { layout: 'sized_binary', bound: 16, max: 16 })
    assert.equal(bounded.kind, 'bytes')

    // The number is the leaf: a width on a fixed one, a maximum on a sized one.
    assert.equal(DataType.fixedAscii(4).fixedByteWidth, 4)
    assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)
    assert.equal(DataType.ascii().fixedByteWidth, null)

    // Neither family answers for the other.
    assert.equal(DataType.binary().stringParameters, null)
    assert.equal(DataType.utf8().bytesParameters, null)
    ```

## Edges

- A leaf that *is* its number - `fixed_utf8`, `sized_ascii`, `fixed_binary` - stated with none -> refused; a bound of `0` -> refused, `at least one byte, got 0`.
- `utf8(32)`, `ascii(4)`, `cp1252(32)`, `binary(16)` -> the sized leaf written short; `large_utf8(64)`, `utf8_view(8)`, `large_binary(16)` -> refused, a large or view leaf holds no maximum.
- A value never carries a maximum: the `dtype` of a cell read out of `sized_utf8(32)` is `utf8`, of `sized_binary(16)` is `binary`.
- `string_parameters` on a code, `bytes_parameters` on a UUID -> `None`; `fixed_byte_width` answers for a fixed string, fixed bytes and a UUID, and a code answers `code_width` instead, the maximum its standard fixes over the text it stores.
- A `yggdryl.string` or `yggdryl.bytes` document over a storage it does not describe -> a foreign field wearing our name, imported as its storage.
- Merging follows [Field](../field.md): two strings and two byte types meet parameter by parameter, and neither meets the other.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- ascii::fields ascii::leaves bytes::fields bytes::leaves bytes::values cast::typed::bytes cast::typed::strings string::leaves string::widths
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- string::codes
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(string|bytes)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "string or bytes or ascii or byte_column"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="string|byte|ASCII|ascii" node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```

## Performance

### Allocations per row

A `Str` holds its first twenty-three bytes inline and shares an `Arc<str>`
above them; a `Bytes` holds thirty inline and shares an `Arc<[u8]>` above
them. Those thresholds are the whole allocation story: below one a cell is
free to build and free to clone, above it it is one shared handle and one
copy. These counts are measured with the counting allocator and asserted in
`rust/tests/allocations.rs` - the two inline thresholds, a column built at
sixteen, a thousand and sixteen thousand rows, and a cell transcoded on each
side of the buffer - not timed: a count is the same on every machine, and a
timing is not. A cell read out of Arrow takes the same door a value does, so
the `read` rows are the value's cost.

| path | cell within the inline buffer | cell past it |
| --- | ---: | ---: |
| `utf8` / `ascii` build | one buffer per column | one buffer per column |
| `utf8` / `ascii` read | 0 | 1 |
| `cp1252` build | one buffer per column | one buffer per column |
| `cp1252` read, all-ASCII cell | 0 | 1 |
| `cp1252` read, transcoded cell | 0 | 2 |
| `binary` read | 0 | 1 |

A column's build cost is its buffers and not its rows: the payload is measured
with [`Charset::encoded_len`](../../charset/index.md) before a byte of it is
built, so the count is equal at sixteen rows and at sixteen thousand. The
`read` row past the inline buffer is one handle per cell out of a buffer Arrow
already shares; removing it needs a storage handle that does not fit
[`Scalar`](../scalar.md)'s pinned forty-eight bytes, so it is recorded rather
than spent.

A transcoded cell past the buffer costs two because the text is built once and
copied once into the shared handle, and `String` and `Arc<str>` have different
layouts, so no conversion between them is free.

### Timings

AMD Ryzen 5 150, 12 logical CPUs, Windows; Python 3.12.13 and Node 24.18,
release builds. Python reports the median of five runs of 10,000 iterations;
Node reports throughput over 2,000 iterations after warmup. These harnesses
measure different boundaries.

The string and byte family boundaries, on that host, from the binding gates
that landed the family (Python: the `datatypes.py` benchmark on the release
wheel; Node: `bench:types` with `YGGDRYL_BENCH_ITERATIONS=20000`). The Rust
rows await a regenerate of `cargo bench --bench types -- '^(string|bytes)/'`.

| operation | Python native boundary | JavaScript native boundary |
| --- | ---: | ---: |
| `fixed_ascii(3)` datatype | 195.9 ns/op | 509,466 ops/s |
| `sized_cp1252(32)` datatype | 543.1 ns/op | 223,772 ops/s |
| `string_parameters` read | 214.9 ns/op | 439,389 ops/s |
| `binary(16)` datatype | 387.9 ns/op | 335,892 ops/s |
| `bytes_parameters` read | 121.5 ns/op | — |
| `fixed_byte_width` read | — | 1,584,937 ops/s |
| string field | 1,499.6 ns/op | 88,122 ops/s |
| bytes field | 1,571.7 ns/op | 79,514 ops/s |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(string|bytes)/'
python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
YGGDRYL_BENCH_ITERATIONS=2000 npm run --prefix node bench:types
```

On Windows, use `python/.venv/Scripts/python.exe` and set
`$env:YGGDRYL_BENCH_ITERATIONS = '2000'` before the Node command.
