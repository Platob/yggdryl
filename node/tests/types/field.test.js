'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const {
  DataType,
  Field,
  MediaType,
  MimeType,
  ProtocolField,
  Uri,
  Url,
  fields,
  intoField,
} = require('yggdryl')

test('intoField memoizes one class-level struct field and renames by cloning', () => {
  let calls = 0
  class Quote {
    static get intoStructField() {
      calls += 1
      return fields.struct('Quote', [fields.int64('id')], { nullable: false })
    }
  }

  const root = intoField(Quote)
  assert.strictEqual(intoField(new Quote()), root)
  assert.strictEqual(intoField(root), root)
  assert.equal(calls, 1)

  const renamed = intoField(Quote, 'quote')
  assert.notStrictEqual(renamed, root)
  assert.equal(renamed.name, 'quote')
  assert.equal(root.name, 'Quote')
  assert.equal(intoField(Quote, '').name, '')
  assert.equal(intoField('price: int64').name, 'price')
  assert.throws(() => intoField(Quote, 7), /name must be a string/)
})

test('intoField requires a static getter and validates class roots', () => {
  let getterCalls = 0
  let cached
  class GetterRow {
    static get intoStructField() {
      getterCalls += 1
      cached ??= fields.struct('GetterRow', [fields.int64('id')], {
        nullable: false,
      })
      return cached
    }
  }

  assert.strictEqual(intoField(GetterRow), GetterRow.intoStructField)
  assert.strictEqual(intoField(new GetterRow()), cached)
  // One access resolves the global memo and one is the explicit getter read.
  assert.equal(getterCalls, 2)

  class ScalarRoot {
    static get intoStructField() {
      return fields.int64('id', { nullable: false })
    }
  }
  class NullableRoot {
    static get intoStructField() {
      return fields.struct('row', [])
    }
  }
  class MissingRoot {
    static get intoStructField() {}
  }
  class MethodRoot {
    static intoStructField() {
      return fields.struct('MethodRoot', [], { nullable: false })
    }
  }
  class StoredRoot {}
  StoredRoot.intoStructField = fields.struct('StoredRoot', [], {
    nullable: false,
  })
  assert.throws(() => intoField(ScalarRoot), /non-null native struct Field/)
  assert.throws(() => intoField(NullableRoot), /non-null native struct Field/)
  assert.throws(() => intoField(MissingRoot), /non-null native struct Field/)
  assert.throws(() => intoField(MethodRoot), /must be a static getter/)
  assert.throws(() => intoField(StoredRoot), /must be a static getter/)
  assert.throws(() => intoField(null), /value must be a Field/)
  assert.throws(() => intoField({}), /value must be a Field/)
})

test('field values infer native datatypes and round-trip canonically', () => {
  const type = new DataType('varchar')
  const field = new Field('symbol', type, false, { venue: 'XPAR' })
  const clonedField = Field.from(field)

  assert.ok(field.equals(clonedField))
  assert.equal(field.compare(clonedField), 0)
  assert.equal(field.stableHash(), clonedField.stableHash())
  assert.ok(Field.fromString(field.toString()).equals(field))
})

test('field JSON remains structural and native-owned', () => {
  const field = new Field(
    'prices',
    DataType.fromString('map<string, array<decimal(12, 2)>>'),
    true,
    { doc: 'Observed prices', source: 'feed' },
  )
  const fieldJson = JSON.parse(JSON.stringify(field))

  assert.ok(Field.fromJSON(fieldJson).equals(field))
})

test('field Arrow-compatible input delegates through the native parser', () => {
  const field = new Field(
    'event_time',
    DataType.fromString('timestamp[us, UTC]'),
    false,
  )

  assert.ok(Field.fromArrow({ toString: () => field.toString() }).equals(field))
  assert.ok(Field.fromArrow(field).equals(field))
  assert.throws(() => Field.fromArrow({}), /own textual representation/)
})

