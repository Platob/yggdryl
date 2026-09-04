'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { AsciiEnum, DataType, Field } = require('yggdryl')

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

test('ASCII is one variable form and one fixed width', () => {
  const ascii = new DataType('ascii')

  assert.equal(ascii.id, 'ascii')
  assert.equal(ascii.kind, 'string')
  assert.equal(ascii.toString(), 'ascii')
  // Variable-width ASCII stores the bytes it is given, so it has no width.
  assert.equal(ascii.asciiWidth, null)
  assert.ok(DataType.from('ascii').equals(ascii))
  assert.ok(DataType.fromString(ascii.toString()).equals(ascii))

  const fixed = DataType.ascii(3)

  assert.equal(fixed.id, 'fixed_ascii')
  assert.equal(fixed.kind, 'string')
  assert.equal(fixed.toString(), 'ascii(3)')
  assert.equal(fixed.asciiWidth, 3)
  assert.ok(DataType.from('ascii(3)').equals(fixed))
  assert.ok(DataType.fromString(fixed.toString()).equals(fixed))
  // The width is part of the identity, so two widths are two datatypes and
  // neither is the variable form.
  assert.ok(!DataType.ascii(4).equals(fixed))
  assert.ok(!fixed.equals(ascii))
  // Any width of at least one byte is storable; only the packed integer
  // stops at sixteen bytes.
  assert.equal(DataType.ascii(64).asciiWidth, 64)
  assert.equal(new DataType('utf8').asciiWidth, null)

  // A name folds case, `_`, `-`, and spaces the way the grammar folds them.
  const names = DataType.logicalNames()
  assert.equal(names.price.toString(), 'decimal64(18,8)')
  assert.ok(DataType.from('Price').equals(names.price))
  assert.equal(DataType.fromLogicalName('UTC_Timestamp').toString(), 'timestamp(ns,"UTC")')
  // The base-type spellings the Arrow/SQL grammar owns keep their meaning.
  assert.equal(DataType.from('int').id, 'int32')
  assert.equal(DataType.from('float').id, 'float32')

  assert.throws(
    () => DataType.ascii(0),
    /expected an ASCII width of at least 1 byte, got 0/,
  )
  assert.throws(() => DataType.ascii(2.5), /width must be a signed 32-bit integer/)
  assert.throws(() => DataType.fromLogicalName('isin'), /currency/)
})

test('a registered code is its own datatype over its standard width', () => {
  // Not a name over a width: `currency` is three bytes with an identity, and
  // `ascii(3)` is three bytes without one.
  const currency = new DataType('currency')

  assert.equal(currency.id, 'currency')
  assert.equal(currency.kind, 'string')
  assert.equal(currency.toString(), 'currency')
  assert.equal(currency.asciiWidth, 3)
  assert.ok(!currency.equals(DataType.ascii(3)))
  assert.ok(DataType.from('currency').equals(currency))
  assert.ok(DataType.from(' CURRENCY ').equals(currency))

  for (const [name, width] of [
    ['country', 2],
    ['currency', 3],
    ['mic', 4],
    // Six bytes, which is a width no ASCII variant has.
    ['cfi', 6],
  ]) {
    const dtype = new DataType(name)
    assert.equal(dtype.id, name)
    assert.equal(dtype.asciiWidth, width, name)
    assert.equal(dtype.kind, 'string', name)
    assert.equal(dtype.asciiPacked('A'), BigInt('A'.charCodeAt(0)) << BigInt(8 * (width - 1)))
  }

  // The packed integer is the value's own bytes, exactly as for a width.
  assert.equal(currency.asciiPacked('USD'), DataType.ascii(3).asciiPacked('USD'))
  assert.equal(currency.asciiValue(0x555344n), 'USD')
  assert.throws(() => new DataType('country').asciiPacked('USD'), /at most 2 bytes/)
  assert.throws(() => DataType.fromString('isin'), /unknown datatype/)
})

