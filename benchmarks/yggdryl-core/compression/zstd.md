# `Zstd` — Zstandard compression

Source: [`crates/yggdryl-core/src/compression/zstd.rs`](../../../crates/yggdryl-core/src/compression/zstd.rs)
· Bench: [`benches/compression.rs`](../../../crates/yggdryl-core/benches/compression.rs)
(`cargo bench -p yggdryl-core --bench compression`)

Backed by `zstd` (bundles libzstd, built via `cc` — a C compiler is required, CMake
is not). Corpus: 1 MiB of English-like text, 200 iterations, `--release`.

## One-shot throughput

| Level | Encode | Decode | Ratio |
| --- | --- | --- | --- |
| 1 | ~2.4 GB/s | ~0.83 GB/s | ~6600× |
| 3 (default) | ~1.4 GB/s | ~0.73 GB/s | ~6600× |
| 19 | ~0.05 GB/s | ~0.61 GB/s | ~6900× |

On this highly-repetitive corpus zstd reaches **far higher ratios than gzip**
(~6600× vs gzip's ~335×) at competitive-to-better speed: **level-1 encode (~2.4 GB/s)
beats gzip's fastest**, and the default level-3 encode (~1.4 GB/s) is ~6× gzip's
default level-6. Decode is ~0.6–0.8 GB/s across levels.

## vs gzip (same corpus)

| | gzip L6 (default) | zstd L3 (default) |
| --- | --- | --- |
| encode | ~0.23 GB/s | **~1.4 GB/s** |
| decode | ~1.0 GB/s | ~0.73 GB/s |
| ratio | ~335× | **~6600×** |

zstd is the stronger default for ratio and encode speed; gzip decodes faster here.
Both ship by default (`gzip` + `zstd` features). See also
[gzip.md](gzip.md).
