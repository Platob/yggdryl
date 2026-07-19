'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Serie, ByteSerie, Field } = yggdryl.typed
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

// -------------------------------------------------------------------------------------
// Fixed-point decimals — the docs/typed.md "Fixed-point decimals" tab
// -------------------------------------------------------------------------------------

test('Decimal128 money: unscaled BigInt get + scale-aware toDecimalString', () => {
  // Money as Decimal128 scale 2: the stored value is the unscaled integer.
  const col = Serie.fromValues([12345n, 5n, -5n], DataTypeId.Decimal128()).withPrecisionScale(10, 2)
  assert.equal(col.get(0), 12345n) // raw unscaled value
  assert.equal(col.toDecimalString(0), '123.45') // scale-aware string
  assert.equal(col.toDecimalString(1), '0.05')
  assert.equal(col.toDecimalString(2), '-0.05')
  assert.ok(col.dtype().equals(DataTypeId.Decimal128()))
  // precision/scale live in the Field metadata
  assert.equal(col.field().precision(), 10)
  assert.equal(col.field().scale(), 2)
  // Serie mirrors of the same metadata
  assert.equal(col.decimalPrecision(), 10)
  assert.equal(col.decimalScale(), 2)
})

test('Decimal32 crosses as a number (i32), not a BigInt', () => {
  const col = Serie.fromValues([12345, 5, -5], DataTypeId.Decimal32()).withPrecisionScale(9, 2)
  assert.equal(col.get(0), 12345) // a plain number
  assert.equal(col.toDecimalString(0), '123.45')
  assert.equal(col.toDecimalString(2), '-0.05')
  assert.ok(col.dtype().equals(DataTypeId.Decimal32()))
})

test('all four decimal widths format the same unscaled money value', () => {
  const d32 = Serie.fromValues([12345], DataTypeId.Decimal32()).withPrecisionScale(9, 2)
  const d64 = Serie.fromValues([12345n], DataTypeId.Decimal64()).withPrecisionScale(18, 2)
  const d128 = Serie.fromValues([12345n], DataTypeId.Decimal128()).withPrecisionScale(38, 2)
  const d256 = Serie.fromValues([12345n], DataTypeId.Decimal256()).withPrecisionScale(76, 2)
  assert.equal(d32.toDecimalString(0), '123.45')
  assert.equal(d64.toDecimalString(0), '123.45')
  assert.equal(d128.toDecimalString(0), '123.45')
  assert.equal(d256.toDecimalString(0), '123.45')
  // each reports its own decimal dtype
  assert.ok(d32.dtype().equals(DataTypeId.Decimal32()))
  assert.ok(d64.dtype().equals(DataTypeId.Decimal64()))
  assert.ok(d128.dtype().equals(DataTypeId.Decimal128()))
  assert.ok(d256.dtype().equals(DataTypeId.Decimal256()))
})

test('Decimal256 fits i128 and beyond i128, round-tripping through toDecimalString', () => {
  // Values that fit i128 take the fast I256::from_i128 path.
  const small = Serie.fromValues([1n, -5n], DataTypeId.Decimal256())
  assert.equal(small.get(0), 1n)
  assert.equal(small.get(1), -5n)
  assert.equal(small.toDecimalString(0), '1') // default scale 0 -> plain integer
  assert.equal(small.toDecimalString(1), '-5')
  // Decimal256's precision defaults to the type max (76) when unset.
  assert.equal(small.decimalPrecision(), 76)
  assert.equal(small.field().precision(), null) // Field carries it only once set

  // A value beyond i128 (2^130 = 4 * 2^128) crosses as an arbitrary-precision BigInt.
  const big = 2n ** 130n
  assert.ok(big > (2n ** 127n - 1n)) // beyond i128::MAX
  const wide = Serie.fromValues([big, -big], DataTypeId.Decimal256())
  assert.equal(wide.get(0), big)
  assert.equal(wide.get(1), -big)
  assert.equal(wide.toDecimalString(0), big.toString())
  assert.equal(wide.toDecimalString(1), (-big).toString())
  assert.deepEqual(wide.values(), [big, -big])

  // A BigInt past the 256-bit range is refused with a guided error.
  assert.throws(() => Serie.fromValues([2n ** 300n], DataTypeId.Decimal256()), /out of range/)
})

