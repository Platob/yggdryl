# Arrow interop — metadata & round-tripping

`to_arrow` is **lossy** and *non-injective*: `u96`, `i96`, `FixedUtf8`, and a runtime-`N`
`FixedBinary` all collapse to the same `FixedSizeBinary(N)`, and `(precision, scale)` /
`(unit, timezone)` are not expressible in a byte width. A naive `from_arrow` would have to *guess*.
Instead the exact logical type is recorded in **field metadata** — Arrow's `Field::metadata`, which
yggdryl models as the centralized [`Headers`](../guide/headers.md) map — and `from_arrow` prefers it
to recover the precise type.

## The reserved keys

`to_arrow` writes these `yggdryl`-namespaced keys **only when the plain mapping is ambiguous**;
`from_arrow` reads them, then **strips** them from the user-visible metadata:

| key | written for | example value |
| --- | --- | --- |
| `yggdryl.logical_type` | a type that shares an Arrow representation with others (`u96`, `i96`, `u128`, `u256`, `fixed_utf8`, and the `Decimal`-backed `d*`) | `"u96"`, `"d128"` |
| `precision` | decimal columns | `"20"` |
| `scale` | decimal columns | `"2"` |
| `unit` | temporal columns | `"microsecond"` |
| `timezone` | timestamp columns | `"UTC"`, `"Europe/Paris"`, `""` (naive) |

Exact primitives (`i32` → `Int32`) and the `Decimal`-backed integers (`i128`/`i256`, which reverse
unambiguously to `Decimal128(38,0)` / `Decimal256(76,0)`) add **no** `yggdryl.logical_type` tag —
the plain mapping already reverses to them. A foreign `FixedSizeBinary(N)` with no yggdryl tag
decodes to the **safe default** — `fixed_binary` of that width — never a guessed wide integer.

## It round-trips exactly

```rust
# #[cfg(feature = "arrow")]
# fn demo() {
use yggdryl_core::io::fixed::{FixedUtf8Field, TypedField, I96, U96};

// u96 -> Arrow FixedSizeBinary(12) (lossy) + a "yggdryl.logical_type" = "u96" tag...
let field = TypedField::<U96>::new("hash", false);
let arrow = field.to_arrow();
assert_eq!(TypedField::<U96>::from_arrow(&arrow), Some(field)); // ...so it round-trips exactly
assert_eq!(TypedField::<I96>::from_arrow(&arrow), None);        // a same-width sibling: not i96

// FixedBinary vs FixedUtf8 (both FixedSizeBinary(N)) are disambiguated by metadata, and any
// user metadata is preserved through the round-trip.
let fu = FixedUtf8Field::new("code", 4, true).with_metadata_entry("charset", "ascii");
let back = FixedUtf8Field::from_arrow(&fu.to_arrow()).unwrap();
assert_eq!(back.metadata().get("charset"), Some("ascii"));
# }
```

## It survives IPC and Parquet

Arrow carries **unknown metadata keys** through its IPC and Parquet serializations, so the
`yggdryl`-namespaced discriminator survives an external round-trip: write a yggdryl column to a
Parquet file with a third-party tool and read it back, and `from_arrow` still recovers `u96` /
`d128` / a naive `ts32` at second resolution. The keys are ordinary string metadata, so they are
visible (as a field's `metadata`) in Python and Node too — the recovery logic itself lives in the
Rust core behind the `arrow` feature.

This is the mechanism the per-type pages rely on: [Primitives](primitives.md) for the
`FixedSizeBinary` / scale-0 `Decimal` widths, [Decimals](decimal.md) for `(precision, scale)`, and
[Temporal](temporal.md) for `(unit, timezone)` and the `Ts32`/`Duration32`/`Ts96` lossy cases.
