# xxhash

Digest bytes, values, handles, and Arrow rows with XXH32, XXH64, XXH3-64, and XXH3-128.

!!! warning "Not a cryptographic hash, and not Iceberg's bucket transform"
    A digest detects accidental change - a truncated upload, a stale cache entry, a
    duplicated row - and nothing more. Never use one as an integrity check against an
    adversary who chooses the input, and never as a password or signature primitive.

    Iceberg's `bucket[N]` transform is pinned by its specification to murmur3 x86_32. A
    partition computed with xxHash would place rows in the wrong buckets and no other
    reader would find them; [iceberg](media.md) never calls this module for partitioning.

## One-shot digests

=== "Rust"

    ```rust
    use yggdryl::xxhash;

    assert_eq!(xxhash::xxh32(b"abc"), 0x32d1_53ff);
    assert_eq!(xxhash::xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
    assert_eq!(xxhash::xxh3(b"abc"), 0x78af_5f94_892f_3950);
    assert_eq!(
        xxhash::xxh128(b"abc"),
        0x06b0_5ab6_733a_6185_78af_5f94_892f_3950,
    );

    // The input comes first and the seed second, here and everywhere.
    assert_ne!(xxhash::xxh64_with_seed(b"abc", 42), xxhash::xxh64(b"abc"));
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    assert xxhash.xxh32(b"abc") == 0x32D153FF
    assert xxhash.xxh64(b"abc") == 0x44BC2CF5AD770999
    assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
    assert xxhash.xxh128(b"abc") == 0x06B05AB6733A618578AF5F94892F3950

    # bytes, bytearray, memoryview, any buffer, or a str as its UTF-8.
    assert xxhash.xxh3("abc") == xxhash.xxh3(bytearray(b"abc"))
    assert xxhash.xxh64(b"abc", seed=42) != xxhash.xxh64(b"abc")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

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
    ```

The four functions answer their native widths with nothing wrapped around the number.
`Digest` is for a caller who wants the algorithm travelling with the value instead, and
`DigestAlgorithm` is the runtime dispatcher when the algorithm is a value rather than a
call - the same relationship [`Codec`](types.md) has to [gzip](coding.md).

## Digest values

=== "Rust"

    ```rust
    use yggdryl::{Digest, DigestAlgorithm};

    let digest = DigestAlgorithm::Xxh3.digest(b"abc");
    assert_eq!(digest.as_u64(), Some(0x78af_5f94_892f_3950));
    assert_eq!(digest.to_string(), "xxh3-64:78af5f94892f3950");
    assert_eq!(Digest::from_str(&digest.to_string())?, digest);
    assert_eq!(digest.into_bytes().len(), 8);

    // Two algorithms are never equal, whatever their payloads: `xxh64` and
    // `xxh3-64` are both 64 bits wide and answer different values.
    assert_ne!(
        Digest::new(DigestAlgorithm::Xxh64, 7),
        Digest::new(DigestAlgorithm::Xxh3, 7),
    );
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    digest = xxhash.digest(b"abc", "xxh3-64")
    assert int(digest) == 0x78AF5F94892F3950
    assert str(digest) == "xxh3-64:78af5f94892f3950"
    assert xxhash.Digest(str(digest)) == digest
    assert len(bytes(digest)) == digest.width == 8

    assert xxhash.Digest.from_int("xxh64", 7) != xxhash.Digest.from_int("xxh3-64", 7)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const digest = xxhash.digest(Buffer.from('abc'), 'xxh3-64')
    assert.equal(digest.value(), 0x78af5f94892f3950n)
    assert.equal(digest.toString(), 'xxh3-64:78af5f94892f3950')
    assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
    assert.equal(digest.bytes().length, digest.width)

    const wide = xxhash.digest(Buffer.from('abc'), 'xxh3-128')
    assert.ok(!wide.equals(digest))
    ```

The rendering carries the algorithm because the value does. Without it the two 64-bit
algorithms would share one spelling, and parsing could not be the exact inverse of
rendering. `into_bytes` is the canonical big-endian representation the reference calls
`XXH*_canonicalFromHash`, so two machines of different endianness store one digest as one
sequence of bytes.

## Streaming states