test('field metadata provides deterministic Map-like operations', () => {
  const field = new Field('price', 'decimal(18, 6)', false)

  assert.equal(field.size, 0)
  assert.equal(field.get('venue'), null)
  assert.equal(field.has('venue'), false)

  field.set('venue', 'XPAR')
  field.update({ currency: 'EUR', source: 'book' })
  field.update([
    { key: 'precision', value: 'micros' },
    { key: 'venue', value: 'XNAS' },
  ])
  field.update(new Map([['session', 'regular']]))

  assert.equal(field.size, 5)
  assert.equal(field.get('venue'), 'XNAS')
  assert.deepEqual(field.keys(), [
    'currency',
    'precision',
    'session',
    'source',
    'venue',
  ])
  assert.deepEqual(field.values(), [
    'EUR',
    'micros',
    'regular',
    'book',
    'XNAS',
  ])
  assert.deepEqual([...field], [
    ['currency', 'EUR'],
    ['precision', 'micros'],
    ['session', 'regular'],
    ['source', 'book'],
    ['venue', 'XNAS'],
  ])

  assert.equal(field.delete('source'), true)
  assert.equal(field.delete('source'), false)
  field.clear()
  assert.equal(field.size, 0)
})

test('field clones are independent and invalid updates are atomic', () => {
  const original = new Field('side', 'string', false, { enum: 'buy,sell' })
  const clone = original.clone()

  clone.set('source', 'fix')
  assert.equal(original.has('source'), false)
  assert.equal(clone.has('source'), true)

  const before = clone.toString()
  assert.throws(() => clone.update([{ key: '', value: 'invalid' }]))
  assert.equal(clone.toString(), before)
})

test('field HTTP metadata is canonical, typed, and HTTPS-compatible', () => {
  const field = new Field('payload', 'binary', false, {
    'HTTPS:Content-Type': 'Application/JSON; Charset=utf-8',
    'HTTP:Content-Encoding': ' gzip,\tbr ',
    'HTTPS:Content-Length': '00042',
    'HTTPS:Location': '../relative',
  })

  assert.equal(field.contentType, 'Application/JSON; Charset=utf-8')
  assert.equal(field.contentEncoding, ' gzip,\tbr ')
  assert.equal(field.contentLength, 42n)
  assert.ok(field.mimeType.equals(MimeType.JSON))
  assert.ok(
    field.mediaType.equals(
      MediaType.fromParts(MimeType.JSON, [MimeType.GZIP, MimeType.BROTLI]),
    ),
  )
  assert.equal(field.get('HTTPS:CONTENT-TYPE'), field.contentType)
  assert.equal(field.getProperty('https', 'CONTENT-TYPE'), field.contentType)
  assert.deepEqual(field.propertyIter('https'), [
    { key: 'content-encoding', value: ' gzip,\tbr ' },
    { key: 'content-length', value: '42' },
    { key: 'content-type', value: 'Application/JSON; Charset=utf-8' },
    { key: 'location', value: '../relative' },
  ])
  assert.deepEqual(field.keys(), [
    'http:content-encoding',
    'http:content-length',
    'http:content-type',
    'http:location',
  ])
  assert.throws(() => field.httpLocation)

  assert.equal(field.removeProperty('https', 'CONTENT-LENGTH'), '42')
  field.setHttpLocation('https://example.test/data')
  assert.equal(field.httpLocation.toString(), 'https://example.test/data')
  assert.equal(field.removeHttpLocation().toString(), 'https://example.test/data')
})

test('field raw HTTP vocabulary validates values and exact u64 lengths', () => {
  const field = new Field('payload', 'binary')
  const rawValues = {
    Accept: 'application/json',
    AcceptEncoding: 'gzip, br',
    AcceptLanguage: 'en, fr;q=0.8',
    AcceptRanges: 'bytes',
    CacheControl: 'public, max-age=60',
    ContentDisposition: 'attachment; filename="data.json"',
    ContentEncoding: 'unknown-coding',
    ContentLanguage: 'en',
    ContentLocation: '../data.json',
    ContentRange: 'bytes 0-9/10',
    ContentType: 'application/json; charset=utf-8',
    Etag: '"revision-1"',
    Expires: 'Sun, 16 Aug 2026 00:00:00 GMT',
    LastModified: 'Sat, 15 Aug 2026 00:00:00 GMT',
    Range: 'bytes=0-9',
    Vary: 'accept-encoding',
  }
  for (const [name, value] of Object.entries(rawValues)) {
    field[`set${name}`](value)
    const property = name[0].toLowerCase() + name.slice(1)
    assert.equal(field[property], value)
  }
  for (const [name, value] of Object.entries(rawValues).reverse()) {
    assert.equal(field[`remove${name}`](), value)
    const property = name[0].toLowerCase() + name.slice(1)
    assert.equal(field[property], null)
  }

  field.setContentLength(2n ** 64n - 1n)
  assert.equal(field.contentLength, 2n ** 64n - 1n)
  assert.equal(field.removeContentLength(), 2n ** 64n - 1n)
  for (const value of [-1n, 2n ** 64n, 1, '1']) {
    assert.throws(() => field.setContentLength(value))
  }
  for (const value of ['a\rb', 'a\nb', 'a\0b', 'a\x7fb', 'a\x1fb']) {
    assert.throws(() => field.setEtag(value))
  }
  field.setEtag('one\ttwo')
  assert.equal(field.etag, 'one\ttwo')
})

