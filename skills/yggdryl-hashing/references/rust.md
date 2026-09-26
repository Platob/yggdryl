# yggdryl-hashing in Rust

`yggdryl::xxhash` (one-shots, states, `reader`/`writer`, `Hashed`,
`arrow::row_digests`) and `yggdryl::txhash` (`TxHash`, `TxHasher`,
`arrow::*`); `Digest`, `DigestAlgorithm` and `Digester` are at the crate root.
No feature flag is needed.

## Digest bytes in one call

The four one-shots answer their native width; a `Digest` carries the
algorithm with the number, so `xxh64` and `xxh3-64` never compare equal.

```rust
use yggdryl::xxhash;
use yggdryl::{Digest, DigestAlgorithm};

assert_eq!(xxhash::xxh32(b"abc"), 0x32d1_53ff);
assert_eq!(xxhash::xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
assert_eq!(xxhash::xxh3(b"abc"), 0x78af_5f94_892f_3950);
assert_eq!(xxhash::xxh128(b"abc"), 0x06b0_5ab6_733a_6185_78af_5f94_892f_3950);

let digest = DigestAlgorithm::Xxh3.digest(b"abc");
assert_eq!(digest, xxhash::digest(b"abc", DigestAlgorithm::Xxh3));
assert_eq!(digest.as_u64(), Some(xxhash::xxh3(b"abc")));
assert_eq!(digest.to_string(), "xxh3-64:78af5f94892f3950");
assert_eq!(Digest::from_str(&digest.to_string())?, digest);
// Canonical big-endian bytes at the algorithm's exact width.
assert_eq!(digest.into_bytes().len(), DigestAlgorithm::Xxh3.width());
assert_ne!(Digest::new(DigestAlgorithm::Xxh64, 7), Digest::new(DigestAlgorithm::Xxh3, 7));
```

## Seed a digest, or give XXH3 a secret

Every algorithm takes a seed; only the XXH3 pair takes a secret, consulted
only past 240 bytes and refused below `SECRET_MINIMUM_LENGTH` (136) whatever
the payload.

```rust
use yggdryl::xxhash::{self, SECRET_MINIMUM_LENGTH, Xxh3};
use yggdryl::{DigestAlgorithm, Error};

assert_ne!(xxhash::xxh64_with_seed(b"abc", 42), xxhash::xxh64(b"abc"));
assert!(DigestAlgorithm::Xxh3.is_secretable() && !DigestAlgorithm::Xxh64.is_secretable());

let secret = vec![0x5a_u8; SECRET_MINIMUM_LENGTH];
let payload = vec![0x11_u8; 241];
assert_ne!(xxhash::xxh3_with_secret(&payload, &secret)?, xxhash::xxh3(&payload));
assert_eq!(xxhash::xxh3_with_secret(b"AAPL", &secret)?, xxhash::xxh3(b"AAPL"));

let short = vec![0x5a_u8; SECRET_MINIMUM_LENGTH - 1];
assert!(matches!(Xxh3::from_secret(&short).unwrap_err(), Error::InvalidSecret { actual: 135, .. }));
```

## Stream bytes through a resumable state

Any split of the same bytes answers the one-shot digest, and reading the
digest does not consume the state. `DigestAlgorithm::digester()` is the
runtime-selected state for an algorithm held in a variable.

```rust
use yggdryl::DigestAlgorithm;
use yggdryl::xxhash::{Xxh3, xxh3};

let payload = b"AAPL,187.23";
let mut state = Xxh3::new();
for chunk in payload.chunks(4) {
    state.write_bytes(chunk);
}
assert_eq!(state.as_u64(), xxh3(payload));
state.write_bytes(b"\n");
assert_eq!(state.as_u64(), xxh3(b"AAPL,187.23\n"), "still running after an answer");

// A `Read` drains through the state in bounded chunks.
let mut fresh = Xxh3::new();
assert_eq!(fresh.write_reader(&mut &payload[..])?, payload.len() as u64);
assert_eq!(fresh.as_u64(), xxh3(payload));

let mut digester = DigestAlgorithm::from_str("xxh3-64")?.digester();
digester.write_bytes(payload);
assert_eq!(digester.as_digest(), fresh.as_digest());
state.clear();
assert_eq!(state.as_u64(), xxh3(b""), "clear returns to the constructed seed");
```

