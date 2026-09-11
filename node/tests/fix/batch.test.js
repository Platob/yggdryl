'use strict'

// The codec's streams and Arrow twins, and the message holder's setters.
//
// Every rule here is the core's, pinned in `rust/tests/fix/batch.rs` and
// `rust/tests/fix/message.rs`; what these check is the crossing - that a
// JavaScript iterable is pulled one item at a time, that batch readers cross
// both ways, that raw-byte batching is observable through `batchByteSize`, and
// that a write to a message is typed by the dictionary and leaves the wire
// alone.

const assert = require('node:assert/strict')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, DataType, Field, Scalar, TextLine, fields, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

const encoder = new TextEncoder()
const PIPE = '|'.charCodeAt(0)

const BULK_CONFIG =
  '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=*,plugin-type=FIX,type=Plugin","type":"read"},' +
  '"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=Plugin":{"Name":"A"},' +
  '"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B"}},"status":200}'

// Every shape a real capture holds, the corpus the readers are tested on.
const CAPTURE = [
  'sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092',
  'raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|',
  '8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|',
  'sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|',
  '8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000',
  'toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1',
  'ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1',
  'After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR',
  'Referential(dbi|equity|dbi;GB00BN7SWP63_XLON_GBX|[quantity-type=])',
  "<Order ClOrdID='XML-1'>body</Order>",
  "Receiving XmlApi: <Execution ExecID='E1'></Execution>",
  'Message rejected because : ignoring OMSSales expiry message',
  'no level printed by this plugin',
  'heartbeat emitted seq=7',
]

const ORDER = '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|'
const REPORT = '8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|'

function configRegistry() {
  const registry = new fix.FixRegistry()
  registry.withUlbridgeFields()
  const direction = Field.from('MsgDirection: msgdirection')
  direction.fix.tag = 385
  registry.insert(direction)
  return registry
}

// The capture as the batches a text reader hands the codec, `rows` lines to a batch.
function capture(lines, rows) {
  const batches = []
  for (let at = 0; at < lines.length; at += Math.max(rows, 1)) {
    const chunk = lines.slice(at, at + Math.max(rows, 1)).map((line) => encoder.encode(line))
    batches.push(new arrow.Table({ body: arrow.vectorFromArray(chunk, new arrow.Binary()) }))
  }
  if (batches.length === 0) {
    return BatchReader.from(new arrow.Table({ body: arrow.vectorFromArray([], new arrow.Binary()) }))
  }
  return BatchReader.from(batches.flatMap((table) => table.batches))
}

// Two hundred wide orders, about 450 bytes each.
function wide() {
  return Array.from(
    { length: 200 },
    (_, index) => `8=FIX.4.4|35=D|11=ORDER-${String(index).padStart(6, '0')}|58=${'x'.repeat(400)}|10=0|`,
  )
}

function one(codec, line) {
  const messages = codec.parseLine(Buffer.from(line))
  const message = messages.next().value
  assert.equal(messages.next().done, true, 'one message')
  return message
}

function column(table, name) {
  return table.getChild(name).toJSON()
}

function rowCounts(reader) {
  return [...reader].map((batch) => batch.numRows)
}

// Every tag a message's children carry, beside the value each holds.
function stated(message) {
  const held = new Map()
  for (let at = 0; at < message.field.fieldLen; at += 1) {
    const tag = message.field.fieldAt(at).fix.tag
    if (tag !== null) held.set(tag, message.byTag(tag))
  }
  return held
}

test('parseLines pulls one line at a time and continues past a refused one', () => {
  const codec = new fix.FixCodec(configRegistry())
  let pulled = 0
  function* lines() {
    for (const line of [BULK_CONFIG, '']) {
      pulled += 1
      yield line
    }
  }
  const messages = codec.parseLines(lines())
  assert.ok(messages instanceof fix.FixMessages)
  assert.equal(messages[Symbol.iterator](), messages)
  // Nothing is pulled until the stream is asked.
  assert.equal(pulled, 0)
  assert.equal(messages.next().value.byName('Name').asJs(), 'A')
  assert.equal(pulled, 1)
  // The second MBean comes out of the same line, without the next pull.
  assert.equal(messages.next().value.byName('Name').asJs(), 'B')
  assert.equal(pulled, 1)
  // An empty line is not a row at all: thrown where it is met, and the
  // stream goes on to say it is done.
  assert.throws(() => messages.next(), /captured row/)
  assert.equal(pulled, 2)
  assert.equal(messages.next().done, true)
  assert.equal(messages.next().done, true)
})