test('the guid is sixteen bytes spelled as one identifier', () => {
  const guid = new DataType('guid')

  assert.equal(guid.id, 'guid')
  assert.equal(guid.kind, 'guid')
  assert.equal(guid.toString(), 'guid')
  assert.equal(guid.asciiWidth, null)
  // `uuid` is what every other system calls it and parses to the same type.
  assert.ok(DataType.from('uuid').equals(guid))
  assert.ok(DataType.fromString(guid.toString()).equals(guid))

  // The identity is the sixteen bytes; the spelling is a rendering of them.
  const text = '01912d68-783e-7c9a-b1f2-0123456789ab'
  const field = new Field('id', guid, false)
  assert.equal(field.defaultJSValue(), '00000000-0000-0000-0000-000000000000')
  assert.ok(Field.fromJSON(field.toJSON()).equals(field))
  assert.ok(Field.fromString(field.toString()).equals(field))
  assert.equal(new Field('id', 'uuid', false).dtype.id, 'guid')
  void text
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

test('a prebuilt vocabulary names the ISO codes a column carries', () => {
  const prebuilt = AsciiEnum.prebuilt()
  assert.deepEqual(Object.keys(prebuilt).sort(), ['country', 'currency', 'exchange', 'mic'])
  // `exchange` is FIX's name for the ISO 10383 code, so it is one list.
  assert.deepEqual(prebuilt.mic, prebuilt.exchange)

  const countries = AsciiEnum.fromLogicalName('Country')
  assert.equal(countries.name, 'country')
  assert.equal(countries.length, prebuilt.country.length)
  // An ISO code names itself, so the member and its value are one spelling.
  assert.equal(countries.get('FR'), 'FR')
  assert.equal(countries.getMember('FR'), 'FR')
  // A prebuilt listing is a constant, so a second build is the same enum.
  assert.ok(AsciiEnum.fromLogicalName('country').equals(countries))
  // `ZZ` is ISO 3166's user-assigned range, so no member names it.
  assert.equal(countries.get('ZZ'), null)
  // The codes pack under the datatype the name resolved to.
  const codes = countries.intoEnum('country')
  assert.equal(codes.FR, new DataType('country').asciiPacked('FR'))

  // A registered name with no prebuilt listing answers an enum of no members,
  // and one that is no registration at all is refused by the vocabulary.
  assert.equal(AsciiEnum.fromLogicalName('tenor').length, 0)
  assert.throws(() => AsciiEnum.fromLogicalName('isin'), /currency/)
})

test('the generated enum names each value by the integer it packs into', () => {
  const venues = new AsciiEnum('Venue', { XNAS: 'XNAS', N_A: 'n/a', _3M: '3M' })
  const Venue = venues.intoEnum(DataType.ascii(4))

  // A member is its value packed big-endian, never its position, so it needs
  // the whole width and crosses as a `bigint`.
  assert.deepEqual({ ...Venue }, { XNAS: 0x584e4153n, N_A: 0x6e2f6100n, _3M: 0x334d0000n })
  assert.equal(Venue.XNAS, DataType.ascii(4).asciiPacked('XNAS'))
  assert.equal(DataType.ascii(4).asciiValue(Venue.N_A), 'n/a')
  assert.equal(Object.isFrozen(Venue), true)
  assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')
  // Name to code only; `members` stays the name to value direction.
  assert.deepEqual(venues.members, { XNAS: 'XNAS', N_A: 'n/a', _3M: '3M' })

  // The same rule names one value at a time, for a vocabulary a caller
  // declares member by member rather than generates from a whole listing.
  assert.equal(AsciiEnum.memberName('n/a'), 'N_A')
  assert.equal(AsciiEnum.memberName('3M'), '_3M')
  assert.equal(AsciiEnum.memberName(''), '_')

  // Sixteen bytes name members too, under the whole 128-bit integer.
  const wide = new AsciiEnum('Wide', { US0378331005: 'US0378331005' })
  assert.equal(
    wide.intoEnum(DataType.ascii(16)).US0378331005,
    0x55533033373833333130303500000000n,
  )

  // A value the width could not store is refused by the width, and a width
  // wider than the packed integer has no codes at all.
  assert.throws(() => venues.intoEnum(DataType.ascii(2)), /at most 2 bytes/)
  assert.throws(() => venues.intoEnum(DataType.ascii(17)), /at most 16 bytes/)
  assert.throws(() => venues.intoEnum('utf8'), /at most 16 bytes/)
})

test('an ASCII value packs into the integer its storage reads as', () => {
  const code4 = DataType.ascii(4)

  assert.equal(code4.asciiPacked('USD'), 0x55534400n)
  assert.equal(code4.asciiPacked('USD\0'), 0x55534400n)
  assert.equal(code4.asciiValue(0x55534400n), 'USD')
  // The order of the integers is the order of the text.
  assert.ok(code4.asciiPacked('EUR') < code4.asciiPacked('USD'))

  const code16 = DataType.ascii(16)
  const isin = code16.asciiPacked('US0378331005')
  assert.equal(isin, 0x55533033373833333130303500000000n)
  assert.equal(code16.asciiValue(isin), 'US0378331005')

  assert.throws(() => code4.asciiPacked('EURO!'), /at most 4 bytes/)
  assert.throws(() => code4.asciiValue(-1n), /wider than the width/)
  // Variable ASCII has no width, so it has no packed integer either.
  assert.throws(() => new DataType('ascii').asciiPacked('USD'), /at most 16 bytes/)
  assert.throws(() => new DataType('utf8').asciiPacked('USD'), /at most 16 bytes/)
})

test('an enum declares itself onto the field its values name', () => {
  const side = new AsciiEnum('Side', { BUY: 'B', SELL: 'S' })

  assert.equal(side.name, 'Side')
  assert.deepEqual(side.members, { BUY: 'B', SELL: 'S' })
  assert.equal(side.get('BUY'), 'B')
  assert.equal(side.getMember('S'), 'SELL')
  assert.equal(side.length, 2)
  assert.deepEqual(side.intoMembers(DataType.ascii(4)), {
    BUY: 0x42000000n,
    SELL: 0x53000000n,
  })
  assert.equal(side.toString(), '{"members":{"BUY":"B","SELL":"S"},"name":"Side"}')
  assert.ok(AsciiEnum.fromJson(side.intoJson()).equals(side))

  // The declaration is ordinary field metadata under one reserved key, so
  // every serialization carries it and it reads back as the enum that wrote it.
  const field = new Field('side', DataType.ascii(4), false)
  field.setAsciiEnum(side)
  assert.equal(field.get('field:enum'), side.intoJson())
  assert.ok(Field.fromJSON(field.toJSON()).asciiEnum.equals(side))
  assert.ok(field.removeAsciiEnum().equals(side))
  assert.equal(field.asciiEnum, null)

  // A value the width could not store is refused at the declaration.
  const wide = new AsciiEnum('Venue', { LONG: 'EUREX' })
  assert.throws(
    () => new Field('venue', DataType.ascii(4)).setAsciiEnum(wide),
    /at most 4 bytes/,
  )
  assert.throws(() => new AsciiEnum('Side', { '': 'B' }), /non-empty member name/)
  assert.throws(() => AsciiEnum.fromJson('[]'), /enum JSON object/)
})

