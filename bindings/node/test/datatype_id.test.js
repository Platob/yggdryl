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
  // Ids live in per-category bands with reserved gaps, not a dense 0..N counter.
  const variants = [
    ['Unknown', 0x0000, 'unknown'],
    ['Bool', 0x0010, 'bool'],
    ['I8', 0x0100, 'i8'],
    ['U8', 0x0101, 'u8'],
    ['I16', 0x0102, 'i16'],
    ['U16', 0x0103, 'u16'],
    ['I32', 0x0104, 'i32'],
    ['U32', 0x0105, 'u32'],
    ['I64', 0x0106, 'i64'],
    ['U64', 0x0107, 'u64'],
    ['I128', 0x0108, 'i128'],
    ['U128', 0x0109, 'u128'],
    ['F32', 0x0201, 'f32'],
    ['F64', 0x0202, 'f64'],
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
  assert.ok(new DataTypeId(0x0106).equals(DataTypeId.I64()))
  assert.ok(DataTypeId.fromU16(0x0202).equals(DataTypeId.F64()))
  assert.equal(DataTypeId.fromU16(0).name(), 'unknown')
  // A foreign / newer id degrades to raw bytes (Unknown), never throws.
  assert.equal(DataTypeId.fromU16(999).name(), 'unknown')
  assert.equal(DataTypeId.fromU16(0x0011).name(), 'unknown') // a reserved gap in the bool band
  assert.equal(new DataTypeId(999).name(), 'unknown')
})