test('a nullable decimal column keeps its validity, metadata, and null-aware formatting', () => {
  const col = Serie.fromOptions([12345n, null, -5n], DataTypeId.Decimal128()).withPrecisionScale(10, 2)
  assert.equal(col.len(), 3)
  assert.equal(col.nullCount(), 1)
  assert.equal(col.get(0), 12345n)
  assert.equal(col.get(1), null) // the null
  assert.equal(col.toDecimalString(0), '123.45')
  assert.equal(col.toDecimalString(1), null) // a null element formats as null
  assert.equal(col.toDecimalString(2), '-0.05')
  assert.equal(col.field().nullable(), true)
  assert.equal(col.field().precision(), 10)
  assert.equal(col.field().scale(), 2)
})

test('withName and withPrecisionScale preserve each other (either order)', () => {
  const a = Serie.fromValues([12345n], DataTypeId.Decimal128()).withName('price').withPrecisionScale(10, 2)
  assert.equal(a.field().name(), 'price')
  assert.equal(a.field().precision(), 10)
  assert.equal(a.field().scale(), 2)

  const b = Serie.fromValues([12345n], DataTypeId.Decimal128()).withPrecisionScale(10, 2).withName('price')
  assert.equal(b.field().name(), 'price')
  assert.equal(b.field().precision(), 10)
  assert.equal(b.field().scale(), 2)
})

test('a decimal column does not reduce', () => {
  const col = Serie.fromValues([12345n, 5n], DataTypeId.Decimal128()).withPrecisionScale(10, 2)
  assert.throws(() => col.sum(), /decimal column does not reduce/)
  assert.throws(() => col.min(), /decimal column does not reduce/)
  assert.throws(() => col.max(), /decimal column does not reduce/)
  assert.throws(() => col.mean(), /decimal column does not reduce/)
})

test('the decimal-only methods refuse a non-decimal column', () => {
  const ints = Serie.fromValues([1n, 2n], DataTypeId.I64())
  assert.throws(() => ints.toDecimalString(0), /not a decimal column/)
  assert.throws(() => ints.decimalPrecision(), /not a decimal column/)
  assert.throws(() => ints.decimalScale(), /not a decimal column/)
})

test('a wrong decimal element shape throws a guided error', () => {
  // Decimal32 takes a number; a bigint is the wrong shape.
  assert.throws(() => Serie.fromValues([12345n], DataTypeId.Decimal32()), /expected a JS number/)
  // Decimal64/128/256 take a bigint; a plain number is the wrong shape.
  assert.throws(() => Serie.fromValues([12345], DataTypeId.Decimal128()), /expected a JS bigint/)
  assert.throws(() => Serie.fromValues([12345], DataTypeId.Decimal256()), /expected a JS bigint/)
})

// -------------------------------------------------------------------------------------
// DataTypeId decimal factories
// -------------------------------------------------------------------------------------

test('DataTypeId decimal factories name their widths', () => {
  assert.equal(DataTypeId.Decimal32().name(), 'decimal32')
  assert.equal(DataTypeId.Decimal64().name(), 'decimal64')
  assert.equal(DataTypeId.Decimal128().name(), 'decimal128')
  assert.equal(DataTypeId.Decimal256().name(), 'decimal256')
  assert.equal(DataTypeId.Decimal32().byteSize(), 4)
  assert.equal(DataTypeId.Decimal256().byteSize(), 32)
  assert.ok(DataTypeId.fromName('decimal128').equals(DataTypeId.Decimal128()))
})

// -------------------------------------------------------------------------------------
// ByteSerie — variable-length + fixed-size byte columns (binary / utf8)
// -------------------------------------------------------------------------------------

