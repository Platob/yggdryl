# `ByteSlice` / `TypedSlice<T>` — bounded-window throughput

Source: [`crates/yggdryl-buffer/src/io/byte_slice.rs`](../../../crates/yggdryl-buffer/src/io/byte_slice.rs)
· [`typed_slice.rs`](../../../crates/yggdryl-buffer/src/io/typed_slice.rs)
· Bench: [`crates/yggdryl-buffer/benches/io.rs`](../../../crates/yggdryl-buffer/benches/io.rs)
(`cargo bench -p yggdryl-buffer --bench io`)

A **slice** is a fixed-length window `[offset, offset + len)` over a `ByteBuffer` — the
bounded, non-growing sibling of the cursor. It wraps a `ByteCursor` and clamps every
read/write to the window, so it should carry no measurable overhead beyond the clamp.
Corpus: a 1 MiB window, 200 iterations, `--release`.

## Throughput

| Operation | Throughput | Cursor baseline |
| --- | --- | --- |
| `ByteSlice` read (64 KiB chunks) | ~24–26 GB/s | ~26 GB/s (`ByteCursor`) |
| `TypedSlice<i64>` array read | ~1.3 GB/s | ~1.0–1.5 GB/s (`TypedCursor<i64>`) |

The window read matches the full-resource cursor read within noise — the clamp is a
`min` on the requested length, and the underlying `ByteCursor` returns an Arrow-backed
slice with no copy. The typed-slice read matches `TypedCursor<i64>`
([`typed_cursor.md`](typed_cursor.md)); it chunks the window bytes and decodes them.

## Why the write path is symmetric

`pwrite_byte_array` clamps `data.len()` to the remaining window and delegates to the
inner cursor, so a slice write is a cursor write of the clamped length — it never
grows (`TypedSlice::pwrite_array` additionally truncates to whole `T` values that fit).
Correctness for the clamping and copy-on-write is covered by
[`tests/byte_slice.rs`](../../../crates/yggdryl-buffer/tests/byte_slice.rs).

## Bindings

The binding benchmarks weigh the byte/typed slices against the platform's native
windowing (Python `memoryview` / a `bytes` sub-range, Node `Buffer.subarray`):
[`bindings/python/benchmarks/bench_io.py`](../../../bindings/python/benchmarks/bench_io.py),
[`bindings/node/benchmark/io.bench.js`](../../../bindings/node/benchmark/io.bench.js).
