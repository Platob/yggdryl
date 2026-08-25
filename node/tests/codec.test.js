'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Readable, Writable } = require('node:stream')
const { WritableStream } = require('node:stream/web')
const test = require('node:test')
const { pathToFileURL } = require('node:url')
const { types: utilTypes } = require('node:util')
const vm = require('node:vm')
const arrow = require('apache-arrow')
const nativeBinding = require('../index.js')
const nativeJsonDumps = nativeBinding.jsonDumpsNative
const nativeYamlDumpAll = nativeBinding.yamlDumpAllNative
const rawNativeWrapperPrototypes = [
  nativeBinding.Scalar.prototype,
  nativeBinding.DataType.prototype,
  nativeBinding.Field.prototype,
  nativeBinding.Uri.prototype,
  nativeBinding.Url.prototype,
  nativeBinding.Urn.prototype,
]
const rawRegExpSourceGetter = Object.getOwnPropertyDescriptor(
  RegExp.prototype,
  'source',
).get
const rawRegExpFlagsGetter = Object.getOwnPropertyDescriptor(
  RegExp.prototype,
  'flags',
).get
const rawNativeIntrinsics = [
  utilTypes.isMap,
  utilTypes.isSet,
  utilTypes.isRegExp,
  (value) => Reflect.apply(rawRegExpSourceGetter, value, []),
  (value) => Reflect.apply(rawRegExpFlagsGetter, value, []),
]

const {
  DataType,
  Field,
  Uri,
  Url,
  Urn,
  Timezone,
  Scalar,
  codec,
  json,
  toml,
  yaml,
} = require('yggdryl')

test('YAML lowers extended JavaScript values to natural shapes', () => {
  const value = {
    bigint: 2n ** 100n,
    bytes: Buffer.from([0, 1, 127, 255]),
    date: new Date('2026-08-15T12:30:00.000Z'),
    infinity: Infinity,
    map: new Map([[{ key: true }, new Set(['XPAR', 'XNAS'])]]),
    nan: NaN,
    negativeZero: -0,
    regexp: new RegExp('a/b(?<name>c)', 'giu'),
    typed: new Uint16Array([0, 65535]),
    undefined,
  }

  for (const format of [yaml]) {
    const bytes = format.dumps(value)
    const decoded = format.loads(bytes)

    assert.ok(Buffer.isBuffer(bytes))
    // An exact integer, bytes, and every float stay themselves; an instant
    // is spelled as its classic ISO string, the loosely typed deal every
    // schemaless wire now makes.
    assert.equal(decoded.bigint, value.bigint)
    assert.deepEqual(decoded.bytes, value.bytes)
    assert.equal(decoded.date, '2026-08-15T12:30:00.000Z')
    assert.equal(decoded.infinity, Infinity)
    assert.ok(Number.isNaN(decoded.nan))
    assert.ok(Object.is(decoded.negativeZero, -0))

    // These are the documented losses: no wrapper carries the class back.
    assert.equal(decoded.regexp, '/a\\/b(?<name>c)/giu')
    assert.ok(decoded.map instanceof Map)
    assert.deepEqual([...decoded.map.keys()], [{ key: true }])
    assert.deepEqual([...decoded.map.values()], [['XPAR', 'XNAS']])
    assert.deepEqual(decoded.typed, [0, 65535])
    assert.ok(Object.hasOwn(decoded, 'undefined'))
    assert.equal(decoded.undefined, null)
  }
  assert.throws(() => json.dumps(value), /non-finite float/)
})

test('a bigint beyond the exact 128-bit range is refused, not rounded', () => {
  assert.equal(json.loads(json.dumps(2n ** 127n - 1n)), 2n ** 127n - 1n)
  assert.equal(json.loads(json.dumps(2n ** 128n - 1n)), 2n ** 128n - 1n)
  assert.throws(
    () => json.dumps(2n ** 128n),
    /exceeds the exact 128-bit integer range/,
  )
  // A bigint small enough to be an ordinary number comes back as one.
  assert.equal(json.loads(json.dumps(42n)), 42)
})

test('Map and Set carry explicit undefined entries as null', () => {
  const value = {
    map: new Map([[undefined, undefined]]),
    set: new Set([undefined]),
  }

  for (const format of [yaml]) {
    const decoded = format.loads(format.dumps(value))
    assert.equal(decoded.map.size, 1)
    assert.equal(decoded.map.get(null), null)
    assert.deepEqual([...decoded.set], [null])
  }
})

test('two JavaScript Map keys that are one native key are refused', () => {
  assert.throws(
    () => json.dumps(new Map([[1, 'number'], [1n, 'bigint']])),
    /duplicate/i,
  )
})

test('native Yggdryl wrappers cross as their canonical text', () => {
  const type = DataType.fromString('struct<id:int64 not null>')

  for (const format of [json, yaml]) {
    const decoded = format.loads(format.dumps({ type }))
    // The wrapper class does not travel; its canonical spelling does, and
    // `DataType.from` reads that spelling back.
    assert.equal(decoded.type, type.toString())
    assert.ok(DataType.from(decoded.type).equals(type))
  }
})

test('native wrapper encoding reads native state instead of replaceable methods', () => {
  const values = [
    DataType.fromString('int64'),
    new Field('id', 'int64', false),
    Uri.fromString('https://example.com/value'),
    Url.fromString('https://example.com/value'),
    Urn.fromString('urn:example:value'),
  ]

  for (const value of values) {
    const canonical = value.toString()
    Object.defineProperty(value, 'toString', {
      value() {
        throw new Error('replaceable JavaScript method must not run')
      },
    })
    assert.equal(json.loads(json.dumps(value)), canonical)
  }
})

test('a URL and a Date read their state from the prototype, not the instance', () => {
  const url = new URL('https://example.com/value?q=1')
  Object.defineProperty(url, 'toString', {
    value() {
      throw new Error('replaceable JavaScript method must not run')
    },
  })
  assert.equal(json.loads(json.dumps(url)), 'https://example.com/value?q=1')

  const date = new Date('2026-08-15T12:30:00.000Z')
  Object.defineProperty(date, 'getTime', {
    value: () => 0,
  })
  assert.equal(json.loads(json.dumps(date)), '2026-08-15T12:30:00.000Z')
  assert.throws(() => json.dumps(new Date(NaN)), /invalid Date/)
})

test('temporal values cross as classic ISO strings; a decimal stays typed', () => {
  const values = {
    at: Scalar.datetime64(1700000000000000n, 'us', 'UTC'),
    naive: Scalar.datetime64(1700000000123456n, 'us', 'NAIVE'),
    on: Scalar.date32(19723),
    sinceMidnight: Scalar.time64(45296000000n, 'us'),
    took: Scalar.duration32(90, 's'),
    price: Scalar.d128(-1050n, 2),
  }

  for (const format of [json, yaml]) {
    const decoded = format.loads(format.dumps(values))
    // The fraction width is the unit, so nothing about the reading is lost -
    // it is just spelled the way every other tool spells it.
    assert.equal(decoded.at, '2023-11-14T22:13:20.000000Z')
    assert.equal(decoded.naive, '2023-11-14T22:13:20.123456')
    assert.equal(decoded.on, '2024-01-01')
    assert.equal(decoded.sinceMidnight, '12:34:56.000000')
    assert.equal(decoded.took, 'PT90S')
    assert.equal(decoded.price, '-10.50')
  }

  assert.equal(
    json.dumps({ price: values.price }).toString(),
    '{"price":"-10.50"}',
  )
  // One instant in two resolutions is one value, and so is one number in two
  // spellings, because the core compares what a value names.
  assert.ok(Scalar.duration32(1, 's').equals(Scalar.duration64(1000n, 'ms')))
  assert.ok(Scalar.d128(150n, 2).equals(Scalar.d128(15n, 1)))
})

