'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Writable } = require('node:stream')
const { ReadableStream, WritableStream } = require('node:stream/web')
const test = require('node:test')
const { pathToFileURL } = require('node:url')

const { MimeType, Scalar, codec, fields, xml } = require('yggdryl')

test('XML is a byte-first single-document facade keyed by its root element', () => {
  const value = xml.loads('<trade><symbol>AAPL</symbol></trade>')
  assert.deepEqual(value, { trade: { symbol: 'AAPL' } })

  const encoded = xml.dumps(value)
  assert.ok(Buffer.isBuffer(encoded))
  assert.equal(encoded.toString('utf8'), '<trade><symbol>AAPL</symbol></trade>')
  assert.deepEqual(xml.loads(encoded), value)
  assert.ok(Object.isFrozen(xml))

  for (const name of [
    'loadsAll',
    'loadAll',
    'dumpAll',
    'loadAllStream',
    'dumpAllStream',
  ]) {
    assert.equal(xml[name], undefined)
  }
})

test("an attribute keys behind '@' and character data beside it keys '#text'", () => {
  const value = xml.loads('<trade id="7" ns:seq="2">filled</trade>')
  assert.deepEqual(value, {
    trade: { '@id': '7', '@ns:seq': '2', '#text': 'filled' },
  })
  // No XML name starts with either character, so neither collides with a
  // child element of the same name, and a prefix is kept verbatim.
  assert.equal(
    xml.dumps(value).toString('utf8'),
    '<trade id="7" ns:seq="2">filled</trade>',
  )
})

test('a repeated element is an array and a record is written in name order', () => {
  const value = xml.loads('<row><tag>a</tag><tag>b</tag><other/></row>')
  assert.deepEqual(value, { row: { tag: ['a', 'b'], other: null } })
  assert.equal(
    xml.dumps(value).toString('utf8'),
    '<row><other/><tag>a</tag><tag>b</tag></row>',
  )
})

test('an element with no character data is absence in both spellings', () => {
  for (const document of ['<row/>', '<row></row>']) {
    assert.deepEqual(xml.loads(document), { row: null }, document)
  }
  // The two spellings are one document, so an empty string reads back as null.
  const empty = xml.dumps({ row: '' })
  assert.equal(empty.toString('utf8'), '<row></row>')
  assert.deepEqual(xml.loads(empty), { row: null })
})

