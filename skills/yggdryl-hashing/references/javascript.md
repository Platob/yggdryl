# yggdryl-hashing in JavaScript

`const { xxhash, txhash } = require('yggdryl')` - the two host namespaces
(their classes `Digest`, `Xxh32`, `Xxh64`, `Xxh3`, `Xxh128`, `TxHash`,
`TxHasher` are also top-level exports). Bytes are `Buffer`, `Uint8Array`,
`ArrayBuffer`, or a string as UTF-8. XXH32 answers a `number`; every wider
answer, seed and instant is a `bigint`.

## Digest bytes in one call

The one-shots answer the native width; a `Digest` carries its algorithm, so
`xxh64` and `xxh3-64` never compare equal.

```javascript
const assert = require('node:assert/strict')
const { xxhash } = require('yggdryl')

const payload = Buffer.from('abc')
assert.equal(xxhash.xxh32(payload), 0x32d153ff)
assert.equal(xxhash.xxh64(payload), 0x44bc2cf5ad770999n)
assert.equal(xxhash.xxh3(payload), 0x78af5f94892f3950n)
assert.equal(xxhash.xxh128(payload), 0x06b05ab6733a618578af5f94892f3950n)
assert.equal(xxhash.xxh3('abc'), xxhash.xxh3(new Uint8Array(payload)))

const digest = xxhash.digest(payload, 'xxh3-64')
assert.equal(digest.value(), xxhash.xxh3(payload))
assert.equal(digest.toString(), 'xxh3-64:78af5f94892f3950')
assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
assert.equal(digest.bytes().length, digest.width)
assert.equal(JSON.stringify({ digest }), '{"digest":"xxh3-64:78af5f94892f3950"}')
assert.ok(!xxhash.Digest.from('xxh64:0000000000000007').equals(xxhash.Digest.from('xxh3-64:0000000000000007')))
```

## Seed a digest, or give XXH3 a secret

Every algorithm takes `{ seed }` (or a bare `bigint`); only the XXH3 pair
takes `{ secret }`, consulted only past 240 bytes and refused below
`SECRET_MINIMUM_LENGTH` (136).

```javascript
const assert = require('node:assert/strict')
const { xxhash } = require('yggdryl')

assert.notEqual(xxhash.xxh64('abc', { seed: 42n }), xxhash.xxh64('abc'))
assert.equal(xxhash.xxh64('abc', 42n), xxhash.xxh64('abc', { seed: 42n }))

const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
const payload = Buffer.alloc(241)
assert.notEqual(xxhash.xxh3(payload, { secret }), xxhash.xxh3(payload))
assert.equal(xxhash.xxh3('AAPL', { secret }), xxhash.xxh3('AAPL'))
assert.throws(
  () => xxhash.xxh3(payload, { secret: new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH - 1) }),
  /at least 136 bytes, got 135/,
)
```

## Stream bytes through a resumable state

Any split of the same bytes answers the one-shot digest; `asDigest()` leaves
the state running and `clone()` forks it. JavaScript picks a state class for
an algorithm rather than a runtime `Digester`.

```javascript
const assert = require('node:assert/strict')
const { xxhash } = require('yggdryl')

const payload = Buffer.from('AAPL,187.23')
const state = new xxhash.Xxh3()
for (let start = 0; start < payload.length; start += 4) {
  state.writeBytes(payload.subarray(start, start + 4))
}
assert.equal(state.asDigest().value(), xxhash.xxh3(payload))

const fork = state.clone()
state.writeBytes('\n')
assert.equal(state.asDigest().value(), xxhash.xxh3('AAPL,187.23\n'))
assert.equal(fork.asDigest().value(), xxhash.xxh3(payload), 'a clone keeps what was fed')

const seeded = new xxhash.Xxh64(7n)
seeded.writeBytes(payload)
seeded.clear()
assert.equal(seeded.asDigest().value(), xxhash.xxh64('', 7n), 'clear keeps the constructed seed')
```

## Digest a stored resource without reading it into JavaScript

