'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')
const arrow = require('apache-arrow')

const { DataType, Field, Scalar, TxHash, TxHasher, txhash, xxhash } = require('yggdryl')

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
  assert.ok(TxHash.fromParts(Scalar.fromJs(date), digest).equals(expected))
  const seconds = TxHash.fromParts(date, digest, 's')
  assert.equal(seconds.unix, 1_700_000_000n)
  assert.equal(seconds.unit, 's')
  assert.ok(seconds.equals(expected.withUnit('s')))
  assert.equal(expected.withUnit('ns').unix, INSTANT * 1000n)
  assert.ok(expected.intoDatetime().equals(Scalar.datetime(INSTANT, 'us', 'UTC')))
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
  const row = seconds.digestScalar(Scalar.fromJs(['AAPL', 100]), 1_700_000_000n)
  const state = new xxhash.Xxh64(7n)
  state.writeScalar(Scalar.fromJs(['AAPL', 100]))
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
  expected.writeScalar(Scalar.fromJs([1_700_000_001n, 'MSFT']))
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
