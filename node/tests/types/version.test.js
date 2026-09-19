'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const arrow = require('apache-arrow')

const { DataType, Field, Scalar, Version, enums, fields, json } = require('yggdryl')
const { Version: NativeVersion } = require('../../index.js')

test('Version native integer parts use u8, u8, u16 and canonical numeric text', () => {
  for (const [text, parts, canonical] of [
    ['0', [0, 0, 0], '0'],
    ['5.0.0', [5, 0, 0], '5'],
    ['005.002.00300', [5, 2, 300], '5.2.300'],
    ['255.255.65535', [255, 255, 65535], '255.255.65535'],
  ]) {
    const value = Version.fromStr(text)
    assert.ok(value instanceof Version)
    assert.ok(value.equals(new Version(...parts)))
    assert.deepEqual([value.major, value.minor, value.patch], parts)
    assert.equal(value.toString(), canonical)
    assert.equal(value.toJSON(), canonical)
    assert.equal(JSON.stringify(value), JSON.stringify(canonical))
    assert.equal('tag' in value, false)
    assert.equal('qualifier' in value, false)
  }
  assert.ok(new Version(5).equals(new Version(5, 0, 0)))
  assert.ok(new Version(5, null, null).equals(new Version(5)))
})

test('Version rejects numeric coercion and width overflow at the native boundary', () => {
  for (const bad of [NaN, Infinity, -Infinity, -1, 0.5, 2 ** 32, Number.MAX_SAFE_INTEGER]) {
    for (let component = 0; component < 3; component += 1) {
      const parts = [1, 2, 3]
      parts[component] = bad
      assert.throws(() => new NativeVersion(...parts), /major|minor|patch/)
    }
  }
  for (const parts of [[256], [1, 256], [1, 2, 65536]]) {
    assert.throws(() => new NativeVersion(...parts), /0\.\.255|0\.\.65535/)
  }
  for (const bad of ['1', 1n, true, {}, []]) {
    for (let component = 0; component < 3; component += 1) {
      const parts = [1, 2, 3]
      parts[component] = bad
      assert.throws(() => new NativeVersion(...parts))
    }
  }
})

test('Version native parser rejects only the numeric components', () => {
  const field = fields.version('release', { nullable: false })
  for (const text of [
    '.1', ' 1', '-1', '+1', 'v1', '256', '999', '1.256', '1.999', 'FIX.5.0',
  ]) {
    assert.throws(() => Version.fromStr(text), /version/i)
    assert.throws(() => Scalar.from(text, { field }), /version/i)
  }
})

test('the empty text is no Version, but reads as null at the datatype', () => {
  // A `Version` is a value, so its own parser refuses the empty text; the
  // datatype door reads an empty text cell entering a non-text column as
  // absence, and a required field is what refuses that.
  assert.throws(() => Version.fromStr(''), /version/i)
  assert.equal(new DataType('version').scalar('').kind, 'null')
  assert.equal(Scalar.from('', { field: fields.version('release') }).kind, 'null')
  assert.throws(
    () => Scalar.from('', { field: fields.version('release', { nullable: false }) }),
    /non-nullable field received null/,
  )
})

test('Version reads a compact FIX service pack as its numeric patch', () => {
  for (const [text, major, minor, patch] of [
    ['5.0sp250', 5, 0, 250],
    ['5.0SP250', 5, 0, 250],
    ['5.0Sp250', 5, 0, 250],
    ['5.0sP250', 5, 0, 250],
    ['005.000sp00250', 5, 0, 250],
    ['5.0SP0', 5, 0, 0],
    ['5.0sp65535', 5, 0, 65535],
    ['255.255sp65535', 255, 255, 65535],
  ]) {
    const parsed = Version.fromStr(text)
    assert.ok(parsed.equals(new Version(major, minor, patch)), text)
  }
})

test('Version folds a patch tail that states no number instead of failing', () => {
  const field = fields.version('release', { nullable: false })
  for (const text of [
    '1.', '1..2', '1.2.3.4', '1.2.65536', '5.0.SP2', '1.2-rc1', '1.2+meta',
    '1.2.-1', '1.0SP2_EP250', '1.0sp250 ',
  ]) {
    const parsed = Version.fromStr(text)
    // The same tail always reads as the same version.
    assert.ok(parsed.equals(Version.fromStr(text)), text)
    assert.doesNotThrow(() => Scalar.from(text, { field }), text)
  }
  assert.ok(!Version.fromStr('1.0-rc1').equals(Version.fromStr('1.0-rc2')))
  assert.ok(!Version.fromStr('1.0-rc1').equals(Version.fromStr('1.0')))
  assert.notEqual(Version.fromStr('1.0-rc1').patch, 0)
})

test('Version order, native hash, clone, and read-only parts share numeric identity', () => {
  const value = new Version(5, 0, 300)
  assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
  assert.equal(value.compare(new Version(5, 0, 10)), 1)
  assert.equal(value.compare(Version.fromStr('005.0.00300')), 0)
  assert.equal(value.stableHash(), Scalar.from(value).stableHash())
  const copy = value.clone()
  assert.notEqual(copy, value)
  assert.ok(copy.equals(value))
  assert.equal(copy.stableHash(), value.stableHash())
  assert.equal(typeof value.stableHash(), 'bigint')
  for (const part of ['major', 'minor', 'patch']) {
    assert.throws(() => { value[part] = 0 }, TypeError)
  }
  assert.deepEqual([value.major, value.minor, value.patch], [5, 0, 300])
  assert.throws(() => value.compare('5.0.300'))
})