test('temporal constructors keep one unit and non-null timezone contract', () => {
  const naive = Timezone.from('naive')
  const values = [
    [Scalar.date32(7), 'date32', 7n, 'd'],
    [Scalar.date64(7n), 'date64', 7n, 'ms'],
    [Scalar.time32(7, 's'), 'time32', 7n, 's'],
    [Scalar.time64(7n, 'us'), 'time64', 7n, 'us'],
    [Scalar.datetime64(7n, 'ns'), 'datetime64', 7n, 'ns'],
    [Scalar.duration32(7, 'ms'), 'duration32', 7n, 'ms'],
    [Scalar.duration64(7n, 'us'), 'duration64', 7n, 'us'],
  ]

  for (const [value, kind, count, unit] of values) {
    assert.equal(value.kind, kind)
    assert.equal(value.count, count)
    assert.equal(value.unit, unit)
    assert.equal(value.zone, 'NAIVE')
  }

  assert.ok(Scalar.date32(7, 'd', naive).equals(Scalar.date32(7)))
  assert.ok(Scalar.date32(7, null, null).equals(Scalar.date32(7)))
  assert.ok(Scalar.date64(7n, 'ms', 'NAIVE').equals(Scalar.date64(7n)))
  assert.ok(Scalar.time32(7, 's', naive).equals(Scalar.time32(7, 's')))
  assert.ok(Scalar.time64(7n, 'us', 'naive').equals(Scalar.time64(7n, 'us')))
  assert.ok(
    Scalar.datetime64(7n, 'ns', Timezone.UTC).equals(
      Scalar.datetime64(7n, 'ns', 'UTC'),
    ),
  )
  assert.ok(Scalar.datetime64(7n, 'ns', null).equals(Scalar.datetime64(7n, 'ns')))
  assert.ok(
    Scalar.duration32(7, 'ms', naive).equals(Scalar.duration32(7, 'ms')),
  )
  assert.ok(
    Scalar.duration64(7n, 'us', 'NAIVE').equals(Scalar.duration64(7n, 'us')),
  )

  assert.throws(() => Scalar.date32(1, 'ms'), /date32 unit must be day/)
  assert.throws(() => Scalar.date64(1n, 's'), /date64 unit must be millisecond/)
  assert.throws(() => Scalar.time32(1, 'us'), /time32 unit/)
  assert.throws(() => Scalar.time64(1n, 'ms'), /time64 unit/)
  assert.throws(() => Scalar.time32(1, 's', 'UTC'), /timezone must be NAIVE/)
  assert.throws(() => Scalar.time64(1n, 'us', Timezone.UTC), /timezone must be NAIVE/)
  assert.throws(() => Scalar.duration32(1, 's', 'UTC'), /duration32 timezone must be NAIVE/)
  assert.throws(
    () => Scalar.duration64(1n, 'ns', Timezone.UTC),
    /duration64 timezone must be NAIVE/,
  )
})

test('Scalar keeps widths, exact coefficients, hashes, and natural accessors', () => {
  const wide = Scalar.d256(-(2n ** 200n), 7)
  assert.equal(Scalar.f16(1.5).kind, 'f16')
  assert.equal(Scalar.f32(1.5).kind, 'f32')
  assert.equal(Scalar.f64(1.5).kind, 'f64')
  assert.equal(wide.kind, 'd256')
  assert.equal(wide.unscaled, -(2n ** 200n))
  assert.equal(wide.scale, 7)
  assert.match(wide.dataType.toString(), /^decimal256/)
  assert.equal(typeof wide.stableHash(), 'bigint')

  const mode = Scalar.fromEnum('io_mode', 'append')
  assert.equal(mode.kind, 'enum')
  assert.equal(mode.enumKind, 'io_mode')
  assert.equal(mode.enumValue, 'append')
  assert.equal(mode.enumOrdinal, 1)
  assert.equal(mode.asJs(), 'append')
  assert.equal(mode.asUtf8(), 'append')
  assert.throws(() => Scalar.fromEnum('io_mode', 'missing'), /unknown/)

  assert.deepEqual(Scalar.fromJs(Buffer.from([0, 255])).asBytes(), Buffer.from([0, 255]))
  assert.equal(Scalar.fromJs('AAPL').asUtf8(), 'AAPL')
  assert.equal(Scalar.fromJs(1).asUtf8(), null)
  const record = Scalar.fromJs({ z: 2, a: 1 })
  assert.equal(record.asJsonUtf8(), '{"a":1,"z":2}')
  assert.deepEqual(record.asJsonBytes(), Buffer.from(record.asJsonUtf8()))
  assert.equal(record.toString(), record.asJsonUtf8())
  assert.deepEqual(record.toJSON(), { a: 1, z: 2 })
  const clone = record.clone()
  assert.notEqual(clone, record)
  assert.ok(clone.equals(record))
  assert.equal(clone.compare(record), 0)
  assert.equal(clone.stableHash(), record.stableHash())
  assert.ok(Scalar.fromJs(1).compare(Scalar.fromJs(2)) < 0)
  assert.equal(Scalar.d128(150n, 2).compare(Scalar.d256(15n, 1)), 0)
})

test('Scalar traversal and persistent updates stay entirely native', () => {
  const instant = Scalar.datetime64(1700000000123456789n, 'ns', 'Europe/Paris')
  const record = Scalar.fromJs({ z: 2, legs: [{ at: instant }] })

  assert.equal(record.length, 2)
  assert.equal(record.isEmpty(), false)
  assert.equal(Scalar.fromJs({}).isEmpty(), true)
  assert.equal(Scalar.fromJs(1).isEmpty(), false)
  assert.equal(record.get('missing'), null)
  assert.equal(record.has('legs'), true)
  assert.equal(record.has('missing'), false)

  const legs = record.get('legs')
  assert.ok(legs instanceof Scalar)
  assert.equal(legs.length, 1)
  assert.ok(legs.get(0).equals(legs.at(0)))
  assert.equal(legs.at(1), null)
  assert.throws(() => legs.at(-1), /non-negative/)

  const nested = record.path('legs.0.at')
  assert.ok(nested instanceof Scalar)
  assert.equal(nested.kind, 'datetime64')
  assert.equal(nested.count, 1700000000123456789n)
  assert.equal(nested.unit, 'ns')
  assert.equal(nested.zone, 'Europe/Paris')
  assert.equal(record.path('legs.9.at'), null)

  // Record iteration is deterministic field-name order and yields values.
  assert.deepEqual([...record].map((value) => value.kind), ['sequence', 'i64'])
  // Sequence iteration yields its exact children.
  assert.equal([...legs][0].path('at').count, instant.count)

  const changed = record.set('z', instant).set('a', 1)
  assert.equal(record.get('z').kind, 'i64')
  assert.equal(changed.get('z').kind, 'datetime64')
  assert.deepEqual([...changed].map((value) => value.kind), [
    'i64',
    'sequence',
    'datetime64',
  ])
  const removed = changed.remove('legs')
  assert.equal(removed.has('legs'), false)
  assert.equal(changed.has('legs'), true)
  assert.ok(removed.remove('missing').equals(removed))

  const decimalKey = Scalar.d128(150n, 2)
  const mapping = Scalar.fromJs(new Map([[decimalKey, instant]]))
  assert.equal(mapping.length, 1)
  assert.equal(mapping.get(Scalar.d128(15n, 1)).count, instant.count)
  assert.equal([...mapping][0].kind, 'd128')
  const added = mapping.set('venue', 'XNAS')
  assert.equal(added.get('venue').asUtf8(), 'XNAS')
  assert.equal(mapping.get('venue'), null)
  assert.equal(added.remove('venue').length, 1)

  assert.throws(() => record.get(0), /field names must be strings/)
  assert.throws(() => record.set(0, 1), /field names must be strings/)
  assert.throws(() => mapping.remove(decimalKey), /string key/)
  assert.throws(() => legs.set(0, 1), /mapping or record/)
})

