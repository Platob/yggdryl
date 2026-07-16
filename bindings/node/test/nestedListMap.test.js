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
