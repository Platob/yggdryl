'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { FieldPath, IOBase, TextOptions } = require('yggdryl')

const CAPTURE = '8=FIX|55=AAPL|35=D\n35=D|55=MSFT\n'

function source(payload = CAPTURE) {
  return IOBase.fromBytes(Buffer.from(payload))
}

function lifted() {
  const options = new TextOptions()
  options.liftNames = ['55']
  return options
}

test('a path parses every segment kind and renders back', () => {
  const path = new FieldPath('order.line[0]')
  assert.equal(path.length, 3)
  assert.deepEqual(path.segments, ['order', 'line', 0])
  assert.equal(path.toString(), 'order.line[0]')
  assert.equal(path.parent().toString(), 'order.line')
})

test('a name carrying a dot is one segment and a bare one is two', () => {
  assert.equal(new FieldPath('"a.b"').length, 1)
  assert.equal(new FieldPath('"a.b"').name, 'a.b')
  assert.equal(new FieldPath('a.b').length, 2)
})

test('the root selects the value it is applied to', () => {
  assert.equal(FieldPath.root().isRoot, true)
  assert.equal(new FieldPath().length, 0)
})

test('a path is built from parts without rendering or reparsing', () => {
  const path = new FieldPath('order').join('line').join(2)
  assert.equal(path.toString(), 'order.line[2]')
  assert.equal(path.equals(new FieldPath('order.line[2]')), true)
})

test('a malformed path is refused where it stopped', () => {
  assert.throws(() => new FieldPath('order.'), /field path/)
})

test('every line becomes one typed row', () => {
  const lines = [...source().readTextLines(new TextOptions())]
  assert.equal(lines.length, 2)
  assert.equal(Number(lines[0].index), 0)
  assert.equal(lines[0].body.toString(), '8=FIX|55=AAPL|35=D')
  assert.notEqual(lines[0].url, null)
})

test('a read wanting no entry builds no tree', () => {
  const lines = [...source().readTextLines(new TextOptions())]
  assert.equal(lines[0].entries, null)
  assert.equal(lines[0].bodytype, null)
})

test('a lifted path builds the tree and is found by path', () => {
  const lines = [...source().readTextLines(lifted())]
  assert.notEqual(lines[0].entries, null)
  assert.equal(lines[0].getEntryByPath('55').value.toString(), 'AAPL')
  assert.equal(lines[1].getEntryByPath('55').value.toString(), 'MSFT')
})

test('a miss is null and the raising form says which path', () => {
  const [line] = [...source().readTextLines(lifted())]
  assert.equal(line.getEntryByPath('nosuch'), null)
  assert.throws(() => line.entryByPath('nosuch'), /nosuch/)
})

test('a resolved path and its text reach the same entry', () => {
  const [line] = [...source().readTextLines(lifted())]
  assert.equal(line.getEntryByPath(new FieldPath('"55"')).value.toString(), 'AAPL')
})

test('the setter creates what is not there and the remover takes it', () => {
  const [line] = [...source().readTextLines(lifted())]
  line.setEntryByPath('order.price', Buffer.from('12'))
  assert.equal(line.getEntryByPath('order.price').value.toString(), '12')
  assert.notEqual(line.removeEntryByPath('order'), null)
  assert.equal(line.getEntryByPath('order.price'), null)
})

test('entries measure, index and list', () => {
  const [line] = [...source().readTextLines(lifted())]
  const entries = line.entries
  assert.ok(entries.length >= 2)
  assert.equal(entries.at(0).key.toString(), '8')
  assert.equal(entries.toArray().length, entries.length)
  assert.notEqual(entries.at(-1), null)
})

test('lift names and renames round-trip through the options', () => {
  const options = lifted()
  assert.equal(options.liftNames.length, 1)
  options.renameColumns = { body: 'payload' }
  assert.deepEqual(options.renameColumns, { body: 'payload' })
  const field = options.sourceField()
  const names = []
  for (let at = 0; at < field.fieldLen; at += 1) {
    names.push(field.fieldAt(at).name)
  }
  assert.ok(names.includes('payload'))
  assert.ok(!names.includes('body'))
  assert.ok(names.includes('55'))
  options.liftNames = null
  assert.equal(options.liftNames, null)
})

test('a rename naming no column is refused when the schema is asked for', () => {
  const options = new TextOptions()
  options.renameColumns = { nosuch: 'x' }
  assert.throws(() => options.sourceField(), /nosuch/)
})
