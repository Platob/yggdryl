# yggdryl-hashing in Python

`from yggdryl import xxhash, txhash` - the two host modules. Bytes are
`bytes`, `bytearray`, `memoryview`, any buffer, or a `str` as UTF-8; Arrow
crosses as `pyarrow`. Algorithms are spelled `"xxh32"`, `"xxh64"`,
`"xxh3-64"` (the default), `"xxh3-128"`.

## Digest bytes in one call

The one-shots answer a plain `int`; a `Digest` carries its algorithm, so
`xxh64` and `xxh3-64` never compare equal.

```python
from yggdryl import xxhash

assert xxhash.xxh32(b"abc") == 0x32D153FF
assert xxhash.xxh64(b"abc") == 0x44BC2CF5AD770999
assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
assert xxhash.xxh128(b"abc") == 0x06B05AB6733A618578AF5F94892F3950
assert xxhash.xxh3("abc") == xxhash.xxh3(b"abc")

digest = xxhash.digest(b"abc", "xxh3-64")
assert int(digest) == xxhash.xxh3(b"abc")
assert str(digest) == "xxh3-64:78af5f94892f3950"
assert xxhash.Digest(str(digest)) == digest
assert len(bytes(digest)) == digest.width == 8
assert xxhash.Digest.from_int("xxh64", 7) != xxhash.Digest.from_int("xxh3-64", 7)
```

## Seed a digest, or give XXH3 a secret

Every algorithm takes `seed=`; only the XXH3 pair takes `secret=`, consulted
only past 240 bytes and refused below `SECRET_MINIMUM_LENGTH` (136).

```python
from yggdryl import xxhash

assert xxhash.xxh64(b"abc", seed=42) != xxhash.xxh64(b"abc")
assert xxhash.is_secretable("xxh3-64") and not xxhash.is_secretable("xxh64")

secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
payload = bytes(241)
assert xxhash.xxh3(payload, secret=secret) != xxhash.xxh3(payload)
assert xxhash.xxh3(b"AAPL", secret=secret) == xxhash.xxh3(b"AAPL")

try:
    xxhash.Xxh3(secret=bytes(xxhash.SECRET_MINIMUM_LENGTH - 1))
    raise AssertionError("a short secret is refused")
except ValueError as error:
    assert "at least 136 bytes, got 135" in str(error)
```

## Stream bytes through a resumable state

Any split of the same bytes answers the one-shot digest; reading the digest
leaves the state running. `write_reader` drains a binary file object in
bounded chunks; `Digester(algorithm)` picks the state at runtime.

```python
import io

from yggdryl import xxhash

payload = b"AAPL,187.23"
state = xxhash.Xxh3()
for start in range(0, len(payload), 4):
    state.write_bytes(payload[start : start + 4])
assert state.as_int() == xxhash.xxh3(payload)
state.write_bytes(b"\n")
assert state.as_int() == xxhash.xxh3(b"AAPL,187.23\n")

fresh = xxhash.Xxh3()
assert fresh.write_reader(io.BytesIO(payload)) == len(payload)
assert fresh.as_digest() == xxhash.digest(payload, "xxh3-64")

digester = xxhash.Digester("xxh3-64")
digester.write_bytes(payload)
assert digester.as_digest() == fresh.as_digest()
state.clear()
assert state.as_int() == xxhash.xxh3(b"")
```

## Digest a stored resource without reading it into Python

`IOBase.read_digest` and `read_range_digest` stream the resource natively, one
bounded chunk at a time, on every backend; a missing resource digests as no
bytes.

```python
import tempfile
from pathlib import Path

from yggdryl import IOBase, xxhash

with tempfile.TemporaryDirectory() as root:
    path = Path(root) / "trades.csv"
    path.write_bytes(b"AAPL,187.23\n")

    handle = IOBase(path)
    assert handle.read_digest() == xxhash.digest(b"AAPL,187.23\n", "xxh3-64")
    assert handle.read_digest("xxh64") == xxhash.digest(b"AAPL,187.23\n", "xxh64")
    assert handle.read_range_digest(0, 4) == xxhash.digest(b"AAPL", "xxh3-64")
    assert IOBase(Path(root) / "never-written.csv").read_digest() == xxhash.digest(b"", "xxh3-64")
```

