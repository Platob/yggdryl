'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { DataType, Field, StringEnum, Version } = require('yggdryl')

test('datatype values infer inputs and round-trip canonical strings', () => {
  const type = new DataType('varchar')
  const clonedType = DataType.from(type)

  assert.ok(type.equals(clonedType))
  assert.equal(type.compare(clonedType), 0)
  assert.equal(type.stableHash(), clonedType.stableHash())
  assert.equal(typeof type.stableHash(), 'bigint')
  assert.ok(DataType.fromString(type.toString()).equals(type))
})

test('named regex captures define a nullable Struct before data is read', () => {
  const inferred = DataType.fromRegex('\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)')
  assert.deepEqual(
    [...inferred].map((field) => field.name),
    ['level', 'id'],
  )
  assert.equal(inferred.field('level').dtype.id, 'string')
  assert.equal(inferred.field('id').dtype.id, 'int64')
  assert.ok([...inferred].every((field) => field.nullable))

  const strings = DataType.fromRegex('(?<id>\\d+)', false)
  assert.equal(strings.field('id').dtype.id, 'string')
  assert.throws(() => DataType.fromRegex('(?<id>'), /regular expression/)
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
  assert.equal(variant.kind, 'nested')
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

  assert.throws(() => DataType.geometry(''), /expected a coordinate reference system/)
  assert.throws(() => DataType.geography('OGC:CRS84', 'euclidean'), /expected one of spherical/)
})

test('every string is one datatype: a layout, a charset, and a bound', () => {
  // The five layouts, each under the name its UTF-8 charset earns.
  for (const [dtype, layout, spelling] of [
    [DataType.utf8(), 'string', 'utf8'],
    [DataType.largeUtf8(), 'large_string', 'large_utf8'],
    [DataType.utf8View(), 'string_view', 'utf8_view'],
    [DataType.fixedUtf8(8), 'fixed_string', 'fixed_utf8(8)'],
    [DataType.string({ layout: 'large_string_view' }), 'large_string_view', 'large_utf8_view'],
  ]) {
    assert.equal(dtype.id, layout, spelling)
    assert.equal(dtype.kind, 'text', spelling)
    assert.equal(dtype.toString(), spelling)
    assert.equal(dtype.charset, 'utf-8', spelling)
    assert.equal(dtype.stringParameters.layout, layout, spelling)
    assert.equal(dtype.stringParameters.charset, 'utf-8', spelling)
    assert.ok(DataType.from(spelling).equals(dtype), spelling)
    assert.ok(DataType.fromJSON(dtype.toJSON()).equals(dtype), spelling)
  }
  assert.ok(
    DataType.from('large_string_view').equals(DataType.string({ layout: 'large_string_view' })),
  )
  assert.ok(DataType.string().equals(DataType.utf8()))
  assert.deepEqual(DataType.utf8().stringParameters, {
    layout: 'string',
    charset: 'utf-8',
  })
  // The SQL spellings are spellings of the same datatype.
  assert.ok(DataType.from('varchar').equals(DataType.utf8()))
  assert.equal(DataType.from('varchar(32)').toString(), 'utf8(32)')
  assert.equal(DataType.from('char(8)').toString(), 'fixed_utf8(8)')

  // A charset or a bound is what a string declares, and the general name
  // states a charset the short names do not cover.
  const latin = DataType.string({ charset: 'windows-1252', max: 32 })
  assert.equal(latin.id, 'string')
  assert.equal(latin.toString(), 'string(windows-1252,32)')
  assert.equal(latin.charset, 'windows-1252')
  assert.deepEqual(latin.stringParameters, {
    layout: 'string',
    charset: 'windows-1252',
    bound: 32,
    max: 32,
  })
  assert.equal(latin.fixedByteWidth, null)
  assert.ok(DataType.from(latin.toString()).equals(latin))
  assert.ok(DataType.string({ charset: 'cp1252', bound: 32 }).equals(latin))
  assert.deepEqual(latin.toJSON(), {
    type: 'string',
    charset: 'windows-1252',
    max: 32,
  })
  assert.ok(DataType.fromJSONBytes(latin.toJSONBytes()).equals(latin))
  // The bound is one number: the exact width on the fixed layout, the
  // maximum elsewhere, and `bound` follows the layout.
  const fixed = DataType.string({
    layout: 'fixed_string',
    charset: 'us-ascii',
    bound: 4,
  })
  assert.ok(fixed.equals(DataType.fixedAscii(4)))
  assert.ok(
    DataType.string({
      layout: 'fixed_string',
      charset: 'us-ascii',
      fixed: 4,
    }).equals(fixed),
  )
  // A US-ASCII spelling names a charset a bare layout would drop, so the
  // layout key refuses it: the charset is its own key.
  assert.throws(() => DataType.string({ layout: 'fixed_ascii', fixed: 4 }), /us-ascii/)
  assert.deepEqual(fixed.stringParameters, {
    layout: 'fixed_string',
    charset: 'us-ascii',
    bound: 4,
    fixed: 4,
  })
  assert.equal(fixed.fixedByteWidth, 4)
  assert.equal(DataType.string({ max: 3 }).toString(), 'utf8(3)')
  assert.equal(DataType.from('utf8(3)').stringParameters.max, 3)

  // A datatype that is not a string answers none of this.
  assert.equal(new DataType('int32').stringParameters, null)
  assert.equal(new DataType('int32').charset, null)
  assert.equal(new DataType('currency').stringParameters, null)
  assert.equal(new DataType('currency').charset, null)

  // A reading the layout does not take, two readings, a width of nothing,
  // and a fixed layout with no width are each refused.
  assert.throws(() => DataType.string({ fixed: 4 }), /expected a maximum on a variable layout/)
  assert.throws(
    () => DataType.string({ layout: 'fixed_string', max: 4 }),
    /expected a fixed width on a fixed layout/,
  )
  assert.throws(() => DataType.string({ bound: 4, max: 4 }), /got more than one/)
  assert.throws(() => DataType.string({ bound: 0 }), /at least one byte, got 0/)
  assert.throws(() => DataType.string({ layout: 'fixed_string' }), /got no width/)
  assert.throws(() => DataType.string({ charset: 'utf-16' }), /invalid charset/)
  assert.throws(() => DataType.string({ layout: 'blob' }), /unknown datatype/)
  assert.throws(() => DataType.fixedUtf8(0), /at least one byte, got 0/)
  assert.throws(() => DataType.fixedUtf8(2.5), /width must be an unsigned 32-bit integer/)
  // The retired tags are no longer read back.
  assert.throws(() => DataType.fromJSON({ type: 'utf8' }), /unknown variant `utf8`/)
})

