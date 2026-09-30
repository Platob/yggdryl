'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const arrow = require('apache-arrow')

const { DataType, Scalar, Serie, Version, enums, fields, json } = require('yggdryl')
const { Version: NativeVersion } = require('../index.js')

test('Version holds a sixteen-bit major and minor and a text patch', () => {
  for (const [text, parts, canonical] of [
    ['0', [0, 0, null], '0'],
    ['5.0.0', [5, 0, null], '5'],
    ['005.002.00300', [5, 2, '300'], '5.2.300'],
    ['256.256', [256, 256, null], '256.256'],
    ['65535.65535.65535', [65535, 65535, '65535'], '65535.65535.65535'],
    ['5.0SP2', [5, 0, '2'], '5.0.2'],
    ['1.0-rc1', [1, 0, '-rc1'], '1.0-rc1'],
  ]) {
    const value = Version.fromStr(text)
    assert.ok(value instanceof Version)
    assert.ok(value.equals(new Version(...parts)), text)
    assert.deepEqual([value.major, value.minor, value.patch], parts)
    assert.equal(value.toString(), canonical)
    assert.equal(value.toJSON(), canonical)
    assert.equal(JSON.stringify(value), JSON.stringify(canonical))
    assert.ok(Version.fromStr(canonical).equals(value), canonical)
    assert.equal('tag' in value, false)
    assert.equal('qualifier' in value, false)
  }
  assert.ok(new Version(5).equals(new Version(5, 0, null)))
  assert.ok(new Version(5, null, null).equals(new Version(5)))
  assert.ok(new Version(5, undefined, undefined).equals(new Version(5)))
  assert.equal(new Version(5).patch, null)
})

test('Version canonicalizes a constructed patch as the parser does', () => {
  const numeric = new Version(5, 0, 300)
  assert.equal(numeric.patch, '300')
  assert.ok(numeric.equals(new Version(5, 0, '300')))
  assert.ok(numeric.equals(new Version(5, 0, '0300')))
  assert.ok(new Version(5, 0, 0).equals(new Version(5)))
  assert.ok(new Version(5, 0, '000').equals(new Version(5)))
  assert.ok(new Version(5, 0, '').equals(new Version(5)))
  // A service pack is the grammar's, so a constructed `sp2` is that text.
  const text = new Version(5, 0, 'sp2')
  assert.equal(text.patch, 'sp2')
  assert.equal(text.toString(), '5.0.sp2')
  assert.ok(!text.equals(Version.fromStr('5.0sp2')))
  assert.equal(new Version(1, 0, 2 ** 32).patch, '4294967296')
  assert.equal(new Version(1, 0, Number.MAX_SAFE_INTEGER).patch, '9007199254740991')
})

test('Version rejects numeric coercion and width overflow at the native boundary', () => {
  for (const [component, name] of [[0, 'major'], [1, 'minor']]) {
    for (const bad of [NaN, Infinity, -Infinity, -1, 0.5, 65536, 2 ** 32, Number.MAX_SAFE_INTEGER]) {
      const parts = [1, 2, '3']
      parts[component] = bad
      assert.throws(() => new NativeVersion(...parts), new RegExp(name), `${name} ${bad}`)
    }
    for (const bad of [-1, 65536, 2 ** 32, 1e20, -1e300]) {
      const parts = [1, 2]
      parts[component] = bad
      assert.throws(() => new NativeVersion(...parts), new RegExp(`${name} must be in 0\\.\\.65535`))
    }
    const parts = [1, 2]
    parts[component] = 256
    assert.equal(new NativeVersion(...parts)[name], 256)
    parts[component] = 65535
    assert.equal(new NativeVersion(...parts)[name], 65535)
    // Text is a patch, never a major or a minor.
    parts[component] = '1'
    assert.throws(() => new NativeVersion(...parts))
  }
  for (const bad of [NaN, Infinity, -Infinity, -1, 0.5, 2 ** 53 + 2]) {
    assert.throws(() => new NativeVersion(1, 2, bad), /patch/, `patch ${bad}`)
  }
  assert.equal(new NativeVersion(1, 2, '3').patch, '3')
  for (const bad of [1n, true, {}, []]) {
    for (let component = 0; component < 3; component += 1) {
      const parts = [1, 2, 3]
      parts[component] = bad
      assert.throws(() => new NativeVersion(...parts))
    }
  }
})

