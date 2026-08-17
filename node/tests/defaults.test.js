'use strict'

const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const { join } = require('node:path')
const test = require('node:test')

const { DataType, Field, fields } = require('..')

// Keyed by DataTypeId: the parameter-free identity of one datatype variant.
function datatypeFixtures() {
  // These children are declared non-null so every fixture projects a
  // materialized default instead of a logical null, and because the core
  // requires the entries struct of a Map and its key to be non-null.
  const item = fields.int32('item', { nullable: false })
  const entries = fields.struct(
    'entries',
    [
      fields.utf8('key', { nullable: false }),
      fields.int32('value', { nullable: true }),
    ],
    { nullable: false },
  )
  return new Map([
    ['null', fields.null('value', { nullable: true })],
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
    ['fixed_size_binary', fields.fixedSizeBinary('value', 4)],
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
      fields.runEndEncoded(
        'value',
        // The core rejects a nullable run-ends field outright.
        fields.int16('run_ends', { nullable: false }),
        fields.utf8('values', { nullable: false }),
      ),
    ],
  ])
}

// A datatype id names one variant (`decimal128`); its kind names the coarse
// family that variant belongs to (`decimal`). `DataType.kind` exposes the
// family, whose vocabulary is owned by rust/src/enums/datatype_kind.rs.
const DATATYPE_KINDS = new Map(
  Object.entries({
    null: ['null'],
    boolean: ['boolean'],
    integer: [
      'int8',
      'int16',
      'int32',
      'int64',
      'uint8',
      'uint16',
      'uint32',
      'uint64',
    ],
    floating: ['float16', 'float32', 'float64'],
    decimal: ['decimal32', 'decimal64', 'decimal128', 'decimal256'],
    temporal: [
      'timestamp',
      'date32',
      'date64',
      'time32',
      'time64',
      'duration',
      'interval',
    ],
    binary: ['binary', 'fixed_size_binary', 'large_binary', 'binary_view'],
    string: ['utf8', 'large_utf8', 'utf8_view'],
    list: [
      'list',
      'list_view',
      'fixed_size_list',
      'large_list',
      'large_list_view',
    ],
    struct: ['struct'],
    union: ['union'],
    map: ['map'],
    dictionary: ['dictionary'],
    run_end_encoded: ['run_end_encoded'],
  }).flatMap(([kind, ids]) => ids.map((id) => [id, kind])),
)

test('all 41 datatypes use canonical schema-directed JavaScript defaults', () => {
  const fixtures = datatypeFixtures()
  assert.equal(fixtures.size, 41)

  const defaults = new Map(
    [...fixtures].map(([id, field]) => [id, field.dataType.defaultJSValue()]),
  )

  assert.equal(defaults.get('null'), null)
  assert.equal(defaults.get('boolean'), false)
  for (const id of [
    'int8',
    'int16',
    'int32',
    'uint8',
    'uint16',
    'uint32',
    'float16',
    'float32',
    'float64',
    'date32',
    'time32',
  ]) {
    assert.equal(defaults.get(id), 0, id)
  }
  for (const id of [
    'int64',
    'uint64',
    'timestamp',
    'date64',
    'time64',
    'duration',
    'decimal32',
    'decimal64',
    'decimal128',
    'decimal256',
  ]) {
    assert.equal(defaults.get(id), 0n, id)
  }
  assert.deepEqual(defaults.get('interval'), [0, 0, 0n])
  for (const id of ['binary', 'large_binary', 'binary_view']) {
    assert.deepEqual(defaults.get(id), Buffer.alloc(0), id)
  }
  assert.deepEqual(defaults.get('fixed_size_binary'), Buffer.alloc(4))
  for (const id of ['utf8', 'large_utf8', 'utf8_view', 'dictionary', 'run_end_encoded']) {
    assert.equal(defaults.get(id), '', id)
  }
  for (const id of ['list', 'list_view', 'large_list', 'large_list_view']) {
    assert.deepEqual(defaults.get(id), [], id)
  }
  assert.deepEqual(defaults.get('fixed_size_list'), [0, 0, 0])

  // A struct value is one ordered positional sequence, one slot per child.
  assert.deepEqual(defaults.get('struct'), [0])
  assert.deepEqual(defaults.get('union'), { typeId: 3, value: 0 })
  assert.ok(defaults.get('map') instanceof Map)
  assert.equal(defaults.get('map').size, 0)
})