`readDigest` and `readRangeDigest` stream the resource natively, one bounded
chunk at a time, on every backend; no byte is copied into the JS heap, and a
missing resource digests as no bytes.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, xxhash } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  const file = path.join(root, 'trades.csv')
  fs.writeFileSync(file, 'AAPL,187.23\n')

  const handle = new IOBase(file)
  assert.ok(handle.readDigest().equals(xxhash.digest('AAPL,187.23\n', 'xxh3-64')))
  assert.ok(handle.readDigest('xxh64').equals(xxhash.digest('AAPL,187.23\n', 'xxh64')))
  assert.ok(handle.readRangeDigest(0, 4).equals(xxhash.digest('AAPL', 'xxh3-64')))
  assert.ok(new IOBase(path.join(root, 'never-written.csv')).readDigest().equals(xxhash.digest('', 'xxh3-64')))
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
```

## Hash bytes you are already moving

`xxhash.reader`, `xxhash.writer` and the write-through `Hashed` handle are
Rust only, and JavaScript states have no `writeReader`. Feed the chunks you
are copying to a state; for a stored resource, `readDigest` hashes without
crossing the bytes at all.

```javascript
const assert = require('node:assert/strict')
const { xxhash } = require('yggdryl')

const chunks = [Buffer.from('AAPL,'), Buffer.from('187.23\n')]
const state = new xxhash.Xxh64()
const copied = []
for (const chunk of chunks) {
  copied.push(chunk)
  state.writeBytes(chunk)
}
assert.equal(state.asDigest().value(), xxhash.xxh64(Buffer.concat(copied)))
```

## Hash a value the same way in every language

`stableHash()` is XXH3-64 over the canonical value feed, answered as a
`bigint` - the same number Rust's and Python's `stable_hash` answer.

```javascript
const assert = require('node:assert/strict')
const { Field, Scalar, xxhash } = require('yggdryl')

const symbol = Scalar.from('AAPL')
assert.equal(symbol.stableHash(), 2_200_133_337_491_048_159n)
assert.equal(Scalar.from(1).stableHash(), 3_061_886_165_360_509_404n)
assert.equal(Scalar.from(1n).stableHash(), Scalar.from(1).stableHash())
assert.equal(Scalar.from(['AAPL', 100n]).stableHash(), 8_969_590_880_303_877_378n)
assert.equal(new Field('c', 'utf8').stableHash(), 16_823_343_363_718_009_761n)

// A framed feed, not the bare UTF-8; equal values answer one digest.
assert.notEqual(symbol.stableHash(), xxhash.xxh3('AAPL'))
assert.equal(symbol.digest().value(), symbol.stableHash())
assert.ok(Scalar.decimal(100n, 2).digest().equals(Scalar.decimal(1n, 0).digest()))
assert.ok(!Scalar.from('1').digest().equals(Scalar.from(Buffer.from('1')).digest()))

const state = new xxhash.Xxh3()
state.writeScalar(symbol)
assert.ok(state.asDigest().equals(symbol.digest()))
```

## Declare a row-digest column and fill it

Mark one field `DIGEST:role=holder` (its `digest` view, or metadata at
construction) and leave its sources ordinary columns; a state's
`applyArrowBatch(root, batch, force?)` adds and fills it through a copied
Arrow IPC batch.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { DataType, Field, Scalar, xxhash } = require('yggdryl')

const key = new Field('key', 'uint64', false)
key.digest.set('role', 'holder')
key.digest.set('sources', '["symbol"]')
const root = new Field(
  'row',
  DataType.fromFields([new Field('symbol', 'utf8', false), new Field('quantity', 'int64', false), key]),
  false,
)
assert.deepEqual(root.digestFieldNames(), ['symbol', 'quantity'])

const batch = new arrow.Table({
  symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
  quantity: arrow.vectorFromArray([100n, 999n], new arrow.Int64()),
}).batches[0]

const filled = new xxhash.Xxh3().applyArrowBatch(root, batch)
assert.deepEqual(filled.schema.fields.map((field) => field.name), ['symbol', 'quantity', 'key'])
assert.deepEqual([...filled.getChild('key')], [Scalar.from(['AAPL']).stableHash(), Scalar.from(['MSFT']).stableHash()])
```

