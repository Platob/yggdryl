'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const binding = require('..')
const { DataType, Field, fields } = binding

test('internal typed-factory bridges stay outside the public package surface', () => {
  for (const name of [
    '_simple',
    '_temporal',
    '_fixedSizeBinary',
    '_decimal',
    '_list',
    '_fromFields',
    '_union',
    '_variant',
    '_dictionary',
    '_map',
    '_mapOf',
    '_runEndEncoded',
    'fromArrowString',
  ]) {
    assert.equal(Object.hasOwn(DataType, name), false, name)
  }
  assert.equal(Object.hasOwn(Field, 'fromArrowString'), false)
  assert.equal(Object.hasOwn(Field.prototype, '_showDiffs'), false)
  assert.equal(Object.hasOwn(DataType.prototype, '_showDiffs'), false)
  for (const name of ['DifferenceIterator', 'JsDifferenceIterator']) {
    assert.equal(Object.hasOwn(binding, name), false, name)
  }
  assert.equal(new DataType('int32').constructor, DataType)
  assert.equal(new Field('id', 'int32').constructor, Field)
  assert.equal(Object.hasOwn(new DataType('int32').constructor, '_simple'), false)
})

test('DataType.fromFields is the iterable-aware native Struct builder', () => {
  const id = fields.int32('id', {
    nullable: false,
    metadata: { physical: 'int32' },
  })
  const type = DataType.fromFields((function* children() { yield id })())

  assert.equal(type.kind, 'struct')
  assert.ok(type.at(0).equals(id))
  assert.equal(type.at(0).get('physical'), 'int32')
  assert.equal(DataType.fromFields([]).length, 0)
  assert.throws(() => DataType.fromFields('not fields'), /iterable of native Field/)
  assert.throws(() => DataType.fromFields(null), /iterable of native Field/)
  assert.throws(() => DataType.fromFields([id, {}]))
  assert.throws(() => DataType.fromFields([id, id]), /duplicate field name/)
})

test('Variant assigns deterministic dense Union IDs through one native builder', () => {
  const text = fields.utf8('text', { nullable: false })
  const code = fields.int64('code', { nullable: false })
  let pulls = 0
  const members = (function* variantMembers() {
    pulls += 1
    yield text
    yield code
  })()
  const dataType = DataType.variant(members)

  assert.equal(pulls, 1)
  assert.equal(dataType.kind, 'union')
  assert.equal(
    dataType.toString(),
    'union(dense,0=field("text",utf8,nullable=false,metadata={}),' +
      '1=field("code",int64,nullable=false,metadata={}))',
  )
  assert.deepEqual(dataType.defaultJSValue(), { typeId: 0, value: '' })
  assert.ok(
    dataType.equals(
      fields.union('holder', [[0, text], [1, code]], 'dense').dataType,
    ),
  )
  assert.ok(
    fields.variant('payload', [text, code], { nullable: false })
      .dataType.equals(dataType),
  )
  assert.equal(
    DataType.fromString(
      'variant(field("text",utf8,nullable=false,metadata={}),' +
        'field("code",int64,nullable=false,metadata={}))',
    ).toString(),
    dataType.toString(),
  )
  assert.throws(
    () => DataType.fromString('variant(sparse,text:utf8,code:int64)'),
    /variant.*dense|dense.*variant/i,
  )
  assert.throws(() => DataType.variant('not fields'), /iterable of native Field/)
  assert.throws(() => DataType.variant([text, text]), /duplicate field name/)
  assert.throws(
    () => DataType.variant(Array.from({ length: 129 }, (_, index) => fields.int8(`m${index}`))),
    /128-member limit|more than 128|at most 128/i,
  )
})