## Digest a stored resource without reading it into memory

`read_digest` and `read_range_digest` stream the handle through
`pstream_bytes`, one bounded chunk at a time, on every backend. A missing
resource digests as no bytes.

```rust
use yggdryl::holder::Buffer;
use yggdryl::xxhash;
use yggdryl::{DigestAlgorithm, IOBase};

let mut handle = Buffer::new();
handle.write_all_bytes(b"AAPL,187.23\n")?;
assert_eq!(handle.read_digest(DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(b"AAPL,187.23\n"));
assert_eq!(handle.read_range_digest(0, 4, DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(b"AAPL"));
assert_eq!(Buffer::new().read_digest(DigestAlgorithm::Xxh3)?.as_u64(), Some(xxhash::xxh3(b"")));
```

## Hash bytes you are already moving

`xxhash::reader` / `writer` hash a copy in the pass that was happening, and
`Hashed<H>` keeps a running digest of a handle's sequential writes so
`read_digest` never reads the bytes back. Rust only.

```rust
use std::io::{Read, Write};

use yggdryl::holder::Buffer;
use yggdryl::xxhash::{self, Hashed};
use yggdryl::{DigestAlgorithm, IOBase};

let payload = b"AAPL,187.23\n";

let mut source = xxhash::reader(payload.as_slice(), DigestAlgorithm::Xxh3);
let mut moved = Vec::new();
source.read_to_end(&mut moved)?;
assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3.digest(payload));

let mut target = xxhash::writer(Vec::new(), DigestAlgorithm::Xxh64);
target.write_all(payload)?;
assert_eq!(target.as_digest(), DigestAlgorithm::Xxh64.digest(payload));
assert_eq!(target.into_inner(), payload);

let mut hashed = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
hashed.write_all_bytes(b"AAPL,")?;
hashed.append_bytes(b"187.23\n")?;
hashed.flush()?;
assert_eq!(hashed.read_digest(DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(payload));
// A positional write makes the state stale; the next digest re-streams, same answer.
hashed.pwrite_all(0, b"MSFT")?;
assert_eq!(hashed.read_digest(DigestAlgorithm::Xxh3)?, DigestAlgorithm::Xxh3.digest(&hashed.read_all_bytes()?));
```

## Hash a value the same way in every language

`stable_hash` is XXH3-64 over the canonical value feed - a tag byte, then the
family's canonical form - so equal values hash equal across widths and the
number is the one Python and JavaScript answer for the same value.

```rust
use yggdryl::xxhash::{self, Xxh3};
use yggdryl::{DataType, DigestAlgorithm, Field, Scalar};

let symbol = Scalar::from("AAPL");
assert_eq!(symbol.stable_hash(), 2_200_133_337_491_048_159);
assert_eq!(Scalar::from(1_i64).stable_hash(), 3_061_886_165_360_509_404);
assert_eq!(Scalar::from(1_i8).stable_hash(), Scalar::from(1_i64).stable_hash());
assert_eq!(Field::new("c", DataType::utf8(), true).stable_hash(), 16_823_343_363_718_009_761);

// The feed frames the value: not the digest of its bare UTF-8.
assert_eq!(&*symbol.as_value_bytes().unwrap(), b"AAPL");
assert_ne!(symbol.stable_hash(), xxhash::xxh3(b"AAPL"));
assert_eq!(symbol.digest(DigestAlgorithm::Xxh3).as_u64(), Some(symbol.stable_hash()));
assert_ne!(Scalar::Null.stable_hash(), Scalar::from("").stable_hash());

// A row digest is the digest of the ordered row value.
let row = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]);
assert_eq!(row.stable_hash(), 8_969_590_880_303_877_378);
let mut state = Xxh3::new();
state.write_scalar(&row);
assert_eq!(state.as_digest(), row.digest(DigestAlgorithm::Xxh3));
```

## Declare a row-digest column and let the schema fill it

Mark one field `DIGEST:role=holder` and leave its sources ordinary columns;
`Field::apply_arrow_batch` (cast, transform, then digest) fills it, adding the
column where the root declares it. `as_digest().apply_arrow_batch` is the
digest step alone.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::Schema;
use yggdryl::{ArrowCastOptions, DataType, DigestAlgorithm, Field, Scalar, StructType};