test('Version native parser rejects only the major and the minor', () => {
  const field = fields.version('release', { nullable: false })
  for (const text of [
    '.1', ' 1', '-1', '+1', 'v1', '65536', '99999', '1.65536', '1.99999', 'FIX.5.0',
  ]) {
    assert.throws(() => Version.fromStr(text), /version/i, text)
    assert.throws(() => Scalar.from(text, { field }), /version/i, text)
  }
  assert.equal(Version.fromStr('256').major, 256)
  assert.equal(Version.fromStr('1.256').minor, 256)
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
  for (const [text, major, minor, patch, canonical] of [
    ['5.0sp250', 5, 0, '250', '5.0.250'],
    ['5.0SP250', 5, 0, '250', '5.0.250'],
    ['5.0Sp250', 5, 0, '250', '5.0.250'],
    ['5.0sP250', 5, 0, '250', '5.0.250'],
    ['005.000sp00250', 5, 0, '250', '5.0.250'],
    ['5.0SP2', 5, 0, '2', '5.0.2'],
    ['5.0SP0', 5, 0, null, '5'],
    ['5.0sp65535', 5, 0, '65535', '5.0.65535'],
    ['65535.65535sp65535', 65535, 65535, '65535', '65535.65535.65535'],
  ]) {
    const parsed = Version.fromStr(text)
    assert.ok(parsed.equals(new Version(major, minor, patch)), text)
    assert.equal(parsed.patch, patch, text)
    assert.equal(parsed.toString(), canonical, text)
  }
})

test('Version keeps a patch tail that states no number as written', () => {
  const field = fields.version('release', { nullable: false })
  for (const [text, patch, canonical] of [
    ['1.2-rc1', '-rc1', '1.2-rc1'],
    ['1.2rc1', 'rc1', '1.2.rc1'],
    ['1.0SP2_EP250', 'SP2_EP250', '1.0.SP2_EP250'],
    ['5.0.SP2', 'SP2', '5.0.SP2'],
    ['1.2.3.4', '3.4', '1.2.3.4'],
    ['1.2.65536', '65536', '1.2.65536'],
    ['1.2+meta', '+meta', '1.2+meta'],
    ['1.2.-1', '-1', '1.2-1'],
    ['1..2', '.2', '1.0..2'],
    ['1.2界', '界', '1.2界'],
    ['1.0sp250 ', 'sp250 ', '1.0.sp250 '],
    ['1.', null, '1'],
  ]) {
    const parsed = Version.fromStr(text)
    assert.equal(parsed.patch, patch, text)
    assert.equal(parsed.toString(), canonical, text)
    assert.ok(Version.fromStr(canonical).equals(parsed), text)
    assert.ok(new Version(parsed.major, parsed.minor, patch).equals(parsed), text)
    assert.ok(Scalar.from(text, { field }).asJs().equals(parsed), text)
  }
  assert.ok(!Version.fromStr('1.0-rc1').equals(Version.fromStr('1.0-rc2')))
  assert.ok(!Version.fromStr('1.0-rc1').equals(Version.fromStr('1.0')))
})

