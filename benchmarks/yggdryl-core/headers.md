# `io::Headers` — benchmark & optimization notes

Time **and** memory for the byte-string header map, over a realistic ~16-entry request set.
The harness is dependency-free and finishes in ~1 s; allocation counts are build-independent,
so the zero-allocation lookup is asserted implicitly by the numbers below (and by the
`io_headers` round-trip tests).

## Run

```bash
cargo bench -p yggdryl-core --bench headers
```

## Rust core (release, counting global allocator, 16 entries)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `get` (hit, case-insensitive) | 36.35 | **0.00** | 0.0 |
| `get` (miss) | 91.34 | **0.00** | 0.0 |
| `content_length` (parse) | 41.00 | **0.00** | 0.0 |
| `build` (16 appends) | 0.46 | 33.00 | 953.0 |
| `to_http_bytes` (render) | 4.42 | **1.00** | 505.0 |
| `parse_http` | 0.36 | 35.00 | 1337.0 |
| binary write+read round-trip | 0.07 | 172.00 | 10329.0 |

## What the numbers show

- **Lookup is zero-allocation.** `get` (and `contains` / the typed helpers) is a linear scan
  of the compact, insertion-ordered entries, comparing names with
  `slice::eq_ignore_ascii_case` — **0 allocs**, whether the name hits (36 Mops/s, scanning to
  the last entry) or misses (91 Mops/s). For the small `n` of a header set this beats hashing:
  no hash to compute, no map to allocate, and order + duplicates are preserved exactly, which
  HTTP requires. `content_length` parses straight from the borrowed value — also 0 allocs.
- **Render is one allocation.** `to_http_bytes` sizes the output up front and fills it in a
  single pass (`1.00 alloc`). `build` / `parse_http` allocate the owned `Box<[u8]>` per name
  and value (the storage is `bytes/bytes`), which is inherent to owning the entries.
- **The binary codec favours robustness over allocation count.** `write_to` emits a
  length-prefixed frame field-by-field through the generic `IOCursor` sink — each `write_all`
  re-seals the `Bytes` sink's `Arc`, so a full 16-entry write+read is allocation-heavy. That
  is the serialization path (not the hot path), and it buys a round-trip of **arbitrary**
  bytes that the HTTP text form cannot represent (names/values containing `:` or `\r\n`).
