'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Readable } = require('node:stream')
const test = require('node:test')
const { pathToFileURL } = require('node:url')

const { Field, IOBase, Scalar, codec, json, toml, yaml } = require('yggdryl')

const f16 = new Field('value', 'float16', false)
const u64 = new Field('value', 'uint64', false)

test('codec loads can return the exact native Scalar', () => {
  for (const format of [json, yaml]) {
    const native = format.loads('1', { scalar: true })
    assert.ok(native instanceof Scalar)
    assert.equal(native.kind, 'u64')
    assert.equal(format.loads('1'), 1)
    assert.equal(format.loads('1', { scalar: false }), 1)
    assert.equal(format.loads('1', { scalar: null }), 1)
  }
})

test('typed TOML keeps core struct canonicalization on the Scalar path', () => {
  const field = new Field(
    'row',
    'struct<value: float16 not null>',
    false,
  )

  const native = toml.loads('value = 1.5', { field, scalar: true })
  assert.ok(native instanceof Scalar)
  assert.equal(native.kind, 'sequence')
  assert.deepEqual(toml.loads('value = 1.5', { field }), { value: 1.5 })
})

test('collection, inferred, and streamed loads keep native Scalars', async () => {
  const eager = json.loadsAll('1\n2\n', { scalar: true })
  assert.deepEqual(eager.map((value) => value.kind), ['u64', 'u64'])
  assert.ok(eager.every((value) => value instanceof Scalar))

  const inferred = codec.from('1', { scalar: true })
  assert.ok(inferred instanceof Scalar)
  assert.equal(inferred.kind, 'u64')

  const streamed = []
  for await (const value of json.loadAll(Readable.from(['1\n', '2\n']), {
    scalar: true,
  })) {
    streamed.push(value)
  }
  assert.deepEqual(streamed.map((value) => value.kind), ['u64', 'u64'])
  assert.ok(streamed.every((value) => value instanceof Scalar))
})

test('single path and stream loads return native Scalars', async (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-values-'))
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  const file = path.join(directory, 'value.json')
  fs.writeFileSync(file, '1')

  const pathValue = json.load(pathToFileURL(file), { scalar: true })
  const streamValue = await json.load(Readable.from(['1']), { scalar: true })

  assert.ok(pathValue instanceof Scalar)
  assert.ok(streamValue instanceof Scalar)
  assert.equal(pathValue.kind, 'u64')
  assert.equal(streamValue.kind, 'u64')
})

test('IOBase.readScalar accepts an inferred field or explicit options', () => {
  const handle = IOBase.fromBytes(Buffer.from('1'))
  handle.mediaType = 'application/json'

  assert.equal(handle.readScalar(), 1)
  assert.equal(handle.readScalar(u64), 1)
  assert.equal(handle.readScalar({ field: u64 }), 1)
  assert.equal(handle.readScalar({}), 1)
  const native = handle.readScalar({ scalar: true })
  assert.ok(native instanceof Scalar)
  assert.equal(native.kind, 'u64')

  assert.throws(() => json.loads('1.5', { scalar: 'yes' }), /scalar must be a boolean/)
  assert.throws(() => handle.readScalar({ scalar: 'yes' }), /scalar must be a boolean/)
  assert.throws(() => json.loads('1.5', { value: true }), /unknown codec option value/)
  assert.throws(() => handle.readScalar({ value: true }), /unknown readScalar option value/)
})