test('Version order, native hash, clone, and read-only parts', () => {
  const value = new Version(5, 0, 300)
  assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
  assert.equal(value.compare(new Version(5, 0, 10)), 1)
  assert.equal(value.compare(Version.fromStr('005.0.00300')), 0)
  // No patch first, then patches in natural order: a run of digits is the
  // number it spells.
  const ordered = [
    '1.0', '1.0-rc1', '1.0-rc2', '1.0-rc10', '1.0.2', '1.0.2.1', '1.0.10', '1.1',
  ].map((text) => Version.fromStr(text))
  for (let index = 1; index < ordered.length; index += 1) {
    assert.equal(ordered[index - 1].compare(ordered[index]), -1, String(ordered[index]))
    assert.equal(ordered[index].compare(ordered[index - 1]), 1, String(ordered[index]))
  }
  // Patches equal in natural order stay two values, told apart by their bytes.
  const padded = Version.fromStr('1.0-rc01')
  assert.ok(!padded.equals(Version.fromStr('1.0-rc1')))
  assert.equal(padded.compare(Version.fromStr('1.0-rc1')), -1)
  assert.equal(value.stableHash(), Scalar.from(value).stableHash())
  const copy = value.clone()
  assert.notEqual(copy, value)
  assert.ok(copy.equals(value))
  assert.equal(copy.stableHash(), value.stableHash())
  assert.equal(typeof value.stableHash(), 'bigint')
  for (const part of ['major', 'minor', 'patch']) {
    assert.throws(() => { value[part] = 0 }, TypeError)
  }
  assert.deepEqual([value.major, value.minor, value.patch], [5, 0, '300'])
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
  for (const held of [value, new Version(5), new Version(1, 0, '-rc1')]) {
    const nested = Scalar.from([{ release: held }]).asJs()
    assert.ok(nested[0].release instanceof Version)
    assert.ok(nested[0].release.equals(held), String(held))
    assert.equal(nested[0].release.patch, held.patch)
  }
  assert.equal(json.dumps(value).toString(), '"5.0.300"')
  assert.equal(json.dumps(new Version(1, 0, '-rc1')).toString(), '"1.0-rc1"')
  assert.ok(json.loads('"5.0.300"', { field }).equals(value))
  assert.ok(json.loads('"5.0.300"', { field, scalar: true }).equals(scalar))
  const literal = { __yggdryl_codec__: 'version', major: 5, minor: 0, patch: '300' }
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
  const vector = arrow.vectorFromArray(
    ['005.0.00300', '5.0', '65535.65535.65535', '1.0-rc1'],
    new arrow.Utf8(),
  )
  const native = Serie.fromArrowArray(vector, field).intoScalar()
  const canonical = ['5.0.300', '5', '65535.65535.65535', '1.0-rc1']
  assert.deepEqual(native.asJs().map(String), canonical)
  assert.ok(native.asJs().every((item) => item instanceof Version))
  assert.deepEqual([...Serie.fromScalars(field, native).intoArrowArray()], canonical)
  const rows = Scalar.from([{ release: value }])
  const inferred = Serie.fromScalars(rows.intoStructField(), rows).intoArrowBatch()
  assert.equal(inferred.schema.fields[0].metadata.get('ARROW:extension:name'), 'yggdryl.version')
  assert.ok(Serie.fromArrowBatch(inferred).intoScalar().asJs()[0][0].equals(value))
})

test('the datatype identifiers are laid out by family', () => {
  // Ninety-one, laid out by family: every identifier sits in its family's
  // range and the list states them in that order, so `url` and `urn` follow
  // `version` in the text family, `sized_utf8` follows `fixed_utf8`, `ric`
  // follows `unit` and `forex` closes the code family, the geospatial pair
  // follows, and the enum family - `state`, `marketdatakind`, `side`,
  // `marketdatatype`, `timeinforce` - closes the list. An identifier is a wire contract laid out by family, so a leaf
  // added later lands beside its family and nothing ever moves.
  assert.equal(enums.dataTypeIds.length, 91)
  assert.equal(enums.dataTypeIds.includes('figi'), true)
  assert.equal(enums.dataTypeIds.includes('bbg'), true)
  const ids = [...enums.dataTypeIds]
  assert.equal(ids.indexOf('url'), ids.indexOf('version') + 1)
  assert.equal(ids.indexOf('urn'), ids.indexOf('url') + 1)
  assert.equal(ids.indexOf('sized_utf8'), ids.indexOf('fixed_utf8') + 1)
  assert.equal(ids.indexOf('ric'), ids.indexOf('unit') + 1)
  assert.equal(ids.indexOf('forex'), ids.indexOf('ric') + 1)
  assert.deepEqual(ids.slice(-7), ['geometry', 'geography', 'state', 'marketdatakind', 'side', 'marketdatatype', 'timeinforce'])
  assert.deepEqual(ids.slice(0, 2), ['null', 'boolean'])
})
