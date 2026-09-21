'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')
const arrow = require('apache-arrow')

const { DataType, Digest, Field, Scalar, TxHash, TxHasher, txhash, xxhash } = require('yggdryl')


const INSTANT = 1_700_000_000_000_000n
const PAYLOAD = Buffer.from('{"symbol": "AAPL", "price": 187.23}\n'.repeat(64))

test('one-shots couple the instant with the plain digest', () => {
  const cases = [
    ['xxh32', txhash.txh32, xxhash.xxh32, 12],
    ['xxh64', txhash.txh64, xxhash.xxh64, 16],
    ['xxh3-64', txhash.txh3, xxhash.xxh3, 16],
    ['xxh3-128', txhash.txh128, xxhash.xxh128, 24],
  ]
  for (const [algorithm, coupled, plain, width] of cases) {
    const value = coupled(PAYLOAD, INSTANT)
    assert.equal(value.unix, INSTANT, algorithm)
    assert.equal(value.unit, 'us', algorithm)
    assert.equal(value.algorithm, algorithm)
    assert.equal(value.width, width)
    assert.equal(value.bytes().length, width)
    assert.equal(value.digest.value(), plain(PAYLOAD), algorithm)
    assert.ok(value.equals(txhash.digest(PAYLOAD, INSTANT, algorithm)), algorithm)
    assert.equal(coupled(PAYLOAD, INSTANT, { seed: 7 }).digest.value(), plain(PAYLOAD, { seed: 7 }), algorithm)
    assert.equal(txhash.width(algorithm), width)
    assert.ok(txhash.dtype(algorithm).equals(DataType.from(`fixed_size_binary[${width}]`)))
    assert.ok(value.dtype.equals(txhash.dtype(algorithm)))
  }
  assert.equal(txhash.DEFAULT_UNIT, 'us')
  assert.equal(txhash.UNIX_WIDTH, 8)
  assert.equal(txhash.TxHash, TxHash)
  assert.equal(txhash.TxHasher, TxHasher)
})

test('the bytes are the instant then the digest, and the spelling round-trips', () => {
  const value = txhash.txh3(PAYLOAD, INSTANT)
  const bytes = Buffer.from(value.bytes())
  assert.equal(bytes.readBigInt64BE(0), INSTANT)
  assert.deepEqual(bytes.subarray(8), Buffer.from(value.digest.bytes()))
  assert.ok(TxHash.fromBytes('us', 'xxh3-64', bytes).equals(value))
  assert.equal(value.toString(), `${INSTANT}@us:${value.digest.toString()}`)
  assert.ok(TxHash.from(value.toString()).equals(value))
  assert.ok(new TxHash(value.toString()).equals(value))
  assert.ok(TxHash.from(value).equals(value))
  assert.equal(JSON.stringify({ value }), JSON.stringify({ value: value.toString() }))
  assert.ok(value.clone().equals(value))
  assert.equal(value.stableHash(), txhash.txh3(PAYLOAD, INSTANT).stableHash())
  assert.throws(() => TxHash.fromBytes('us', 'xxh3-128', bytes), /expected 24 bytes/)
  assert.throws(() => new TxHash('1@d:xxh3-64:0000000000000000'), /unix unit/)
  const negative = txhash.txh3(PAYLOAD, -1n)
  assert.deepEqual(Buffer.from(negative.bytes()).subarray(0, 8), Buffer.alloc(8, 0xff))
})

test('values order by instant then digest and never across algorithms', () => {
  const earlier = txhash.txh3(PAYLOAD, INSTANT)
  const later = txhash.txh3(PAYLOAD, INSTANT + 1n)
  assert.equal(earlier.compare(later), -1)
  assert.equal(later.compare(earlier), 1)
  assert.equal(earlier.compare(txhash.txh3(PAYLOAD, INSTANT)), 0)
  assert.ok(Buffer.compare(Buffer.from(earlier.bytes()), Buffer.from(later.bytes())) < 0)
  assert.ok(!earlier.equals(txhash.txh64(PAYLOAD, INSTANT)))
  assert.ok(!earlier.equals(earlier.withUnit('s')))
})

// Parity with rust/tests/hashing/txhash.rs `uuid_projection_*` and the
// `TxHash::into_uuid` doc example; every expectation below is pinned there.
const I64_MIN = -(2n ** 63n)
const I64_MAX = 2n ** 63n - 1n
const hex = (payload, digits) => payload.toString(16).padStart(digits, '0')

