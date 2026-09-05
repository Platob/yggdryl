'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { AvroSchema, Scalar, avro } = require('yggdryl')

const SCHEMA = {
  type: 'record',
  name: 'trade',
  doc: 'one fill',
  fields: [
    { name: 'symbol', type: 'string', 'field-id': 1 },
    { name: 'qty', type: 'int', 'field-id': 2 },
  ],
}

test('AvroSchema accepts natural values, native Scalars, JSON text, and bytes', () => {
  const natural = new AvroSchema(SCHEMA)
  const native = AvroSchema.from(Scalar.fromJs(SCHEMA))
  const text = new avro.Schema('"long"')
  const bytes = AvroSchema.from(Buffer.from('"long"'))

  assert.equal(avro.Schema, AvroSchema)
  assert.ok(Object.isFrozen(avro))
  assert.equal(natural.kind, 'record')
  assert.equal(typeof natural.fingerprint, 'bigint')
  assert.equal(native.fingerprint, natural.fingerprint)
  assert.equal(text.fingerprint, bytes.fingerprint)
  assert.equal(
    natural.canonicalForm,
    '{"name":"trade","type":"record","fields":[' +
      '{"name":"symbol","type":"string"},' +
      '{"name":"qty","type":"int"}]}',
  )
  assert.equal(natural.fingerprint, 3214887102531460143n)
  assert.equal(natural.intoCanonicalForm(), natural.canonicalForm)
  assert.equal(natural.toString(), natural.canonicalForm)
  assert.ok(!natural.canonicalForm.includes('doc'))
  assert.equal(natural.intoJSON().fields[0]['field-id'], 1)
  assert.notEqual(AvroSchema.from(natural), natural)
  assert.equal(AvroSchema.from(natural).fingerprint, natural.fingerprint)
  const clone = natural.clone()
  assert.ok(clone.equals(natural))
  assert.equal(clone.compare(natural), 0)
  assert.equal(clone.stableHash(), natural.stableHash())
  // Fingerprints use Parsing Canonical Form, while value identity retains
  // annotations and extension attributes needed by resolution and round trip.
  const canonicalOnly = new AvroSchema(JSON.parse(natural.canonicalForm))
  assert.equal(canonicalOnly.fingerprint, natural.fingerprint)
  assert.ok(!canonicalOnly.equals(natural))
  assert.notEqual(canonicalOnly.compare(natural), 0)
  assert.ok(new AvroSchema('"int"').compare(new AvroSchema('"long"')) < 0)
})

test('Avro object containers round-trip writer schema, metadata, and rows', () => {
  const rows = function* () {
    yield { symbol: 'AAPL', qty: 100 }
    yield { symbol: 'MSFT', qty: 25 }
  }
  const metadata = new Map([
    ['source', 'node'],
    ['__proto__', 'ordinary metadata'],
  ])
  const encoded = avro.dumps(rows(), SCHEMA, metadata)
  const decoded = avro.loads(encoded)

  assert.ok(Buffer.isBuffer(encoded))
  assert.ok(decoded.schema instanceof AvroSchema)
  assert.equal(decoded.schema.fingerprint, new AvroSchema(SCHEMA).fingerprint)
  assert.deepEqual(decoded.rows, [
    { qty: 100, symbol: 'AAPL' },
    { qty: 25, symbol: 'MSFT' },
  ])
  assert.equal(decoded.metadata.source, 'node')
  assert.equal(decoded.metadata.__proto__, 'ordinary metadata')
  assert.ok(Object.hasOwn(decoded.metadata, '__proto__'))
})

test('a reader schema applies aliases, promotions, and defaults natively', () => {
  const reader = {
    type: 'record',
    name: 'trade',
    fields: [
      { name: 'quantity', aliases: ['qty'], type: 'long' },
      { name: 'note', type: 'string', default: 'none' },
    ],
  }
  const encoded = avro.dumps([{ symbol: 'AAPL', qty: 100 }], SCHEMA)

  assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [
    { note: 'none', quantity: 100 },
  ])
})

test('all Avro decode paths honor one validated native limits shape', () => {
  const encoded = avro.dumps(
    [
      { symbol: 'AAPL', qty: 100 },
      { symbol: 'MSFT', qty: 25 },
      { symbol: 'GOOG', qty: 5 },
    ],
    SCHEMA,
  )
  const single = avro.dumpsSingle({ symbol: 'AAPL', qty: 100 }, SCHEMA)
  const limitedContainer = avro.dumps([1, 2, 3], '"long"')
  const primitive = avro.loads(limitedContainer, { maxNodes: 3 })

  assert.equal(primitive.schema.kind, 'long')
  assert.deepEqual(primitive.rows, [1, 2, 3])
  assert.throws(
    () => AvroSchema.from(Buffer.from(JSON.stringify(SCHEMA)), { maxInputBytes: 4 }),
    /byte limit exceeded|at most 4 bytes/i,
  )
  assert.throws(() => new AvroSchema(SCHEMA, { maxDepth: 1 }), /depth|deep/i)
  assert.throws(() => avro.loads(encoded, { maxInputBytes: encoded.length - 1 }), /at most/i)
  assert.throws(() => avro.loads(limitedContainer, { maxNodes: 2 }), /at most 2 rows/i)
  assert.throws(
    () => avro.loadsSingle(single, SCHEMA, { maxInputBytes: single.length - 1 }),
    /at most/i,
  )
  assert.throws(() => avro.loads(encoded, { maxNodes: -1 }), /non-negative safe integer/i)
  assert.throws(() => avro.loads(encoded, { maxNodes: 1.5 }), /non-negative safe integer/i)
  assert.throws(() => avro.loads(encoded, { unknown: 1 }), /unknown Avro decode option/i)
  assert.throws(
    () => avro.loads(limitedContainer, {
      maxDepth: 1,
      readerSchema: { type: 'array', items: 'long' },
    }),
    /depth|deep/i,
  )
  assert.throws(
    () => AvroSchema.from(SCHEMA, { readerSchema: SCHEMA }),
    /only valid for Avro container and block decoding/i,
  )
  assert.throws(
    () => AvroSchema.from(SCHEMA, { readerSchema: null }),
    /only valid for Avro container and block decoding/i,
  )
})

