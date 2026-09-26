'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const arrow = require('apache-arrow')

const { DataType, Scalar, Serie, Version, enums, fields, json } = require('yggdryl')
const { Version: NativeVersion } = require('../index.js')

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
  assert.equal(Serie.fromDefault(field).intoArrowScalar(), '0')
  const value = new Version(5, 0, 300)
  const scalar = Scalar.from(value)
  assert.equal(Serie.fromScalars(field, [scalar]).intoArrowScalar(), '5.0.300')
  const vector = arrow.vectorFromArray(['005.0.00300', '5.0', '255.255.65535'], new arrow.Utf8())
  const native = Serie.fromArrowArray(vector, field).intoScalar()
  assert.deepEqual(native.asJs().map(String), ['5.0.300', '5', '255.255.65535'])
  assert.ok(native.asJs().every((item) => item instanceof Version))
  assert.deepEqual(
    [...Serie.fromScalars(field, native).intoArrowArray()],
    ['5.0.300', '5', '255.255.65535'],
  )
  const rows = Scalar.from([{ release: value }])
  const inferred = Serie.fromScalars(rows.intoStructField(), rows).intoArrowBatch()
  assert.equal(inferred.schema.fields[0].metadata.get('ARROW:extension:name'), 'yggdryl.version')
  assert.ok(Serie.fromArrowBatch(inferred).intoScalar().asJs()[0][0].equals(value))
})

test('the datatype identifiers are laid out by family', () => {
  // Eighty-five, laid out by family: every identifier sits in its family's
  // range and the list states them in that order, so `url` and `urn` follow
  // `version` in the text family, `sized_utf8` follows `fixed_utf8`, and
  // the geospatial pair closes the list. An identifier is a wire contract
  // laid out by family, so a leaf added later lands beside its family and
  // nothing ever moves.
  assert.equal(enums.dataTypeIds.length, 87)
  assert.equal(enums.dataTypeIds.includes('figi'), true)
  const ids = [...enums.dataTypeIds]
  assert.equal(ids.indexOf('url'), ids.indexOf('version') + 1)
  assert.equal(ids.indexOf('urn'), ids.indexOf('url') + 1)
  assert.equal(ids.indexOf('sized_utf8'), ids.indexOf('fixed_utf8') + 1)
  assert.deepEqual(ids.slice(-2), ['geometry', 'geography'])
  assert.deepEqual(ids.slice(0, 2), ['null', 'boolean'])
})
