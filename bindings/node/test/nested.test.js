'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataType, Field, StructField, StructSerie, I32Serie, U8Serie, Utf8Serie } = yggdryl.types

// A flat struct column: { id: i32, name: utf8 (with a null) }.
function table() {
  const ids = new I32Serie([1, 2, 3])
  const names = new Utf8Serie(['ann', null, 'cara'])
  const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
  return StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
}

// ---- StructField -----------------------------------------------------------------------

test('the types namespace exposes the nested classes', () => {
  for (const cls of [StructField, StructSerie]) {
    assert.equal(typeof cls, 'function')
  }
})

test('struct field shape', () => {
  const schema = new StructField(
    'person',
    [new Field('id', DataType.i64(), false), new Field('name', DataType.utf8(), true)],
    true,
  )
  assert.equal(schema.name, 'person')
  assert.equal(schema.typeName, 'struct')
  assert.ok(schema.nullable)
  assert.equal(schema.numFields, 2)
  assert.equal(schema.indexOf('name'), 1)
  assert.equal(schema.field(1).name, 'name')
  assert.equal(schema.fieldNamed('id').name, 'id')
  assert.equal(schema.fieldNamed('missing'), null)
  assert.deepEqual(schema.fields().map((f) => f.name), ['id', 'name'])
})

test('struct field nests', () => {
  const inner = new StructField('point', [new Field('x', DataType.f64(), false)], false)
  const outer = new StructField('shape', [inner], true)
  assert.equal(outer.numFields, 1)
  const recovered = outer.field(0)
  assert.ok(recovered instanceof StructField)
  assert.equal(recovered.name, 'point')
})

test('struct field builders are immutable', () => {
  const base = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  const renamed = base.withName('t').withNullable(false)
  assert.ok(base.name === 's' && base.nullable)
  assert.ok(renamed.name === 't' && !renamed.nullable)
  const grown = base.withField(new Field('b', DataType.utf8(), true))
  assert.ok(base.numFields === 1 && grown.numFields === 2)
})

test('struct field value semantics', () => {
  const a = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  const b = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.ok(StructField.deserializeBytes(a.serializeBytes()).equals(a))
  assert.ok(a.copy().equals(a))
})

// ---- StructSerie -----------------------------------------------------------------------

test('struct serie build and navigate', () => {
  const t = table()
  assert.equal(t.length, 3)
  assert.equal(t.numColumns, 2)
  assert.equal(t.field(1).name, 'name')
  // A child crosses as bytes; reconstruct it with the matching Serie class.
  const idsBack = I32Serie.deserializeBytes(t.columnBytes(0))
  assert.equal(idsBack.get(0), 1)
  const namesBack = Utf8Serie.deserializeBytes(t.columnBytesNamed('name'))
  assert.equal(namesBack.get(0), 'ann')
  assert.equal(namesBack.get(1), null)
  assert.equal(t.columnBytesNamed('missing'), null)
})

test('struct serie schema/column count mismatch throws', () => {
  const schema = new StructField('s', [new Field('a', DataType.i32(), true)], false)
  assert.throws(() => StructSerie.fromColumns(schema, []))
})

test('struct serie serialize round trip', () => {
  const t = table()
  assert.ok(StructSerie.deserializeBytes(t.serializeBytes()).equals(t))
})

test('struct serie nests', () => {
  const x = new I32Serie([1, 2])
  const y = new U8Serie([3, 4])
  const innerSchema = new StructField('p', [x.toField('x'), y.toField('y')], false)
  const inner = StructSerie.fromColumns(innerSchema, [x.serializeBytes(), y.serializeBytes()])

  const tag = new Utf8Serie(['a', 'b'])
  const outerSchema = new StructField('o', [inner.toField('point'), tag.toField('tag')], false)
  const outer = StructSerie.fromColumns(outerSchema, [inner.serializeBytes(), tag.serializeBytes()])

  assert.equal(outer.numColumns, 2)
  assert.ok(outer.field(0) instanceof StructField)
  assert.ok(StructSerie.deserializeBytes(outer.serializeBytes()).equals(outer))
})

test('struct serie value semantics and toString', () => {
  const a = table()
  const b = table()
  assert.ok(a.equals(b))
  assert.ok(a.copy().equals(a))
  assert.ok(a.toString().startsWith('StructSerie(len=3'))
  assert.equal(a.hasNulls, false) // no null struct rows (the name *column* has a null, not a row)
})

test('to_field nullability reflects struct rows, not child nulls', () => {
  const schema = table().toField('person')
  assert.ok(schema instanceof StructField)
  assert.equal(schema.name, 'person')
  assert.equal(schema.nullable, false)
})