## Fill holders with a seeded state

The state's seed and secret reach every holder of its width; the running
digest is untouched, and `force = true` recomputes written cells.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { DataType, Field, Scalar, xxhash } = require('yggdryl')

const key = new Field('key', 'uint64', false, { 'DIGEST:role': 'holder' })
const root = new Field('row', DataType.fromFields([new Field('symbol', 'utf8', false), key]), false)
const batch = new arrow.Table({ symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()) }).batches[0]

const state = new xxhash.Xxh3(7n)
state.writeBytes('an unrelated running stream')
const running = state.asDigest()
const filled = state.applyArrowBatch(root, batch)
assert.ok(state.asDigest().equals(running))

const expected = new xxhash.Xxh3(7n)
expected.writeScalar(Scalar.from(['AAPL']))
assert.deepEqual([...filled.getChild('key')], [expected.asDigest().value()])
assert.deepEqual([...state.applyArrowBatch(root, filled, true).getChild('key')], [expected.asDigest().value()])
```

## Digest every row or cell of a batch

Not bound in JavaScript: `row_digests` and `column_digests` are Rust and
Python only. Declare a holder column and fill it with a state's
`applyArrowBatch` (above), or digest one row value with
`Scalar.from([...]).digest(algorithm)`.

## Couple an instant with a digest

A `TxHash` is a Unix count (microseconds unless named) then a digest: its
bytes sort by time within one unit and algorithm, and its spelling names both.

```javascript
const assert = require('node:assert/strict')
const { txhash, xxhash } = require('yggdryl')

const instant = 1_700_000_000_000_000n // 2023-11-14T22:13:20Z in microseconds
const value = txhash.txh3('AAPL', instant)
assert.deepEqual([value.unix, value.unit, value.width], [instant, 'us', 16])
assert.equal(value.digest.value(), xxhash.xxh3('AAPL'))
assert.equal(value.toString(), '1700000000000000@us:xxh3-64:dfb0aa5c25cce8c5')

const bytes = Buffer.from(value.bytes())
assert.equal(bytes.readBigInt64BE(0), instant)
assert.ok(txhash.TxHash.fromBytes('us', 'xxh3-64', value.bytes()).equals(value))
assert.ok(txhash.TxHash.from(value.toString()).equals(value))

const seconds = value.withUnit('s')
assert.equal(seconds.unix, 1_700_000_000n)
assert.ok(seconds.digest.equals(value.digest) && !seconds.equals(value))
assert.equal(txhash.txh3('B', 1n).compare(txhash.txh3('A', 0n)), 1)
```

## Configure a hasher once for many values

`TxHasher(algorithm, unit, seed)` settles the configuration once;
`fromState` carries an XXH3 secret in; `digestScalar` couples an instant with
a value's canonical feed.

```javascript
const assert = require('node:assert/strict')
const { Scalar, txhash, xxhash } = require('yggdryl')

const seconds = new txhash.TxHasher('xxh64', 's', 7n)
const value = seconds.digest('AAPL', 1_700_000_000n)
assert.equal(value.unit, 's')
assert.equal(value.digest.value(), xxhash.xxh64('AAPL', { seed: 7n }))
assert.equal(seconds.unixOf('2023-11-14T22:13:20Z'), 1_700_000_000n)
assert.ok(!seconds.digestScalar(Scalar.from(['AAPL', 100]), 1_700_000_000n).digest.equals(value.digest))

const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
const secretive = txhash.TxHasher.fromState(new xxhash.Xxh3(0n, secret), 'ms')
const long = Buffer.alloc(241, 0x11)
assert.equal(secretive.digest(long, 5n).digest.value(), xxhash.xxh3(long, { secret }))
```

## Read an instant from any spelling

`unixOf` reads a `bigint` or safe-integer count, a `Date` (its UTC instant),
timestamp text, or a `Scalar`; a finer count floors.

```javascript
const assert = require('node:assert/strict')
const { txhash } = require('yggdryl')