test('intoUuid packs the microsecond instant as a UUIDv7', () => {
  const digest = Digest.from('xxh64:0123456789abcdef')
  for (const [nanoseconds, expected] of [
    [0n, '00000000-0000-7000-8123-456789abcdef'],
    // The sub-microsecond nanoseconds are floored away.
    [999n, '00000000-0000-7000-8123-456789abcdef'],
    [1_000n, '00000000-0000-7004-8123-456789abcdef'],
    [999_999n, '00000000-0000-7ffb-8123-456789abcdef'],
    [1_000_000n, '00000000-0001-7000-8123-456789abcdef'],
    [1_000_000_000n, '00000000-03e8-7000-8123-456789abcdef'],
    [I64_MAX, '08637bd0-5af6-7c66-8123-456789abcdef'],
  ]) {
    const value = TxHash.fromParts(nanoseconds, digest, 'ns')
    const raw = Buffer.from(value.bytes())
    const projected = value.intoUuid()
    assert.ok(projected instanceof Scalar, String(nanoseconds))
    assert.equal(projected.dtype.id, 'uuid', String(nanoseconds))
    assert.equal(projected.asJs(), expected, String(nanoseconds))
    assert.deepEqual(Buffer.from(value.bytes()), raw, 'projection does not mutate the value')
    assert.equal(raw.readBigInt64BE(0), nanoseconds)
  }

  // The doc example: a UUIDv7 ordered by instant to the microsecond, and
  // none to project before the epoch, where the raw bytes still hold the
  // two's-complement count.
  const one = Digest.from('xxh64:0000000000000001')
  const epoch = TxHash.fromParts(0n, one, 'ns')
  const micro = TxHash.fromParts(1n, one, 'us')
  const before = TxHash.fromParts(-1n, one, 'ns')
  assert.equal(epoch.intoUuid().asJs(), '00000000-0000-7000-8000-000000000001')
  assert.ok(epoch.intoUuid().asJs() < micro.intoUuid().asJs())
  assert.throws(() => before.intoUuid(), /UUIDv7/)
  assert.ok(Buffer.compare(Buffer.from(before.bytes()), Buffer.from(epoch.bytes())) > 0)
})

test('intoUuid orders microsecond instants before every digest bit', () => {
  const instants = [0n, 1_000n, 15_000n, 16_000n, 65_535_000n, 65_536_000n, 1_000_000_000n, I64_MAX]
  const highest = Digest.from('xxh64:ffffffffffffffff')
  const lowest = Digest.from('xxh64:0000000000000000')
  for (let index = 1; index < instants.length; index += 1) {
    const earlier = TxHash.fromParts(instants[index - 1], highest, 'ns')
    const later = TxHash.fromParts(instants[index], lowest, 'ns')
    assert.equal(earlier.compare(later), -1, 'native ordering still compares the signed count')
    assert.ok(earlier.intoUuid().asJs() < later.intoUuid().asJs(), `${instants[index - 1]} < ${instants[index]}`)
  }
  // Within one microsecond the instant ties, and the digest orders.
  const low = TxHash.fromParts(1_000n, Digest.from('xxh64:0000000000000001'), 'ns')
  const high = TxHash.fromParts(1_999n, Digest.from('xxh64:0000000000000002'), 'ns')
  assert.ok(low.intoUuid().asJs() < high.intoUuid().asJs())
})

test('intoUuid normalizes units and keeps the restatement overflow', () => {
  const digest = Digest.from('xxh3-64:0000000000000007')
  for (const seconds of [0n, 2n]) {
    const expected = TxHash.fromParts(seconds, digest, 's').intoUuid()
    for (const [unit, scale] of [['s', 1n], ['ms', 1_000n], ['us', 1_000_000n], ['ns', 1_000_000_000n]]) {
      const projected = TxHash.fromParts(seconds * scale, digest, unit).intoUuid()
      assert.ok(projected.equals(expected), unit)
    }
  }
  for (const [unit, scale] of [['s', 1_000_000_000n], ['ms', 1_000_000n], ['us', 1_000n]]) {
    // BigInt division truncates toward zero, as Rust's does.
    assert.equal(TxHash.fromParts(I64_MAX / scale, digest, unit).intoUuid().dtype.id, 'uuid')
    // A count below the epoch restates, and is then refused as a UUIDv7.
    assert.throws(() => TxHash.fromParts(I64_MIN / scale, digest, unit).intoUuid(), /UUIDv7/)
    for (const count of [I64_MIN / scale - 1n, I64_MAX / scale + 1n]) {
      let expected
      assert.throws(() => txhash.restateUnix(count, unit, 'ns'), (error) => {
        expected = error.message
        return true
      })
      assert.equal(expected, 'unix restatement overflows int64')
      assert.throws(() => TxHash.fromParts(count, digest, unit).intoUuid(), { message: expected })
    }
  }
})

