# Hashing

`yggdryl::xxhash` digests bytes, values, handles, and Arrow rows with XXH32, XXH64, XXH3-64, and XXH3-128, and `yggdryl::txhash` couples a UTC instant with one of those digests into one sortable value; `hashing/` holds only the stable-hash adapters the two share.

## Contract

| Key | Value |
| --- | --- |
| Owner | `xxhash/` and `txhash/` are the two implementation folders at the crate root, `hashing/` the private adapters they share, and there is no second dispatcher; `Digest`, `DigestAlgorithm`, and `Digester` stay root vocabulary. Python `yggdryl.hashing.xxhash` / `yggdryl.hashing.txhash` and JavaScript `hashing.xxhash` / `hashing.txhash` are the host paths; no other module path is kept (JavaScript also exports the classes they carry at top level: `Digest`, `Xxh32`, `Xxh64`, `Xxh3`, `Xxh128`, `TxHash`, `TxHasher`). |
| `xxhash` owns | `xxh32`, `xxh64`, `xxh3`, `xxh128` and their `_with_seed` forms (the XXH3 pair also `_with_secret` and `_with_seed_and_secret`), `digest`, `SECRET_MINIMUM_LENGTH`, the `Xxh32`, `Xxh64`, `Xxh3`, `Xxh128` states, `reader` / `writer`, `Hashed<H>`, `arrow::row_digests` / `column_digests`, and the value methods `as_value_bytes`, `write_bytes`, `digest`, `stable_hash` |
| `txhash` owns | `TxHash`, `TxHasher`, `txh32`, `txh64`, `txh3`, `txh128`, `digest`, `restate_unix`, `unix_from_scalar`, `unix_now`, `width`, `dtype`, `Scalar::txhash`, `arrow::unix_array` / `row_txhashes` / `column_txhashes` / `compose` / `decompose`; `TxHasher::row_txhashes` / `column_txhashes` / `apply_arrow_batch`, the same columns and holder fill under the hasher's algorithm, seed and secret, and for the two columns its unit; and `DIGEST:time` / `DIGEST:unit` on a holder |
| Algorithms | `DigestAlgorithm::ALL`: `xxh32`, `xxh64`, `xxh3-64`, `xxh3-128`; `width()` 4, 8, 8, 16 bytes; XXH3-64 is the default and what every `stable_hash` answers |
| Arguments | Input first; the seed or the instant second, everywhere |
| `Digest` | The algorithm carried with the number; `DigestAlgorithm` dispatches at runtime, as [`Codec`](coding/index.md) does for [gzip](coding/gzip.md). Spelled `<algorithm>:<hex>`, `from_str` the exact inverse; `into_bytes` is the canonical big-endian form, the reference's `XXH*_canonicalFromHash`; two algorithms are never equal |
| Seeds and secrets | A seed: every algorithm. A custom secret: the XXH3 pair only (`is_secretable`), consulted only for inputs longer than 240 bytes, at least `SECRET_MINIMUM_LENGTH` (136) bytes |
| States | Any split of the same bytes answers the one-shot digest; reading the digest leaves the state running; `clear()` returns to the constructed seed and secret; each Rust state is a `std::hash::Hasher` and its own `BuildHasher` |
| Handles | `read_digest` and `read_range_digest` on every `IOBase`, inherited unchanged by every backend and wrapper; streamed through [`pstream_bytes`](holder/iobase/bytes.md), retaining one bounded chunk, never calling `read_all_bytes`; a missing resource digests as no bytes, per the laziness contract; a container is a typed `Error::NotAtomic` naming the kind, and folder and recursive digests are absent; a wrapper answers for the bytes it presents, `handle()` for the bytes it holds |
| `Hashed<H>` | Answers from its running state and never reads the bytes back, but only while that state covers the whole value: writes strictly sequential from offset 0, counted after `flush`. A positional write makes the next digest re-stream and re-arm, with an identical answer |
| Value feed | `write_bytes` is a total prefix-free feed: one [`DataTypeId`](types/datatype.md) tag byte, then the family's canonical form, integers little-endian ([Encoding](#encoding)); `as_value_bytes` is the payload alone - no tag, no length - borrowed, never allocating, `None` for `Null`, `Sequence`, `Mapping`, `Record` |
| `stable_hash` | XXH3-64 over the feed; [`Field`](types/field.md), [`Uri`](uri/index.md), `DataType`, `MimeType`, and Iceberg values hash their canonical rendering the same way |
| Row digests | `row_digests` hashes the selected values as one `Scalar::Sequence` through `write_bytes`, on every datatype family except `variant`: nulls, nesting, dictionaries, unions, run-end encodings, geospatial. The column is `UInt32` for XXH32, `UInt64` for XXH64 and XXH3-64, `FixedSizeBinary(16)` big-endian for XXH3-128 |
| Holders | A holder's `DIGEST:sources` selects relative to its own Struct, `["*"]` and absence both meaning every field except a `DIGEST:role=holder`; `apply_arrow_batch` fills every holder under a non-null Struct root, and a state's running digest is untouched ([Digest holders](#digest-holders-and-row-digests)) |
| `TxHash` | `unit`, `unix`, `digest`; the digest is exactly what `xxhash` answers for the same bytes, so `txhash` defines no second hash. Spelled `<unix>@<unit>:<algorithm>:<hex>`, `from_str` the exact inverse; two units or two algorithms are never equal |
| TxHash bytes | The instant as a big-endian `i64`, then the digest's canonical bytes: 12, 16, or 24 bytes for XXH32, the two 64-bit algorithms, XXH3-128. Stored as `fixed_size_binary[12|16|24]`; sixteen bytes imply XXH3-64, the project default |
| Order | A value compares unit, then signed count, then digest, and never normalizes instants across units. Its bytes sort by time only within one unit, one algorithm, and one sign range: every negative count sorts after every nonnegative one ([Order](#order-and-uuidv7-projection)) |
| Instant | UTC always: a zoned datetime already counts from the epoch, a naive one reads as if it were UTC, a date is its midnight. The unit is a clock resolution - `s`, `ms`, `us`, `ns` - microseconds when none is named; a coarser restatement floors, a finer one scales exactly |
| UUIDv7 | `TxHash::into_uuid` is RFC 9562 UUIDv7, what `Uuid::from_v7` packs: the instant restated to signed nanoseconds and floored to the microsecond - the Unix millisecond in the leading 48 bits, the twelve-bit sub-millisecond fraction behind the version - then the digest's low 62 bits. It needs a 64-bit digest and an instant from the epoch to the 48-bit millisecond count, encodes neither unit nor algorithm, allocates nothing on success, and orders by instant to the microsecond; a lossy fingerprint, not an inverse, and the raw bytes and value order do not change |
| Ordered bytes | `TxHash::into_ordered_bytes` is the same ordering with nothing spent on a layout: the instant restated to signed nanoseconds with its sign bit flipped in bytes 0..8, then all 64 digest bits in bytes 8..16. It needs a 64-bit digest, encodes neither unit nor algorithm, and is what a `fixed[16]` column holds where `into_uuid` would have given an identifier. Rust-only |
| Coupled columns | `txhash::arrow` answers `fixed_size_binary(12|16|24)`, one coupled value per row, whose digest half is exactly `row_digests` or `column_digests` of the same rows under the same algorithm; the instant column is read once as `int64` counts at the declared unit, nulls kept; `compose` and `decompose` are inverses ([Coupled columns](#coupled-columns)) |
| Coupled holders | `DIGEST:time` names the field whose instant a holder stores in front of its digest; `DIGEST:unit` is its resolution, microseconds when absent, and only beside `DIGEST:time` ([Coupled holders](#coupled-holders)) |
| FIX identities | a message's `currhashcode` is the XXH3-64 of what the message *states* but the standard header and trailer, less `MsgType(35)` - the event's own facts, the names it goes by, its parents, its state and place, then the text, the metadata, `MsgType`, the FIX fields it lifted, then the entry tree - and never the frame a hop carried it in, never the chain it is in, whose bracketed cross code is one hop's, nor the columns a row happened to lay them out in, so a message read back out of a row is the same message and one logged at two hops is one message - and its `crosshashcode` the XXH3-64 of the code its chain shares; `curruuid` is the UUIDv7 [`TxHash`](#txhash-values) couples its instant and `currhashcode` into, and `crossuuid` the UUIDv8 of `crosshashcode`. Never a second engine, and the recipes live with [FIX messages](fix/message.md#typed-tags) and the [graph](graph.md) traits that derive them |
| Feature flag | none: `xxhash::arrow` and `txhash::arrow` are always compiled |
| Not | A cryptographic hash, an adversarial integrity check, or a uniqueness guarantee; not Iceberg `bucket[N]`, which is murmur3 x86_32 ([Iceberg](media/iceberg/index.md) never calls this module) |
| Bindings | Bytes: Python `bytes`, `bytearray`, `memoryview`, any buffer, `str` as UTF-8; JavaScript `Buffer`, `Uint8Array`, `ArrayBuffer`, string as UTF-8. Every `unix` argument is an `int` / `bigint`, a `datetime` / `Date`, timestamp text, or a `Scalar`. Handle digests, `Scalar.digest`, `stable_hash`, a state's `write_scalar` and `apply_arrow_batch`, `TxHasher`, and `TxHash.into_uuid` (a `uuid` `Scalar`) are bound everywhere; `Digester`, `as_value_bytes`, the digest arrays, and the coupled columns are Rust and Python only; `DigestReader`, `DigestWriter`, and `Hashed<H>` are Rust only |

## Use

The four one-shot functions answer their native widths with nothing wrapped around the number; a `Digest` carries the algorithm with it.

=== "Rust"

    ```rust
    use yggdryl::xxhash;
    use yggdryl::{Digest, DigestAlgorithm};

    assert_eq!(xxhash::xxh32(b"abc"), 0x32d1_53ff);
    assert_eq!(xxhash::xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
    assert_eq!(xxhash::xxh3(b"abc"), 0x78af_5f94_892f_3950);
    assert_eq!(xxhash::xxh128(b"abc"), 0x06b0_5ab6_733a_6185_78af_5f94_892f_3950);
    // The input comes first and the seed second, here and everywhere.
    assert_ne!(xxhash::xxh64_with_seed(b"abc", 42), xxhash::xxh64(b"abc"));

    let digest = DigestAlgorithm::Xxh3.digest(b"abc");
    assert_eq!(digest.algorithm(), DigestAlgorithm::Xxh3);
    assert_eq!(digest.as_u64(), Some(xxhash::xxh3(b"abc")));
    assert_eq!(digest.to_string(), "xxh3-64:78af5f94892f3950");
    assert_eq!(Digest::from_str(&digest.to_string())?, digest);
    assert_eq!(digest.into_bytes().len(), DigestAlgorithm::Xxh3.width());

    // Two algorithms are never equal, whatever their payloads: `xxh64` and
    // `xxh3-64` are both 64 bits wide and answer different values.
    assert_ne!(
        Digest::new(DigestAlgorithm::Xxh64, 7),
        Digest::new(DigestAlgorithm::Xxh3, 7),
    );
    assert_eq!(
        DigestAlgorithm::ALL.map(DigestAlgorithm::as_str),
        ["xxh32", "xxh64", "xxh3-64", "xxh3-128"],
    );
    ```

=== "Python"

    ```python
    from yggdryl.hashing import xxhash

    assert xxhash.xxh32(b"abc") == 0x32D153FF
    assert xxhash.xxh64(b"abc") == 0x44BC2CF5AD770999
    assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
    assert xxhash.xxh128(b"abc") == 0x06B05AB6733A618578AF5F94892F3950
    # bytes, bytearray, memoryview, any buffer, or a str as its UTF-8.
    assert xxhash.xxh3("abc") == xxhash.xxh3(bytearray(b"abc"))
    assert xxhash.xxh64(b"abc", seed=42) != xxhash.xxh64(b"abc")

    digest = xxhash.digest(b"abc", "xxh3-64")
    assert digest.algorithm == "xxh3-64"
    assert int(digest) == xxhash.xxh3(b"abc")
    assert str(digest) == "xxh3-64:78af5f94892f3950"
    assert xxhash.Digest(str(digest)) == digest
    assert len(bytes(digest)) == digest.width == 8

    assert xxhash.Digest.from_int("xxh64", 7) != xxhash.Digest.from_int("xxh3-64", 7)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { hashing } = require('yggdryl')
    const { xxhash } = hashing

    const payload = Buffer.from('abc')
    // XXH32 answers a number - 32 bits always fit one exactly - and the wider
    // algorithms answer bigints.
    assert.equal(xxhash.xxh32(payload), 0x32d153ff)
    assert.equal(xxhash.xxh64(payload), 0x44bc2cf5ad770999n)
    assert.equal(xxhash.xxh3(payload), 0x78af5f94892f3950n)
    assert.equal(xxhash.xxh128(payload), 0x06b05ab6733a618578af5f94892f3950n)
    // Buffer, Uint8Array, ArrayBuffer, or a string as its UTF-8.
    assert.equal(xxhash.xxh3('abc'), xxhash.xxh3(new Uint8Array(payload)))
    assert.notEqual(xxhash.xxh64(payload, { seed: 42n }), xxhash.xxh64(payload))

    const digest = xxhash.digest(payload, 'xxh3-64')
    assert.equal(digest.algorithm, 'xxh3-64')
    assert.equal(digest.value(), xxhash.xxh3(payload))
    assert.equal(digest.toString(), 'xxh3-64:78af5f94892f3950')
    assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
    assert.equal(digest.bytes().length, digest.width)

    const seven64 = xxhash.Digest.from('xxh64:0000000000000007')
    assert.ok(!seven64.equals(xxhash.Digest.from('xxh3-64:0000000000000007')))
    ```

## Streaming

Feed bytes with `write_bytes` and read the digest at any commit boundary. `DigestAlgorithm::digester()` is the runtime-selected state, what `Encoder` is to `Codec`; Python spells it `xxhash.Digester(algorithm)`, and JavaScript picks a state class instead.

=== "Rust"

    ```rust
    use yggdryl::DigestAlgorithm;
    use yggdryl::xxhash::{Xxh3, xxh3};

    let payload = b"AAPL,187.23";
    for split in [1, 4, payload.len()] {
        let mut state = Xxh3::new();
        for chunk in payload.chunks(split) {
            state.write_bytes(chunk);
        }
        // The split never changes the answer.
        assert_eq!(state.as_u64(), xxh3(payload));
    }

    // Answering does not consume the state, so a running digest can be read at
    // every commit boundary rather than only at the end.
    let mut state = Xxh3::new();
    state.write_bytes(b"AAPL");
    assert_eq!(state.as_u64(), xxh3(b"AAPL"));
    state.write_bytes(b",187.23");
    assert_eq!(state.as_u64(), xxh3(payload));

    // An algorithm held in a variable pays one dispatch for the same answer.
    let mut digester = DigestAlgorithm::Xxh3.digester();
    digester.write_bytes(payload);
    assert_eq!(digester.as_digest(), state.as_digest());
    ```

=== "Python"

    ```python
    from yggdryl.hashing import xxhash

    payload = b"AAPL,187.23"
    for split in (1, 4, len(payload)):
        state = xxhash.Xxh3()
        for index in range(0, len(payload), split):
            state.write_bytes(payload[index : index + split])
        assert int(state.as_digest()) == xxhash.xxh3(payload)

    state = xxhash.Xxh3()
    state.write_bytes(b"AAPL")
    assert int(state.as_digest()) == xxhash.xxh3(b"AAPL")
    state.write_bytes(b",187.23")
    assert int(state.as_digest()) == xxhash.xxh3(payload)

    digester = xxhash.Digester("xxh3-64")
    digester.write_bytes(payload)
    assert digester.as_digest() == state.as_digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { hashing } = require('yggdryl')
    const { xxhash } = hashing

    const payload = Buffer.from('AAPL,187.23')
    for (const split of [1, 4, payload.length]) {
      const state = new xxhash.Xxh3()
      for (let index = 0; index < payload.length; index += split) {
        state.writeBytes(payload.subarray(index, index + split))
      }
      assert.equal(state.asDigest().value(), xxhash.xxh3(payload))
    }

    const state = new xxhash.Xxh3()
    state.writeBytes(Buffer.from('AAPL'))
    assert.equal(state.asDigest().value(), xxhash.xxh3('AAPL'))
    state.writeBytes(Buffer.from(',187.23'))
    assert.equal(state.asDigest().value(), xxhash.xxh3(payload))
    ```

## Seeds and secrets

The examples hash 241 bytes, past the cutoff where a custom secret is consulted. One-shot calls and states take the same arguments.

=== "Rust"

    ```rust
    use yggdryl::xxhash::{self, SECRET_MINIMUM_LENGTH, Xxh3};
    use yggdryl::{DigestAlgorithm, Error};

    assert!(!DigestAlgorithm::Xxh64.is_secretable());
    assert!(DigestAlgorithm::Xxh3.is_secretable());

    let secret = vec![0x5a_u8; SECRET_MINIMUM_LENGTH];
    let payload = vec![0x11_u8; 241];
    assert_ne!(
        xxhash::xxh3_with_secret(&payload, &secret)?,
        xxhash::xxh3(&payload),
    );
    // At or below the cutoff the secret is not consulted at all.
    assert_eq!(xxhash::xxh3_with_secret(b"AAPL", &secret)?, xxhash::xxh3(b"AAPL"));

    // A short secret is refused by length, whatever the payload: the reference
    // only consults a secret past its 240-byte cutoff, and a secret that is
    // sometimes used is worse than one that is refused.
    let short = vec![0x5a_u8; SECRET_MINIMUM_LENGTH - 1];
    let error = Xxh3::from_secret(&short).unwrap_err();
    assert!(matches!(error, Error::InvalidSecret { actual: 135, .. }));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl.hashing import xxhash

    assert not xxhash.is_secretable("xxh64")
    assert xxhash.is_secretable("xxh3-64")

    secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
    payload = bytes(241)
    assert xxhash.xxh3(payload, secret=secret) != xxhash.xxh3(payload)
    # At or below the cutoff the secret is not consulted at all.
    assert xxhash.xxh3(b"AAPL", secret=secret) == xxhash.xxh3(b"AAPL")

    with pytest.raises(ValueError, match="at least 136 bytes, got 135"):
        xxhash.Xxh3(secret=bytes(xxhash.SECRET_MINIMUM_LENGTH - 1))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { hashing } = require('yggdryl')
    const { xxhash } = hashing

    const payload = Buffer.alloc(241)
    const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
    assert.notEqual(xxhash.xxh3(payload, { secret }), xxhash.xxh3(payload))
    // At or below the cutoff the secret is not consulted at all.
    const brief = Buffer.from('AAPL')
    assert.equal(xxhash.xxh3(brief, { secret }), xxhash.xxh3(brief))

    const truncated = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH - 1)
    assert.throws(
      () => xxhash.xxh3(payload, { secret: truncated }),
      /at least 136 bytes, got 135/,
    )
    ```

## Handles

Digest an `IOBase` handle's bytes without reading them whole.

=== "Rust"

    ```rust
    use yggdryl::xxhash;
    use yggdryl::holder::Buffer;
    use yggdryl::{DigestAlgorithm, IOBase};

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"AAPL,187.23\n")?;
    assert_eq!(
        handle.read_digest(DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"AAPL,187.23\n"),
    );
    assert_eq!(
        handle.read_range_digest(0, 4, DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"AAPL"),
    );

    // A resource with no bytes digests as no bytes, per the laziness contract -
    // absence is emptiness, not a third answer to branch on.
    assert_eq!(
        Buffer::new().read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
        Some(xxhash::xxh3(b"")),
    );
    ```

=== "Python"

    ```python
    import tempfile
    from pathlib import Path

    from yggdryl import IOBase
    from yggdryl.hashing import xxhash

    with tempfile.TemporaryDirectory() as root:
        path = Path(root) / "trades.csv"
        path.write_bytes(b"AAPL,187.23\n")

        handle = IOBase(path)
        assert handle.read_digest("xxh3-64") == xxhash.digest(b"AAPL,187.23\n", "xxh3-64")
        assert handle.read_range_digest(0, 4) == xxhash.digest(b"AAPL", "xxh3-64")

        missing = IOBase(Path(root) / "never-written.csv")
        assert missing.read_digest() == xxhash.digest(b"", "xxh3-64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, hashing } = require('yggdryl')
    const { xxhash } = hashing

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-digest-'))
    try {
      const file = path.join(root, 'trades.csv')
      const payload = Buffer.from('AAPL,187.23\n')
      fs.writeFileSync(file, payload)

      const handle = new IOBase(file)
      assert.ok(handle.readDigest('xxh3-64').equals(xxhash.digest(payload, 'xxh3-64')))
      assert.ok(handle.readRangeDigest(0, 4).equals(xxhash.digest('AAPL', 'xxh3-64')))

      const missing = new IOBase(path.join(root, 'never-written.csv'))
      assert.ok(missing.readDigest().equals(xxhash.digest(Buffer.alloc(0), 'xxh3-64')))
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

### Wrappers and write-through

Rust only: `DigestReader`, `DigestWriter`, and `Hashed<H>` build on `Read`, `Write`, and the handle they wrap, and a [coding](coding/index.md) wrapper and its backing handle answer different questions.

```rust
use std::io::{Read, Write};

use yggdryl::gzip::Gzip;
use yggdryl::xxhash::{self, Hashed};
use yggdryl::holder::Buffer;
use yggdryl::{DigestAlgorithm, IOBase};

let payload = b"AAPL,187.23\n";

// A coding wrapper answers for the bytes it presents; the handle underneath
// answers for the bytes it holds.
let mut coded = Gzip::new(Buffer::new());
coded.write_all_bytes(payload)?;
coded.flush()?;
assert_eq!(coded.read_digest(DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(payload));
let compressed = coded.handle().read_all_bytes()?;
assert_eq!(
    coded.handle().read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(&compressed),
);
assert_ne!(compressed.as_slice(), payload.as_slice());

// Hash a payload that is already being moved, in the pass that was already
// happening, rather than reading it a second time.
let mut source = xxhash::reader(payload.as_slice(), DigestAlgorithm::Xxh3);
let mut moved = Vec::new();
source.read_to_end(&mut moved)?;
assert_eq!(moved, payload);
assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3.digest(payload));
let mut target = xxhash::writer(Vec::new(), DigestAlgorithm::Xxh64);
target.write_all(payload)?;
assert_eq!(target.as_digest(), DigestAlgorithm::Xxh64.digest(payload));
assert_eq!(target.into_inner(), payload);

// Sequential writes from offset 0: answered from the running state, and the
// bytes are never read back.
let mut hashed = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
hashed.write_all_bytes(b"AAPL,")?;
hashed.append_bytes(b"187.23\n")?;
hashed.flush()?;
assert_eq!(hashed.read_digest(DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(payload));

// A positional write the running state cannot follow makes it stale, which is
// not an error and not silent corruption: the next digest re-streams the
// handle and re-arms, and the answer is identical either way.
hashed.pwrite_all(0, b"MSFT")?;
assert_eq!(
    hashed.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(&hashed.read_all_bytes()?),
);
```

## Values

`as_value_bytes` is the payload alone, so hashing it agrees with any xxHash over the same UTF-8; `write_bytes` frames every variant, and `digest` and `stable_hash` hash that frame. `stable_hash` is XXH3-64 over this feed everywhere; the tree has no second hash family and no second spelling.

=== "Rust"

    ```rust
    use yggdryl::xxhash::{self, Xxh3};
    use yggdryl::{DigestAlgorithm, Scalar};

    let symbol = Scalar::from("AAPL");
    assert_eq!(&*symbol.as_value_bytes().unwrap(), b"AAPL");
    // The feed frames the value, so its digest is not the bare UTF-8's.
    assert_ne!(symbol.digest(DigestAlgorithm::Xxh3).as_u64(), Some(xxhash::xxh3(b"AAPL")));
    assert_eq!(
        symbol.stable_hash(),
        symbol.digest(DigestAlgorithm::Xxh3).as_u64().unwrap(),
    );

    // Equal values answer one digest, across widths, which is the invariant
    // every binding relies on.
    assert_eq!(Scalar::from(1_i8), Scalar::from(1_i64));
    assert_eq!(Scalar::from(1_i8).stable_hash(), Scalar::from(1_i64).stable_hash());
    // Values that differ stay apart, across variant boundaries.
    assert_ne!(Scalar::from("1").stable_hash(), Scalar::from(0x31_u8).stable_hash());
    // A null and an empty string are not the same absence.
    assert_ne!(Scalar::Null.stable_hash(), Scalar::from("").stable_hash());

    // A state feeds a value the way the value digests itself.
    let mut state = Xxh3::new();
    state.write_scalar(&symbol);
    assert_eq!(state.as_digest(), symbol.digest(DigestAlgorithm::Xxh3));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar
    from yggdryl.hashing import xxhash

    symbol = Scalar.from_("AAPL")
    assert symbol.as_value_bytes() == b"AAPL"
    assert symbol.digest() == symbol.digest("xxh3-64")
    assert int(symbol.digest()) == symbol.stable_hash() != xxhash.xxh3(b"AAPL")

    # Equal values answer one digest, across widths.
    assert Scalar.decimal(100, 2).digest() == Scalar.decimal(1, 0).digest()
    assert DataType("float32").scalar(1.5).digest() == DataType("float64").scalar(1.5).digest()
    # Values that differ stay apart, across variant boundaries.
    assert Scalar.from_("1").digest() != Scalar.from_(b"1").digest()
    assert Scalar.from_(None).digest() != Scalar.from_("").digest()

    state = xxhash.Xxh3()
    state.write_scalar(symbol)
    assert state.as_digest() == symbol.digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, hashing } = require('yggdryl')
    const { xxhash } = hashing

    const symbol = Scalar.from('AAPL')
    assert.ok(symbol.digest().equals(symbol.digest('xxh3-64')))
    assert.equal(symbol.digest().value(), symbol.stableHash())
    assert.notEqual(symbol.stableHash(), xxhash.xxh3('AAPL'))

    // Equal values answer one digest, across widths.
    assert.ok(Scalar.decimal(100n, 2).digest().equals(Scalar.decimal(1n, 0).digest()))
    // Values that differ stay apart, across variant boundaries.
    assert.ok(!Scalar.from('1').digest().equals(Scalar.from(Buffer.from('1')).digest()))

    const state = new xxhash.Xxh3()
    state.writeScalar(symbol)
    assert.ok(state.asDigest().equals(symbol.digest()))
    ```

## Encoding

The tag byte is a wire contract laid out by family: every [`DataTypeKind`](types/datatype.md#identity-and-family) owns a range of bytes, its leaves sit in it and a leaf added later takes the next free byte of its family, so a stored digest never moves; the same byte is what the [value stream](types/value-stream.md) writes after its version. A digest identifies the value, not its storage width.

The tag is the value's own [`DataTypeId`](types/datatype.md), except where a family compares equal across its members and one member's tag then stands for all of them: integers feed `int128` or `uint128` by sign, floats and decimals feed their widest member, every [string](types/text.md) feeds `utf8` (`0x51`) whatever its leaf, and a geography feeds `geometry`. A [code](types/codes.md) feeds its own id - `country`, `currency`, `mic`, `cfi`, `isin`, `cusip`, `sedol`, `side`, `state`, `timeinforce` - so a `currency` and a `country` holding the same three bytes are two digests, as they are two values. Bytes feed `binary` whatever their layout.

The bytes moved once, together, when the identifiers were laid out by family: every stored digest of every value changed in that commit, and none has since; the family layout is what keeps the next leaf from moving any.

| Variant | Tag | Feed after the tag |
| --- | --- | --- |
| `Null` | `null` | nothing |
| `Bool` | `boolean` | `0x00` or `0x01` |
| `I8`..`U128` | `uint128`, or `int128` when negative | magnitude as `u128` little-endian |
| `F16`/`F32`/`F64` | `float64` | the common `f64` reading's IEEE bits, little-endian |
| `D32`..`D256` | `decimal256` | normalized coefficient as `i256` little-endian, then scale as one signed byte |
| `String` | `utf8` | length `u64` little-endian, then the characters as UTF-8; the leaf - charset, shape and fixed width - never feeds |
| a registered code | the code's own id | length `u64` little-endian, then the trimmed text |
| `Uuid` | `uuid` | the 16 big-endian bytes, with no length |
| `Version` | `version` | rendered length `u64` little-endian, then the canonical rendering |
| `Vocabulary` | `dictionary` | length-prefixed vocabulary identity, then the member ordinal |
| `Bytes` | `binary` | length `u64` little-endian, then the bytes |
| `Geospatial` | `geometry` | length `u64` little-endian, then the WKB |
| `Date32`/`Date64` | `date64` | unit class byte, normalized count as `i128` little-endian, length-prefixed timezone |
| `Time32`/`Time64` | `time64` | as above |
| `DateTime64` | `datetime64` | as above |
| `Duration32`/`Duration64` | `duration64` | as above |
| `Interval` | `interval` | months and days as `i32` little-endian, nanoseconds as `i64` little-endian, then the layout unit as one byte |
| `Sequence` | `list` | element count `u64` little-endian, then each element's feed |
| `Mapping` | `map` | entry count `u64` little-endian, then each key feed and value feed in stored order |
| `Struct` | `struct` | entry count `u64` little-endian, then per sorted entry a length-prefixed name and the value's feed |

## Digest holders and row digests

A digest holder is a field carrying `DIGEST:role=holder`; a state's `apply_arrow_batch` fills every holder the root declares with its own seed, secret, and a `force` switch, while `root.as_digest().apply_arrow_batch(&batch)` is the seedless form - the same [`stable_hash`](types/scalar.md) every other reader computes - that [`Field::apply_arrow_batch`](types/field.md#applying-a-schemas-declarations) runs beside the partition step. `row_digests` reads a batch rather than a holder: a row is the ordered `Scalar::Sequence` of its non-holder columns in schema order, element count included, and the answer never builds one.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::cast::AsArray as _;
    use arrow_array::types::UInt64Type;
    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::Schema;
    use yggdryl::xxhash::Xxh3;
    use yggdryl::xxhash::arrow::row_digests;
    use yggdryl::{DataType, DigestAlgorithm, Field, Scalar, StructType};

    let symbol = Field::new("symbol", DataType::utf8(), false);
    let quantity = Field::new("quantity", DataType::Int64, false);
    let mut holder = Field::new("row_digest", DataType::UInt64, false);
    holder.as_digest_mut().set_holder()?;
    holder.as_digest_mut().set_sources(["symbol"])?;
    let root = DataType::from(StructType::from_fields([symbol.clone(), quantity.clone(), holder])?)
        .required_field("row");

    // The batch has no holder column; the fill adds it where the root declares it.
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![symbol.into_arrow_field()?, quantity.into_arrow_field()?])),
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "AAPL"])),
            Arc::new(Int64Array::from(vec![100, 999])),
        ],
    )?;

    let mut state = Xxh3::with_seed(7);
    state.write_bytes(b"an unrelated running stream");
    let running = state.as_u64();
    let filled = state.apply_arrow_batch(&root, batch, false)?;
    assert_eq!(state.as_u64(), running, "filling does not consume the state");

    // `DIGEST:sources` narrows the fill to `symbol`, under the state's seed.
    let mut expected = Xxh3::with_seed(7);
    expected.write_scalar(&Scalar::from_sequence([Scalar::from("AAPL")]));
    let cells = filled.column(2).as_primitive::<UInt64Type>();
    assert_eq!(cells.values(), &[expected.as_u64(), expected.as_u64()]);

    // `row_digests` takes every column but the holder, so the quantity splits
    // the rows, and the stored holder value never feeds.
    let rows = row_digests(&filled, DigestAlgorithm::Xxh3)?;
    let rows = rows.as_primitive::<UInt64Type>();
    let first = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]);
    assert_eq!(Some(rows.value(0)), first.digest(DigestAlgorithm::Xxh3).as_u64());
    assert_ne!(rows.value(0), rows.value(1));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, Scalar
    from yggdryl.hashing import xxhash

    holder = Field("row_digest", "uint64", nullable=False)
    holder.digest.set_holder()
    holder.digest.sources = ["symbol"]
    root = Field(
        "row",
        DataType.from_fields(
            [Field("symbol", "utf8", nullable=False), Field("quantity", "int64", nullable=False), holder]
        ),
        nullable=False,
    )
    batch = pa.record_batch({"symbol": pa.array(["AAPL", "AAPL"]), "quantity": pa.array([100, 999])})

    state = xxhash.Xxh3(seed=7)
    state.write_bytes(b"an unrelated running stream")
    running = state.as_digest()
    filled = state.apply_arrow_batch(root, batch)
    assert state.as_digest() == running

    expected = xxhash.Xxh3(seed=7)
    expected.write_scalar(Scalar.from_(["AAPL"]))
    assert filled.column("row_digest").to_pylist() == [int(expected.as_digest())] * 2

    rows = xxhash.row_digests(filled)
    assert rows[0].as_py() == int(Scalar.from_(["AAPL", 100]).digest())
    assert rows[0].as_py() != rows[1].as_py()
    ```

=== "JavaScript"

    `row_digests` and `column_digests` are Rust and Python only; a state fills holders through a copied Arrow IPC batch.

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, Field, Scalar, hashing } = require('yggdryl')
    const { xxhash } = hashing

    const holder = new Field('row_digest', 'uint64', false, {
      'DIGEST:role': 'holder',
      'DIGEST:sources': '["symbol"]',
    })
    const root = new Field(
      'row',
      DataType.fromFields([new Field('symbol', 'utf8', false), new Field('quantity', 'int64', false), holder]),
      false,
    )
    const batch = new arrow.Table({
      symbol: arrow.vectorFromArray(['AAPL', 'AAPL'], new arrow.Utf8()),
      quantity: arrow.vectorFromArray([100n, 999n], new arrow.Int64()),
    }).batches[0]

    const state = new xxhash.Xxh3(7n)
    state.writeBytes('an unrelated running stream')
    const running = state.asDigest()
    const filled = state.applyArrowBatch(root, batch)
    assert.ok(state.asDigest().equals(running))

    const expected = new xxhash.Xxh3(7n)
    expected.writeScalar(Scalar.from(['AAPL']))
    const digest = expected.asDigest().value()
    assert.deepEqual([...filled.getChild('row_digest')], [digest, digest])
    ```

Each visible row is framed as an ordered `Scalar::Sequence` and streamed through the canonical value feed. Nested Struct holders are filled deepest first.

| Holder setting | Effect |
| --- | --- |
| `DIGEST:sources` | canonical JSON array of unique non-empty paths, for example `["id","line.price"]`; its order is the feed order, and it states nothing on the fields it names |
| Source syntax | relative to the containing Struct: an exact whole field name wins, then dots descend through Struct fields only |
| `["*"]` or no `DIGEST:sources` | every field of the containing Struct except a holder; `[]` hashes an empty sequence, and `"*"` beside a path is refused |
| Selected nested Struct with one direct holder | feeds that holder's digest payload instead of hashing the Struct again, which is the bypass a nested holder earns |
| `DIGEST:algorithm` | `xxh32`, `xxh64`, `xxh3-64`, or `xxh3-128`; it must fit the holder's storage mapping |
| `DIGEST:time`, `DIGEST:unit` | the holder stores an instant in front of its digest and is a `fixed_size_binary` of the coupled width; [Coupled holders](#coupled-holders) owns the layout |
| No `DIGEST:algorithm` | a receiver whose output width fits the holder, with its seed and secret |
| Fresh default by holder type | `int32`/`uint32` picks XXH32, `int64`/`uint64` picks XXH3-64, `fixed_size_binary(16)` picks XXH3-128 |
| `force=false` | a cell equal to the holder `Field`'s default is computed, every non-default value preserved |
| `force=true` | every visible holder is recomputed |
| `root.as_digest().apply_arrow_batch` | the seedless form: no configuration crosses into it, and `force` is never on |
| Bindings | `field.digest.apply_arrow_batch(batch)` and `field.apply_arrow_batch(batch)` in Python; `state.apply_arrow_batch(root, batch, force=True)`; `state.applyArrowBatch(root, batch, true)` copies the batch through Arrow IPC |

| Schema | Selected values for `row_digests` |
| --- | --- |
| Any schema | every field except a `DIGEST:role=holder`, in schema order |
| Only holders | the empty sequence, for every row |

`row_digests` reads a batch, not a holder, so it always takes that whole selection; narrowing is a holder's `DIGEST:sources` and belongs to `apply_arrow_batch`. Names, roles, and other metadata choose the values but never enter the byte feed. The [`DigestField` selection helpers](types/protocol.md) answer the same set without hashing a batch.

`row_digests` always uses its `algorithm` argument and the selection above. `column_digests` is the single-column form, each answer the cell's own value with no row framing.

## TxHash values

The four one-shots couple a microsecond instant with the plain digest of a buffer. The spelling names the unit and the algorithm, because both are part of the value: without them a microsecond and a nanosecond count of the same digits would share one rendering.

=== "Rust"

    ```rust
    use yggdryl::txhash::{self, TxHash};
    use yggdryl::xxhash;
    use yggdryl::{DigestAlgorithm, Scalar, TimeUnit, Timezone};

    let instant = 1_700_000_000_000_000; // 2023-11-14T22:13:20Z in microseconds
    let value = txhash::txh3(b"AAPL", instant);
    assert_eq!((value.unix(), value.unit(), value.width()), (instant, TimeUnit::Microsecond, 16));
    assert_eq!(value.digest(), DigestAlgorithm::Xxh3.digest(b"AAPL"));

    // The bytes are the instant big-endian, then the digest's canonical bytes.
    let bytes = value.into_bytes();
    assert_eq!(&bytes[..8], &instant.to_be_bytes());
    assert_eq!(&bytes[8..], &xxhash::xxh3(b"AAPL").to_be_bytes());
    assert_eq!(TxHash::from_bytes(TimeUnit::Microsecond, DigestAlgorithm::Xxh3, &bytes)?, value);

    let spelled = txhash::digest(b"abc", 0, DigestAlgorithm::Xxh128);
    assert_eq!(spelled.to_string(), "0@us:xxh3-128:06b05ab6733a618578af5f94892f3950");
    assert_eq!(TxHash::from_str(&spelled.to_string())?, spelled);

    // A coarser unit floors the instant and keeps the digest.
    let seconds = txhash::txh3(b"AAPL", instant + 999_999).with_unit(TimeUnit::Second)?;
    assert_eq!(seconds.unix(), 1_700_000_000);
    assert_eq!(seconds.digest(), value.digest());
    assert_eq!(
        seconds.into_datetime(),
        Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC)?,
    );
    // Two units, like two algorithms, are never equal.
    assert_ne!(seconds, seconds.with_unit(TimeUnit::Millisecond)?);
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl.hashing import txhash, xxhash

    instant = 1_700_000_000_000_000  # 2023-11-14T22:13:20Z in microseconds
    value = txhash.txh3(b"AAPL", instant)
    assert (value.unix, value.unit, value.width) == (instant, "us", 16)
    assert int(value.digest) == xxhash.xxh3(b"AAPL")

    # The bytes are the instant big-endian, then the digest's canonical bytes.
    assert bytes(value) == instant.to_bytes(8, "big", signed=True) + bytes(value.digest)
    assert txhash.TxHash.from_bytes("us", "xxh3-64", bytes(value)) == value

    spelled = txhash.digest(b"abc", 0, "xxh3-128")
    assert str(spelled) == "0@us:xxh3-128:06b05ab6733a618578af5f94892f3950"
    assert txhash.TxHash(str(spelled)) == spelled

    # A coarser unit floors the instant and keeps the digest.
    seconds = txhash.txh3(b"AAPL", instant + 999_999).with_unit("s")
    assert seconds.unix == 1_700_000_000
    assert seconds.digest == value.digest
    assert seconds.into_datetime().as_py() == dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    # Two units, like two algorithms, are never equal.
    assert seconds != seconds.with_unit("ms")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar, hashing } = require('yggdryl')
    const { txhash, xxhash } = hashing

    const instant = 1_700_000_000_000_000n // 2023-11-14T22:13:20Z in microseconds
    const value = txhash.txh3('AAPL', instant)
    assert.deepEqual([value.unix, value.unit, value.width], [instant, 'us', 16])
    assert.equal(value.digest.value(), xxhash.xxh3('AAPL'))

    // The bytes are the instant big-endian, then the digest's canonical bytes.
    const bytes = Buffer.from(value.bytes())
    assert.equal(bytes.readBigInt64BE(0), instant)
    assert.deepEqual(bytes.subarray(8), Buffer.from(value.digest.bytes()))
    assert.ok(txhash.TxHash.fromBytes('us', 'xxh3-64', value.bytes()).equals(value))

    const spelled = txhash.digest('abc', 0, 'xxh3-128')
    assert.equal(spelled.toString(), '0@us:xxh3-128:06b05ab6733a618578af5f94892f3950')
    assert.ok(txhash.TxHash.from(spelled.toString()).equals(spelled))

    // A coarser unit floors the instant and keeps the digest.
    const seconds = txhash.txh3('AAPL', instant + 999_999n).withUnit('s')
    assert.equal(seconds.unix, 1_700_000_000n)
    assert.ok(seconds.digest.equals(value.digest))
    assert.ok(
      seconds
        .intoDatetime()
        .equals(new DataType('datetime64(s,"UTC")').scalar(1_700_000_000n)),
    )
    // Two units, like two algorithms, are never equal.
    assert.ok(!seconds.equals(seconds.withUnit('ms')))
    ```

## Order and UUIDv7 projection

A value orders by unit, then signed count, then digest; its bytes agree with that order only within one unit, one algorithm, and one sign range. `into_uuid` restates the instant to signed nanoseconds, floors it to the microsecond and projects RFC 9562 UUIDv7 through [`Uuid::from_v7`](types/uuid.md), which any UUIDv7 reader reads the instant out of, orders by instant to the microsecond in every unit and keeps only the digest's low 62 bits; an instant before the epoch has no UUIDv7 and is refused. `into_ordered_bytes` is the same ordering with no layout over it - sixteen bytes, the flipped instant then the whole digest - for a holder that wants the order without the layout; a [FIX message](fix/message.md#typed-tags) takes the UUIDv7 instead, because its identity is a `uuid` column a reader can read.

=== "Rust"

    ```rust
    use yggdryl::txhash::{self, TxHash};
    use yggdryl::{Digest, DigestAlgorithm, Error, TimeUnit};

    let one = Digest::new(DigestAlgorithm::Xxh64, 1);
    let epoch = TxHash::new_in(0, TimeUnit::Nanosecond, one)?;
    let later = TxHash::new_in(1, TimeUnit::Nanosecond, one)?;
    assert!(epoch < later && epoch.into_bytes() < later.into_bytes());

    // Before the epoch a value still orders first, but its two's-complement
    // bytes sort after every nonnegative count.
    let before = TxHash::new_in(-1, TimeUnit::Nanosecond, one)?;
    assert!(before < epoch && before.into_bytes() > epoch.into_bytes());

    // The projection is a UUIDv7, ordered by instant to the microsecond;
    // before the epoch there is none to project.
    assert_eq!(epoch.into_uuid()?.to_string(), "00000000-0000-7000-8000-000000000001");
    let micro = TxHash::new_in(1, TimeUnit::Microsecond, one)?;
    assert!(epoch.into_uuid()? < micro.into_uuid()?);
    assert!(matches!(before.into_uuid().unwrap_err(), Error::InvalidRecord { ref path, .. } if path == "$"));

    // One instant in two units: two values, one UUID.
    let second = TxHash::new_in(1, TimeUnit::Second, one)?;
    let nanos = TxHash::new_in(1_000_000_000, TimeUnit::Nanosecond, one)?;
    assert_ne!(second, nanos);
    assert_eq!(second.into_uuid()?, nanos.into_uuid()?);

    // A digest that is not 64 bits wide is refused, never narrowed.
    let refused = txhash::txh128(b"AAPL", 0).into_uuid().unwrap_err();
    assert!(matches!(refused, Error::InvalidRecord { ref path, .. } if path == "$.digest"));

    // The same ordering as sixteen plain bytes: the flipped instant leads,
    // and all 64 digest bits follow. Rust-only.
    assert_eq!(
        epoch.into_ordered_bytes()?,
        [0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    );
    assert!(before.into_ordered_bytes()? < epoch.into_ordered_bytes()?);
    assert!(epoch.into_ordered_bytes()? < later.into_ordered_bytes()?);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl.hashing import txhash, xxhash

    one = xxhash.Digest.from_int("xxh64", 1)
    epoch = txhash.TxHash.from_parts(0, one, unit="ns")
    later = txhash.TxHash.from_parts(1, one, unit="ns")
    assert epoch < later and bytes(epoch) < bytes(later)

    # Before the epoch a value still orders first, but its bytes sort last.
    before = txhash.TxHash.from_parts(-1, one, unit="ns")
    assert before < epoch and bytes(before) > bytes(epoch)

    projected = epoch.into_uuid()
    assert projected.as_py() == "00000000-0000-7000-8000-000000000001"
    assert projected < txhash.TxHash.from_parts(1, one, unit="us").into_uuid()
    with pytest.raises(ValueError, match="UUIDv7"):
        before.into_uuid()

    # One instant in two units: two values, one UUID.
    second = txhash.TxHash.from_parts(1, one, unit="s")
    nanos = txhash.TxHash.from_parts(1_000_000_000, one, unit="ns")
    assert second != nanos
    assert second.into_uuid() == nanos.into_uuid()

    with pytest.raises(ValueError, match="expected a 64-bit digest for UUIDv7"):
        txhash.txh128(b"AAPL", 0).into_uuid()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { hashing } = require('yggdryl')
    const { txhash, xxhash } = hashing

    const one = xxhash.Digest.from('xxh64:0000000000000001')
    const bytesOf = (value) => Buffer.from(value.bytes())
    const epoch = txhash.TxHash.fromParts(0n, one, 'ns')
    const later = txhash.TxHash.fromParts(1n, one, 'ns')
    assert.equal(epoch.compare(later), -1)
    assert.ok(Buffer.compare(bytesOf(epoch), bytesOf(later)) < 0)

    // Before the epoch a value still orders first, but its bytes sort last.
    const before = txhash.TxHash.fromParts(-1n, one, 'ns')
    assert.equal(before.compare(epoch), -1)
    assert.ok(Buffer.compare(bytesOf(before), bytesOf(epoch)) > 0)

    assert.equal(epoch.intoUuid().asJs(), '00000000-0000-7000-8000-000000000001')
    assert.ok(epoch.intoUuid().asJs() < txhash.TxHash.fromParts(1n, one, 'us').intoUuid().asJs())
    assert.throws(() => before.intoUuid(), /UUIDv7/)

    // One instant in two units: two values, one UUID.
    const second = txhash.TxHash.fromParts(1n, one, 's')
    const nanos = txhash.TxHash.fromParts(1_000_000_000n, one, 'ns')
    assert.ok(!second.equals(nanos))
    assert.ok(second.intoUuid().equals(nanos.intoUuid()))

    assert.throws(() => txhash.txh128('AAPL', 0n).intoUuid(), /expected a 64-bit digest for UUIDv7/)
    ```

With `us` the instant floored to microseconds, the 128 bits are `(us / 1000) << 80 | 7 << 76 | ((us % 1000) * 4096 / 1000) << 64 | 0b10 << 62 | digest_low62`, exactly what `Uuid::from_v7` packs. Neither the unit nor the algorithm is encoded, so XXH64 and XXH3-64 with one payload project one UUID, and the sub-microsecond nanoseconds and the two discarded digest bits cannot be recovered. A [`uuid`](types/uuid.md) is what comes back: a value `Scalar` in Python and JavaScript.

`into_ordered_bytes` is the layout-free twin: `t` big-endian in bytes 0..8 and the whole 64-bit digest in bytes 8..16, so nothing is discarded and nothing is reserved. It answers `[u8; 16]`, a `fixed_size_binary(16)` column holds it, and it is Rust-only - the bindings reach it through the FIX identities that use it.

## Instants

Every spelling of an instant resolves to one unix count: an integer is the count already, a datetime is its instant whatever its zone, a date is its midnight, and text reads as a timestamp with or without an offset, or as a date.

=== "Rust"

    ```rust
    use yggdryl::txhash;
    use yggdryl::{Scalar, TimeUnit, Timezone};

    let unit = TimeUnit::Microsecond;
    let kolkata = Timezone::from_str("Asia/Kolkata")?;
    // A zoned datetime already counts from the epoch; the zone moves nothing.
    assert_eq!(
        txhash::unix_from_scalar(&Scalar::from_datetime(1_700_000_000, TimeUnit::Second, kolkata)?, unit)?,
        1_700_000_000_000_000,
    );
    assert_eq!(txhash::unix_from_scalar(&Scalar::from("1970-01-01T00:00:01+01:00"), unit)?, -3_599_000_000);
    assert_eq!(txhash::unix_from_scalar(&Scalar::date32(1), unit)?, 86_400_000_000);

    // A finer count floors, so two instants keep their order across units.
    assert_eq!(txhash::restate_unix(1_999, TimeUnit::Nanosecond, unit)?, 1);
    assert_eq!(txhash::restate_unix(-1, TimeUnit::Nanosecond, unit)?, -1);
    assert!(txhash::restate_unix(i64::MAX, TimeUnit::Second, unit).is_err());

    // The clock, at the unit asked for.
    assert!(txhash::unix_now(TimeUnit::Second)? > 1_700_000_000);
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl.hashing import txhash

    aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    kolkata = aware.astimezone(dt.timezone(dt.timedelta(hours=5, minutes=30)))
    # A zoned datetime already counts from the epoch; the zone moves nothing.
    assert txhash.unix_of(kolkata) == txhash.unix_of(aware) == 1_700_000_000_000_000
    assert txhash.unix_of("1970-01-01T00:00:01+01:00") == -3_599_000_000
    assert txhash.unix_of(dt.date(1970, 1, 2)) == 86_400_000_000
    assert txhash.unix_of(aware, unit="s") == 1_700_000_000

    # A finer count floors, so two instants keep their order across units.
    assert txhash.restate_unix(1_999, "ns", "us") == 1
    assert txhash.restate_unix(-1, "ns", "us") == -1

    # The clock, at the unit asked for.
    assert txhash.unix_now("s") > 1_700_000_000
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { hashing } = require('yggdryl')
    const { txhash } = hashing

    const aware = new Date('2023-11-14T22:13:20Z')
    // A Date is its UTC instant; text with an offset already counts from the epoch.
    assert.equal(txhash.unixOf(aware), 1_700_000_000_000_000n)
    assert.equal(txhash.unixOf('2023-11-15T03:43:20+05:30'), 1_700_000_000_000_000n)
    assert.equal(txhash.unixOf('1970-01-01T00:00:01+01:00'), -3_599_000_000n)
    assert.equal(txhash.unixOf(aware, 's'), 1_700_000_000n)

    // A finer count floors, so two instants keep their order across units.
    assert.equal(txhash.restateUnix(1_999n, 'ns', 'us'), 1n)
    assert.equal(txhash.restateUnix(-1, 'ns', 'us'), -1n)

    // The clock, at the unit asked for.
    assert.ok(txhash.unixNow('s') > 1_700_000_000n)
    ```

## TxHasher

`TxHasher` settles the resolution, algorithm, seed, and secret once; a secret reaches it through the configured state, as the states carry one.

=== "Rust"

    ```rust
    use yggdryl::txhash::TxHasher;
    use yggdryl::xxhash::{self, Xxh3};
    use yggdryl::{DigestAlgorithm, Scalar, TimeUnit};

    let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh64)?.with_seed(7);
    let value = seconds.digest(b"AAPL", 1_700_000_000);
    assert_eq!(value.unit(), TimeUnit::Second);
    assert_eq!(value.digest().as_u64(), Some(xxhash::xxh64_with_seed(b"AAPL", 7)));
    assert_eq!(seconds.unix_of(&Scalar::from("2023-11-14T22:13:20Z"))?, 1_700_000_000);

    // A value's canonical feed, under the same configuration.
    let row = seconds.digest_scalar(&Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100)]), 1_700_000_000);
    assert_ne!(row.digest(), value.digest());

    // A secret travels through the state that holds it.
    let secret = vec![0x5a_u8; xxhash::SECRET_MINIMUM_LENGTH];
    let secretive = TxHasher::from_digester(TimeUnit::Millisecond, Xxh3::from_secret(&secret)?.into())?;
    let long = vec![0x11_u8; 241];
    assert_eq!(secretive.digest(&long, 5).digest().as_u64(), Some(xxhash::xxh3_with_secret(&long, &secret)?));
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.hashing import txhash, xxhash

    seconds = txhash.TxHasher("xxh64", unit="s", seed=7)
    value = seconds.digest(b"AAPL", 1_700_000_000)
    assert value.unit == "s"
    assert int(value.digest) == xxhash.xxh64(b"AAPL", seed=7)
    assert seconds.unix_of("2023-11-14T22:13:20Z") == 1_700_000_000

    # A value's canonical feed, under the same configuration.
    row = seconds.digest_scalar(Scalar.from_(["AAPL", 100]), 1_700_000_000)
    assert row.digest != value.digest

    # A secret travels through the state that holds it.
    secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
    secretive = txhash.TxHasher.from_state(xxhash.Xxh3(secret=secret), unit="ms")
    long = b"\x11" * 241
    assert int(secretive.digest(long, 5).digest) == xxhash.xxh3(long, secret=secret)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, hashing } = require('yggdryl')
    const { txhash, xxhash } = hashing

    const seconds = new txhash.TxHasher('xxh64', 's', 7n)
    const value = seconds.digest('AAPL', 1_700_000_000n)
    assert.equal(value.unit, 's')
    assert.equal(value.digest.value(), xxhash.xxh64('AAPL', { seed: 7n }))
    assert.equal(seconds.unixOf('2023-11-14T22:13:20Z'), 1_700_000_000n)

    // A value's canonical feed, under the same configuration.
    const row = seconds.digestScalar(Scalar.from(['AAPL', 100]), 1_700_000_000n)
    assert.ok(!row.digest.equals(value.digest))

    // A secret travels through the state that holds it.
    const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
    const secretive = txhash.TxHasher.fromState(new xxhash.Xxh3(0n, secret), 'ms')
    const long = Buffer.alloc(241, 0x11)
    assert.equal(secretive.digest(long, 5n).digest.value(), xxhash.xxh3(long, { secret }))
    ```

## Coupled columns

A row's coupled value is its row digest with the instant beside it, so a coupled column and a plain digest column agree wherever they overlap. `unix_array` reads every instant column the way a scalar is read: a timestamp of any resolution or zone, a date, or an integer, restated at the unit asked for.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::cast::AsArray as _;
    use arrow_array::types::UInt64Type;
    use arrow_array::{
        Array, Date32Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
        TimestampNanosecondArray, TimestampSecondArray,
    };
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::txhash;
    use yggdryl::xxhash::arrow::row_digests;
    use yggdryl::{DigestAlgorithm, TimeUnit};

    let (unit, algorithm) = (TimeUnit::Microsecond, DigestAlgorithm::Xxh3);
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("symbol", DataType::Utf8, false)])),
        vec![Arc::new(StringArray::from(vec!["AAPL", "AAPL"]))],
    )?;
    let instants = TimestampMicrosecondArray::from(vec![1_700_000_000_000_000, 1_700_000_000_000_001])
        .with_timezone("UTC");

    let coupled = txhash::arrow::row_txhashes(&batch, &instants, unit, algorithm)?;
    assert_eq!(coupled.data_type(), &DataType::FixedSizeBinary(16));

    // The halves come back out: the instant column, and exactly the row digests.
    let (times, digests) = txhash::arrow::decompose(coupled.as_ref(), unit, algorithm)?;
    assert_eq!(&digests, &row_digests(&batch, algorithm)?);
    assert_eq!(times.as_ref(), &instants as &dyn Array);
    assert_eq!(&txhash::arrow::compose(times.as_ref(), digests.as_ref(), unit, algorithm)?, &coupled);
    // The same content at two instants: one digest, two coupled values.
    let halves = digests.as_primitive::<UInt64Type>();
    assert_eq!(halves.value(0), halves.value(1));
    assert_ne!(coupled.as_fixed_size_binary().value(0), coupled.as_fixed_size_binary().value(1));

    let micros = |array: &dyn Array| txhash::arrow::unix_array(array, unit);
    let zoned = TimestampSecondArray::from(vec![Some(1_700_000_000), None]).with_timezone("Asia/Kolkata");
    let read = micros(&zoned)?;
    // The zone moves nothing: the count already names the instant, and nulls stay.
    assert_eq!(read.value(0), 1_700_000_000_000_000);
    assert!(read.is_null(1));
    // A finer count floors, a date is its midnight, an integer is the count already.
    assert_eq!(micros(&TimestampNanosecondArray::from(vec![1_999, -1]))?.values(), &[1, -1]);
    assert_eq!(micros(&Date32Array::from(vec![1]))?.values(), &[86_400_000_000]);
    assert_eq!(micros(&Int64Array::from(vec![42]))?.values(), &[42]);
    ```

=== "Python"

    ```python
    import datetime as dt

    import pyarrow as pa

    from yggdryl.hashing import txhash, xxhash

    batch = pa.record_batch({"symbol": pa.array(["AAPL", "AAPL"])})
    instants = pa.array([1_700_000_000_000_000, 1_700_000_000_000_001], pa.timestamp("us", tz="UTC"))

    coupled = txhash.row_txhashes(batch, instants)
    assert coupled.type == pa.binary(16)

    # The halves come back out: the instant column, and exactly the row digests.
    times, digests = txhash.decompose(coupled)
    assert digests == xxhash.row_digests(batch)
    assert times == instants
    assert txhash.compose(times, digests) == coupled
    # The same content at two instants: one digest, two coupled values.
    assert digests[0] == digests[1] and coupled[0] != coupled[1]

    seconds = pa.array([1_700_000_000, None], pa.timestamp("s", tz="Asia/Kolkata"))
    # The zone moves nothing: the count already names the instant, and nulls stay.
    assert txhash.unix_array(seconds).to_pylist() == [1_700_000_000_000_000, None]
    assert txhash.unix_array(seconds, "s").to_pylist() == [1_700_000_000, None]
    # A finer count floors, a date is its midnight, an integer is the count already.
    assert txhash.unix_array(pa.array([1_999, -1], pa.timestamp("ns"))).to_pylist() == [1, -1]
    assert txhash.unix_array(pa.array([dt.date(1970, 1, 2)])).to_pylist() == [86_400_000_000]
    assert txhash.unix_array(pa.array([42], pa.int64())).to_pylist() == [42]
    ```

=== "JavaScript"

    The coupled columns are Rust and Python only. JavaScript couples one value at a time through [`TxHasher.digestScalar`](#txhasher), reads one instant through [`txhash.unixOf`](#instants), and fills coupled holders through [`TxHasher.applyArrowBatch`](#coupled-holders).

## Coupled holders

A holder naming `DIGEST:time` stores the instant it names in front of its digest. Everything else about it is the [digest holder contract](#digest-holders-and-row-digests): `DIGEST:sources` narrows what the digest reads, the instant column included by default; `DIGEST:algorithm` or the storage width picks the algorithm; a written cell is preserved unless forced.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array as _, RecordBatch, StringArray, TimestampMicrosecondArray};
    use arrow_schema::Schema;
    use yggdryl::txhash::TxHash;
    use yggdryl::{ArrowCastOptions, DataType, DigestAlgorithm, Field, Scalar, StructType, TimeUnit, Timezone};

    let event = Field::new("event", DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?, false);
    let symbol = Field::new("symbol", DataType::utf8(), false);
    let mut key = Field::new("key", DataType::fixed_binary(16)?, false);
    key.as_digest_mut().set_holder()?;
    key.as_digest_mut().set_time("event")?;
    key.as_digest_mut().set_unit(TimeUnit::Second)?;
    let root = DataType::from(StructType::from_fields([event.clone(), symbol.clone(), key])?).required_field("row");

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![event.into_arrow_field()?, symbol.into_arrow_field()?])),
        vec![
            Arc::new(TimestampMicrosecondArray::from(vec![1_700_000_000_000_000, 1_700_000_000_999_999]).with_timezone("UTC")),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )?;

    // The schema pipeline fills the holder, and filling again changes nothing.
    let filled = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
    assert_eq!(root.apply_arrow_batch(&filled, true, true, true, ArrowCastOptions::new())?, filled);

    let cells = filled.column(2).as_any().downcast_ref::<arrow_array::FixedSizeBinaryArray>().unwrap();
    let second = TxHash::from_bytes(TimeUnit::Second, DigestAlgorithm::Xxh3, cells.value(1))?;
    // The instant floors to the declared unit; the digest reads every field but the holder.
    assert_eq!(second.unix(), 1_700_000_000);
    let row = Scalar::from_sequence([
        Scalar::from_datetime(1_700_000_000_999_999, TimeUnit::Microsecond, Timezone::UTC)?,
        Scalar::from("MSFT"),
    ]);
    assert_eq!(second.digest(), row.digest(DigestAlgorithm::Xxh3));
    ```

=== "Python"

    ```python
    import datetime as dt

    import pyarrow as pa

    from yggdryl import DataType, Field, Scalar
    from yggdryl.hashing import txhash

    key = Field("key", "fixed_size_binary[16]", nullable=False)
    key.digest.set_holder()
    key.digest.time = "event"
    key.digest.unit = "s"
    root = Field(
        "row",
        DataType.from_fields([Field("event", "timestamp[us, UTC]", nullable=False), Field("symbol", "utf8", nullable=False), key]),
        nullable=False,
    )
    batch = pa.record_batch(
        {
            "event": pa.array([1_700_000_000_000_000, 1_700_000_000_999_999], pa.timestamp("us", tz="UTC")),
            "symbol": pa.array(["AAPL", "MSFT"]),
        }
    )

    # The schema pipeline fills the holder, and filling again changes nothing.
    filled = root.apply_arrow_batch(batch)
    assert root.apply_arrow_batch(filled) == filled

    second = txhash.TxHash.from_bytes("s", "xxh3-64", filled.column("key")[1].as_py())
    # The instant floors to the declared unit; the digest reads every field but the holder.
    assert second.unix == 1_700_000_000
    row = Scalar.from_([dt.datetime(2023, 11, 14, 22, 13, 20, 999_999, tzinfo=dt.timezone.utc), "MSFT"])
    assert second.digest == row.digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, Field, Scalar, hashing } = require('yggdryl')
    const { txhash } = hashing

    const key = new Field('key', 'fixed_size_binary[16]', false, {
      'DIGEST:role': 'holder',
      'DIGEST:time': 'event',
      'DIGEST:unit': 's',
    })
    const root = new Field(
      'row',
      DataType.fromFields([new Field('event', 'int64', false), new Field('symbol', 'utf8', false), key]),
      false,
    )
    const batch = new arrow.Table({
      event: arrow.vectorFromArray([1_700_000_000n, 1_700_000_001n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    }).batches[0]

    const filled = new txhash.TxHasher().applyArrowBatch(root, batch)
    const second = txhash.TxHash.fromBytes('s', 'xxh3-64', filled.getChild('key').get(1))
    // An integer instant is the count already; the digest reads every field but the holder.
    assert.equal(second.unix, 1_700_000_001n)
    assert.ok(second.digest.equals(Scalar.from([1_700_000_001n, 'MSFT']).digest()))
    ```

| Holder setting | Effect |
| --- | --- |
| `DIGEST:time` | one field path relative to the containing Struct, resolved as a source is; the field must be a datetime, a date, or an integer, never a holder |
| `DIGEST:unit` | `s`, `ms`, `us`, or `ns`, canonicalized on write; refused without `DIGEST:time`; absent means microseconds |
| storage | `fixed_size_binary[12]` for XXH32, `[16]` for XXH64 or XXH3-64, `[24]` for XXH3-128; `DIGEST:algorithm` must fit it |
| the instant column | feeds the digest like any other column unless `DIGEST:sources` leaves it out |
| a null instant | a nullable holder stores null; a required holder refuses, naming the row |
| removal order | `remove_unit`, then `remove_time`, then `remove_role`; each refuses while what depends on it stands |
| Bindings | Python takes and answers `pyarrow` arrays for every column function and fills through `field.apply_arrow_batch` or a `TxHasher`; JavaScript fills through `TxHasher.applyArrowBatch` |

## Edges

- Same payload, two algorithms -> never equal (`xxh64` vs `xxh3-64`).
- JavaScript `xxh32` -> a number; the wider three -> bigints; seed as `{ seed: 42n }`.
- Python `bytearray` / `memoryview` -> a bounded window, never borrowed; 1.7x slower than `bytes`.
- JavaScript string -> UTF-8 encoded on the way in; 7.5x slower than `Buffer`.
- One-byte call -> 166 ns Python, 496 ns Node of binding overhead; gone by 64 KiB.
- No `xxhash` C package in `python/.venv` -> `(C libxxhash)` rows skipped; `python/tests/hashing/xxhash` skipped by `pytest.importorskip`.
- An empty chunk -> contributes nothing, wherever it sits.
- A secret with a payload of 240 bytes or fewer -> never consulted; the derived secret and the seed answer.
- A secret below `SECRET_MINIMUM_LENGTH` -> refused whatever the payload: `Error::InvalidSecret { actual: 135, .. }`, `ValueError`, or `at least 136 bytes, got 135`.
- `clear()` -> keeps the constructed seed and secret; it never returns an unseeded state.
- `Hasher::finish` on `Xxh128` -> the low 64 bits; `as_u128` carries the full value.
- `apply_arrow_batch` on a running state -> fills the batch's holders and leaves the running digest unchanged.
- Missing resource -> the digest of no bytes, never an error.
- Container handle -> `Error::NotAtomic`; folder and recursive digests do not exist.
- Positional write into `Hashed<H>` -> stale state, and the next digest re-streams.
- `as_value_bytes` on a decimal or temporal -> coefficient or stored count at storage width; scale, unit, and zone are type, not payload.
- Subtree past `DataType::PARSE_RECURSION_LIMIT` -> one reserved `0xff` replaces it; no allocation, no panic; values differing only below that depth collide.
- Null cell in `column_digests` -> feeds the null tag, so it never collides with an empty string.
- A `variant` column -> refused by name; its binary encoding lands with the Iceberg v3 layer, so there is no value to feed.
- A `field` whose storage does not match the array given to `column_digests` -> reconciled to the field first, strictly, so a layout difference answers the same digest and a value the declaration cannot hold is named.
- The same value on a big-endian machine -> the same digest; every integer in the feed is little-endian.
- An `ascii`, `sized_ascii(n)`, `fixed_ascii(n)`, `sized_utf8(n)` or `cp1252` cell holding the same characters -> one digest; every string is one value and feeds the `utf8` tag.
- A `currency` and a `country` cell holding the same text -> two digests; a code feeds its own id, and a code never digests like the string that spells it.
- A `geometry` and a `geography` cell over the same WKB -> one digest; both feed the `geometry` tag.
- Holder-local `DIGEST:sources` or `DIGEST:algorithm` -> ignored by `row_digests`; they configure [`apply_arrow_batch`](#digest-holders-and-row-digests) only.
- A path through a list, map, or union -> that value is selected whole, never traversed.
- A holder, or `DIGEST:sources`/`DIGEST:algorithm`, under a list, map, union, dictionary, or run-end layout -> refused by path; a fill descends into Struct children only.
- A holder that selects itself or another holder in the same Struct -> refused.
- A Struct with several direct holders -> ambiguous; name the intended nested holder by a path.
- A holder column missing from the batch -> added in the position the root declares.
- Children below a null Struct -> not read and not changed.
- A non-default holder under `force=false` -> preserved, though nothing proves which algorithm, seed, or secret produced it.
- A nullable holder -> null is unfilled and a present zero is kept; a required integer holder reads zero as unfilled.
- An algorithm differing from the receiver's, declared or resolved from a holder's width -> a fresh unseeded state, so a seed and a secret reach only the holders sharing that receiver's width.
- `DigestField::apply_arrow_batch` -> the seedless state's answer, never forcing; a seed, a secret, or a recompute is a state's `apply_arrow_batch`.
- A signed holder -> bit-cast to the same-width unsigned payload, so `int32`/`uint32` and `int64`/`uint64` schemas answer one digest.
- A set high bit in a signed holder -> reads as a negative integer, with no overflow and no loss.
- An instant before the epoch -> orders first as a value and as a UUID, but last as bytes: a big-endian two's-complement count sorts by time within one sign range only.
- One instant in two units -> two values that never compare equal and are not normalized; value order compares the unit before the count.
- A count that does not fit the finer unit -> `ArithmeticOverflow`, never a wrapped count; `into_uuid` past signed 64-bit nanoseconds answers the same `unix restatement overflows int64`.
- `into_uuid` on an XXH32 or XXH3-128 digest -> `Error::InvalidRecord` at `$.digest`, `expected a 64-bit digest for UUIDv7, got xxh3-128 (128 bits)`; `ValueError` in Python, a thrown error in JavaScript. An instant before the epoch or past the 48-bit millisecond count -> `Error::InvalidRecord` at `$`, as `Uuid::from_v7` refuses it.
- Two digests differing only in their six high bits, or XXH64 and XXH3-64 over one payload -> one UUID; the projection is a 58-bit fingerprint.
- `d`, `year_month`, `day_time`, `month_day_nano` as a unit -> refused; a unix count is a clock resolution.
- A time of day, a duration, an interval, a null, a boolean, or a float as an instant -> refused by kind.
- Timestamp text -> read at the resolution its own digits spell, then restated like every other intake; `1970-01-01T00:00:00.0000019` at microseconds is `1`.
- Date-only text -> that day's midnight, as a date scalar is.
- Timestamp text past the year 2262 with seven or more fractional digits -> refused as out of range; those digits name nanoseconds, and a signed 64-bit count of them ends there.
- A spelling naming another unit or algorithm than `from_scalar` asks for -> refused rather than restated.
- The same sixteen bytes read under XXH64 and XXH3-64 -> two values that render and compare apart, because the algorithm is part of the value.
- `TxHasher::from_digester` -> bytes already fed to the state are discarded; a hasher is a configuration, not a running digest.
- `TxHasher::with_seed` -> drops a secret; a seed and a secret are built as one state and given to `from_digester`.
- Python `True` or JavaScript `true` as an instant -> `TypeError`, never `1`.
- JavaScript `number` instants -> safe integers only; a fraction or a wider number is refused.
- An instant column shorter or longer than the batch -> refused by length.
- A text, boolean, float, list, or dictionary column as the instant -> refused by datatype; it is read as a column, never per cell.
- A `uint64` instant above `i64::MAX` -> refused naming the value.
- An empty batch -> an empty coupled column of the right width.
- A null instant -> a null coupled cell in every column function; the digest functions answer no nulls, so a coupled column carries exactly the instant column's nulls.
- `compose` with a digest column of another width -> refused naming the expected width; `int32` and `int64` are read as the same bits `uint32` and `uint64` hold.
- `decompose` on a `fixed_size_binary` of another width -> refused naming the coupled width.
- A `DIGEST:time` path through a list, map, union, dictionary, or run-end layout -> refused by path, as a holder there is.
- `DIGEST:time` or `DIGEST:unit` off a holder -> refused as belonging only to a holder.
- Nested Structs -> the instant is read after nested holders are final, and a row null at any Struct above the leaf is null.
- A seeded `TxHasher` -> its seed and secret reach every holder sharing its algorithm's width, as `Digester::apply_arrow_batch` states; the holder's own `DIGEST:unit` always wins over the hasher's.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- xxhash:: txhash:: hashing::
    cargo test --features "parquet iceberg" -p yggdryl --test allocations -- the_canonical_value_feed_allocates_nothing borrowed_value_bytes_allocate_nothing coupled_value_bytes_allocate_nothing reading_an_instant_out_of_a_value_allocates_nothing txhash_uuid_projection_allocates_nothing_at_any_corpus_size
    cargo test --features "parquet iceberg" -p yggdryl --test types -- stable_hash
    cargo bench -p yggdryl --bench hashing
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/hashing
    python/.venv/bin/python python/benchmarks/digest.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/txhash.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/hashing/xxhash/xxhash.test.js node/tests/hashing/txhash/txhash.test.js
    npm run --prefix node bench:hashing:xxhash
    npm run --prefix node bench:hashing:txhash
    ```

## Performance

The numbers come from containerized x86_64 Linux runs on one host (Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB): rustc 1.94.1 release with thin LTO, CPython 3.11.15, Node 22.22.2, fixtures built outside every measured loop ([benchmarks](benchmarks.md)). They were measured earlier, when the drivers were `rust/benchmarks/xxhash.rs`, `rust/benchmarks/txhash.rs`, `node/benchmarks/xxhash.js`, and `node/benchmarks/txhash.js`; the move to `rust/benchmarks/hashing/` and `node/benchmarks/hashing/` kept every case name, fixture byte, and cost assertion, and nothing was rerun, so each regenerate command below runs the same cases under the new driver. The `into_uuid` case in `txhash_value` postdates these runs and has no row.

The Arrow groups report rows per second for missing or default holders, preserved populated holders, and forced recomputation. The JavaScript rows include the IPC copy that binding requires.

### Throughput per algorithm and size

Bytes per second, higher is better; the 64 MiB row is memory-bound rather than hash-bound. XXH3 is roughly four times XXH32 and twice XXH64 once a payload is worth vectorizing.

| payload | xxh32 | xxh64 | xxh3-64 | xxh3-128 |
| --- | --- | --- | --- | --- |
| 1 B | 0.25 GB/s | 0.16 GB/s | 0.21 GB/s | 0.18 GB/s |
| 4 B | 0.70 GB/s | 0.73 GB/s | 0.88 GB/s | 0.78 GB/s |
| 16 B | 2.70 GB/s | 2.81 GB/s | 3.96 GB/s | 2.58 GB/s |
| 64 B | 5.12 GB/s | 4.87 GB/s | 9.39 GB/s | 5.28 GB/s |
| 128 B | 5.86 GB/s | 6.52 GB/s | 11.61 GB/s | 7.41 GB/s |
| 240 B | 6.15 GB/s | 7.86 GB/s | 13.57 GB/s | 8.74 GB/s |
| 1 KiB | 6.48 GB/s | 11.57 GB/s | 20.52 GB/s | 16.44 GB/s |
| 64 KiB | 6.55 GB/s | 12.99 GB/s | 28.13 GB/s | 27.55 GB/s |
| 1 MiB | 6.43 GB/s | 12.74 GB/s | 25.84 GB/s | 25.88 GB/s |
| 64 MiB | 4.58 GB/s | 5.97 GB/s | 6.54 GB/s | 6.64 GB/s |

```bash
cargo bench -p yggdryl --bench hashing -- xxhash_size
```

### What this module costs over the protocol it wraps

All three columns hash the same bytes with the same implementation; the difference is argument normalization at this module's boundary.

| payload | `xxhash::xxh3` | direct `twox-hash` call | `DigestAlgorithm::digest` |
| --- | --- | --- | --- |
| 1 B | 4.9 ns | 4.1 ns | 4.9 ns |
| 64 B | 7.4 ns | 8.1 ns | 7.3 ns |
| 240 B | 19.0 ns | 22.5 ns | 19.3 ns |
| 4 KiB | 141.9 ns | 124.1 ns | 145.6 ns |
| 1 MiB | 37.8 µs | 37.0 µs | 39.1 µs |

From 4 KiB up the columns sit inside each other's run-to-run spread. Carrying the algorithm in a `Digest` costs nothing measurable.

```bash
cargo bench -p yggdryl --bench hashing -- xxhash_wrapper
```

### Handle digests and write-through

| case | time | throughput |
| --- | --- | --- |
| one-shot, 1 MiB | 0.040 ms | 26.00 GB/s |
| streamed in 64 KiB windows, 1 MiB | 0.040 ms | 26.42 GB/s |
| one-shot, 64 MiB | 9.153 ms | 7.33 GB/s |
| streamed in 64 KiB windows, 64 MiB | 8.938 ms | 7.51 GB/s |

Streaming in `pstream_bytes` windows costs nothing against hashing the payload whole.

| 64 MiB local file | time | peak resident after |
| --- | --- | --- |
| `read_digest` | 8.85 ms | 77.9 MiB (unchanged) |
| `read_all_bytes` then digest | 72.98 ms | 140.5 MiB |

The memory column is why `read_digest` exists: the streamed read left the high-water mark unchanged.

| case | time |
| --- | --- |
| `Hashed<H>` write-through, 4 MiB | 0.762 ms |
| plain write then a digest pass, 4 MiB | 0.948 ms |
| `std::io::copy`, 4 MiB | 0.322 ms |
| through `DigestReader` | 0.685 ms |
| through `DigestWriter` | 0.540 ms |

`Hashed<H>` saves the second pass, the 0.19 ms. The wrappers hash on a copy already happening.

```bash
cargo bench -p yggdryl --bench hashing -- xxhash_streaming
cargo bench -p yggdryl --bench hashing -- xxhash_handle
cargo bench -p yggdryl --bench hashing -- xxhash_hashed
cargo bench -p yggdryl --bench hashing -- xxhash_stream_wrappers
```

### The value feed

`rust/benchmarks/hashing/xxhash/values.rs` measures it. The feed row reuses one state, as an Arrow column does; the digest row builds a fresh state per value. The feed allocates nothing, which `rust/tests/allocations.rs` pins with a counting allocator.

| value | feed into a reused state | `digest` | `stable_hash` |
| --- | ---: | ---: | ---: |
| leaf string | 58.4 ns | 64.7 ns | 54.7 ns |
| integer | 50.8 ns | 57.5 ns | 47.5 ns |
| decimal | 74.9 ns | 81.7 ns | 75.7 ns |
| four-column row | 157.0 ns | 168.7 ns | 158.3 ns |
| 64-field record | 2.32 µs | 2.29 µs | 2.28 µs |
| 32-deep nest | 1.43 µs | 1.43 µs | 1.39 µs |

`stable_hash` on the short canonical renderings it sees measures the rendering; the second column is the algorithm alone.

| value | `stable_hash` | the same bytes through `xxh3` |
| --- | ---: | ---: |
| field name (19 chars) | 356.1 ns | 6.8 ns |
| URI (52 chars) | 98.1 ns | 7.1 ns |
| datatype expression (78 chars) | 725.5 ns | 13.9 ns |

```bash
cargo bench -p yggdryl --bench hashing -- xxhash_value
cargo bench -p yggdryl --bench hashing -- xxhash_stable_hash
```

### Arrow row digests

`rust/benchmarks/hashing/xxhash/arrow.rs` measures it. Each case digests 65,536 rows of four columns. Both paths answer the same digests; the fallback's dictionary-encoded text has no buffer arm and reads through the scalar boundary.

| case | time | per row |
| --- | ---: | ---: |
| buffer path (`int64`, `utf8`, `float64`, `utf8`) | 11.47 ms | 175 ns |
| scalar fallback (same shape, text dictionary-encoded) | 20.84 ms | 318 ns |
| materializing each row as a `Scalar` first | 29.32 ms | 447 ns |
| buffer path, XXH3-128 | 11.24 ms | 172 ns |

Answering 128 bits instead of 64 costs nothing.

```bash
cargo bench -p yggdryl --bench hashing -- xxhash_row_digests
```

### The coupling beside the digest

All three columns hash the same bytes with the same implementation; the difference is laying the instant beside the answer, which is one copy of eight bytes.

| payload | `xxhash::xxh3` | `txhash::txh3` | `txhash::txh128` |
| --- | ---: | ---: | ---: |
| 16 B | 3.55 ns | 3.74 ns | 6.20 ns |
| 240 B | 18.1 ns | 19.1 ns | 28.1 ns |
| 4 KiB | 148.7 ns | 147.2 ns | 162.7 ns |
| 64 KiB | 2.38 µs | 2.33 µs | 2.43 µs |

From 4 KiB up the columns sit inside each other's run-to-run spread; below it the coupling is the eight-byte copy it is. A `TxHasher` clones its configured state per answer, and XXH3 keeps its secret on the heap, which is the algorithm's cost rather than the coupling's:

| case | `TxHasher` | the one-shot beside it |
| --- | ---: | ---: |
| `digest`, 240 B | 72.2 ns | 19.1 ns (`txh3`) |
| `digest_scalar`, four-column row | 157.9 ns | 151.5 ns (`Scalar::txhash`) |

```bash
cargo bench -p yggdryl --bench hashing -- txhash_coupling
cargo bench -p yggdryl --bench hashing -- txhash_hasher
```

### The value and the instant

The value's projections are inline work; the two that allocate are the spelling, by contract, and `into_scalar`, whose byte payload is shared storage.

| operation | time |
| --- | ---: |
| `into_bytes` | 2.26 ns |
| `from_bytes` | 18.4 ns |
| `with_unit` | 20.6 ns |
| `into_datetime` | 11.0 ns |
| `into_scalar` | 29.9 ns |
| `to_string` | 204 ns |
| `from_str` | 186 ns |

Reading an instant out of a value costs the unit arithmetic for an integer or a datetime and the timestamp parser for text.

| instant | `unix_from_scalar` |
| --- | ---: |
| integer | 10.1 ns |
| zoned datetime | 15.4 ns |
| timestamp text | 75.9 ns |
| `restate_unix`, nanoseconds to microseconds | 11.1 ns |
| `unix_now` | 29.3 ns |

```bash
cargo bench -p yggdryl --bench hashing -- txhash_value
cargo bench -p yggdryl --bench hashing -- txhash_instant
```

### Coupled column and holder costs

`rust/benchmarks/hashing/txhash/arrow.rs` measures them; each case runs 65,536 rows. The coupled column costs what the digest column costs plus one eight-byte copy per row; the instant column is free when it is already at the holder's unit and one pass of integer division otherwise.

| case | time | rows/s |
| --- | ---: | ---: |
| `xxhash::arrow::row_digests`, XXH3-64 | 14.02 ms | 4.68 M |
| `row_txhashes`, XXH3-64 | 13.65 ms | 4.80 M |
| `row_txhashes`, XXH128 | 14.57 ms | 4.50 M |
| `xxhash::arrow::column_digests`, one text column | 6.33 ms | 10.4 M |
| `column_txhashes`, one text column | 6.85 ms | 9.57 M |
| `unix_array`, microseconds read as microseconds | 72.7 ns | shares the buffer |
| `unix_array`, nanoseconds floored to microseconds | 574 µs | 114 M |
| `compose` | 724 µs | 90.5 M |
| `decompose` | 787 µs | 83.2 M |

The holder fill pays for the digest of every source; the coupling adds the instant read and the wider cell. Both rows fill one 65,536-row batch through `DigestField::apply_arrow_batch`, IPC excluded.

| holder | time | rows/s |
| --- | ---: | ---: |
| plain `uint64` holder | 28.6 ms | 2.29 M |
| coupled `fixed_size_binary[16]` holder | 32.5 ms | 2.01 M |

```bash
cargo bench -p yggdryl --bench hashing -- txhash_columns
cargo bench -p yggdryl --bench hashing -- txhash_apply_arrow_batch
```

### xxHash at the bindings

The Python rows ran a release wheel (`--min-time 0.1 --repeat 5`) against the C `libxxhash` binding on one 1,080,000-byte payload.

```text
xxh3 payload                     44773.2 ns    24.12 GB/s
xxh3 payload (C libxxhash)       66506.4 ns    16.24 GB/s
xxh64 payload                       87825.4 ns    12.30 GB/s
xxh64 payload (C libxxhash)         85947.4 ns    12.57 GB/s
xxh3 payload (bytearray)         75534.4 ns    14.30 GB/s
xxh3 payload (memoryview)        75835.4 ns    14.24 GB/s
xxh3 payload (str)               44825.5 ns    24.09 GB/s
xxh3        1 B                    165.8 ns     0.01 GB/s
xxh3        1 B (C libxxhash)       78.0 ns     0.01 GB/s
```

```bash
python/.venv/bin/python python/benchmarks/digest.py --min-time 0.2 --repeat 5
```

The Node rows ran a release addon on the same payload; `streamed 64 KiB` and `scalar leaf digest` belong to [Streaming](#streaming) and [Values](#values).

```text
xxh3         1 B                                 495.9 ns     0.00 GB/s
xxh3      1024 B                                 505.0 ns     2.03 GB/s
xxh3     65536 B                                2756.8 ns    23.77 GB/s
xxh32 payload                                    170537.4 ns     6.33 GB/s
xxh64 payload                                     87036.2 ns    12.41 GB/s
xxh3 payload                                   48139.1 ns    22.43 GB/s
xxh128 payload                                  51937.5 ns    20.79 GB/s
xxh3 payload (Uint8Array)                      50711.2 ns    21.30 GB/s
xxh3 payload (string)                         364511.2 ns     2.96 GB/s
xxh3 payload (streamed 64 KiB)                 71816.4 ns    15.04 GB/s
scalar leaf digest                                 2399.2 ns     0.00 GB/s
```

```bash
npm run --prefix node bench:hashing:xxhash
```

### TxHash at the bindings

Every coupled row pairs with the plain digest row for the same bytes, so the difference is reading the instant, laying it beside the digest, and the value object that crosses back. The Python rows ran a release wheel (`--min-time 0.2 --repeat 5`). A `datetime` instant costs the conversion every `Scalar` intake shares, minus the zone lookup a unix count does not need; the batch rows fill 4,096 rows through PyArrow.

```text
xxh3         16 B                                     171.5 ns     0.09 GB/s
txh3         16 B                                     236.3 ns     0.07 GB/s
xxh3        240 B                                     187.3 ns     1.28 GB/s
txh3        240 B                                     258.4 ns     0.93 GB/s
xxh3       4096 B                                     316.3 ns    12.95 GB/s
txh3       4096 B                                     393.6 ns    10.41 GB/s
xxh3      65536 B                                    2386.6 ns    27.46 GB/s
txh3      65536 B                                    2493.5 ns    26.28 GB/s
txh3 240 B (datetime instant)                        1871.9 ns     0.13 GB/s
hasher.digest 240 B                                   353.2 ns     0.68 GB/s
hasher.digest_scalar (four-column row)                373.0 ns     0.00 GB/s
Scalar.digest (four-column row)                       295.8 ns     0.00 GB/s
bytes(value)                                          155.5 ns     0.10 GB/s
TxHash.from_bytes                                     279.5 ns     0.06 GB/s
TxHash(str)                                           348.6 ns     0.05 GB/s
unix_of(datetime)                                    1675.5 ns     0.00 GB/s
xxhash.row_digests                                    757.6 us     5.41 M row/s
txhash.row_txhashes                                   790.6 us     5.18 M row/s
txhash.compose                                         47.5 us    86.17 M row/s
txhash.decompose                                       54.4 us    75.23 M row/s
txhash.unix_array                                       4.4 us   931.07 M row/s
plain holder apply_arrow_batch                       2183.0 us     1.88 M row/s
coupled holder apply_arrow_batch                     2403.8 us     1.70 M row/s
```

The Node rows ran a release addon. A digest crosses back as a `bigint` and a coupled value as a `TxHash` instance, which is the fixed cost every `txh3` row shows beside `xxh3`; a `Date` and a string cross as themselves and are read natively, so an instant that is not a `bigint` costs the parser, not a `Scalar` object. The batch rows fill 4,096 rows through Arrow IPC, both copies included.

```text
xxh3        16 B                                    522.3 ns     0.03 GB/s
txh3        16 B                                   2621.2 ns     0.01 GB/s
xxh3       240 B                                    516.9 ns     0.46 GB/s
txh3       240 B                                   2770.6 ns     0.09 GB/s
xxh3      4096 B                                    646.5 ns     6.34 GB/s
txh3      4096 B                                   3105.4 ns     1.32 GB/s
xxh3     65536 B                                   3188.1 ns    20.56 GB/s
txh3     65536 B                                   5332.3 ns    12.29 GB/s
txh3 240 B (Date instant)                          4967.5 ns     0.05 GB/s
hasher.digest 240 B                                3727.8 ns     0.06 GB/s
hasher.digestScalar (four-column row)              3105.0 ns     0.00 GB/s
Scalar.digest (four-column row)                    2867.0 ns     0.00 GB/s
value.bytes()                                      2519.6 ns     0.01 GB/s
TxHash.fromBytes                                   3305.5 ns     0.00 GB/s
TxHash.from(string)                                3172.8 ns     0.01 GB/s
txhash.unixOf(Date)                                1781.3 ns     0.00 GB/s
plain holder applyArrowBatch                       2625.7 us     1.56 M row/s
coupled holder applyArrowBatch                     2821.7 us     1.45 M row/s
```

```bash
python/.venv/bin/python python/benchmarks/txhash.py --min-time 0.2 --repeat 5
npm run --prefix node bench:hashing:txhash
```