test('an item that is not bytes is refused where it is met', () => {
  const codec = new fix.FixCodec(seed())
  const mixed = codec.parseLines([Buffer.from('8=FIX.4.4|35=D|11=A|10=0|'), 42])
  assert.equal(mixed.next().value.byTag(11).asJs(), 'A')
  assert.throws(() => mixed.next(), TypeError)
  assert.equal(mixed.next().done, true)
  // A generator that throws throws as itself.
  function* failing() {
    yield '8=FIX.4.4|35=D|11=A|10=0|'
    throw new RangeError('the source broke')
  }
  const broken = codec.parseLines(failing())
  assert.equal(broken.next().value.byTag(11).asJs(), 'A')
  assert.throws(() => broken.next(), RangeError)
  // Something that is not iterable is refused before anything is pulled.
  assert.throws(() => codec.parseLines(42), TypeError)
})

test('parseTextLines pulls one line at a time', () => {
  const codec = new fix.FixCodec(seed(), { captureNames: ['beginstring'] })
  let pulled = 0
  function* lines() {
    for (const body of ['8=FIX.4.4|35=D|11=A|10=0|', '8=FIX.4.4|35=D|11=B|10=0|']) {
      pulled += 1
      yield new TextLine(pulled - 1, Buffer.from(body))
    }
  }
  const messages = codec.parseTextLines(lines())
  assert.equal(pulled, 0)
  assert.equal(messages.next().value.byTag(11).asJs(), 'A')
  assert.equal(pulled, 1)
  assert.equal(messages.next().value.byTag(11).asJs(), 'B')
  assert.equal(messages.next().done, true)
  // A capture speaks per row: `beginstring` reads the frame at 4.2, where tag
  // 32 is `lastshares`, and the column is still the dictionary's own.
  const [old] = codec.parseTextLines([
    new TextLine(0, Buffer.from('8=FIX.4.4|35=8|32=100|10=0|'), ['FIX.4.2']),
  ])
  assert.notEqual(old.field.indexOf('lastqty'), null)
  assert.notEqual(old.getByName('lastshares'), null)
  // A bulk document is many messages, and the stream door yields each.
  assert.equal(
    [...new fix.FixCodec(configRegistry()).parseTextLines([new TextLine(0, Buffer.from(BULK_CONFIG))])].length,
    2,
  )
})

test("a row's pluginid fills its own column and selects nothing", () => {
  // A venue field beside the specification's: one namespace, so `VENUETAG`
  // resolves whatever the row's plugin is called, and the membership the
  // field carries is provenance a caller filters on.
  const registry = seed()
  for (const [name, tag, dialect] of [['VenueTag', 5001, 'venue'], ['OtherTag', 5002, 'elsewhere']]) {
    const field = Field.from(`${name}: utf8`)
    field.fix.tag = tag
    field.fix.branches = [dialect]
    registry.insert(field)
  }
  assert.deepEqual(registry.dialects(), ['elsewhere', 'venue'])
  const captureNames = ['pluginid', 'prevpluginid']
  const codec = new fix.FixCodec(registry, { captureNames })
  const body = Buffer.from('MSGTYPE=D|CLORDID=A|VENUETAG=dark')
  // A line and the captures its header declared, in that order.
  const lined = (plugin, previous = null, held = body) => new TextLine(0, held, [plugin, previous])

  // A `pluginid` capture - a plugin named like a dictionary, one no
  // dictionary is named after, a null, an empty string - fills the crate's
  // own field exactly as it was spelled and selects no dialect: the venue's
  // field resolves under every one of them.
  for (const spelled of ['venue', 'VENUE', 'OMS_X1_TradeCapture', null, '']) {
    const [message] = codec.parseTextLine(lined(spelled))
    assert.equal(message.byTag(5001).asJs(), 'dark', `${spelled}`)
    assert.equal(message.byName('venuetag').asJs(), 'dark', `${spelled}`)
    const held = message.getByName('pluginid')
    assert.equal(held === null ? null : held.asJs(), spelled, `${spelled}`)
    // A message root is not a dictionary member.
    assert.deepEqual(message.field.fix.branches, [])
  }

  // Nothing fills `prevpluginid` but a capture of that name, and the two
  // session names are only ever what the line itself spells, through the
  // aliases a bridge row writes them under.
  const [carried] = codec.parseTextLine(lined('venue', 'ULFilter'))
  assert.equal(carried.getByName('prevpluginid').asJs(), 'ULFilter')
  const spoken = '|#SYMBOL=TTF|#ULFROMSESSIONNAME=OMS_X1_OrderOut|#ULTOSESSIONNAME=ULMSG_BROKER_BDG_DMZ_CLI|'
  const [stated] = codec.parseTextLine(lined('venue', null, Buffer.from(spoken)))
  assert.equal(stated.getByName('sendersessionname').asJs(), 'OMS_X1_OrderOut')
  assert.equal(stated.getByName('targetsessionname').asJs(), 'ULMSG_BROKER_BDG_DMZ_CLI')
  assert.equal(stated.getByName('prevpluginid'), null)
  assert.equal(stated.getByName('sendersessionid'), null)
})

