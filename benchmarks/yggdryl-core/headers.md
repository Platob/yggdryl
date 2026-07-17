# `headers::Headers` — media type + mtime benchmark

Time **and** memory for the centralized media-type accessors (`Content-Type` /
`Content-Encoding` → [`MimeType`](../../crates/yggdryl-core/src/mimetype.rs) /
[`MediaType`](../../crates/yggdryl-core/src/mediatype.rs)) and the epoch-microseconds
`mtime` codec on [`Headers`](../../crates/yggdryl-core/src/headers.rs), plus the plain
get/insert baseline. Dependency-free harness with the same counting allocator as the other
benches.

## Run

```bash
cargo bench -p yggdryl-core --bench headers
cargo test  -p yggdryl-core --test headers   # functional suite
```

## Rust core (release, counting global allocator)

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `content_type` (get, borrow) | 69 | **0.00** | 0.0 |
| `mime_type` (parse primary) | 2.9 | 3.00 | 50.0 |
| `media_type` (type + encoding fold) | 0.95 | 8.00 | 411.0 |
| `set_mime_type` (replace) | 6.6 | 2.00 | 28.0 |
| `mtime` (get, parse decimal) | 26 | **0.00** | 0.0 |
| **`set_mtime`** (render decimal) | 6.5 | **2.00** | 26.0 |
| `get` (plain, present) | 77 | 0.00 | 0.0 |
| `insert` (replace, present) | 7.7 | 2.00 | 15.0 |

## What the numbers show

- **Reads that borrow allocate nothing.** `content_type` and `mtime` (get) return a borrowed
  `&str` / parse an integer off it — `0.00` allocs/op. `mime_type` / `media_type` build owned
  value types (a `MimeType` owns its essence string; a `MediaType` owns a list), so they cost
  one allocation per owned field — the price of materializing the parsed type.
- **`mtime` is an allocation-free render.** `set_mtime` writes the epoch-microseconds decimal
  straight into a fixed **stack buffer** (no `format!` / `String` temporary) before storing —
  so it costs exactly the same **2 allocations** as a plain `insert` (the entry's name + value
  boxes) and nothing more, and it round-trips signed values (including `i64::MIN`, tested).
- **One place interprets the media headers.** `Content-Type` / `Content-Encoding` are read and
  written only through these accessors; `media_type` folds the encoding tokens into the layered
  stack (`application/x-tar` + `gzip` → `[application/x-tar, application/gzip]`), and
  `set_media_type` writes the comma-joined essences back — so the io layer and `Uri` share one
  interpretation.
