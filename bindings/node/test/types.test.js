'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataType, Field } = yggdryl.types

// ---------------------------------------------------------------------------------------
// DataType
// ---------------------------------------------------------------------------------------

test('the types namespace exposes DataType and Field', () => {
  for (const cls of [DataType, Field]) {
    assert.equal(typeof cls, 'function')
  }
})

test('named factories: name and byte width', () => {
  assert.deepEqual([DataType.u8().name, DataType.u8().byteWidth], ['u8', 1])
  assert.deepEqual([DataType.i256().name, DataType.i256().byteWidth], ['i256', 32])
  assert.deepEqual([DataType.f16().name, DataType.f16().byteWidth], ['f16', 2])
  assert.deepEqual([DataType.utf8().name, DataType.utf8().byteWidth], ['utf8', 4])
  assert.deepEqual([DataType.largeBinary().name, DataType.largeBinary().byteWidth], ['large_binary', 8])
})

test('byName covers every type and rejects unknown', () => {
  for (const name of ['u96', 'i128', 'f64', 'binary', 'large_utf8', 'null']) {
    assert.equal(DataType.byName(name).name, name)
  }
  assert.throws(() => DataType.byName('nonesuch'), /unknown data type/)
})

test('fixed-size types take a runtime width', () => {
  const fb = DataType.fixedBinary(16)
  assert.equal(fb.name, 'fixed_binary')
  assert.equal(fb.byteWidth, 16)
  assert.ok(fb.isBinary() && fb.isFixedWidth() && !fb.isVariableLength())

  const fu = DataType.fixedUtf8(4)
  assert.ok(fu.isUtf8() && fu.isFixedWidth())
  // Same Arrow width, different logical type -> not equal.
  assert.ok(!fb.equals(DataType.fixedBinary(8)))
  assert.ok(!fu.equals(fb))
})

test('category drill-down', () => {
  const row = (dt) => [
    dt.isInteger(), dt.isUnsignedInteger(), dt.isSignedInteger(), dt.isSigned(),
    dt.isFloating(), dt.isNumeric(), dt.isUtf8(), dt.isBinary(),
    dt.isFixedWidth(), dt.isVariableLength(),
  ]
  assert.deepEqual(row(DataType.u32()), [true, true, false, false, false, true, false, false, true, false])
  assert.deepEqual(row(DataType.i32()), [true, false, true, true, false, true, false, false, true, false])
  assert.deepEqual(row(DataType.f64()), [false, false, false, true, true, true, false, false, true, false])
  assert.deepEqual(row(DataType.utf8()), [false, false, false, false, false, false, true, false, false, true])
  assert.deepEqual(row(DataType.binary()), [false, false, false, false, false, false, false, true, false, true])
})

test('category string and toString', () => {
  assert.equal(DataType.u8().category, 'unsigned_integer')
  assert.equal(DataType.i8().category, 'signed_integer')
  assert.equal(DataType.f32().category, 'float')
  assert.equal(DataType.utf8().category, 'utf8')
  assert.equal(DataType.null().category, 'null')
  assert.equal(DataType.fixedBinary(16).toString(), 'DataType(fixed_binary[16])')
  assert.equal(DataType.i32().toString(), 'DataType(i32)')
})

test('DataType equality', () => {
  assert.ok(DataType.i64().equals(DataType.i64()))
  assert.ok(!DataType.i64().equals(DataType.u64()))
})

// ---------------------------------------------------------------------------------------
// Field
// ---------------------------------------------------------------------------------------

test('field construction and properties', () => {
  const f = new Field('id', DataType.i64()) // nullable defaults to true
  assert.equal(f.name, 'id')
  assert.equal(f.typeName, 'i64')
  assert.equal(f.byteWidth, 8)
  assert.equal(f.nullable, true)
  assert.ok(f.dataType.equals(DataType.i64()))
  assert.ok(f.isInteger() && f.isSigned())
  assert.equal(f.metadata.size, 0)

  const strict = new Field('id', DataType.i64(), false)
  assert.equal(strict.nullable, false)
})

test('field metadata from an object, and value equality', () => {
  const a = new Field('t', DataType.f64(), true, { unit: 'seconds' })
  const b = new Field('t', DataType.f64(), true, { unit: 'seconds' })
  assert.equal(a.metadata.get('unit'), 'seconds')
  assert.ok(a.equals(b)) // metadata is part of the value
  assert.ok(!a.equals(new Field('t', DataType.f64())))
})

test('field metadata builders are non-mutating', () => {
  const base = new Field('t', DataType.utf8())
  const tagged = base.withMetadataEntry('charset', 'utf8').withMetadataEntry('lang', 'en')
  assert.deepEqual(tagged.metadata.keys(), ['charset', 'lang'])
  assert.equal(base.metadata.size, 0) // base untouched

  const replaced = base.withMetadata({ only: 'this' })
  assert.deepEqual(replaced.metadata.toObject(), { only: 'this' })

  // The metadata accessor returns a copy — mutating it does not affect the field.
  const meta = tagged.metadata
  meta.insert('extra', 'x')
  assert.ok(!tagged.metadata.has('extra'))
})

test('field copy is an independent equal value', () => {
  const a = new Field('x', DataType.utf8(), true, { k: 'v' })
  const dup = a.copy()
  assert.ok(dup.equals(a))
})