test('the codec answers the pins it was given', () => {
  const registry = seed()
  const bare = new fix.FixCodec(registry)
  assert.equal(bare.version, null)
  assert.equal(bare.separator, null)
  assert.equal(bare.payloadColumn, 'body')
  assert.deepEqual(bare.nullValues, ['', 'null', '<null>'])
  assert.equal(bare.direction, 'sent')
  // The default target, stated once in the core and read here.
  assert.equal(bare.batchByteSize, 128 * 1024 * 1024)

  const pinned = new fix.FixCodec(registry, {
    version: 'FIX.4.2',
    separator: PIPE,
    payloadColumn: 'line',
    nullValues: ['<none>'],
    direction: 'RECV',
    batchByteSize: 4096,
  })
  assert.equal(pinned.version, '4.2')
  // No pin names a dialect: the dictionary is one namespace.
  assert.equal('branch' in pinned, false)
  assert.equal(pinned.separator, PIPE)
  assert.equal(pinned.payloadColumn, 'line')
  assert.deepEqual(pinned.nullValues, ['<none>'])
  assert.equal(pinned.direction, 'recv')
  assert.equal(pinned.batchByteSize, 4096)
  assert.equal(new fix.FixCodec(registry, { direction: 'unknown' }).direction, 'unknown')
  assert.throws(() => new fix.FixCodec(registry, { direction: 'sideways' }), /sent, recv, unknown/)
  assert.throws(() => new fix.FixCodec(registry, { batchByteSize: 1.5 }))
  // The payload column names a batch column; a line's body is its own, so
  // the line door reads the same frame without naming anything.
  const [read] = pinned.parseTextLines([new TextLine(0, Buffer.from('8=FIX.4.2|35=D|11=A|10=0|'))])
  assert.equal(read.byTag(11).asJs(), 'A')
})

test('the schema is decided before the first row is read', () => {
  const reader = new fix.FixCodec(seed()).parseTextArrowReader(capture([], 1))
  const names = []
  for (let at = 0; at < reader.field.fieldLen; at += 1) names.push(reader.field.fieldAt(at).name)
  // The capture's own column leads; the fixed columns follow, named by their
  // folded names, each carrying its tag on the field.
  assert.deepEqual(names.slice(0, 6), ['body', 'beginstring', 'bodylength', 'msgtype', 'sendercompid', 'targetcompid'])
  assert.deepEqual(names.slice(-2), ['nofixentries', 'nounmappedfixentries'])
  assert.equal(reader.field.fieldAt(3).fix.tag, 35)
  // And an empty capture yields no batch at all.
  assert.equal(reader.intoTable().numRows, 0)
})

test('a row in is a row out', () => {
  const parsed = new fix.FixCodec(seed()).parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable()
  assert.equal(parsed.numRows, CAPTURE.length, 'every line, including the ones that are not messages')
  const msgtype = column(parsed, 'msgtype')
  assert.equal(msgtype[0], 'D', 'a framed row states its type')
  assert.equal(msgtype.at(-1), null, 'a sentence states no type')
})