test('field typed media pair updates and malformed removal are atomic', () => {
  const field = new Field('payload', 'binary')
  field.setMediaType(
    MediaType.fromParts(MimeType.CSV, [
      MimeType.GZIP,
      MimeType.COMPRESS,
      MimeType.ZSTD,
    ]),
  )
  assert.equal(field.contentType, 'text/csv')
  assert.equal(field.contentEncoding, 'gzip, compress, zstd')
  const before = JSON.stringify(field)

  assert.throws(() =>
    field.setMediaType(MediaType.fromParts(MimeType.JSON, [MimeType.BZIP2])),
  )
  assert.equal(JSON.stringify(field), before)

  field.setMimeType(MimeType.JSON)
  assert.equal(field.contentType, 'application/json')
  assert.equal(field.contentEncoding, 'gzip, compress, zstd')
  assert.ok(field.removeMimeType().equals(MimeType.JSON))
  assert.equal(field.contentEncoding, 'gzip, compress, zstd')

  field.setContentType('application/json')
  field.setContentEncoding('identity')
  const malformed = JSON.stringify(field)
  assert.throws(() => field.mediaType)
  assert.throws(() => field.removeMediaType())
  assert.equal(JSON.stringify(field), malformed)
})

test('typed names, locations, and protocol properties share Arrow metadata', () => {
  const field = new Field('price', 'decimal(18, 6)', false)

  field.setAlias('close')
  field.setComment('closing price')
  field.setDisplay('Close')
  // Catalog coordinates belong to whichever protocol names them.
  field.setProperty('iceberg', 'table_name', 'bars')
  field.setParquetFieldId(-2147483648)
  field.setLocation(
    Uri.fromString('s3://warehouse/bars/day=2026-08-15/data.parquet'),
  )

  assert.equal(field.alias, 'close')
  assert.equal(field.comment, 'closing price')
  assert.equal(field.display, 'Close')
  // A protocol view falls back to the field's straight key for both.
  assert.equal(field.iceberg.comment, 'closing price')
  assert.equal(field.iceberg.display, 'Close')
  assert.equal(field.getProperty('iceberg', 'table_name'), 'bars')
  assert.equal(field.get('table_name'), null)
  assert.equal(field.parquetFieldId, -2147483648)
  assert.equal(field.get('PARQUET:field_id'), '-2147483648')
  assert.ok(
    field.location.equals(
      Url.fromString('s3://warehouse/bars/day=2026-08-15/data.parquet'),
    ),
  )
  assert.equal(field.get('location'), field.location.toString())

  assert.equal(field.setProperty('POSTGRES', 'type', 'numeric(18,6)'), null)
  assert.equal(field.setProperty('postgres', 'column', 'close'), null)
  assert.equal(field.setProperty('iceberg', 'field-id', '7'), null)
  assert.equal(field.setProperty('fix', 'tag', '44'), null)
  assert.equal(field.setProperty('field', 'role', 'measure'), null)
  assert.equal(field.getProperty('postgres', 'type'), 'numeric(18,6)')
  assert.equal(field.hasProperty('postgres', 'column'), true)
  assert.deepEqual(field.propertyIter('postgres'), [
    { key: 'column', value: 'close' },
    { key: 'type', value: 'numeric(18,6)' },
  ])
  assert.equal(field.get('postgres:type'), 'numeric(18,6)')

  assert.equal(field.setProperty('postgres', 'type', 'decimal'), 'numeric(18,6)')
  assert.equal(field.removeProperty('postgres', 'type'), 'decimal')
  assert.equal(field.removeProperty('postgres', 'type'), null)
  field.clearProperties('postgres')
  assert.deepEqual(field.propertyIter('postgres'), [])
  assert.equal(field.hasProperty('iceberg', 'field-id'), true)

  assert.equal(field.removeAlias(), 'close')
  assert.equal(field.removeComment(), 'closing price')
  assert.equal(field.removeDisplay(), 'Close')
  assert.equal(field.display, null)
  assert.equal(field.removeParquetFieldId(), -2147483648)
  assert.ok(
    field
      .removeLocation()
      .equals(Url.fromString('s3://warehouse/bars/day=2026-08-15/data.parquet')),
  )
  assert.equal(field.location, null)
  assert.equal(field.parquetFieldId, null)
})