=== "Rust"

    ```rust
    use yggdryl::xxhash::{Xxh3, xxh3};

    let payload = b"symbol,price\nAAPL,187.23\n";
    for split in [1, 7, payload.len()] {
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
    assert_eq!(state.as_u64(), state.as_u64());
    state.write_bytes(b",187.23");
    assert_eq!(state.as_u64(), xxh3(b"AAPL,187.23"));
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    payload = b"symbol,price\nAAPL,187.23\n"
    for split in (1, 7, len(payload)):
        state = xxhash.Xxh3()
        for index in range(0, len(payload), split):
            state.write_bytes(payload[index : index + split])
        assert int(state.as_digest()) == xxhash.xxh3(payload)

    state = xxhash.Xxh3()
    state.write_bytes(b"AAPL")
    assert state.as_digest() == state.as_digest()
    state.write_bytes(b",187.23")
    assert int(state.as_digest()) == xxhash.xxh3(b"AAPL,187.23")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const payload = Buffer.from('symbol,price\nAAPL,187.23\n')
    for (const split of [1, 7, payload.length]) {
      const state = new xxhash.Xxh3()
      for (let index = 0; index < payload.length; index += split) {
        state.writeBytes(payload.subarray(index, index + split))
      }
      assert.equal(state.asDigest().value(), xxhash.xxh3(payload))
    }

    const state = new xxhash.Xxh3()
    state.writeBytes(Buffer.from('AAPL'))
    assert.ok(state.asDigest().equals(state.asDigest()))
    state.writeBytes(Buffer.from(',187.23'))
    assert.equal(state.asDigest().value(), xxhash.xxh3(Buffer.from('AAPL,187.23')))
    ```

Chunk invariance is the property everything else here rests on. A message spliced from two
spans - a record with its row header removed from the middle of a line - hashes the same as
the equivalent joined string, without ever building the join, and an empty chunk contributes
nothing wherever it sits. A hash that depended on where the boundary fell would be a silent
correctness bug.

`clear()` returns a state to the seed and secret it was **constructed with**, not to an
unseeded one. In Rust each state is also a `std::hash::Hasher` and its own `BuildHasher`, so
it drops into a `HashMap` carrying its seed; `Hasher::finish` on `Xxh128` answers the low
64 bits because the trait's return type cannot carry more, and `as_u128` is the full value.

## Seeds and secrets

XXH32 and XXH64 take a seed and never a secret. Only the XXH3 pair is secretable, and
`DigestAlgorithm::is_secretable` is what a caller asks first.

XXH3 consults a custom secret only for inputs **longer than 240 bytes**. At or below that
length the algorithm uses its derived secret and the seed, which is the protocol's own rule
for the seed-and-secret family and what keeps a one-shot and a streaming state answering one
value for the same bytes. A short secret is rejected by length whatever the payload, so a
secret is never silently the wrong one - but a caller hashing short values with a secret is
hashing them with the default secret, by design. The examples below use a payload past the
cutoff for that reason.

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

    from yggdryl import xxhash

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
    const { xxhash } = require('yggdryl')

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