let symbol = Field::new("symbol", DataType::utf8(), false);
let quantity = Field::new("quantity", DataType::Int64, false);
let mut key = Field::new("key", DataType::UInt64, false);
key.as_digest_mut().set_holder()?;
key.as_digest_mut().set_sources(["symbol"])?;
let root = DataType::from(StructType::from_fields([symbol.clone(), quantity.clone(), key])?).required_field("row");

let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![symbol.into_arrow_field()?, quantity.into_arrow_field()?])),
    vec![Arc::new(StringArray::from(vec!["AAPL", "MSFT"])), Arc::new(Int64Array::from(vec![100, 999]))],
)?;

let filled = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
let keys = filled.column(2).as_primitive::<UInt64Type>();
let expected = Scalar::from_sequence([Scalar::from("MSFT")]).digest(DigestAlgorithm::Xxh3);
assert_eq!(Some(keys.value(1)), expected.as_u64());
// Filling again changes nothing: a written cell is preserved.
assert_eq!(root.apply_arrow_batch(&filled, true, true, true, ArrowCastOptions::new())?, filled);
assert_eq!(root.as_digest().apply_arrow_batch(&batch)?.column(2), filled.column(2));
```

## Fill holders with a seeded state

A state's `apply_arrow_batch` fills every holder under its own seed and
secret; `force = true` recomputes written cells. The running digest is
untouched.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{RecordBatch, StringArray};
use arrow_schema::Schema;
use yggdryl::xxhash::Xxh3;
use yggdryl::{DataType, Field, Scalar, StructType};

let symbol = Field::new("symbol", DataType::utf8(), false);
let mut key = Field::new("key", DataType::UInt64, false);
key.as_digest_mut().set_holder()?;
let root = DataType::from(StructType::from_fields([symbol.clone(), key])?).required_field("row");
let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![symbol.into_arrow_field()?])),
    vec![Arc::new(StringArray::from(vec!["AAPL"]))],
)?;

let mut state = Xxh3::with_seed(7);
state.write_bytes(b"an unrelated running stream");
let running = state.as_u64();
let filled = state.apply_arrow_batch(&root, batch, false)?;
assert_eq!(state.as_u64(), running);

let mut expected = Xxh3::with_seed(7);
expected.write_scalar(&Scalar::from_sequence([Scalar::from("AAPL")]));
assert_eq!(filled.column(1).as_primitive::<UInt64Type>().value(0), expected.as_u64());
```

## Digest every row or cell of a batch

`row_digests` hashes each row's non-holder columns as one ordered value;
`column_digests` hashes each cell alone. Neither builds a `Scalar` per row.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use yggdryl::xxhash::arrow::{column_digests, row_digests};
use yggdryl::{DataType, DigestAlgorithm, Field, Scalar};

let symbols: ArrayRef = Arc::new(StringArray::from(vec![Some("AAPL"), None]));
let batch = RecordBatch::try_from_iter([
    ("symbol", Arc::clone(&symbols)),
    ("quantity", Arc::new(Int64Array::from(vec![100, 100])) as ArrayRef),
])?;

let rows = row_digests(&batch, DigestAlgorithm::Xxh3)?;
let rows = rows.as_primitive::<UInt64Type>();
let first = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]);
assert_eq!(Some(rows.value(0)), first.digest(DigestAlgorithm::Xxh3).as_u64());

let cells = column_digests(symbols, &Field::new("symbol", DataType::utf8(), true), DigestAlgorithm::Xxh3)?;
let cells = cells.as_primitive::<UInt64Type>();
assert_eq!(cells.value(0), Scalar::from("AAPL").stable_hash());
assert_eq!(cells.value(1), Scalar::Null.stable_hash(), "a null feeds the null tag");
```

## Couple an instant with a digest

A `TxHash` is a Unix count (microseconds unless named) then a digest: its
bytes sort by time within one unit and algorithm, and its spelling names both.

```rust
use yggdryl::txhash::{self, TxHash};
use yggdryl::xxhash;
use yggdryl::{DigestAlgorithm, TimeUnit};