test('field ID uses canonical signed int32 Arrow metadata', () => {
  const field = new Field('id', 'int64', false)
  field.set('PARQUET:field_id', '+00017')
  assert.equal(field.parquetFieldId, 17)
  assert.equal(field.get('PARQUET:field_id'), '17')

  field.setParquetFieldId(2147483647)
  assert.equal(Field.fromJSON(field.toJSON()).parquetFieldId, 2147483647)
  assert.equal(Field.fromString(field.toString()).parquetFieldId, 2147483647)

  assert.throws(() => field.set('PARQUET:field_id', '2147483648'))
  assert.throws(() => field.set('PARQUET:field_id', '1.0'))
  assert.throws(() => field.setParquetFieldId(2147483648))
  assert.throws(() => field.setParquetFieldId(-2147483649))
  assert.throws(() => field.setParquetFieldId(1.5))
  assert.throws(() => field.setParquetFieldId(Number.NaN))
  assert.throws(() => field.setParquetFieldId(Number.POSITIVE_INFINITY))
  assert.throws(() => field.setParquetFieldId('17'))
  assert.equal(field.parquetFieldId, 2147483647)
})

test('typed metadata rejects invalid updates atomically', () => {
  const field = new Field('id', 'int64', false, { source: 'feed' })
  const before = field.toString()

  assert.throws(() => field.setAlias(''))
  assert.throws(() => field.setProperty('postgres', '', 'integer'))
  assert.throws(() => field.setProperty('1invalid', 'type', 'integer'))
  assert.throws(() => field.set('location', 'urn:isbn:9780131103627'))
  assert.equal(field.toString(), before)
  assert.equal(field.setProperty('postgres', 'default', ''), null)
  assert.equal(field.getProperty('postgres', 'default'), '')
  assert.equal(field.removeProperty('postgres', 'default'), '')
})

test('dictionary field options remain native value state', () => {
  const field = Field.fromString(
    'field("codes",dictionary(int16,utf8),nullable=true,dictionary_id=42,dictionary_is_ordered=true,metadata={})',
  )

  assert.equal(field.dictionaryId, 42n)
  assert.equal(field.dictionaryIsOrdered, true)
  field.setDictionaryOptions(7n, false)
  assert.equal(field.dictionaryId, 7n)
  assert.equal(field.dictionaryIsOrdered, false)
  assert.ok(Field.fromString(field.toString()).equals(field))

  const wide = Field.fromString(
    'field("wide",dictionary(int16,utf8),nullable=true,dictionary_id=9007199254740993,metadata={})',
  )
  assert.equal(wide.dictionaryId, 9007199254740993n)
  const json = JSON.parse(JSON.stringify(wide))
  assert.equal(json.dictionary_id, '9007199254740993')
  assert.ok(Field.fromJSON(json).equals(wide))
})

test('malformed fields never use a permissive fallback', () => {
  assert.throws(() => Field.fromString('name: list<'))
})