test('Scalar arithmetic infers JavaScript operands once and stays native', () => {
  const forty = Scalar.fromJs(40)
  assert.ok(forty.add(2).equals(Scalar.fromJs(42)))
  assert.ok(forty.subtract(Scalar.fromJs(2)).equals(Scalar.fromJs(38)))
  assert.ok(Scalar.fromJs(6).multiply(7).equals(Scalar.fromJs(42)))
  assert.ok(Scalar.fromJs(84).divide(2).equals(Scalar.fromJs(42)))
  assert.ok(Scalar.fromJs(5).remainder(2).equals(Scalar.fromJs(1)))
  assert.ok(Scalar.fromJs(5).negate().equals(Scalar.fromJs(-5)))
  assert.ok(Scalar.fromJs(-5).absolute().equals(Scalar.fromJs(5)))

  assert.ok(
    Scalar.d128(105n, 2).add(Scalar.d128(2n, 1)).equals(Scalar.d128(125n, 2)),
  )
  assert.ok(
    Scalar.d128(1n, 0)
      .divide(Scalar.d128(2n, 0))
      .equals(Scalar.d128(5n, 1)),
  )
  assert.ok(
    Scalar.d128(1n, 0)
      .divide(Scalar.d128(128n, 0))
      .equals(Scalar.d128(78125n, 7)),
  )
  assert.throws(
    () => Scalar.d128(1n, 0).divide(Scalar.d128(3n, 0)),
    (error) =>
      error instanceof RangeError &&
      error.code === 'ERR_YGGDRYL_INEXACT_ARITHMETIC',
  )
  assert.equal(Scalar.f16(1.5).multiply(Scalar.f32(2)).kind, 'f32')

  const instant = Scalar.datetime64(1000n, 'ms', 'UTC')
  assert.ok(
    instant
      .add(Scalar.duration64(2n, 's'))
      .equals(Scalar.datetime64(3000n, 'ms', 'UTC')),
  )
  assert.ok(
    instant
      .subtract(Scalar.datetime64(500n, 'ms', 'UTC'))
      .equals(Scalar.duration64(500n, 'ms')),
  )

  assert.throws(
    () => Scalar.fromJs(1).divide(0),
    (error) =>
      error instanceof RangeError &&
      error.code === 'ERR_YGGDRYL_DIVISION_BY_ZERO' &&
      /division by zero/i.test(error.message),
  )
  assert.throws(
    () => Scalar.fromJs(9223372036854775807n).add(1n),
    (error) =>
      error instanceof RangeError && error.code === 'ERR_YGGDRYL_ARITHMETIC_OVERFLOW',
  )
  assert.throws(
    () => Scalar.fromJs('a').add('b'),
    (error) =>
      error instanceof TypeError &&
      error.code === 'ERR_YGGDRYL_INVALID_ARITHMETIC' &&
      /addition/i.test(error.message),
  )
  for (const hidden of [
    '_addNative',
    '_subtractNative',
    '_multiplyNative',
    '_divideNative',
    '_remainderNative',
    '_negateNative',
    '_absoluteNative',
  ]) {
    assert.equal(forty[hidden], undefined, hidden)
  }
})

test('Field-directed natural JSON keeps exact typed values', () => {
  const narrow = json.loads('7', {
    field: new Field('value', 'int16', false),
    scalar: true,
  })
  assert.ok(narrow instanceof Scalar)
  assert.equal(narrow.kind, 'i16')

  const decimal = new Field('price', 'decimal256(40,2)', false)
  const decoded = json.loads('"123456789012345678901234567890.50"', {
    field: decimal,
  })
  assert.equal(decoded.kind, 'd256')
  assert.equal(decoded.scale, 2)
  assert.equal(decoded.unscaled, 12345678901234567890123456789050n)

  const row = new Field(
    'trade',
    'struct<quantity: int32 not null, symbol: utf8 not null>',
    false,
  )
  assert.deepEqual(
    json.loads('{"quantity":2,"symbol":"AAPL"}', { field: row }),
    { quantity: 2, symbol: 'AAPL' },
  )
  class Trade {
    static get intoStructField() {
      return row
    }
  }
  assert.deepEqual(
    json.loads('{"quantity":2,"symbol":"AAPL"}', { field: Trade }),
    { quantity: 2, symbol: 'AAPL' },
  )
  assert.deepEqual(
    json.loads('{"quantity":2,"symbol":"AAPL"}', { field: new Trade() }),
    { quantity: 2, symbol: 'AAPL' },
  )
  assert.equal(json.loads('1', { field: 'value: int64 not null' }), 1)
  assert.throws(() => json.loads('1', { field: {} }), /static intoStructField getter/)
})

test('Scalar Arrow scalar and array interop uses standard IPC', () => {
  const vector = arrow.vectorFromArray(Int32Array.from([1, 2, 3]))
  const values = Scalar.fromArrowArray(vector)
  assert.equal(values.kind, 'sequence')
  assert.deepEqual(values.asJs(), [1, 2, 3])
  assert.deepEqual([...values.intoArrowArray()], [1, 2, 3])

  const scalarVector = arrow.vectorFromArray(Int32Array.of(42))
  const scalar = Scalar.fromArrowScalar(scalarVector)
  assert.equal(scalar.kind, 'i32')
  assert.equal(scalar.intoArrowScalar(), 42)
  assert.throws(
    () => Scalar.fromArrowScalar(vector),
    /one-item Arrow Vector/,
  )

  const empty = Scalar.fromJs([])
  assert.throws(() => empty.intoArrowArray(), /empty.*pass a Field/i)
  const emptyVector = empty.intoArrowArray(new Field('value', 'int32', true))
  assert.equal(emptyVector.length, 0)

  const overflowing = arrow.vectorFromArray(Int32Array.of(200))
  assert.deepEqual(
    Scalar.fromArrowArray(overflowing, new Field('value', 'int8', false)).asJs(),
    [0],
  )
})

test('Scalar Field accessors redirect to core inference', () => {
  const scalar = Scalar.fromJs(42).intoField()
  assert.equal(scalar.name, 'value')
  assert.equal(scalar.dataType.toString(), 'int64')
  assert.equal(scalar.nullable, false)

  const item = Scalar.fromJs([1, null]).intoArrayField()
  assert.equal(item.name, 'item')
  assert.equal(item.dataType.toString(), 'int64')
  assert.equal(item.nullable, true)

  const root = Scalar.fromJs([{ id: 1, venue: null }, { id: 2, venue: 'XNAS' }])
    .intoStructField()
  assert.equal(root.name, 'row')
  assert.equal(root.nullable, false)
  const children = [...root.dataType]
  assert.deepEqual(children.map((child) => child.name), ['id', 'venue'])
  assert.equal(children[1].nullable, true)

  assert.throws(() => Scalar.fromJs([]).intoArrayField(), /empty Sequence/)
  assert.throws(() => Scalar.fromJs([[1]]).intoStructField(), /field names/)
})

