# Converters — numeric cast, flexible parse, render

Source: [`crates/yggdryl-converter/src/converter.rs`](../../../crates/yggdryl-converter/src/converter.rs)
· Bench: [`crates/yggdryl-converter/benches/converter.rs`](../../../crates/yggdryl-converter/benches/converter.rs)
(`cargo bench -p yggdryl-converter --bench converter`)

The converter family maps between representations: a dtype-keyed numeric **cast**
(bulk, little-endian bytes), a flexible string **parse**, and a **render** back to
text. The benchmark exercises the three on `i32`. Corpus: 1 MiB (256 Ki `i32`) for the
cast, 100 k values for parse / render, `--release`.

## Core throughput

| Operation | Throughput |
| --- | --- |
| `cast_bytes` i32 → i64 (bulk) | ~1.3 GB/s |
| `parse_bytes` string → i32 | ~50 MB/s |
| `format_bytes` i32 → string | ~69 MB/s |

The **bulk cast is memcpy-class** — one decode + `as` + encode per element over a
contiguous buffer, no per-value dispatch. Parse and render are per-scalar: parse takes
the fast path (plain decimal, no allocation) yet still runs a `str → int` state
machine per value, and render formats each `i32` through `Display`.

## Binding comparison — the bulk byte path is the fast path

The bindings weigh `yggdryl.converter` against each platform's native scalar
conversion for the **same three operations**.

Python `yggdryl.converter` vs `int()` / `str()` / `array` widen
([`bench_converter.py`](../../../bindings/python/benchmarks/bench_converter.py)):

| Operation | yggdryl | stdlib | ratio |
| --- | --- | --- | --- |
| parse → i32 | ~10.5 MB/s | ~62.2 MB/s | ~0.17× |
| format i32 | ~11.4 MB/s | ~50.0 MB/s | ~0.23× |
| cast i32 → i64 (bulk) | ~573 MB/s | ~49.6 MB/s | **~11.6×** |

Node `yggdryl.converter` vs `Number` / `String` / `BigInt64Array`
([`converter.bench.js`](../../../bindings/node/benchmark/converter.bench.js)):

| Operation | yggdryl | native | ratio |
| --- | --- | --- | --- |
| parse → i32 | ~13.3 MB/s | ~218.8 MB/s | ~0.06× |
| format i32 | ~10.0 MB/s | ~96.7 MB/s | ~0.10× |
| cast i32 → i64 (bulk) | ~1.19 GB/s | ~22.2 MB/s | **~53.8×** |

**Reading these:** the **bulk `cast` over a byte buffer is the fast path** and wins
decisively — ~11.6× (Python) / ~53.8× (Node) — because it crosses the FFI **once** and
does the whole widen in native code, whereas the engines' element-wise typed-array
widening (`array('q', …)`, `BigInt64Array.from`) pays per-element in-language cost.

The **per-scalar `parse` / `format` trail native `int()` / `Number` / `String`**
(~0.06×–0.23×): each call crosses the FFI boundary for one value, and that fixed
per-call cost dwarfs the tiny parse itself, while the engines' built-ins stay
in-language. The lesson mirrors the [typed-buffer report](../buffer/primitive_buffer.md):
**batch through bytes** — convert a whole buffer with one `cast` call rather than
looping `parse` / `format` per element — to stay on the winning path.

## Optimization history

- **Dtype-keyed byte facade** — `PrimitiveType::cast_bytes` dispatches the source ×
  target matrix once, then runs a tight decode → `as` → encode loop over the
  contiguous buffer, so a bulk cast is a single FFI crossing at ~1.3 GB/s.
- **Fastest-format-first parse** — the parser tries plain signed decimal first and
  allocates only when a value actually uses a radix prefix or `_` / `,` separators, so
  the common case never touches the heap.
- **Allocation-free `as` cast** — numeric conversion is Rust's total `as`, never a
  checked/boxed path, so the cast loop has no per-element branch beyond the write.
