'use strict'

// The generic record entry points: anything in, one native reader out.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, IOBase } = require('..')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-entrypoints-'))
}

function trades() {
  return new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
  })
}

function rowsOf(handle) {
  const table = handle.readArrow().toTable()
  return table.numRows
}

test('every Arrow JS shape a caller holds writes through one entry point', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const table = trades()

  const sources = [
    ['table', table],
    ['batch', table.batches[0]],
    ['reader', arrow.RecordBatchReader.from(arrow.tableToIPC(table))],
    ['native', BatchReader.from(table)],
    ['ipc', arrow.tableToIPC(table)],
  ]
  for (const [name, source] of sources) {
    const handle = new IOBase(path.join(root, `${name}.arrows`))
    handle.writeArrow(source)

    assert.equal(rowsOf(handle), 2, name)
  }
})

test('a sequence of sources is written end to end', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const table = trades()

  const handle = new IOBase(path.join(root, 'sequence.arrows'))
  handle.writeArrow([table, table.batches[0]])

  assert.equal(rowsOf(handle), 4)
})

test('a generator of tables is consumed as it is pulled', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  let produced = 0

  function* pages() {
    for (let page = 0; page < 3; page += 1) {
      produced += 1
      yield trades()
    }
  }

  const handle = new IOBase(path.join(root, 'generated.arrows'))
  handle.writeArrow(pages())

  assert.equal(produced, 3)
  assert.equal(rowsOf(handle), 6)
})

test('plain records and named columns both name rows', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const records = new IOBase(path.join(root, 'records.arrows'))
  records.writeArrow([
    { id: 1, venue: 'XNAS' },
    { id: 2, venue: null },
  ])
  assert.equal(rowsOf(records), 2)

  // One object is one row when its values are scalars, and a set of columns
  // when they are sequences.
  const single = new IOBase(path.join(root, 'single.arrows'))
  single.writeArrow({ id: 7, venue: 'XLON' })
  assert.equal(rowsOf(single), 1)

  const columns = new IOBase(path.join(root, 'columns.arrows'))
  columns.writeArrow({ id: Int32Array.from([1, 2, 3]), venue: ['a', 'b', 'c'] })
  assert.equal(rowsOf(columns), 3)
})

test('a bare vector fills the one column a declared schema names', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const handle = new IOBase(path.join(root, 'vector.arrows'))
  const options = handle.recordOptions()
  options.schema = Field.from('row: struct<id: int32> not null')
  handle.writeArrow(arrow.vectorFromArray(Int32Array.from([1, 2, 3])), options)

  const table = handle.readArrow().toTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(
    table.schema.fields.map((field) => field.name),
    ['id'],
  )

  // Nothing else says which column a bare vector is, so it is refused rather
  // than named by guesswork.
  const unnamed = new IOBase(path.join(root, 'unnamed.arrows'))
  assert.throws(
    () => unnamed.writeArrow(arrow.vectorFromArray(Int32Array.from([1]))),
    /bare Vector names no column/,
  )
})

test('an async source makes the call async and nothing else does', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const table = trades()

  async function* pages() {
    yield table
    yield table.batches[0]
  }

  const asynchronous = new IOBase(path.join(root, 'async.arrows'))
  const pending = asynchronous.writeArrow(pages())
  assert.ok(pending instanceof Promise)
  await pending
  assert.equal(rowsOf(asynchronous), 4)

  // An Arrow JS reader implements both iteration protocols, and the
  // synchronous one is the one that runs.
  const synchronous = new IOBase(path.join(root, 'sync.arrows'))
  assert.equal(
    synchronous.writeArrow(arrow.RecordBatchReader.from(arrow.tableToIPC(table))),
    undefined,
  )
  assert.equal(rowsOf(synchronous), 2)
})

test('appendArrow adds to what is already there', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const handle = new IOBase(path.join(root, 'appended.arrows'))
  handle.writeArrow(trades())
  handle.appendArrow([{ id: 3n, venue: 'XLON' }])

  assert.equal(rowsOf(handle), 3)
})

test('readArrow is the same call the reader pair makes', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const handle = new IOBase(path.join(root, 'same.arrows'))
  handle.writeArrow(trades())

  assert.equal(
    handle.readArrow().toTable().numRows,
    handle.readArrowBatchReader().toTable().numRows,
  )
})

test('a value that names no rows is refused by name', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'refused.arrows'))

  assert.throws(() => handle.writeArrow(42), /rows must be a BatchReader/)
  assert.throws(() => handle.writeArrow([]), /empty sequence names no schema/)
  assert.throws(() => handle.writeArrow(null), /rows must be given/)
})

test('a foreign copy of apache-arrow is recognized by name and shape', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const table = trades()

  // A table from another copy of the package fails `instanceof` against this
  // one, so the constructor name plus the shape that name promises is what
  // recognizes it.
  const foreign = Object.create(Object.getPrototypeOf(table), {
    constructor: { value: { name: 'Table' } },
  })
  Object.defineProperties(foreign, {
    batches: { value: table.batches, enumerable: true },
    schema: { value: table.schema, enumerable: true },
  })

  const handle = new IOBase(path.join(root, 'foreign.arrows'))
  handle.writeArrow(foreign)

  assert.equal(rowsOf(handle), 2)
})