test('Avro blocks stay compressed and lazy until each block is requested', () => {
  const encoded = avro.dumps(
    [
      { symbol: 'AAPL', qty: 100 },
      { symbol: 'MSFT', qty: 25 },
    ],
    SCHEMA,
    new Map([
      ['source', 'node'],
      ['__proto__', 'ordinary metadata'],
    ]),
  )
  const blocks = avro.blocks(encoded)

  assert.equal(blocks[Symbol.iterator](), blocks)
  assert.equal(blocks.schema.fingerprint, new AvroSchema(SCHEMA).fingerprint)
  assert.equal(blocks.get('source'), 'node')
  assert.equal(blocks.get('missing'), undefined)
  assert.equal(blocks.metadata.__proto__, 'ordinary metadata')
  assert.ok(Object.hasOwn(blocks.metadata, '__proto__'))

  const first = blocks.next()
  assert.equal(first.done, false)
  assert.equal(first.value.count, 2n)
  assert.ok(first.value.size > 0n)
  assert.deepEqual(first.value.rows(), [
    { qty: 100, symbol: 'AAPL' },
    { qty: 25, symbol: 'MSFT' },
  ])
  assert.deepEqual(blocks.next(), { value: undefined, done: true })
  assert.deepEqual(blocks.next(), { value: undefined, done: true })

  const empty = avro.blocks(avro.dumps([], SCHEMA))
  assert.deepEqual(empty.next(), { value: undefined, done: true })
  assert.deepEqual(empty.next(), { value: undefined, done: true })

  // The header opens successfully; the malformed closing marker is not read
  // until `next`, and the iterator fuses after that first error.
  const corrupt = Buffer.from(encoded)
  corrupt[corrupt.length - 1] ^= 0xff
  const delayed = avro.blocks(corrupt)
  assert.throws(() => delayed.next(), /synchronization marker/i)
  assert.deepEqual(delayed.next(), { value: undefined, done: true })
})

test('Avro block resolution compiles once and block limits apply on demand', () => {
  const reader = {
    type: 'record',
    name: 'trade',
    fields: [
      { name: 'quantity', aliases: ['qty'], type: 'long' },
      { name: 'note', type: 'string', default: 'none' },
    ],
  }
  const encoded = avro.dumps([{ symbol: 'AAPL', qty: 100 }], SCHEMA)
  const resolved = avro.blocks(encoded, { readerSchema: reader })
  assert.deepEqual(resolved.next().value.rows(), [
    { note: 'none', quantity: 100 },
  ])

  const limitedEncoded = avro.dumps([1, 2, 3], '"long"')
  const limited = avro.blocks(limitedEncoded, { maxNodes: 2 })
  assert.throws(() => limited.next(), /at most 2 rows/i)
  assert.deepEqual(limited.next(), { value: undefined, done: true })
})

test('single-object framing uses and verifies the schema fingerprint', () => {
  const schema = new AvroSchema(SCHEMA)
  const row = { symbol: 'AAPL', qty: 100 }
  const encoded = avro.dumpsSingle(row, schema)

  assert.deepEqual(encoded.subarray(0, 2), Buffer.from([0xc3, 0x01]))
  assert.deepEqual(avro.loadsSingle(encoded, schema), {
    qty: 100,
    symbol: 'AAPL',
  })
  assert.deepEqual(schema.fromSingleObject(schema.intoSingleObject(row)), {
    qty: 100,
    symbol: 'AAPL',
  })
  assert.throws(
    () => avro.loadsSingle(encoded, '"long"'),
    /fingerprint/i,
  )

  const decimalSchema = {
    type: 'bytes',
    logicalType: 'decimal',
    precision: 10,
    scale: 2,
  }
  const exact = avro.loadsSingle(
    avro.dumpsSingle(Scalar.decimal(18750n, 2), decimalSchema),
    decimalSchema,
  )
  assert.ok(exact instanceof Scalar)
  assert.equal(exact.kind, 'd64')
  assert.equal(exact.unscaled, 18750n)
  assert.equal(exact.scale, 2)
})

test('Avro invalid inputs retain native diagnostics and boundary types', () => {
  assert.throws(() => new AvroSchema('not json'), /Avro|JSON/i)
  assert.throws(() => avro.loads(Buffer.from('not an object container')), /Avro/i)
  assert.throws(() => avro.loads('not bytes'), /must be bytes/i)
  assert.throws(() => avro.loads(Buffer.alloc(0), SCHEMA), /decode options|unknown/i)
  assert.throws(() => avro.dumps('one row', SCHEMA), /non-string iterable/i)
  assert.throws(
    () => avro.dumps([], SCHEMA, { source: 1 }),
    /Avro metadata keys and values must be strings/i,
  )
})