test('a string in a declared charset reads back from its structural JSON', () => {
  // One `string` tag carries the charset; the parsed document and the bytes
  // it came from are the same door, so both read it back.
  const ascii = DataType.fixedAscii(4)
  assert.deepEqual(ascii.toJSON(), {
    type: 'string',
    layout: 'fixed_string',
    charset: 'us-ascii',
    fixed: 4,
  })
  assert.ok(DataType.fromJSON(ascii.toJSON()).equals(ascii))
  const field = new Field('ccy', ascii, false)
  assert.ok(Field.fromJSON(field.toJSON()).equals(field))
  const latin = DataType.string({ charset: 'windows-1252', max: 32 })
  assert.ok(DataType.fromJSON(latin.toJSON()).equals(latin))
})

test('ASCII is one variable form and one fixed width', () => {
  const ascii = DataType.ascii()

  assert.equal(ascii.id, 'string')
  assert.equal(ascii.kind, 'text')
  assert.equal(ascii.toString(), 'ascii')
  assert.equal(ascii.charset, 'us-ascii')
  // Variable-width ASCII stores the bytes it is given, so it has no width.
  assert.equal(ascii.fixedByteWidth, null)
  assert.deepEqual(ascii.stringParameters, {
    layout: 'string',
    charset: 'us-ascii',
  })
  assert.ok(DataType.from('ascii').equals(ascii))
  assert.ok(DataType.fromString(ascii.toString()).equals(ascii))
  assert.ok(DataType.string({ charset: 'us-ascii' }).equals(ascii))
  // `ascii(4)` is a maximum; the fixed four-byte column is `fixed_ascii(4)`.
  assert.equal(DataType.from('ascii(4)').stringParameters.max, 4)
  assert.equal(DataType.from('ascii(4)').fixedByteWidth, null)

  const fixed = DataType.fixedAscii(3)

  assert.equal(fixed.id, 'fixed_string')
  assert.equal(fixed.kind, 'text')
  assert.equal(fixed.toString(), 'fixed_ascii(3)')
  assert.equal(fixed.fixedByteWidth, 3)
  assert.equal(fixed.charset, 'us-ascii')
  assert.ok(DataType.from('fixed_ascii(3)').equals(fixed))
  assert.ok(DataType.fromString(fixed.toString()).equals(fixed))
  assert.ok(DataType.fromJSONBytes(fixed.toJSONBytes()).equals(fixed))
  // The width is part of the identity, so two widths are two datatypes and
  // neither is the variable form.
  assert.ok(!DataType.fixedAscii(4).equals(fixed))
  assert.ok(!fixed.equals(ascii))
  assert.ok(!fixed.equals(DataType.fixedUtf8(3)))
  // Any width of at least one byte is storable; only the packed integer
  // stops at sixteen bytes.
  assert.equal(DataType.fixedAscii(64).fixedByteWidth, 64)
  assert.equal(DataType.utf8().fixedByteWidth, null)

  // A name folds case, `_`, `-`, and spaces the way the grammar folds them.
  const names = DataType.logicalNames()
  assert.equal(names.price.toString(), 'float64')
  assert.ok(DataType.from('Price').equals(names.price))
  assert.equal(DataType.fromLogicalName('UTC_Timestamp').toString(), 'datetime64(ns,"UTC")')
  assert.equal(DataType.fromLogicalName('language').toString(), 'fixed_ascii(2)')
  // The base-type spellings the Arrow/SQL grammar owns keep their meaning.
  assert.equal(DataType.from('int').id, 'int32')
  assert.equal(DataType.from('float').id, 'float32')

  assert.throws(() => DataType.fixedAscii(0), /expected a width of at least one byte, got 0/)
  assert.throws(() => DataType.fixedAscii(2.5), /width must be an unsigned 32-bit integer/)
  assert.throws(() => DataType.fromLogicalName('sedol'), /currency/)
})

