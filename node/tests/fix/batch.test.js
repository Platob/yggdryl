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


// A codec that reads every message type. The corpora below are captures, and
// a capture holds the session traffic and the bridge rows stating no type
// that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
// itself.
function reading(registry, options) {
  return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
}

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

const encoder = new TextEncoder()
const PIPE = '|'.charCodeAt(0)
// The one intake clock the Rust suites build undated messages under
// (`fixed_codec` in `rust/tests/fix.rs`), stated here as a message's own
// SendingTime so two builds settle the same identity.
const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)
// The columns a row must carry a value at: the settled identity, and the
// version every message opens with.
const REQUIRED = ['currunix', 'creaunix', 'curruuid', 'crossuuid', 'currhashcode', 'crosshashcode', 'beginstring']

// Two frames on one row: a line is none, one or many messages, and this
// one is two.
const TWO_FRAMES = '8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|'

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
  'no level printed by this logger',
  'heartbeat emitted seq=7',
]

// The capture lines that carry a message. A line that opens no frame, states
// no bridge pair and carries no document carries nothing to read
//: `After Enrichment ->` and `heartbeat emitted seq=7` write
// their pairs into a sentence, which names no separator for them, so they are
// prose that happens to hold an `=`, and the other three hold no pair at all.
const CARRYING = [0, 1, 2, 3, 4, 5, 6, 9, 10].map((at) => CAPTURE[at])

const ORDER = '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|'
const REPORT = '8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|'

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

/** One exact column, as the unscaled coefficients Arrow JS renders it. */
function exactColumn(table, name) {
  return column(table, name).map((held) => (held === null ? null : BigInt(JSON.parse(held))))
}

function mapColumn(table, name) {
  return Array.from(table.getChild(name), (value) => {
    if (value === null) return null
    assert.ok(value instanceof arrow.MapRow)
    return [...value]
  })
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
  const codec = reading(seed())
  let pulled = 0
  function* lines() {
    for (const line of [TWO_FRAMES, '']) {
      pulled += 1
      yield line
    }
  }
  const messages = codec.parseLines(lines())
  assert.ok(messages instanceof fix.FixMessages)
  assert.equal(messages[Symbol.iterator](), messages)
  // Nothing is pulled until the stream is asked.
  assert.equal(pulled, 0)
  assert.equal(messages.next().value.byTag(11).asJs(), 'A')
  assert.equal(pulled, 1)
  // The second frame comes out of the same line, without the next pull.
  assert.equal(messages.next().value.byTag(11).asJs(), 'B')
  assert.equal(pulled, 1)
  // An empty line is not a row at all: thrown where it is met, and the
  // stream goes on to say it is done.
  assert.throws(() => messages.next(), /captured row/)
  assert.equal(pulled, 2)
  assert.equal(messages.next().done, true)
  assert.equal(messages.next().done, true)
})

test('an item that is not bytes is refused where it is met', () => {
  const codec = reading(seed())
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
  const codec = reading(seed(), { captureNames: ['beginstring'] })
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
  // 32 is `lastshares`, and what it restates to is the quantity the event
  // last traded rather than a column beside it.
  const [old] = codec.parseTextLines([
    new TextLine(0, Buffer.from('8=FIX.4.4|35=8|32=100|10=0|'), ['FIX.4.2']),
  ])
  assert.equal(old.field.indexOf('lastqty'), null, 'the event holds it')
  assert.equal(old.lastqty, '100')
  assert.notEqual(old.getByName('lastshares'), null)
  // A row of two frames is two messages, and the stream door yields each.
  assert.equal([...codec.parseTextLines([new TextLine(0, Buffer.from(TWO_FRAMES))])].length, 2)
})