let instant = 1_700_000_000_000_000; // 2023-11-14T22:13:20Z in microseconds
let value = txhash::txh3(b"AAPL", instant);
assert_eq!((value.unix(), value.unit(), value.width()), (instant, TimeUnit::Microsecond, 16));
assert_eq!(value.digest(), DigestAlgorithm::Xxh3.digest(b"AAPL"));
assert_eq!(value.to_string(), "1700000000000000@us:xxh3-64:dfb0aa5c25cce8c5");

let bytes = value.into_bytes();
assert_eq!(&bytes[..8], &instant.to_be_bytes());
assert_eq!(&bytes[8..], &xxhash::xxh3(b"AAPL").to_be_bytes());
assert_eq!(TxHash::from_bytes(TimeUnit::Microsecond, DigestAlgorithm::Xxh3, &bytes)?, value);
assert_eq!(TxHash::from_str(&value.to_string())?, value);

// A coarser unit floors the instant and keeps the digest; two units never compare equal.
let seconds = value.with_unit(TimeUnit::Second)?;
assert_eq!(seconds.unix(), 1_700_000_000);
assert_ne!(seconds, value);
assert!(txhash::txh3(b"B", 1) > txhash::txh3(b"A", 0), "value order is time first");
```

## Configure a hasher once for many values

`TxHasher` settles unit, algorithm, seed and secret once; `digest_scalar`
couples an instant with a value's canonical feed.

```rust
use yggdryl::txhash::TxHasher;
use yggdryl::xxhash::{self, Xxh3};
use yggdryl::{DigestAlgorithm, Scalar, TimeUnit};

let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh64)?.with_seed(7);
let value = seconds.digest(b"AAPL", 1_700_000_000);
assert_eq!(value.unit(), TimeUnit::Second);
assert_eq!(value.digest().as_u64(), Some(xxhash::xxh64_with_seed(b"AAPL", 7)));
assert_eq!(seconds.unix_of(&Scalar::from("2023-11-14T22:13:20Z"))?, 1_700_000_000);

let row = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]);
assert_ne!(seconds.digest_scalar(&row, 1_700_000_000).digest(), value.digest());

// A secret reaches a hasher through the state that holds it.
let secret = vec![0x5a_u8; xxhash::SECRET_MINIMUM_LENGTH];
let secretive = TxHasher::from_digester(TimeUnit::Millisecond, Xxh3::from_secret(&secret)?.into())?;
let long = vec![0x11_u8; 241];
assert_eq!(secretive.digest(&long, 5).digest().as_u64(), Some(xxhash::xxh3_with_secret(&long, &secret)?));
```

## Read an instant from any spelling

A zoned datetime already counts from the epoch, a naive one reads as UTC, a
date is its midnight, text parses with or without an offset; a finer count
floors.

```rust
use yggdryl::txhash;
use yggdryl::{Scalar, TimeUnit, Timezone};

let unit = TimeUnit::Microsecond;
let kolkata = Timezone::from_str("Asia/Kolkata")?;
let zoned = Scalar::from_datetime(1_700_000_000, TimeUnit::Second, kolkata)?;
assert_eq!(txhash::unix_from_scalar(&zoned, unit)?, 1_700_000_000_000_000);
assert_eq!(txhash::unix_from_scalar(&Scalar::from("1970-01-01T00:00:01+01:00"), unit)?, -3_599_000_000);
assert_eq!(txhash::unix_from_scalar(&Scalar::date32(1), unit)?, 86_400_000_000);
assert_eq!(txhash::restate_unix(1_999, TimeUnit::Nanosecond, unit)?, 1);
assert!(txhash::restate_unix(i64::MAX, TimeUnit::Second, unit).is_err(), "overflow is refused");
assert!(txhash::unix_now(TimeUnit::Second)? > 1_700_000_000);
```

## Project a sortable UUIDv7

`into_uuid` packs the microsecond and all 64 digest bits into RFC 9562
UUIDv7 - nothing hashed again, nothing narrowed; `into_sequenced_uuid` is the
event layout (millisecond, 12-bit sequence, seeded payload). Both need a
64-bit digest and an instant at or after the epoch.

```rust
use yggdryl::txhash::{self, TxHash};
use yggdryl::{Digest, DigestAlgorithm, TimeUnit};