test('every byte column is one datatype: a layout and a bound', () => {
  for (const [dtype, layout] of [
    [DataType.binary(), 'binary'],
    [DataType.largeBinary(), 'large_binary'],
    [DataType.binaryView(), 'binary_view'],
    [DataType.fixedSizeBinary(16), 'fixed_size_binary'],
  ]) {
    assert.equal(dtype.id, layout)
    assert.equal(dtype.kind, 'bytes')
    assert.equal(dtype.bytesParameters.layout, layout)
    assert.equal(dtype.stringParameters, null)
    assert.ok(
      DataType.bytes({
        layout,
        ...(dtype.fixedByteWidth ? { fixed: 16 } : {}),
      }).equals(dtype),
    )
    assert.ok(DataType.from(dtype.toString()).equals(dtype))
    assert.ok(DataType.fromJSON(dtype.toJSON()).equals(dtype))
  }
  assert.ok(DataType.bytes().equals(DataType.binary()))
  assert.deepEqual(DataType.binary().bytesParameters, { layout: 'binary' })
  assert.equal(DataType.fixedSizeBinary(16).toString(), 'fixed_size_binary(16)')
  assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)
  assert.deepEqual(DataType.fixedSizeBinary(16).bytesParameters, {
    layout: 'fixed_size_binary',
    bound: 16,
    fixed: 16,
  })
  assert.ok(
    DataType.bytes({ layout: 'fixed_binary', bound: 16 }).equals(DataType.fixedSizeBinary(16)),
  )
  // `binary(16)` is a maximum of sixteen bytes; the fixed slot is the width.
  const bounded = DataType.bytes({ max: 16 })
  assert.equal(bounded.toString(), 'binary(16)')
  assert.equal(bounded.fixedByteWidth, null)
  assert.deepEqual(bounded.bytesParameters, {
    layout: 'binary',
    bound: 16,
    max: 16,
  })
  assert.ok(DataType.from('varbinary(16)').equals(bounded))
  assert.deepEqual(bounded.toJSON(), { type: 'binary', max: 16 })
  assert.ok(DataType.fromJSON(bounded.toJSON()).equals(bounded))
  assert.ok(DataType.from('bytes').equals(DataType.binary()))
  // A UUID is bytes with an identity, so it is not a byte column.
  assert.equal(new DataType('uuid').bytesParameters, null)

  assert.throws(() => DataType.bytes({ fixed: 4 }), /expected a maximum on a variable layout/)
  assert.throws(() => DataType.bytes({ layout: 'fixed_size_binary' }), /got no width/)
  assert.throws(() => DataType.fixedSizeBinary(0), /at least one byte, got 0/)
  assert.throws(() => DataType.fixedSizeBinary(-1), /byteWidth must be an unsigned 32-bit integer/)
  assert.throws(
    () => DataType.fromJSON({ type: 'fixed_size_binary', width: 16 }),
    /unknown variant `fixed_size_binary`/,
  )
})

