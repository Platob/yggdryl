'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Serie, ByteSerie, StructSerie } = yggdryl.typed
const { DataTypeId } = yggdryl.datatype_id

// The real `apache-arrow` (JS) interop lib. It is a devDependency, but if it (or the network
// that installs it) is unavailable the pure-Rust round-trip below still exercises toIpc/fromIpc,
// so the interop assertions are made conditional rather than hard-failing.
let arrow = null
try {
  arrow = require('apache-arrow')
} catch {
  arrow = null
}
const hasArrow = arrow !== null

// A small table: an int `id` column + a utf8 `name` column.
function buildSerie() {
  return StructSerie.fromColumns(
    [
      Serie.fromValues([1n, 2n, 3n], DataTypeId.I64()),
      ByteSerie.fromValues(['ada', 'bo', 'cy'], DataTypeId.Utf8()),
    ],
    ['id', 'name'],
  )
}

// -------------------------------------------------------------------------------------
// Pure-Rust round-trip — needs NO apache-arrow JS lib (always runs).
// -------------------------------------------------------------------------------------

test('StructSerie.toIpc → fromIpc round-trips through Arrow IPC (pure Rust)', () => {
  const serie = buildSerie()
  const buf = serie.toIpc()
  assert.ok(Buffer.isBuffer(buf), 'toIpc returns a Node Buffer')
  assert.ok(buf.length > 0)

  const back = StructSerie.fromIpc(buf)
  assert.equal(back.numColumns(), 2)
  assert.equal(back.len(), 3)
  assert.deepEqual(back.columnNames(), ['id', 'name'])
  assert.deepEqual(back.column(0).toList(), [1n, 2n, 3n])
  assert.deepEqual(back.columnByName('name').values(), ['ada', 'bo', 'cy'])
})

test('StructSerie.toIpc refuses a struct with null rows (guided error)', () => {
  const serie = StructSerie.fromColumns([Serie.fromValues([1n, 2n], DataTypeId.I64())], ['id'])
  serie.pushNull()
  assert.equal(serie.nullCount(), 1)
  assert.throws(() => serie.toIpc(), /row-level validity|null row/)
})

test('StructSerie.fromIpc rejects empty / invalid IPC bytes (guided error)', () => {
  assert.throws(() => StructSerie.fromIpc(Buffer.from([])), /Arrow IPC stream/)
  assert.throws(() => StructSerie.fromIpc(Buffer.from([1, 2, 3, 4, 5])), /Arrow IPC stream/)
})

// -------------------------------------------------------------------------------------
// REAL apache-arrow (JS) interop — skipped when the lib is not installed (offline).
// -------------------------------------------------------------------------------------

test('apache-arrow tableFromIPC reads a StructSerie.toIpc buffer', { skip: !hasArrow }, () => {
  const serie = buildSerie()
  const buf = serie.toIpc()

  const table = arrow.tableFromIPC(buf)
  assert.equal(table.numCols, 2)
  assert.equal(table.numRows, 3)
  assert.deepEqual(
    table.schema.fields.map((f) => f.name),
    ['id', 'name'],
  )
  // The name column values match.
  assert.deepEqual(Array.from(table.getChild('name').toArray()), ['ada', 'bo', 'cy'])
  // The id column round-trips as bigints.
  assert.equal(table.getChild('id').get(0), 1n)
})

test('StructSerie.fromIpc reads an apache-arrow tableToIPC buffer', { skip: !hasArrow }, () => {
  const serie = buildSerie()
  // Round arrow → arrow first (produces an apache-arrow-authored IPC stream), then into us.
  const table = arrow.tableFromIPC(serie.toIpc())
  const ipc = arrow.tableToIPC(table, 'stream') // a Uint8Array

  const back = StructSerie.fromIpc(Buffer.from(ipc))
  assert.equal(back.numColumns(), 2)
  assert.equal(back.len(), 3)
  assert.deepEqual(back.columnNames(), ['id', 'name'])
  assert.deepEqual(back.column(0).toList(), [1n, 2n, 3n])
  assert.deepEqual(back.columnByName('name').values(), ['ada', 'bo', 'cy'])
})