test('every native hint category matches its schema-directed default projection', () => {
  for (const [id, field] of datatypeFixtures()) {
    const dataType = field.dataType
    const value = dataType.defaultJSValue()
    const hint = dataType.defaultJSHint()
    assert.equal(hint.kind, DATATYPE_KINDS.get(id), id)
    assert.equal(hint.nullable, false)
    assert.equal(
      hint.constructor,
      value === null ? null : Object(value).constructor,
      id,
    )
    assert.equal(hint, dataType.defaultJSHint())
    assert.ok(Object.isFrozen(hint))
  }
})

test('hint-only calls skip default values and exact Field schema caches', () => {
  const packagePath = join(__dirname, '..')
  const script = String.raw`
    'use strict'
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const packagePath = process.argv[1]
    const native = require(path.join(packagePath, 'index.js'))
    const calls = {
      dataHint: 0,
      dataSchema: 0,
      dataValue: 0,
      fieldSchema: 0,
      fieldValue: 0,
    }

    function instrument(prototype, name, counter) {
      const method = prototype[name]
      assert.equal(typeof method, 'function', name)
      Object.defineProperty(prototype, name, {
        configurable: true,
        value(...args) {
          calls[counter] += 1
          return Reflect.apply(method, this, args)
        },
      })
    }

    instrument(native.DataType.prototype, '_defaultJSHintNative', 'dataHint')
    instrument(native.DataType.prototype, '_defaultJSValueNative', 'dataValue')
    instrument(native.DataType.prototype, '_defaultJSValueSchemaNative', 'dataSchema')
    instrument(native.Field.prototype, '_defaultJSValueNative', 'fieldValue')
    instrument(native.Field.prototype, '_defaultJSValueSchemaNative', 'fieldSchema')

    const { fields } = require(path.join(packagePath, 'binding.js'))
    const huge = fields.fixedSizeBinary('huge', 67_108_865, { nullable: true })
    const repeated = fields.fixedSizeList(
      'repeated',
      fields.int32('item'),
      500_000,
      { nullable: true },
    )
    const dictionary = fields.dictionary('dictionary', 'int16', huge.dataType)
    const encoded = fields.runEndEncoded(
      'encoded',
      fields.int16('run_ends', { nullable: false }),
      fields.fixedSizeBinary('values', 67_108_865),
    )
    const nested = fields.struct('nested', [huge])
    const union = fields.union('union', [[7, huge]], 'dense')
    const nothing = fields.null('nothing', { nullable: true })

    const hugeHint = huge.defaultJSHint()
    assert.equal(hugeHint, huge.defaultJSHint())
    assert.equal(hugeHint.constructor, Buffer)
    assert.equal(repeated.defaultJSHint().constructor, Array)
    assert.equal(dictionary.defaultJSHint().constructor, Buffer)
    assert.equal(encoded.defaultJSHint().constructor, Buffer)
    assert.equal(nested.defaultJSHint().constructor, Array)
    assert.equal(union.defaultJSHint().kind, 'union')
    assert.equal(nothing.defaultJSHint().constructor, null)
    assert.deepEqual(calls, {
      dataHint: 7,
      dataSchema: 0,
      dataValue: 0,
      fieldSchema: 0,
      fieldValue: 0,
    })

    const small = fields.int32('small', { nullable: false })
    assert.equal(small.defaultJSValue(), 0)
    assert.equal(calls.fieldSchema, 1)
    assert.equal(calls.fieldValue, 1)
  `
  const child = spawnSync(process.execPath, ['-e', script, packagePath], {
    encoding: 'utf8',
    timeout: 30_000,
  })
  assert.equal(
    child.status,
    0,
    `hint bridge child exited ${child.status}:\n${child.stdout}\n${child.stderr}`,
  )
})

