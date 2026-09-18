'use strict'

// The one walk over events: `FixCodec.lifecycle` states each message as the
// one after the live message it follows, and `lifecycleArrowReader` is the
// same walk over batches of rows.
//
// Every rule is the core's, pinned in `rust/tests/fix/`; what these check is
// the crossing - the stream a JavaScript iterable feeds one message at a
// time, the facts each walked message carries, and the batch twin.

const assert = require('node:assert/strict')
const path = require('node:path')
const test = require('node:test')

const { BatchReader, DataType, IOBase, Scalar, TextLine, TextOptions, fields, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')
// A second of a ULBridge's own capture, anonymized: the corpus

// A codec that reads every message type. The corpora below are captures, and
// a capture holds the session traffic and the bridge rows stating no type
// that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
// itself.
function reading(registry, options) {
  return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
}

// `rust/tests/fix/dataset.rs` reads.
const CAPTURE = path.join(__dirname, '..', '..', '..', 'rust', 'tests', 'fix', 'ulbridge.log')
// The bridge's own row header, as the core spells it: what a line states
// about itself in front of the payload.
const ROWHEADER =
  String.raw`^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<threadId>[1-9]\d*)` +
  String.raw`(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] ` +
  String.raw`\[(?P<pluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) `
const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

/** The bridge capture as the messages a text read answers. */
function captured(codec) {
  const options = new TextOptions()
  options.rowheader = ROWHEADER
  const messages = []
  for (const line of new IOBase(CAPTURE).readTextLines(options)) {
    for (const message of codec.parseTextLine(line)) messages.push(message)
  }
  return messages
}

/** One order's life, as a venue and its client tell it. */
const LIFE = [
  // The order, sent under the client's own identifier.
  '8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|',
  // Acknowledged under the venue's, which now names the same chain.
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|',
  // Half of it done.
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|',
  // Filled: the chain ends here.
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=100|14=100|151=0|32=50|31=12.6|60=20260102-10:15:33.000|10=0|',
]

test('a message no live one precedes is answered as it came', () => {
  const registry = seed()
  const codec = reading(registry, { defaultSendingTime: SENDING })
  const heartbeat = codec.parseLine(Buffer.from('8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|')).next().value
  const [walked] = codec.lifecycle([heartbeat])
  // No cross code names no chain, so nothing precedes it and its own
  // identity is the chain's.
  assert.equal(walked.crosscode, '')
  assert.equal(walked.crossuuid, walked.curruuid)
  assert.equal(walked.prevuuid, null)
  assert.equal(walked.seqnum, 0)
  assert.ok(walked.equals(heartbeat))

  // Two chains are walked apart: each message follows the live one of its
  // own chain.
  const lines = [
    '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|60=20260102-10:15:30.000|10=0|',
    '8=FIX.4.4|35=D|11=B1|55=MSFT|54=1|60=20260102-10:15:31.000|10=0|',
    '8=FIX.4.4|35=8|11=A1|37=A1|17=E1|150=0|39=0|60=20260102-10:15:32.000|10=0|',
    '8=FIX.4.4|35=8|11=B1|37=B1|17=E2|150=0|39=0|60=20260102-10:15:33.000|10=0|',
  ]
  const walkedPair = [...codec.lifecycle(lines.map((line) => codec.parseLine(Buffer.from(line)).next().value))]
  assert.equal(walkedPair.length, 4)
  assert.equal(walkedPair[0].prevuuid, null)
  assert.equal(walkedPair[1].prevuuid, null, 'another chain, another first message')
  assert.equal(walkedPair[2].prevuuid, walkedPair[0].curruuid)
  assert.equal(walkedPair[3].prevuuid, walkedPair[1].curruuid)
})

test('the stream is lazy, pulls one message at a time and throws what its source throws', () => {
  const registry = seed()
  const codec = reading(registry, { defaultSendingTime: SENDING })
  const parsed = LIFE.map((line) => codec.parseLine(Buffer.from(line)).next().value)

  // What is not iterable is refused before anything is pulled.
  assert.throws(() => codec.lifecycle(42), TypeError)
  // The walk reads its source in order, so an item that is not a message
  // ends the pull and throws in place of the stream's end.
  const mixed = codec.lifecycle([parsed[0], '8=FIX.4.4|35=0|10=0|'])
  assert.equal(mixed.next().done, false)
  assert.throws(() => mixed.next(), TypeError)
  assert.equal(mixed.next().done, true)

  // A failure of the iterable throws as itself, once, and ends the stream.
  function* failing() {
    yield parsed[0]
    throw new RangeError('the source broke')
  }
  const broken = codec.lifecycle(failing())
  assert.equal(broken.next().done, false)
  assert.throws(() => broken.next(), RangeError)
  assert.equal(broken.next().done, true)

  // The stream is its own iterator, and exhaustion is fused.
  const stream = codec.lifecycle(parsed)
  assert.equal(stream[Symbol.iterator](), stream)
  assert.equal([...stream].length, LIFE.length)
  assert.equal(stream.next().done, true)
})

test('the walk crosses Arrow both ways without a second parse', () => {
  const registry = seed()
  const codec = reading(registry, { defaultSendingTime: SENDING })
  const schema = fix.schema(registry)
  const parsed = LIFE.map((line) => codec.parseLine(Buffer.from(line)).next().value)

  const walked = codec.lifecycleArrowReader(codec.arrowReader(schema, parsed))
  assert.ok(walked instanceof BatchReader)
  const back = [...codec.messages(walked)]
  assert.equal(back.length, LIFE.length)
  // The same walk the message stream answers, through the rows: each
  // message states its place in the chain, the one before it, and the
  // content it was parsed from. The identity is the row's own - a clock
  // the intake settled is not a column, so a message read back settles its
  // own - and the chain it names is what the walk states.
  const expected = [...codec.lifecycle(parsed)]
  for (const [at, message] of back.entries()) {
    assert.equal(message.seqnum, expected[at].seqnum, `message ${at}`)
    assert.equal(message.crosscode, expected[at].crosscode, `message ${at}`)
    assert.deepEqual(message.entries(), expected[at].entries(), `message ${at}`)
    assert.equal(message.prevuuid, at === 0 ? null : back[at - 1].curruuid, `message ${at}`)
    assert.deepEqual(message.parentuuids, at === 0 ? [] : [back[at - 1].curruuid], `message ${at}`)
    assert.equal(message.crossuuid, back[0].crossuuid, `message ${at}`)
  }
  // The source is consumed, as every batch door consumes one.
  const source = codec.arrowReader(schema, parsed)
  codec.lifecycleArrowReader(source)
  assert.ok(source.consumed)
})

test('a bridge capture parses whole and walks its chains', () => {
  const registry = seed()
  const codec = reading(registry, {
    captureNames: ['timestamp', 'threadId', 'msgsessionid', 'msgctxid', 'msgseqnum', 'pluginid', 'level'],
  })
  const messages = captured(codec)
  // Every line that carries a message is one message, the JSON documents
  // among them (`rust/tests/fix/dataset.rs`).
  assert.equal(messages.length, 94)

  // What a bridge's row header states reaches the capture, and what its own
  // namespaces state reaches the metadata.
  const report = messages.find((message) => message.header().msgtype === '8')
  assert.equal(report.capture().pluginid, 'ULBridge')
  assert.match(report.capture().msgctxid, /^[0-9a-f]{10}$/)
  assert.match(report.capture().msgsessionid, /^[0-9a-f]{8}$/)
  assert.ok(Object.keys(report.metadata).some((key) => key.startsWith('ullink.')))
  assert.ok(Object.keys(report.metadata).some((key) => key.startsWith('firm.')))
  assert.ok(Object.keys(report.metadata).every((key) => key === key.toLowerCase()))
  // The event reads the report: the instrument, the side, the price and the
  // quantity, and the identifiers the message is known by.
  assert.equal(report.event().isincode, 'CH0012214059')
  assert.equal(report.event().miccode, 'XSWX')
  assert.equal(report.side, 'BUY')
  assert.ok(Object.keys(report.identifiers).includes('clordid'))
  // The parties merge to one group with the counter synced.
  const parties = report.entries().find((entry) => entry.tag === 453)
  assert.equal(parties.value, '8')
  assert.equal(parties.entries.length, 8)

  // The walk states a predecessor for every message that has one.
  const walked = [...codec.lifecycle(messages)]
  assert.equal(walked.length, messages.length)
  assert.equal(walked.filter((message) => message.prevuuid !== null).length, 35)
  assert.equal(walked.filter((message) => message.seqnum > 0).length, 35)
  assert.ok(walked.every((message) => message.parentuuids.length === (message.prevuuid === null ? 0 : 1)))

  // And the Arrow twin answers the same walk over the same corpus.
  const schema = fix.schema(registry)
  const rows = codec.lifecycleArrowReader(codec.arrowReader(schema, messages))
  const chained = [...codec.messages(rows)]
  assert.equal(chained.length, messages.length)
  assert.equal(chained.filter((message) => message.prevuuid !== null).length, 35)
})

test('a transaction time stating only a day leaves the sending clock standing', () => {
  const codec = reading(seed(), { defaultSendingTime: SENDING })
  // `60=20260814` states a day and no clock, so the event is the sending
  // time rather than midnight (`rust/tests/fix/`).
  const day = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|60=20260814|10=0|'))
  assert.equal(day.currunix, 1_704_190_530_000_000_000n)
  assert.equal(day.header().sendingtime, day.currunix)
  const [walked] = codec.lifecycle([day])
  assert.equal(walked.currunix, day.currunix)
})