## Handle digests

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{DigestAlgorithm, xxhash};

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;

    assert_eq!(
        handle.read_digest(DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
    );
    assert_eq!(
        handle.read_range_digest(0, 6, DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"symbol"),
    );

    // A resource that does not exist digests as no bytes, per the laziness
    // contract - absence is emptiness, not a third answer to branch on.
    assert_eq!(
        Buffer::new().read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
        Some(xxhash::xxh3(b"")),
    );
    ```

=== "Python"

    ```python
    import tempfile
    from pathlib import Path

    from yggdryl import IOBase, xxhash

    with tempfile.TemporaryDirectory() as root:
        path = Path(root) / "trades.csv"
        path.write_bytes(b"symbol,price\nAAPL,187.23\n")

        handle = IOBase(path)
        assert handle.read_digest("xxh3-64") == xxhash.digest(handle.read_bytes(), "xxh3-64")
        assert handle.read_range_digest(0, 6) == xxhash.digest(b"symbol", "xxh3-64")

        missing = IOBase(Path(root) / "never-written.csv")
        assert missing.read_digest() == xxhash.digest(b"", "xxh3-64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, xxhash } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-digest-'))
    try {
      const file = path.join(root, 'trades.csv')
      const payload = Buffer.from('symbol,price\nAAPL,187.23\n')
      fs.writeFileSync(file, payload)

      const handle = new IOBase(file)
      assert.ok(handle.readDigest('xxh3-64').equals(xxhash.digest(payload, 'xxh3-64')))
      assert.ok(handle.readRangeDigest(0, 6).equals(xxhash.digest(Buffer.from('symbol'), 'xxh3-64')))

      const missing = new IOBase(path.join(root, 'never-written.csv'))
      assert.ok(missing.readDigest().equals(xxhash.digest(Buffer.alloc(0), 'xxh3-64')))
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

`read_digest` streams through [`pstream_bytes`](holder.md) and retains one bounded chunk, so
memory is flat in the object's size: a 64 GiB file costs one window rather than a copy.
Nothing calls `read_all_bytes`. Both methods are derived, so every backend and every wrapper
inherits them unchanged - which is what makes the two questions a compressed handle can be
asked stay distinct:

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{DigestAlgorithm, xxhash};

let plain = b"symbol,price\nAAPL,187.23\n";
let mut handle = Gzip::new(Buffer::new());
handle.write_all_bytes(plain)?;
handle.flush()?;

// The wrapper answers for the bytes it presents.
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
    Some(xxhash::xxh3(plain)),
);
// The handle underneath answers for the bytes it holds.
let compressed = handle.handle().read_all_bytes()?;
assert_eq!(
    handle.handle().read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
    Some(xxhash::xxh3(&compressed)),
);
assert_ne!(xxhash::xxh3(plain), xxhash::xxh3(&compressed));
```

A container holds no bytes of its own, so it is a typed `Error::NotAtomic` naming the kind
rather than an answer. Which files a folder digest would cover, in what order, is a
convention no format states; folder and recursive digests are deliberately absent.

## Reading and writing through a digest

!!! note "Rust only"
    `DigestReader` and `DigestWriter` are built on `Read` and `Write`, which neither binding
    has a native spelling for - the same reason [gzip](coding.md)'s `reader`/`writer` pair is
    Rust only.

```rust
use std::io::{Read, Write};

use yggdryl::{DigestAlgorithm, xxhash};

// Hash a payload that is already being moved, in the pass that was already
// happening, rather than reading it a second time.
let mut source = xxhash::reader(b"AAPL,187.23".as_slice(), DigestAlgorithm::Xxh3);
let mut moved = Vec::new();
source.read_to_end(&mut moved)?;
assert_eq!(moved, b"AAPL,187.23");
assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3.digest(&moved));

let mut target = xxhash::writer(Vec::new(), DigestAlgorithm::Xxh64);
target.write_all(b"AAPL,187.23")?;
assert_eq!(target.as_digest(), DigestAlgorithm::Xxh64.digest(b"AAPL,187.23"));
assert_eq!(target.into_inner(), b"AAPL,187.23");
```

## Hashed handles

!!! note "Rust only"
    `Hashed<H>` wraps an `IOBase`, which the bindings reach as `IOBase` rather than as a
    generic type parameter.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::xxhash::Hashed;
use yggdryl::DigestAlgorithm;

let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
handle.write_all_bytes(b"symbol,price\n")?;
handle.append_bytes(b"AAPL,187.23\n")?;
handle.flush()?;

// Answered from the running state; the bytes are never read back.
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(b"symbol,price\nAAPL,187.23\n"),
);

// A positional write the running state cannot follow makes it stale, which is
// not an error and not silent corruption: the next digest re-streams the
// handle and re-arms, and the answer is identical either way.
handle.pwrite_all(7, b"PRICE")?;
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
);
```

The running state covers writes that are strictly sequential from offset 0 - a whole write,
repeated appends, a streamed record write - which is the case a lake actually writes. It is
consulted only when it covers the whole value, which is what makes pending writes count only
after `flush`: a handle that stages writes until publication is streamed until its size
catches up.

## The canonical value feed

Every `Scalar` has one canonical byte representation, and it is what a digest, a row key,
and `stable_hash` all read. Two spellings answer two different questions.

`as_value_bytes` is the **payload alone** - no tag, no length - so hashing a string equals
hashing its UTF-8 and agrees with any other xxHash implementation on the same bytes. It
borrows wherever the value already holds bytes and never allocates.

`write_bytes` is the **total, prefix-free feed** over every variant.

!!! note "The borrowed view is Rust only"
    Both bindings reach the feed - `Scalar.digest` and a state's `write_scalar` read exactly
    these bytes. `as_value_bytes` is the borrowed view of a payload the value already holds,
    which has no spelling that survives the boundary without the copy it exists to avoid.

=== "Rust"

    ```rust
    use yggdryl::{DigestAlgorithm, Scalar, xxhash};

    let symbol = Scalar::from("AAPL");
    assert_eq!(&*symbol.as_value_bytes().unwrap(), b"AAPL");
    assert_eq!(
        xxhash::xxh3(&symbol.as_value_bytes().unwrap()),
        xxhash::xxh3(b"AAPL"),
    );

    // Equal values answer one digest, across widths.
    assert_eq!(Scalar::from(1_i8), Scalar::from(1_i64));
    assert_eq!(
        Scalar::from(1_i8).digest(DigestAlgorithm::Xxh3),
        Scalar::from(1_i64).digest(DigestAlgorithm::Xxh3),
    );
    // Values that differ stay apart, across variant boundaries.
    assert_ne!(
        Scalar::from("1").digest(DigestAlgorithm::Xxh3),
        Scalar::from(0x31_u8).digest(DigestAlgorithm::Xxh3),
    );
    // A null and an empty string are not the same absence.
    assert_ne!(
        Scalar::Null.digest(DigestAlgorithm::Xxh3),
        Scalar::from("").digest(DigestAlgorithm::Xxh3),
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, xxhash

    assert Scalar.from_py("AAPL").digest() == Scalar.from_py("AAPL").digest("xxh3-64")
    assert int(Scalar.from_py("AAPL").digest()) == Scalar.from_py("AAPL").stable_hash()

    # Equal values answer one digest, across widths.
    assert Scalar.decimal(100, 2).digest() == Scalar.decimal(1, 0).digest()
    assert Scalar.float(1.5, 32).digest() == Scalar.float(1.5, 64).digest()
    # Values that differ stay apart, across variant boundaries.
    assert Scalar.from_py("1").digest() != Scalar.from_py(b"1").digest()
    assert Scalar.from_py(None).digest() != Scalar.from_py("").digest()

    state = xxhash.Xxh3()
    state.write_scalar(Scalar.from_py("AAPL"))
    assert state.as_digest() == Scalar.from_py("AAPL").digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, xxhash } = require('yggdryl')

    const symbol = Scalar.fromJs('AAPL')
    assert.ok(symbol.digest().equals(symbol.digest('xxh3-64')))
    assert.equal(symbol.digest().value(), symbol.stableHash())

    // Equal values answer one digest, across widths.
    assert.ok(Scalar.decimal(100n, 2).digest().equals(Scalar.decimal(1n, 0).digest()))
    // Values that differ stay apart, across variant boundaries.
    assert.ok(!Scalar.fromJs('1').digest().equals(Scalar.fromJs(Buffer.from('1')).digest()))

    const state = new xxhash.Xxh3()
    state.writeScalar(symbol)
    assert.ok(state.asDigest().equals(symbol.digest()))
    ```

### Encoding

Every value begins with one tag byte, a [`DataTypeId`](types.md) discriminant. That byte
is a wire contract: inserting a variant into `DataTypeId` anywhere but the end changes
stored digests, and a test pinning every value is what turns that into a failure rather
than a surprise. Every integer in the feed is little-endian, explicitly, so a digest does
not depend on the machine that computed it.

`Scalar` compares across widths - `I8(1)`, `I64(1)` and `U8(1)` are one value, as are
`F32(1.5)` and `F64(1.5)`, and `D128(100, 2)` and `D256(1, 0)` - so the feed writes each
family's canonical form rather than its storage width. A digest identifies the value, not
the box it came in.

| Variant | Tag | Feed after the tag |
| --- | --- | --- |
| `Null` | `null` | nothing |
| `Bool` | `boolean` | `0x00` or `0x01` |
| `I8`..`U128` | `uint128`, or `int128` when negative | magnitude as `u128` little-endian |
| `F16`/`F32`/`F64` | `float64` | the common `f64` reading's IEEE bits, little-endian |
| `D128`/`D256` | `decimal256` | normalized coefficient as `i256` little-endian, then scale as one signed byte |
| `String` | `utf8` | length `u64` little-endian, then UTF-8 |
| `Enum` | `dictionary` | length-prefixed enum identity, then the member ordinal |
| `Bytes` | `binary` | length `u64` little-endian, then the bytes |
| `Geospatial` | `geometry` | length `u64` little-endian, then the WKB |
| `Date32`/`Date64` | `date64` | unit class byte, normalized count as `i128` little-endian, length-prefixed timezone |
| `Time32`/`Time64` | `time64` | as above |
| `DateTime64` | `timestamp` | as above |
| `Duration32`/`Duration64` | `duration64` | as above |
| `Sequence` | `list` | element count `u64` little-endian, then each element's feed |
| `Mapping` | `map` | entry count `u64` little-endian, then each key feed and value feed in stored order |
| `Record` | `struct` | entry count `u64` little-endian, then per sorted entry a length-prefixed name and the value's feed |

`as_value_bytes` answers `None` for `Null`, `Sequence`, `Mapping`, and `Record`, which have
no payload without a framing. It keeps the storage width the feed collapses, and a decimal
answers its coefficient while a temporal answers its stored count - the scale, unit, and
zone beside them are the value's type rather than its payload.

Nesting is bounded by the shared structured-value limit, `DataType::PARSE_RECURSION_LIMIT`.
A subtree past it feeds one reserved `0xff` in place of the subtree, so the walk stays total
and allocation-free for any caller input and never reaches a panic; values differing only
below that depth are indistinguishable, exactly as `Scalar::dtype` refuses to name them.

## One hash contract

`stable_hash` is XXH3-64 over this feed, everywhere. There is no second hash family in the
tree and no second spelling of this one:

```rust
use yggdryl::{DigestAlgorithm, Scalar, text};

let value = Scalar::from("AAPL");
assert_eq!(
    value.stable_hash(),
    value.digest(DigestAlgorithm::Xxh3).as_u64().unwrap(),
);
// Equal values hash equally across widths, which is the invariant every
// binding relies on.
assert_eq!(
    Scalar::from(1_i8).stable_hash(),
    Scalar::from(1_i64).stable_hash()
);
let _ = text::Format::Json;
```

Values that carry text rather than a `Scalar` - `Field`, `Uri`, `DataType`, `MimeType`, the
Iceberg values - hash their canonical rendering through the same algorithm, so
`Field::stable_hash` and `xxhash::xxh3` of that rendering are one number reached two
ways.

## Filling digest holders

A digest holder is a field carrying `digest:role=holder`. Every resumable state fills all
holders in one Arrow `RecordBatch` under an authoritative non-null Struct root:

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::xxhash::Xxh3;
use yggdryl::{DataType, DigestAlgorithm, Field, Scalar};

let symbol = Field::new("symbol", DataType::Utf8, false);
let mut holder = Field::new("row_digest", DataType::UInt64, false);
holder.as_digest_mut().set_holder()?;
holder.as_digest_mut().set_paths(["symbol"])?;
holder
    .as_digest_mut()
    .set_algorithm(DigestAlgorithm::Xxh3)?;
let root = DataType::from_fields([symbol, holder])?.required_field("row");

// The target root adds the missing holder in its declared position.
let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![ArrowField::new(
        "symbol",
        ArrowDataType::Utf8,
        false,
    )])),
    vec![Arc::new(StringArray::from(vec!["AAPL"]))],
)?;

