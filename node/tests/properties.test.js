'use strict'

// `node/properties.js`: option properties given by name, and the warning a
// name no property owns raises.
//
// Every options-taking door takes the options' properties as a plain object
// beside the options, each set on a copy by its own setter. A name no setter
// owns is skipped with an `UnknownPropertyWarning` process warning naming it
// and the closest property there is, so a typo is heard without failing.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { closest, settableProperties } = require('../properties.js')
const { IOBase, RecordOptions, TextLine, TextOptions, http, iceberg } = require('yggdryl')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-properties-'))
}

// The `UnknownPropertyWarning` messages `operation` raised, once the process
// has dispatched them.
async function warnings(operation) {
  const caught = []
  const listener = (warning) => {
    if (warning.name === 'UnknownPropertyWarning') caught.push(warning.message)
  }
  process.on('warning', listener)
  try {
    operation()
  } finally {
    // Dispatched on a later tick: heard here even when `operation` threw.
    await new Promise((resolve) => setImmediate(resolve))
    process.off('warning', listener)
  }
  return caught
}

function csv(root) {
  const handle = new IOBase(path.join(root, 'trades.csv'))
  handle.writeText('symbol;price\nAAPL;187\n')
  return handle
}

test('a suggestion is the closest name within a third of its length', () => {
  assert.equal(closest('seperator', ['separator', 'quote']), 'separator')
  assert.equal(closest('warehouse', ['separator', 'quote']), undefined)
  // What a class sets is read off its own prototype: getters and methods
  // are no properties to set.
  const names = settableProperties(RecordOptions)
  assert.ok(names.has('separator'))
  assert.ok(!names.has('mimeType'))
  assert.ok(!names.has('clone'))
})

test('an unknown record property warns naming the closest and is skipped', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = csv(root)

  let table
  const caught = await warnings(() => {
    table = handle.readArrowReader({ seperator: ';', header: true }).intoTable()
  })
  assert.deepEqual(caught, [
    "RecordOptions has no settable property 'seperator'; it is ignored; did you mean 'separator'?",
  ])
  assert.equal(table.numCols, 1, 'the default separator read one column')
  // Never assigned onto the copy either, where it would have done nothing.
  assert.deepEqual(await warnings(() => handle.readArrowField({ separator: ';' })), [])
})

test('a property of another class, a getter or an absent value', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = csv(root)
  // `rowheader` is a text setting: on CSV options it names nothing to set.
  assert.equal((await warnings(() => handle.readArrowField({ rowheader: '^x' }))).length, 1)
  assert.equal((await warnings(() => handle.readArrowField({ mimeType: 'text/plain' }))).length, 1)
  // An undefined value is skipped, and its name still checked.
  assert.deepEqual(await warnings(() => handle.readArrowField({ separator: undefined })), [])
  assert.equal((await warnings(() => handle.readArrowField({ seprator: undefined }))).length, 1)
  // A known name the encoding cannot honour is the setter's own refusal.
  assert.throws(() => handle.readArrowField({ sheet: 'Sheet1' }), /\$\.sheet/)
})

test('an options value is built with its properties by name', async () => {
  const options = new RecordOptions('text/csv', { separator: ';', header: false })
  assert.equal(options.separator, ';')
  assert.equal(options.header, false)
  const text = new TextOptions({ rowheader: '^(?<level>[A-Z]+) ', autotype: true })
  assert.equal(text.rowheader, '^(?<level>[A-Z]+) ')
  assert.equal(text.autotype, true)
  assert.ok(text instanceof TextOptions)
  const caught = await warnings(() => new TextOptions({ autotyp: true }))
  assert.match(caught[0], /'autotyp'.*did you mean 'autotype'/)
})

test('a line and a text handle take the text properties the same way', (t) => {
  const header = '^(?<level>[A-Z]+) '
  const line = new TextLine(0n, 'INFO started', null, null, { rowheader: header })
  assert.deepEqual(line.captures, ['INFO'])

  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'app.log'))
  handle.writeText('INFO started\n')
  handle.intoText(new TextOptions({ rowheader: header }))
  // A bag alone keeps what the handle retained and adds to it.
  handle.intoText({ startRownum: 1n })
  assert.equal(handle.recordOptions().rowheader, header)
  assert.equal(handle.recordOptions().startRownum, 1n)
})

test('Iceberg options take their fields by name, per value and per call', async (t) => {
  const caught = await warnings(() => new iceberg.IcebergOptions({ commitRetres: 2 }))
  assert.match(caught[0], /IcebergOptions has no settable property 'commitRetres'.*'commitRetries'/)

  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const catalog = new iceberg.Catalog(root)
  const rows = [{ id: 1n }, { id: 2n }]
  const table = catalog.append('nyc.trades', rows, { dataMimeType: 'avro' })
  const formats = table.dataFiles().map((file) => file.mimeType.toString())
  assert.deepEqual(formats, ['application/avro'])
  assert.equal((await warnings(() => table.append(rows, { dataFromat: 'avro' }))).length, 1)
})

test('a session takes HttpOptions properties beside its named keys', async () => {
  const session = new http.Session('http://127.0.0.1:9', { maxAttempts: 2, cookies: false })
  assert.ok(session)
  const caught = await warnings(() => new http.Session(undefined, { timout: 5 }))
  assert.match(caught[0], /HttpOptions has no settable property 'timout'.*did you mean 'timeout'/)
})

test('a repeated typo is heard once per process', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = csv(root)
  const caught = await warnings(() => {
    for (let index = 0; index < 3; index += 1) handle.readArrowField({ quoet: '"' })
  })
  assert.equal(caught.length, 1)
})
