'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { DataType } = require('yggdryl')

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

test('recursive datatypes expose fields as a collection', () => {
  const nested = DataType.fromString(
    'struct<id: bigint not null, payload: array<struct<name: string, score: decimal(18, 4)>>>',
  )

  assert.equal(nested.length, 2)
  assert.equal(nested.at(0).name, 'id')
  assert.equal(nested.at(-1).name, 'payload')
  assert.equal(nested.get('id').nullable, false)
  assert.equal(nested.getByName('id').name, 'id')
  assert.equal(nested.get(1).name, 'payload')
  assert.equal(nested.get('missing'), null)
  assert.equal(nested.contains('payload'), true)
  assert.equal(nested.contains(nested.at(0)), true)
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
