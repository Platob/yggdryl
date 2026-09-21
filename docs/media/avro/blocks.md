# Avro blocks, codecs, and limits

What one write emits and what one read is allowed to spend: the block codec, the synchronization marker, and the bounds every decode path carries.

## Contract

| Item | Behaviour |
| --- | --- |
| Owns | `avro::AvroOptions` block settings, and the `avro_block_codec` / `avro_sync_marker` pair the generic [`RecordOptions`](../options.md) exposes |
| Block codec | `null`, `deflate` (default), `zstandard`; `snappy` in builds with the `parquet` feature |
| Sync marker | absent - a fresh marker per write - or exactly 16 bytes, for byte-reproducible output |
| Validated | the codec name is checked before a row source is pulled, and a header naming a codec this build does not implement is refused by name |
| Blocks | each block carries its row count and its compressed payload; a reader jumps a block it never asks for ([Read](read.md#streaming-a-large-container)) |
| Limits | input bytes bound the container and each decompressed block, depth bounds schema and datum nesting, and the node budget bounds rows and per-datum allocation |
| Bindings | Python snake-case keywords on every decode, JavaScript camel-case options; Python `RecordOptions.block_codec` / `.sync_marker`, JavaScript `withBlockCodec` / `withSyncMarker` |

## Use

The generic [`RecordOptions`](../options.md) exposes both Avro settings without downcasting, and the writer validates the codec name before pulling a row source.

=== "Rust"

    ```rust
    use yggdryl::media::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::AVRO)?;
    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("zstandard")?;
    options.set_avro_sync_marker(Some(b"0123456789abcdef"))?;
    assert_eq!(options.avro_sync_marker(), Some(b"0123456789abcdef"));
    ```

=== "Python"

    ```python
    from yggdryl import RecordOptions

    options = RecordOptions("trades.avro")
    assert options.block_codec == "deflate"
    assert options.sync_marker is None

    options.block_codec = "zstandard"
    options.sync_marker = b"0123456789abcdef"
    assert options.sync_marker == b"0123456789abcdef"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { RecordOptions } = require('yggdryl')

    const options = RecordOptions.from('trades.avro')
      .withBlockCodec('zstandard')
      .withSyncMarker(Buffer.from('0123456789abcdef'))

    assert.equal(options.blockCodec, 'zstandard')
    assert.deepEqual(options.syncMarker, Buffer.from('0123456789abcdef'))
    ```

A fixed marker is what makes a write byte-reproducible: the same rows under the same codec produce the same container, so a conformance check can diff bytes rather than only semantics. An absent marker draws a fresh one per write; the `Scalar` surface's [`write_container`](write.md) derives one from the schema and the encoded rows instead.

## Codecs and limits

Input bytes bound the container and each decompressed block, depth bounds schema and datum nesting, and the node budget bounds rows and per-datum allocation.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Limits, Scalar};
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(r#""long""#)?;
    let mut bytes = Buffer::new();
    avro::write_container(&mut bytes, &schema, &[], &[Scalar::from(7_i64)])?;

    let limits = Limits::new(8, 1_024, 8, 1);
    assert_eq!(
        avro::read_container_with_limits(&bytes, limits)?.rows,
        [Scalar::from(7_i64)]
    );
    ```

=== "Python"

    ```python
    from yggdryl import avro

    encoded = avro.dumps([7], '"long"')
    decoded = avro.loads(
        encoded,
        max_depth=8,
        max_input_bytes=1_024,
        max_nodes=8,
    )

    assert decoded.rows == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const encoded = avro.dumps([7], '"long"')
    const decoded = avro.loads(encoded, {
      maxDepth: 8,
      maxInputBytes: 1024,
      maxNodes: 8,
    })

    assert.deepEqual(decoded.rows, [7])
    ```

| header codec | implementation |
| --- | --- |
| `null`, `deflate`, `zstandard` | The crate's own [`Codec`](../../coding/index.md) implementations |
| `snappy` | Raw Snappy followed by a big-endian CRC-32 of the uncompressed block; builds carrying the `parquet` feature |
| `bzip2`, `xz`, any other name | Refused, naming it and listing what this build implements |

Avro's `deflate` is the raw stream, with no zlib wrapper, and a compressed block is decompressed through a hard ceiling, so a small block cannot decompress the process to death.

## Edges

- `set_avro_block_codec` or `set_avro_sync_marker` on options for another encoding -> typed record error.
- a sync marker of any length but 16 bytes -> refused; an absent marker generates a fresh one per write.
- a codec name this build does not implement -> refused, naming it and listing the ones it has; `snappy` needs the `parquet` feature.
- a snappy block shorter than its four-byte CRC-32, or one whose CRC does not match -> refused rather than decoded.
- a block that decompresses past the input bound -> refused at the ceiling, not after the allocation.
- the header's marker missing after a block -> refused, so a truncated or spliced container is caught at the block that lost it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test media avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro_blocks
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    ```
