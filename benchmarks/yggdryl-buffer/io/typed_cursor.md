# `TypedCursor<T>` — element-typed cursor throughput

Source: [`crates/yggdryl-buffer/src/io/typed_cursor.rs`](../../../crates/yggdryl-buffer/src/io/typed_cursor.rs)
· Bench: [`crates/yggdryl-buffer/benches/io.rs`](../../../crates/yggdryl-buffer/benches/io.rs)
(`cargo bench -p yggdryl-buffer --bench io`)

`TypedCursor<T>` is the element-typed cursor whose native unit is a `T` value:
`pwrite_array` / `pread_array` move whole `T`s (little-endian), `tell` / `seek` count
in `T` units, and a write past the end fills the gap with the `T`
[`default_value`](../../../crates/yggdryl-buffer/src/io/typed_io_base.rs). It wraps a
`ByteCursor`, so it inherits copy-on-write and the byte/bit positions. Corpus: 1 MiB
of `i64` (131 072 values), 200 iterations, `--release`.

## Throughput

| Operation | Throughput |
| --- | --- |
| `pwrite_array` (`TypedCursor<i64>`) | ~2.7–2.8 GB/s |
| `pread_array` (`TypedCursor<i64>`) | ~1.5–1.6 GB/s |

This matches the raw `IOBase::pwrite_i64_array` / `pread_i64_array` path
([`io_base.md`](io_base.md)) — the typed cursor adds no measurable overhead over the
byte-level typed accessors. The binding benchmarks weigh the same operation against
the platform's native `i64` codec (`array.array('q')` in Python,
`BigInt64Array` in Node): [`bindings/python/benchmarks/bench_io.py`](../../../bindings/python/benchmarks/bench_io.py),
[`bindings/node/benchmark/io.bench.js`](../../../bindings/node/benchmark/io.bench.js).

## Optimization: zero-copy little-endian write

The first cut of `pwrite_array` built an intermediate `Vec<u8>` one value at a time
(`value.write_le(&mut buf)`) and then did a resolve-seek / fill / re-seek / write
dance. Two fixes, mirroring `primitive_io!`'s array write:

1. On a little-endian host the in-memory bytes of `&[T]` already equal the wire form,
   so reinterpret the slice as `&[u8]` (guarded by `#[cfg(target_endian = "little")]`,
   with the value-building loop retained for big-endian) and write it in one shot — no
   per-element re-encode, no intermediate `Vec`.
2. Only pay the gap-fill (and its extra seek) when the write actually opens a gap past
   the end; the common append/overwrite is a single `pwrite_byte_array` at the resolved
   start.

| | before | after |
| --- | --- | --- |
| `pwrite_array` (`TypedCursor<i64>`) | ~1.28 GB/s | **~2.8 GB/s (~2.2×)** |

## Optimization: stack-buffer single read

`pread_one` allocated a `Vec<u8>` of `WIDTH` bytes per call. It now reads into an
8-byte stack scratch for the `WIDTH <= 8` native primitives (all of them), with a heap
fallback for any wider user `T`, so single-element reads do not touch the allocator.

| | before | after |
| --- | --- | --- |
| `pread_array` (`TypedCursor<i64>`) | ~0.96 GB/s | **~1.56 GB/s (~1.6×)** |

(The array read improved chiefly by benchmarking a pre-frozen `ByteBuffer` wrapped in a
fresh cursor — an `Arc` bump — instead of re-serialising the buffer each iteration.)

## Wide integers use the safe per-element path

`i96` / `i128` / `i256` are `IoPrimitive`s too, so `TypedCursor<i256>` works. They set
`REINTERPRET_LE = false` (the `i96` storage width differs from its 12-byte wire width,
and Arrow's `i256` in-memory layout is not guaranteed), so their writes encode each
value with `write_le` rather than the zero-copy reinterpret. Over a 1 MiB payload:

| Operation | Throughput |
| --- | --- |
| `pwrite_array` (`TypedCursor<i256>`, 32 B each) | ~1.0 GB/s |
| `pread_array` (`TypedCursor<i256>`) | ~1.0 GB/s |

The bindings weigh the wide-integer cursors against the platform's native big integer
(Python `int`, Node `BigInt`).

## Default-fill is correct-by-construction, not free-riding on zero

Growing a typed cursor past the end fills the gap with `default_byte_array`, i.e. the
`T` `Default` value's little-endian bytes. For every native primitive that default is
zero, so it coincides with the byte cursor's zero-fill — but routing the fill through
`default_value` keeps it correct for a hypothetical non-zero-default `T` and documents
the intent. Coverage:
[`tests/typed_cursor.rs`](../../../crates/yggdryl-buffer/tests/typed_cursor.rs).

## Read path left safe

As with `IOBase::pread_<T>_array`, `pread_array` does **not** pre-size a `Vec<T>` from
the requested `count` (which would risk OOM on an over-request like
`count = usize::MAX`); it reads only the bytes actually available and chunks them, so
it degrades gracefully.
