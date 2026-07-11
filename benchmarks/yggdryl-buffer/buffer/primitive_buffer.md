# Typed buffers — construct, byte round-trips, Arrow

Source: [`crates/yggdryl-buffer/src/`](../../../crates/yggdryl-buffer/src/)
· Bench: [`crates/yggdryl-buffer/benches/buffer.rs`](../../../crates/yggdryl-buffer/benches/buffer.rs)
(`cargo bench -p yggdryl-buffer --bench buffer`)

The typed buffers (`I8Buffer` … `F64Buffer`, `BooleanBuffer`) are immutable,
cheaply-shared contiguous stores. The benchmark exercises the three value
operations on an `I32Buffer` — constructing from values, serialising to
little-endian bytes, and deserialising back — plus the zero-copy Arrow wrap. Corpus:
256 Ki `i32` (1 MiB), 200 iterations, `--release`.

## Core throughput

| Operation | Throughput |
| --- | --- |
| `from_slice` (construct) | ~3.0 GB/s |
| `serialize_bytes` | ~3.2 GB/s |
| `deserialize_bytes` | ~3.1 GB/s |

All three are memcpy-bound: `as_bytes` reinterprets the aligned value slice with no
copy, `serialize_bytes` copies it out once (little-endian hosts), and
`deserialize_bytes` decodes the little-endian bytes back into an owned buffer.

## Zero-copy Arrow (feature `arrow`)

`I32Buffer::from_arrow` wraps an Arrow `ScalarBuffer` by sharing its allocation — an
`Arc` bump, **independent of size** — versus copying values in:

| Construct 1 MiB | Throughput |
| --- | --- |
| `from_arrow` (zero-copy) | ~O(1) — effectively instant (>10⁵ GB/s) |
| `from_slice` (copy) | ~3.3 GB/s |

The wrap is constant-time regardless of length, so the larger the buffer the larger
the win. `to_arrow` is likewise a zero-copy `Arc` bump.

## Binding comparison — the byte path is the fast path

The bindings weigh each buffer against the platform's native contiguous typed array
for the **same three pure operations** (pre-built inputs; the native `serialize` is a
real byte copy, the fair analogue of `serialize_bytes`).

Python `I32Buffer` vs `array.array('i', …)`
([`bench_buffer.py`](../../../bindings/python/benchmarks/bench_buffer.py)):

| Operation | yggdryl | `array` | ratio |
| --- | --- | --- | --- |
| construct (from `list`) | ~158 MB/s | ~110 MB/s | **~1.44×** |
| serialize | ~1.36 GB/s | ~2.80 GB/s | ~0.49× |
| deserialize | ~2.86 GB/s | ~3.00 GB/s | ~0.95× |

Node `I32Buffer` vs `Int32Array`
([`buffer.bench.js`](../../../bindings/node/benchmark/buffer.bench.js)):

| Operation | yggdryl | `Int32Array` | ratio |
| --- | --- | --- | --- |
| construct (from `Array`) | ~38 MB/s | ~2.07 GB/s | ~0.02× |
| serialize | ~2.79 GB/s | ~2.72 GB/s | **~1.02×** |
| deserialize | ~3.25 GB/s | ~2.77 GB/s | **~1.17×** |

**Reading these:** the **byte round-trip is the fast path** — `deserialize_bytes` is
~0.95× (Python) / ~1.17× (Node) and Node `serialize_bytes` ~1.02×, all competitive
with or ahead of the engine's native typed array. `array.tobytes()` is a tight native
memcpy that yggdryl's cross-FFI copy trails at ~0.49×.

**Element-wise construction from a language-native list** tells two stories:
`I32Buffer(list)` **beats** `array.array` in Python (~1.44× — PyO3 batch-extracts the
`Vec<i32>`), but on Node it is ~0.02× of `Int32Array.from` because napi marshals each
JS number across the FFI individually while `Int32Array.from` stays in-engine. For
bulk data on Node, **build from bytes** (`deserializeBytes` over a `Buffer`), which is
the competitive path above; a future zero-copy constructor accepting a `Buffer` /
`TypedArray` directly (per `CLAUDE.md` rule 9) would close the element-wise gap.

## Optimization history

- **Byte-backed value view** — buffers store an `arrow_buffer::ScalarBuffer<T>` (or an
  `Arc<Vec<T>>` without the `arrow` feature), so `as_slice` is an aligned zero-copy
  view and `as_bytes` reinterprets it with no allocation.
- **Zero-copy Arrow construction** — `from_arrow` shares the Arrow allocation (an
  `Arc` bump), constant-time versus the ~3.3 GB/s copying `from_slice`.
- **LE fast path** — on little-endian hosts `serialize_bytes` is a single copy of the
  reinterpreted bytes rather than a per-element encode loop.
