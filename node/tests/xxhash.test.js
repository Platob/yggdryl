'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { test } = require('node:test')

const { IOBase, Scalar, xxhash } = require('yggdryl')

const PAYLOAD = Buffer.from('{"symbol": "AAPL", "price": 187.23}\n'.repeat(512))

/** One payload per XXH3 size branch, plus the boundaries between them. */
const BRANCHES = [0, 1, 3, 4, 8, 9, 16, 17, 64, 128, 129, 240, 241, 4096]

function corpus(length) {
  const bytes = Buffer.alloc(length)
  for (let index = 0; index < length; index += 1) bytes[index] = (index * 31 + 7) % 256
  return bytes
}

function secret(length) {
  const bytes = Buffer.alloc(length)
  for (let index = 0; index < length; index += 1) bytes[index] = (index * 17 + 3) % 256
  return bytes
}

test('published vectors pin every algorithm', () => {
  const empty = Buffer.alloc(0)
  assert.equal(xxhash.xxh32(empty), 0x02cc5d05)
  assert.equal(xxhash.xxh64(empty), 0xef46db3751d8e999n)
  assert.equal(xxhash.xxh3_64(empty), 0x2d06800538d394c2n)
  assert.equal(xxhash.xxh3_128(empty), 0x99aa06d3014798d86001c324468d497fn)

  const abc = Buffer.from('abc')
  assert.equal(xxhash.xxh32(abc), 0x32d153ff)
  assert.equal(xxhash.xxh64(abc), 0x44bc2cf5ad770999n)
  assert.equal(xxhash.xxh3_64(abc), 0x78af5f94892f3950n)
  assert.equal(xxhash.xxh3_128(abc), 0x06b05ab6733a618578af5f94892f3950n)
})

test('a 32-bit answer is a number and the wider ones are bigints', () => {
  assert.equal(typeof xxhash.xxh32(PAYLOAD), 'number')
  assert.equal(typeof xxhash.xxh64(PAYLOAD), 'bigint')
  assert.equal(typeof xxhash.xxh3_64(PAYLOAD), 'bigint')
  assert.equal(typeof xxhash.xxh3_128(PAYLOAD), 'bigint')
  // A 32-bit value always fits a JS number exactly, so nothing is lost.
  assert.ok(Number.isSafeInteger(xxhash.xxh32(PAYLOAD)))
})

test('every byte shape reads the same bytes', () => {
  const expected = xxhash.xxh3_64(PAYLOAD)
  assert.equal(xxhash.xxh3_64(new Uint8Array(PAYLOAD)), expected)
  assert.equal(xxhash.xxh3_64(PAYLOAD.buffer.slice(PAYLOAD.byteOffset, PAYLOAD.byteOffset + PAYLOAD.length)), expected)
  // A window into a larger buffer is the window's bytes, not the whole.
  const window = new Uint8Array(PAYLOAD.buffer, PAYLOAD.byteOffset + 10, 10)
  assert.equal(xxhash.xxh3_64(window), xxhash.xxh3_64(PAYLOAD.subarray(10, 20)))
  // A string is its UTF-8.
  assert.equal(xxhash.xxh3_64('é—wide'), xxhash.xxh3_64(Buffer.from('é—wide', 'utf8')))
})

test('a seed changes every algorithm', () => {
  for (const length of BRANCHES) {
    const data = corpus(length)
    assert.notEqual(xxhash.xxh32(data, { seed: 42 }), xxhash.xxh32(data))
    assert.notEqual(xxhash.xxh64(data, { seed: 42n }), xxhash.xxh64(data))
    assert.notEqual(xxhash.xxh3_64(data, { seed: 42n }), xxhash.xxh3_64(data))
    assert.notEqual(xxhash.xxh3_128(data, { seed: 42n }), xxhash.xxh3_128(data))
  }
  // A number seed is accepted wherever it is exact.
  assert.equal(xxhash.xxh64(PAYLOAD, { seed: 42 }), xxhash.xxh64(PAYLOAD, { seed: 42n }))
  assert.throws(() => xxhash.xxh64(PAYLOAD, { seed: -1 }), TypeError)
})

test('a custom secret changes the XXH3 pair and is validated by length', () => {
  const custom = secret(xxhash.SECRET_MINIMUM_LENGTH)
  assert.notEqual(xxhash.xxh3_64(PAYLOAD, { secret: custom }), xxhash.xxh3_64(PAYLOAD))
  assert.notEqual(xxhash.xxh3_128(PAYLOAD, { secret: custom }), xxhash.xxh3_128(PAYLOAD))

  // XXH3's own rule for the seed-and-secret family: at or below 240 bytes the
  // derived secret and the seed decide, which is what keeps the one-shot and
  // the streaming state answering one value.
  for (const length of [0, 1, 64, 240]) {
    const short = corpus(length)
    assert.equal(xxhash.xxh3_64(short, { secret: custom }), xxhash.xxh3_64(short))
  }
  for (const length of [241, 1024]) {
    const long = corpus(length)
    assert.notEqual(xxhash.xxh3_64(long, { secret: custom }), xxhash.xxh3_64(long))
  }

  const short = secret(xxhash.SECRET_MINIMUM_LENGTH - 1)
  assert.throws(() => xxhash.xxh3_64(PAYLOAD, { secret: short }), /at least 136 bytes, got 135/)
  assert.throws(() => new xxhash.Xxh3_128(0n, short), /at least 136 bytes, got 135/)
})