test('a registered code is its own datatype over its standard width', () => {
  // Not a name over a width: `currency` is three bytes with an identity, and
  // `fixed_ascii(3)` is three bytes without one.
  const currency = new DataType('currency')

  assert.equal(currency.id, 'currency')
  assert.equal(currency.kind, 'code')
  assert.equal(currency.toString(), 'currency')
  assert.equal(currency.fixedByteWidth, 3)
  assert.equal(currency.stringParameters, null)
  assert.ok(!currency.equals(DataType.fixedAscii(3)))
  assert.ok(DataType.from('currency').equals(currency))
  assert.ok(DataType.from(' CURRENCY ').equals(currency))

  for (const [name, width] of [
    ['country', 2],
    ['currency', 3],
    ['mic', 4],
    // Six bytes, which is a width no ASCII variant has.
    ['cfi', 6],
    // Twelve: two letters of prefix, nine of national number, one check digit.
    ['isin', 12],
    // The FIX codes store the width their standard fixes, one letter or not.
    ['side', 4],
    ['msgdirection', 4],
    ['state', 10],
    ['timeinforce', 8],
  ]) {
    const dtype = new DataType(name)
    assert.equal(dtype.id, name)
    assert.equal(dtype.fixedByteWidth, width, name)
    assert.equal(dtype.kind, 'code', name)
    assert.equal(dtype.asciiPacked('A'), BigInt('A'.charCodeAt(0)) << BigInt(8 * (width - 1)))
  }

  // The packed integer is the value's own bytes, exactly as for a width.
  assert.equal(currency.asciiPacked('USD'), DataType.fixedAscii(3).asciiPacked('USD'))
  assert.equal(currency.asciiValue(0x555344n), 'USD')
  assert.throws(() => new DataType('country').asciiPacked('USD'), /at most 2 bytes/)
  assert.throws(() => DataType.fromString('sedol'), /unknown datatype/)
})

test('the uuid is sixteen bytes spelled as one identifier', () => {
  const uuid = new DataType('uuid')

  assert.equal(uuid.id, 'uuid')
  assert.equal(uuid.kind, 'uuid')
  assert.equal(uuid.toString(), 'uuid')
  assert.equal(uuid.fixedByteWidth, 16)
  // The canonical spelling parses back to the same type.
  assert.ok(DataType.from('uuid').equals(uuid))
  assert.ok(DataType.fromString(uuid.toString()).equals(uuid))

  // The identity is the sixteen bytes; the spelling is a rendering of them.
  const text = '01912d68-783e-7c9a-b1f2-0123456789ab'
  const field = new Field('id', uuid, false)
  assert.equal(field.defaultJSValue(), '00000000-0000-0000-0000-000000000000')
  assert.ok(Field.fromJSON(field.toJSON()).equals(field))
  assert.ok(Field.fromString(field.toString()).equals(field))
  assert.equal(new Field('id', 'uuid', false).dtype.id, 'uuid')
  void text
})

test('version keeps a native numeric value under its string Arrow representation', () => {
  const version = new DataType('version')

  assert.equal(version.id, 'version')
  assert.equal(version.kind, 'text')
  assert.equal(version.toString(), 'version')
  assert.equal(version.fixedByteWidth, null)
  assert.ok(DataType.fromString(version.toString()).equals(version))

  const field = new Field('version', version, false)
  assert.ok(field.defaultJSValue().equals(new Version(0)))
  assert.ok(Field.fromJSON(field.toJSON()).equals(field))
})

test('url is a validated canonical location', () => {
  const url = new DataType('url')

  assert.equal(url.id, 'url')
  assert.equal(url.kind, 'text')
  assert.equal(url.toString(), 'url')
  assert.equal(url.fixedByteWidth, null)
  assert.ok(DataType.from('url').equals(url))
  assert.ok(DataType.fromString(url.toString()).equals(url))

  const field = new Field('url', url, false)
  assert.ok(Field.fromJSON(field.toJSON()).equals(field))
  assert.ok(Field.fromString(field.toString()).equals(field))
  assert.equal(new Field('url', 'url', false).dtype.id, 'url')

  // A location has no zero, so the default is the shortest URL the validator
  // accepts: the filesystem root.
  assert.equal(field.defaultJSValue(), 'file:///')
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
  assert.deepEqual(
    [...nested].map((field) => field.name),
    ['id', 'payload'],
  )

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
  assert.throws(() => DataType.fromArrow({ toString: () => 42 }), /must return a string/)
})

