'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { AsciiDictionary, AsciiEnum, DataType, Field } = require('yggdryl')

test('datatype values infer inputs and round-trip canonical strings', () => {
  const type = new DataType('varchar')
  const clonedType = DataType.from(type)

  assert.ok(type.equals(clonedType))
  assert.equal(type.compare(clonedType), 0)
  assert.equal(type.stableHash(), clonedType.stableHash())
  assert.equal(typeof type.stableHash(), 'bigint')
  assert.ok(DataType.fromString(type.toString()).equals(type))
})

test('generic time selects its physical width through native unit parsing', () => {
  // `kind` is the coarse family shared by every temporal variant; the selected
  // physical width (`time32` vs `time64`) is the variant identity and shows up
  // in the canonical display that `fromString` round-trips losslessly.
  for (const [unit, canonical] of [
    ['seconds', 'time32(s)'],
    ['milli seconds', 'time32(ms)'],
    ['microseconds', 'time64(us)'],
    ['\u00b5s', 'time64(us)'],
    ['nanoseconds', 'time64(ns)'],
  ]) {
    const selected = DataType.time(unit)
    assert.equal(selected.kind, 'temporal', unit)
    assert.equal(selected.toString(), canonical, unit)
    assert.ok(DataType.fromString(`time(${unit})`).equals(selected), unit)
  }

  assert.throws(() => DataType.time('year_month'), /temporal resolution/)
  assert.throws(() => DataType.time('fortnight'))
  assert.throws(() => DataType.time())
})

test('bare variant is the self-describing datatype, not the union sugar', () => {
  const variant = DataType.variant()

  assert.equal(variant.id, 'variant')
  assert.equal(variant.kind, 'variant')
  assert.equal(variant.toString(), 'variant')
  assert.ok(new DataType('variant').equals(variant))
  assert.ok(DataType.fromString(variant.toString()).equals(variant))
  // The parenthesis disambiguates: members keep building the dense union.
  assert.equal(DataType.fromString('variant(only:int64)').id, 'union')
})

test('geometry and geography fill and display their shared defaults', () => {
  const geometry = DataType.geometry()

  assert.equal(geometry.id, 'geometry')
  assert.equal(geometry.kind, 'geospatial')
  assert.equal(geometry.toString(), 'geometry')
  assert.ok(DataType.geometry('OGC:CRS84').equals(geometry))
  assert.ok(new DataType('geometry').equals(geometry))

  const projected = DataType.geometry('EPSG:3857')
  assert.equal(projected.toString(), 'geometry("EPSG:3857")')
  assert.ok(DataType.fromString(projected.toString()).equals(projected))

  const geography = DataType.geography()
  assert.equal(geography.id, 'geography')
  assert.equal(geography.kind, 'geospatial')
  assert.equal(geography.toString(), 'geography')
  assert.ok(DataType.geography('OGC:CRS84', 'spherical').equals(geography))

  const vincenty = DataType.geography('OGC:CRS84', 'vincenty')
  assert.equal(vincenty.toString(), 'geography("OGC:CRS84","vincenty")')
  assert.ok(new DataType("geography('OGC:CRS84', 'vincenty')").equals(vincenty))
  assert.ok(DataType.fromString(vincenty.toString()).equals(vincenty))

  assert.throws(
    () => DataType.geometry(''),
    /expected a coordinate reference system/,
  )
  assert.throws(
    () => DataType.geography('OGC:CRS84', 'euclidean'),
    /expected one of spherical/,
  )
})

test('ASCII widths select their storage once and resolve registered names', () => {
  const currency = DataType.ascii(3)

  assert.equal(currency.id, 'ascii24')
  assert.equal(currency.kind, 'string')
  assert.equal(currency.toString(), 'ascii24')
  assert.equal(currency.asciiWidth, 3)
  assert.ok(new DataType('ascii24').equals(currency))
  assert.ok(DataType.from('ascii(3)').equals(currency))
  assert.ok(DataType.from('currency').equals(currency))
  assert.ok(DataType.fromString(currency.toString()).equals(currency))
  // The family constructor selects the width once: the storage is the id.
  assert.equal(DataType.ascii(2).id, 'ascii16')
  assert.equal(DataType.ascii(4).id, 'ascii32')
  assert.equal(DataType.ascii(5).id, 'ascii64')
  assert.equal(DataType.ascii(8).asciiWidth, 8)
  assert.equal(DataType.ascii(12).id, 'ascii96')
  assert.equal(DataType.ascii(16).id, 'ascii128')
  assert.equal(new DataType('ascii96').asciiWidth, 12)
  assert.equal(new DataType('ascii128').asciiWidth, 16)
  assert.equal(new DataType('utf8').asciiWidth, null)

  // A registration is a name over a width, not a type: it displays as the width.
  assert.ok(DataType.fromLogicalName('Currency').equals(currency))
  assert.equal(DataType.fromLogicalName(' currency ').toString(), 'ascii24')
  assert.equal(DataType.fromLogicalName('cfi').toString(), 'ascii64')
  assert.equal(DataType.fromLogicalName('country').toString(), 'ascii16')
  const names = DataType.logicalNames()
  assert.deepEqual(Object.keys(names), ['country', 'currency', 'cfi'])
  assert.ok(names.currency instanceof DataType)
  assert.ok(names.currency.equals(currency))

  assert.throws(
    () => DataType.ascii(17),
    /expected an ASCII width from 1 to 16 bytes, got 17/,
  )
  assert.throws(() => DataType.ascii(0), /got 0/)
  assert.throws(() => DataType.ascii(2.5), /width must be a signed 32-bit integer/)
  assert.throws(() => DataType.fromLogicalName('isin'), /currency/)
  assert.throws(() => DataType.fromString('ascii'))
})

