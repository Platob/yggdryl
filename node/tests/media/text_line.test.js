'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { FieldPath, IOBase, TextLine, TextOptions } = require('yggdryl')

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
  assert.equal(lines[0].body, '8=FIX|55=AAPL|35=D')
  assert.equal(lines[0].decodedByteSize, 0)
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
  assert.equal(lines[0].getEntryByPath('55').value, 'AAPL')
  assert.equal(lines[1].getEntryByPath('55').value, 'MSFT')
})

test('a miss is null and the raising form says which path', () => {
  const [line] = [...source().readTextLines(lifted())]
  assert.equal(line.getEntryByPath('nosuch'), null)
  assert.throws(() => line.entryByPath('nosuch'), /nosuch/)
})

test('a resolved path and its text reach the same entry', () => {
  const [line] = [...source().readTextLines(lifted())]
  assert.equal(line.getEntryByPath(new FieldPath('"55"')).value, 'AAPL')
})

test('the setter creates what is not there and the remover takes it', () => {
  const [line] = [...source().readTextLines(lifted())]
  line.setEntryByPath('order.price', '12')
  assert.equal(line.getEntryByPath('order.price').value, '12')
  line.setEntryByPath('order.price', Buffer.from('13'))
  assert.equal(line.getEntryByPath('order.price').value, '13')
  assert.notEqual(line.removeEntryByPath('order'), null)
  assert.equal(line.getEntryByPath('order.price'), null)
})

test('entries measure, index and list', () => {
  const [line] = [...source().readTextLines(lifted())]
  const entries = line.entries
  assert.ok(entries.length >= 2)
  assert.equal(entries.at(0).key, '8')
  assert.equal(entries.toArray().length, entries.length)
  assert.notEqual(entries.at(-1), null)
})

test('an entry is text, and its bytes are beside it', () => {
  const [line] = [...source().readTextLines(lifted())]
  const entry = line.getEntryByPath('55')
  assert.equal(typeof entry.key, 'string')
  assert.equal(typeof entry.value, 'string')
  assert.equal(entry.value, 'AAPL')
  assert.ok(Buffer.isBuffer(entry.keyBytes))
  assert.ok(Buffer.isBuffer(entry.valueBytes))
  assert.deepEqual(entry.keyBytes, Buffer.from('55'))
  assert.deepEqual(entry.valueBytes, Buffer.from('AAPL'))
})

test('a string body round-trips as text', () => {
  const line = new TextLine(3, '58=caf\u00e9|10=0|', ['FIX.4.4', null])
  assert.equal(Number(line.index), 3)
  assert.equal(line.body, '58=caf\u00e9|10=0|')
  assert.equal(line.decodedByteSize, 0)
  assert.deepEqual(line.captures, ['FIX.4.4', null])
})

test('a buffer body with one Latin-1 byte is decoded where the line is made', () => {
  // `caf` then the lone 0xE9 Windows-1252 gives `\u00e9`, among UTF-8.
  const wire = Buffer.concat([Buffer.from('58=caf'), Buffer.from([0xe9]), Buffer.from('|10=0|')])
  const line = new TextLine(0, wire)
  assert.equal(line.body, '58=caf\u00e9|10=0|')
  assert.equal(line.decodedByteSize, 1)
  // A buffer that was text costs nothing and counts nothing.
  assert.equal(new TextLine(0, Buffer.from('58=caf\u00e9|10=0|')).decodedByteSize, 0)
})

test('the five bytes the classic table leaves undefined read as their own code points', () => {
  for (const byte of [0x81, 0x8d, 0x8f, 0x90, 0x9d]) {
    const line = new TextLine(0, Buffer.from([0x61, byte, 0x62]))
    assert.equal(line.body, `a${String.fromCharCode(byte)}b`)
    assert.equal(line.decodedByteSize, 1)
  }
  // And the classic row reads as the table has it.
  const quoted = new TextLine(0, Buffer.from([0x80, 0x20, 0x93, 0x71, 0x94]))
  assert.equal(quoted.body, '\u20ac \u201cq\u201d')
  assert.equal(quoted.decodedByteSize, 3)
})

test('a valid character is kept beside a lone byte on the same line', () => {
  // The decode is per byte: the UTF-8 `\u00e9` stays, the lone 0xE9 becomes one.
  const wire = Buffer.concat([
    Buffer.from('58=caf'),
    Buffer.from([0xe9]),
    Buffer.from(' caf\u00e9|10=0|'),
  ])
  const line = new TextLine(0, wire)
  assert.equal(line.body, '58=caf\u00e9 caf\u00e9|10=0|')
  assert.equal(line.decodedByteSize, 1)
})

test('a byte that was not text reaches a lifted entry decoded, with its bytes beside it', () => {
  const wire = Buffer.concat([Buffer.from('8=FIX|55=caf'), Buffer.from([0xe9]), Buffer.from('|10=0\n')])
  const [line] = [...IOBase.fromBytes(wire).readTextLines(lifted())]
  assert.equal(line.decodedByteSize, 1)
  const entry = line.getEntryByPath('55')
  assert.equal(entry.value, 'caf\u00e9')
  assert.deepEqual(entry.valueBytes, Buffer.from('caf\u00e9'))
})

test('a .log read through the reader carries body as a string column', () => {
  const table = source('first\nsecond\n').readArrowReader(new TextOptions()).intoTable()
  const body = table.schema.fields.find((field) => field.name === 'body')
  assert.equal(body.type.toString(), 'Utf8')
  assert.equal(body.nullable, false)
  assert.deepEqual([...table.getChild('body')], ['first', 'second'])
  const rows = [...source('first\nsecond\n').readRecords(new TextOptions())]
  assert.deepEqual(
    rows.map((row) => typeof row.body),
    ['string', 'string'],
  )
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

test('a trailing as names what the path reached', () => {
  const path = new FieldPath('order.line[0].price as price')
  assert.equal(path.alias, 'price')
  assert.equal(path.columnName, 'price')
  assert.equal(path.length, 4)
  assert.equal(path.toString(), 'order.line[0].price as price')
})

test('without an alias the last segment names it', () => {
  const path = new FieldPath('order.line.price')
  assert.equal(path.alias, null)
  assert.equal(path.columnName, 'price')
})

test('a segment may still be named as', () => {
  assert.equal(new FieldPath('order.as').alias, null)
  assert.equal(new FieldPath('order.as').columnName, 'as')
  assert.equal(new FieldPath('assets').alias, null)
})

test('a malformed alias is refused', () => {
  for (const text of ['price as', 'price as one two', 'as name']) {
    assert.throws(() => new FieldPath(text), /field path/)
  }
})

test('a lifted path names its column with its alias', () => {
  const options = new TextOptions()
  options.liftNames = ['"55" as symbol']
  const field = options.sourceField()
  const names = []
  for (let at = 0; at < field.fieldLen; at += 1) {
    names.push(field.fieldAt(at).name)
  }
  assert.ok(names.includes('symbol'))
  assert.ok(!names.includes('55'))
})
