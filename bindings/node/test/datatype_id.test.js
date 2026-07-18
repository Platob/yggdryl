'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataTypeId } = yggdryl.datatype_id

// -------------------------------------------------------------------------------------
// Namespace + variant factories
// -------------------------------------------------------------------------------------

test('the datatype_id namespace exposes DataTypeId with a factory per variant', () => {
  assert.equal(typeof DataTypeId, 'function')
  const variants = [
    ['Unknown', 0, 'unknown'],
    ['Bool', 1, 'bool'],
    ['I8', 2, 'i8'],
    ['U8', 3, 'u8'],
    ['I16', 4, 'i16'],
    ['U16', 5, 'u16'],
    ['I32', 6, 'i32'],
    ['U32', 7, 'u32'],
    ['I64', 8, 'i64'],
    ['U64', 9, 'u64'],
    ['I128', 10, 'i128'],
    ['U128', 11, 'u128'],
    ['F32', 12, 'f32'],
    ['F64', 13, 'f64'],
  ]
  for (const [factory, id, name] of variants) {
    const d = DataTypeId[factory]()
    assert.ok(d instanceof DataTypeId)
    assert.equal(d.id, id, `${factory}.id`)
    assert.equal(d.asU16(), id, `${factory}.asU16()`)
    assert.equal(d.name(), name, `${factory}.name()`)
    assert.equal(d.toString(), name, `${factory}.toString()`)
  }
})

// -------------------------------------------------------------------------------------
// id + name round-trips (constructor / fromU16 / fromName)
// -------------------------------------------------------------------------------------

test('constructor + fromU16 round-trip the u16 id; unknown ids degrade to Unknown', () => {
  assert.ok(new DataTypeId(8).equals(DataTypeId.I64()))
  assert.ok(DataTypeId.fromU16(13).equals(DataTypeId.F64()))
  assert.equal(DataTypeId.fromU16(0).name(), 'unknown')
  // A foreign / newer id degrades to raw bytes (Unknown), never throws.
  assert.equal(DataTypeId.fromU16(999).name(), 'unknown')
  assert.equal(new DataTypeId(999).name(), 'unknown')
})

test('fromName parses (case-insensitive); an unknown token throws the guided error', () => {
  assert.ok(DataTypeId.fromName('i32').equals(DataTypeId.I32()))
  assert.ok(DataTypeId.fromName('F64').equals(DataTypeId.F64())) // case-insensitive
  assert.equal(DataTypeId.fromName('unknown').name(), 'unknown')
  assert.throws(() => DataTypeId.fromName('nope'), /unknown data type name/)
  assert.throws(() => DataTypeId.fromName('nope'), /i64/) // the accepted tokens are named
})

// -------------------------------------------------------------------------------------
// Widths + classification
// -------------------------------------------------------------------------------------

test('byteSize / bitSize per type; Unknown is 0', () => {
  assert.equal(DataTypeId.Unknown().byteSize(), 0)
  assert.equal(DataTypeId.Unknown().bitSize(), 0)
  assert.equal(DataTypeId.Bool().byteSize(), 1)
  assert.equal(DataTypeId.Bool().bitSize(), 1) // 1 byte stored, 1 bit logically
  assert.equal(DataTypeId.I32().byteSize(), 4)
  assert.equal(DataTypeId.I32().bitSize(), 32)
  assert.equal(DataTypeId.I64().byteSize(), 8)
  assert.equal(DataTypeId.U128().byteSize(), 16)
  assert.equal(DataTypeId.U128().bitSize(), 128)
})

test('classification predicates', () => {
  assert.equal(DataTypeId.I32().isInteger(), true)
  assert.equal(DataTypeId.Bool().isInteger(), false) // bool is not an integer
  assert.equal(DataTypeId.I32().isSigned(), true)
  assert.equal(DataTypeId.U32().isSigned(), false)
  assert.equal(DataTypeId.F64().isSigned(), true) // floats are signed
  assert.equal(DataTypeId.F32().isFloat(), true)
  assert.equal(DataTypeId.I32().isFloat(), false)
  assert.equal(DataTypeId.Bool().isBool(), true)
  assert.equal(DataTypeId.I8().isBool(), false)
  assert.equal(DataTypeId.I32().isFixedWidth(), true)
  assert.equal(DataTypeId.Unknown().isFixedWidth(), false)
})

test('elementCount divides bytes by the width; Unknown and negatives are 0', () => {
  assert.equal(DataTypeId.I32().elementCount(20), 5)
  assert.equal(DataTypeId.I32().elementCount(22), 5) // whole elements only
  assert.equal(DataTypeId.I64().elementCount(24), 3)
  assert.equal(DataTypeId.Unknown().elementCount(100), 0)
  assert.equal(DataTypeId.I32().elementCount(-4), 0)
})

// -------------------------------------------------------------------------------------
// Value semantics
// -------------------------------------------------------------------------------------

test('equals is identity over the element type', () => {
  assert.ok(DataTypeId.I64().equals(DataTypeId.fromU16(8)))
  assert.ok(!DataTypeId.I64().equals(DataTypeId.U64()))
  assert.ok(!DataTypeId.I32().equals(DataTypeId.F32()))
})