test('Scalar Arrow record and table interop uses the native schema engine', () => {
  const table = arrow.tableFromArrays({
    id: Int32Array.from([1, 2]),
    symbol: ['AAPL', 'MSFT'],
  })
  const root = new Field(
    'row',
    DataType.fromFields([
      new Field('id', 'int32', false),
      new Field('symbol', 'utf8', false),
    ]),
    false,
  )
  const rows = Scalar.fromArrowTable(table, root)
  assert.deepEqual(rows.asJs(), [[1, 'AAPL'], [2, 'MSFT']])
  const restored = rows.intoArrowTable(root)
  assert.equal(restored.numRows, 2)
  assert.deepEqual([...restored.getChild('id')], [1, 2])

  const inferred = Scalar.fromJs([{ id: 1 }, { id: 2 }]).intoArrowBatch()
  assert.deepEqual([...inferred.getChild('id')], [1n, 2n])
  assert.throws(
    () => Scalar.fromJs([]).intoArrowTable(),
    /cannot infer a Struct Field from empty rows; pass a Struct Field/i,
  )
})

test('a Date is the JavaScript spelling of a UTC millisecond datetime64', () => {
  const date = new Date('2026-08-15T12:30:00.000Z')
  assert.ok(Scalar.fromJs(date).equals(Scalar.datetime64(1786797000000n, 'ms', 'UTC')))
  assert.ok(Scalar.fromJs(date).asJs() instanceof Date)

  // On the wire every temporal is its classic ISO string; the typed reading
  // comes back wherever a schema names the column's datatype.
  assert.equal(
    json.loads(json.dumps(Scalar.datetime64(1786797000n, 's', 'NAIVE'))),
    '2026-08-15T12:30:00',
  )
  assert.equal(
    json.loads(json.dumps(Scalar.datetime64(1786797000000n, 'ms', 'Europe/Paris'))),
    '2026-08-15T14:30:00.000+02:00[Europe/Paris]',
  )
})

test('null crosses everywhere a value goes', () => {
  // Null is a value, not a trap: alone, inside arrays, as an object value,
  // and through both codecs, it stays null - and undefined lowers to it.
  assert.equal(Scalar.fromJs(null).kind, 'null')
  assert.equal(Scalar.fromJs(null).asJs(), null)
  assert.deepEqual(json.loads(json.dumps({ gap: null, list: [null, 1] })), {
    gap: null,
    list: [null, 1],
  })
  assert.deepEqual(yaml.loads(yaml.dumps([null])), [null])
})

test('fromJs and asJs are the conversion every codec entry point crosses', () => {
  // The pivot answers what a JavaScript value becomes, losses included.
  assert.equal(Scalar.fromJs(new Set([1, 2])).kind, 'sequence')
  assert.deepEqual(Scalar.fromJs(new Set([1, 2])).asJs(), [1, 2])
  assert.equal(Scalar.fromJs(new Map([['id', 1]])).kind, 'mapping')
  assert.deepEqual(Scalar.fromJs(new Map([['id', 1]])).asJs(), new Map([['id', 1]]))
  assert.equal(Scalar.fromJs({ id: 1 }).kind, 'record')
  assert.equal(Scalar.fromJs(undefined).kind, 'null')

  // dumps is fromJs with bytes on the far side, and loads is asJs - except
  // the instant, which the wire spells as its classic string.
  const value = { id: 1, tags: new Set(['a']) }
  assert.deepEqual(json.loads(json.dumps(value)), Scalar.fromJs(value).asJs())
  assert.equal(json.loads(json.dumps({ at: new Date(0) })).at, '1970-01-01T00:00:00.000Z')
  assert.throws(() => Scalar.fromJs({}, { maxDepth: 0 }), /between 1 and 48/)
})

test('same-named native wrapper subclasses cannot lose application state', () => {
  const NativeDataType = DataType
  const NativeField = Field
  const NativeUri = Uri
  const NativeUrl = Url
  const NativeUrn = Urn
  const SubDataType = class DataType extends NativeDataType {
    constructor() {
      super('int64')
      this.applicationState = true
    }
  }
  const SubField = class Field extends NativeField {
    constructor() {
      super('id', 'int64', false)
      this.applicationState = true
    }
  }
  const SubUri = class Uri extends NativeUri {
    constructor() {
      super('https://example.com/value')
      this.applicationState = true
    }
  }
  const SubUrl = class Url extends NativeUrl {
    constructor() {
      super('https://example.com/value')
      this.applicationState = true
    }
  }
  const SubUrn = class Urn extends NativeUrn {
    constructor() {
      super('urn:example:value')
      this.applicationState = true
    }
  }

  for (const [value, name] of [
    [new SubDataType(), 'DataType'],
    [new SubField(), 'Field'],
    [new SubUri(), 'Uri'],
    [new SubUrl(), 'Url'],
    [new SubUrn(), 'Urn'],
  ]) {
    assert.throws(() => json.dumps(value), new RegExp(`${name} subclasses`))
  }
})

test('a class instance crosses as its own properties and nothing else', () => {
  let constructorCalls = 0
  class Order {
    static yggdrylType = 'orders.Order'

    constructor(id) {
      constructorCalls += 1
      this.id = id
    }
  }

  const bytes = yaml.dumps(new Order(42))
  const text = bytes.toString()
  constructorCalls = 0
  const decoded = yaml.loads(bytes)

  // No name travels beside the data, so nothing on the read side can be asked
  // to look one up, import a module, or run a constructor.
  assert.doesNotMatch(text, /yggdryl/)
  assert.doesNotMatch(text, /orders\.Order/)
  assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
  assert.deepEqual(decoded, { id: 42 })
  assert.equal(constructorCalls, 0)
})

test('decoded object keys never reach the prototype chain', () => {
  const value = {}
  Object.defineProperty(value, '__proto__', {
    enumerable: true,
    value: { polluted: true },
  })
  Object.defineProperty(value, 'constructor', {
    enumerable: true,
    value: 'payload data',
  })

  const decoded = yaml.loads(yaml.dumps(value))
  assert.deepEqual(decoded.__proto__, { polluted: true })
  assert.equal(decoded.constructor, 'payload data')
  assert.equal(Object.prototype.polluted, undefined)
  assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
})

test('built-in brands cannot be spoofed by constructor names', () => {
  const NamedMap = class Map {
    constructor() {
      this.id = 1
    }
  }
  const NamedDate = class Date {
    constructor() {
      this.id = 2
    }
  }
  const NamedDataType = class DataType {
    constructor() {
      this.id = 3
    }
  }

  for (const Constructor of [NamedMap, NamedDate, NamedDataType]) {
    const decoded = yaml.loads(yaml.dumps(new Constructor()))
    assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
    assert.deepEqual(decoded, { id: new Constructor().id })
  }
})