test('several small input batches accumulate and one large batch splits by rows', () => {
  const registry = seed()
  const lines = wide()
  // Twenty input batches of ten rows, far under the default target.
  const whole = rowCounts(new fix.FixCodec(registry).parseTextArrowReader(capture(lines, 10)))
  assert.deepEqual(whole, [200], 'one batch under the byte target')

  // Under a target holding about five input batches, the output batches are
  // fewer than the input ones and no row is lost.
  const bounded = rowCounts(
    new fix.FixCodec(registry, { batchByteSize: 5 * 10 * 470 }).parseTextArrowReader(capture(lines, 10)),
  )
  assert.ok(bounded.length >= 2 && bounded.length < 20, `${bounded.length} batches`)
  assert.equal(bounded.reduce((sum, rows) => sum + rows, 0), 200)
  for (const rows of bounded.slice(0, -1)) assert.ok(rows > 10, 'an input batch did not close an output batch alone')

  // One input batch charges every row the same share of its bytes, so the
  // cut is even, and that many rows of raw capture is about the target.
  const target = 4096
  const split = rowCounts(
    new fix.FixCodec(registry, { batchByteSize: target }).parseTextArrowReader(capture(lines, lines.length)),
  )
  assert.ok(split.length > 1)
  assert.equal(split.reduce((sum, rows) => sum + rows, 0), 200, 'the bound shapes batches, it does not drop rows')
  const closed = split.slice(0, -1)
  assert.ok(closed.every((rows) => rows === closed[0]))
  const perRow = Math.floor(lines.reduce((sum, line) => sum + line.length, 0) / lines.length)
  assert.ok(closed[0] * perRow >= target / 2 && closed[0] * perRow <= target * 2)

  // A target no row fits under closes a batch after every row, so one
  // enormous line can never produce an empty batch.
  const each = rowCounts(new fix.FixCodec(registry, { batchByteSize: 1 }).parseTextArrowReader(capture(lines, lines.length)))
  assert.equal(each.length, 200)
  assert.ok(each.every((rows) => rows === 1))
})

test('messages to batches close on the arrival records raw bytes', () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry)
  const schema = fix.schema(registry)
  const lines = wide()
  assert.deepEqual(rowCounts(codec.arrowReader(schema, codec.parseLines(lines))), [200])

  // A bound of about ten lines of pairs cuts the stream into batches of
  // about ten, and every row survives the cut.
  const many = rowCounts(new fix.FixCodec(registry, { batchByteSize: 10 * 450 }).arrowReader(schema, codec.parseLines(lines)))
  assert.ok(many.length >= 10 && many.length < 40, `${many.length} batches`)
  assert.equal(many.reduce((sum, rows) => sum + rows, 0), 200)
  assert.ok(many.slice(0, -1).every((rows) => rows >= 5 && rows <= 20))

  // A target of one byte is a batch a message.
  assert.equal(rowCounts(new fix.FixCodec(registry, { batchByteSize: 1 }).arrowReader(schema, codec.parseLines(lines))).length, 200)
})

test('the filling reader fills what the filling pass fills and leaves the record alone', () => {
  const codec = new fix.FixCodec(seed())
  const bare = codec.parseTextArrowReader(capture([REPORT], 1)).intoTable()
  assert.deepEqual(column(bare, 'leavesqty'), [null])

  const filled = codec.enrichMessagesArrowReader(codec.parseTextArrowReader(capture([REPORT], 1))).intoTable()
  const [message] = codec.enrichMessages(codec.parseLines([REPORT]))
  // The columns the message pass fills, with the values it fills.
  assert.deepEqual(column(filled, 'leavesqty'), [60])
  assert.equal(message.byTag(151).asJs(), 60)
  assert.deepEqual(column(filled, 'avgpx'), [10.5])
  assert.equal(message.byTag(6).asJs(), 10.5)
  // A derived tag the fixed row does not carry is filled on the message and
  // has no column to appear in: the row is a projection of the message.
  assert.equal(message.byTag(381).asJs(), 420)
  assert.equal(filled.schema.fields.some((field) => field.name === 'grosstradeamt'), false)
  // The schema is the same schema: the carried column still leads.
  assert.deepEqual(filled.schema.fields.map((field) => field.name), bare.schema.fields.map((field) => field.name))
  // The arrival record is untouched either way.
  assert.deepEqual(JSON.stringify(column(filled, 'nofixentries')), JSON.stringify(column(bare, 'nofixentries')))
})

