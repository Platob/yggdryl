'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Serie, ByteSerie, Field, StructSerie, StructField, ListSerie, MapSerie } = yggdryl.typed
const { DataTypeId } = yggdryl.datatype_id

// -------------------------------------------------------------------------------------
// StructSerie — the "table": heterogeneous, equal-length child columns
// -------------------------------------------------------------------------------------

test('StructSerie.fromColumns builds a table over a Serie and a ByteSerie', () => {
  const table = StructSerie.fromColumns(
    [
      Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()),
      ByteSerie.fromValues(['a', 'b', 'c'], DataTypeId.Utf8()),
    ],
    ['id', 'name'],
  )

  assert.equal(table.numColumns(), 2)
  assert.equal(table.len(), 3)
  assert.deepEqual(table.columnNames(), ['id', 'name'])

  // A numeric child comes back as a Serie (copy) — read it like any column.
  const id = table.column(0)
  assert.deepEqual(id.toList(), [1n, 2n, 3n])
  assert.ok(id.dtype().equals(DataTypeId.I64()))

  // The byte child comes back as a ByteSerie — its values read as strings.
  const name = table.columnByName('name')
  assert.deepEqual(name.values(), ['a', 'b', 'c'])

  // An absent column is null.
  assert.equal(table.columnByName('missing'), null)
  assert.equal(table.column(9), null)
})

test('StructSerie.row marshals each child element to its JS shape', () => {
  const table = StructSerie.fromColumns(
    [
      Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()),
      ByteSerie.fromValues(['a', 'b', 'c'], DataTypeId.Utf8()),
    ],
    ['id', 'name'],
  )

  assert.deepEqual(table.row(0), [1n, 'a'])
  assert.deepEqual(table.row(1), [2n, 'b'])
  assert.equal(table.row(3), null) // out of range
})

test('StructSerie.setColumn replaces a column and appends a new one', () => {
  const table = StructSerie.fromColumns(
    [Serie.fromValues([1n, 2n, 3n], DataTypeId.I64())],
    ['id'],
  )

  // Replace an existing column (renamed to the given name).
  table.setColumn('id', Serie.fromValues([10n, 20n, 30n], DataTypeId.I64()))
  assert.deepEqual(table.column(0).toList(), [10n, 20n, 30n])

  // Append a new column of the matching length.
  table.setColumn('score', Serie.fromValues([1.5, 2.5, 3.5], DataTypeId.F64()))
  assert.equal(table.numColumns(), 2)
  assert.deepEqual(table.columnNames(), ['id', 'score'])
  assert.deepEqual(table.columnByName('score').toList(), [1.5, 2.5, 3.5])

  // A length mismatch on replace throws.
  assert.throws(
    () => table.setColumn('id', Serie.fromValues([1n, 2n], DataTypeId.I64())),
    /length mismatch|rows/,
  )
})

test('StructSerie length-mismatch and names-mismatch throw a guided Error', () => {
  assert.throws(
    () =>
      StructSerie.fromColumns([
        Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()),
        Serie.fromValues([1n, 2n], DataTypeId.I64()),
      ]),
    /every child must share|rows/,
  )

  assert.throws(
    () =>
      StructSerie.fromColumns(
        [Serie.fromValues([1n, 2n, 3n], DataTypeId.I64())],
        ['id', 'extra'],
      ),
    /names length/,
  )
})

test('StructSerie.pushNull grows a null row and field() reports the schema', () => {
  const table = StructSerie.fromColumns(
    [
      Serie.fromValues([1n, 2n], DataTypeId.I64()),
      ByteSerie.fromValues(['a', 'b'], DataTypeId.Utf8()),
    ],
    ['id', 'name'],
  )
  table.pushNull()
  assert.equal(table.len(), 3)
  assert.equal(table.nullCount(), 1)
  assert.equal(table.isValid(2), false)

  const field = table.field()
  assert.deepEqual(field.names(), ['id', 'name'])
  assert.equal(field.numFields(), 2)
  assert.equal(field.nullable(), true) // a validity buffer now exists
})

// -------------------------------------------------------------------------------------
// StructField — the struct schema (name + ordered child fields)
// -------------------------------------------------------------------------------------

test('StructField names / field lookup / equals', () => {
  const fields = [
    new Field('city', DataTypeId.Utf8(), false),
    new Field('zip', DataTypeId.I32(), false),
  ]
  const address = new StructField('address', fields)

  assert.equal(address.name(), 'address')
  assert.equal(address.numFields(), 2)
  assert.deepEqual(address.names(), ['city', 'zip'])
  assert.ok(address.field(1).dtype().equals(DataTypeId.I32()))
  assert.equal(address.fieldByName('city').name(), 'city')
  assert.equal(address.field(9), null)

  const same = new StructField('address', [
    new Field('city', DataTypeId.Utf8(), false),
    new Field('zip', DataTypeId.I32(), false),
  ])
  assert.ok(address.equals(same))

  const different = new StructField('address', [new Field('city', DataTypeId.Utf8(), false)])
  assert.equal(address.equals(different), false)
})

// -------------------------------------------------------------------------------------
// ListSerie — a variable-length list over a flattened child column
// -------------------------------------------------------------------------------------

test('ListSerie over an int child — push demarcates the sub-lists', () => {
  const child = Serie.fromValues([1n, 2n, 3n, 4n, 5n], DataTypeId.I64())
  const list = new ListSerie(child, 'nums')
  list.push(2) // [1, 2]
  list.push(0) // []
  list.push(3) // [3, 4, 5]

  assert.equal(list.len(), 3)
  assert.deepEqual(list.list(0), [1n, 2n])
  assert.deepEqual(list.list(1), [])
  assert.deepEqual(list.list(2), [3n, 4n, 5n])
  assert.equal(list.list(9), null) // out of range

  // The flattened child comes back as a Serie copy.
  assert.deepEqual(list.values().toList(), [1n, 2n, 3n, 4n, 5n])

  // A null list reads as null.
  list.pushNull()
  assert.equal(list.len(), 4)
  assert.equal(list.list(3), null)
  assert.equal(list.nullCount(), 1)

  assert.equal(list.field().dtype().name(), 'list')
})

// -------------------------------------------------------------------------------------
// MapSerie — a key->value map (utf8 -> int32)
// -------------------------------------------------------------------------------------

test('MapSerie utf8 -> int32 — get returns entries arrays', () => {
  const keys = ByteSerie.fromValues(['a', 'b', 'c'], DataTypeId.Utf8())
  const vals = Serie.fromValues([1, 2, 3], DataTypeId.I32())
  const map = new MapSerie(keys, vals, 'prices')
  map.push(2) // {"a": 1, "b": 2}
  map.push(1) // {"c": 3}

  assert.equal(map.len(), 2)
  assert.deepEqual(map.get(0), [
    ['a', 1],
    ['b', 2],
  ])
  assert.deepEqual(map.get(1), [['c', 3]])
  assert.equal(map.get(9), null)

  assert.deepEqual(map.keys().values(), ['a', 'b', 'c'])
  assert.deepEqual(map.values().toList(), [1, 2, 3])
  assert.equal(map.keysSorted(), false)
  assert.equal(map.field().dtype().name(), 'map')
})

test('MapSerie rejects a nullable key column with a guided Error', () => {
  const keys = ByteSerie.fromOptions(['a', null, 'c'], DataTypeId.Utf8())
  const vals = Serie.fromValues([1, 2, 3], DataTypeId.I32())
  assert.throws(() => new MapSerie(keys, vals, 'm'), /key column must be non-nullable|cannot be null/)
})
