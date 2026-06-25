// Tests for Compression — naming, parsing and round-trips.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const { Compression, MimeType, BytesIO, LocalPath } = require('..')

let counter = 0

test('parses names and extensions', () => {
  assert.strictEqual(new Compression('gzip').name, 'gzip')
  assert.strictEqual(Compression.fromStr('GZ').name, 'gzip')
  assert.strictEqual(new Compression('zst').name, 'zstd')
  assert.strictEqual(new Compression(' snappy ').name, 'snappy')
  assert.strictEqual(new Compression('store').name, 'none')

  assert.strictEqual(new Compression('gzip').extension, 'gz')
  assert.strictEqual(new Compression('none').extension, null)
  assert.strictEqual(Compression.fromExtension('.zst').name, 'zstd')
  assert.strictEqual(Compression.fromExtension('txt'), null)

  assert.throws(() => new Compression('lzo'))
})

test('none is identity', () => {
  const codec = new Compression('none')
  assert.strictEqual(codec.isAvailable, true)
  const payload = Buffer.from('the quick brown fox')
  assert.deepStrictEqual(codec.compress(payload), payload)
  assert.deepStrictEqual(codec.decompress(payload), payload)
})

for (const name of ['gzip', 'zstd', 'snappy']) {
  test(`round-trips ${name}`, () => {
    const codec = new Compression(name)
    assert.strictEqual(codec.isAvailable, true) // the addon enables all three backends
    const payload = Buffer.from(Array.from({ length: 4096 }, (_, i) => i % 251))
    const packed = codec.compress(payload)
    assert.deepStrictEqual(codec.decompress(packed), payload)
  })
}

test('toString', () => {
  assert.strictEqual(new Compression('zstd').toString(), 'zstd')
})

test('fromMime', () => {
  assert.strictEqual(Compression.fromMime(new MimeType('application/gzip')).name, 'gzip')
  assert.strictEqual(Compression.fromMime(new MimeType('application/json')), null)
})

test('fromMedia', () => {
  const { MediaType } = require('..')
  assert.strictEqual(Compression.fromMedia(MediaType.fromStr('csv.gz')).name, 'gzip')
  assert.strictEqual(Compression.fromMedia(MediaType.fromStr('csv')), null)
})

test('fromStats', () => {
  const p = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_stats.csv.gz`)
  new LocalPath(p).write(Buffer.from('col\n1\n'))
  assert.strictEqual(Compression.fromStats(new LocalPath(p).stats()).name, 'gzip')
})

for (const kind of ['bytesio', 'localpath']) {
  const make = (data) => {
    if (kind === 'bytesio') return new BytesIO(Buffer.from(data))
    const p = path.join(os.tmpdir(), `yggdryl_comp_${process.pid}_${counter++}.bin`)
    new LocalPath(p).write(Buffer.from(data))
    return new LocalPath(p)
  }

  test(`io compress/decompress (${kind})`, () => {
    const payload = Buffer.from(Array.from({ length: 2048 }, (_, i) => i % 251))
    const packed = make(payload).compress('zstd')
    assert.ok(packed instanceof BytesIO)
    assert.deepStrictEqual(packed.decompress('zstd').getValue(), payload)
  })
}

test('io decompress infers codec from extension', () => {
  const payload = Buffer.from('inferred from the .gz extension')
  const packed = new BytesIO(payload).compress('gzip').getValue()
  const p = path.join(os.tmpdir(), `yggdryl_comp_${process.pid}_${counter++}.txt.gz`)
  new LocalPath(p).write(packed)
  assert.deepStrictEqual(new LocalPath(p).decompress().getValue(), payload)
})

test('io decompress infers codec from magic bytes', () => {
  // An in-memory buffer has no extension, so the codec is sniffed from magic.
  const packed = new BytesIO(Buffer.from('sniffed from magic')).compress('gzip').getValue()
  assert.deepStrictEqual(
    new BytesIO(packed).decompress().getValue(),
    Buffer.from('sniffed from magic'),
  )
})