test('messages and arrowReader invert each other', () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry, { nullValues: [] })
  const schema = fix.schema(registry)
  const parsed = [...codec.parseLines(CAPTURE)]
  const again = [...codec.messages(codec.arrowReader(schema, parsed))]
  assert.equal(again.length, parsed.length)
  for (const [at, held] of again.entries()) {
    const message = parsed[at]
    // The same arrival record, the same wire, the same digest, the same
    // stated values by tag, and the same row again.
    assert.deepEqual(held.arrivals(), message.arrivals())
    assert.deepEqual(held.intoBytes(PIPE), message.intoBytes(PIPE))
    assert.deepEqual(held.digest(), message.digest())
    for (const tag of [35, 11, 55, 54, 17, 37]) {
      const value = message.getByTag(tag)
      if (value !== null && value.kind !== 'null') assert.ok(held.byTag(tag).equals(value), `tag ${tag}`)
    }
    assert.ok(held.intoRow(schema).equals(message.intoRow(schema)))
  }
  // And the batches the second pass makes are the batches the first made.
  const first = codec.arrowReader(schema, parsed).intoIpc()
  const second = codec.arrowReader(schema, again).intoIpc()
  assert.deepEqual(first, second)
})

test('messages pull from the reader one batch at a time', () => {
  const codec = new fix.FixCodec(seed())
  const source = codec.parseTextArrowReader(capture(CAPTURE, 3))
  const messages = codec.messages(source)
  assert.ok(messages instanceof fix.FixMessages)
  assert.ok(source.consumed, 'the stream is taken, not copied')
  const first = messages.next().value
  assert.equal(first.byTag(11).asJs(), 'ORDER-1')
  assert.equal(Buffer.from(first.byName('body').asJs()).toString(), CAPTURE[0], 'a carried column is a child of its own name')
  assert.equal([...messages].length, CAPTURE.length - 1)
})

test('a failure behind a batch stream arrives with the batch', () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry)
  const message = one(codec, ORDER)
  const reader = codec.arrowReader(fix.schema(registry), [message, 'not a message'])
  // The batch reader pulls the messages, so the failure is that pull's, and
  // it names the item that was met.
  assert.throws(() => reader.intoTable(), /FixMsg/)
})

test('byte in, byte out over the whole corpus', () => {
  const registry = seed()
  // The convention that drops a stated absence is deliberately not
  // byte-preserving, so it is turned off to measure the reader rather than
  // the convention.
  const codec = new fix.FixCodec(registry, { nullValues: [], separator: PIPE })
  const chunks = []
  const written = codec.writeArrowReader(codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)), {
    write(chunk) {
      chunks.push(Buffer.from(chunk))
    },
  })
  assert.equal(written, CAPTURE.length)
  const back = Buffer.concat(chunks).toString().split('\n')
  assert.equal(back.pop(), '')
  assert.equal(back.length, CAPTURE.length)
  const plain = new fix.FixCodec(registry, { nullValues: [] })
  for (const [at, line] of back.entries()) {
    assert.equal(line, plain.parseLine(Buffer.from(CAPTURE[at])).next().value.intoBytes(PIPE).toString(), CAPTURE[at])
  }
  // An Arrow JS table is a source too, the sink is whatever writes chunks,
  // and a batch without the arrival record is refused before a row is read.
  const facets = codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable().select(['symbol', 'side'])
  const refused = []
  assert.throws(() => codec.writeArrowReader(facets, { write: (chunk) => refused.push(chunk) }), /arrival record/)
  assert.deepEqual(refused, [])
  // A sink that throws throws as itself.
  assert.throws(
    () =>
      codec.writeArrowReader(codec.parseTextArrowReader(capture(CAPTURE, 1)), {
        write() {
          throw new RangeError('disk full')
        },
      }),
    /disk full/,
  )
  assert.throws(() => codec.writeArrowReader(capture(CAPTURE, 1), {}), /write\(chunk/)
})