test('recursive datatypes expose fields as a collection', () => {
  const nested = DataType.fromString(
    'struct<id: bigint not null, payload: array<struct<name: string, score: decimal(18, 4)>>>',
  )

  assert.equal(nested.length, 2)
  assert.equal(nested.getFieldAt(0).name, 'id')
  assert.equal(nested.getFieldAt(-1).name, 'payload')
  assert.equal(nested.getField('id').nullable, false)
  assert.equal(nested.getFieldByPath('id').name, 'id')
  assert.equal(nested.getField(1).name, 'payload')
  assert.equal(nested.getField('missing'), null)
  assert.throws(() => nested.field('missing'), /missing/)
  assert.equal(nested.contains('payload'), true)
  assert.equal(nested.contains(nested.getFieldAt(0)), true)
  assert.equal(nested.contains(2), false)
  assert.deepEqual(nested.keys(), ['id', 'payload'])
  assert.deepEqual([...nested].map((field) => field.name), ['id', 'payload'])

  const canonical = nested.toString()
  assert.ok(DataType.fromString(canonical).equals(nested))
})

test('datatype JSON remains structural and native-owned', () => {
  const type = DataType.fromString('map<string, array<decimal(12, 2)>>')
  const typeJson = JSON.parse(JSON.stringify(type))

  assert.ok(DataType.fromJSON(typeJson).equals(type))
})

test('datatype Arrow-compatible input delegates through the native parser', () => {
  const type = DataType.fromString('timestamp[us, UTC]')

  assert.ok(DataType.fromArrow({ toString: () => type.toString() }).equals(type))
  assert.ok(DataType.fromArrow(type).equals(type))
  assert.ok(
    DataType.fromArrow({
      [Symbol.toPrimitive]() {
        throw new Error('generic string coercion must not run')
      },
      toString: () => type.toString(),
    }).equals(type),
  )
  assert.throws(() => DataType.fromArrow({}), /own textual representation/)
  assert.throws(
    () => DataType.fromArrow({ toString: () => 42 }),
    /must return a string/,
  )
})

test('datatype direct structural JSON rejects invalid parameter states', () => {
  const valid = DataType.fromString('list<field("item",int32,nullable=false,metadata={})>')
  assert.ok(DataType.fromJSON(valid.toJSON()).equals(valid))
  assert.throws(() =>
    DataType.fromJSON({ type: 'fixed_size_binary', width: -1 }),
  )
  assert.throws(() =>
    DataType.fromJSON({ type: 'decimal32', precision: 10, scale: 0 }),
  )
})

test('malformed recursive datatypes never use a permissive fallback', () => {
  assert.throws(() => DataType.fromString('struct<a: int64'))
  assert.throws(() => DataType.fromString('decimal(0, 9) trailing'))
})

test('the ASCII vocabulary registers as it encodes and keeps first codes', () => {
  const currencies = new AsciiDictionary('ascii32')

  assert.equal(currencies.push('USD'), 0)
  assert.equal(currencies.push('EUR'), 1)
  assert.equal(currencies.push('USD'), 0)
  assert.equal(currencies.length, 2)
  assert.deepEqual(currencies.values(), ['USD', 'EUR'])
  assert.equal(currencies.get(1), 'EUR')
  assert.equal(currencies.get(2), null)
  assert.equal(currencies.getCode('USD'), 0)
  // Storage pads with trailing NUL, so the padded spelling is the same value.
  assert.equal(currencies.getCode('USD\0'), 0)
  assert.equal(currencies.getCode('JPY'), null)
  assert.equal(currencies.dtype.toString(), 'dictionary(int32,ascii32)')
  assert.equal(currencies.key.toString(), 'int32')
  assert.equal(currencies.valuesDtype.toString(), 'ascii32')
  // The rendered text is the `fromValues` call that rebuilds it.
  assert.equal(
    currencies.toString(),
    'AsciiDictionary.fromValues("ascii32", ["USD", "EUR"], "int32")',
  )
  assert.ok(
    AsciiDictionary.fromValues('ascii32', ['USD', 'EUR'], 'int32').equals(currencies),
  )

  // What the width refuses is refused here, never silently registered.
  assert.throws(() => currencies.push('EURO!'), /ASCII text of at most 4 bytes/)
  assert.throws(() => currencies.push('€UR'), /ASCII text of at most 4 bytes/)
  assert.throws(() => currencies.push('US\0D'), /ASCII text of at most 4 bytes/)
  assert.equal(currencies.length, 2)
  assert.throws(() => new AsciiDictionary('utf8'), /expected an ASCII width/)
  assert.throws(
    () => new AsciiDictionary('ascii32', 'int16'),
    /expected an int32 or int64 key datatype/,
  )
})