## Hash bytes you are already moving

`xxhash.reader`, `xxhash.writer` and the write-through `Hashed` handle are
Rust only. In Python, feed the chunks you are copying to a state
(`write_bytes`) or hand the file object to `write_reader`; for a stored
resource, `read_digest` never crosses the bytes into Python at all.

```python
import io

from yggdryl import xxhash

source = io.BytesIO(b"AAPL,187.23\n" * 1000)
target = io.BytesIO()
state = xxhash.Xxh64()
while chunk := source.read(4096):
    target.write(chunk)
    state.write_bytes(chunk)
assert state.as_digest() == xxhash.digest(target.getvalue(), "xxh64")
```

## Hash a value the same way in every language

`stable_hash` is XXH3-64 over the canonical value feed, so equal values hash
equal across widths and the number is the one Rust and JavaScript answer.

```python
from yggdryl import DataType, Field, Scalar, xxhash

symbol = Scalar.from_("AAPL")
assert symbol.stable_hash() == 2_200_133_337_491_048_159
assert Scalar.from_(1).stable_hash() == 3_061_886_165_360_509_404
assert Scalar.from_(["AAPL", 100]).stable_hash() == 8_969_590_880_303_877_378
assert Field("c", "utf8").stable_hash() == 16_823_343_363_718_009_761

# Equal values, one digest; a framed feed, not the bare UTF-8.
assert DataType("float32").scalar(1.5).digest() == DataType("float64").scalar(1.5).digest()
assert Scalar.decimal(100, 2).digest() == Scalar.decimal(1, 0).digest()
assert symbol.as_value_bytes() == b"AAPL" and symbol.stable_hash() != xxhash.xxh3(b"AAPL")
assert int(symbol.digest()) == symbol.stable_hash()
assert Scalar.from_(None).digest() != Scalar.from_("").digest()

state = xxhash.Xxh3()
state.write_scalar(symbol)
assert state.as_digest() == symbol.digest()
```

## Declare a row-digest column and let the schema fill it

Mark one field a holder through its `digest` view and leave its sources
ordinary columns; `Field.apply_arrow_batch` (cast, transform, then digest)
adds and fills it. `root.digest.apply_arrow_batch` is the digest step alone.

```python
import pyarrow as pa

from yggdryl import DataType, Field, Scalar

key = Field("key", "uint64", nullable=False)
key.digest.set_holder()
key.digest.sources = ["symbol"]
root = Field(
    "row",
    DataType.from_fields([Field("symbol", "utf8", nullable=False), Field("quantity", "int64", nullable=False), key]),
    nullable=False,
)
batch = pa.record_batch({"symbol": ["AAPL", "MSFT"], "quantity": pa.array([100, 999], pa.int64())})

filled = root.apply_arrow_batch(batch)
assert filled.schema.names == ["symbol", "quantity", "key"]
assert filled.column("key").to_pylist() == [Scalar.from_([s]).stable_hash() for s in ("AAPL", "MSFT")]
assert root.apply_arrow_batch(filled) == filled, "a written cell is preserved"
assert root.digest.apply_arrow_batch(batch).column("key") == filled.column("key")
assert root.digest_field_names == ["symbol", "quantity"]
```

## Fill holders with a seeded state

A state's `apply_arrow_batch` fills every holder under its own seed and
secret; `force=True` recomputes written cells. The running digest is
untouched.

```python
import pyarrow as pa

from yggdryl import DataType, Field, Scalar, xxhash

key = Field("key", "uint64", nullable=False)
key.digest.set_holder()
root = Field("row", DataType.from_fields([Field("symbol", "utf8", nullable=False), key]), nullable=False)
batch = pa.record_batch({"symbol": ["AAPL"]})

state = xxhash.Xxh3(seed=7)
state.write_bytes(b"an unrelated running stream")
running = state.as_digest()
filled = state.apply_arrow_batch(root, batch)
assert state.as_digest() == running

expected = xxhash.Xxh3(seed=7)
expected.write_scalar(Scalar.from_(["AAPL"]))
assert filled.column("key").to_pylist() == [expected.as_int()]
assert state.apply_arrow_batch(root, filled, force=True) == filled
```

