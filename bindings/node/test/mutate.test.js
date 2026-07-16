'use strict'

// Phase 9 — random-access mutation, mirrored from the core `dyn AnySerie` surface. Node has no
// operators / `__setitem__`, so each capability is a named camelCase method:
//   • setChildAt(index, serie) / setChildBy(name, serie) — replace / add-or-replace a nested child
//     column (struct col[i] / list item@0 / map keys@0 values@1); a leaf column is a guided error;
//   • setSlice(offset, otherSerie) — length-preserving range overwrite on a leaf column (a nested
//     column is a guided error; the source must match the target's leaf type — setSlice does NOT cast);
//   • slice(start, length) get on the leaf columns (the nested columns already have it, returning a
//     serializeBytes frame);
//   • the Phase-9 cast-anything arithmetic ops — a utf8 / decimal / temporal Serie right operand is
//     now COERCED into the LEFT type by the core (only a genuinely non-convertible cell errors).

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  StructField,
  StructSerie,
  ListSerie,
  MapSerie,
  I32Serie,
  I64Serie,
  Utf8Serie,
} = yggdryl.types
const { D64Serie } = yggdryl.decimal
const { Date32Serie } = yggdryl.temporal

// struct { x: i32, y: i32 } from two same-length arrays (non-nullable struct rows).
function numStruct(xs, ys) {
  const x = new I32Serie(xs)
  const y = new I32Serie(ys)
  const schema = new StructField('s', [x.toField('x'), y.toField('y')], false)
  return StructSerie.fromColumns(schema, [x.serializeBytes(), y.serializeBytes()])
}

// list<i32> = [[1, 2], [3]] — flattened item child [1, 2, 3], offsets [0, 2, 3].
function i32List() {
  const items = new I32Serie([1, 2, 3])
  return ListSerie.fromParts(items.toField('item'), items.serializeBytes(), [0, 2, 3])
}

// map<i32, i32> = [{1:10, 2:20}, {3:30}] — keys [1,2,3], values [10,20,30], offsets [0, 2, 3].
function i32Map() {
  const keys = new I32Serie([1, 2, 3])
  const vals = new I32Serie([10, 20, 30])
  return MapSerie.fromParts(
    keys.toField('key'),
    keys.serializeBytes(),
    vals.toField('value'),
    vals.serializeBytes(),
    [0, 2, 3],
  )
}

// ---------------------------------------------------------------------------------------
// setChildAt / setChildBy — struct (replace by index + dict-like add/replace by name)
// ---------------------------------------------------------------------------------------

test('setChildAt replaces a struct column by index (schema name preserved, type + data swapped)', () => {
  const st = numStruct([1, 2, 3], [10, 20, 30])
  st.setChildAt(0, new I64Serie(['100', '200', '300']))
  assert.equal(st.field(0).name, 'x') // the slot's schema name is preserved
  assert.equal(st.field(0).typeName, 'i64') // the type is now i64
  assert.deepEqual(I64Serie.deserializeBytes(st.columnBytes(0)).toOptions(), ['100', '200', '300'])
  assert.deepEqual(I32Serie.deserializeBytes(st.columnBytes(1)).toOptions(), [10, 20, 30]) // col 1 untouched
})

test('setChildBy replaces an existing struct column by name', () => {
  const st = numStruct([1, 2], [10, 20])
  st.setChildBy('y', new I64Serie(['77', '88']))
  assert.equal(st.numColumns, 2)
  assert.deepEqual(I64Serie.deserializeBytes(st.columnBytes(1)).toOptions(), ['77', '88'])
})

test('setChildBy adds a brand-new struct column (dict-like add)', () => {
  const st = numStruct([1, 2], [10, 20])
  st.setChildBy('score', new I32Serie([7, 8]))
  assert.equal(st.numColumns, 3)
  assert.equal(st.field(2).name, 'score')
  assert.deepEqual(I32Serie.deserializeBytes(st.columnBytes(2)).toOptions(), [7, 8])
})

// ---------------------------------------------------------------------------------------
// setChildAt / setChildBy — list item + map key/value
// ---------------------------------------------------------------------------------------

test('setChildAt(0) replaces the list item child (offsets unchanged)', () => {
  const list = i32List()
  list.setChildAt(0, new I64Serie(['100', '200', '300']))
  assert.deepEqual(list.offsets, [0, 2, 3])
  assert.deepEqual(I64Serie.deserializeBytes(list.itemBytes()).toOptions(), ['100', '200', '300'])
})

test('setChildBy("item") replaces the list item child', () => {
  const list = i32List()
  list.setChildBy('item', new I64Serie(['9', '8', '7']))
  assert.deepEqual(I64Serie.deserializeBytes(list.itemBytes()).toOptions(), ['9', '8', '7'])
})

test('a non-zero list child index is a guided error', () => {
  assert.throws(() => i32List().setChildAt(1, new I32Serie([1, 2, 3])), /list|index|item/i)
})

test('setChildAt replaces the map keys (0) and values (1) columns', () => {
  const map = i32Map()
  map.setChildAt(0, new I64Serie(['5', '6', '7'])) // keys (must stay non-null)
  map.setChildAt(1, new I64Serie(['50', '60', '70'])) // values
  assert.deepEqual(I64Serie.deserializeBytes(map.keys()).toOptions(), ['5', '6', '7'])
  assert.deepEqual(I64Serie.deserializeBytes(map.values()).toOptions(), ['50', '60', '70'])
})