test("a row's msgpluginid fills its own column and selects nothing", () => {
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
  const captureNames = ['msgpluginid', 'prevmsgpluginid']
  const codec = reading(registry, { captureNames })
  const body = Buffer.from('MSGTYPE=D|CLORDID=A|VENUETAG=dark')
  // A line and the captures its header declared, in that order.
  const lined = (plugin, previous = null, held = body) => new TextLine(0, held, [plugin, previous])

  // A `msgpluginid` capture - a plugin named like a dictionary, one no
  // dictionary is named after, a null, an empty string - fills the crate's
  // own field exactly as it was spelled and selects no dialect: the venue's
  // field resolves under every one of them.
  for (const spelled of ['venue', 'VENUE', 'OMS_X1_TradeCapture', null, '']) {
    const [message] = codec.parseTextLine(lined(spelled))
    assert.equal(message.byTag(5001).asJs(), 'dark', `${spelled}`)
    assert.equal(message.byName('venuetag').asJs(), 'dark', `${spelled}`)
    // A capture is the message's own, typed: an empty spelling states
    // nothing, as an absent one does.
    assert.equal(message.capture().msgpluginid, spelled === '' ? null : spelled, `${spelled}`)
    // A message root is not a dictionary member.
    assert.deepEqual(message.field.fix.branches, [])
  }

  // Nothing fills `prevmsgpluginid` but a capture of that name, and the two
  // session names are only ever what the line itself spells, through the
  // aliases a bridge row writes them under.
  const [carried] = codec.parseTextLine(lined('venue', 'ULFilter'))
  assert.equal(carried.capture().msgpluginid, 'venue')
  const spoken = '|#SYMBOL=TTF|#TECH.CLIENTID=MCFP2|'
  const [stated] = codec.parseTextLine(lined('venue', null, Buffer.from(spoken)))
  // A bridge's own namespaced key is the message's metadata, folded once.
  assert.deepEqual(stated.metadata, { 'tech.clientid': 'MCFP2' })
  assert.equal(stated.capture().msgsessionid, null)
})

test('the codec answers the pins it was given', () => {
  const registry = seed()
  const bare = reading(registry)
  // A codec pins no version: a row states one, or the line implies it.
  assert.equal(bare.version, undefined)
  assert.equal(bare.separator, null)
  assert.equal(bare.payloadColumn, 'body')
  assert.deepEqual(bare.nullValues, ['', 'null', '<null>'])
  assert.equal(bare.direction, 'S')
  // The default target, stated once in the core and read here.
  assert.equal(bare.batchByteSize, 128 * 1024 * 1024)

  const pinned = reading(registry, {
    separator: PIPE,
    payloadColumn: 'line',
    nullValues: ['<none>'],
    direction: 'Receive',
    batchByteSize: 4096,
  })
  // No pin names a dialect: the dictionary is one namespace.
  assert.equal('branch' in pinned, false)
  assert.equal(pinned.separator, PIPE)
  assert.equal(pinned.payloadColumn, 'line')
  assert.deepEqual(pinned.nullValues, ['<none>'])
  assert.equal(pinned.direction, 'R')
  assert.equal(pinned.batchByteSize, 4096)
  // The empty text is no pin; a spelling outside tag 385's set is refused
  // naming the set.
  assert.equal(reading(registry, { direction: '' }).direction, null)
  assert.throws(() => reading(registry, { direction: 'sideways' }), /R, S/)
  assert.throws(() => reading(registry, { batchByteSize: 1.5 }))
  // The payload column names a batch column; a line's body is its own, so
  // the line door reads the same frame without naming anything.
  const [read] = pinned.parseTextLines([new TextLine(0, Buffer.from('8=FIX.4.2|35=D|11=A|10=0|'))])
  assert.equal(read.byTag(11).asJs(), 'A')
})

test("threads read what one thread reads and a message carries its row's cells", () => {
  const one = reading(seed())
  const four = reading(seed(), { threads: 4 })
  assert.equal(one.threads, 1)
  assert.equal(four.threads, 4)
  assert.equal(reading(seed(), { threads: 0 }).threads, 1)
  // The line doors answer on four threads what they answer on one: the
  // same messages, in the same order.
  // By content code and wire: the lines state no clock, so each parse dates
  // them by its own now, and the identity the instant derives differs.
  const stated = (messages) => [...messages].map((held) => [held.currhashcode, held.intoText('|')])
  const lines = CAPTURE.map((line) => Buffer.from(line))
  assert.deepEqual(stated(four.parseLines(lines)), stated(one.parseLines(lines)))
  const parsed = four.parseTextArrowReader(capture(CAPTURE, 3)).intoTable()
  assert.deepEqual(stated(four.messages(parsed)), stated(one.messages(parsed)))
  // A message read back out of a row carries the row's own cells - the
  // body its line was cut from - and one parsed from bytes carries none.
  const [held] = one.messages(parsed)
  assert.deepEqual(Object.keys(held.carried), ['body'])
  assert.equal(Buffer.from(held.carried.body.asJs()).toString(), CAPTURE[0])
  assert.deepEqual(one.parseLine(lines[0]).next().value.carried, {})
})