test('category names each type’s band; numeric / byte-like / fixed-size predicates', () => {
  assert.equal(DataTypeId.Unknown().category(), 'null')
  assert.equal(DataTypeId.Bool().category(), 'boolean')
  assert.equal(DataTypeId.I64().category(), 'integer')
  assert.equal(DataTypeId.F64().category(), 'float')
  assert.equal(DataTypeId.Decimal128().category(), 'decimal')
  assert.equal(DataTypeId.Binary().category(), 'binary')
  assert.equal(DataTypeId.FixedUtf8().category(), 'utf8')
  assert.ok(DataTypeId.I64().isNumeric() && DataTypeId.Decimal32().isNumeric())
  assert.ok(!DataTypeId.Bool().isNumeric() && !DataTypeId.Utf8().isNumeric())
  assert.ok(DataTypeId.FixedBinary().isByteLike() && DataTypeId.FixedBinary().isFixedSize())
  assert.ok(!DataTypeId.Binary().isFixedSize())
  assert.ok(!DataTypeId.I64().isTemporal() && !DataTypeId.I64().isNested())
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
// Byte types — variable-length + fixed-size binary / utf8
// -------------------------------------------------------------------------------------

test('the byte-type factories name their band ids and round-trip', () => {
  const byteTypes = [
    ['Binary', 0x0500, 'binary'],
    ['Utf8', 0x0600, 'utf8'],
    ['FixedBinary', 0x0510, 'fixed_binary'],
    ['FixedUtf8', 0x0610, 'fixed_utf8'],
  ]
  for (const [factory, id, name] of byteTypes) {
    const d = DataTypeId[factory]()
    assert.ok(d instanceof DataTypeId)
    assert.equal(d.asU16(), id, `${factory}.asU16()`)
    assert.equal(d.name(), name, `${factory}.name()`)
    assert.ok(DataTypeId.fromU16(id).equals(d), `fromU16(${id})`)
    assert.ok(DataTypeId.fromName(name).equals(d), `fromName(${name})`)
    // byte types have no id-derivable element width and are not fixed-width
    assert.equal(d.byteSize(), 0, `${factory}.byteSize()`)
    assert.equal(d.isFixedWidth(), false, `${factory}.isFixedWidth()`)
  }
})

test('byte-type predicates: isBinary / isUtf8 / isVariableLength', () => {
  const binary = DataTypeId.Binary()
  const utf8 = DataTypeId.Utf8()
  const fixedBinary = DataTypeId.FixedBinary()
  const fixedUtf8 = DataTypeId.FixedUtf8()

  // isBinary — Binary | FixedBinary
  assert.equal(binary.isBinary(), true)
  assert.equal(fixedBinary.isBinary(), true)
  assert.equal(utf8.isBinary(), false)
  assert.equal(fixedUtf8.isBinary(), false)

  // isUtf8 — Utf8 | FixedUtf8
  assert.equal(utf8.isUtf8(), true)
  assert.equal(fixedUtf8.isUtf8(), true)
  assert.equal(binary.isUtf8(), false)
  assert.equal(fixedBinary.isUtf8(), false)

  // isVariableLength — Binary | Utf8 only (the offsets + data layout)
  assert.equal(binary.isVariableLength(), true)
  assert.equal(utf8.isVariableLength(), true)
  assert.equal(fixedBinary.isVariableLength(), false)
  assert.equal(fixedUtf8.isVariableLength(), false)

  // a numeric type is none of these
  assert.equal(DataTypeId.I64().isBinary(), false)
  assert.equal(DataTypeId.I64().isUtf8(), false)
  assert.equal(DataTypeId.I64().isVariableLength(), false)
})

test('the large byte-type factories name their band ids and round-trip', () => {
  const largeTypes = [
    ['LargeBinary', 0x0502, 'large_binary'],
    ['LargeUtf8', 0x0602, 'large_utf8'],
  ]
  for (const [factory, id, name] of largeTypes) {
    const d = DataTypeId[factory]()
    assert.ok(d instanceof DataTypeId)
    assert.equal(d.asU16(), id, `${factory}.asU16()`)
    assert.equal(d.name(), name, `${factory}.name()`)
    assert.ok(DataTypeId.fromU16(id).equals(d), `fromU16(${id})`)
    assert.ok(DataTypeId.fromName(name).equals(d), `fromName(${name})`)
    // large byte types have no id-derivable element width and are not fixed-width
    assert.equal(d.byteSize(), 0, `${factory}.byteSize()`)
    assert.equal(d.isFixedWidth(), false, `${factory}.isFixedWidth()`)
  }
})

test('large byte-type predicates: isBinary / isUtf8 / isVariableLength / isLarge', () => {
  const largeBinary = DataTypeId.LargeBinary()
  const largeUtf8 = DataTypeId.LargeUtf8()

  // isLarge — LargeBinary | LargeUtf8 only
  assert.equal(largeBinary.isLarge(), true)
  assert.equal(largeUtf8.isLarge(), true)
  assert.equal(DataTypeId.Binary().isLarge(), false)
  assert.equal(DataTypeId.Utf8().isLarge(), false)
  assert.equal(DataTypeId.FixedBinary().isLarge(), false)
  assert.equal(DataTypeId.I64().isLarge(), false)

  // a large binary is a binary, variable-length, not fixed-size
  assert.equal(largeBinary.isBinary(), true)
  assert.equal(largeBinary.isUtf8(), false)
  assert.equal(largeBinary.isVariableLength(), true)
  assert.equal(largeBinary.isFixedSize(), false)

  // a large utf8 is a utf8, variable-length, not fixed-size
  assert.equal(largeUtf8.isUtf8(), true)
  assert.equal(largeUtf8.isBinary(), false)
  assert.equal(largeUtf8.isVariableLength(), true)
  assert.equal(largeUtf8.isFixedSize(), false)
})

// -------------------------------------------------------------------------------------
// Value semantics
// -------------------------------------------------------------------------------------

test('equals is identity over the element type', () => {
  assert.ok(DataTypeId.I64().equals(DataTypeId.fromU16(0x0106)))
  assert.ok(!DataTypeId.I64().equals(DataTypeId.U64()))
  assert.ok(!DataTypeId.I32().equals(DataTypeId.F32()))
})