test('Field nullability governs every schema-directed default slot', () => {
  // A factory call that says nothing about nullability produces a nullable
  // field, exactly as the Python factories do, so its default is the logical
  // null rather than a materialized zero.
  assert.equal(fields.int32('id').nullable, true)
  assert.equal(fields.int32('id').defaultJSValue(), null)
  assert.equal(fields.int32('id', { nullable: true }).defaultJSValue(), null)
  assert.equal(fields.int32('id', { nullable: false }).defaultJSValue(), 0)
  assert.equal(fields.null('nothing', { nullable: true }).defaultJSValue(), null)
  assert.throws(
    () => fields.null('nothing', { nullable: false }).defaultJSValue(),
    /non-nullable field has only a logical-null default/,
  )

  const payload = fields.struct(
    'payload',
    [
      fields.int32('id', { nullable: false }),
      fields.utf8('note', { nullable: true, metadata: { child: 'kept' } }),
    ],
    { nullable: false, metadata: { root: 'kept' } },
  )

  // One ordered slot per child, each honoring its own child nullability.
  assert.deepEqual(payload.defaultJSValue(), [0, null])
  // Metadata stays owned by the Field; a projected value never carries it.
  assert.equal(payload.get('root'), 'kept')
  assert.equal(payload.dataType.at(1).get('child'), 'kept')
  // The bare datatype projects the same ordered sequence.
  assert.deepEqual(payload.dataType.defaultJSValue(), [0, null])
})

test('Union and run-end nullable defaults preserve the core logical-null layout', () => {
  const member = fields.utf8('selected', { nullable: true })
  const union = fields.union('choice', [[7, member]], 'dense', {
    nullable: true,
  })
  const run = fields.runEndEncoded(
    'encoded',
    fields.int16('run_ends', { nullable: false }),
    fields.utf8('values', { nullable: true }),
    { nullable: true },
  )

  assert.deepEqual(union.defaultJSValue(), { typeId: 7, value: null })
  assert.equal(run.defaultJSValue(), null)
})

test('nullable Struct Arrow defaults mask uninhabited hidden children', () => {
  const outer = fields.struct(
    'outer',
    [fields.struct('inner', [fields.null('required')])],
    { nullable: true },
  )

  assert.equal(outer.defaultJSValue(), null)
  assert.equal(outer.defaultArrowScalar(), null)
})

test('deep Struct defaults remain bounded nested positional sequences', () => {
  let field = fields.int32('leaf', { nullable: false })
  for (let depth = 0; depth < 32; depth += 1) {
    field = fields.struct(`level_${depth}`, [field], { nullable: false })
  }

  let value = field.defaultJSValue()
  for (let depth = 31; depth > 0; depth -= 1) {
    assert.ok(Array.isArray(value), `level_${depth}`)
    assert.equal(value.length, 1, `level_${depth}`)
    value = value[0]
  }
  assert.deepEqual(value, [0])
})

test('deep collection defaults project every nested Struct slot', () => {
  const item = fields.struct('item', [fields.int32('id', { nullable: false })], {
    nullable: false,
    metadata: { logical: 'item' },
  })
  const fixed = fields.fixedSizeList('items', item, 3, { nullable: false })
  const first = fixed.defaultJSValue()
  const second = fixed.defaultJSValue()

  assert.deepEqual(first, [[0], [0], [0]])
  assert.deepEqual(second, first)
  assert.notEqual(first, second)
  assert.notEqual(first[0], second[0])
  assert.equal(fixed.dataType.at(0).get('logical'), 'item')
})

test('mutable default containers are fresh on every projection', () => {
  const binary = fields.fixedSizeBinary('data', 2).dataType
  const list = fields.list('items', fields.int32('item')).dataType
  const map = fields.mapOf('mapping', 'utf8', 'int32').dataType
  const struct = fields.struct('payload', [
    fields.int32('id', { nullable: false }),
  ]).dataType

  const firstBinary = binary.defaultJSValue()
  const secondBinary = binary.defaultJSValue()
  const firstList = list.defaultJSValue()
  const secondList = list.defaultJSValue()
  const firstMap = map.defaultJSValue()
  const secondMap = map.defaultJSValue()
  assert.notEqual(firstBinary, secondBinary)
  assert.notEqual(firstList, secondList)
  assert.notEqual(firstMap, secondMap)
  firstBinary[0] = 9
  firstList.push(1)
  firstMap.set('changed', 1)
  assert.deepEqual(secondBinary, Buffer.alloc(2))
  assert.deepEqual(secondList, [])
  assert.equal(secondMap.size, 0)

  const first = struct.defaultJSValue()
  const second = struct.defaultJSValue()
  assert.notEqual(first, second)
  first.push(1)
  assert.deepEqual(second, [0])
})