test('intoUuid discards only the high two digest bits and the algorithm', () => {
  const project = (algorithm, payload) =>
    TxHash.fromParts(1n, Digest.from(`${algorithm}:${hex(payload, 16)}`), 'ns').intoUuid()
  const payload = 0x0123_4567_89ab_cdefn
  const expected = project('xxh64', payload)
  assert.ok(project('xxh3-64', payload).equals(expected))
  for (let bit = 62n; bit < 64n; bit += 1n) {
    assert.ok(project('xxh64', payload ^ (1n << bit)).equals(expected), String(bit))
  }
  for (let bit = 0n; bit < 62n; bit += 1n) {
    assert.ok(!project('xxh64', payload ^ (1n << bit)).equals(expected), String(bit))
  }
})

test('intoUuid refuses non-64-bit digests without narrowing', () => {
  for (const [algorithm, digits, bits] of [['xxh32', 8, 32], ['xxh3-128', 32, 128]]) {
    for (const payload of [0n, 7n, (1n << BigInt(bits)) - 1n]) {
      const value = TxHash.fromParts(0n, Digest.from(`${algorithm}:${hex(payload, digits)}`), 'ns')
      assert.throws(() => value.intoUuid(), {
        message: `invalid record value at $.digest: expected a 64-bit digest for UUIDv7, got ${algorithm} (${bits} bits)`,
      })
    }
  }
})

test('every instant spelling reads the same way', () => {
  const digest = xxhash.digest(Buffer.from('AAPL'), 'xxh3-64')
  const expected = txhash.txh3('AAPL', INSTANT)
  assert.ok(TxHash.fromParts(INSTANT, digest).equals(expected))
  assert.ok(TxHash.fromParts(Number(INSTANT / 1_000_000n) * 1_000_000, digest).equals(expected))
  const date = new Date('2023-11-14T22:13:20Z')
  assert.ok(TxHash.fromParts(date, digest).equals(expected), 'a Date is its UTC instant')
  assert.ok(TxHash.fromParts('2023-11-14T22:13:20Z', digest).equals(expected))
  assert.ok(TxHash.fromParts('2023-11-14T22:13:20', digest).equals(expected), 'naive reads as UTC')
  assert.ok(TxHash.fromParts('2023-11-15T03:43:20+05:30', digest).equals(expected), 'a zone moves nothing')
  assert.ok(TxHash.fromParts(Scalar.from(date), digest).equals(expected))
  const seconds = TxHash.fromParts(date, digest, 's')
  assert.equal(seconds.unix, 1_700_000_000n)
  assert.equal(seconds.unit, 's')
  assert.ok(seconds.equals(expected.withUnit('s')))
  assert.equal(expected.withUnit('ns').unix, INSTANT * 1000n)
  assert.ok(expected.intoDatetime().equals(new DataType('datetime64(us,"UTC")').scalar(INSTANT)))
  assert.deepEqual(Buffer.from(expected.intoScalar().asBytes()), Buffer.from(expected.bytes()))
  assert.throws(() => TxHash.fromParts(true, digest), TypeError)
  assert.throws(() => TxHash.fromParts(1.5, digest), /safe integer/)
  assert.throws(() => TxHash.fromParts('yesterday', digest))
  assert.throws(() => TxHash.fromParts(INSTANT, digest, 'd'), /unix unit/)

  assert.equal(txhash.unixOf(date), INSTANT)
  assert.equal(txhash.unixOf(date, 's'), 1_700_000_000n)
  assert.equal(txhash.unixOf(1_700_000_000, 's'), 1_700_000_000n)
  assert.equal(txhash.restateUnix(1_999n, 'ns', 'us'), 1n)
  assert.equal(txhash.restateUnix(-1, 'ns', 'us'), -1n)
  assert.equal(txhash.restateUnix(1, 'd', 's'), 86_400n)
  assert.throws(() => txhash.restateUnix(1, 's', 'd'))
  const now = txhash.unixNow()
  assert.ok(now > INSTANT)
  assert.ok(now / 1_000_000n >= txhash.unixNow('s') - 1n)
  assert.throws(() => txhash.unixNow('d'))
})

