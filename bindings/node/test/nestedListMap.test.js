'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  DataType,
  Field,
  StructField,
  StructSerie,
  ListField,
  ListSerie,
  MapField,
  MapSerie,
  I32Serie,
  I64Serie,
  Utf8Serie,
} = yggdryl.types
const { D64Serie } = yggdryl.decimal

// ---- exposure --------------------------------------------------------------------------

test('the types namespace exposes the list/map nested classes', () => {
  for (const cls of [ListField, ListSerie, MapField, MapSerie]) {
    assert.equal(typeof cls, 'function')
  }
})

// ---- ListField -------------------------------------------------------------------------

test('list field shape and value semantics', () => {
  const schema = new ListField('scores', new Field('item', DataType.i32(), true), true)
  assert.equal(schema.name, 'scores')
  assert.equal(schema.typeName, 'list')
  assert.ok(schema.nullable)
  assert.equal(schema.dataType.name, 'list')
  assert.ok(schema.item instanceof Field)
  assert.equal(schema.item.name, 'item')

  const other = new ListField('scores', new Field('item', DataType.i32(), true), true)
  assert.ok(schema.equals(other))
  assert.equal(schema.hashCode(), other.hashCode())
  assert.ok(ListField.deserializeBytes(schema.serializeBytes()).equals(schema))
  assert.ok(schema.copy().equals(schema))

  // Immutable builders.
  const renamed = schema.withName('vals').withNullable(false)
  assert.ok(schema.name === 'scores' && schema.nullable)
  assert.ok(renamed.name === 'vals' && !renamed.nullable)
})

// ---- ListSerie -------------------------------------------------------------------------

// List<i32>: two rows over the flat child [10, 20, 30, 40] -> row0 = [10,20,30], row1 = [40].
function scores() {
  const child = new I32Serie([10, 20, 30, 40])
  return ListSerie.fromParts(child.toField('item'), child.serializeBytes(), [0, 3, 4])
}

test('list serie build via fromParts and navigate', () => {
  const col = scores()
  assert.equal(col.length, 2)
  assert.equal(col.nullCount, 0)
  assert.equal(col.hasNulls, false)
  assert.deepEqual(col.offsets, [0, 3, 4])
  assert.equal(col.dataType.name, 'list')

  // The flattened child crosses as bytes; reconstruct it with the matching Serie class.
  const back = I32Serie.deserializeBytes(col.itemBytes())
  assert.equal(back.length, 4)
  assert.equal(back.get(3), 40)

  const field = col.toField('scores')
  assert.ok(field instanceof ListField)
  assert.equal(field.name, 'scores')
  assert.equal(field.item.name, 'item')
})

test('list serie serialize round trip and copy', () => {
  const col = scores()
  assert.ok(ListSerie.deserializeBytes(col.serializeBytes()).equals(col))
  assert.ok(col.copy().equals(col))
  assert.ok(col.toString().startsWith('ListSerie(len=2'))
})

test('list serie carries null list rows via the present mask', () => {
  // 3 rows: [10, 20], null, [30] over the flat child [10, 20, 30].
  const child = new I32Serie([10, 20, 30])
  const col = ListSerie.fromParts(
    child.toField('item'),
    child.serializeBytes(),
    [0, 2, 2, 3],
    [true, false, true],
  )
  assert.equal(col.length, 3)
  assert.equal(col.nullCount, 1)
  assert.ok(col.hasNulls)
  assert.ok(ListSerie.deserializeBytes(col.serializeBytes()).equals(col))
})

// ---- MapField --------------------------------------------------------------------------

test('map field shape and value semantics', () => {
  const schema = new MapField(
    'counts',
    new Field('key', DataType.utf8(), false),
    new Field('value', DataType.i64(), true),
    true,
    false,
  )
  assert.equal(schema.name, 'counts')
  assert.equal(schema.typeName, 'map')
  assert.ok(schema.nullable)
  assert.equal(schema.keysSorted, false)
  assert.equal(schema.key.name, 'key')
  assert.equal(schema.value.name, 'value')
  assert.equal(schema.dataType.name, 'map')

  const other = new MapField(
    'counts',
    new Field('key', DataType.utf8(), false),
    new Field('value', DataType.i64(), true),
    true,
    false,
  )
  assert.ok(schema.equals(other))
  assert.equal(schema.hashCode(), other.hashCode())
  assert.ok(MapField.deserializeBytes(schema.serializeBytes()).equals(schema))
  assert.ok(schema.copy().equals(schema))
  assert.ok(schema.withKeysSorted(true).keysSorted)
})