test('XML refuses what no document can spell, before a destination is opened', () => {
  assert.throws(
    () => xml.loads('<row>text<id>1</id></row>'),
    /character data or child elements, not both/,
  )
  assert.throws(
    () => xml.dumps({ row: { '#text': 'text', id: 1 } }),
    /not both/,
  )
  assert.throws(() => xml.loads('<a/><b/>'), /xml/i)
  assert.throws(() => xml.loads('<row><id>1</id>'), /xml/i)
  for (const name of ['', '1st', 'a b']) {
    assert.throws(() => xml.dumps({ [name]: null }), /expected an XML name/)
  }
  assert.throws(
    () => xml.dumps({ row: { tag: [['a']] } }),
    /repeats an element/,
  )

  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-xml-safe-'))
  const destination = path.join(directory, 'existing.xml')
  fs.writeFileSync(destination, '<keep/>')
  try {
    assert.throws(() => xml.dump({ '1st': null }, destination), /expected an XML name/)
    assert.equal(fs.readFileSync(destination, 'utf8'), '<keep/>')
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
})

test('the indent option lays children out and never a leaf character data', () => {
  const value = xml.loads('<row><a>x</a><b><c>y</c></b></row>')
  assert.equal(
    xml.dumps(value, { indent: 2 }).toString('utf8'),
    '<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>',
  )
  assert.match(xml.dumps(value, { indent: '\t' }).toString('utf8'), /\n\t<a>x<\/a>/)
  // The default is one flat line, and `null` asks for no layout at all.
  const flat = '<row><a>x</a><b><c>y</c></b></row>'
  assert.equal(xml.dumps(value).toString('utf8'), flat)
  assert.equal(xml.dumps(value, { indent: null }).toString('utf8'), flat)

  for (const indent of [null, 0, 2, 255, '\t']) {
    const encoded = xml.dumps(value, { indent })
    assert.deepEqual(xml.loads(encoded), value, `indent ${String(indent)}`)
    assert.deepEqual(
      codec.into(value, { format: 'xml', indent }),
      encoded,
      `generic redirect at indent ${String(indent)}`,
    )
  }
  // A leaf's own character data is never laid out: the layout would be value.
  assert.equal(xml.dumps({ row: 'x' }, { indent: 4 }).toString('utf8'), '<row>x</row>')
})

test('every leaf is character data until a field types it', () => {
  const document = '<row><n>1</n><f>1.5</f><b>true</b><day>2024-01-01</day></row>'
  assert.deepEqual(xml.loads(document), {
    row: { n: '1', f: '1.5', b: 'true', day: '2024-01-01' },
  })

  const field = fields.struct(
    'row',
    [
      fields.int64('n', { nullable: false }),
      fields.float64('f', { nullable: false }),
      fields.boolean('b', { nullable: false }),
      fields.date32('day', { nullable: false }),
    ],
    { nullable: false },
  )
  const typed = xml.loads(document, { field })
  assert.equal(typed.n, 1)
  assert.equal(typed.f, 1.5)
  assert.equal(typed.b, true)
  assert.ok(typed.day.equals(Scalar.date(19723)))
})

test('a field reads the shapes a document cannot spell', () => {
  const field = fields.struct(
    'row',
    [
      fields.list('tag', fields.utf8('item'), { nullable: false }),
      fields.list('none', fields.utf8('item'), { nullable: false }),
    ],
    { nullable: false },
  )
  // One occurrence is a one-item list; no occurrence at all is the empty one.
  assert.deepEqual(xml.loads('<row><tag>a</tag></row>', { field }), {
    tag: ['a'],
    none: [],
  })
  assert.deepEqual(xml.loads('<row><tag>a</tag><tag>b</tag></row>', { field }), {
    tag: ['a', 'b'],
    none: [],
  })
})

test('"xml" normalizes as a format name and content that opens a tag infers it', () => {
  assert.deepEqual(codec.from('<row><id>1</id></row>'), { row: { id: '1' } })
  assert.deepEqual(codec.from('<row><id>1</id></row>', { format: 'xml' }), {
    row: { id: '1' },
  })
  assert.equal(MimeType.XML.format, 'xml')
  assert.equal(MimeType.fromPath('trades.xml').format, 'xml')
  // JSON and YAML keep the content they can read.
  assert.deepEqual(codec.from('{"id":1}'), { id: 1 })
  assert.deepEqual(codec.from('id: 2\n'), { id: 2 })

  assert.deepEqual(
    xml.loads(codec.into({ row: { id: '3' } }, { format: 'xml' })),
    { row: { id: '3' } },
  )
  // The accepted vocabulary a refusal names carries xml beside the rest.
  assert.throws(
    () => codec.into({ row: null }, { format: 'xhtml' }),
    /expected json, jsonl\/ndjson, yaml\/yml, toml, or xml/,
  )
})

test('XML paths, descriptors, readers, and writers stay native and caller-owned', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-xml-path-'))
  const source = path.join(directory, 'source.xml')
  const destination = path.join(directory, 'destination.xml')
  fs.writeFileSync(source, '<row><id>4</id></row>')

  const readSync = fs.readSync
  const writeFileSync = fs.writeFileSync
  try {
    fs.readSync = () => {
      throw new Error('real XML paths must bypass JavaScript read staging')
    }
    assert.deepEqual(xml.load(pathToFileURL(source)), { row: { id: '4' } })
    assert.deepEqual(codec.from(pathToFileURL(source)), { row: { id: '4' } })

    fs.writeFileSync = () => {
      throw new Error('real XML paths must bypass JavaScript write staging')
    }
    xml.dump({ row: { id: '5' } }, destination)
  } finally {
    fs.readSync = readSync
    fs.writeFileSync = writeFileSync
  }

  try {
    assert.deepEqual(xml.load(pathToFileURL(destination)), { row: { id: '5' } })
    const descriptor = fs.openSync(destination, 'r')
    try {
      assert.deepEqual(xml.load(descriptor), { row: { id: '5' } })
    } finally {
      fs.closeSync(descriptor)
    }
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }

  async function* split() {
    yield '<row><label>\ud83d'
    yield '\ude42</label></row>'
  }
  const decoded = await xml.load(split())
  assert.deepEqual(decoded, { row: { label: '\u{1f642}' } })

  const webInput = new ReadableStream({
    start(controller) {
      controller.enqueue(Buffer.from('<row><id>6</id></row>'))
      controller.close()
    },
  })
  assert.deepEqual(await xml.load(webInput), { row: { id: '6' } })

  const chunks = []
  const output = new Writable({
    write(chunk, _encoding, done) {
      chunks.push(Buffer.from(chunk))
      done()
    },
  })
  await xml.dump({ row: { id: '7' } }, output)
  assert.deepEqual(xml.loads(Buffer.concat(chunks)), { row: { id: '7' } })
  assert.equal(output.writableEnded, false)

  const webChunks = []
  let webClosed = false
  const webOutput = new WritableStream({
    write(chunk) {
      webChunks.push(Buffer.from(chunk))
    },
    close() {
      webClosed = true
    },
  })
  await codec.into({ row: { id: '8' } }, webOutput, { format: 'xml' })
  assert.deepEqual(xml.loads(Buffer.concat(webChunks)), { row: { id: '8' } })
  assert.equal(webClosed, false)
})

test('XML limits and depth stay native', () => {
  assert.throws(() => xml.loads('<row><id>1</id></row>', { maxInputBytes: 3 }), /input byte limit/i)
  assert.throws(() => xml.loads('<row><a/><b/><c/></row>', { maxNodes: 2 }), /node limit/i)

  let nested = { leaf: 'x' }
  for (let index = 0; index < 24; index += 1) nested = { nested }
  const document = xml.dumps({ row: nested })
  assert.deepEqual(xml.loads(document), { row: nested })
  assert.throws(() => xml.loads(document, { maxDepth: 8 }), /depth/i)

  let deep = { value: 'x' }
  for (let index = 0; index < 49; index += 1) deep = { nested: deep }
  assert.throws(() => xml.dumps({ row: deep }), /maxDepth 48/)
})