test('a seeded ASCII vocabulary is a value carried by its holder', () => {
  const seeded = AsciiDictionary.fromValues('ascii32', ['USD', 'EUR', 'USD'])

  assert.deepEqual(seeded.values(), ['USD', 'EUR'])
  assert.ok(seeded.equals(AsciiDictionary.fromValues(DataType.ascii(4), ['USD', 'EUR'])))
  // Equality is the width, the key type, and the values in order.
  assert.equal(seeded.equals(AsciiDictionary.fromValues('ascii32', ['EUR', 'USD'])), false)
  assert.equal(seeded.equals(AsciiDictionary.fromValues('ascii64', ['USD', 'EUR'])), false)
  const wide = AsciiDictionary.fromValues('ascii32', ['USD', 'EUR'], 'int64')
  assert.equal(seeded.equals(wide), false)
  assert.equal(wide.dtype.toString(), 'dictionary(int64,ascii32)')

  // A clone is a second vocabulary: it grows without touching the first.
  const forked = seeded.clone()
  assert.equal(forked.push('JPY'), 2)
  assert.equal(seeded.length, 2)
  assert.equal(seeded.getCode('JPY'), null)
})

test('the generated enum names each value by the integer it packs into', () => {
  const venues = AsciiDictionary.fromValues('ascii32', ['XNAS', 'n/a', '3M'])
  const Venue = venues.intoEnum('Venue')

  // A member is its value packed big-endian, never its position, so it needs
  // the whole width and crosses as a `bigint`.
  assert.deepEqual({ ...Venue }, { XNAS: 0x584e4153n, N_A: 0x6e2f6100n, _3M: 0x334d0000n })
  assert.equal(Venue.XNAS, new DataType('ascii32').asciiPacked('XNAS'))
  assert.equal(new DataType('ascii32').asciiValue(Venue.N_A), 'n/a')
  assert.equal(Object.isFrozen(Venue), true)
  assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')
  // Name to code only; `values` stays the code to value direction.
  assert.deepEqual(venues.values(), ['XNAS', 'n/a', '3M'])

  // The same rule names one value at a time, for a vocabulary a caller
  // declares member by member rather than generates from a whole listing.
  assert.equal(AsciiDictionary.memberName('n/a'), 'N_A')
  assert.equal(AsciiDictionary.memberName('3M'), '_3M')
  assert.equal(AsciiDictionary.memberName(''), '_')

  // A name that opens and closes with `_` drops its trailing underscores, so
  // both runtimes name the same members.
  const shapes = AsciiDictionary.fromValues('ascii64', ['-a-', '--b--', '-'])
  assert.deepEqual({ ...shapes.intoEnum('Shape') }, {
    _A: 0x2d612d0000000000n,
    __B: 0x2d2d622d2d000000n,
    _: 0x2d00000000000000n,
  })

  // Sixteen bytes name members too, under the whole 128-bit integer.
  const wide = AsciiDictionary.fromValues('ascii128', ['US0378331005']).intoEnum('Wide')
  assert.equal(wide.US0378331005, 0x55533033373833333130303500000000n)

  assert.throws(() => venues.intoEnum(''), /non-empty enum name/)
  assert.throws(() => venues.intoEnum(null), /non-empty enum name/)
  assert.throws(
    () => AsciiDictionary.fromValues('ascii32', ['n/a', 'n-a']).intoEnum('Venue'),
    /both name the member N_A/,
  )
})

test('an ASCII value packs into the integer its storage reads as', () => {
  const ascii32 = new DataType('ascii32')

  assert.equal(ascii32.asciiPacked('USD'), 0x55534400n)
  assert.equal(ascii32.asciiPacked('USD\0'), 0x55534400n)
  assert.equal(ascii32.asciiValue(0x55534400n), 'USD')
  // The order of the integers is the order of the text.
  assert.ok(ascii32.asciiPacked('EUR') < ascii32.asciiPacked('USD'))

  const ascii128 = new DataType('ascii128')
  const isin = ascii128.asciiPacked('US0378331005')
  assert.equal(isin, 0x55533033373833333130303500000000n)
  assert.equal(ascii128.asciiValue(isin), 'US0378331005')

  assert.throws(() => ascii32.asciiPacked('EURO!'), /at most 4 bytes/)
  assert.throws(() => ascii32.asciiValue(-1n), /wider than the width/)
  assert.throws(() => new DataType('utf8').asciiPacked('USD'), /an ASCII width/)
})