// ---- MapSerie --------------------------------------------------------------------------

// Map<utf8, i64>: two rows over 3 entries -> row0 = {a->1, b->2}, row1 = {c->3}.
function counts() {
  const keys = new Utf8Serie(['a', 'b', 'c'])
  const values = new I64Serie(['1', '2', '3']) // i64 values cross as strings
  return MapSerie.fromParts(
    keys.toField('key'),
    keys.serializeBytes(),
    values.toField('value'),
    values.serializeBytes(),
    [0, 2, 3],
  )
}

test('map serie build via fromParts and navigate', () => {
  const col = counts()
  assert.equal(col.length, 2)
  assert.equal(col.nullCount, 0)
  assert.equal(col.keysSorted, false)
  assert.deepEqual(col.offsets, [0, 2, 3])
  assert.equal(col.dataType.name, 'map')

  // The flattened key/value columns cross as bytes.
  const keysBack = Utf8Serie.deserializeBytes(col.keys())
  assert.equal(keysBack.get(1), 'b')
  const valuesBack = I64Serie.deserializeBytes(col.values())
  assert.equal(valuesBack.get(2), '3')

  const field = col.toField('counts')
  assert.ok(field instanceof MapField)
  assert.equal(field.key.name, 'key')
  assert.equal(field.value.name, 'value')
})

test('map serie get_value_bytes probes by serialized key', () => {
  const col = counts()
  // Probe key bytes are a leaf key's canonical bytes (utf8 -> the raw UTF-8 bytes).
  const i64le = (n) => {
    const b = Buffer.alloc(8)
    b.writeBigInt64LE(BigInt(n))
    return b
  }
  assert.ok(col.getValueBytes(0, Buffer.from('b')).equals(i64le(2))) // row0 {a->1, b->2}
  assert.ok(col.getValueBytes(0, Buffer.from('a')).equals(i64le(1)))
  assert.ok(col.getValueBytes(1, Buffer.from('c')).equals(i64le(3))) // row1 {c->3}
  assert.equal(col.getValueBytes(0, Buffer.from('z')), null) // absent key
  assert.equal(col.getValueBytes(1, Buffer.from('a')), null) // a is not in row 1
})

test('map serie serialize round trip and copy', () => {
  const col = counts()
  assert.ok(MapSerie.deserializeBytes(col.serializeBytes()).equals(col))
  assert.ok(col.copy().equals(col))
  assert.ok(col.toString().startsWith('MapSerie(len=2'))
})

// ---- struct-of-nested build (exercises the four-way child field union) ------------------

test('struct serie holds a list child, built from column bytes within Node', () => {
  const listCol = scores() // List<i32>, length 2
  const tag = new Utf8Serie(['x', 'y']) // length 2
  const schema = new StructField(
    'rec',
    [listCol.toField('scores'), tag.toField('tag')],
    false,
  )
  const struct = StructSerie.fromColumns(schema, [listCol.serializeBytes(), tag.serializeBytes()])
  assert.equal(struct.numColumns, 2)
  assert.ok(struct.field(0) instanceof ListField) // the four-way union returns the ListField
  assert.equal(struct.length, 2)

  // The list child round-trips out of the struct frame.
  const childList = ListSerie.deserializeBytes(struct.columnBytesNamed('scores'))
  assert.ok(childList.equals(listCol))
  assert.ok(StructSerie.deserializeBytes(struct.serializeBytes()).equals(struct))
})

// ---- ListSerie deep get/set -------------------------------------------------------------

test('list serie getAt / setAt address the flattened item child (child 0)', () => {
  const col = scores() // child [10,20,30,40], rows [10,20,30] / [40]
  assert.equal(col.getAt([0, 0]), 10) // flattened item 0
  assert.equal(col.getAt([0, 3]), 40)
  assert.equal(col.getPath('[0][2]'), 30) // index-terminal path -> the cell
  col.setAt([0, 0], 99)
  assert.equal(col.getAt([0, 0]), 99)
  assert.throws(() => col.getAt([0, 99]), /out of bounds/)
})

test('list serie get / childAt expose the row items and the flattened child', () => {
  const col = scores()
  assert.equal(col.numChildren(), 1)
  const row0 = I32Serie.deserializeBytes(col.get(0)) // row 0 = [10,20,30]
  assert.equal(row0.length, 3)
  assert.equal(row0.get(2), 30)
  const child = I32Serie.deserializeBytes(col.childAt(0)) // the flat child [10,20,30,40]
  assert.equal(child.length, 4)
  // A name-terminal path returns the item sub-column frame.
  const byName = I32Serie.deserializeBytes(col.getPath('item'))
  assert.equal(byName.length, 4)
})

