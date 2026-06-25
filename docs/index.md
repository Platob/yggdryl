# yggdryl

**One streaming byte-IO core — `Io`, compression and HTTP — for Rust, Python and Node.**
Write the same high-level code in any of the three languages and get near-Rust
throughput, because all the work happens in a dependency-light Rust core and the
bindings are thin wrappers that never copy your bytes through the host language.

```python
# Python — looks like requests, runs in Rust
import yggdryl
session = yggdryl.HttpSession()
data = session.get("https://example.com/data.csv.gz").content
```

```javascript
// Node — looks like fetch/axios, runs in Rust
const { HttpSession } = require("yggdryl");
const data = (await new HttpSession().get("https://example.com/data.csv.gz")).content;
```

```rust
// Rust — the core
use yggdryl_http::HttpSession;
let data = HttpSession::new().get("https://example.com/data.csv.gz")?.bytes()?;
```

## Why

A reader (think Arrow / Parquet / CSV / JSON) should not care **where** its bytes
live — an in-memory buffer, a memory-mapped file, or a remote HTTP object — nor
whether they arrive **random** (read a footer, a column chunk) or **streamed**
(scan record batches). yggdryl unifies all of that behind **one trait, `Io`**, and
builds compression and a `requests`-like HTTP client on top of it, each handle
composing with the next with **at most one copy** of the data.

- **One trait for all bytes** — `Io` carries reads, writes, a cursor and
  positional `pread`/`pwrite`. A memory backend gets zero-copy reads for free; a
  streamed backend (an HTTP body, a decompressor) just implements `read`.
- **Streamed compression over `Io`** — gzip / Zstandard / Snappy encoders and
  decoders are themselves `Io` handles, so they slot straight into an HTTP body or
  a file with no buffering of the whole payload.
- **A `requests`-like HTTP client** — connection pooling, retries with
  resume-on-drop, a **seekable** response body (read a footer without downloading
  the object), concurrent `send_many`, cookies and redirects.
- **Three languages, one surface** — the Rust core is the source of truth; the
  Python (PyO3) and Node (napi-rs) bindings expose the same names and semantics.

## Headline numbers

Measured on one developer machine (localhost, no real network) — see
[Benchmarks](benchmarks.md) for the full tables and how to reproduce.

| | yggdryl | baseline | gain |
| --- | --- | --- | --- |
| HTTP GET (small body, latency, Python) | 0.53 ms | `requests` 0.83 ms | **1.6×** |
| HTTP GET (8 MiB, throughput, Python) | 912 MiB/s | `requests` 530 MiB/s | **1.7×** |
| gzip compress (Python) | 14 MiB/s | stdlib `gzip` 9 MiB/s | **1.5×** |
| `copy` BytesIO → BytesIO (Rust core) | 8.4 GiB/s | — | zero-copy |
| `HttpStream` windowed read (Rust core) | 1.35 GiB/s | — | streamed |

…plus `zstd`/`snappy` codecs the Python/Node standard libraries don't ship at all.

## Next

- **[Quickstart](quickstart.md)** — install and the first ten lines in each language.
- **[Byte IO](io.md)** — the `Io` trait and its backends.
- **[Compression](compression.md)** — streamed codecs.
- **[HTTP](http.md)** — the client, streaming, cookies, redirects.
