# Headers — serialize, deserialize, and the get/set/mutate hot paths

Source: [`crates/yggdryl-http/src/`](../../../crates/yggdryl-http/src/)
· Bench: [`crates/yggdryl-http/benches/headers.rs`](../../../crates/yggdryl-http/benches/headers.rs)
(`cargo bench -p yggdryl-http --bench headers`)

Corpus: a realistic 16-entry header block (~16-byte keys, ~32-byte values), ~800 bytes
per serialised block, 200 000 iterations.

## Throughput

| Op | Rate |
| --- | --- |
| `serialize_bytes` | ~950 MB/s |
| `deserialize_bytes` | ~260 MB/s |
| `get` (byte key) | ~16 Mops/s |
| `insert` (add/update) | ~8 Mops/s |
| **`get_mut` in-place value mutation** | **~67 Mops/s** |

## Optimization history

- **Zero-copy in-place mutation.** Extending a value through `get_mut` (~67 Mops/s) is
  roughly **8× faster** than the add/update `insert` path (~8 Mops/s): it skips the map
  re-lookup and the value re-allocation, mutating the existing `Vec<u8>` in place. This is
  the reason `Headers::get_mut` / `HeadersBased::get_header_mut` exist — patch or append to
  a header value without cloning the map or re-inserting the entry.
- **Deterministic, allocation-lean codec.** Serialisation is a single growable `Vec`
  with `u32` length prefixes (no per-entry allocation); the ordered `BTreeMap` makes the
  bytes canonical so equality/hashing agree with `serialize_bytes` without a normalisation
  pass.