## Digest every row or cell of a batch

`row_digests` hashes each row's non-holder columns as one ordered value;
`column_digests` hashes each cell alone. Both answer a `pyarrow` array.

```python
import pyarrow as pa

from yggdryl import Field, Scalar, xxhash

batch = pa.record_batch({"symbol": ["AAPL", None], "quantity": pa.array([100, 100], pa.int64())})

rows = xxhash.row_digests(batch)
assert rows.type == pa.uint64()
assert rows[0].as_py() == Scalar.from_(["AAPL", 100]).stable_hash()
assert xxhash.row_digests(batch, "xxh3-128").type == pa.binary(16)

cells = xxhash.column_digests(batch.column("symbol"), Field("symbol", "utf8"))
assert cells.to_pylist() == [Scalar.from_("AAPL").stable_hash(), Scalar.from_(None).stable_hash()]
```

## Couple an instant with a digest

A `TxHash` is a Unix count (microseconds unless named) then a digest: its
bytes sort by time within one unit and algorithm, and its spelling names both.

```python
from yggdryl import txhash, xxhash

instant = 1_700_000_000_000_000  # 2023-11-14T22:13:20Z in microseconds
value = txhash.txh3(b"AAPL", instant)
assert (value.unix, value.unit, value.width) == (instant, "us", 16)
assert int(value.digest) == xxhash.xxh3(b"AAPL")
assert str(value) == "1700000000000000@us:xxh3-64:dfb0aa5c25cce8c5"

assert bytes(value) == instant.to_bytes(8, "big", signed=True) + bytes(value.digest)
assert txhash.TxHash.from_bytes("us", "xxh3-64", bytes(value)) == value
assert txhash.TxHash(str(value)) == value

seconds = value.with_unit("s")
assert seconds.unix == 1_700_000_000 and seconds.digest == value.digest
assert seconds != value
assert txhash.txh3(b"B", 1) > txhash.txh3(b"A", 0)
```

## Configure a hasher once for many values

`TxHasher` settles unit, algorithm, seed and secret once; `digest_scalar`
couples an instant with a value's canonical feed.

```python
from yggdryl import Scalar, txhash, xxhash

seconds = txhash.TxHasher("xxh64", unit="s", seed=7)
value = seconds.digest(b"AAPL", 1_700_000_000)
assert value.unit == "s"
assert int(value.digest) == xxhash.xxh64(b"AAPL", seed=7)
assert seconds.unix_of("2023-11-14T22:13:20Z") == 1_700_000_000
assert seconds.digest_scalar(Scalar.from_(["AAPL", 100]), 1_700_000_000).digest != value.digest

secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
secretive = txhash.TxHasher.from_state(xxhash.Xxh3(secret=secret), unit="ms")
long = b"\x11" * 241
assert int(secretive.digest(long, 5).digest) == xxhash.xxh3(long, secret=secret)
```

## Read an instant from any spelling

`unix_of` reads an `int`, a `datetime` (any zone; naive reads as UTC), a
`date` (its midnight), timestamp text or a `Scalar`; a finer count floors.

```python
import datetime as dt

from yggdryl import txhash

aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
kolkata = aware.astimezone(dt.timezone(dt.timedelta(hours=5, minutes=30)))
assert txhash.unix_of(kolkata) == txhash.unix_of(aware) == 1_700_000_000_000_000
assert txhash.unix_of("1970-01-01T00:00:01+01:00") == -3_599_000_000
assert txhash.unix_of(dt.date(1970, 1, 2)) == 86_400_000_000
assert txhash.unix_of(aware, unit="s") == 1_700_000_000
assert txhash.restate_unix(1_999, "ns", "us") == 1
assert txhash.unix_now("s") > 1_700_000_000
try:
    txhash.unix_of(True)
    raise AssertionError("a bool is not an instant")
except TypeError:
    pass
```

## Project a sortable UUIDv7

`into_uuid` packs the microsecond and all 64 digest bits into UUIDv7 and
answers a `uuid` `Scalar`; `into_sequenced_uuid(sequence, seed)` is the event
layout. Both need a 64-bit digest and an instant at or after the epoch.