test('datatype direct structural JSON rejects invalid parameter states', () => {
  const valid = DataType.fromString('list<field("item",int32,nullable=false,metadata={})>')
  assert.ok(DataType.fromJSON(valid.toJSON()).equals(valid))
  assert.throws(() =>
    DataType.fromJSON({
      type: 'binary',
      layout: 'fixed_size_binary',
      fixed: -1,
    }),
  )
  assert.throws(() => DataType.fromJSON({ type: 'decimal32', precision: 10, scale: 0 }))
})

test('malformed recursive datatypes never use a permissive fallback', () => {
  assert.throws(() => DataType.fromString('struct<a: int64'))
  assert.throws(() => DataType.fromString('decimal(0, 9) trailing'))
})

test('a prebuilt vocabulary names the ISO codes a column carries', () => {
  const prebuilt = StringEnum.prebuilt()
  assert.deepEqual(Object.keys(prebuilt).sort(), [
    'country',
    'currency',
    'exchange',
    'mic',
    'msgdirection',
    'side',
    'state',
    'timeinforce',
  ])
  // `exchange` is FIX's name for the ISO 10383 code, so it is one list.
  assert.deepEqual(prebuilt.mic, prebuilt.exchange)

  const countries = StringEnum.fromLogicalName('Country')
  assert.equal(countries.name, 'country')
  assert.equal(countries.length, prebuilt.country.length)
  // An ISO code names itself, so the member and its value are one spelling.
  assert.equal(countries.get('FR'), 'FR')
  assert.equal(countries.getMember('FR'), 'FR')
  // A prebuilt listing is a constant, so a second build is the same enum.
  assert.ok(StringEnum.fromLogicalName('country').equals(countries))
  // `ZZ` is ISO 3166's user-assigned range, so no member names it.
  assert.equal(countries.get('ZZ'), null)
  // The codes pack under the datatype the name resolved to.
  const codes = countries.intoEnum('country')
  assert.equal(codes.FR, new DataType('country').asciiPacked('FR'))

  // A registered name with no prebuilt listing answers an enum of no members,
  // and one that is no registration at all is refused by the vocabulary.
  assert.equal(StringEnum.fromLogicalName('tenor').length, 0)
  // An open identifier space has no listing to prebuild either.
  assert.equal(StringEnum.fromLogicalName('isin').length, 0)
  assert.throws(() => StringEnum.fromLogicalName('sedol'), /currency/)
})

test('the generated enum names each value by the integer it packs into', () => {
  const venues = new StringEnum('Venue', {
    XNAS: 'XNAS',
    N_A: 'n/a',
    _3M: '3M',
  })
  const Venue = venues.intoEnum(DataType.fixedAscii(4))

  // A member is its value packed big-endian, never its position, so it needs
  // the whole width and crosses as a `bigint`.
  assert.deepEqual({ ...Venue }, { XNAS: 0x584e4153n, N_A: 0x6e2f6100n, _3M: 0x334d0000n })
  assert.equal(Venue.XNAS, DataType.fixedAscii(4).asciiPacked('XNAS'))
  assert.equal(DataType.fixedAscii(4).asciiValue(Venue.N_A), 'n/a')
  assert.equal(Object.isFrozen(Venue), true)
  assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')
  // Name to code only; `members` stays the name to value direction.
  assert.deepEqual(venues.members, { XNAS: 'XNAS', N_A: 'n/a', _3M: '3M' })

  // The same rule names one value at a time, for a vocabulary a caller
  // declares member by member rather than generates from a whole listing.
  assert.equal(StringEnum.memberName('n/a'), 'N_A')
  assert.equal(StringEnum.memberName('3M'), '_3M')
  assert.equal(StringEnum.memberName(''), '_')

  // Sixteen bytes name members too, under the whole 128-bit integer.
  const wide = new StringEnum('Wide', { US0378331005: 'US0378331005' })
  assert.equal(
    wide.intoEnum(DataType.fixedAscii(16)).US0378331005,
    0x55533033373833333130303500000000n,
  )

  // A value the width could not store is refused by the width, and a width
  // wider than the packed integer has no codes at all - nor does a string
  // that is not fixed US-ASCII.
  assert.throws(() => venues.intoEnum(DataType.fixedAscii(2)), /at most 2 bytes/)
  assert.throws(() => venues.intoEnum(DataType.fixedAscii(17)), /at most 16 bytes/)
  assert.throws(() => venues.intoEnum('utf8'), /at most 16 bytes/)
  assert.throws(() => venues.intoEnum('ascii(4)'), /at most 16 bytes, or a registered code/)
  assert.throws(() => venues.intoEnum('fixed_utf8(4)'), /at most 16 bytes, or a registered code/)
})