test('typed field factories cover every native datatype variant', () => {
  const item = fields.int8('item', { nullable: false })
  const entries = fields.struct(
    'entries',
    [
      fields.utf8('key', { nullable: false }),
      fields.int64('value'),
    ],
    { nullable: false },
  )
  const runEnds = fields.int16('run_ends', { nullable: false })
  const values = fields.utf8('values')
  // Keyed by the native DataTypeId - the parameter-free identity of one
  // variant. The coarse family a variant belongs to is its kind.
  const byId = new Map([
    ['null', fields.null('value')],
    ['boolean', fields.boolean('value')],
    ['int8', fields.int8('value')],
    ['int16', fields.int16('value')],
    ['int32', fields.int32('value')],
    ['int64', fields.int64('value')],
    ['uint8', fields.uint8('value')],
    ['uint16', fields.uint16('value')],
    ['uint32', fields.uint32('value')],
    ['uint64', fields.uint64('value')],
    ['float16', fields.float16('value')],
    ['float32', fields.float32('value')],
    ['float64', fields.float64('value')],
    ['timestamp', fields.timestamp('value', 'us', 'Europe/Paris')],
    ['date32', fields.date32('value')],
    ['date64', fields.date64('value')],
    ['time32', fields.time32('value', 'ms')],
    ['time64', fields.time64('value', 'ns')],
    ['duration', fields.duration('value', 'us')],
    ['interval', fields.interval('value', 'month_day_nano')],
    ['binary', fields.binary('value')],
    ['fixed_size_binary', fields.fixedSizeBinary('value', 16)],
    ['large_binary', fields.largeBinary('value')],
    ['binary_view', fields.binaryView('value')],
    ['utf8', fields.utf8('value')],
    ['large_utf8', fields.largeUtf8('value')],
    ['utf8_view', fields.utf8View('value')],
    ['list', fields.list('value', item)],
    ['list_view', fields.listView('value', item)],
    ['fixed_size_list', fields.fixedSizeList('value', item, 3)],
    ['large_list', fields.largeList('value', item)],
    ['large_list_view', fields.largeListView('value', item)],
    ['struct', fields.struct('value', [item])],
    ['union', fields.union('value', [[3, item]], 'dense')],
    ['dictionary', fields.dictionary('value', 'int16', 'utf8')],
    ['decimal32', fields.decimal32('value', 9, 2)],
    ['decimal64', fields.decimal64('value', 18, 2)],
    ['decimal128', fields.decimal128('value', 38, 2)],
    ['decimal256', fields.decimal256('value', 76, 2)],
    ['map', fields.map('value', entries, true)],
    [
      'run_end_encoded',
      fields.runEndEncoded('value', runEnds, values),
    ],
  ])

  assert.equal(byId.size, 41)
  assert.ok([...byId.values()].every((value) => value instanceof Field))
  // Every factory above was called without a nullable option, and the Python
  // factories default the same way, so one declared schema cannot disagree
  // about nullability across the two languages.
  for (const [id, value] of byId) {
    assert.equal(value.nullable, true, id)
  }
  for (const [id, value] of byId) {
    // Canonical display opens with the variant id and appends its parameters.
    assert.equal(value.dataType.toString().split(/[(<]/, 1)[0], id, id)
  }
  assert.deepEqual(
    new Set([...byId.values()].map((value) => value.dataType.kind)),
    new Set([
      'null',
      'boolean',
      'integer',
      'floating',
      'decimal',
      'temporal',
      'binary',
      'string',
      'list',
      'struct',
      'union',
      'map',
      'dictionary',
      'run_end_encoded',
    ]),
  )
})

test('nested factories preserve exact child metadata and dictionary state', () => {
  const item = fields.dictionary('item', 'int16', 'utf8', {
    nullable: false,
    metadata: { logical: 'status' },
  })
  item.setDictionaryOptions(42n, true)

  const values = fields.list('values', item, {
    metadata: new Map([['owner', 'events']]),
  })
  const child = values.dataType.at(0)

  assert.ok(child.equals(item))
  assert.equal(child.dictionaryId, 42n)
  assert.equal(child.dictionaryIsOrdered, true)
  assert.equal(child.get('logical'), 'status')
  assert.equal(values.get('owner'), 'events')
})

test('metadata entry overlays use last-write-wins without coercion', () => {
  const field = fields.int32('id', {
    metadata: [
      ['source', 'first'],
      { key: 'source', value: 'last' },
      ['owner', 'events'],
    ],
  })

  assert.equal(field.get('source'), 'last')
  assert.equal(field.get('owner'), 'events')

  field.update([
    ['source', 'updated'],
    ['source', 'final'],
  ])
  assert.equal(field.get('source'), 'final')
  assert.throws(() => field.update(new Map([['attempts', 3]])), /must be strings/)
  assert.throws(() => fields.int32('bad', { metadata: [['key']] }), /two items/)
  assert.throws(() => fields.int32('bad', { nullable: null }), /must be a boolean/)
  assert.equal(field.has('attempts'), false)
})

test('typed factory parameters delegate native validation', () => {
  // Canonical display names the selected physical variant, which `kind` folds
  // into its family (`decimal`, `temporal`).
  assert.equal(fields.decimal('small', 38).dataType.toString(), 'decimal128(38,0)')
  assert.equal(fields.decimal('wide', 39).dataType.toString(), 'decimal256(39,0)')
  assert.equal(fields.time('coarse', 'ms').dataType.toString(), 'time32(ms)')
  assert.equal(fields.time('precise', 'ns').dataType.toString(), 'time64(ns)')
  assert.equal(
    fields.timestamp('event', 'us', 'Custom/Accepted').dataType.toString(),
    'timestamp(us,"Custom/Accepted")',
  )

  assert.throws(() => fields.timestamp('event', 'year_month'))
  assert.throws(() => fields.interval('window', 'us'))
  assert.throws(() => fields.time('clock', 'day_time'))
  assert.throws(() => fields.time('clock'))
  assert.throws(() => fields.decimal32('amount', 10))
  assert.throws(() =>
    fields.runEndEncoded(
      'encoded',
      fields.int16('run_ends', { nullable: true }),
      fields.utf8('values'),
    ),
  )
})

test('defaulted temporal and decimal overloads share exact option handling', () => {
  const options = {
    nullable: true,
    metadata: { source: 'overload' },
  }
  for (const [factory, explicit] of [
    [fields.time32, 'millisecond'],
    [fields.time64, 'microsecond'],
    [fields.duration, 'microsecond'],
    [fields.interval, 'month_day_nano'],
  ]) {
    const shorthand = factory('value', options)
    const expanded = factory('value', explicit, options)
    assert.ok(shorthand.equals(expanded))
    assert.equal(shorthand.nullable, true)
    assert.equal(shorthand.get('source'), 'overload')
  }

  for (const [factory, precision] of [
    [fields.decimal, 38],
    [fields.decimal32, 9],
    [fields.decimal64, 18],
    [fields.decimal128, 38],
    [fields.decimal256, 39],
  ]) {
    const shorthand = factory('value', precision, options)
    const expanded = factory('value', precision, 0, options)
    assert.ok(shorthand.equals(expanded))
    assert.equal(shorthand.nullable, true)
    assert.equal(shorthand.get('source'), 'overload')
  }
})

test('schema equality and difference iterators handle recursive metadata', () => {
  const left = fields.struct(
    'row',
    [fields.int32('id', { nullable: false, metadata: { source: 'left' } })],
    { metadata: { root: 'left' } },
  )
  const right = fields.struct(
    'row',
    [fields.int32('id', { nullable: false, metadata: { source: 'right' } })],
    { metadata: { root: 'right' } },
  )

  assert.equal(left.equals(right), false)
  assert.equal(left.equals(right, false), true)
  assert.equal(left.dataType.equals(right.dataType), false)
  assert.equal(left.dataType.equals(right.dataType, false), true)
  assert.equal(left.showDiff(left), '✓ equal')
  assert.equal(left.dataType.showDiff(left.dataType), '✓ equal')

  const differences = left.showDiffs(right)
  assert.equal(typeof differences.next, 'function')
  const lines = [...differences]
  assert.ok(lines.some((line) => line.includes('metadata') && line.includes('≠')))
  assert.ok(lines.every((line) => !line.includes('\u001b')))
  assert.deepEqual([...left.showDiffs(right, false)], [])
})

test('wide difference iterators advance after their source wrappers are dropped', () => {
  const differences = (() => {
    const left = DataType.fromFields(
      Array.from({ length: 1_024 }, (_, index) =>
        fields.int32(`left_${index.toString().padStart(4, '0')}`),
      ),
    )
    const right = DataType.fromFields(
      Array.from({ length: 1_024 }, (_, index) =>
        fields.int32(`right_${index.toString().padStart(4, '0')}`),
      ),
    )
    return left.showDiffs(right, false)
  })()

  // The source wrappers and their temporary child arrays are now GC-eligible.
  // Each call advances only the owned native cursor and keeps it usable.
  const first = differences.next()
  const second = differences.next()
  assert.equal(first.done, false)
  assert.equal(second.done, false)
  assert.match(first.value, /\.fields\[0\]\.name/)
  assert.match(second.value, /\.fields\[1\]\.name/)
})

test('difference output retains physical layout checks without metadata', () => {
  const left = fields.int32('value', { nullable: false })
  const right = fields.int64('value', { nullable: true })
  const lines = [...left.showDiffs(right, false)]

  assert.equal(left.equals(right, false), false)
  assert.ok(lines.some((line) => line.includes('$.nullable')))
  assert.ok(lines.some((line) => line.includes('$.data_type')))
  assert.equal(left.showDiff(right, false), lines.join('\n'))
})
