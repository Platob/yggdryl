# `IOBase` — typed primitive & bit arrays

Source: [`crates/yggdryl-core/src/io/io_base.rs`](../../../crates/yggdryl-core/src/io/io_base.rs)
· Bench: [`crates/yggdryl-core/benches/io.rs`](../../../crates/yggdryl-core/benches/io.rs)
(`cargo bench -p yggdryl-core --bench io`)

The typed accessors (`pread_i64` / `pread_i64_array` / `pwrite_bit_array` / …) are
**default** methods layered on `pread_byte_array` / `pwrite_byte_array`, so every
`IOBase` implementor gets them. Corpus: 1 MiB, 200 iterations, `--release`.

## Throughput

| Operation | Throughput |
| --- | --- |
| `pwrite_i64_array` | ~2.4–2.5 GB/s |
| `pread_i64_array` | ~1.5–1.6 GB/s |
| `pwrite_bit_array` (1 bit/value) | ~62 MB/s |

## Optimization: little-endian array write

`pwrite_<T>_array` originally built an intermediate `Vec<u8>` one value at a time
(`extend_from_slice(&v.to_le_bytes())`). On a little-endian host the in-memory bytes
of `&[T]` already equal the wire form, so we now reinterpret the slice as `&[u8]`
(guarded by `#[cfg(target_endian = "little")]`, with the byte-building loop retained
for big-endian) and write it in one shot:

| | before | after |
| --- | --- | --- |
| `pwrite_i64_array` | ~1.27 GB/s | **~2.5 GB/s (~2×)** |

Correctness is covered by the endianness edge-case tests
([`tests/typed_io.rs`](../../../crates/yggdryl-core/tests/typed_io.rs)).

## Known slow path: bit packing

`pwrite_bit_array` is scalar (one masked read-modify-write per bit) at ~62 MB/s.
Rewriting the loop to walk a `(byte, bit)` cursor instead of dividing per bit was
**neutral** — the compiler already lowers `/8` and `%8` on the constant to shifts,
so the cost is the ~1 M-iteration loop itself, not the arithmetic.

**Identified follow-up (not yet applied):** a bulk packer that writes whole interior
bytes from 8 bools at a time and only read-modify-writes the partial edge bytes,
and/or a `#[cfg(target_endian)]`-style zero-copy path for byte-aligned bitmaps.

## Read path left safe

A symmetric zero-copy `pread_<T>_array` (filling a pre-sized `Vec<T>`) was **not**
applied: it would need to allocate `count` elements up front, which risks OOM when a
caller over-requests (e.g. `count = usize::MAX`). The current read allocates only
the bytes actually available, so it degrades gracefully. Read stays at ~1.5 GB/s.
