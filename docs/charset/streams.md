# Streams

`Decoder`, `Reader`, and `Writer` are the same four charset operations over bytes a caller already holds in pieces.

## Contract

| | |
| --- | --- |
| Owns | `charset::Decoder`, `charset::Reader`, `charset::Writer` |
| Select | `Charset::decoder`, `Charset::transcriber`, `Charset::reader`, `Charset::writer` |
| Retains | At most three bytes, the longest sequence a chunk boundary can split |
| `Decoder` | `push` into a `String`, `push_bytes` into UTF-8 bytes, `finish` to refuse a payload that stops mid-sequence; built by `decoder` to refuse a byte it cannot read, or by `transcriber` to read every byte as `transcribe` does and refuse none |
| `Reader` | `Read` yielding UTF-8; fails on the read that reaches a truncated end |
| `Writer` | `Write` taking UTF-8; `finish` flushes and refuses a half-written scalar |
| Python | Rust only; `charset.decode` and `charset.encode` are the whole-buffer doors |
| JavaScript | Rust only; `charset.decode` and `charset.encode` are the whole-buffer doors |

## Use

A chunked decode answers exactly what a whole one answers, at every split.

```rust
use yggdryl::Charset;

let mut decoder = Charset::Utf8.decoder();
let mut text = String::new();

// The two bytes of "é" arrive in different chunks.
decoder.push(b"caf\xc3", &mut text)?;
assert_eq!(text, "caf");
assert!(decoder.is_pending());

decoder.push(b"\xa9", &mut text)?;
assert_eq!(text, "café");
decoder.finish()?;
```

## Readers and writers

```rust
use std::io::{Read, Write};

use yggdryl::Charset;

let body = "symbol,désk\nAAPL,€1\n";

// A source in one charset, read as UTF-8.
let wire = Charset::Cp1252.encode(body)?.into_owned();
let mut decoded = String::new();
Charset::Cp1252
    .reader(std::io::Cursor::new(wire.clone()))
    .read_to_string(&mut decoded)?;
assert_eq!(decoded, body);

// And back, in whatever chunks the caller writes.
let mut encoded = Vec::new();
{
    let mut writer = Charset::Cp1252.writer(&mut encoded);
    for piece in body.as_bytes().chunks(3) {
        writer.write_all(piece)?;
    }
    writer.finish()?;
}
assert_eq!(encoded, wire);
```

## Edges

- A payload that stops mid-sequence -> `Decoder::finish` and `Writer::finish` refuse it, and `Reader` fails on the read that reaches the end. Dropping a `Writer` without finishing loses a half-written scalar silently, which is what `finish` exists to prevent.
- Bytes that are simply invalid -> reported where they are, not held back: only a sequence that could still be completed is retained.
- A run of bytes that could never complete a sequence -> refused once the carry fills, rather than accumulated without bound.
- `Charset::Utf8.reader` -> the source unchanged, and its bytes are not revalidated. `Charset::decode` is the validating door.
- `Decoder::consumed` -> input bytes taken, which is also where a refusal's position is measured from.

## Commands

```bash
cargo test --features "parquet iceberg" -p yggdryl --lib charset::tests::a_chunked
cargo test --features "parquet iceberg" -p yggdryl --lib charset::tests::a_writer
cargo bench -p yggdryl --bench charset -- charset_streaming
```