let mut state = Xxh3::with_seed(7);
state.write_bytes(b"an unrelated running stream");
let running = state.as_u64();
let filled = state.fill_arrow_batch(&root, batch, false)?;

let mut expected = Xxh3::with_seed(7);
expected.write_scalar(&Scalar::from_sequence([Scalar::from("AAPL")]));
assert_eq!(
    filled.column(1).as_primitive::<UInt64Type>().value(0),
    expected.as_u64(),
);
assert_eq!(state.as_u64(), running, "filling does not consume the state");
```

`digest:paths` is a canonical JSON array of unique, non-empty paths stored on a holder, for
example `["id","line.price"]`. Its order is the
hash-feed order. Paths are relative to the containing Struct: an exact whole field name wins,
then dots descend through Struct fields only. A list, map, union, or other value may be selected
whole but cannot be traversed. An absent key uses the containing Struct's component fallback;
`[]` deliberately hashes an empty sequence.

Nested Struct holders are filled deepest first. Selecting a nested Struct with one direct holder
feeds that holder's digest payload instead of hashing the Struct again. Signed integer holder
storage is first bit-cast back to the same-width unsigned payload, so `int32`/`uint32` and
`int64`/`uint64` schemas produce the same containing digest. With no holder the Struct feeds
normally; multiple direct holders are ambiguous and must be replaced by a path to the intended
nested holder. A holder cannot select itself or another holder in the same Struct.

Each visible row is framed as an ordered `Scalar::Sequence` and streamed through the state's
canonical value feed. With `force=false`, a cell equal to its holder Field's default is computed
and every non-default value is trusted and preserved. With `force=true`, every visible holder is
recomputed. Thus a nullable holder treats null as unfilled while preserving a present zero; a
required integer holder treats zero as unfilled. Children hidden below a null Struct are not read
or changed. An existing non-default value carries no proof that its declared algorithm, seed, or
secret produced it; set `force` when that provenance is not trusted.

`digest:algorithm` is the canonical algorithm token a holder explicitly requests: `xxh32`,
`xxh64`, `xxh3-64`, or `xxh3-128`. Without it, a receiver whose output width fits the holder is
used with its seed and secret. Otherwise the holder type selects the best fresh default: `int32`
or `uint32` selects XXH32, `int64` or `uint64` selects XXH3-64, and
`fixed_size_binary(16)` selects XXH3-128. Signed holders store the identical output bits through
the explicit signed/unsigned field bit cast; a set high bit therefore appears as a negative integer,
without overflow or loss. An explicit algorithm must fit the same storage mapping. If it differs
from the receiver, its state is fresh and unseeded
because the receiver's configuration
belongs to a different algorithm. Python spells the operation
`state.fill_arrow_batch(root, batch, force=True)`; JavaScript spells it
`state.fillArrowBatch(root, batch, true)` and copies the batch through Arrow IPC.

## Arrow row digests

!!! note "Rust only"
    Row and column digest arrays answer an `ArrayRef`. Both bindings reach Arrow through
    their own runtime holders, so the column is built there rather than crossing.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{Int64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::Schema;
use yggdryl::{DataType, DigestAlgorithm, Field};
use yggdryl::xxhash::arrow::row_digests;

let mut symbol = Field::new("symbol", DataType::Utf8, false);
symbol.as_digest_mut().set_component()?;
let quantity = Field::new("quantity", DataType::Int64, false);
let mut stored = Field::new("row_digest", DataType::UInt64, false);
stored.as_digest_mut().set_holder()?;

let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![
        symbol.into_arrow()?,
        quantity.into_arrow()?,
        stored.into_arrow()?,
    ])),
    vec![
        Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
        Arc::new(Int64Array::from(vec![100, 250, 999])),
        Arc::new(UInt64Array::from(vec![11, 22, 33])),
    ],
)?;

let digests = row_digests(&batch, DigestAlgorithm::Xxh3)?;
let digests = digests.as_primitive::<UInt64Type>();
// `symbol` is the explicit component. Differences in the unmarked quantity
// and the prior holder do not feed the digest.
assert_eq!(digests.value(0), digests.value(2));
assert_ne!(digests.value(0), digests.value(1));
```

