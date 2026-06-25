# Benchmarks

Two harnesses, both in the repo, both reproducible:

1. **The Rust core** (`cargo bench`) — the library's true ceiling, no FFI in the path.
2. **From the bindings** (`benchmarks/compare.py`, `benchmarks/compare.mjs`) — the
   *same high-level code* run through yggdryl and through the host-language
   stalwarts (`requests` / Node `http`, the standard-library `gzip` / `zlib`) on
   the same in-process server and in-memory payload.

All figures are from one developer machine (localhost, no real network) — treat
them as ratios, not absolutes, and re-run them yourself.

!!! note "Honesty first"
    yggdryl is not faster at *everything*. The wins below are real and reproduced
    here; the one spot the standard library still leads (gzip *decompress*, where
    `zlib` is a hand-tuned C decoder) is shown in the repo's
    [`benchmarks/README.md`](https://github.com/Platob/yggdryl/blob/main/benchmarks/README.md) too.

## Same code, two backends (Python)

| workload | yggdryl | `requests` / stdlib | speedup |
| --- | --- | --- | --- |
| GET small body (latency) | 0.53 ms | 0.83 ms | **1.6×** |
| GET 8 MiB body (throughput) | 912 MiB/s | 530 MiB/s | **1.7×** |
| gzip compress | 14 MiB/s | 9 MiB/s | **1.5×** |
| zstd / snappy | available | *(not in stdlib)* | — |

## The Rust core

| workload | result |
| --- | --- |
| `Io::pread` (memory, positional) | 1.3 ns |
| `copy` BytesIO → BytesIO (zero-copy) | 8.4 GiB/s |
| `read_to_end` chunked | 13.7 GiB/s |
| compression `Io`-stream decompress | = one-shot (no overhead) |
| `HttpStream` windowed `read_to_end` | 1.35 GiB/s |
| footer via `pread` (one Range, no full download) | 0.44 ms |
| `send_many` vs sequential | ≈6× |

## Reproduce

```bash
# Rust core
cargo bench -p yggdryl-io
cargo bench -p yggdryl-compression --all-features
cargo bench -p yggdryl-http --all-features

# Same code vs the host-language stalwarts
(cd bindings/python && maturin develop) && pip install requests && python3 benchmarks/compare.py
(cd bindings/node && npm run build) && node benchmarks/compare.mjs
```

See the repo's [`benchmarks/`](https://github.com/Platob/yggdryl/tree/main/benchmarks)
folder for the full tables and the measurement methodology (including `TCP_NODELAY`
on both sides for a fair fight).