test('the schema is decided before the first row is read', () => {
  const reader = reading(seed()).parseTextArrowReader(capture([], 1))
  const names = []
  for (let at = 0; at < reader.field.fieldLen; at += 1) names.push(reader.field.fieldAt(at).name)
  // The capture's own column leads; the fixed columns follow, opening on
  // when the event happened, each named by its folded name and carrying
  // its tag.
  assert.equal(names[0], 'body')
  assert.equal(names[1], 'currunix')
  const header = names.indexOf('beginstring')
  assert.ok(header > 0)
  assert.equal(reader.field.fieldAt(header).fix.tag, 8)
  // One list closes the row: the whole content record, unresolved keys at
  // tag 0, under the counter that counts it.
  assert.deepEqual(names.slice(-2), ['nofixentries', 'fixentries'])
  assert.equal(reader.field.fieldAt(names.indexOf('msgtype')).fix.tag, 35)
  // And an empty capture yields no batch at all.
  assert.equal(reader.intoTable().numRows, 0)
})

test('a capture answers one row per message, not one per line', () => {
  const parsed = reading(seed()).parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable()
  assert.equal(parsed.numRows, CARRYING.length, 'one row a message; the text reader answers one a line')
  assert.deepEqual(
    column(parsed, 'body').map((body) => Buffer.from(body).toString()),
    CARRYING,
  )
  const msgtype = column(parsed, 'msgtype')
  assert.equal(msgtype[0], 'D', 'a framed row states its type')
  assert.equal(msgtype.at(-1), null, 'a document that states no type is `unknown`')
})

test('several small input batches accumulate and one large batch splits by rows', () => {
  const registry = seed()
  const lines = wide()
  // Twenty input batches of ten rows, far under the default target.
  const whole = rowCounts(reading(registry).parseTextArrowReader(capture(lines, 10)))
  assert.deepEqual(whole, [200], 'one batch under the byte target')

  // Under a target holding about five input batches, the output batches are
  // fewer than the input ones and no row is lost.
  const bounded = rowCounts(
    reading(registry, { batchByteSize: 5 * 10 * 470 }).parseTextArrowReader(capture(lines, 10)),
  )
  assert.ok(bounded.length >= 2 && bounded.length < 20, `${bounded.length} batches`)
  assert.equal(bounded.reduce((sum, rows) => sum + rows, 0), 200)
  for (const rows of bounded.slice(0, -1)) assert.ok(rows > 10, 'an input batch did not close an output batch alone')

  // One input batch charges every row the same share of its bytes, so the
  // cut is even, and that many rows of raw capture is about the target.
  const target = 4096
  const split = rowCounts(
    reading(registry, { batchByteSize: target }).parseTextArrowReader(capture(lines, lines.length)),
  )
  assert.ok(split.length > 1)
  assert.equal(split.reduce((sum, rows) => sum + rows, 0), 200, 'the bound shapes batches, it does not drop rows')
  // The charge is what each row lands as - the leaves of every column and
  // a per-row width - so the cut is even and the count is the target's,
  // not the raw line's.
  const closed = split.slice(0, -1)
  assert.ok(closed.every((rows) => rows === closed[0]))
  assert.ok(closed[0] >= 1)

  // A target no row fits under closes a batch after every row, so one
  // enormous line can never produce an empty batch.
  const each = rowCounts(reading(registry, { batchByteSize: 1 }).parseTextArrowReader(capture(lines, lines.length)))
  assert.equal(each.length, 200)
  assert.ok(each.every((rows) => rows === 1))
})

test('messages to batches close on the arrival records raw bytes', () => {
  const registry = seed()
  const codec = reading(registry)
  const schema = fix.schema(registry)
  const lines = wide()
  assert.deepEqual(rowCounts(codec.arrowReader(schema, codec.parseLines(lines))), [200])

  // A bound of about ten lines of pairs cuts the stream into batches of
  // about ten, and every row survives the cut.
  const many = rowCounts(reading(registry, { batchByteSize: 10 * 450 }).arrowReader(schema, codec.parseLines(lines)))
  assert.ok(many.length > 1 && many.length < 40, `${many.length} batches`)
  assert.equal(many.reduce((sum, rows) => sum + rows, 0), 200)
  assert.ok(many.slice(0, -1).every((rows) => rows > 0))

  // A target of one byte is a batch a message.
  assert.equal(rowCounts(reading(registry, { batchByteSize: 1 }).arrowReader(schema, codec.parseLines(lines))).length, 200)
})

