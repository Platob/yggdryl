# Benchmarks

Published throughput results for yggdryl, **organized to mirror the source tree** —
each report lives at the path of the code file it measures:

| Report | Measures |
| --- | --- |
| [yggdryl-core/io/byte_buffer.md](yggdryl-core/io/byte_buffer.md) | `ByteBuffer` positioned byte IO + resource transfer |
| [yggdryl-core/io/io_base.md](yggdryl-core/io/io_base.md) | `IOBase` typed primitive & bit arrays |
| [yggdryl-core/buffer/primitive_buffer.md](yggdryl-core/buffer/primitive_buffer.md) | typed buffers: construct, byte round-trips, Arrow |
| [yggdryl-core/codec/converter.md](yggdryl-core/codec/converter.md) | converters: numeric cast, flexible parse, render |
| [yggdryl-core/compression/gzip.md](yggdryl-core/compression/gzip.md) | gzip one-shot & streaming |
| [yggdryl-core/compression/zstd.md](yggdryl-core/compression/zstd.md) | zstd one-shot & vs gzip |

## Producing the numbers

The Rust core benches are dependency-free (`harness = false`); the binding scripts
weigh yggdryl against each platform's native equivalent:

```bash
cargo bench -p yggdryl-core                            # Rust core (io, buffer, compression)
(cd bindings/python && uv run maturin develop --release)  # then:
uv run python bindings/python/benchmarks/bench_io.py
uv run python bindings/python/benchmarks/bench_buffer.py
uv run python bindings/python/benchmarks/bench_converter.py
uv run python bindings/python/benchmarks/bench_compression.py
(cd bindings/node && npm run build)                    # then:
node bindings/node/benchmark/io.bench.js
node bindings/node/benchmark/buffer.bench.js
node bindings/node/benchmark/converter.bench.js
node bindings/node/benchmark/compression.bench.js
```

## Reading the numbers

- **Release only.** A debug extension is ~20× slower; the binding scripts say so.
- **Single-run, machine-dependent.** These are representative figures from one run
  on the reference machine below — treat them as ballpark, not guarantees. Expect
  run-to-run variance, especially for memory-bound paths (tens of GB/s).
- **Environment:** Windows 11, x86-64 (MSVC), `--release` (`lto`, `codegen-units=1`).

Each report also records the **optimizations** the benchmark surfaced, so the file
doubles as a performance changelog for its code.