test('a set value is typed by the registry field and appended when absent', () => {
  const registry = seed()
  const message = one(new fix.FixCodec(registry), ORDER)
  const before = message.size
  const declared = registry.fieldByTag(34)

  message.set(34, 7)

  assert.equal(message.size, before + 1, 'appended, not inserted')
  const child = message.field.fieldAt(before)
  assert.equal(child.name, declared.name, "the dictionary's spelling")
  assert.ok(child.dtype.equals(declared.dtype), "the dictionary's type")
  assert.equal(child.fix.tag, 34)
  assert.equal(child.nullable, false, 'a stated value is non-null')
  assert.equal(message.byTag(34).asJs(), 7)
  assert.equal(message.byName('MsgSeqNum').asJs(), 7, 'reached by name through the registry')
})

test('a set value replaces an existing child in place and keeps the tag index', () => {
  const message = one(new fix.FixCodec(seed()), ORDER)
  const before = stated(message)
  const at = message.field.indexOf('symbol')

  message.set(55, 'MSFT')
  message.set('Side', '2')

  assert.equal(message.field.indexOf('symbol'), at, 'same position')
  assert.equal(message.size, before.size + 2, 'two unknown children beside the tagged ones')
  assert.equal(message.byTag(55).asJs(), 'MSFT')
  assert.equal(message.byTag(54).asJs(), '2')
  for (const [tag, value] of before) {
    if (tag !== 55 && tag !== 54) assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
  }
})

test('a set leaves the entries, the wire and the digest untouched', () => {
  const parsed = one(new fix.FixCodec(seed()), ORDER)
  const message = parsed.clone()
  message.set(55, 'MSFT')
  message.set(38, Scalar.float(100))
  assert.notEqual(message.remove(54), null)
  assert.deepEqual(message.arrivals(), parsed.arrivals())
  assert.equal(message.intoBytes(PIPE).toString(), ORDER)
  assert.deepEqual(message.digest(), parsed.digest(), "the digest is the arrival record's")
  // A null is stored as a stated null.
  message.set(55, null)
  assert.equal(message.field.fieldAt(message.field.indexOf('symbol')).nullable, true)
  assert.equal(message.getByTag(55).kind, 'null')
})

test('an unknown name is refused and the message stands', () => {
  const message = one(new fix.FixCodec(seed()), ORDER)
  const before = message.clone()
  assert.throws(() => message.set('nosuchfield', 'y'), /nosuchfield/)
  assert.ok(message.equals(before))
  // A value the field refuses is refused the same way.
  assert.throws(() => message.set(34, 'not a number'))
  assert.ok(message.equals(before))
  // A key is a tag or a name.
  assert.throws(() => message.set(55n, 'y'), TypeError)
  // An unknown name still reaches the child spelled that way, which keeps
  // its own field.
  const at = message.field.indexOf('venuething')
  message.set('Venue_Thing', '8')
  assert.equal(message.field.fieldAt(at).name, 'venuething')
  assert.ok(message.field.fieldAt(at).dtype.equals(DataType.from('utf8')))
  assert.equal(message.byName('venuething').asJs(), '8')
})

test('a bare unknown tag is appended under its decimal spelling', () => {
  const message = one(new fix.FixCodec(seed()), ORDER)
  message.set(7777, 'custom')
  const child = message.field.fieldAt(message.size - 1)
  assert.equal(child.name, '7777')
  assert.equal(child.nullable, true)
  assert.equal(message.byTag(7777).asJs(), 'custom')
  // A second write reaches the same child rather than a second one.
  message.set(7777, 'again')
  assert.equal(message.byTag(7777).asJs(), 'again')
  assert.equal(message.field.indexOf('7777'), message.size - 1)
  // The one the line already carried is replaced where it stands.
  message.set(9999, 'y')
  assert.equal(message.byTag(9999).asJs(), 'y')
})