test('a parse fills what the dictionary derives, through both doors', () => {
  const codec = reading(seed())
  const rows = codec.parseTextArrowReader(capture([REPORT], 1)).intoTable()
  const [message] = codec.parseLines([REPORT])
  // There is no enriching pass: what a message implies is filled where it
  // is parsed, so the row a capture lands in states it already.
  // An exact column crosses as its unscaled coefficient at the one scale
  // this crate keeps a number at.
  assert.deepEqual(exactColumn(rows, 'leavesqty'), [60n * 10n ** 18n])
  assert.ok(message.byTag(151).equals(Scalar.decimal(60n)))
  assert.deepEqual(exactColumn(rows, 'avgpx'), [105n * 10n ** 17n])
  assert.ok(message.byTag(6).equals(Scalar.decimal(105n, 1)))
  // A derived tag the fixed row does not carry is filled on the message and
  // has no column to appear in: the row is a projection of the message.
  assert.ok(message.byTag(381).equals(Scalar.decimal(420n)))
  assert.equal(rows.schema.fields.some((field) => field.name === 'grosstradeamt'), false)
  // The carried column still leads the row.
  assert.equal(rows.schema.fields[0].name, 'body')
})

test('messages and arrowReader invert each other', () => {
  const registry = seed()
  const codec = reading(registry, { nullValues: [] })
  const schema = fix.schema(registry)
  const parsed = [...codec.parseLines(CAPTURE)]
  const again = [...codec.messages(codec.arrowReader(schema, parsed))]
  assert.equal(again.length, parsed.length)
  for (const [at, held] of again.entries()) {
    const message = parsed[at]
    // The same arrival record, the same wire, the same digest, the same
    // stated values by tag, and the same row again.
    assert.deepEqual(held.entries(), message.entries())
    assert.equal(held.currhashcode, message.currhashcode)
    assert.equal(held.curruuid, message.curruuid)
    // The row states the sending clock, so a message read back emits it
    // where a parsed one settled it silently.
    assert.equal(
      held.intoBytes(PIPE).toString().replace(/52=[^|]*\|/, ''),
      message.intoBytes(PIPE).toString().replace(/52=[^|]*\|/, ''),
    )
    for (const tag of [35, 11, 55, 54, 17, 37, 65017, 65039]) {
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

test('the identifiers Map crosses native rows and Arrow as a nullable sorted Map', () => {
  const registry = seed()
  const codec = reading(registry, { defaultSendingTime: SENDING })
  const schema = fix.schema(registry)
  // A message states its identifiers as a typed fact: the map the event
  // holds, read back as a plain object on the message and as a Map column
  // in the row, sorted by the key the core sorts on.
  const lines = [
    '8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|',
    '8=FIX.4.4|35=D|10=0|',
  ]
  const messages = lines.map((line) => one(codec, line))
  assert.deepEqual(messages[0].identifiers, { clordid: 'C-001', execid: 'E-09', orderid: 'O-01' })
  assert.deepEqual(Object.keys(messages[0].identifiers), ['clordid', 'execid', 'orderid'])
  assert.deepEqual(messages[1].identifiers, {})

  const table = codec.arrowReader(schema, messages).intoTable()
  const mapping = table.schema.fields.find((field) => field.name === 'identifiers')
  assert.equal(mapping.nullable, true)
  assert.equal(mapping.type.typeId, arrow.Type.Map)
  assert.equal(mapping.type.keysSorted, true)
  const entries = mapping.type.children[0]
  assert.equal(entries.nullable, false)
  assert.equal(entries.type.children[0].nullable, false)
  assert.equal(mapping.type.keyType.typeId, arrow.Type.Utf8)
  assert.equal(mapping.type.valueType.typeId, arrow.Type.Utf8)
  assert.deepEqual(mapColumn(table, 'identifiers'), [
    [['clordid', 'C-001'], ['execid', 'E-09'], ['orderid', 'O-01']],
    null,
  ])

  // And a row read back states the same facts.
  const restored = [...codec.messages(table)]
  assert.equal(restored.length, messages.length)
  for (const [at, held] of restored.entries()) {
    assert.deepEqual(held.identifiers, messages[at].identifiers, `message ${at}`)
    assert.ok(held.intoRow(schema).equals(messages[at].intoRow(schema)), `message ${at}`)
  }

  // The bridge's own namespaced keys cross the same way, as `metadata`.
  const bridged = one(codec, 'MSGTYPE=D|CLORDID=A|TECH.CLIENTID=X1|')
  assert.deepEqual(bridged.metadata, { 'tech.clientid': 'X1' })
  const held = [...codec.messages(codec.arrowReader(schema, [bridged]).intoTable())]
  assert.deepEqual(held[0].metadata, bridged.metadata)
})

test('a scalar alias never takes the identifiers Map name', () => {
  const scalar = fields.utf8('venueid')
  scalar.fix.tag = 9001
  scalar.fix.names = ['Identifiers']
  const registry = fix.FixRegistry.fromFields([scalar])
  // The scalar answers the folded name a lookup asks for; the group is
  // reached by its counter and by the path grammar.
  assert.equal(registry.fieldByName('identifiers').name, 'venueid')
  assert.equal(registry.fieldByCounter(65020).name, 'identifiers')
  assert.equal(registry.fieldByPath('identifiers').name, 'identifiers')
  // A message carries the scalar in its row and the map as its own fact.
  const schema = fields.struct('row', [scalar, registry.fieldByTag(52)], { nullable: false })
  const value = new fix.FixMsg(schema, { venueid: 'scalar', sendingtime: SENDING }, registry)
  assert.equal(value.byTag(9001).asJs(), 'scalar')
  assert.deepEqual(value.identifiers, {})
  value.set(65020, new Map([['clordid', 'C-1']]))
  assert.deepEqual(value.identifiers, { clordid: 'C-1' })
  assert.equal(value.byTag(9001).asJs(), 'scalar')
  assert.equal(value.size, 1, 'the map is a fact, never a row child')
})

test('messages pull from the reader one batch at a time', () => {
  const codec = reading(seed())
  const source = codec.parseTextArrowReader(capture(CAPTURE, 3))
  const messages = codec.messages(source)
  assert.ok(messages instanceof fix.FixMessages)
  assert.ok(source.consumed, 'the stream is taken, not copied')
  const first = messages.next().value
  assert.equal(first.byTag(11).asJs(), 'ORDER-1')
  assert.equal(first.getByName('body'), null, "a carried column stays the row's, never a child of the message")
  assert.equal([...messages].length, CARRYING.length - 1)
})

test('a failure behind a batch stream arrives with the batch', () => {
  const registry = seed()
  const codec = reading(registry)
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
  const codec = reading(registry, { nullValues: [], separator: PIPE, defaultSendingTime: SENDING })
  const chunks = []
  const rows = codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoIpc()
  const written = codec.writeArrowReader(BatchReader.fromIpc(rows), {
    write(chunk) {
      chunks.push(Buffer.from(chunk))
    },
  })
  // One line out per message, so the lines that carried none are not there.
  assert.equal(written, CARRYING.length)
  const back = Buffer.concat(chunks).toString().split('\n')
  assert.equal(back.pop(), '')
  assert.equal(back.length, CARRYING.length)
  // Each written line is what the row's own message emits: the wire is
  // rebuilt from the record, never from the columns.
  const messages = [...codec.messages(BatchReader.fromIpc(rows))]
  assert.equal(messages.length, CARRYING.length)
  for (const [at, line] of back.entries()) {
    assert.equal(line, messages[at].intoBytes(PIPE).toString(), CARRYING[at])
  }
  // An Arrow JS table is a source too, the sink is whatever writes chunks,
  // and a batch without the content record is refused before a row is read.
  const facets = codec.parseTextArrowReader(capture(CAPTURE, CAPTURE.length)).intoTable().select(['symbol', 'side'])
  const refused = []
  assert.throws(() => codec.writeArrowReader(facets, { write: (chunk) => refused.push(chunk) }), /arrival record|content record|fixentries/)
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
  const message = one(reading(registry), ORDER)
  const before = message.size
  const declared = registry.fieldByTag(1)

  message.set(1, 'ACC-1')

  assert.equal(message.size, before + 1, 'appended, not inserted')
  const child = message.field.fieldAt(before)
  assert.equal(child.name, declared.name, "the dictionary's spelling")
  assert.ok(child.dtype.equals(declared.dtype), "the dictionary's type")
  assert.equal(child.fix.tag, 1)
  assert.equal(child.nullable, false, 'a stated value is non-null')
  assert.equal(message.byTag(1).asJs(), 'ACC-1')
  assert.equal(message.byName('Account').asJs(), 'ACC-1', 'reached by name through the registry')

  // A header tag is a typed fact: it fills the holder and the row is
  // exactly as long as it was. So is `Price(44)`, which the message lifts
  // and holds exact, so it takes an exact value and not a float.
  const held = message.size
  message.set(34, 7)
  message.set(44, '10.5')
  assert.equal(message.size, held)
  assert.equal(message.header().msgseqnum, 7)
  assert.equal(message.byTag(34).asJs(), 7)
  assert.equal(message.px, '10.5')
  assert.ok(message.byTag(44).equals(Scalar.decimal(105n, 1)))
})

test('a set value replaces an existing child in place and keeps the tag index', () => {
  const message = one(reading(seed()), ORDER)
  const before = stated(message)
  const at = message.field.indexOf('symbol')

  message.set(55, 'MSFT')
  message.set('Side', '2')

  assert.equal(message.field.indexOf('symbol'), at, 'same position')
  assert.equal(message.size, before.size + 2, 'two unknown children beside the tagged ones')
  assert.equal(message.byTag(55).asJs(), 'MSFT')
  // A side is stored as the explicit value the wire code names.
  assert.equal(message.byTag(54).asJs(), 'SELL')
  // Content identity changes; every other tag still reaches its previous
  // value, the settled clocks and the chain identity included.
  for (const [tag, value] of before) {
    if (tag === 55 || tag === 54) continue
    if (tag === 65017 || tag === 65039) {
      assert.equal(message.byTag(tag).equals(value), false, 'the identity follows the content')
      continue
    }
    assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
  }
  assert.equal(message.byTag(65017).asJs(), message.currhashcode)
})

test('a set leaves the entries, the wire and the digest untouched', () => {
  const parsed = one(reading(seed()), ORDER)
  const message = parsed.clone()
  message.set(55, 'MSFT')
  message.set(1, 'ACC-1')
  // The written `Account(1)` is an entry and the removed `Side(54)` was
  // one, so the two cancel out: the side is an ordinary child now.
  assert.notEqual(message.remove(54), null)
  assert.deepEqual(message.entries().length, parsed.entries().length, 'the written child is an entry')
  // `OrderQty(38)` is a fact the message lifts, so writing it moves no
  // child and adds no entry.
  message.set(38, '100')
  assert.deepEqual(message.entries().length, parsed.entries().length)
  assert.equal(message.qty, '100')
  assert.ok(message.byTag(38).equals(Scalar.decimal(100n)))
  // A null is stored as a stated null.
  message.set(55, null)
  assert.equal(message.field.fieldAt(message.field.indexOf('symbol')).nullable, true)
  assert.equal(message.getByTag(55).kind, 'null')
})

test('an unknown name is refused and the message stands', () => {
  const message = one(reading(seed()), ORDER)
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
  const message = one(reading(seed()), ORDER)
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
  const message = one(reading(seed()), ORDER)
  const before = stated(message)
  const count = message.size
  const wire = message.clone().intoBytes(PIPE).toString()

  assert.equal(message.remove(55).asJs(), 'AAPL')
  assert.equal(message.getByTag(55), null)
  assert.equal(message.size, count - 1)
  for (const [tag, value] of before) {
    if (tag === 55) continue
    if (tag === 65017 || tag === 65039) {
      assert.equal(message.byTag(tag).equals(value), false, 'the identity follows the content')
      continue
    }
    assert.ok(message.byTag(tag).equals(value), `tag ${tag}`)
  }
  // By name, by decimal, and a miss.
  assert.equal(message.remove('VenueThing').asJs(), '7')
  assert.equal(message.remove(9999).asJs(), 'x')
  assert.equal(message.remove(55), null)
  assert.equal(message.remove('nosuchfield'), null)
  assert.equal(message.size, count - 3)
  // The entries follow the row, so the wire no longer states what left it.
  const emitted = message.intoBytes(PIPE).toString()
  assert.ok(wire.includes('55=AAPL'))
  assert.equal(emitted.includes('55=AAPL'), false)
  assert.equal(emitted.includes('9999=x'), false)
})

test('a row is refused where it cannot state the settled identity', () => {
  const registry = seed()
  const codec = reading(registry, { defaultSendingTime: SENDING })
  const schema = fix.schema(registry)
  const parsed = one(codec, ORDER)
  const row = parsed.intoRow(schema)

  // Every required column is one the message settles, so a row nulling one
  // is refused at that column, located, and nothing is built.
  for (const name of REQUIRED) {
    const at = schema.indexOf(name)
    assert.notEqual(at, null, name)
    const cells = Array.from({ length: schema.fieldLen }, (_, index) => row.at(index))
    cells[at] = null
    assert.throws(() => fix.FixMsg.fromRow(schema, cells, registry), new RegExp(name), name)
  }

  // Removing the side takes its child out of the row, and the trait falls
  // back to the unknown it reads off a message that states none.
  const message = parsed.clone()
  const size = message.size
  assert.notEqual(message.remove(54), null)
  assert.equal(message.side, 'UNKNOWN')
  assert.equal(message.size, size - 1)
  // An ordinary child leaves, and the content identity follows it.
  const identity = message.currhashcode
  assert.equal(message.remove('VenueThing').asJs(), '7')
  assert.notEqual(message.currhashcode, identity)
  const settled = message.clone()
  assert.equal(message.remove('absent'), null)
  assert.ok(message.equals(settled))
})

test('a row reads back into the message that made it', () => {
  const registry = seed()
  // A whole-millisecond default SendingTime keeps every replay clock exact, so
  // `asJs` states each one as a `Date` below rather than as wider text.
  const codec = reading(registry, { defaultSendingTime: new Date(1_704_190_530_000) })
  const schema = fix.schema(registry)
  const parsed = one(codec, ORDER)
  const row = parsed.intoRow(schema)

  const held = fix.FixMsg.fromRow(schema, row, registry)

  // The content is the row's children, and the facts are read off the
  // columns that hold them, so the message emits what it emitted.
  assert.deepEqual(held.entries(), parsed.entries())
  assert.equal(held.currhashcode, parsed.currhashcode)
  assert.equal(held.curruuid, parsed.curruuid)
  for (const tag of [8, 35, 11, 55, 54]) assert.ok(held.byTag(tag).equals(parsed.byTag(tag)), `tag ${tag}`)
  // And it makes the row it came from, whole.
  assert.ok(held.intoRow(schema).equals(row))
  // A row stated as plain JavaScript crosses the same gate the constructor
  // does, but the settled identity keeps its exact layouts: `asJs` states
  // the nanosecond clocks as millisecond `Date`s, which a read back refuses
  // at the first required column rather than restating it.
  // Every cell crosses the gate the constructor crosses, so a row stated as
  // plain JavaScript - its clocks as `Date`s, its UUIDs as text - reads back
  // into the same message, and the process default is the registry when none
  // is named.
  const named = fix.FixMsg.fromRow(schema, row.asJs(), registry)
  assert.deepEqual(named.entries(), parsed.entries())
  assert.ok(named.byTag(55).equals(parsed.byTag(55)))
  assert.equal(named.currhashcode, held.currhashcode)
  assert.notEqual(fix.FixMsg.fromRow(schema, row).registry, null)
})

test("a capture's own columns are carried and never become facts", () => {
  const registry = seed()
  const codec = reading(registry)
  // The object the line came out of is one of the capture's own columns:
  // no column of the fixed row states it, so a capture that knows it
  // declares it beside the body and the row number.
  const line = fields.struct(
    'line',
    [fields.utf8('url'), fields.int64('rownum'), fields.binary('body'), fields.url('sourceurl')],
    { nullable: false },
  )
  const schema = fix.schemaCarrying(line, fix.schema(registry))
  // A carried column is nullable whatever the capture declared: only a pass
  // holding the source row can state one.
  assert.ok(schema.field('url').nullable)
  const parsed = one(codec, ORDER)

  // A message parsed out of a line carries nothing: the capture's own
  // columns are null in its row, the one the crate tags among them.
  assert.deepEqual(parsed.carried, {})
  const row = parsed.intoRow(schema).asJs()
  for (const carrier of ['url', 'rownum', 'body', 'sourceurl']) {
    assert.equal(row[schema.indexOf(carrier)], null, carrier)
  }

  // A row a reader stated them on reads back carrying them, each under its
  // column's name: no child, nothing to answer by name, the content
  // identity untouched, and a write to the crate's own column refused
  // rather than silently kept.
  const stated = [...row]
  stated[schema.indexOf('url')] = 'file:///capture.log'
  stated[schema.indexOf('rownum')] = 42n
  const again = fix.FixMsg.fromRow(schema, stated, registry)
  assert.deepEqual(again.entries(), parsed.entries())
  assert.equal(again.currhashcode, parsed.currhashcode)
  assert.deepEqual(Object.keys(again.carried).sort(), ['rownum', 'url'])
  assert.equal(again.carried.url.asJs(), 'file:///capture.log')
  assert.equal(Number(again.carried.rownum.asJs()), 42)
  for (const carrier of ['url', 'rownum', 'body']) {
    assert.equal(again.getByName(carrier), null, carrier)
  }
  assert.throws(() => again.set('sourceurl', 'file:///capture.log'), /sourceurl/)

  // So a message states them again at their columns, and nowhere else.
  const written = again.intoRow(schema).asJs()
  assert.equal(written[schema.indexOf('url')], 'file:///capture.log')
  assert.equal(Number(written[schema.indexOf('rownum')]), 42)
  for (const carrier of ['body', 'sourceurl']) {
    assert.equal(written[schema.indexOf(carrier)], null, carrier)
  }
  assert.equal(written[schema.indexOf('symbol')], 'AAPL')
  assert.ok(!again.intoText('|').includes('65026='))
})

test('a row without the entries column has no entries', () => {
  const registry = seed()
  const codec = reading(registry)
  const wideSchema = fix.schema(registry)
  const columns = []
  for (let at = 0; at < wideSchema.fieldLen; at += 1) {
    const held = wideSchema.fieldAt(at)
    if (held.name !== 'fixentries' && held.name !== 'nofixentries') columns.push(held)
  }
  const narrow = fields.struct('fix', columns, { nullable: false })
  const parsed = one(codec, ORDER)
  const row = parsed.intoRow(narrow)
  const held = fix.FixMsg.fromRow(narrow, row, registry)
  assert.deepEqual(held.entries(), [])
  // The typed facts survive the column that holds each; the content does
  // not, because the record is what it was rebuilt from, so the wire is the
  // header, the fields the message lifted and the trailer, with nothing
  // between them.
  const emitted = held.intoBytes(PIPE).toString()
  assert.equal(emitted, '8=FIX.4.4|35=D|11=A1|10=0|')
  assert.equal(held.header().msgtype, parsed.header().msgtype)
  assert.equal(held.crosscode, parsed.crosscode)
  // The side does not survive: it is an ordinary child, and a row read back
  // without the record has no content for the trait to read it off.
  assert.equal(parsed.side, 'BUY')
  assert.equal(held.side, 'UNKNOWN')
  assert.equal(held.size, 0)
  assert.equal(held.getByTag(55), null)
  // The row it makes states the facts it kept and nothing of the content it
  // could not rebuild - so the identity it writes is its own, over what it
  // now says, rather than the one the parsed message settled.
  const again = held.intoRow(narrow)
  for (const name of ['currunix', 'creaunix', 'crossuuid', 'crosshashcode']) {
    const at = narrow.indexOf(name)
    assert.ok(again.at(at).equals(row.at(at)), name)
  }
  assert.equal(again.at(narrow.indexOf('curruuid')).asJs(), held.curruuid)
  assert.notEqual(held.curruuid, parsed.curruuid)
  assert.equal(again.at(narrow.indexOf('symbol')).kind, 'null')
  assert.equal(row.at(narrow.indexOf('symbol')).asJs(), 'AAPL')
  // A row that does not fit the schema is refused.
  assert.throws(() => fix.FixMsg.fromRow(narrow, { nosuchcolumn: 1 }, registry))
})

test('format answers the rows one message field holds, both doors', () => {
  // Pinned by `format_messages_answers_one_row_per_message_under_the_field`
  // and `format_arrow_reader_answers_the_batches_format_messages_answers_rows`
  // in `rust/tests/fix/format.rs`.
  const registry = seed()
  const codec = reading(registry)
  // The fixed row itself as the target, so a formatted row keeps every column
  // the capture landed in.
  const schema = fix.schema(registry)
  const names = (held) => Array.from({ length: held.fieldLen }, (_, at) => held.fieldAt(at).name)

  const messages = [...codec.parseLine(Buffer.from(ORDER))]
  const rows = codec.formatMessages(messages, schema)
  assert.equal(rows.length, 1)
  const held = rows[0].asJs()
  assert.equal(held[schema.indexOf('symbol')], 'AAPL')
  // The record closes a formatted row exactly as it closes a parsed one.
  assert.ok(held[schema.indexOf('fixentries')].length > 0)

  // The Arrow twin answers the same row, one batch at a time, and decides its
  // schema before a row is read.
  const source = codec.arrowReader(schema, messages)
  const formatted = codec.formatArrowReader(source, schema)
  const table = formatted.intoTable()
  assert.deepEqual(
    table.schema.fields.map((field) => field.name),
    names(schema),
  )
  assert.equal(table.numRows, 1)
  assert.deepEqual(column(table, 'symbol'), ['AAPL'])
})