test('defaultJSHint is frozen, cached, metadata-independent, and mutation-aware', () => {
  const dataType = fields.int64('id').dataType
  const dataTypeHint = dataType.defaultJSHint()
  assert.equal(dataTypeHint, dataType.defaultJSHint())
  assert.deepEqual(dataTypeHint, {
    kind: 'integer',
    constructor: BigInt,
    nullable: false,
  })
  assert.deepEqual(Object.keys(dataTypeHint), ['kind', 'constructor', 'nullable'])
  assert.ok(Object.isFrozen(dataTypeHint))

  // Declared non-null so the later setNullable(true) is a real transition and
  // the cached hint has to be invalidated.
  const field = fields.int32('id', {
    nullable: false,
    metadata: { source: 'one' },
  })
  const original = field.defaultJSHint()
  field.set('source', 'two')
  field.setName('renamed')
  assert.equal(field.defaultJSHint(), original)

  field.setNullable(true)
  const nullable = field.defaultJSHint()
  assert.notEqual(nullable, original)
  assert.equal(nullable.nullable, true)
  assert.equal(nullable.constructor, Number)

  field.setDataType('utf8')
  const changed = field.defaultJSHint()
  assert.notEqual(changed, nullable)
  assert.equal(changed.kind, 'string')
  assert.equal(changed.constructor, String)
  assert.equal(field.defaultJSValue(), null)
  field.setNullable(false)
  assert.equal(field.defaultJSValue(), '')

  const struct = fields.struct('payload', [
    fields.int32('id', { nullable: false }),
  ]).dataType
  const structHint = struct.defaultJSHint()
  assert.equal(structHint.kind, 'struct')
  assert.equal(structHint.constructor, Array)
  assert.deepEqual(struct.defaultJSValue(), [0])
})

test('nested Field metadata never changes a cached Struct default hint', () => {
  const field = fields.struct(
    'payload',
    [fields.int32('id', { nullable: false, metadata: { source: 'one' } })],
    { nullable: false },
  )
  const hint = field.defaultJSHint()
  assert.equal(hint.kind, 'struct')
  assert.equal(hint.constructor, Array)

  field.setDataType(
    DataType.fromFields([
      fields.int32('id', { nullable: false, metadata: { source: 'two' } }),
    ]),
  )
  assert.equal(field.defaultJSHint(), hint)
  assert.equal(field.dataType.at(0).get('source'), 'two')
  assert.deepEqual(field.defaultJSValue(), [0])

  // A wrapper reports the category of the value it encodes.
  const dictionary = fields.dictionary('logical', 'int8', field.dataType)
  const run = fields.runEndEncoded(
    'run',
    fields.int16('run_ends', { nullable: false }),
    fields.struct('values', [
      fields.int32('id', { metadata: { source: 'three' } }),
    ]),
  )
  assert.equal(dictionary.defaultJSHint().constructor, Array)
  assert.equal(run.defaultJSHint().constructor, Array)
})

test('Struct hint caches never contaminate exact Field default projections', () => {
  const left = fields.struct('left', [fields.int32('id', { nullable: false })], {
    nullable: false,
    metadata: { owner: 'left' },
  })
  const right = fields.struct(
    'right',
    [fields.int32('id', { nullable: false })],
    { nullable: false, metadata: { owner: 'right' } },
  )

  // Exercise both orderings: a hint before an exact default and an exact
  // default before its metadata-independent hint.
  const leftHint = left.defaultJSHint()
  const leftValue = left.defaultJSValue()
  const rightValue = right.defaultJSValue()
  const rightHint = right.defaultJSHint()

  assert.deepEqual(leftValue, [0])
  assert.deepEqual(rightValue, [0])
  assert.notEqual(leftValue, rightValue)
  // A hint describes layout only, so two differently annotated Structs of one
  // layout report the same frozen category.
  assert.deepEqual(leftHint, rightHint)

  left.set('owner', 'changed')
  right.setName('renamed')
  assert.equal(left.defaultJSHint(), leftHint)
  assert.equal(right.defaultJSHint(), rightHint)
  assert.equal(left.get('owner'), 'changed')
  assert.equal(right.name, 'renamed')
  // The exact Field default is recomputed after mutation and keeps projecting
  // the complete positional layout.
  assert.deepEqual(left.defaultJSValue(), [0])
  assert.deepEqual(right.defaultJSValue(), [0])
})