test('a protocol view is a Map over one namespace of bare names', () => {
  const field = new Field('price', 'decimal(18, 6)', false, { doc: 'shared' })
  const iceberg = field.iceberg

  assert.ok(iceberg instanceof ProtocolField)
  assert.throws(() => new ProtocolField(), /no `constructor`/)
  assert.equal(iceberg.scheme, 'iceberg')
  assert.equal(iceberg.prefix, 'iceberg')
  assert.equal(iceberg.key('doc'), 'iceberg:doc')
  assert.equal(iceberg.size, 0)
  assert.equal(iceberg.get('doc'), null)
  assert.equal(iceberg.has('doc'), false)

  iceberg.set('doc', 'closing price')
  iceberg.update({ 'schema-id': '3', 'field-id': '7' })
  iceberg.update([{ key: 'sort-order', value: 'asc' }])
  iceberg.update(new Map([['partition', 'day']]))

  assert.equal(iceberg.size, 5)
  assert.equal(iceberg.has('doc'), true)
  assert.deepEqual(iceberg.keys(), [
    'doc',
    'field-id',
    'partition',
    'schema-id',
    'sort-order',
  ])
  assert.deepEqual(iceberg.values(), ['closing price', '7', 'day', '3', 'asc'])
  assert.deepEqual(iceberg.entries()[0], { key: 'doc', value: 'closing price' })
  assert.deepEqual([...iceberg], [
    ['doc', 'closing price'],
    ['field-id', '7'],
    ['partition', 'day'],
    ['schema-id', '3'],
    ['sort-order', 'asc'],
  ])
  assert.deepEqual(new Map(iceberg).get('field-id'), '7')
  assert.deepEqual(Object.fromEntries(iceberg), iceberg.toJSON())
  assert.equal(JSON.stringify(iceberg), iceberg.toString())
  assert.equal(
    iceberg.toString(),
    '{"doc":"closing price","field-id":"7","partition":"day","schema-id":"3","sort-order":"asc"}',
  )

  assert.equal(iceberg.delete('sort-order'), true)
  assert.equal(iceberg.delete('sort-order'), false)
  assert.equal(iceberg.size, 4)

  iceberg.clear()
  assert.equal(iceberg.size, 0)
  // Clearing one protocol never reaches a key that belongs to no protocol.
  assert.equal(field.get('doc'), 'shared')
  assert.throws(() => iceberg.set('', 'unnamed'))
  assert.throws(() => iceberg.update([{ key: '', value: 'unnamed' }]))
  assert.throws(
    () => iceberg.update('doc'),
    /field metadata must be an object, Map, or entry array/,
  )
  assert.equal(iceberg.size, 0)
})

test('a protocol view stays live on the field it was taken from', () => {
  const field = new Field('price', 'decimal(18, 6)', false)
  const iceberg = field.iceberg

  iceberg.set('doc', 'closing price')
  assert.equal(field.getProperty('iceberg', 'doc'), 'closing price')
  assert.equal(field.get('iceberg:doc'), 'closing price')
  assert.equal(field.has('iceberg:doc'), true)
  assert.deepEqual(field.propertyIter('iceberg'), [
    { key: 'doc', value: 'closing price' },
  ])

  // A write on the field is a read through the view, and the other way round.
  field.setProperty('iceberg', 'field-id', '7')
  assert.equal(iceberg.get('field-id'), '7')
  assert.equal(iceberg.size, 2)
  field.set('iceberg:doc', 'last trade')
  assert.equal(iceberg.get('doc'), 'last trade')

  // Two views of one field are two windows onto the same metadata.
  const same = field.protocol('ICEBERG')
  same.delete('doc')
  assert.equal(iceberg.has('doc'), false)
  assert.equal(field.getProperty('iceberg', 'doc'), null)

  field.clearProperties('iceberg')
  assert.equal(iceberg.size, 0)

  // A clone is its own field, so a view of the original never writes into it.
  iceberg.set('doc', 'closing price')
  const clone = field.clone()
  iceberg.set('doc', 'last trade')
  assert.equal(clone.getProperty('iceberg', 'doc'), 'closing price')
  assert.equal(field.getProperty('iceberg', 'doc'), 'last trade')
})

test('the HTTP protocol view covers HTTPS and is ASCII case-insensitive', () => {
  const field = new Field('payload', 'binary', false, {
    'HTTPS:Content-Type': 'application/json',
  })

  assert.equal(field.http.get('content-type'), 'application/json')
  assert.equal(field.http.get('CONTENT-TYPE'), 'application/json')
  assert.equal(field.http.has('Content-Type'), true)

  // HTTPS shares HTTP's one namespace, so both spell the same prefix.
  const https = field.protocol('HTTPS')
  assert.equal(https.scheme, 'https')
  assert.equal(https.prefix, 'http')
  assert.equal(https.key('content-type'), 'http:content-type')
  assert.equal(https.get('Content-Type'), 'application/json')

  https.set('Cache-Control', 'public, max-age=60')
  assert.equal(field.get('http:cache-control'), 'public, max-age=60')
  assert.equal(field.cacheControl, 'public, max-age=60')
  assert.deepEqual(field.http.keys(), ['cache-control', 'content-type'])
  assert.equal(field.protocol('Http').size, 2)
  assert.equal(https.delete('CACHE-CONTROL'), true)
  assert.equal(field.http.size, 1)
})