test('built-in subclasses fail explicitly instead of losing application state', () => {
  class MyMap extends Map {
    constructor() {
      super([['id', 1]])
      this.applicationState = true
    }
  }
  class MyDate extends Date {
    constructor() {
      super(0)
      this.applicationState = true
    }
  }
  class MyBytes extends Uint8Array {
    constructor() {
      super([1, 2])
      this.applicationState = true
    }
  }

  assert.throws(() => json.dumps(new MyMap()), /Map subclasses/)
  assert.throws(() => yaml.dumps(new MyDate()), /Date subclasses/)
  assert.throws(() => json.dumps(new MyBytes()), /Uint8Array subclasses/)
})

test('cross-realm slot-based built-ins fail instead of losing their contents', () => {
  for (const [value, name] of [
    [vm.runInNewContext('new Map([["id", 1]])'), 'Map'],
    [vm.runInNewContext('new Set([1, 2])'), 'Set'],
    [vm.runInNewContext('new RegExp("a/b", "giu")'), 'RegExp'],
  ]) {
    assert.throws(() => json.dumps(value), new RegExp(`cross-realm ${name}`))
  }
})

test('reserved transport keys and non-string map keys do not collide', () => {
  const publicBinding = require('yggdryl')
  const nativeHelpers = [
    'codecLoadsInferredNative',
    'jsonDumpPathNative',
    'jsonDumpsNative',
    'jsonLinesDumpAllNative',
    'jsonLinesDumpPathNative',
    'jsonLinesLoadPathNative',
    'jsonLinesLoadsNative',
    'jsonLoadPathNative',
    'jsonLoadsNative',
    'tomlDumpPathNative',
    'tomlDumpsNative',
    'tomlLoadPathNative',
    'tomlLoadsNative',
    'yamlDumpAllNative',
    'yamlDumpAllPathNative',
    'yamlDumpPathNative',
    'yamlDumpsNative',
    'yamlLoadAllPathNative',
    'yamlLoadPathNative',
    'yamlLoadsAllNative',
    'yamlLoadsNative',
  ]
  assert.equal(publicBinding.TaggedValue, undefined)
  assert.equal(publicBinding.codecDumps, undefined)
  assert.equal(publicBinding.Scalar._fromJsNative, undefined)
  assert.equal(publicBinding.Scalar.prototype._asJsNative, undefined)
  for (const name of nativeHelpers) assert.equal(publicBinding[name], undefined)

  const generatedTypes = fs.readFileSync(
    path.join(__dirname, '..', 'index.d.ts'),
    'utf8',
  )
  for (const name of nativeHelpers) assert.doesNotMatch(generatedTypes, new RegExp(name))
  const value = {}
  Object.defineProperty(value, '__yggdryl_codec__', {
    enumerable: true,
    value: 'timestamp',
  })
  Object.defineProperty(value, '__proto__', {
    enumerable: true,
    value: 'data',
  })
  Object.defineProperties(value, {
    unit: { enumerable: true, value: 'ms' },
    value: { enumerable: true, value: '0' },
    zone: { enumerable: true, value: null },
  })

  const decoded = json.loads(json.dumps(value))
  assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
  assert.equal(decoded.__yggdryl_codec__, 'timestamp')
  assert.equal(decoded.__proto__, 'data')
  assert.equal(decoded.unit, 'ms')
  assert.equal(decoded.value, '0')
  assert.equal(decoded instanceof Date, false)

  const mapped = yaml.loads(yaml.dumps(new Map([[42, 'answer']])))
  assert.ok(mapped instanceof Map)
  assert.equal(mapped.get(42), 'answer')

  assert.throws(
    () => yaml.loads('1: integer\n1.0: floating\n'),
    /collide under JavaScript Map equality/,
  )
})

