'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Writable } = require('node:stream')
const { ReadableStream, WritableStream } = require('node:stream/web')
const test = require('node:test')
const { pathToFileURL } = require('node:url')

const { DataType, Value, codec, toml } = require('yggdryl')

// A Map whose keys are not text has no plain TOML spelling, so each layer
// expands into a wrapper table, its envelope body, the entry array, and one
// pair - four wire levels for one JavaScript container. That gap is what the
// preflight below has to see.
function nestedMap(count) {
  let value = 0
  for (let index = 0; index < count; index += 1) {
    value = new Map([[1, value]])
  }
  return value
}

test('TOML is a byte-first single-document facade', () => {
  const value = toml.loads('title = "Yggdryl"\n[owner]\nname = "Ada"\n')
  assert.deepEqual(value, { title: 'Yggdryl', owner: { name: 'Ada' } })

  const encoded = toml.dumps(value)
  assert.ok(Buffer.isBuffer(encoded))
  assert.deepEqual(toml.loads(encoded), value)
  assert.ok(Object.isFrozen(toml))

  for (const name of [
    'loadsAll',
    'loadAll',
    'dumpAll',
    'loadAllStream',
    'dumpAllStream',
  ]) {
    assert.equal(toml[name], undefined)
  }
})

test('TOML lowers exotic JavaScript values through core envelopes', () => {
  const value = {
    bigint: 2n ** 100n,
    bytes: Buffer.from([0, 1, 127, 255]),
    infinity: Infinity,
    map: new Map([[{ key: true }, new Set(['XPAR', 'XNAS'])]]),
    nan: NaN,
    negativeZero: -0,
    regexp: new RegExp('a/b(?<name>c)', 'giu'),
    schema: DataType.fromString('struct<id:int64 not null>'),
    typed: new Uint16Array([0, 65535]),
    undefined,
  }

  const decoded = toml.loads(toml.dumps(value))
  assert.equal(decoded.bigint, value.bigint)
  assert.deepEqual(decoded.bytes, value.bytes)
  assert.equal(decoded.infinity, Infinity)
  assert.ok(Number.isNaN(decoded.nan))
  assert.ok(Object.is(decoded.negativeZero, -0))
  assert.equal(decoded.regexp, '/a\\/b(?<name>c)/giu')
  assert.ok(decoded.map instanceof Map)
  assert.deepEqual([...decoded.map.values()], [['XPAR', 'XNAS']])
  assert.equal(decoded.schema, value.schema.toString())
  assert.deepEqual(decoded.typed, [0, 65535])
  assert.ok(Object.hasOwn(decoded, 'undefined'))
  assert.equal(decoded.undefined, null)

  const collision = { $yggdryl: { version: 1, type: 'null' } }
  assert.deepEqual(toml.loads(toml.dumps(collision)), collision)

  for (const root of [null, 'scalar root', [1, 2], Buffer.from([1, 2])]) {
    assert.deepEqual(toml.loads(toml.dumps(root)), root)
  }
})