test('every well-known protocol has its own live field accessor', () => {
  const protocols = [
    'http',
    'file',
    'urn',
    'postgres',
    'postgresql',
    'mysql',
    'arrow',
    'sql',
    'glue',
    'iceberg',
    'fix',
    'field',
    'digest',
    'identity',
    'partition',
    's3',
    'gs',
    'az',
    'spark',
    'polars',
    'pandas',
  ]
  const field = new Field('price', 'decimal(18, 6)', false)

  // `field` names a child on a schema node, so its property view is the one
  // accessor that is not simply its scheme name.
  const accessors = { field: 'fieldProperties' }

  for (const protocol of protocols) {
    const view = field[accessors[protocol] ?? protocol]
    assert.equal(view.scheme, protocol, protocol)
    assert.equal(view.prefix, protocol, protocol)
    assert.equal(view.key('doc'), `${protocol}:doc`, protocol)
    view.set('doc', protocol)
  }

  assert.equal(field.size, protocols.length)
  assert.deepEqual(
    field.keys(),
    protocols.map((protocol) => `${protocol}:doc`).sort(),
  )
  for (const protocol of protocols) {
    assert.equal(field.getProperty(protocol, 'doc'), protocol, protocol)
    assert.equal(field.protocol(protocol).size, 1, protocol)
  }

  // An unparsable scheme is the core's own message, not a binding invention.
  assert.throws(
    () => field.protocol('1invalid'),
    /invalid scheme expression at byte 0: scheme must start with an ASCII letter/,
  )
})

test('digest roles select effective components and validate atomically', () => {
  const symbol = new Field('symbol', 'utf8', false)
  const price = new Field('price', 'float64', false)
  const holder = new Field('row_digest', 'uint64', false)
  holder.digest.set('role', 'holder')

  const before = holder.digest.entries()
  assert.throws(
    () => holder.digest.update({ note: 'output', role: 'invalid' }),
    /holder or component/,
  )
  assert.deepEqual(holder.digest.entries(), before)
  assert.throws(
    () => new Field('bad', 'uint64', true, { 'digest:role': 'invalid' }),
    /holder or component/,
  )

  const defaults = new Field(
    'row',
    DataType.fromFields([symbol, price, holder]),
    false,
  )
  assert.equal(defaults.hasDigestComponents, false)
  assert.deepEqual(defaults.digestFieldNames(), ['symbol', 'price'])
  assert.equal(defaults.digestFieldLen, 2)
  assert.deepEqual(
    defaults.digestFields().map((child) => child.name),
    ['symbol', 'price'],
  )
  assert.deepEqual(
    [...defaults.onlyDigestFields().dtype].map((child) => child.name),
    ['symbol', 'price'],
  )

  const venue = new Field('venue', 'utf8', false)
  venue.digest.set('role', 'component')
  const explicit = new Field(
    'row',
    DataType.fromFields([symbol, venue, price, holder]),
    false,
  )
  assert.equal(explicit.hasDigestComponents, true)
  assert.deepEqual(explicit.digestFieldNames(), ['venue'])
  assert.equal(explicit.digestFieldLen, 1)
  assert.deepEqual(
    explicit.digestFields().map((child) => child.name),
    ['venue'],
  )
  assert.deepEqual(
    [...explicit.onlyDigestFields().dtype].map((child) => child.name),
    ['venue'],
  )

  const holdersOnly = new Field(
    'row',
    DataType.fromFields([holder]),
    false,
  ).onlyDigestFields()
  assert.deepEqual([...holdersOnly.dtype], [])
  assert.equal(holdersOnly.digestFieldLen, 0)

  assert.deepEqual(symbol.digestFields(), [])
  assert.deepEqual(symbol.digestFieldNames(), [])
  assert.equal(symbol.digestFieldLen, 0)
  assert.equal(symbol.hasDigestComponents, false)
  assert.throws(() => symbol.onlyDigestFields(), /expected a struct root/)
})