The selected values remain an ordered `Scalar::Sequence` in schema order, including its element-count
framing. One or more fields carrying `digest:role=component` are the exact selection. If there is no
explicit component, every field except `digest:role=holder` is selected; an unmarked schema therefore
keeps the full-row behavior. A schema containing only holders hashes the empty sequence for every
row. Field names, roles, and other metadata choose the values but do not enter the byte feed.

That sequence-feed equality is the contract and the test on every datatype family, nulls, nested
structs, lists, maps, dictionaries, unions, and geospatial values. The
[`DigestField` selection helpers](types.md#digest-components-and-holders) expose the same effective
component set without hashing a batch.

`row_digests` always uses its `algorithm` argument and the direct role selection above. It does not
resolve holder-local `digest:paths` or `digest:algorithm`; those configure
[`fill_arrow_batch`](#filling-digest-holders) only.

`column_digests` is the single-column form: each answer is the cell's own value, with no row
framing around it. Nulls feed the null tag, so a null and an empty string never collide.

The column is `UInt32` for XXH32, `UInt64` for the two 64-bit algorithms, and
`FixedSizeBinary(16)` of canonical big-endian bytes for XXH3-128, which no Arrow integer is
wide enough to hold.

## Benchmarks

`rust/benchmarks/xxhash.rs`, `python/benchmarks/xxhash.py`, and
`node/benchmarks/xxhash.js` measure the same protocol from the three sides. One containerized
x86_64 Linux run (Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin LTO;
CPython 3.11.15; Node 22.22.2), `cargo bench --bench xxhash`. Fixtures are built once, outside
every measured loop. The Arrow groups report rows per second for missing/default holders,
preserved populated holders, and forced recomputation; the JavaScript rows include its required IPC
copy.

### Throughput per algorithm and size

Bytes per second, higher is better. Below a few hundred bytes a call's fixed cost dominates,
which is what the first rows show rather than hide; the 64 MiB row is memory-bound rather than
hash-bound, and every algorithm converges there.

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

XXH3 is roughly four times XXH32 and twice XXH64 once a payload is worth vectorizing, which is
why it is the default and what `stable_hash` answers.

### What this module costs over the protocol it wraps

Both rows hash the same bytes with the same implementation, so the difference is the argument
normalization at this module's boundary and nothing else.

| payload | `xxhash::xxh3` | direct `twox-hash` call | `DigestAlgorithm::digest` |
| --- | --- | --- | --- |
| 1 B | 4.9 ns | 4.1 ns | 4.9 ns |
| 64 B | 7.4 ns | 8.1 ns | 7.3 ns |
| 240 B | 19.0 ns | 22.5 ns | 19.3 ns |
| 4 KiB | 141.9 ns | 124.1 ns | 145.6 ns |
| 1 MiB | 37.8 µs | 37.0 µs | 39.1 µs |

At 1 byte the wrapper costs about 0.8 ns, which is a call's own overhead rather than anything
this module does with the bytes; from 4 KiB up the rows sit inside each other's run-to-run
spread, in both directions. Carrying the algorithm in a `Digest` costs nothing measurable.

### Streaming, and digesting a stored object

| case | time | throughput |
| --- | --- | --- |
| one-shot, 1 MiB | 0.040 ms | 26.00 GB/s |
| streamed in 64 KiB windows, 1 MiB | 0.040 ms | 26.42 GB/s |
| one-shot, 64 MiB | 9.153 ms | 7.33 GB/s |
| streamed in 64 KiB windows, 64 MiB | 8.938 ms | 7.51 GB/s |

Streaming in the window `pstream_bytes` yields costs nothing against hashing the payload whole,
which is what makes the handle read below free of a trade-off.

| 64 MiB local file | time | peak resident after |
| --- | --- | --- |
| `read_digest` | 8.85 ms | 77.9 MiB (unchanged) |
| `read_all_bytes` then digest | 72.98 ms | 140.5 MiB |

That memory column is the reason `read_digest` exists. The streamed read left the process's
high-water mark exactly where it found it; reading the value whole added the file. The digest
read is also 8x faster here, because it never allocates or fills 64 MiB it is going to throw
away - and the gap widens with the file, while the memory gap becomes the whole difference
between working and not.

| case | time |
| --- | --- |
| `Hashed<H>` write-through, 4 MiB | 0.762 ms |
| plain write then a digest pass, 4 MiB | 0.948 ms |
| `std::io::copy`, 4 MiB | 0.322 ms |
| through `DigestReader` | 0.685 ms |
| through `DigestWriter` | 0.540 ms |

`Hashed<H>` saves the second pass, which is the 0.19 ms. The reader and writer pay for the hash
on top of a copy that was already happening, which is the point: the alternative is copying and
then reading the payload again.

### The value feed

Nanoseconds per value. The feed row reuses one state, which is what an Arrow column does; the
digest row builds a fresh state per value, which is what a single call pays - XXH3 keeps its
secret on the heap, so that construction is the gap. The feed itself allocates nothing, which
`rust/tests/allocations.rs` pins with the counting allocator rather than this measuring.

| value | feed into a reused state | `digest` | `stable_hash` |
| --- | --- | --- | --- |
| leaf string | 58.4 ns | 64.7 ns | 54.7 ns |
| integer | 50.8 ns | 57.5 ns | 47.5 ns |
| decimal | 74.9 ns | 81.7 ns | 75.7 ns |
| four-column row | 157.0 ns | 168.7 ns | 158.3 ns |
| 64-field record | 2.32 µs | 2.29 µs | 2.28 µs |
| 32-deep nest | 1.43 µs | 1.43 µs | 1.39 µs |

`stable_hash` on the short canonical renderings it actually sees:

| value | `stable_hash` | the same bytes through `xxh3` |
| --- | --- | --- |
| field name (19 chars) | 356.1 ns | 6.8 ns |
| URI (52 chars) | 98.1 ns | 7.1 ns |
| datatype expression (78 chars) | 725.5 ns | 13.9 ns |

At these lengths the hash is the small half by more than an order of magnitude: what the first
column measures is the canonical rendering, and the second is the algorithm on its own. The
pre-swap FNV-1a numbers are not reproducible from this tree, and deliberately so - the fold is
deleted, because one hash contract is worth more than a few nanoseconds on a short string that
the rendering dominates anyway.

### Arrow row digests

65,536 rows, four columns.

| case | time | per row |
| --- | --- | --- |
| buffer path (`int64`, `utf8`, `float64`, `utf8`) | 11.47 ms | 175 ns |
| scalar fallback (same shape, text dictionary-encoded) | 20.84 ms | 318 ns |
| materializing each row as a `Scalar` first | 29.32 ms | 447 ns |
| buffer path, XXH3-128 | 11.24 ms | 172 ns |

The two paths answer the same digests; the fallback row is the identical logical content with
its two text columns dictionary-encoded, which has no buffer arm and reads through the shared
scalar boundary. Reading buffers directly is worth about 1.8x against that, and 2.6x against
building every row as a value first. Answering 128 bits instead of 64 costs nothing.

### At the bindings

`python/benchmarks/xxhash.py --min-time 0.1 --repeat 5`, release wheel, against the `xxhash`
package binding C `libxxhash` on the same 1,080,000-byte payload:

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

Two honest numbers here. The `bytearray` and `memoryview` rows are 1.7x slower than `bytes`,
because those buffers cannot be borrowed under this crate's own safety rule and are read
through a bounded window instead - the cost of never allocating a copy of the payload. And at
one byte this binding is twice the C one: that is PyO3's call plus the buffer dispatch, and it
stops mattering by a kilobyte. Where the payload is worth vectorizing, the Rust XXH3 kernel is
ahead of the C build available here.

`npm run bench:xxhash` in `node/`, release addon, on the same 1,080,000-byte payload:

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

The one-byte row is 496 ns against Python's 166 ns: NAPI's call overhead, flat across every
size below a kilobyte, and gone by 64 KiB where both bindings reach the Rust kernel's own
speed. A `Buffer` is borrowed; a `string` is 7.5x slower because it is UTF-8 encoded on the way
in, so hash bytes rather than text when the text is already encoded somewhere.

## DigestAlgorithm: a hash over anything


`DigestAlgorithm` names one xxHash algorithm and is the only place a name selects an
implementation, the way `Codec` is for content codings. `Digest` is the value it answers -
the algorithm carried with the number, so `xxh64` and `xxh3-64`, both 64 bits wide, can
never be confused for one another. `Digester` is the runtime-selected streaming state, what
`Encoder` is to `Codec`.

```rust
use yggdryl::{Digest, DigestAlgorithm};

let digest = DigestAlgorithm::Xxh3.digest(b"AAPL");
assert_eq!(digest.algorithm(), DigestAlgorithm::Xxh3);
assert_eq!(Digest::from_str(&digest.to_string())?, digest);
assert_eq!(digest.into_bytes().len(), DigestAlgorithm::Xxh3.width());

// A caller who knows the algorithm at compile time uses the concrete state in
// `yggdryl::xxhash` and pays no dispatch; this is the form for one held in a
// variable.
let mut digester = DigestAlgorithm::Xxh3.digester();
digester.write_bytes(b"AA");
digester.write_bytes(b"PL");
assert_eq!(digester.as_digest(), digest);

// Only the XXH3 pair takes a custom secret; every algorithm takes a seed.
assert!(!DigestAlgorithm::Xxh64.is_secretable());
assert!(DigestAlgorithm::Xxh3.is_secretable());
assert_eq!(
    DigestAlgorithm::ALL.map(DigestAlgorithm::as_str),
    ["xxh32", "xxh64", "xxh3-64", "xxh3-128"],
);
```

[xxhash](xxhash.md) owns the four implementations, the resumable states, the handle and
Arrow surfaces, and the canonical `Scalar` byte feed every `stable_hash` reads.