test('an ASCII value packs into the integer its storage reads as', () => {
  const code4 = DataType.fixedAscii(4)

  assert.equal(code4.asciiPacked('USD'), 0x55534400n)
  assert.equal(code4.asciiPacked('USD\0'), 0x55534400n)
  assert.equal(code4.asciiValue(0x55534400n), 'USD')
  // The order of the integers is the order of the text.
  assert.ok(code4.asciiPacked('EUR') < code4.asciiPacked('USD'))

  const code16 = DataType.fixedAscii(16)
  const isin = code16.asciiPacked('US0378331005')
  assert.equal(isin, 0x55533033373833333130303500000000n)
  assert.equal(code16.asciiValue(isin), 'US0378331005')

  assert.throws(() => code4.asciiPacked('EURO!'), /at most 4 bytes/)
  assert.throws(() => code4.asciiValue(-1n), /wider than the width/)
  // Variable ASCII has no width, so it has no packed integer either.
  assert.throws(() => new DataType('ascii').asciiPacked('USD'), /at most 16 bytes/)
  assert.throws(() => new DataType('utf8').asciiPacked('USD'), /at most 16 bytes/)
  assert.throws(() => DataType.fixedUtf8(4).asciiPacked('USD'), /at most 16 bytes/)
})

test('an enum declares itself onto the field its values name', () => {
  const side = new StringEnum('Side', { BUY: 'B', SELL: 'S' })

  assert.equal(side.name, 'Side')
  assert.deepEqual(side.members, { BUY: 'B', SELL: 'S' })
  assert.equal(side.get('BUY'), 'B')
  assert.equal(side.getMember('S'), 'SELL')
  assert.equal(side.length, 2)
  assert.deepEqual(side.intoMembers(DataType.fixedAscii(4)), {
    BUY: 0x42000000n,
    SELL: 0x53000000n,
  })
  assert.equal(side.toString(), '{"members":{"BUY":"B","SELL":"S"},"name":"Side"}')
  assert.ok(StringEnum.fromJson(side.intoJson()).equals(side))

  // The declaration is ordinary field metadata under one reserved key, so
  // every serialization carries it and it reads back as the enum that wrote it.
  const field = new Field('side', DataType.fixedAscii(4), false)
  field.setStringEnum(side)
  assert.equal(field.get('field:enum'), side.intoJson())
  assert.ok(Field.fromJSONBytes(field.toJSONBytes()).stringEnum.equals(side))
  assert.ok(Field.fromString(field.toString()).stringEnum.equals(side))
  assert.ok(field.removeStringEnum().equals(side))
  assert.equal(field.stringEnum, null)

  // A value the width could not store is refused at the declaration, and so
  // is a field whose values do not pack: the enum needs a fixed US-ASCII
  // string of at most sixteen bytes or a code.
  const wide = new StringEnum('Venue', { LONG: 'EUREX' })
  assert.throws(
    () => new Field('venue', DataType.fixedAscii(4)).setStringEnum(wide),
    /at most 4 bytes/,
  )
  assert.throws(
    () => new Field('venue', DataType.utf8()).setStringEnum(side),
    /expected a fixed US-ASCII string of at most 16 bytes, or a registered code, got utf8/,
  )
  const coded = new Field('ccy', 'currency', false)
  coded.setStringEnum(side)
  assert.ok(coded.stringEnum.equals(side))
  assert.throws(() => new StringEnum('Side', { '': 'B' }), /non-empty member name/)
  assert.throws(() => StringEnum.fromJson('[]'), /enum JSON object/)
})