test('a hasher carries unit, seed, and a secret through a state', () => {
  const plain = new TxHasher()
  assert.equal(plain.unit, 'us')
  assert.equal(plain.algorithm, 'xxh3-64')
  assert.equal(plain.width, 16)
  assert.ok(plain.dtype.equals(DataType.from('fixed_size_binary[16]')))
  assert.ok(plain.digest(PAYLOAD, INSTANT).equals(txhash.txh3(PAYLOAD, INSTANT)))

  const seconds = new TxHasher('xxh64', 's', 7n)
  const date = new Date('2023-11-14T22:13:20Z')
  const value = seconds.digest(PAYLOAD, date)
  assert.equal(value.unit, 's')
  assert.equal(value.unix, 1_700_000_000n)
  assert.equal(value.digest.value(), xxhash.xxh64(PAYLOAD, { seed: 7n }))
  assert.equal(seconds.unixOf(date), 1_700_000_000n)
  const row = seconds.digestScalar(Scalar.from(['AAPL', 100]), 1_700_000_000n)
  const state = new xxhash.Xxh64(7n)
  state.writeScalar(Scalar.from(['AAPL', 100]))
  assert.ok(row.digest.equals(state.asDigest()))
  assert.equal(seconds.clone().unit, 's')
  assert.throws(() => new TxHasher('xxh3-64', 'd'), /unix unit/)
  assert.throws(() => new TxHasher('md5'))

  const secret = Buffer.alloc(xxhash.SECRET_MINIMUM_LENGTH, 0x5a)
  const secretive = new xxhash.Xxh3(3n, secret)
  secretive.writeBytes('already fed and forgotten')
  const hasher = TxHasher.fromState(secretive, 'ms')
  const long = Buffer.alloc(241, 0x11)
  assert.equal(hasher.digest(long, 5n).digest.value(), xxhash.xxh3(long, { seed: 3n, secret }))
  assert.equal(hasher.unit, 'ms')
  assert.equal(TxHasher.fromState(new xxhash.Xxh32(9)).digest(PAYLOAD, 1n).digest.value(), xxhash.xxh32(PAYLOAD, { seed: 9 }))
  assert.equal(TxHasher.fromState(new xxhash.Xxh128()).width, 24)
})

test('a coupled holder is filled with the instant in front', () => {
  const event = new Field('event', 'int64', false)
  const symbol = new Field('symbol', 'utf8', false)
  const holder = new Field('key', 'fixed_size_binary[16]', false)
  holder.digest.set('role', 'holder')
  holder.digest.set('time', 'event')
  holder.digest.set('unit', 'seconds')
  assert.equal(holder.digest.get('unit'), 's', 'the unit canonicalizes on write')
  assert.throws(() => holder.digest.set('unit', 'day'))
  assert.throws(() => holder.digest.set('time', ''))
  const root = new Field('row', DataType.fromFields([event, symbol, holder]), false)
  const source = new arrow.Table({
    event: arrow.vectorFromArray([1_700_000_000n, 1_700_000_001n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
  }).batches[0]

  const filled = new TxHasher('xxh3-64', 'us', 7n).applyArrowBatch(root, source)
  assert.deepEqual(filled.schema.fields.map((field) => field.name), ['event', 'symbol', 'key'])
  const cell = TxHash.fromBytes('s', 'xxh3-64', filled.getChild('key').get(1))
  assert.equal(cell.unix, 1_700_000_001n)
  const expected = new xxhash.Xxh3(7n)
  expected.writeScalar(Scalar.from([1_700_000_001n, 'MSFT']))
  assert.ok(cell.digest.equals(expected.asDigest()))

  const again = new TxHasher('xxh3-64', 'us', 7n).applyArrowBatch(root, filled)
  assert.deepEqual(
    Buffer.from(arrow.tableToIPC(new arrow.Table(filled), 'stream')),
    Buffer.from(arrow.tableToIPC(new arrow.Table(again), 'stream')),
    'filling again changes nothing',
  )
  assert.equal('_applyArrowBatchIpcNative' in TxHasher.prototype, false)
  assert.equal('_digestNative' in TxHasher.prototype, false)
  assert.equal('_fromPartsNative' in TxHash, false)
  assert.throws(() => new TxHasher().applyArrowBatch(root, source, 'yes'), /boolean/)
})