test('an enum declares itself onto the field its values name', () => {
  const side = new AsciiEnum('Side', { BUY: 'B', SELL: 'S' })

  assert.equal(side.name, 'Side')
  assert.deepEqual(side.members, { BUY: 'B', SELL: 'S' })
  assert.equal(side.get('BUY'), 'B')
  assert.equal(side.getMember('S'), 'SELL')
  assert.equal(side.length, 2)
  assert.deepEqual(side.intoMembers('ascii32'), { BUY: 0x42000000n, SELL: 0x53000000n })
  assert.deepEqual(side.intoDictionary('ascii32').values(), ['B', 'S'])
  assert.equal(side.toString(), '{"members":{"BUY":"B","SELL":"S"},"name":"Side"}')
  assert.ok(AsciiEnum.fromJson(side.intoJson()).equals(side))

  // The declaration is ordinary field metadata under one reserved key, so
  // every serialization carries it and it reads back as the enum that wrote it.
  const field = new Field('side', 'ascii32', false)
  field.setAsciiEnum(side)
  assert.equal(field.get('field:enum'), side.intoJson())
  assert.ok(Field.fromJSON(field.toJSON()).asciiEnum.equals(side))
  assert.ok(field.removeAsciiEnum().equals(side))
  assert.equal(field.asciiEnum, null)

  // A value the width could not store is refused at the declaration.
  const wide = new AsciiEnum('Venue', { LONG: 'EUREX' })
  assert.throws(() => new Field('venue', 'ascii32').setAsciiEnum(wide), /at most 4 bytes/)
  assert.throws(() => new AsciiEnum('Side', { '': 'B' }), /non-empty member name/)
  assert.throws(() => AsciiEnum.fromJson('[]'), /enum JSON object/)
})

test('an ASCII vocabulary encodes Arrow columns whose codes continue', () => {
  const currencies = new AsciiDictionary('ascii32')
  const column = currencies.intoArrowArray(['USD', null, 'EUR', 'USD'])

  assert.equal(column.length, 4)
  assert.deepEqual(Array.from(column.data[0].values), [0, 0, 1, 0])
  assert.equal(column.get(1), null)
  // The values array is the width's padded FixedSizeBinary storage.
  assert.deepEqual(Array.from(column.get(0)), [...Buffer.from('USD\0', 'latin1')])
  assert.equal(column.type.valueType.byteWidth, 4)

  // The vocabulary grows to the union, so a second column keeps the codes.
  const next = currencies.intoArrowArray(['JPY', 'EUR'])
  assert.deepEqual(Array.from(next.data[0].values), [2, 1])
  assert.deepEqual(currencies.values(), ['USD', 'EUR', 'JPY'])

  // The first column still carries the vocabulary it was encoded with.
  const recovered = AsciiDictionary.fromArrowArray(column)
  assert.deepEqual(recovered.values(), ['USD', 'EUR'])
  assert.equal(recovered.dtype.toString(), 'dictionary(int32,ascii32)')
  assert.ok(recovered.equals(AsciiDictionary.fromValues('ascii32', ['USD', 'EUR'])))

  const wide = new AsciiDictionary('ascii64', 'int64')
  const wideColumn = wide.intoArrowArray(['NASDAQ', null, 'NYSE'])
  const wideBack = AsciiDictionary.fromArrowArray(wideColumn)
  assert.equal(wideBack.key.toString(), 'int64')
  assert.deepEqual(wideBack.values(), ['NASDAQ', 'NYSE'])

  assert.throws(
    () => AsciiDictionary.fromArrowArray(arrow.vectorFromArray(['USD'])),
    /a dictionary array of int32 or int64 keys over an ASCII width/,
  )
  assert.throws(
    () => AsciiDictionary.fromArrowArray('USD'),
    /must be an Apache Arrow Vector/,
  )

  // A refused column registers nothing: the mutation fails atomically.
  assert.throws(() => currencies.intoArrowArray(['GBP', 'EURO!']), /at most 4 bytes/)
  assert.deepEqual(currencies.values(), ['USD', 'EUR', 'JPY'])

  // The copied-IPC bridge stays inside the loader.
  assert.equal(currencies._intoArrowArrayIpcNative, undefined)
  assert.equal(currencies._intoEnumNative, undefined)
  assert.equal(AsciiDictionary._fromArrowArrayIpcNative, undefined)
})