```python
from yggdryl import DataType, txhash, xxhash

value = txhash.txh3(b"AAPL", 1_700_000_000_000_000)
uuid = value.into_uuid()
assert uuid.dtype == DataType("uuid")
assert uuid.as_py() == "018bcfe5-6800-7003-9fb0-aa5c25cce8c5"
assert value.into_sequenced_uuid(3, 7).as_py() == "018bcfe5-6800-7003-a8b4-89f75338ad76"

one = xxhash.Digest.from_int("xxh64", 1)
earlier = txhash.TxHash.from_parts(0, one, unit="ns")
later = txhash.TxHash.from_parts(1_000, one, unit="ns")
assert earlier.into_uuid() < later.into_uuid()
for refused in (txhash.txh128(b"AAPL", 0), txhash.TxHash.from_parts(-1, one, unit="ns")):
    try:
        refused.into_uuid()
        raise AssertionError("no UUIDv7 for a 128-bit digest or a pre-epoch instant")
    except ValueError as error:
        assert "UUIDv7" in str(error)
```

## Build a coupled column over a batch

`row_txhashes` lays each row's digest beside its instant as
`fixed_size_binary[16]`; `decompose` and `compose` are inverses, and the
digest half is exactly `row_digests`.

```python
import pyarrow as pa

from yggdryl import txhash, xxhash

batch = pa.record_batch({"symbol": ["AAPL", "AAPL"]})
instants = pa.array([1_700_000_000_000_000, 1_700_000_000_000_001], pa.timestamp("us", tz="UTC"))

coupled = txhash.row_txhashes(batch, instants)
assert coupled.type == pa.binary(16)
times, digests = txhash.decompose(coupled)
assert digests == xxhash.row_digests(batch)
assert times == instants
assert txhash.compose(times, digests) == coupled
assert digests[0] == digests[1] and coupled[0] != coupled[1]
assert txhash.unix_array(pa.array([1_700_000_000, None], pa.timestamp("s"))).to_pylist() == [
    1_700_000_000_000_000,
    None,
]
```

## Store an instant in front of a holder's digest

`digest.time` names the field whose instant the holder stores first,
`digest.unit` its resolution (microseconds when absent); the holder is a
`fixed_size_binary` of the coupled width.

```python
import datetime as dt

import pyarrow as pa

from yggdryl import DataType, Field, Scalar, txhash

key = Field("key", "fixed_size_binary[16]", nullable=False)
key.digest.set_holder()
key.digest.time = "event"
key.digest.unit = "s"
root = Field(
    "row",
    DataType.from_fields(
        [Field("event", "timestamp[us, UTC]", nullable=False), Field("symbol", "utf8", nullable=False), key]
    ),
    nullable=False,
)
batch = pa.record_batch(
    {"event": pa.array([1_700_000_000_999_999], pa.timestamp("us", tz="UTC")), "symbol": ["MSFT"]}
)

filled = root.apply_arrow_batch(batch)
value = txhash.TxHash.from_bytes("s", "xxh3-64", filled.column("key")[0].as_py())
assert value.unix == 1_700_000_000
row = Scalar.from_([dt.datetime(2023, 11, 14, 22, 13, 20, 999_999, tzinfo=dt.timezone.utc), "MSFT"])
assert value.digest == row.digest()
```

## Gotchas in Python

- `Scalar.stable_hash()` is not `xxhash.xxh3(value_bytes)`: the feed is
  framed. `as_value_bytes()` is the bare payload.
- A `bytearray`/`memoryview` is copied in a bounded window (about 1.7x slower
  than `bytes`); a `str` is its UTF-8.
- States are mutable and unhashable; `Digest` and `TxHash` are immutable,
  hashable, ordered and picklable.
- `True` is refused as an instant (`TypeError`), never read as `1`.
- `TxHasher(seed=...)` drops a secret; give a secret through
  `TxHasher.from_state(xxhash.Xxh3(seed=..., secret=...))`.
- `row_digests` ignores a holder's `DIGEST:sources`; narrowing belongs to the
  holder fill.
- xxHash is not cryptographic and is not Iceberg `bucket[N]` (murmur3).