test('partition markers name the columns a path spells out', () => {
  const year = new Field('year', 'int32', false)
  assert.equal(year.isPartition, false)

  const marked = year.withPartition(true)
  assert.ok(marked instanceof Field)
  assert.equal(marked.isPartition, true)
  assert.equal(marked.get('field:partition'), 'true')
  // `withPartition` copies, so the field it was called on is unchanged.
  assert.equal(year.isPartition, false)

  year.setPartition(true)
  assert.equal(year.isPartition, true)
  year.setPartition(false)
  assert.equal(year.isPartition, false)
  assert.equal(year.has('field:partition'), false)

  const schema = Field.from(
    'row: struct<year: int32 not null, price: float64 not null> not null',
  )
  assert.equal(schema.hasPartitionFields, false)
  assert.equal(schema.partitionFieldLen, 0)
  assert.deepEqual(schema.partitionFieldNames(), [])
  assert.deepEqual(schema.partitionFields(), [])
  // Subtracting nothing is the field itself.
  assert.ok(schema.withoutPartitionFields().equals(schema))

  const partitioned = schema.withPartitionFields(['year'])
  assert.ok(partitioned instanceof Field)
  assert.equal(partitioned.hasPartitionFields, true)
  assert.equal(partitioned.partitionFieldLen, 1)
  assert.deepEqual(partitioned.partitionFieldNames(), ['year'])

  const [partitionField] = partitioned.partitionFields()
  assert.ok(partitionField instanceof Field)
  assert.equal(partitionField.name, 'year')
  assert.equal(partitionField.isPartition, true)

  const path = partitioned.onlyPartitionFields()
  assert.ok(path instanceof Field)
  assert.equal(path.name, 'row')
  assert.deepEqual([...path.dtype].map((child) => child.name), ['year'])

  const leaf = partitioned.withoutPartitionFields()
  assert.ok(leaf instanceof Field)
  assert.deepEqual([...leaf.dtype].map((child) => child.name), ['price'])

  // A partition column nobody stores is a layout error, not a silent omission.
  assert.throws(
    () => partitioned.withPartitionFields(['session']),
    /expected a column of "row" to partition on, got "session"/,
  )
  assert.throws(() => year.onlyPartitionFields(), /expected a struct root/)
  assert.throws(() => year.withPartitionFields(['year']), /expected a struct root/)
})

test('two schemas merge by widening and unioning their columns', () => {
  // Spelled `not null` on both sides, so the merged column staying required is
  // the merge's doing rather than the parser's default.
  const left = DataType.from('struct<id:int32 not null,venue:utf8 not null>')
  const right = DataType.from('struct<id:int64 not null,price:float64 not null>')

  const merged = left.mergeWith(right)

  assert.equal(merged.length, 3)
  assert.ok(merged.getField('id').dtype.equals(DataType.from('int64')))
  assert.equal(merged.getField('id').nullable, false)

  // A column only one side carries arrives nullable.
  assert.equal(merged.getField('venue').nullable, true)
  assert.equal(merged.getField('price').nullable, true)

  // Order is the receiver's, with additions appended.
  assert.deepEqual(merged.keys(), ['id', 'venue', 'price'])

  // Narrowing meets at the tightest type naming both.
  assert.ok(DataType.from('int32').mergeWith('int64', false).equals(DataType.from('int32')))

  // Null yields, bytes win over text, text wins over numbers.
  assert.ok(DataType.from('null').mergeWith('utf8').equals(DataType.from('utf8')))
  assert.ok(DataType.from('utf8').mergeWith('binary').equals(DataType.from('binary')))
  assert.ok(DataType.from('int64').mergeWith('utf8').equals(DataType.from('utf8')))

  // A pair with no meeting point that is not a re-encoding is refused.
  assert.throws(() => DataType.from('boolean').mergeWith('int64'))
})

test('merging fields carries nullability and unions metadata', () => {
  const held = new Field('price', 'int32', false)
  held.setProperty('iceberg', 'doc', 'held')
  const other = new Field('price', 'int64', true)
  other.setProperty('iceberg', 'doc', 'other')
  other.setProperty('iceberg', 'id', '7')

  const merged = held.mergeWith(other)

  assert.ok(merged.dtype.equals(DataType.from('int64')))
  assert.equal(merged.nullable, true, 'either side being nullable carries over')
  assert.equal(merged.getProperty('iceberg', 'doc'), 'held', 'the receiver wins')
  assert.equal(merged.getProperty('iceberg', 'id'), '7')
})