test('list serie get returns null for a null list row', () => {
  const child = new I32Serie([10, 20, 30])
  const col = ListSerie.fromParts(
    child.toField('item'),
    child.serializeBytes(),
    [0, 2, 2, 3],
    [true, false, true],
  ) // row 0 = [10,20], row 1 = null, row 2 = [30]
  assert.equal(col.get(1), null)
  const row0 = I32Serie.deserializeBytes(col.get(0))
  assert.equal(row0.length, 2)
})

// ---- MapSerie deep get/set --------------------------------------------------------------

test('map serie getAt / setAt address the key (child 0) and value (child 1) columns', () => {
  const col = counts() // keys ['a','b','c'], values [1,2,3]
  assert.equal(col.getAt([0, 0]), 'a') // keys cell 0
  assert.equal(col.getAt([1, 2]), '3') // values cell 2 (i64 -> string)
  assert.equal(col.getPath('[1][0]'), '1') // values cell 0 via an index-terminal path
  col.setAt([1, 0], '99')
  assert.equal(col.getAt([1, 0]), '99')
  assert.throws(() => col.getAt([0, 9]), /out of bounds/)
})

test('map serie get / childNamed expose the row entries and the flattened children', () => {
  const col = counts()
  assert.equal(col.numChildren(), 2)
  const row0 = StructSerie.deserializeBytes(col.get(0)) // {a->1, b->2} as a [keys, values] struct
  assert.equal(row0.length, 2)
  assert.equal(row0.numColumns, 2)
  const keys = Utf8Serie.deserializeBytes(col.childNamed('key'))
  assert.equal(keys.get(0), 'a')
  const values = I64Serie.deserializeBytes(col.getPath('value')) // name-terminal -> value column
  assert.equal(values.get(2), '3')
})

// ---- regression: confirmed Phase 5b defects on the list/map nested columns ---------------

// FIX 1: a deep setAt into the i32 item leaf validates like column() — an out-of-range or
// fractional value throws, never a silent ToInt32 wrap / truncation.
test('list serie setAt rejects an out-of-range / fractional value into the i32 item leaf', () => {
  const col = scores() // flattened item child is i32
  assert.throws(() => col.setAt([0, 0], 5000000000), /out of range for i32/)
  assert.throws(() => col.setAt([0, 0], 1.5), /whole number/)
  assert.equal(col.getAt([0, 0]), 10) // unchanged
})

// FIX 5: a negative / fractional coordinate is a guided error on list and map columns too.
test('list serie coords reject a negative or fractional coordinate', () => {
  const col = scores()
  assert.throws(() => col.getAt([-1, 0]), /non-negative integers/)
  assert.throws(() => col.getAt([0, 0.5]), /non-negative integers/)
})

test('map serie coords reject a negative or fractional coordinate', () => {
  const col = counts()
  assert.throws(() => col.setAt([0, -1], 'x'), /non-negative integers/)
  assert.throws(() => col.getCell([0.5, 0]), /non-negative integers/)
})

// FIX 6: slice() returns a fresh sub-range on list and map columns (the Node mirror of s[a:b]).
test('list serie slice returns a sub-range', () => {
  const col = scores() // 2 rows: [10,20,30] / [40]
  const first = ListSerie.deserializeBytes(col.slice(0, 1))
  assert.equal(first.length, 1)
  const row0 = I32Serie.deserializeBytes(first.get(0))
  assert.deepEqual([row0.get(0), row0.get(1), row0.get(2)], [10, 20, 30])
})

test('map serie slice returns a sub-range', () => {
  const col = counts() // 2 rows
  const first = MapSerie.deserializeBytes(col.slice(0, 1))
  assert.equal(first.length, 1)
  const tail = MapSerie.deserializeBytes(col.slice(1, 99)) // clamped, never throws
  assert.equal(tail.length, 1)
})

// A decimal item LEAF has no native cross-language scalar form, so a deep get is a guided error.
test('list serie deep get of a decimal item leaf is a guided error', () => {
  const dec = new D64Serie(10, 2, ['1.00', '2.00'])
  const col = ListSerie.fromParts(DataType.d64().field('item', true), dec.serializeBytes(), [0, 1, 2])
  assert.throws(() => col.getAt([0, 0]), /not supported through deep indexing|getColumn/)
})
