# Wide integers

`yggdryl-core` adds three **wide signed integers** that flank native `i128`:

- **`i96`** — a 96-bit two's-complement integer, held canonically in an `i128` and
  re-wrapped to 96 bits, so its arithmetic reuses the native 128-bit path.
- **`i128`** — native Rust `i128`, used directly.
- **`i256`** — Apache Arrow's own 256-bit integer (the core is Arrow-backed), with its
  full, tested arithmetic.

Each has value semantics (`Eq` / `Ord` / `Hash`), a base-10 `Display`, and a
little-endian byte round-trip (`to_le_bytes` / `from_le_bytes`, 12 / 16 / 32 bytes), so
each is an `IoPrimitive` — a [`TypedCursor`](io.md) reads and writes it.

## Arithmetic

The operators `+`, `-`, `*`, `/`, `%`, and unary `-` **panic on overflow** like the
primitive integers, with `checked_*` / `wrapping_*` / `saturating_*` / `overflowing_*`
for the other overflow behaviours.

```rust
use yggdryl_core::{i96, i256};

// i96 flanks i64/i128.
let a = i96::from_i64(1_000_000_000_000);
assert_eq!((a * i96::from_i64(1000)).to_i128(), 1_000_000_000_000_000);
assert_eq!(i96::MAX.checked_add(i96::ONE), None);        // overflow -> None
assert_eq!(i96::MAX.wrapping_add(i96::ONE), i96::MIN);   // wraps around

// i256 carries values far beyond i128.
let big = i256::from_i128(i128::MAX) * i256::from_i128(2);
assert_eq!(big.to_i128(), None);                          // exceeds i128
assert_eq!(i256::from_le_bytes(big.to_le_bytes()), big);  // 32-byte round-trip
```

## In the bindings

The wide integers have **no fixed-width scalar** in Python or JS, so the
[cursors](io.md#wide-integers-i96-i128-i256) marshal their values as an
arbitrary-precision `int` (Python) / `BigInt` (Node). An out-of-range value raises
(`OverflowError` / a thrown `Error`). The arithmetic itself lives in the Rust core;
the bindings expose the IO cursors, not the scalar types.

## Benchmarks

The wide-integer typed cursor has a throughput benchmark alongside the native typed
cursor (`cargo bench -p yggdryl-core --bench io`); see the
[`TypedCursor` report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/typed_cursor.md).