let value = txhash::txh3(b"AAPL", 1_700_000_000_000_000);
let uuid = value.into_uuid()?;
assert_eq!(uuid.version(), 7);
assert_eq!(uuid.to_string(), "018bcfe5-6800-7003-9fb0-aa5c25cce8c5");
assert_eq!(value.into_sequenced_uuid(3, 7)?.to_string(), "018bcfe5-6800-7003-a8b4-89f75338ad76");

let one = Digest::new(DigestAlgorithm::Xxh64, 1);
let earlier = TxHash::new_in(0, TimeUnit::Nanosecond, one)?;
let later = TxHash::new_in(1_000, TimeUnit::Nanosecond, one)?;
assert!(earlier.into_uuid()? < later.into_uuid()?);
assert!(earlier.into_ordered_bytes()? < later.into_ordered_bytes()?);
assert!(txhash::txh128(b"AAPL", 0).into_uuid().is_err(), "a 128-bit digest is never narrowed");
assert!(TxHash::new_in(-1, TimeUnit::Nanosecond, one)?.into_uuid().is_err());
```

## Build a coupled column over a batch

`row_txhashes` lays each row's digest beside its instant as a
`fixed_size_binary(16)`; `decompose` and `compose` are inverses, and the
digest half is exactly `row_digests`.

```rust
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema};
use yggdryl::txhash;
use yggdryl::xxhash::arrow::row_digests;
use yggdryl::{DigestAlgorithm, TimeUnit};

let (unit, algorithm) = (TimeUnit::Microsecond, DigestAlgorithm::Xxh3);
let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![Field::new("symbol", DataType::Utf8, false)])),
    vec![Arc::new(StringArray::from(vec!["AAPL", "AAPL"]))],
)?;
let instants =
    TimestampMicrosecondArray::from(vec![1_700_000_000_000_000, 1_700_000_000_000_001]).with_timezone("UTC");

let coupled = txhash::arrow::row_txhashes(&batch, &instants, unit, algorithm)?;
assert_eq!(coupled.data_type(), &DataType::FixedSizeBinary(16));
let (times, digests) = txhash::arrow::decompose(coupled.as_ref(), unit, algorithm)?;
assert_eq!(&digests, &row_digests(&batch, algorithm)?);
assert_eq!(times.as_ref(), &instants as &dyn Array);
assert_eq!(&txhash::arrow::compose(times.as_ref(), digests.as_ref(), unit, algorithm)?, &coupled);
```

## Store an instant in front of a holder's digest

`DIGEST:time` names the field whose instant the holder stores first,
`DIGEST:unit` its resolution (microseconds when absent); the holder is a
`fixed_binary` of the coupled width.

```rust
use std::sync::Arc;

use arrow_array::{FixedSizeBinaryArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::Schema;
use yggdryl::txhash::TxHash;
use yggdryl::{ArrowCastOptions, DataType, DigestAlgorithm, Field, StructType, TimeUnit, Timezone};

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
        Arc::new(TimestampMicrosecondArray::from(vec![1_700_000_000_999_999]).with_timezone("UTC")),
        Arc::new(StringArray::from(vec!["MSFT"])),
    ],
)?;
let filled = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
let cells = filled.column(2).as_any().downcast_ref::<FixedSizeBinaryArray>().expect("fixed_size_binary");
let value = TxHash::from_bytes(TimeUnit::Second, DigestAlgorithm::Xxh3, cells.value(0))?;
assert_eq!(value.unix(), 1_700_000_000, "the instant floors to the declared unit");
```

## Gotchas in Rust

- `Scalar::stable_hash()` is not `xxh3` of the value's bytes: the feed is
  framed (tag byte, length). Use `as_value_bytes` for the bare payload.
- `Hasher::finish` on `Xxh128` is the low 64 bits; read `as_u128`.
- `TxHasher::with_seed` drops a secret; build `Xxh3::from_seed_and_secret`
  and pass it to `TxHasher::from_digester`.
- `TxHasher::from_digester` discards bytes already fed to the state - a
  hasher is a configuration, not a running digest.
- A holder or `DIGEST:time` under a serie, map, union, dictionary or run-end
  layout is refused by path; holders live in Structs only.
- `row_digests` ignores a holder's `DIGEST:sources`; narrowing belongs to
  `apply_arrow_batch`.
- A `variant` column is refused by name in `row_digests` / `column_digests`.
- xxHash is not cryptographic and is not Iceberg `bucket[N]` (murmur3).
