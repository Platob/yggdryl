'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Serie, Field } = yggdryl.typed
const { DataTypeId } = yggdryl.datatype_id

// -------------------------------------------------------------------------------------
// Build a column and reduce it — the docs/typed.md "Build a column and reduce it" tab
// -------------------------------------------------------------------------------------

test('fromValues builds a column and the vectorized reductions run over it', () => {
  const col = Serie.fromValues([4n, 8n, 15n, 16n, 23n, 42n], DataTypeId.I64())
  assert.equal(col.len(), 6)
  assert.equal(col.isEmpty(), false)
  assert.equal(col.get(0), 4n)
  assert.deepEqual(col.toList(), [4n, 8n, 15n, 16n, 23n, 42n])
  assert.deepEqual(col.values(), [4n, 8n, 15n, 16n, 23n, 42n]) // raw values
  assert.equal(col.sum(), 108n) // vectorized reduction over the data buffer
  assert.equal(col.min(), 4n)
  assert.equal(col.max(), 42n)
  assert.equal(col.mean(), 18.0)
  assert.equal(col.nullCount(), 0)
  assert.ok(col.dtype().equals(DataTypeId.I64()))
})

// -------------------------------------------------------------------------------------
// Nulls — the docs/typed.md "Nulls — a nullable column" tab
// -------------------------------------------------------------------------------------

test('fromOptions builds the validity bitmap; get/isNull/nullCount are null-aware', () => {
  const col = Serie.fromOptions([1, null, 3, null, 5], DataTypeId.I32())
  assert.equal(col.len(), 5)
  assert.equal(col.nullCount(), 2)
  assert.equal(col.get(0), 1)
  assert.equal(col.get(1), null) // the null
  assert.ok(col.isNull(1) && col.isValid(0))
  assert.equal(col.isValid(1), false)
  assert.equal(JSON.stringify(col.toList()), '[1,null,3,null,5]')
  // raw values surface the stored default (0) in null slots
  assert.deepEqual(col.values(), [1, 0, 3, 0, 5])
  // a fromOptions column is nullable (it carries a validity buffer)
  assert.equal(col.field().nullable(), true)
})

// -------------------------------------------------------------------------------------
// A column's Field — the docs/typed.md "A column's Field" tab
// -------------------------------------------------------------------------------------

test('Field describes name / dtype / nullable; a column reports its own field', () => {
  const field = new Field('price', DataTypeId.I64(), true)
  assert.equal(field.name(), 'price')
  assert.ok(field.dtype().equals(DataTypeId.I64()))
  assert.equal(field.nullable(), true)

  const col = Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()).withName('id')
  assert.equal(col.field().name(), 'id')
  assert.equal(col.field().nullable(), false) // no nulls -> non-nullable
  assert.ok(col.field().dtype().equals(DataTypeId.I64()))
})