test('streaming agrees with the one-shot at every split', () => {
  const cases = [
    [() => new xxhash.Xxh32(), (data) => BigInt(xxhash.xxh32(data))],
    [() => new xxhash.Xxh64(), (data) => xxhash.xxh64(data)],
    [() => new xxhash.Xxh3_64(), (data) => xxhash.xxh3_64(data)],
    [() => new xxhash.Xxh3_128(), (data) => xxhash.xxh3_128(data)],
  ]
  for (const [build, oneShot] of cases) {
    for (const split of [1, 7, 64, 240, 1024, PAYLOAD.length]) {
      const state = build()
      for (let index = 0; index < PAYLOAD.length; index += split) {
        state.writeBytes(PAYLOAD.subarray(index, index + split))
      }
      const digest = state.asDigest()
      const value = digest.value()
      assert.equal(typeof value === 'number' ? BigInt(value) : value, oneShot(PAYLOAD))
      // Answering does not consume the state.
      assert.ok(digest.equals(state.asDigest()))
    }
  }
})

test('clear returns to the constructed seed', () => {
  const state = new xxhash.Xxh64(11n)
  assert.equal(state.seed, 11n)
  assert.equal(state.algorithm, 'xxh64')
  state.writeBytes(PAYLOAD)
  state.clear()
  assert.equal(state.asDigest().value(), xxhash.xxh64(Buffer.alloc(0), { seed: 11n }))
  assert.notEqual(state.asDigest().value(), xxhash.xxh64(Buffer.alloc(0)))
})

test('a state clone carries everything fed so far', () => {
  const state = new xxhash.Xxh3_64()
  state.writeBytes(Buffer.from('AAPL'))
  const copy = state.clone()
  copy.writeBytes(Buffer.from(',187.23'))
  assert.equal(state.asDigest().value(), xxhash.xxh3_64(Buffer.from('AAPL')))
  assert.equal(copy.asDigest().value(), xxhash.xxh3_64(Buffer.from('AAPL,187.23')))
})

test('a digest carries its algorithm and round trips', () => {
  for (const algorithm of ['xxh32', 'xxh64', 'xxh3-64', 'xxh3-128']) {
    const digest = xxhash.digest(PAYLOAD, algorithm)
    assert.equal(digest.algorithm, algorithm)
    assert.ok(digest.toString().startsWith(`${algorithm}:`))
    assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
    assert.ok(xxhash.Digest.fromBytes(algorithm, digest.bytes()).equals(digest))
    assert.equal(digest.bytes().length, digest.width)
    assert.equal(digest.bits, digest.width * 8)
    assert.equal(JSON.stringify(digest), JSON.stringify(digest.toString()))
    assert.ok(digest.clone().equals(digest))
    assert.equal(typeof digest.stableHash(), 'bigint')
  }
})

test('two algorithms never compare equal', () => {
  const left = xxhash.digest(PAYLOAD, 'xxh64')
  const right = xxhash.digest(PAYLOAD, 'xxh3-64')
  assert.ok(!left.equals(right))
  assert.equal(left.compare(right), -1)
  assert.equal(left.compare(left.clone()), 0)
})

test('an unknown algorithm names the accepted vocabulary', () => {
  assert.throws(() => xxhash.digest(PAYLOAD, 'xxh3'), /xxh3-64/)
})

test('a value digests its canonical bytes', () => {
  const value = Scalar.fromJs('AAPL')
  assert.ok(value.digest().equals(value.digest('xxh3-64')))
  assert.equal(value.digest().value(), value.stableHash())
  // Equal values answer equal digests across widths.
  assert.ok(Scalar.decimal(100n, 2).digest().equals(Scalar.decimal(1n, 0).digest()))
  // And values that differ stay apart across variant boundaries.
  assert.ok(!Scalar.fromJs('1').digest().equals(Scalar.fromJs(Buffer.from('1')).digest()))
})

test('a state feeds a value like the value digests itself', () => {
  const value = Scalar.fromJs({ symbol: 'AAPL', quantity: 100n })
  const state = new xxhash.Xxh3_64()
  state.writeScalar(value)
  assert.ok(state.asDigest().equals(value.digest('xxh3-64')))
})

test('a handle digests its bytes', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-xxhash-'))
  try {
    const file = path.join(root, 'trades.csv')
    fs.writeFileSync(file, PAYLOAD)
    const handle = new IOBase(file)
    for (const algorithm of ['xxh32', 'xxh64', 'xxh3-64', 'xxh3-128']) {
      assert.ok(handle.readDigest(algorithm).equals(xxhash.digest(PAYLOAD, algorithm)))
    }
    assert.ok(handle.readDigest().equals(handle.readDigest('xxh3-64')))
    assert.ok(
      handle.readRangeDigest(0, 16).equals(xxhash.digest(PAYLOAD.subarray(0, 16), 'xxh3-64')),
    )

    // A resource that does not exist digests as empty, per the laziness
    // contract; a container throws instead.
    const missing = new IOBase(path.join(root, 'never-written.csv'))
    assert.ok(missing.readDigest().equals(xxhash.digest(Buffer.alloc(0), 'xxh3-64')))
    assert.throws(() => new IOBase(root).readDigest(), /directory/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})