test('TOML writes a temporal in its own syntax and a decimal in an envelope', () => {
  const written = toml.dumps({
    at: new Date('2026-08-15T12:30:00.000Z'),
    on: Value.date(19723),
    since: Value.time(27120n, 's'),
    price: Value.decimal(-1050n, 2),
  })

  const text = written.toString('utf8')
  assert.match(text, /"at" = 2026-08-15T12:30:00/)
  assert.match(text, /"on" = 2024-01-01/)
  assert.match(text, /"since" = 07:32:00/)
  // TOML has no decimal of its own, so an exact one keeps its envelope.
  assert.match(text, /"price" = \{ "\$yggdryl" = \{ version = 1, type = "decimal"/)

  const decoded = toml.loads(written)
  assert.ok(decoded.at instanceof Date)
  assert.equal(decoded.at.toISOString(), '2026-08-15T12:30:00.000Z')
  assert.ok(decoded.on.equals(Value.date(19723)))
  assert.ok(decoded.since.equals(Value.time(27120n, 's')))
  assert.ok(decoded.price.equals(Value.decimal(-1050n, 2)))
})

test('TOML emission applies requested depth to the expanded wire value', () => {
  const defaultBoundary = nestedMap(12)
  const defaultBytes = toml.dumps(defaultBoundary)
  assert.deepEqual(toml.loads(defaultBytes), defaultBoundary)
  assert.deepEqual(
    codec.from(codec.into(defaultBoundary, { format: 'toml' }), {
      format: 'toml',
    }),
    defaultBoundary,
  )
  assert.throws(() => toml.dumps(nestedMap(13)), /depth/i)
  assert.throws(() => codec.into(nestedMap(13), { format: 'toml' }), /depth/i)
  assert.throws(() => toml.dumps(nestedMap(2), { maxDepth: 3 }), /depth/i)

  const customBoundary = nestedMap(3)
  const customBytes = toml.dumps(customBoundary, { maxDepth: 12 })
  assert.deepEqual(toml.loads(customBytes, { maxDepth: 12 }), customBoundary)
  assert.throws(() => toml.dumps(customBoundary, { maxDepth: 11 }), /depth/i)
  assert.throws(() => toml.dumps(nestedMap(4), { maxDepth: 12 }), /depth/i)
})

test('native TOML date-times arrive as temporal values and write back exactly', () => {
  const source =
    'offset = 1979-05-27T07:32:00Z\nlocal = 1979-05-27T07:32:00\ndate = 1979-05-27\ntime = 07:32:00\n'
  const decoded = toml.loads(source)

  // A local date-time is exactly what a Date holds - a naive count of whole
  // milliseconds - so it arrives as one. The other three have no JavaScript
  // spelling and stay native values rather than being rounded into a Date.
  assert.ok(decoded.local instanceof Date)
  assert.equal(decoded.local.toISOString(), '1979-05-27T07:32:00.000Z')
  assert.ok(decoded.offset.equals(Value.timestamp(296638320n, 's', 'UTC')))
  assert.ok(decoded.date.equals(Value.date(3433)))
  assert.ok(decoded.time.equals(Value.time(27120n, 's')))

  assert.equal(
    toml.dumps(decoded).toString('utf8'),
    '"date" = 1979-05-27\n"local" = 1979-05-27T07:32:00\n"offset" = 1979-05-27T07:32:00Z\n"time" = 07:32:00\n',
  )
})

test('a class instance crosses TOML as its own properties', () => {
  let constructorCalls = 0
  class Order {
    static yggdrylType = 'orders.Order'

    constructor(id) {
      constructorCalls += 1
      this.id = id
    }
  }

  const encoded = toml.dumps(new Order(42))
  constructorCalls = 0
  assert.doesNotMatch(encoded.toString('utf8'), /orders\.Order/)
  assert.deepEqual(toml.loads(encoded), { id: 42 })
  assert.equal(constructorCalls, 0)
})

test('TOML content accepts strings and exact byte views', () => {
  assert.deepEqual(toml.loads('id = 42\n'), { id: 42 })
  assert.deepEqual(toml.load('id = 43\n'), { id: 43 })

  const framed = Buffer.from('xxid = 44\nyy')
  const view = new DataView(framed.buffer, framed.byteOffset + 2, 8)
  assert.deepEqual(toml.loads(view), { id: 44 })

  const shared = new SharedArrayBuffer(8)
  new Uint8Array(shared).set(Buffer.from('id = 45\n'))
  assert.deepEqual(toml.loads(shared), { id: 45 })
})

test('generic content inference delegates ambiguous syntax to the core', () => {
  assert.deepEqual(codec.from('{"id":1}'), { id: 1 })
  assert.deepEqual(codec.from('[1,2]'), [1, 2])
  assert.deepEqual(codec.from('id: 2\n'), { id: 2 })
  assert.deepEqual(codec.from('id = 3\n'), { id: 3 })
  assert.equal(codec.from('missing.toml'), 'missing.toml')
  assert.throws(() => codec.from(''), /yaml/i)
  assert.throws(() => codec.from('# shared comment syntax\n'), /yaml/i)

  assert.deepEqual(codec.from('id = 4\n', { format: 'toml' }), { id: 4 })
  assert.deepEqual(codec.from('', { format: 'toml' }), {})
  assert.deepEqual(codec.from('# TOML comment\n', { format: 'toml' }), {})
})

test('TOML paths and generic .toml inference use native readers and writers', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-path-'))
  const source = path.join(directory, 'source.toml')
  const destination = path.join(directory, 'destination.toml')
  const misleading = path.join(directory, 'explicit.json')
  fs.writeFileSync(source, 'id = 46\n')

  const readSync = fs.readSync
  const writeFileSync = fs.writeFileSync
  try {
    fs.readSync = () => {
      throw new Error('real TOML paths must bypass JavaScript read staging')
    }
    assert.deepEqual(toml.load(source), { id: 46 })
    assert.deepEqual(codec.from(source), { id: 46 })

    fs.writeFileSync = () => {
      throw new Error('real TOML paths must bypass JavaScript write staging')
    }
    toml.dump({ id: 47 }, destination)
    codec.into({ id: 48 }, misleading, { format: 'toml' })
  } finally {
    fs.readSync = readSync
    fs.writeFileSync = writeFileSync
  }

  try {
    assert.deepEqual(toml.load(pathToFileURL(destination)), { id: 47 })
    assert.deepEqual(codec.from(misleading, { format: 'toml' }), { id: 48 })
    assert.deepEqual(
      codec.from(Buffer.from('id = 49\n'), { format: 'toml' }),
      { id: 49 },
    )
    assert.deepEqual(
      toml.loads(codec.into({ id: 50 }, { format: 'toml' })),
      { id: 50 },
    )
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('TOML file descriptors stay caller-owned', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-fd-'))
  const source = path.join(directory, 'source.toml')
  const destination = path.join(directory, 'destination.toml')
  fs.writeFileSync(source, 'id = 51\n')
  const sourceFd = fs.openSync(source, 'r')
  const destinationFd = fs.openSync(destination, 'w+')
  try {
    assert.deepEqual(toml.load(sourceFd), { id: 51 })
    assert.ok(fs.fstatSync(sourceFd).isFile())
    toml.dump({ id: 52 }, destinationFd)
    assert.ok(fs.fstatSync(destinationFd).isFile())
  } finally {
    fs.closeSync(sourceFd)
    fs.closeSync(destinationFd)
  }
  try {
    assert.deepEqual(toml.load(destination), { id: 52 })
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('TOML async readers preserve split strings and bounded single-document semantics', async () => {
  async function* splitUnicode() {
    yield 'label = "\ud83d'
    yield '\ude42"\n[nested]\n'
    yield 'value = 53\n'
  }

  const decoded = await toml.load(splitUnicode())
  assert.deepEqual(decoded, {
    label: '\u{1f642}',
    nested: { value: 53 },
  })
  assert.deepEqual(
    [...decoded.label].map((character) => character.codePointAt(0)),
    [0x1f642],
  )
  let pulls = 0
  async function* inferredToml() {
    pulls += 1
    yield 'id = '
    pulls += 1
    yield '54\n'
  }
  assert.deepEqual(await codec.from(inferredToml()), { id: 54 })
  assert.equal(pulls, 2)

  const webInput = new ReadableStream({
    start(controller) {
      controller.enqueue(Buffer.from('id = 55\n'))
      controller.close()
    },
  })
  assert.deepEqual(await toml.load(webInput), { id: 55 })
})

test('TOML Node and WHATWG writers honor errors, backpressure, and no-close ownership', async () => {
  const chunks = []
  const nodeOutput = new Writable({
    write(chunk, _encoding, done) {
      setImmediate(() => {
        chunks.push(Buffer.from(chunk))
        done()
      })
    },
  })
  await toml.dump({ id: 56 }, nodeOutput)
  assert.deepEqual(toml.loads(Buffer.concat(chunks)), { id: 56 })
  assert.equal(nodeOutput.writableEnded, false)

  const webChunks = []
  let webClosed = false
  const webOutput = new WritableStream({
    write(chunk) {
      webChunks.push(Buffer.from(chunk))
    },
    close() {
      webClosed = true
    },
  })
  await codec.into({ id: 57 }, webOutput, { format: 'toml' })
  assert.deepEqual(toml.loads(Buffer.concat(webChunks)), { id: 57 })
  assert.equal(webClosed, false)

  const expected = new Error('TOML sink rejected bytes')
  const failing = new Writable({
    write(_chunk, _encoding, done) {
      done(expected)
    },
  })
  await assert.rejects(toml.dump({ id: 58 }, failing), (error) => error === expected)
})

test('TOML errors and limits stay native and path conversion is non-destructive', () => {
  assert.throws(() => toml.loads('id = 1\nid = 2\n'), /toml/i)

  let nested = { leaf: true }
  for (let index = 0; index < 24; index += 1) nested = { nested }
  const nestedBytes = toml.dumps(nested)
  assert.deepEqual(toml.loads(nestedBytes), nested)
  assert.throws(() => toml.loads(nestedBytes, { maxDepth: 8 }), /depth/i)

  let deep = { value: 1 }
  for (let index = 0; index < 49; index += 1) deep = { nested: deep }
  assert.throws(() => toml.dumps(deep), /maxDepth 48/)

  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-safe-'))
  const destination = path.join(directory, 'existing.toml')
  fs.writeFileSync(destination, 'keep = "me"\n')
  const cyclic = {}
  cyclic.self = cyclic
  try {
    assert.throws(() => toml.dump(cyclic, destination), /cyclic/)
    assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
    assert.throws(() => toml.dump(nestedMap(13), destination), /depth/i)
    assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
    assert.throws(
      () =>
        codec.into(nestedMap(4), destination, {
          format: 'toml',
          maxDepth: 12,
        }),
      /depth/i,
    )
    assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})