test('Field carries a Headers copy, equals, and toString', () => {
  const a = new Field('id', DataTypeId.I32(), false)
  const b = new Field('id', DataTypeId.I32(), false)
  const c = new Field('id', DataTypeId.I32(), true)
  assert.ok(a.equals(b))
  assert.ok(!a.equals(c))
  const headers = a.headers() // the binding headers.Headers
  assert.equal(headers.name(), 'id')
  assert.ok(headers.typeId().equals(DataTypeId.I32()))
  assert.equal(headers.nullable(), false)
  assert.match(a.toString(), /Field\(/)

  // an unnamed field
  const unnamed = new Field(null, DataTypeId.F64(), true)
  assert.equal(unnamed.name(), null)
  assert.ok(unnamed.dtype().equals(DataTypeId.F64()))
})

// -------------------------------------------------------------------------------------
// Edges
// -------------------------------------------------------------------------------------

test('an empty column: len 0, empty, sum 0, min/max/mean null', () => {
  const col = Serie.fromValues([], DataTypeId.I64())
  assert.equal(col.len(), 0)
  assert.equal(col.isEmpty(), true)
  assert.equal(col.nullCount(), 0)
  assert.deepEqual(col.toList(), [])
  assert.equal(col.sum(), 0n)
  assert.equal(col.min(), null)
  assert.equal(col.max(), null)
  assert.equal(col.mean(), null)
})

test('an all-null column', () => {
  const col = Serie.fromOptions([null, null, null], DataTypeId.I32())
  assert.equal(col.len(), 3)
  assert.equal(col.nullCount(), 3)
  assert.equal(col.get(0), null)
  assert.equal(col.isValid(0), false)
  assert.deepEqual(col.toList(), [null, null, null])
})

test('out-of-range get returns null', () => {
  const col = Serie.fromValues([10, 20, 30], DataTypeId.I32())
  assert.equal(col.get(2), 30)
  assert.equal(col.get(3), null)
  assert.equal(col.get(1000), null)
  assert.equal(col.isValid(1000), false) // out of range is never valid
  assert.equal(col.isNull(1000), true) // ... and is_null == !is_valid
})

test('wide unsigned/signed 128-bit values round-trip via BigInt', () => {
  const big = 10000000000000000000n // > 2^53 and > u64/2, exact only as a BigInt
  const u = Serie.fromValues([1n, 2n, big], DataTypeId.U128())
  assert.equal(u.get(2), big)
  assert.equal(u.sum(), 1n + 2n + big)
  assert.equal(u.max(), big)

  const i = Serie.fromValues([-5n, 7n], DataTypeId.I128())
  assert.equal(i.get(0), -5n)
  assert.equal(i.sum(), 2n)
  assert.equal(i.min(), -5n)

  const u64 = Serie.fromValues([100n, 200n], DataTypeId.U64())
  assert.equal(u64.sum(), 300n)
  assert.equal(u64.get(1), 200n)
})

test('float column: sum/mean are numbers; min/max ignore NaN', () => {
  const col = Serie.fromValues([1.5, 2.5, 4.0], DataTypeId.F64())
  assert.equal(col.sum(), 8.0)
  assert.equal(col.mean(), 8.0 / 3)
  assert.equal(col.get(0), 1.5)

  const withNan = Serie.fromValues([1.0, NaN, 3.0], DataTypeId.F64())
  assert.equal(withNan.min(), 1.0) // NaN ignored
  assert.equal(withNan.max(), 3.0) // NaN ignored

  const f32 = Serie.fromValues([1.0, 2.0], DataTypeId.F32())
  assert.equal(f32.sum(), 3.0)
})

test('narrow integer columns cross as numbers', () => {
  const i8 = Serie.fromValues([-1, 2, 127], DataTypeId.I8())
  assert.equal(i8.get(0), -1)
  assert.equal(i8.get(2), 127)
  assert.equal(i8.sum(), 128n) // integer sums cross as BigInt
  assert.equal(i8.max(), 127)

  const u8 = Serie.fromValues([0, 255], DataTypeId.U8())
  assert.equal(u8.get(1), 255)
  assert.equal(u8.sum(), 255n)
})

test('a boolean column stores/reads booleans and refuses to reduce', () => {
  const col = Serie.fromValues([true, false, true], DataTypeId.Bool())
  assert.equal(col.len(), 3)
  assert.equal(col.get(0), true)
  assert.equal(col.get(1), false)
  assert.deepEqual(col.toList(), [true, false, true])
  assert.ok(col.dtype().equals(DataTypeId.Bool()))
  // Bit is not Reduce — the numeric aggregations throw the guided error
  assert.throws(() => col.sum(), /boolean column does not reduce/)
  assert.throws(() => col.min(), /boolean column does not reduce/)
  assert.throws(() => col.max(), /boolean column does not reduce/)
  assert.throws(() => col.mean(), /boolean column does not reduce/)
})

test('filter compacts by a boolean array or a boolean Serie mask', () => {
  const col = Serie.fromValues([4n, 8n, 15n, 16n, 23n, 42n], DataTypeId.I64())

  const byArray = col.filter([true, false, true, false, true, false])
  assert.deepEqual(byArray.toList(), [4n, 15n, 23n])

  const mask = Serie.fromValues([false, false, false, true, true, true], DataTypeId.Bool())
  const bySerie = col.filter(mask)
  assert.deepEqual(bySerie.toList(), [16n, 23n, 42n])

  // a non-boolean Serie mask is refused with a guided error
  assert.throws(() => col.filter(Serie.fromValues([1n], DataTypeId.I64())), /boolean column/)
})

test('withName does not mutate the original and clears no data', () => {
  const col = Serie.fromValues([1n, 2n, 3n], DataTypeId.I64())
  const named = col.withName('id')
  assert.equal(named.field().name(), 'id')
  assert.deepEqual(named.toList(), [1n, 2n, 3n])
  assert.equal(col.field().name(), null) // original unchanged
})

// -------------------------------------------------------------------------------------
// Guided errors on the build side
// -------------------------------------------------------------------------------------

test('a wrong element shape throws a guided error', () => {
  // a plain number where a bigint (i64) is required
  assert.throws(() => Serie.fromValues([4], DataTypeId.I64()), /expected a JS bigint/)
  // a bigint where a number (i32) is required
  assert.throws(() => Serie.fromValues([4n], DataTypeId.I32()), /expected a JS number/)
  // a number where a boolean is required
  assert.throws(() => Serie.fromValues([1], DataTypeId.Bool()), /expected a JS boolean/)
  // Unknown has no typed column
  assert.throws(() => Serie.fromValues([1n], DataTypeId.Unknown()), /no typed Serie/)
})