test('a protocol view merges in place and only adds', () => {
  const source = new Field('price', 'int64')
  source.setProperty('iceberg', 'doc', 'source')
  source.setProperty('iceberg', 'id', '7')

  const target = new Field('price', 'int64')
  target.setProperty('iceberg', 'doc', 'target')
  target.setProperty('glue', 'comment', 'glue')

  target.iceberg.mergeWith(source.iceberg)

  // A name already held keeps its value; a new one arrives.
  assert.equal(target.getProperty('iceberg', 'doc'), 'target')
  assert.equal(target.getProperty('iceberg', 'id'), '7')

  // A scoped merge leaves every other protocol alone.
  assert.equal(target.getProperty('glue', 'comment'), 'glue')
})

test('JSON reads every shape and writes bytes', () => {
  const field = new Field('row', DataType.from('struct<id:int64 not null>'), false)

  const raw = field.toJSONBytes()
  assert.ok(Buffer.isBuffer(raw))

  // `toJSON` and `toJSONBytes` are the same document; only the key order
  // differs, because one goes through a sorted map and the other keeps the
  // struct's declaration order.
  assert.deepEqual(JSON.parse(raw.toString()), JSON.parse(JSON.stringify(field)))

  // Text and objects share one reader; bytes have their own, because napi
  // cannot discriminate a typed array inside a union.
  assert.ok(Field.fromJSON(raw.toString()).equals(field))
  assert.ok(Field.fromJSON(JSON.parse(raw.toString())).equals(field))
  assert.ok(Field.fromJSONBytes(raw).equals(field))
  assert.ok(Field.fromJSONBytes(new Uint8Array(raw)).equals(field))

  // A datatype answers the same.
  const dtype = field.dtype
  assert.ok(DataType.fromJSONBytes(dtype.toJSONBytes()).equals(dtype))
  assert.ok(DataType.fromJSON(JSON.parse(JSON.stringify(dtype))).equals(dtype))
})

test('every format carries the same nested shape', () => {
  const deep = new Field(
    'row',
    DataType.from('struct<levels:list<struct<sym:utf8,px:decimal(18,4)>>,tags:map<utf8,int64>>'),
    false,
  )

  assert.ok(Field.fromJSONBytes(deep.toJSONBytes()).equals(deep))
  assert.ok(Field.fromJSON(JSON.parse(JSON.stringify(deep))).equals(deep))

  // Nesting is carried, not flattened into a string.
  const document = JSON.parse(deep.toJSONBytes().toString())
  const levels = document.dtype.fields[0].dtype
  assert.equal(levels.type, 'list')
  assert.equal(levels.field.dtype.fields[0].name, 'sym')
  assert.equal(document.dtype.fields[1].dtype.type, 'map')
})

test('unnesting flattens structs and exploding reaches inside collections', () => {
  const row = new Field(
    'row',
    DataType.from(
      'struct<id:int64 not null,line:struct<px:float64 not null>,' +
        'levels:list<float64>,tags:map<utf8,int64>>',
    ),
    false,
  )

  const leaves = row.unnestFields()
  assert.deepEqual(
    leaves.map((child) => child.name),
    ['id', 'line.px', 'levels', 'tags'],
  )

  // A leaf under a nullable ancestor is nullable, and a list is a leaf here.
  assert.equal(leaves[0].nullable, false)
  assert.equal(leaves[1].nullable, true)

  // Every name it answers is one the path accessor resolves.
  for (const leaf of leaves) {
    assert.notEqual(row.getFieldByPath(leaf.name), null)
  }

  const exploded = row.explodeFields()
  assert.deepEqual(
    exploded.map((child) => child.name),
    ['id', 'line', 'levels', 'tags'],
  )
  assert.ok(exploded[0].dtype.equals(DataType.from('int64')), 'not a collection')
  assert.ok(exploded[2].dtype.equals(DataType.from('float64')), 'a list answers its item')
  assert.equal(exploded[3].dtype.length, 2, 'a map answers its entries struct')

  // A datatype answers the same, so descending never changes the calls.
  assert.deepEqual(
    row.dtype.unnestFields().map((child) => child.name),
    leaves.map((child) => child.name),
  )
})
