'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')
const binding = require('yggdryl')
const { DataType, Field, Version, fields } = binding

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
  assert.equal(Object.hasOwn(Field.prototype, '_castArrowArrayIpcNative'), false)
  assert.equal(Object.hasOwn(DataType.prototype, '_showDiffs'), false)
  for (const name of ['DifferenceIterator', 'JsDifferenceIterator']) {
    assert.equal(Object.hasOwn(binding, name), false, name)
  }
  assert.equal(new DataType('int32').constructor, DataType)
  assert.equal(new Field('id', 'int32').constructor, Field)
  assert.equal(Object.hasOwn(new DataType('int32').constructor, '_simple'), false)
})

test('the bits reading crosses every same-width pair', () => {
  const bits = { representation: 'bits' }
  const unsigned32 = arrow.vectorFromArray(
    [0, 2 ** 31 - 1, 2 ** 31, 2 ** 32 - 1, null],
    new arrow.Uint32(),
  )
  const signed32 = fields.int32('digest').castArrowArray(unsigned32, bits)
  assert.equal(signed32.type.toString(), 'Int32')
  assert.deepEqual(Array.from(signed32), [0, 2 ** 31 - 1, -(2 ** 31), -1, null])
  assert.deepEqual(
    Array.from(fields.uint32('digest').castArrowArray(signed32, bits)),
    Array.from(unsigned32),
  )

  const unsigned64 = arrow.vectorFromArray(
    [0n, 2n ** 63n - 1n, 2n ** 63n, 2n ** 64n - 1n, null],
    new arrow.Uint64(),
  )
  const signed64 = fields.int64('digest').castArrowArray(unsigned64, bits)
  assert.equal(signed64.type.toString(), 'Int64')
  assert.deepEqual(Array.from(signed64), [0n, 2n ** 63n - 1n, -(2n ** 63n), -1n, null])
  assert.deepEqual(
    Array.from(fields.uint64('digest').castArrowArray(signed64, bits)),
    Array.from(unsigned64),
  )

  // Eight bytes are eight bytes: the integer, its opposite sign and the raw
  // payload are one buffer under three readings, and the chain round-trips.
  const stored = fields.fixedSizeBinary('digest', 8).castArrowArray(unsigned64, bits)
  assert.deepEqual(Array.from(stored.get(3)), new Array(8).fill(255))
  assert.deepEqual(
    Array.from(fields.uint64('digest').castArrowArray(stored, bits)),
    Array.from(unsigned64),
  )

  const empty = fields
    .int64('digest')
    .castArrowArray(arrow.vectorFromArray([], new arrow.Uint64()), bits)
  assert.equal(empty.type.toString(), 'Int64')
  assert.equal(empty.length, 0)

  // The reading says what the bytes mean; nullability still says what an
  // absent value means.
  const required = fields
    .int64('digest', { nullable: false })
    .castArrowArray(arrow.vectorFromArray([null, 2n ** 64n - 1n], new arrow.Uint64()), bits)
  assert.deepEqual(Array.from(required), [0n, -1n])
  assert.throws(
    () =>
      fields
        .int64('digest', { nullable: false })
        .castArrowArray(arrow.vectorFromArray([null], new arrow.Uint64()), {
          ...bits,
          nullability: 'strict',
        }),
    /required Arrow field \$\.digest holds 1 null values/,
  )

  // Four bytes are not eight, so this stays the ordinary numeric widening.
  assert.deepEqual(Array.from(fields.int64('digest').castArrowArray(unsigned32, bits)), [
    0n,
    2n ** 31n - 1n,
    2n ** 31n,
    2n ** 32n - 1n,
    null,
  ])

  assert.throws(() => fields.int32('digest').castArrowArray([0]), /must be an Apache Arrow Vector/)
})