test('source intent is type-driven without existence probes', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-'))
  const file = path.join(directory, 'value.yaml')
  try {
    yaml.dump({ id: 42 }, file)
    // A string is content even when an existing file has that exact name.
    const statSync = fs.statSync
    let probed = false
    try {
      fs.statSync = (...arguments_) => {
        probed = true
        return statSync(...arguments_)
      }
      assert.equal(yaml.load(file), file)
    } finally {
      fs.statSync = statSync
    }
    assert.equal(probed, false)
    assert.equal(yaml.load('id: 43\n').id, 43)

    const yamlUrl = pathToFileURL(file)
    assert.equal(yaml.load(yamlUrl).id, 42)
    assert.equal(codec.from(yamlUrl).id, 42)

    const jsonFile = path.join(directory, 'value.json')
    const jsonUrl = pathToFileURL(jsonFile)
    codec.into({ id: 44 }, jsonUrl)
    assert.equal(codec.from(jsonUrl).id, 44)

    const jsonLinesFile = path.join(directory, 'rows.jsonl')
    const rows = [[1, 2], { id: 45 }]
    codec.into(rows, jsonLinesFile)
    assert.deepEqual(codec.from(pathToFileURL(jsonLinesFile)), rows)

    const misleadingJson = path.join(directory, 'actually-yaml.json')
    fs.writeFileSync(misleadingJson, 'id: 46\n')
    assert.deepEqual(
      codec.from(pathToFileURL(misleadingJson), { format: 'yaml' }),
      { id: 46 },
    )

    const misleadingYaml = path.join(directory, 'actually-json.yaml')
    codec.into({ id: 47 }, misleadingYaml, { format: 'json' })
    assert.deepEqual(json.load(pathToFileURL(misleadingYaml)), { id: 47 })

    const missingJsonLinesFile = path.join(directory, 'missing.jsonl')
    assert.equal(codec.from(missingJsonLinesFile), missingJsonLinesFile)
    assert.equal(codec.from(path.join(directory, 'missing.json')), path.join(directory, 'missing.json'))
    assert.equal(codec.from(path.join(directory, 'missing.yaml')), path.join(directory, 'missing.yaml'))
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('string and byte-view content preserve exact source boundaries', () => {
  assert.deepEqual(json.loads('{"id":42}'), { id: 42 })
  assert.deepEqual(yaml.loads('id: 43\n'), { id: 43 })

  const framed = Buffer.from('xx{"id":44}yy')
  const view = new DataView(framed.buffer, framed.byteOffset + 2, 9)
  assert.deepEqual(json.loads(view), { id: 44 })

  const shared = new SharedArrayBuffer(7)
  new Uint8Array(shared).set(Buffer.from('[45,46]'))
  assert.deepEqual(json.loads(shared), [45, 46])
})

test('native path readers and writers bypass JavaScript whole-file buffers', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-native-path-'))
  const source = path.join(directory, 'source.yaml')
  const destination = path.join(directory, 'destination.json')
  fs.writeFileSync(source, 'id: 47\n')
  const readSync = fs.readSync
  const writeFileSync = fs.writeFileSync
  try {
    fs.readSync = () => {
      throw new Error('JavaScript read buffer must not be used for a real path')
    }
    assert.deepEqual(yaml.load(pathToFileURL(source)), { id: 47 })

    fs.writeFileSync = () => {
      throw new Error('JavaScript output buffer must not be used for a path')
    }
    json.dump({ id: 48 }, destination)
  } finally {
    fs.readSync = readSync
    fs.writeFileSync = writeFileSync
  }
  try {
    assert.deepEqual(JSON.parse(fs.readFileSync(destination, 'utf8')), { id: 48 })
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('file descriptors remain caller-owned after synchronous codec I/O', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-fd-'))
  const source = path.join(directory, 'source.json')
  const destination = path.join(directory, 'destination.yaml')
  fs.writeFileSync(source, '{"id":49}')
  const sourceFd = fs.openSync(source, 'r')
  const destinationFd = fs.openSync(destination, 'w+')
  try {
    const fstatSync = fs.fstatSync
    try {
      fs.fstatSync = () => {
        throw new Error('descriptor reads must act without a metadata probe')
      }
      assert.deepEqual(json.load(sourceFd), { id: 49 })
    } finally {
      fs.fstatSync = fstatSync
    }
    assert.ok(fs.fstatSync(sourceFd).isFile())
    yaml.dump({ id: 50 }, destinationFd)
    assert.ok(fs.fstatSync(destinationFd).isFile())
  } finally {
    fs.closeSync(sourceFd)
    fs.closeSync(destinationFd)
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('load and dump redirect Node and WHATWG streams with sound async behavior', async () => {
  assert.deepEqual(
    await json.load(Readable.from(['{"id":', '51}'])),
    { id: 51 },
  )

  const rows = []
  for await (const row of json.loadAll(
    Readable.from(['{"id":52}\n', '{"id":53}\n']),
  )) {
    rows.push(row)
  }
  assert.deepEqual(rows, [{ id: 52 }, { id: 53 }])

  const nodeChunks = []
  const nodeOutput = new Writable({
    write(chunk, _encoding, done) {
      nodeChunks.push(Buffer.from(chunk))
      done()
    },
  })
  await yaml.dump({ id: 54 }, nodeOutput)
  assert.deepEqual(yaml.loads(Buffer.concat(nodeChunks)), { id: 54 })

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
  await json.dump({ id: 55 }, webOutput)
  assert.deepEqual(json.loads(Buffer.concat(webChunks)), { id: 55 })
  assert.equal(webClosed, false)
})

test('redirected stream writes preserve errors and document backpressure', async () => {
  const expected = new Error('sink rejected frame')
  const failing = new Writable({
    write(_chunk, _encoding, done) {
      done(expected)
    },
  })
  await assert.rejects(yaml.dump({ id: 56 }, failing), (error) => error === expected)

  const cyclic = {}
  cyclic.self = cyclic
  const partialChunks = []
  const partial = new Writable({
    write(chunk, _encoding, done) {
      partialChunks.push(Buffer.from(chunk))
      done()
    },
  })
  await assert.rejects(yaml.dumpAll([{ id: 57 }, cyclic], partial), /cyclic/)
  const partialBytes = Buffer.concat(partialChunks)
  assert.doesNotMatch(partialBytes.toString(), /^---|\n---/)
  assert.deepEqual(yaml.loads(partialBytes), { id: 57 })

  let produced = 0
  let completedFrames = 0
  async function* values() {
    for (let id = 0; id < 3; id += 1) {
      assert.equal(produced, completedFrames)
      produced += 1
      yield { id }
    }
  }
  const chunks = []
  const slow = new Writable({
    write(chunk, _encoding, done) {
      chunks.push(Buffer.from(chunk))
      setImmediate(() => {
        if (chunk.length !== 0 && chunk[chunk.length - 1] === 0x0a) {
          completedFrames += 1
        }
        done()
      })
    },
  })
  await json.dumpAll(values(), slow)
  assert.equal(produced, 3)
  assert.equal(completedFrames, 3)
  assert.deepEqual(json.loadsAll(Buffer.concat(chunks)), [
    { id: 0 },
    { id: 1 },
    { id: 2 },
  ])
})

test('generic stream inference consumes anonymous sources exactly once', async () => {
  let pulls = 0
  async function* source() {
    pulls += 1
    yield '{"id":'
    pulls += 1
    yield '57}'
  }
  assert.deepEqual(await codec.from(source()), { id: 57 })
  assert.equal(pulls, 2)

  const rows = []
  for await (const row of codec.from(
    Readable.from(['{"id":58}\n', '{"id":59}\n']),
    { format: 'jsonl' },
  )) {
    rows.push(row)
  }
  assert.deepEqual(rows, [{ id: 58 }, { id: 59 }])

  let coerced = false
  assert.throws(
    () => codec.from('{}', {
      format: {
        toString() {
          coerced = true
          return 'json'
        },
      },
    }),
    /format must be a string/,
  )
  assert.equal(coerced, false)
  assert.equal(codec.from('\u00a0{"id":60}'), '{"id":60}')
})

test('string stream chunks preserve split Unicode surrogate pairs centrally', async () => {
  async function* jsonChunks() {
    yield '{"value":"\ud83d'
    yield '\ude42"}'
  }
  assert.deepEqual(await json.load(jsonChunks()), { value: '🙂' })
  assert.deepEqual(await codec.from(jsonChunks()), { value: '🙂' })

  const jsonLinesValues = []
  for await (const value of json.loadAllStream(jsonChunksWithNewline())) {
    jsonLinesValues.push(value)
  }
  assert.deepEqual(jsonLinesValues, [{ value: '🙂' }])

  async function* jsonChunksWithNewline() {
    yield '{"value":"\ud83d'
    yield '\ude42"}\n'
  }

  const yamlValues = []
  for await (const value of yaml.loadAllStream(yamlChunks())) {
    yamlValues.push(value)
  }
  assert.deepEqual(yamlValues, [{ value: '🙂' }])

  async function* yamlChunks() {
    yield '---\nvalue: "\ud83d'
    yield '\ude42"\n'
  }

  async function* danglingHighSurrogate() {
    yield 'value: \ud83d'
  }
  async function* unpairedLowSurrogate() {
    yield 'value: \ude42'
  }
  assert.deepEqual(await yaml.load(danglingHighSurrogate()), { value: '�' })
  assert.deepEqual(await yaml.load(unpairedLowSurrogate()), { value: '�' })
})

test('path and single-value stream reads enforce limits before unbounded allocation', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-limit-'))
  const file = path.join(directory, 'oversized.json')
  try {
    fs.writeFileSync(file, '')
    fs.truncateSync(file, 64 * 1024 * 1024 + 1)
    assert.throws(() => json.load(pathToFileURL(file)), /input limit/)
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }

  const chunk = Buffer.alloc(1024 * 1024)
  let yielded = 0
  async function* oversized() {
    for (let index = 0; index < 65; index += 1) {
      yielded += 1
      yield chunk
    }
  }
  await assert.rejects(() => json.loadStream(oversized()), /input limit/)
  assert.equal(yielded, 65)

  let pulledInvalid = false
  async function* invalidOptionsSource() {
    pulledInvalid = true
    yield '{}'
  }
  await assert.rejects(
    () => json.loadStream(invalidOptionsSource(), { maxDepth: 0 }),
    /between 1 and 48/,
  )
  assert.equal(pulledInvalid, false)
})

test('buffered JSON Lines and YAML document collections round-trip', () => {
  const values = [{ id: 1 }, { id: 2 }, { id: 3 }]

  assert.deepEqual(json.loadsAll(json.dumpAll(values)), values)
  assert.deepEqual(yaml.loadsAll(yaml.dumpAll(values)), values)
})

test('buffered collection encoders stop before unbounded materialization', () => {
  let pulled = 0
  let closed = false
  function* tooMany() {
    try {
      while (true) {
        pulled += 1
        yield { id: pulled }
      }
    } finally {
      closed = true
    }
  }

  assert.throws(() => json.dumpAll(tooMany()), /1024-document limit/)
  assert.equal(pulled, 1025)
  assert.equal(closed, true)
  assert.throws(
    () =>
      nativeYamlDumpAll(
        Array(1025).fill(null),
        undefined,
        'default',
        rawNativeWrapperPrototypes,
        rawNativeIntrinsics,
      ),
    /1024-document limit/,
  )
})

test('generic JSON Lines operations treat the value as a row collection', async () => {
  const rows = [[1, 2], { id: 2 }]
  const bytes = codec.into(rows, { format: 'jsonl' })
  assert.deepEqual(codec.from(bytes, { format: 'ndjson' }), rows)
  assert.deepEqual(codec.from(bytes, { format: 'json-lines' }), rows)

  const decoded = codec.fromStream(
    Readable.from([Buffer.from('[1,2]\n'), Buffer.from('{"id":2}\n')]),
    { format: 'json_lines' },
  )
  assert.equal(typeof decoded[Symbol.asyncIterator], 'function')
  const streamedRows = []
  for await (const row of decoded) streamedRows.push(row)
  assert.deepEqual(streamedRows, rows)

  const chunks = []
  const output = new Writable({
    write(chunk, _encoding, done) {
      chunks.push(Buffer.from(chunk))
      done()
    },
  })
  await codec.intoStream(rows, output, { format: 'jsonl' })
  assert.deepEqual(codec.from(Buffer.concat(chunks), { format: 'jsonl' }), rows)
})

test('JSON Lines streams decode incrementally across arbitrary chunks', async () => {
  let pulls = 0
  async function* chunks() {
    pulls += 1
    yield Buffer.from('{"id":1}\n{"i')
    pulls += 1
    yield Buffer.from('d":2}\r\n')
  }

  const iterator = json.loadAllStream(chunks())[Symbol.asyncIterator]()
  assert.deepEqual((await iterator.next()).value, { id: 1 })
  assert.equal(pulls, 1)
  assert.deepEqual((await iterator.next()).value, { id: 2 })
  assert.equal(pulls, 2)
  assert.equal((await iterator.next()).done, true)
})

test('one-byte stream chunks do not require repeated whole-line copies', async () => {
  const payload = 'x'.repeat(32 * 1024)
  const encoded = Buffer.from(JSON.stringify({ payload }) + '\n')
  async function* bytes() {
    for (const byte of encoded) yield Buffer.of(byte)
  }

  const values = []
  for await (const value of json.loadAllStream(bytes())) values.push(value)
  assert.equal(values.length, 1)
  assert.equal(values[0].payload, payload)
})

test('YAML streams preserve block content and decode one document at a time', async () => {
  const input = Readable.from([
    Buffer.from('---\nid: 1\nnote: |\n  keep\n  ---\n'),
    Buffer.from('---\nid: 2\n'),
  ])
  const values = []
  for await (const value of yaml.loadAllStream(input)) values.push(value)

  assert.deepEqual(values, [
    { id: 1, note: 'keep\n---\n' },
    { id: 2 },
  ])
})

test('YAML streams preserve explicit null documents at every position', async () => {
  const input = Readable.from([
    Buffer.from('---\nnull\n---\n'),
    Buffer.from('---\nid: 1\n---'),
  ])
  const values = []
  for await (const value of yaml.loadAllStream(input)) values.push(value)

  assert.deepEqual(values, [null, null, { id: 1 }, null])
})

test('YAML stream framing matches core marker separation rules', async () => {
  const content = Buffer.from('---#not-a-marker\n...#not-an-end\n')
  const buffered = yaml.loadsAll(content)
  const streamed = []
  for await (const value of yaml.loadAllStream(Readable.from([content]))) {
    streamed.push(value)
  }
  assert.deepEqual(streamed, buffered)
})

test('YAML streams frame LF, CRLF, and lone CR across chunk boundaries', async () => {
  async function collect(chunks) {
    async function* source() {
      yield* chunks
    }
    const values = []
    for await (const value of yaml.loadAllStream(source())) values.push(value)
    return values
  }

  const loneCarriageReturn = Buffer.from('---\rid: 1\r---\rid: 2\r')
  assert.deepEqual(await collect([loneCarriageReturn]), [
    { id: 1 },
    { id: 2 },
  ])
  assert.deepEqual(
    await collect([Buffer.from('---\r')]),
    yaml.loadsAll(Buffer.from('---\r')),
  )

  const crlf = Buffer.from('---\r\nid: 1\r\n---\r\nid: 2\r\n')
  assert.deepEqual(
    await collect(['---\r', '\nid: 1\r', '\n---\r', '\nid: 2\r', '\n']),
    yaml.loadsAll(crlf),
  )
})

test('YAML stream preamble classification uses exact YAML bytes', async () => {
  async function collect(chunks) {
    async function* source() {
      yield* chunks
    }
    const values = []
    for await (const value of yaml.loadAllStream(source())) values.push(value)
    return values
  }

  async function assertParity(content, chunks = [content]) {
    let expected
    try {
      expected = yaml.loadsAll(content)
    } catch (bufferedError) {
      await assert.rejects(
        () => collect(chunks),
        (streamError) => {
          assert.match(streamError.message, /YAML stream error/)
          assert.match(streamError.message, /cumulative byte/)
          assert.equal(streamError.cause?.name, bufferedError.name)
          return true
        },
      )
      return
    }
    assert.deepEqual(await collect(chunks), expected)
  }

  assert.deepEqual(await collect(['...\n']), [])
  assert.deepEqual(await collect(['# comment\n', '...\n']), [])
  await assertParity('  %YAML 1.2\n', ['  %Y', 'AML 1.2\n'])
  await assertParity('%YAML 1.2\n...\n', ['%YAML 1.2\n', '...\n'])
  await assertParity('%YAML 1.2\n---\nid: 1\n')

  const nonBreakingSpaceScalar = '\u00a0# scalar, not a comment\n'
  assert.deepEqual(yaml.loadsAll(nonBreakingSpaceScalar), [
    '# scalar, not a comment',
  ])
  await assertParity(nonBreakingSpaceScalar, ['\u00a0', '# scalar, not a comment\n'])

  // JavaScript trimStart() treats these controls as whitespace. YAML framing
  // must pass them to the native parser instead of silently discarding them.
  await assertParity('\u000b\n', ['\u000b', '\n'])
  await assertParity('\u000c\n', ['\u000c', '\n'])
})

test('YAML stream document splits preserve block-scalar end context', async () => {
  async function collectOneByteChunks(content) {
    async function* chunks() {
      for (const byte of Buffer.from(content)) yield Buffer.of(byte)
    }
    const values = []
    for await (const value of yaml.loadAllStream(chunks())) values.push(value)
    return values
  }

  for (const indicator of ['|', '|-', '|+', '>', '>-', '>+']) {
    const empty = `text: ${indicator}\n---\nid: 2\n`
    assert.deepEqual(await collectOneByteChunks(empty), yaml.loadsAll(empty))

    const populated = `text: ${indicator}\n  first\n  second\n---\nid: 2\n`
    assert.deepEqual(
      await collectOneByteChunks(populated),
      yaml.loadsAll(populated),
    )
  }

  const loneCarriageReturn = 'text: |\r---\rid: 2\r'
  assert.deepEqual(
    await collectOneByteChunks(loneCarriageReturn),
    yaml.loadsAll(loneCarriageReturn),
  )

  await assert.rejects(
    () => collectOneByteChunks('items: [1\n---\nid: 2\n'),
    /cumulative byte 10/,
  )
})

test('streamed writers frame and flush each value separately', async () => {
  const chunks = []
  const output = new Writable({
    write(chunk, _encoding, done) {
      chunks.push(Buffer.from(chunk))
      done()
    },
  })

  async function* values() {
    yield [1, 2]
    yield { id: 2 }
  }

  await json.dumpAllStream(values(), output)
  assert.ok(chunks.length >= 2)
  assert.deepEqual(json.loadsAll(Buffer.concat(chunks)), [[1, 2], { id: 2 }])
})

test('stream errors report cumulative byte offsets', async () => {
  const input = Readable.from([
    Buffer.from('{"id":1}\n'),
    Buffer.from('{bad}\n'),
  ])

  await assert.rejects(
    async () => {
      for await (const _value of json.loadAllStream(input)) {
        // Drain the iterator to surface its second-line failure.
      }
    },
    /cumulative byte 10 \(frame byte 1\)/,
  )

  await assert.rejects(
    async () => {
      for await (const _value of json.loadAllStream(
        Readable.from([Buffer.from([0xc2, 0xa0, 0x0a])]),
      )) {
        // A non-breaking space is not legal JSON whitespace.
      }
    },
    /JSON Lines stream error/,
  )
})

test('structured codec formatting redirects to the core for every destination', () => {
  const value = { outer: { enabled: true, values: [1, 2] } }
  assert.equal(
    json.dumps(value, { indent: 2 }).toString(),
    '{\n  "outer": {\n    "enabled": true,\n    "values": [\n      1,\n      2\n    ]\n  }\n}',
  )
  assert.deepEqual(json.loads(json.dumps(value, { indent: null })), value)
  assert.match(yaml.dumps(value, { indent: null }).toString(), /^\{.*\}\n?$/s)

  for (const [name, format] of [['json', json], ['yaml', yaml], ['toml', toml]]) {
    for (const indent of [null, 0, 2, 255, '\t']) {
      const encoded = format.dumps(value, { indent })
      assert.deepEqual(format.loads(encoded), value, `${name} indent ${String(indent)}`)
      assert.deepEqual(
        codec.into(value, { format: name, indent }),
        encoded,
        `${name} generic redirect`,
      )
    }
  }

  assert.throws(() => json.dumps(value, { indent: -1 }), /indent/)
  assert.throws(() => json.dumps(value, { indent: '  ' }), /indent/)
  assert.throws(() => json.dumps(value, { indent: 256 }), /indent/)
})

test('nullable parser limits reach byte, inferred, collection, path, and stream decoders', async (t) => {
  assert.deepEqual(json.loads('{"id":1}', { maxDepth: null }), { id: 1 })
  assert.throws(() => json.loads('{"id":1}', { maxInputBytes: 3 }), /input byte limit/i)
  assert.throws(() => json.loads('[1,2]', { maxNodes: 2 }), /node limit/i)
  assert.throws(() => json.loadsAll('1\n2\n', { maxDocuments: 1 }), /document limit/i)
  assert.throws(() => yaml.loadsAll('---\n1\n---\n2\n', { maxDocuments: 1 }), /document limit/i)

  const field = new Field('row', 'struct<id:int64 not null>', false)
  assert.throws(
    () => codec.from('{"id":1}', { field, maxNodes: 1 }),
    /node limit/i,
  )

  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-options-'))
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  const file = path.join(directory, 'value.json')
  fs.writeFileSync(file, '{"id":1}')
  assert.throws(
    () => json.load(pathToFileURL(file), { maxInputBytes: 3 }),
    /input limit/i,
  )

  await assert.rejects(
    () => json.load(Readable.from(['{"id":', '1}']), { maxInputBytes: 4 }),
    /4-byte input limit/,
  )
  const rows = []
  await assert.rejects(
    async () => {
      for await (const row of json.loadAllStream(
        Readable.from(['1\n', '2\n']),
        { maxDocuments: 1 },
      )) rows.push(row)
    },
    /document limit/i,
  )
  assert.deepEqual(rows, [1])

  assert.throws(() => json.loads('1', { maxNodes: -1 }), /maxNodes/)
  assert.throws(() => json.loads('1', { maxInputBytes: 1.5 }), /maxInputBytes/)
  assert.throws(() => json.loads('1', { maxDocuments: '1' }), /maxDocuments/)
})

test('depth limits reject adversarial nested input without recursion overflow', () => {
  let value = 0
  for (let index = 0; index < 32; index += 1) value = [value]

  assert.throws(() => json.dumps(value, { maxDepth: 16 }), /maxDepth 16/)
  const bytes = json.dumps(value)
  assert.throws(() => json.loads(bytes, { maxDepth: 16 }), /depth/i)

  let adversarial = 0
  for (let index = 0; index < 5000; index += 1) adversarial = [adversarial]
  assert.throws(
    () => json.dumps(adversarial, { maxDepth: 5000 }),
    /between 1 and 48/,
  )
  assert.throws(() => json.dumps(adversarial), /maxDepth 48/)
  assert.throws(
    () =>
      nativeJsonDumps(
        {},
        5000,
        'default',
        rawNativeWrapperPrototypes,
        rawNativeIntrinsics,
      ),
    /between 1 and 48/,
  )
})

test('malformed streams and cyclic objects fail deterministically', () => {
  const cyclic = {}
  cyclic.self = cyclic

  assert.throws(() => yaml.dumps(cyclic), /cyclic/)
  assert.throws(() => json.loads('{"missing":]'), /JSON/i)
  assert.throws(() => yaml.loads('value: [unterminated'), /YAML/i)
})

test('the byte codings round-trip and read node:zlib output', () => {
  const zlibNative = require('node:zlib')
  const { gzip, zlib, zstd } = require('yggdryl')
  const payload = Buffer.from('{"id":1}\n'.repeat(512))

  assert.deepEqual(gzip.loads(gzip.dumps(payload)), payload)
  assert.deepEqual(gzip.loads(zlibNative.gzipSync(payload)), payload)
  assert.deepEqual(zlibNative.gunzipSync(gzip.dumps(payload, 9)), payload)
  assert.deepEqual(zlib.loads(zlib.dumps(payload)), payload)
  assert.deepEqual(zstd.loads(zstd.dumps(payload, 9)), payload)
})

test('raw DEFLATE round-trips, reads node:zlib, and shares no framing with zlib', () => {
  const zlibNative = require('node:zlib')
  const { zlib } = require('yggdryl')
  const payload = Buffer.from('{"id":1}\n'.repeat(512))

  assert.deepEqual(zlib.loadsRaw(zlib.dumpsRaw(payload)), payload)
  assert.deepEqual(zlib.loadsRaw(zlibNative.deflateRawSync(payload)), payload)
  assert.deepEqual(zlibNative.inflateRawSync(zlib.dumpsRaw(payload, 9)), payload)
  // The raw output is the framed output without the two-byte header and the
  // four-byte checksum, which is the whole of the difference.
  assert.equal(zlib.dumpsRaw(payload).length + 6, zlib.dumps(payload).length)

  // Nothing in unframed bytes says which framing they are, so the pair is
  // named rather than sniffed - and each half refuses the other's output
  // instead of decoding it into something plausible.
  assert.throws(() => zlib.loads(zlib.dumpsRaw(payload)), /deflate/)
  assert.throws(() => zlib.loadsRaw(zlib.dumps(payload)), /deflate/)
})