test('default Arrow scalar materialization is typed and rejects Arrow-JS gaps', () => {
  // Every field below is declared non-null, because a nullable field has the
  // logical null as its Arrow default and would never reach materialization.
  const required = { nullable: false }
  assert.equal(fields.int32('id').dataType.defaultArrowScalar(), 0)
  assert.equal(fields.utf8('note', { nullable: true }).defaultArrowScalar(), null)
  assert.deepEqual(
    fields.binaryView('binary_view', required).defaultArrowScalar(),
    new Uint8Array(),
  )
  assert.equal(fields.utf8View('utf8_view', required).defaultArrowScalar(), '')
  assert.equal(
    fields.largeList('large_list', fields.int32('item'), required)
      .defaultArrowScalar().length,
    0,
  )

  const payload = fields.struct(
    'payload',
    [fields.int32('id', required)],
    required,
  )
  assert.equal(payload.defaultArrowScalar().id, 0)
  assert.equal(
    fields.dictionary('category', 'int8', 'utf8', required).defaultArrowScalar(),
    '',
  )
  assert.equal(
    fields.union('choice', [[3, fields.int32('member', required)]], 'dense', required)
      .defaultArrowScalar(),
    0,
  )
  assert.equal(
    fields.list('items', fields.int32('item'), required).defaultArrowScalar().length,
    0,
  )
  assert.notEqual(fields.decimal256('wide', 76, required).defaultArrowScalar(), null)
  assert.notEqual(fields.struct('empty', [], required).defaultArrowScalar(), null)
  assert.equal(
    fields.fixedSizeList('empty', fields.int32('item'), 0, required)
      .defaultArrowScalar().length,
    0,
  )
  assert.equal(fields.null('nothing', { nullable: true }).defaultArrowScalar(), null)
  assert.equal(fields.null('nothing').dataType.defaultArrowScalar(), null)
  assert.throws(
    () => fields.null('nothing', required).defaultArrowScalar(),
    /non-nullable field has only a logical-null default/,
  )

  const unsupported = [
    fields.listView('list_view', fields.int32('item')),
    fields.largeListView('large_list_view', fields.int32('item')),
    fields.runEndEncoded(
      'run_end_encoded',
      fields.int16('run_ends', { nullable: false }),
      fields.utf8('values'),
    ),
  ]
  for (const field of unsupported) {
    assert.throws(
      () => field.defaultArrowScalar(),
      /Apache Arrow JS cannot materialize .* unsupported/,
      field.name,
    )
  }
})

test('compatibility normalization mirrors core Arrow and conservative Spark policy', () => {
  const source = fields.struct(
    'payload',
    [
      fields.uint8('small'),
      fields.largeUtf8('text'),
      fields.listView('items', fields.float16('item')),
      fields.dictionary('category', 'int8', 'utf8'),
    ],
    { nullable: true, metadata: { owner: 'events' } },
  )

  const arrow = source.toSchemeCompat('arrow')
  assert.ok(arrow.equals(source))
  assert.notEqual(arrow, source)

  const spark = source.toSchemeCompat('spark')
  assert.equal(spark.name, 'payload')
  assert.equal(spark.nullable, true)
  assert.equal(spark.get('owner'), 'events')
  // `kind` is the coarse family; the canonical display names the exact variant.
  assert.deepEqual(spark.dataType.values().map((field) => field.dataType.kind), [
    'integer',
    'string',
    'list',
    'string',
  ])
  assert.deepEqual(
    [0, 1, 3].map((index) => String(spark.dataType.at(index).dataType)),
    ['int16', 'utf8', 'utf8'],
  )
  assert.equal(String(spark.dataType.at(2).dataType.at(0).dataType), 'float32')

  assert.ok(
    fields.uint64('wide').dataType.toSchemeCompat('spark').equals(
      fields.decimal128('expected', 20, 0).dataType,
    ),
  )
  assert.throws(
    () => fields.timestamp('created', 'nanosecond').toSchemeCompat('spark'),
    /expected timestamp of us, got ns.*not a schema normalization/,
  )
  const extension = fields.largeUtf8('logical', {
    metadata: { 'ARROW:extension:name': 'example.logical' },
  })
  assert.throws(
    () => extension.toSchemeCompat('spark'),
    /would relabel Arrow extension storage/,
  )
  assert.throws(
    () => source.toSchemeCompat('postgres'),
    /expected one of arrow, spark, polars, pandas, iceberg, got "postgres"/,
  )
})

test('public defaults expose no private native bridge names', () => {
  for (const prototype of [DataType.prototype, Field.prototype]) {
    assert.equal('_defaultJSValueNative' in prototype, false)
    assert.equal('_defaultJSValueSchemaNative' in prototype, false)
    assert.equal('_defaultJSHintNative' in prototype, false)
    assert.equal('_defaultArrowScalarIpcNative' in prototype, false)
  }
})
