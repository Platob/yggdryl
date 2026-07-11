# `ByteBuffer` / `ByteCursor` — cursor byte IO

Source: [`crates/yggdryl-buffer/src/io/byte_buffer.rs`](../../../crates/yggdryl-buffer/src/io/byte_buffer.rs)
· [`byte_cursor.rs`](../../../crates/yggdryl-buffer/src/io/byte_cursor.rs)
· Bench: [`crates/yggdryl-buffer/benches/io.rs`](../../../crates/yggdryl-buffer/benches/io.rs)
(`cargo bench -p yggdryl-buffer --bench io`)

IO is split `std::io::Cursor`-style: a [`ByteBuffer`] is immutable storage, a
[`ByteCursor`] holds a share plus a position and does the advancing reads/writes
(copy-on-write, so the buffer stays intact). Corpus: 1 MiB, 64 KiB chunks, 200
iterations, `--release`.

## Cursor IO throughput

| Operation | Throughput |
| --- | --- |
| `pwrite_byte_array` (sequential) | ~1.7 GB/s |
| `pread_byte_array` (allocating) | ~17 GB/s |
| `pread_into` (allocation-free) | ~18 GB/s |
| `pread_io` (cursor → cursor transfer) | ~1.1 GB/s |
| `pwrite_i64_array` | ~2.0 GB/s |
| `pread_i64_array` | ~1.1 GB/s |

Reads are memcpy-bound (high variance). `pread_into` avoids the per-chunk `Vec`
allocation that `pread_byte_array` pays.

## Zero-copy Arrow (feature `arrow`)

`ByteBuffer::from_arrow_byte_buffer` wraps an Arrow `Buffer` by sharing its
allocation — an `Arc` bump, **independent of size** — versus copying it in:

| Construct 1 MiB | Throughput |
| --- | --- |
| `from_arrow_byte_buffer` (zero-copy) | ~O(1) — effectively instant (>10⁵ GB/s) |
| `from_bytes` (copy) | ~2.3 GB/s |

The wrap is constant-time regardless of buffer size, so the larger the buffer the
larger the win. A cursor that then **writes** copies-on-write, leaving the Arrow
allocation untouched. `to_arrow_byte_buffer` is likewise zero-copy when the buffer
is already Arrow-backed. Enable with `--features arrow` (off by default so the core
carries no Arrow vocabulary).

## Binding comparison — bypassing the FFI copy

Python `ByteCursor` vs stdlib `io.BytesIO` (1 MiB, 64 KiB chunks, `--release`;
[`bench_io.py`](../../../bindings/python/benchmarks/bench_io.py)):

| Operation | yggdryl | `BytesIO` | ratio |
| --- | --- | --- | --- |
| write | ~2.3 GB/s | ~0.7 GB/s | **~3.2×** |
| read (`pread_into` / `readinto`) | ~23.6 GB/s | ~25.4 GB/s | ~0.93× |

The **`pread_into(bytearray)`** fill-into method reads with zero per-call allocation
(no `bytes` crosses the FFI), lifting read from **~0.26× → ~0.93×** of `BytesIO` — a
3.5× improvement over the allocating `pread_byte_array`. The residual ~7% is per-call
FFI overhead; a true zero-copy read (Python buffer protocol / `memoryview`) is
**blocked by the abi3 limited API** we target, so `pread_into` is the ceiling here.
Node mirrors it with `preadInto(Buffer)`, filling the JS `Buffer` in place.

## Optimization history

- **`pread_into`** — allocation-free read (overridden by `ByteCursor` to copy
  straight from the backing slice); used by streaming, transfers, and the bindings.
- **Capacity-preserving COW** — a cursor's first write copies the buffer *with its
  spare capacity*, so a `with_byte_capacity`-preallocated buffer keeps its headroom
  and sequential writes don't reallocate (restored write to ~3.2× `BytesIO`).
- **LE array write** — `pwrite_<T>_array` reinterprets `&[T]` as bytes on
  little-endian hosts (no per-element loop), ~2× the naïve build-a-`Vec` path.