test('remove answers the value and the other tags still reach their children', () => {
  const message = one(new fix.FixCodec(seed()), ORDER)
  const before = stated(message)
  const count = message.size

  assert.equal(message.remove(55).asJs(), 'AAPL')
  assert.equal(message.getByTag(55), null)
  assert.equal(message.size, count - 1)
  for (const [tag, value] of before) {
    if (tag !== 55) assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
  }
  // By name, by decimal, and a miss.
  assert.equal(message.remove('VenueThing').asJs(), '7')
  assert.equal(message.remove(9999).asJs(), 'x')
  assert.equal(message.remove(55), null)
  assert.equal(message.remove('nosuchfield'), null)
  assert.equal(message.size, count - 3)
  assert.equal(message.intoBytes(PIPE).toString(), ORDER, 'the entries are untouched')
})

test('a row reads back into the message that made it', () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry)
  const schema = fix.schema(registry)
  const parsed = one(codec, ORDER)
  const row = parsed.intoRow(schema)

  const held = fix.FixMsg.fromRow(schema, row, registry)

  assert.ok(held.field.equals(schema), 'the root is the schema')
  assert.deepEqual(held.arrivals(), parsed.arrivals())
  assert.equal(held.intoBytes(PIPE).toString(), ORDER)
  assert.deepEqual(held.digest(), parsed.digest())
  for (const tag of [8, 35, 11, 55, 54]) assert.ok(held.byTag(tag).equals(parsed.byTag(tag)), `tag ${tag}`)
  // And it makes the row it came from, whole.
  assert.ok(held.intoRow(schema).equals(row))
  // A row stated as a plain object crosses the same gate the constructor
  // does, and the process default is the registry when none is named.
  const named = fix.FixMsg.fromRow(schema, row.asJs(), registry)
  assert.deepEqual(named.arrivals(), parsed.arrivals())
  assert.ok(named.byTag(55).equals(parsed.byTag(55)))
  assert.notEqual(fix.FixMsg.fromRow(schema, row).registry, null)
})

test("a row carrying its capture's own columns returns to its schema whole", () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry)
  const line = fields.struct('line', [fields.utf8('url'), fields.int64('rownum'), fields.binary('body')], { nullable: false })
  const schema = fix.schemaCarrying(line, fix.schema(registry))
  const parsed = one(codec, ORDER)

  // A parsed message has no capture columns: they are null in its row.
  const row = parsed.intoRow(schema).asJs()
  assert.equal(row[schema.indexOf('url')], null)
  assert.equal(row[schema.indexOf('rownum')], null)

  // Read back, the message holds them as children, and a written one lands
  // in its column: a column no tag names takes the child of its name.
  const held = fix.FixMsg.fromRow(schema, parsed.intoRow(schema), registry)
  assert.ok(held.intoRow(schema).equals(parsed.intoRow(schema)))
  held.set('url', 'file:///capture.log')
  held.set('rownum', 42n)
  const filled = held.intoRow(schema).asJs()
  assert.equal(filled[schema.indexOf('url')], 'file:///capture.log')
  assert.equal(Number(filled[schema.indexOf('rownum')]), 42)
  assert.equal(filled[schema.indexOf('symbol')], 'AAPL')
  const again = fix.FixMsg.fromRow(schema, held.intoRow(schema), registry)
  assert.deepEqual(again.arrivals(), parsed.arrivals())
  assert.equal(again.byName('url').asJs(), 'file:///capture.log')
})

test('a row without the entries column has no entries', () => {
  const registry = seed()
  const codec = new fix.FixCodec(registry)
  const wideSchema = fix.schema(registry)
  const columns = []
  for (let at = 0; at < wideSchema.fieldLen; at += 1) {
    const held = wideSchema.fieldAt(at)
    if (held.name !== 'nofixentries' && held.name !== 'nounmappedfixentries') columns.push(held)
  }
  const narrow = fields.struct('fix', columns, { nullable: false })
  const parsed = one(codec, ORDER)
  const row = parsed.intoRow(narrow)
  const held = fix.FixMsg.fromRow(narrow, row, registry)
  assert.deepEqual(held.arrivals(), [])
  assert.equal(held.intoBytes(PIPE).length, 0)
  assert.ok(held.byTag(55).equals(parsed.byTag(55)))
  assert.ok(held.intoRow(narrow).equals(row))
  // A row that does not fit the schema is refused.
  assert.throws(() => fix.FixMsg.fromRow(narrow, { nosuchcolumn: 1 }, registry))
})