test('Version survives Scalar, nested native values, and declared JSON boundaries', () => {
  const value = new Version(5, 0, 300)
  const field = fields.version('release', { nullable: false })
  const scalar = Scalar.from(value)
  assert.equal(scalar.id, 'version')
  assert.equal(scalar.dtype.id, 'version')
  assert.ok(scalar.asJs() instanceof Version)
  assert.ok(scalar.asJs().equals(value))
  assert.ok(Scalar.from('005.0.00300', { field }).equals(scalar))
  assert.ok(Scalar.from(value, { field }).equals(scalar))
  const nested = Scalar.from([{ release: value }]).asJs()
  assert.ok(nested[0].release instanceof Version)
  assert.ok(nested[0].release.equals(value))
  assert.equal(json.dumps(value).toString(), '"5.0.300"')
  assert.ok(json.loads('"5.0.300"', { field }).equals(value))
  assert.ok(json.loads('"5.0.300"', { field, scalar: true }).equals(scalar))
  const literal = { __yggdryl_codec__: 'version', major: 5, minor: 0, patch: 300 }
  assert.deepEqual(Scalar.from(literal).asJs(), literal)
  assert.throws(() => Scalar.from(Object.create(Version.prototype)))
})

test('Version field defaults and hints expose the native value with Arrow string storage', () => {
  const dtype = new DataType('version')
  const field = fields.version('release', { nullable: false })
  assert.ok(dtype.defaultJSValue() instanceof Version)
  assert.ok(field.defaultJSValue().equals(new Version(0)))
  assert.equal(fields.version('release').defaultJSValue(), null)
  assert.equal(dtype.defaultJSHint().constructor, Version)
  assert.equal(field.defaultJSHint().constructor, Version)
  assert.equal(field.defaultJSHint().nullable, false)
  assert.equal(fields.version('release').defaultJSHint().nullable, true)
  assert.equal(dtype.defaultArrowScalar(), '0')
  assert.equal(field.defaultArrowScalar(), '0')
  const value = new Version(5, 0, 300)
  const scalar = Scalar.from(value)
  assert.equal(scalar.intoArrowScalar(field), '5.0.300')
  const vector = arrow.vectorFromArray(['005.0.00300', '5.0', '255.255.65535'], new arrow.Utf8())
  const native = Scalar.fromArrowArray(vector, field)
  assert.deepEqual(native.asJs().map(String), ['5.0.300', '5', '255.255.65535'])
  assert.ok(native.asJs().every((item) => item instanceof Version))
  assert.deepEqual([...native.intoArrowArray(field)], ['5.0.300', '5', '255.255.65535'])
  const rows = Scalar.from([{ release: value }])
  const inferred = rows.intoArrowBatch()
  assert.equal(inferred.schema.fields[0].metadata.get('ARROW:extension:name'), 'yggdryl.version')
  assert.ok(Scalar.fromArrowBatch(inferred).asJs()[0][0].equals(value))
})

test('generic MsgType datatype and field helpers are retired', () => {
  assert.equal('msgtype' in fields, false)
  assert.equal(enums.dataTypeIds.includes('msgtype'), false)
  // Eighty-six: `msgdirection` was retired (discriminant 58, never
  // reused), so `url` keeps its byte 59 and sits one index earlier;
  // `timezone`, `mimetype` and `mediatype` were appended after it, then
  // `cusip` and `sedol` as code datatypes of their own, and `bloomberg`
  // after them - the one code whose width is only a bound, because a ticker,
  // a market and a yellow key have no fixed length between them. The families
  // appended the rest: three versioned uuid leaves (69-71, retired when uuid
  // became one datatype again, never reused), `large_binary_view` and
  // `sized_binary` when the byte family became six real leaves, and the
  // thirteen string leaves beyond the five UTF-8 ones - `sized_utf8`, then
  // the six US-ASCII and the six windows-1252 shapes - when the string family
  // became eighteen; and `urn` last, the name beside the `url` location.
  assert.equal(enums.dataTypeIds.includes('msgdirection'), false)
  assert.equal(enums.dataTypeIds.length, 84)
  assert.equal(enums.dataTypeIds.indexOf('url'), 58)
  assert.equal(enums.dataTypeIds.indexOf('urn'), 83)
  assert.equal(enums.dataTypeIds.indexOf('utf8'), 27)
  assert.equal(enums.dataTypeIds.indexOf('sized_utf8'), 70)
  assert.deepEqual(enums.dataTypeIds.slice(-5), [
    'cp1252_view',
    'large_cp1252_view',
    'fixed_cp1252',
    'sized_cp1252',
    'urn',
  ])
  assert.throws(() => new DataType('msgtype'))
  assert.throws(() => new Field('code', 'msgtype'))
  assert.equal(Scalar.from('UConfiguration').asJs(), 'UConfiguration')
})
