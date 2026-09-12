# Transcoded

`charset::Transcoded` wraps any `IOBase` handle and presents its decoded UTF-8 bytes, so nothing downstream learns the charset.

## Contract

| | |
| --- | --- |
| Owns | `charset::Transcoded<H>` |
| Select | `Transcoded::infer` from the handle's own media type; `Transcoded::new` from an explicit `Charset` |
| Presents | UTF-8 on read, the charset's bytes on write |
| Media type | The wrapped one with its charset removed, because that is what the bytes now are |
| Composes | Any [`IOBase`](../holder/index.md), including a [`Coded`](../coding/index.md) handle; `Transcoded` is itself an `IOBase` and an `IOMedia` |
| Random access | Resume points recorded by one streamed pass; a positional read seeks to the point before it and decodes forward at most one stride |
| Materializes | Only on an explicit `open` or a positional write, and until `close` |
| Commit | `flush`, `close`, or `into_handle`; never `pwrite` alone |
| `Charset::Utf8` | Changes nothing and revalidates nothing, exactly as `Codec::Identity` does for codings |
| Python | Rust only |
| JavaScript | Rust only |

## Use

A handle that declares its charset on its media type needs no wrapper for the [record reader](../media/text.md#declaring-a-charset) or the [structured codec](../text/index.md): both read the declaration where they build their transport. `Transcoded` is for the decoded bytes themselves - `read_all_bytes`, the digest, a cursor over the text - for [random access](#random-access) into them, and for a charset laid over a wrong declaration, `Transcoded::new(handle, Charset::Utf8)` around a stale `charset=iso-8859-1`. A `Transcoded` over a located file is re-opened by its location by the record reader, which reads the file raw under the wrapper's media type - the one with the charset removed - so a record read decodes nothing through it: declare on the media type instead.

```rust
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, IOBase, MediaType};

let body = "symbol,désk\nAAPL,€1\n";
let wire = Charset::Cp1252.encode(body)?.into_owned();
let source = Buffer::from_bytes(wire)
    .with_media_type(MediaType::from_str("text/csv;charset=windows-1252")?);

let handle = Transcoded::infer(source);
assert_eq!(handle.charset(), Charset::Cp1252);
assert_eq!(handle.read_all_bytes()?, body.as_bytes());

// The presented bytes are UTF-8, so the handle no longer declares a charset.
assert_eq!(handle.media_type().charset(), None);
```

## Random access

A decode has no seek of its own - the byte at a decoded offset is only reachable by decoding what precedes it - so this handle builds one out of the streaming decoder it already has.

The first positional read past the beginning makes one streamed pass and records a resume point every 64 KiB of decoded output: the source offset and the decoded offset of the same position, taken only where the decoder holds no partial sequence. Every later positional read binary-searches that index, asks the wrapped handle for bytes *at* the resume point, and decodes forward at most one stride. Reading from offset zero streams straight through and builds nothing.

```rust
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, IOBase, IOCursor};

let body = "symbol,désk,prix €\n".repeat(8 * 1024);
let source = Buffer::from_bytes(Charset::Cp1252.encode(&body)?.into_owned());
let handle = Transcoded::new(source, Charset::Cp1252);

// The decoded length is longer than the encoded one, and is measured by the
// same pass that indexes the value.
assert_eq!(handle.size(), body.len() as u64);
assert!(handle.size() > handle.handle().size());

// A window near the end costs one backing-store read, not a decode of the
// whole prefix.
let line = "symbol,désk,prix €\n";
let tail = handle.read_range_bytes(handle.size() - line.len() as u64, line.len())?;
assert_eq!(tail, line.as_bytes());

// And the shared cursor walks it like any other handle.
let mut cursor = handle.cursor_at(7);
let mut window = [0_u8; 5];
let read = cursor.read_next(&mut window)?;
assert_eq!(&window[..read], "désk".as_bytes());
```

Two properties make the index sound, and both are contracts rather than assumptions: every charset here is stateless past a sequence boundary, so a byte offset is the whole of what a resume needs; and `Decoder::is_pending` is what says a boundary has been reached. A shift-state encoding would need more than an offset to resume at.

## Composing with a coding

A coding wraps the encoded bytes, so it comes off first and the charset second. Both are declared on one media type.

```rust
use yggdryl::charset::Transcoded;
use yggdryl::coding::Coded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, Codec, IOBase, MediaType};

let body = "symbol,désk\n";
let wire = Charset::Cp1252.encode(body)?.into_owned();
let source = Buffer::from_bytes(Codec::Gzip.dump(&wire)?).with_media_type(
    MediaType::from_str("text/csv;charset=windows-1252;encodings=application/gzip")?,
);

let decompressed = Coded::infer(source);
assert_eq!(decompressed.read_all_bytes()?, wire);
// The coding changed how the bytes were packed, not what encoding they are.
assert_eq!(decompressed.media_type().charset(), Some(Charset::Cp1252));

let text = Transcoded::infer(decompressed);
assert_eq!(text.read_all_bytes()?, body.as_bytes());
```

## Edges

- A write through `Transcoded` -> held until `flush`, `close`, or `into_handle`, then encoded whole; a scalar the charset has no byte for is the `Err` from that call.
- A write, then a read -> the index the previous value built is dropped, because it was a claim about bytes that are gone.
- `Transcoded::infer` over a handle declaring no charset -> `Charset::Utf8`, which changes nothing.
- A positional read past the end -> empty, exactly as `IOBase::pread` spells it everywhere else.
- `size` -> a read, not a forward of the wrapped size: a charset changes how many bytes a scalar takes. It is the pass the index already needed, so asking for both costs one.

## Commands

```bash
cargo test --features "parquet iceberg" -p yggdryl --test charset handles::
cargo test -p yggdryl --test iobase_calls a_random_read
cargo bench -p yggdryl --bench charset -- charset_streaming
```