test('setChildBy replaces the map key / value columns by name', () => {
  const map = i32Map()
  map.setChildBy('key', new I64Serie(['5', '6', '7']))
  map.setChildBy('value', new I64Serie(['50', '60', '70']))
  assert.deepEqual(I64Serie.deserializeBytes(map.keys()).toOptions(), ['5', '6', '7'])
  assert.deepEqual(I64Serie.deserializeBytes(map.values()).toOptions(), ['50', '60', '70'])
})

// ---------------------------------------------------------------------------------------
// setChild guided errors: length mismatch, leaf column, non-Serie child
// ---------------------------------------------------------------------------------------

test('setChildAt with a wrong-length child throws a guided error', () => {
  assert.throws(() => numStruct([1, 2, 3], [10, 20, 30]).setChildAt(0, new I32Serie([1, 2])), /length|rows/i)
})

test('setChildAt on a leaf column is a guided error (not nested)', () => {
  assert.throws(() => new I32Serie([1, 2, 3]).setChildAt(0, new I32Serie([4, 5, 6])), /nested|leaf|set/i)
})

test('setChildBy on a leaf column is a guided error (not nested)', () => {
  assert.throws(() => new I32Serie([1, 2, 3]).setChildBy('x', new I32Serie([4, 5, 6])), /nested|leaf|set/i)
})

test('setChildAt with a non-Serie child throws a guided error', () => {
  assert.throws(() => numStruct([1, 2], [10, 20]).setChildAt(0, 42), /Serie column/i)
})

// ---------------------------------------------------------------------------------------
// setSlice — length-preserving range overwrite (leaf) + guided errors
// ---------------------------------------------------------------------------------------

test('setSlice overwrites a range in place, reads back, keeps the length', () => {
  const col = new I32Serie([0, 0, 0, 0, 0])
  col.setSlice(1, new I32Serie([7, null])) // overwrite rows 1..3 (a null cell is preserved)
  assert.deepEqual(col.toOptions(), [0, 7, null, 0, 0])
  assert.equal(col.length, 5) // length preserved
})

test('setSlice on a var (utf8) column overwrites the range', () => {
  const col = new Utf8Serie(['a', 'b', 'c', 'd'])
  col.setSlice(1, new Utf8Serie(['X', 'Y']))
  assert.deepEqual(col.toOptions(), ['a', 'X', 'Y', 'd'])
})

test('setSlice past the end is a guided out-of-range error', () => {
  assert.throws(() => new I32Serie([1, 2, 3]).setSlice(2, new I32Serie([9, 9, 9])), /range|bound|index/i)
})

test('setSlice with a mismatched source type is a guided error (setSlice does not cast)', () => {
  assert.throws(() => new I32Serie([0, 0, 0]).setSlice(0, new Utf8Serie(['a', 'b', 'c'])), /type|mismatch|expected/i)
})

test('setSlice on a nested column is a guided error (use setChildAt / setChildBy)', () => {
  assert.throws(() => numStruct([1, 2], [10, 20]).setSlice(0, numStruct([3, 4], [30, 40])), /nested|setChild|set_child/i)
})

test('setSlice with a non-Serie source throws a guided error', () => {
  assert.throws(() => new I32Serie([0, 0, 0]).setSlice(0, 5), /Serie column/i)
})

// ---------------------------------------------------------------------------------------
// slice — the range GET on leaf columns (same-class), clamped, never throws
// ---------------------------------------------------------------------------------------

test('slice returns a fresh same-class sub-column', () => {
  const out = new I32Serie([1, 2, 3, 4, 5]).slice(1, 2)
  assert.ok(out instanceof I32Serie)
  assert.deepEqual(out.toOptions(), [2, 3])
})

test('slice clamps an over-long length to the column end (never throws)', () => {
  assert.deepEqual(new I32Serie([1, 2, 3, 4, 5]).slice(3, 10).toOptions(), [4, 5])
})

test('slice on a utf8 column preserves values and null-ness', () => {
  assert.deepEqual(new Utf8Serie(['a', null, 'c', 'd']).slice(1, 2).toOptions(), [null, 'c'])
})

test('slice on a decimal column keeps its (precision, scale)', () => {
  const out = new D64Serie(4, 2, ['1.50', '2.50', '3.50']).slice(1, 2)
  assert.ok(out instanceof D64Serie)
  assert.deepEqual(out.toOptions(), ['2.50', '3.50'])
})

// ---------------------------------------------------------------------------------------
// cast-anything arithmetic ops — a utf8 / decimal / temporal Serie right operand coerces
// ---------------------------------------------------------------------------------------

test('cast-anything: an i64 column + a utf8 column of numeric strings coerces element-wise', () => {
  assert.deepEqual(new I64Serie(['1', '2']).add(new Utf8Serie(['10', '20'])).toOptions(), ['11', '22'])
})

test('cast-anything: an i64 column + a decimal column of whole values coerces', () => {
  // The decimal 3.00 casts to the i64 3; 5 + 3 = 8 (result follows the LEFT i64 column).
  assert.deepEqual(new I64Serie(['5']).add(new D64Serie(4, 2, ['3.00'])).toOptions(), ['8'])
})

test('cast-anything: an i64 column + a temporal column coerces via the raw count', () => {
  // Date32 unit "d": 1970-01-02 is 1 day past the epoch (raw count 1); 5 + 1 = 6.
  const days = new Date32Serie('d', null, ['1970-01-02'])
  assert.deepEqual(new I64Serie(['5']).add(days).toOptions(), ['6'])
})

test('cast-anything: a genuinely non-numeric utf8 operand is a guided parse error', () => {
  assert.throws(() => new I64Serie(['1']).add(new Utf8Serie(['a'])), /cannot parse|utf8 operand|number/i)
})