test('ByteSerie: a variable-length binary column from Buffers', () => {
  const values = [Buffer.from([1, 2, 3]), Buffer.from([]), Buffer.from([255, 254])]
  const col = ByteSerie.fromValues(values, DataTypeId.Binary())
  assert.equal(col.len(), 3)
  assert.equal(col.isEmpty(), false)

  const first = col.get(0)
  assert.ok(Buffer.isBuffer(first)) // a binary element crosses as a Buffer
  assert.deepEqual(first, Buffer.from([1, 2, 3]))
  assert.deepEqual(col.get(1), Buffer.from([])) // an empty blob
  assert.deepEqual(col.get(2), Buffer.from([255, 254]))
  assert.equal(col.get(3), null) // out of range

  assert.deepEqual(col.toList(), values)
  assert.deepEqual(col.values(), values)

  assert.ok(col.dtype().equals(DataTypeId.Binary()))
  assert.equal(col.width(), null) // variable-length -> no fixed width
  assert.equal(col.nullCount(), 0)
  assert.ok(col.field().dtype().equals(DataTypeId.Binary()))
  assert.equal(col.field().nullable(), false)
  assert.equal(col.field().byteWidth(), null)
  assert.match(col.toString(), /ByteSerie\(/)
})

test('ByteSerie: a utf8 column round-trips a multibyte string; withName copies', () => {
  const col = ByteSerie.fromValues(['hello', 'café', '日本語', ''], DataTypeId.Utf8())
  assert.equal(col.len(), 4)
  assert.equal(col.get(0), 'hello')
  assert.equal(col.get(1), 'café') // é is 2 UTF-8 bytes
  assert.equal(col.get(2), '日本語') // 3 chars, 9 bytes
  assert.equal(col.get(3), '') // empty string
  assert.deepEqual(col.toList(), ['hello', 'café', '日本語', ''])
  assert.deepEqual(col.values(), ['hello', 'café', '日本語', ''])
  assert.ok(col.dtype().equals(DataTypeId.Utf8()))
  assert.equal(col.width(), null)

  // withName produces a named copy over the same bytes; the original is unchanged
  const named = col.withName('greeting')
  assert.equal(named.field().name(), 'greeting')
  assert.equal(named.get(2), '日本語')
  assert.equal(col.field().name(), null)
})

test('ByteSerie: a nullable binary column via fromOptions', () => {
  const col = ByteSerie.fromOptions([Buffer.from([1]), null, Buffer.from([2, 3])], DataTypeId.Binary())
  assert.equal(col.len(), 3)
  assert.equal(col.nullCount(), 1)
  assert.deepEqual(col.get(0), Buffer.from([1]))
  assert.equal(col.get(1), null) // the null
  assert.deepEqual(col.get(2), Buffer.from([2, 3]))
  assert.ok(col.isNull(1) && col.isValid(0))
  assert.equal(col.isValid(1), false)
  assert.deepEqual(col.toList(), [Buffer.from([1]), null, Buffer.from([2, 3])])
  assert.equal(col.field().nullable(), true)
})

test('ByteSerie: a fixed_binary column zero-pads short and truncates long values', () => {
  const col = ByteSerie.fromValues(
    [Buffer.from([1, 2]), Buffer.from([9, 9, 9, 9, 9, 9])],
    DataTypeId.FixedBinary(),
    4
  )
  assert.equal(col.len(), 2)
  assert.equal(col.width(), 4) // the fixed element byte width
  assert.equal(col.field().byteWidth(), 4)

  assert.deepEqual(col.get(0), Buffer.from([1, 2, 0, 0])) // short value zero-padded to 4
  assert.deepEqual(col.get(1), Buffer.from([9, 9, 9, 9])) // long value truncated to 4
  assert.ok(col.dtype().equals(DataTypeId.FixedBinary()))
  assert.equal(col.field().nullable(), false)
  assert.match(col.toString(), /width=4/)
})

test('ByteSerie: a nullable fixed_utf8 column', () => {
  const col = ByteSerie.fromOptions(['ab', null, 'cd'], DataTypeId.FixedUtf8(), 4)
  assert.equal(col.len(), 3)
  assert.equal(col.width(), 4)
  assert.equal(col.nullCount(), 1)
  assert.equal(col.get(1), null) // the null
  // a non-null element is decoded over the full fixed width (short value zero-padded)
  assert.ok(col.get(0).startsWith('ab'))
  assert.ok(col.dtype().equals(DataTypeId.FixedUtf8()))
  assert.equal(col.field().nullable(), true)
  assert.equal(col.field().byteWidth(), 4)
})

test('ByteSerie: guided errors on the build side', () => {
  // a fixed-size dtype needs a width
  assert.throws(
    () => ByteSerie.fromValues([Buffer.from([1])], DataTypeId.FixedBinary()),
    /fixed-size column needs a width/
  )
  // a variable-length dtype takes no width
  assert.throws(
    () => ByteSerie.fromValues([Buffer.from([1])], DataTypeId.Binary(), 4),
    /variable-length column takes no width/
  )
  // a non-byte dtype is refused
  assert.throws(
    () => ByteSerie.fromValues([Buffer.from([1])], DataTypeId.I64()),
    /not a byte column/
  )
  // a string where a Buffer (binary) is required
  assert.throws(
    () => ByteSerie.fromValues(['nope'], DataTypeId.Binary()),
    /expected a Buffer element for a binary column/
  )
  // a Buffer where a string (utf8) is required
  assert.throws(
    () => ByteSerie.fromValues([Buffer.from([1])], DataTypeId.Utf8()),
    /expected a string element for a utf8 column/
  )
})