const aware = new Date('2023-11-14T22:13:20Z')
assert.equal(txhash.unixOf(aware), 1_700_000_000_000_000n)
assert.equal(txhash.unixOf('2023-11-15T03:43:20+05:30'), 1_700_000_000_000_000n)
assert.equal(txhash.unixOf('1970-01-01T00:00:01+01:00'), -3_599_000_000n)
assert.equal(txhash.unixOf(aware, 's'), 1_700_000_000n)
assert.equal(txhash.restateUnix(1_999n, 'ns', 'us'), 1n)
assert.ok(txhash.unixNow('s') > 1_700_000_000n)
assert.throws(() => txhash.unixOf(true), TypeError)
```

## Project a sortable UUIDv7

`intoUuid()` packs the microsecond and all 64 digest bits into UUIDv7 and
answers a `uuid` `Scalar`; `intoSequencedUuid(sequence, seed)` is the event
layout. Both need a 64-bit digest and an instant at or after the epoch.

```javascript
const assert = require('node:assert/strict')
const { txhash, xxhash } = require('yggdryl')

const value = txhash.txh3('AAPL', 1_700_000_000_000_000n)
const uuid = value.intoUuid()
assert.equal(uuid.dtype.id, 'uuid')
assert.equal(uuid.asJs(), '018bcfe5-6800-7003-9fb0-aa5c25cce8c5')
assert.equal(value.intoSequencedUuid(3n, 7n).asJs(), '018bcfe5-6800-7003-a8b4-89f75338ad76')

const one = xxhash.Digest.from('xxh64:0000000000000001')
const earlier = txhash.TxHash.fromParts(0n, one, 'ns')
const later = txhash.TxHash.fromParts(1_000n, one, 'ns')
assert.ok(earlier.intoUuid().asJs() < later.intoUuid().asJs())
assert.throws(() => txhash.txh128('AAPL', 0n).intoUuid(), /expected a 64-bit digest for UUIDv7/)
assert.throws(() => txhash.TxHash.fromParts(-1n, one, 'ns').intoUuid(), /UUIDv7/)
```

## Build a coupled column over a batch

Not bound in JavaScript: `row_txhashes`, `column_txhashes`, `compose`,
`decompose` and `unix_array` are Rust and Python only. Couple one value at a
time with `TxHasher.digestScalar`, or declare a coupled holder (below).

## Store an instant in front of a holder's digest

`DIGEST:time` names the field whose instant the holder stores first,
`DIGEST:unit` its resolution (microseconds when absent); `TxHasher`'s
`applyArrowBatch` fills it.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { DataType, Field, Scalar, txhash } = require('yggdryl')

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
  event: arrow.vectorFromArray([1_700_000_001n], new arrow.Int64()),
  symbol: arrow.vectorFromArray(['MSFT'], new arrow.Utf8()),
}).batches[0]

const filled = new txhash.TxHasher().applyArrowBatch(root, batch)
const value = txhash.TxHash.fromBytes('s', 'xxh3-64', filled.getChild('key').get(0))
assert.equal(value.unix, 1_700_000_001n)
assert.ok(value.digest.equals(Scalar.from([1_700_000_001n, 'MSFT']).digest()))
```

## Gotchas in JavaScript

- `stableHash()`, `xxh64`/`xxh3`/`xxh128`, every seed, `unix` and instant are
  `bigint`; only `xxh32` and an `Xxh32` seed are `number`. Mixing `bigint` and
  `number` in arithmetic throws.
- A `number` instant must be a safe integer; a fraction is refused, and
  `true` is a `TypeError`, never `1`.
- Holder fills copy the batch through Arrow IPC both ways; keep large tables
  native and fill once per batch.
- `row_digests`, `column_digests`, coupled columns, `Digester`,
  `as_value_bytes`, `reader`/`writer` and `Hashed` are not bound.
- `Digest` and `TxHash` serialize as their canonical spelling under
  `JSON.stringify`; read them back with `Digest.from` / `TxHash.from`.
- xxHash is not cryptographic and is not Iceberg `bucket[N]` (murmur3).