test('DataType.fromFields is the iterable-aware native Struct builder', () => {
  const id = fields.int32('id', {
    nullable: false,
    metadata: { physical: 'int32' },
  })
  const type = DataType.fromFields(
    (function* children() {
      yield id
    })(),
  )

  assert.equal(type.kind, 'nested')
  assert.ok(type.getFieldAt(0).equals(id))
  assert.equal(type.getFieldAt(0).get('physical'), 'int32')
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
  const dtype = DataType.variant(members)

  assert.equal(pulls, 1)
  assert.equal(dtype.kind, 'nested')
  assert.equal(
    dtype.toString(),
    'union(dense,0=field("text",utf8,nullable=false,metadata={}),' +
      '1=field("code",int64,nullable=false,metadata={}))',
  )
  assert.deepEqual(dtype.defaultJSValue(), { typeId: 0, value: '' })
  assert.ok(
    dtype.equals(
      fields.union(
        'holder',
        [
          [0, text],
          [1, code],
        ],
        'dense',
      ).dtype,
    ),
  )
  assert.ok(fields.denseUnion('payload', [text, code], { nullable: false }).dtype.equals(dtype))
  assert.equal(
    DataType.fromString(
      'variant(field("text",utf8,nullable=false,metadata={}),' +
        'field("code",int64,nullable=false,metadata={}))',
    ).toString(),
    dtype.toString(),
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
  assert.equal(fields.duration, undefined)
  const item = fields.int8('item', { nullable: false })
  const entries = fields.struct(
    'entries',
    [fields.utf8('key', { nullable: false }), fields.int64('value')],
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
    ['datetime64', fields.datetime64('value', 'us', 'Europe/Paris')],
    ['date32', fields.date32('value')],
    ['date64', fields.date64('value')],
    ['time32', fields.time32('value', 'ms')],
    ['time64', fields.time64('value', 'ns')],
    ['duration32', fields.duration32('value', 'ms')],
    ['duration64', fields.duration64('value', 'us')],
    ['interval', fields.interval('value', 'month_day_nano')],
    ['binary', fields.binary('value')],
    ['fixed_size_binary', fields.fixedSizeBinary('value', 16)],
    ['large_binary', fields.largeBinary('value')],
    ['binary_view', fields.binaryView('value')],
    // One string datatype, five layouts: the identity is the layout, and
    // the charset-named factories pick a layout and a charset once.
    ['string', fields.utf8('value')],
    ['fixed_string', fields.fixedAscii('value', 4)],
    ['string_view', fields.utf8View('value')],
    ['large_string', fields.largeUtf8('value')],
    ['large_string_view', fields.string('value', { layout: 'large_string_view' })],
    ['country', fields.country('value')],
    ['currency', fields.currency('value')],
    ['mic', fields.mic('value')],
    ['cfi', fields.cfi('value')],
    ['isin', fields.isin('value')],
    ['side', fields.side('value')],
    ['state', fields.state('value')],
    ['timeinforce', fields.timeinforce('value')],
    ['uuid', fields.uuid('value')],
    ['version', fields.version('value')],
    ['url', fields.url('value')],
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
    ['run_end_encoded', fields.runEndEncoded('value', runEnds, values)],
    ['variant', fields.variant('value')],
    ['geometry', fields.geometry('value')],
    ['geography', fields.geography('value', 'OGC:CRS84', 'vincenty')],
  ])

  // The factories cover every datatype Arrow has a layout for. `int128` and
  // `uint128` are the two identifiers `Scalar` stores and `DataType` cannot,
  // so no field builds them.
  assert.equal(byId.size, 58)
  assert.deepEqual(
    [...byId.keys()].sort(),
    binding.enums.dataTypeIds.filter((id) => id !== 'int128' && id !== 'uint128').sort(),
  )
  assert.ok([...byId.values()].every((value) => value instanceof Field))
  // Every factory above was called without a nullable option, and the Python
  // factories default the same way, so one declared schema cannot disagree
  // about nullability across the two languages.
  for (const [id, value] of byId) {
    assert.equal(value.nullable, true, id)
  }
  // Canonical display opens with the variant id and appends its parameters.
  // A string is the exception: its identity is the layout, and it renders
  // under the name its charset earns - `utf8` for UTF-8, `ascii` for
  // US-ASCII - with the bound as the parameter.
  const spellings = new Map([
    ['string', 'utf8'],
    ['fixed_string', 'fixed_ascii'],
    ['string_view', 'utf8_view'],
    ['large_string', 'large_utf8'],
    ['large_string_view', 'large_utf8_view'],
  ])
  for (const [id, value] of byId) {
    assert.equal(value.dtype.toString().split(/[(<]/, 1)[0], spellings.get(id) ?? id, id)
  }
  assert.deepEqual(
    new Set([...byId.values()].map((value) => value.dtype.kind)),
    new Set([
      'null',
      'boolean',
      'integer',
      'floating',
      'decimal',
      'temporal',
      'bytes',
      'text',
      'code',
      'nested',
      'geospatial',
      'uuid',
    ]),
  )
})

test('the ascii factories build the variable form and one fixed width', () => {
  const free = fields.ascii('note')
  const currency = fields.fixedAscii('ccy', 3, { nullable: false })

  // Both are the one string datatype in US-ASCII: the variable layout has
  // no width to answer, the fixed layout answers its own.
  assert.equal(free.dtype.id, 'string')
  assert.equal(free.dtype.charset, 'us-ascii')
  assert.equal(free.dtype.fixedByteWidth, null)
  assert.equal(free.nullable, true)
  assert.equal(currency.dtype.id, 'fixed_string')
  assert.equal(currency.dtype.charset, 'us-ascii')
  assert.equal(currency.dtype.fixedByteWidth, 3)
  assert.equal(currency.nullable, false)
  assert.ok(currency.dtype.equals(DataType.fixedAscii(3)))
  assert.ok(free.dtype.equals(DataType.ascii()))
  assert.ok(!free.dtype.equals(fields.utf8('note').dtype))
  assert.equal(fields.fixedAscii('iso', 2).dtype.fixedByteWidth, 2)
  assert.equal(fields.uuid('id').dtype.id, 'uuid')
  assert.equal(fields.uuid('id', { nullable: false }).nullable, false)
  assert.equal(fields.version('release').dtype.id, 'version')
  assert.ok(fields.version('release', { nullable: false }).defaultJSValue().equals(new Version(0)))
  // A fixed width past the packed integer is still storage, so it builds.
  assert.equal(fields.fixedAscii('isin', 64).dtype.fixedByteWidth, 64)
  assert.equal(fields.fixedAscii('code', 12).nullable, true)
  assert.equal(fields.ascii('note').defaultJSValue(), null)
  assert.equal(fields.ascii('note', { nullable: false }).defaultJSValue(), '')
  assert.equal(fields.fixedAscii('ccy', 4).defaultJSValue(), null)
  assert.equal(fields.fixedAscii('ccy', 4, { nullable: false }).defaultJSValue(), '')
  assert.throws(() => fields.fixedAscii('code', 0), /expected a width of at least one byte, got 0/)
  assert.throws(() => fields.fixedUtf8('code', 0), /expected a width of at least one byte, got 0/)
})

test('the string and bytes factories declare the datatype beside the field', () => {
  // The parameter keys build the datatype; every other key is a field
  // option, checked as one.
  const latin = fields.string('note', {
    charset: 'windows-1252',
    max: 32,
    nullable: false,
  })
  assert.equal(latin.dtype.toString(), 'string(windows-1252,32)')
  assert.deepEqual(latin.dtype.stringParameters, {
    layout: 'string',
    charset: 'windows-1252',
    bound: 32,
    max: 32,
  })
  assert.equal(latin.nullable, false)
  assert.ok(fields.string('note').dtype.equals(DataType.utf8()))
  assert.ok(
    fields.string('code', { layout: 'fixed_utf8', fixed: 8 }).dtype.equals(DataType.fixedUtf8(8)),
  )
  assert.equal(
    fields
      .string('code', {
        layout: 'large_string',
        charset: 'us-ascii',
        metadata: { role: 'ticker' },
      })
      .get('role'),
    'ticker',
  )
  assert.throws(
    () => fields.string('note', { fixed: 4 }),
    /expected a maximum on a variable layout/,
  )
  assert.throws(() => fields.string('note', { layout: 'fixed_string' }), /width/)
  assert.throws(() => fields.string('note', 'utf8'), /field options must be a plain object/)

  const blob = fields.bytes('payload', { max: 16, nullable: false })
  assert.equal(blob.dtype.toString(), 'binary(16)')
  assert.deepEqual(blob.dtype.bytesParameters, {
    layout: 'binary',
    bound: 16,
    max: 16,
  })
  assert.equal(blob.nullable, false)
  assert.ok(fields.bytes('payload').dtype.equals(DataType.binary()))
  assert.ok(
    fields
      .bytes('key', { layout: 'fixed_size_binary', fixed: 16 })
      .dtype.equals(DataType.fixedSizeBinary(16)),
  )
  assert.equal(fields.bytes('key', { layout: 'fixed_binary', bound: 16 }).dtype.fixedByteWidth, 16)
  assert.throws(
    () => fields.bytes('payload', { fixed: 4 }),
    /expected a maximum on a variable layout/,
  )
  assert.throws(() => fields.bytes('payload', { bound: 4, max: 4 }), /got more than one/)
})

test('the url factory builds a validated, canonical location column', () => {
  const location = fields.url('location')

  assert.equal(location.dtype.id, 'url')
  assert.equal(location.dtype.kind, 'text')
  assert.equal(location.nullable, true)
  assert.equal(fields.url('location', { nullable: false }).nullable, false)
  assert.equal(fields.url('location', { metadata: { role: 'source' } }).get('role'), 'source')
  const declared = fields.url('location', { nullable: false })
  assert.equal(location.defaultJSValue(), null)
  // A location has no zero, so the non-null column's default is the shortest
  // URL the validator accepts: the filesystem root.
  assert.equal(declared.defaultJSValue(), 'file:///')

  // A location is stored canonically, not as the caller happened to spell it:
  // the scheme and host case fold, percent-escapes take their upper-case
  // spelling, and a bare path is the file URL it names.
  assert.deepEqual(
    Array.from(
      declared.castArrowArray(
        arrow.vectorFromArray(['HTTPS://example.com/a%2fb', '/lake/part.txt'], new arrow.Utf8()),
      ),
    ),
    ['https://example.com/a%2Fb', 'file:///lake/part.txt'],
  )

  // Relative text names no location, so it is refused rather than stored as
  // itself - and the empty string is refused with the rest of it.
  for (const relative of ['./rel', 'example.com/x', '']) {
    assert.throws(
      () => declared.castArrowArray(arrow.vectorFromArray([relative], new arrow.Utf8())),
      /does not read as url/,
      relative,
    )
  }

  // Absence is still absence: a nullable location column holds nulls, which
  // is what an unlocated handle writes instead of an empty string.
  assert.deepEqual(
    Array.from(
      location.castArrowArray(
        arrow.vectorFromArray([null, 'https://example.com/'], new arrow.Utf8()),
      ),
    ),
    [null, 'https://example.com/'],
  )
})

test('the registered codes build their own datatype at their own width', () => {
  // ISO 3166-1 is two letters, ISO 4217 three, ISO 10383 four, ISO 10962 six
  // and ISO 6166 twelve: each factory builds the code, never the ASCII width
  // that would hold the same bytes without the identity.
  const declared = new Map([
    ['country', [fields.country('venue_country'), 2]],
    ['currency', [fields.currency('settlement_ccy'), 3]],
    ['mic', [fields.mic('venue'), 4]],
    ['cfi', [fields.cfi('classification'), 6]],
    ['isin', [fields.isin('instrument'), 12]],
  ])

  for (const [name, [value, width]] of declared) {
    assert.equal(value.dtype.id, name, name)
    assert.equal(value.dtype.fixedByteWidth, width, name)
    assert.equal(value.dtype.kind, 'code', name)
    assert.equal(value.dtype.stringParameters, null, name)
    assert.ok(value.dtype.equals(new DataType(name)), name)
    assert.equal(value.nullable, true, name)
  }

  assert.ok(!fields.currency('ccy').dtype.equals(fields.fixedAscii('ccy', 3).dtype))
  assert.equal(declared.get('country')[0].name, 'venue_country')
  assert.equal(fields.currency('ccy', { nullable: false }).nullable, false)
  assert.equal(fields.mic('venue', { metadata: { source: 'iso' } }).get('source'), 'iso')
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
  const child = values.dtype.getFieldAt(0)

  assert.ok(child.equals(item))
  assert.equal(child.dictionaryId, 42n)
  assert.equal(child.dictionaryIsOrdered, true)
  assert.equal(child.get('logical'), 'status')
  assert.equal(values.get('owner'), 'events')
})

test('metadata entry overlays use last-write-wins without coercion', () => {
  const field = fields.int32('id', {
    metadata: [['source', 'first'], { key: 'source', value: 'last' }, ['owner', 'events']],
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
  assert.equal(fields.decimal('small', 38).dtype.toString(), 'decimal128(38,0)')
  assert.equal(fields.decimal('wide', 39).dtype.toString(), 'decimal256(39,0)')
  assert.equal(fields.time('coarse', 'ms').dtype.toString(), 'time32(ms)')
  assert.equal(fields.time('precise', 'ns').dtype.toString(), 'time64(ns)')
  assert.equal(
    fields.datetime64('event', 'us', 'Custom/Accepted').dtype.toString(),
    'datetime64(us,"Custom/Accepted")',
  )

  assert.throws(() => fields.datetime64('event', 'year_month'))
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
    [fields.duration32, 'millisecond'],
    [fields.duration64, 'microsecond'],
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
  assert.equal(left.dtype.equals(right.dtype), false)
  assert.equal(left.dtype.equals(right.dtype, false), true)
  assert.equal(left.showDiff(left), '✓ equal')
  assert.equal(left.dtype.showDiff(left.dtype), '✓ equal')

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
  assert.ok(lines.some((line) => line.includes('$.dtype')))
  assert.equal(left.showDiff(right, false), lines.join('\n'))
})
