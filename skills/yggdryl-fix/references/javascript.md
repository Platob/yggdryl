# yggdryl-fix in JavaScript

`const { fix } = require('yggdryl')` - the namespace holds `FixRegistry`,
`FixCodec`, `FixMsg`, `schema` and the process default; lines are `Buffer`s,
batches are the package's `BatchReader` (Arrow JS crosses as copied IPC). The
examples load `config/fix`, the committed dictionary of a yggdryl checkout (not
shipped in the npm package): point it at your copy, or set `YGGDRYL_FIX_REGISTRY`.

## Load the committed dictionary once and share it

`FixRegistry.fromHandle` takes a path, URL or `IOBase`; install it as the
process default and every door given no registry reads it.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

// `config/fix` of a yggdryl checkout: the dictionary is not shipped in the package.
const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
assert.equal(registry.msgtype('D').name, 'newordersingle')
assert.equal(registry.msgtype('8').name, 'executionreport')

// Installed before anything resolves the default, it is what every default reads.
fix.FixRegistry.installEnv(registry)
assert.ok(fix.FixRegistry.fromEnv().equals(registry))
const orders = new fix.FixCodec(undefined, { includeMsgtypes: ['D'] })
assert.ok(orders.registry.equals(registry))
assert.equal(fix.schema().indexOf('msgtype'), fix.schema(registry).indexOf('msgtype'))
```

## Look fields up in the one namespace

A bare number is a tag, a string a folded name or a dotted path; a field
identity (`field.fix.id`) is only ever looked up through `fieldById`.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))

assert.equal(registry.field(55).name, 'symbol')
assert.equal(registry.fieldByName('Sym_Bol').fix.tag, 55)
assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
// The counter names the group it opens; a path reaches through the group.
assert.equal(registry.fieldByCounter(453).name, 'parties')
assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
// A name reads four word pairs either way: offer/ask, size/qty, bid/demand, px/price.
assert.equal(registry.fieldByName('AskPrice').fix.tag, 133)
assert.equal(registry.fieldByName('DemandQty').fix.tag, 134)

// The identity is the tag and the folded name; a bare number is never one.
const held = registry.field(55).fix.id
assert.equal(registry.fieldById(held).name, 'symbol')
assert.equal(registry.getField(held), null)

// A field reads its values by a named code set the dictionary holds once.
const side = registry.field(54)
assert.equal(side.fix.codeset, 'sidecodeset')
const { name, codes } = registry.codesetOf(side)
assert.equal(name, 'sidecodeset')
assert.deepEqual([codes[0].value, codes[0].name], ['1', 'Buy'])
```

## Put FIX facts on a field

`field.fix` is the protocol view over the field's `FIX:` metadata; a refused
write throws and leaves the field unchanged.

```javascript
const assert = require('node:assert/strict')
const { Field } = require('yggdryl')

const field = Field.from('OrderQty: decimal128(20, 8)')
field.fix.tag = 38
field.fix.names = ['Qty', 'Quantity']
field.fix.sources = ['Venue', 'desk']

assert.equal(field.fix.tag, 38)
assert.equal(field.get('FIX:names'), '["Qty","Quantity"]')
assert.deepEqual(field.fix.sources, ['desk', 'venue'])
assert.equal(field.get('FIX:sources'), '["desk","venue"]')
// Derived on every read from the tag and the folded name, never stored.
const spelled = Field.from('order_qty: int64')
spelled.fix.tag = 38
assert.equal(spelled.fix.id, field.fix.id)
assert.equal(field.has('FIX:id'), false)

assert.throws(() => { field.fix.tag = 0 }, /FIX:tag/)
assert.equal(field.fix.tag, 38)
```

## Decode one captured line

`parseLine` takes a whole captured line as a `Buffer` - verb, prose and remarks
included - and answers an iterable of every message it carries.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

const [message] = codec.parseLine(Buffer.from('recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|'))
// The group is its list: its length is the count, and tag 453 reaches nothing.
assert.equal(message.byName('parties').length, 1)
assert.equal(message.byPath('Parties[0].PartyID').asJs(), 'BROKER')

// Two frames on one line are two messages; a sentence is none.
const both = Buffer.from('8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|')
assert.equal([...codec.parseLine(both)].length, 2)
assert.equal([...codec.parseLine(Buffer.from('After Enrichment -> ACCOUNT=A1 SIDE=1'))].length, 0)
// The single-frame door refuses a body holding a second frame.
assert.throws(() => codec.parseFixLine(both), /expected one frame/)
```

## Read only the message types you need

The type filter is read off the `35=` a row states, before a message is built,
so a refused keepalive costs one look.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const lines = [
  '8=FIX.4.4|35=0|112=TEST|10=0|',
  '8=FIX.4.4|35=D|11=A|55=AAPL|10=0|',
  '8=FIX.4.4|35=8|37=O1|10=0|',
]

// The default refuses Heartbeat, TestRequest and the untyped row.
const codec = new fix.FixCodec(registry)
assert.deepEqual(codec.excludeMsgtypes, ['0', '1', 'unknown'])
assert.equal([...codec.parseLines(lines)].length, 2)

// Naming what to read, in any spelling the dictionary resolves, is the whole answer.
const orders = new fix.FixCodec(registry, { includeMsgtypes: ['NewOrderSingle'] })
assert.deepEqual(orders.includeMsgtypes, ['D'])
assert.equal([...orders.parseLines(lines)].length, 1)

// An empty refusal reads the session whole, as an audit does.
const audit = new fix.FixCodec(registry, { excludeMsgtypes: [] })
assert.equal([...audit.parseLines(lines)].length, 3)

// Routing before any parse: the stated type, read without a dictionary.
assert.equal(fix.FixCodec.inferMsgtypeBytes(Buffer.from('8=FIX.4.4|35=AE|')).toString(), 'AE')
```

## Read typed facts off a message

A message holds each fact once: the header on `header()`, the market reading as
getters (decimals as exact text, instants as `bigint`), everything else in the
content row the dictionary typed.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|202=105|10=0|'))

assert.deepEqual([message.header().beginstring, message.header().msgtype], ['FIX.4.4', 'D'])
// The category its type files under, and the option strike it identifies.
assert.deepEqual([message.marketdatakind, message.strikepx], ['ORDR', '105'])
// A coded value reads as its name; the wire keeps its code.
assert.equal(message.byTag(54).asJs(), 'BUYS')
assert.equal(message.side, 'BUYS')
assert.equal(message.quantity, '100')
assert.equal(message.byName('symbol').asJs(), 'AAPL')
// The first stated OrderID, ClOrdID, ... names the order's chain, stored under its side.
assert.equal(message.crosscode, '10:1:A1')
// The names it goes by are identifiers: a source, a type and a value.
assert.equal(message.identifiers.toString(), '[clordid=A1]')
// Instants are bigint nanoseconds since the epoch, UTC.
assert.equal(message.currunix, 1_767_348_930_000_000_000n)
// The entries are the content row as a tree of { tag, name, value, entries }.
assert.deepEqual(message.entries().map((entry) => entry.name), ['symbol', 'side', 'strikeprice', 'timeinforce'])
```

## Compose a message and write facts

`new fix.FixMsg(root, value, registry)` builds a message from a root `Field` and
a plain object; `set` and `remove` write a typed fact's holder or the row, and
settle the identity again.

```javascript
const assert = require('node:assert/strict')
const { Field, fields, fix } = require('yggdryl')

const tagged = (name, tag) => {
  const field = Field.from(`${name}: utf8`)
  field.fix.tag = tag
  return field
}
const [msgtype, clordid, symbol] = [tagged('MsgType', 35), tagged('ClOrdID', 11), tagged('Symbol', 55)]
const registry = fix.FixRegistry.fromFields([msgtype, clordid, symbol])
const root = fields.struct('NewOrderSingle', [msgtype, clordid, symbol], { nullable: false })
const message = new fix.FixMsg(root, { MsgType: 'D', ClOrdID: 'A1', Symbol: 'AAPL' }, registry)
assert.equal(message.header().msgtype, 'D')
assert.equal(message.crosscode, '10:0:A1')

const before = message.currhashcode
message.set('Symbol', 'MSFT')
assert.equal(message.byTag(55).asJs(), 'MSFT')
assert.notEqual(message.currhashcode, before, 'a write settles the identity again')
assert.equal(message.remove(55).asJs(), 'MSFT')
assert.equal(message.getByTag(55), null)
```

## Encode a message back to the wire

`intoText` / `intoBytes` re-emit what the message states - header, lifted
fields, entries, trailer - with SOH unless you pass a separator; `digest`
hashes those bytes whatever the separator.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|'))

// Header, lifted quantity, entries (the side as its wire code, the derived
// day order), then the trailer.
const text = message.intoText('|')
assert.equal(text, '8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|55=AAPL|54=1|59=0|10=000|')
assert.equal(message.intoText(), text.replaceAll('|', '\x01'))
assert.ok(message.intoBytes().equals(Buffer.from(text.replaceAll('|', '\x01'))))

// Emission is idempotent, and the digest ignores the separator.
const again = codec.parseFixLine(Buffer.from(text))
assert.equal(again.intoText('|'), text)
assert.ok(again.digest().equals(message.digest()))
```

## Stream a capture into Arrow rows

`parseTextArrowReader` takes a `BatchReader` with a payload column (`body` by
default) and answers a `BatchReader` of FIX rows, one per message, the
capture's own columns following the shared ones; parsing is pooled across `threads`.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const arrow = require('apache-arrow')
const { BatchReader, fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const capture = new arrow.Table({
  url: arrow.vectorFromArray(['file:///s.log', 'file:///s.log', 'file:///s.log'], new arrow.Utf8()),
  rownum: arrow.vectorFromArray([7n, 8n, 9n], new arrow.Int64()),
  body: arrow.vectorFromArray([
    'recv 8=FIX.4.4|35=D|11=A|55=AAPL|10=0|',
    'heartbeat emitted seq=7',
    '8=FIX.4.4|35=D|11=B|55=MSFT|10=0|8=FIX.4.4|35=8|37=O1|11=B|10=0|',
  ], new arrow.Utf8()),
})

const codec = new fix.FixCodec(registry, { threads: 4, batchRowSize: 10_000 })
const read = codec.parseTextArrowReader(BatchReader.from(capture))
// The schema is decided before a row is read: the shared columns lead, the capture follows them, `fixentries` closes.
const columns = Array.from({ length: read.field.fieldLen }, (_, index) => read.field.fieldAt(index).name)
assert.equal(columns[0], 'curruuid')
const at = columns.indexOf('url')
assert.deepEqual(columns.slice(at - 1, at + 3), ['partyids', 'url', 'rownum', 'body'])
assert.equal(read.field.fieldAt(read.field.fieldLen - 1).name, 'fixentries')

const held = read.intoTable()
// One row per message: the sentence carried none, the last line two.
assert.deepEqual([...held.getChild('rownum')], [7n, 9n, 9n])
assert.deepEqual([...held.getChild('symbol')], ['AAPL', 'MSFT', null])
```

## Read a log file with a row header

Let the text reader cut lines and capture the row header, then hand its batches
to the codec: an `mtime` capture dates the line, and every other capture either
fills the FIX field it is named after or leads the row as context.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, TextOptions, fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const options = new TextOptions()
options.rowheader = '^(?P<mtime>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\.\\d{3}) (?P<level>IN|OUT) +'
options.timezone = 'UTC'

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const log = path.join(directory, 'session.log')
fs.writeFileSync(log, [
  '2026-01-02 10:15:30.250 IN  8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|10=0|',
  '2026-01-02 10:15:30.500 OUT 8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|10=0|',
  '2026-01-02 10:15:31.000 OUT 8=FIX.4.4|35=0|10=0|',
].join('\n'))
const handle = new IOBase(log).intoText(options)

// Bulk: the text reader's batches straight into FIX rows.
const rows = new fix.FixCodec(registry, { threads: 2 }).parseTextArrowReader(handle.readArrowReader()).intoTable()
assert.equal(rows.numRows, 2, 'the heartbeat is refused by default')
assert.deepEqual([...rows.getChild('level')], ['IN', 'OUT'])
// The line's clock became each message's instant; no SendingTime was invented.
assert.equal(rows.getChild('sendingtime').nullCount, 2)

// Line by line: the handle reads under its own options, and the codec learns
// what the captures are called from them.
const lines = [...handle.readTextLines()]
assert.equal(lines.length, 3)
assert.deepEqual(options.captureNames, ['mtime', 'level'])
const codec = new fix.FixCodec(registry, { captureNames: options.captureNames })
const messages = [...codec.parseTextLines(lines)]
assert.deepEqual(messages.map((message) => message.recdunix), [1_767_348_930_250_000_000n, 1_767_348_930_500_000_000n])

fs.rmSync(directory, { recursive: true, force: true })
```

## Land messages in the fixed row and back

`fix.schema` is the one row every message answers as; `arrowReader` and
`messages` cross between messages and batches, any record medium stores the
batches, and `writeArrowReader` writes them back as wire lines to any
`{ write(chunk) }` sink. A key no dictionary resolves lands in `metadata`,
unless it names an identifier the message captures - then it rides
`fixentries` under `0:<key>`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const codec = new fix.FixCodec(registry, { separator: 124 })
const schema = fix.schema(registry, 'fix')
const lines = ['8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|18=G|9999=x|10=0|', '8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|']
const parsed = [...codec.parseLines(lines)]

// One message as one row, and back; a column is found by name.
const row = parsed[0].intoRow(schema)
assert.equal(row.toJSON()[schema.indexOf('msgtype')], 'D')
// What no column holds is `fixentries`, keyed `tag:name`; a key no dictionary
// resolves is no field, and lands in `metadata` under its own spelling.
assert.deepEqual(row.toJSON()[schema.indexOf('fixentries')], { '18:execinst': 'G' })
assert.deepEqual(row.toJSON()[schema.indexOf('metadata')], { 9999: 'x' })
assert.ok(fix.FixMsg.fromRow(schema, row, registry).intoRow(schema).equals(row))

// An unresolved key naming an identifier is captured: held in its set, it rides
// `fixentries` under `0:<key>` and leaves `metadata`.
const bridged = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=ORDER-2|55=AAPL|54=1|RICCODE=AAPL.O|10=0|'))
assert.equal(bridged.securityids.get('ric'), 'AAPL.O')
const cells = bridged.intoRow(schema).toJSON()
assert.equal(cells[schema.indexOf('fixentries')]['0:riccode'], 'AAPL.O')
assert.equal((cells[schema.indexOf('metadata')] ?? {}).riccode, undefined)

// A stream of messages as batches, landed in Parquet without a per-row detour.
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const stored = new IOBase(path.join(directory, 'capture.parquet'))
stored.overwriteArrowReader(codec.arrowReader(schema, parsed))
const again = [...codec.messages(stored.readArrowReader())]
assert.deepEqual(again.map((message) => message.currhashcode), parsed.map((message) => message.currhashcode))

// And out to the wire, one line per row.
const chunks = []
assert.equal(codec.writeArrowReader(stored.readArrowReader(), { write: (chunk) => chunks.push(Buffer.from(chunk)) }), 2)
assert.ok(Buffer.concat(chunks).toString().startsWith('8=FIX.4.4|35=D|11=ORDER-1|18=G|9999=x|'))
fs.rmSync(directory, { recursive: true, force: true })
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order and side under one `crossuuid`, within one market data kind (`marketdatakind`); a
report stating no side joins the one side alive under its identifiers. A fill's
execution, split off at the parse, is a chain of its own and never restates,
follows or ends its order. A codec pinned `{ sortedLifecycle: true }` reads a source already in
instant order as it comes, one epoch hour at a time, and answers the same walk. The walk yields
each `curruuid` once within `dedupWindowMs` of event time, one minute unless
the codec says otherwise; `{ dedupWindowMs: null }` yields every restated twin too.
A snapshot grid's view is the live message as of its tick: dated at it, so its
`curruuid` is that instant's, with the live message's content and place.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const codec = new fix.FixCodec(registry)
const lines = [
  '8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|',
  '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|',
  '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|',
]
// Parsed, nothing follows anything: each names only the chain it spells.
const parsed = [...codec.parseLines(lines)]
assert.ok(parsed.every((held) => held.prevuuid === null))
// Each takes its place at its instant: the execution stands after its report.
assert.deepEqual(parsed.map((held) => held.seqnum), [0, 1, 0, 0])
// Three lines, four messages: the fill's report and the execution it reports.
assert.equal(parsed.length, 4)

const [order, ack, fill, execution] = codec.lifecycle(parsed)
// Sorted by event time, joined by the identifiers each message went by; each
// follows one of an earlier instant, so each keeps its own place.
assert.deepEqual([order.seqnum, ack.seqnum, fill.seqnum], [0, 0, 0])
assert.equal(ack.prevuuid, order.curruuid)
assert.equal(fill.prevuuid, ack.curruuid)
assert.ok([ack, fill].every((held) => held.crossuuid === order.crossuuid))
// The reports stated no side: they joined the buy alive under A1 and O1.
assert.ok([ack, fill].every((held) => held.side === 'BUYS' && held.crosscode === '10:1:A1'))
assert.deepEqual([fill.marketdatakind, fill.state], ['ORDR', 'FILLED'])
// Every walked message states when its chain began.
assert.ok([ack, fill].every((held) => held.creaunix === order.currunix))
assert.deepEqual([execution.marketdatakind, execution.state], ['EXEC', 'FILLED'])
assert.deepEqual([execution.seqnum, execution.prevuuid], [1, null])

// Rows already in Arrow chain in place, under the schema they were read with.
const rows = codec.arrowReader(fix.schema(registry), codec.parseLines(lines))
const chained = codec.lifecycleArrowReader(rows).intoTable()
// Two chains: the order's, and its fill's execution.
assert.equal(chained.numRows, 4)
assert.equal(new Set([...chained.getChild('crossuuid')].map(String)).size, 2)
```

## Share what lifecycles learn about instruments

A lifecycle learns each message's ISIN - the one key - its CFI code, country,
market, ticker, currency, pair and security codes into an `IsinRegistry`, and
fills what later messages of that instrument leave unsaid, as `derived`
identifiers and the ticker, CFI and currency facts, never the wire; a parse
through the same codec fills derived identifiers from the table its door
fixed. A codec without one learns into a registry of each walk's own;
`isinRegistry` shares one across walks run one after another, bound to a
store with `fromUrl` and written back with `commit()` only where it moved,
and `FixCodec.fromEnv()` shares the process's own, `IsinRegistry.fromEnv()`.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { IsinRegistry, fix } = require('yggdryl')

const instruments = new IsinRegistry()
const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')), { isinRegistry: instruments })

// The first walk states Holcim's ISIN, RIC, CFI code, ticker and market.
const stated = '8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|10=0|'
for (const _ of codec.lifecycle([...codec.parseLines([Buffer.from(stated)])])) void _
assert.equal(instruments.get('CH0012214059').ric, 'HOLN.S')

// A later parse naming only the ticker on the market takes the ISIN from the
// table, derived; the walk fills the CFI code as a market fact.
const [parsed] = [...codec.parseLines([Buffer.from('8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|')])]
assert.equal(parsed.isincode, 'CH0012214059')
assert.ok(parsed.securityids.isDerived('isin'))
const [later] = [...codec.lifecycle([parsed])]
assert.equal(later.cficode, 'ESVUFR')
// The table is an Arrow stream: a golden file loads with `fromUrl`.
assert.notEqual(IsinRegistry.fromArrowReader(instruments.intoArrowReader()).get('CH0012214059'), null)
```

## Follow a replace chain's parents

A message that states an identifier again under another value is a step in
its chain: `lifecycle` keeps the value before it as the type's parent
(`orderid` leaves `parentorderid` and the chain's first as `origorderid`,
`clordid` leaves `origclordid`), and joins a replace to its order by that
parent too. `registry.parentsOf('orderid')` lists them, nearest first, from
the `FIX:parents` a field states.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const KINDS = ['orderid', 'parentorderid', 'origorderid', 'clordid', 'origclordid']
const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
const lines = [
  '8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|',
  '8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|',
  '8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|',
  '8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|',
].map((line) => Buffer.from(line))
const chained = [...reader.lifecycle([...reader.parseLines(lines)])]

// What a message holds under each type, '-' where it holds none.
const held = (message) => KINDS.map((kind) => message.identifiers.get(kind) ?? '-')
assert.deepEqual(held(chained[0]), ['-', '-', '-', 'C1', '-'])
assert.deepEqual(held(chained[1]), ['O1', '-', '-', 'C1', '-'])
// Each replace names the value before it and the chain's first.
assert.deepEqual(held(chained[2]), ['O2', 'O1', 'O1', 'C2', 'C1'])
assert.deepEqual(held(chained[3]), ['O3', 'O2', 'O1', 'C3', 'C2'])
// One chain: the replaces joined the order by the parent they state.
assert.ok(chained.every((message) => message.crossuuid === chained[0].crossuuid))
```

## Split fills and batches at the parse

The parse splits what a message reports, once, so nothing downstream states a
fill twice: an execution report is its order's report (`marketdatakind` `ORDR`, its
own state; `QUOT` where it names a `QuoteID(117)`) - one of no fill from its
parse - and one that fills adds one `EXEC` message reading `FILLED`, chained
under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`; a trade
(`AE`) adds one sided execution per `NoSides(552)` occurrence; a batch (`marketdatakind` `ORDB`, `QUOB`, `EXEB` or `TRDB`:
an order list, a mass order, a cross, a mass quote, a match report) adds one
message per entry, filed under its item (`ORDR`, `QUOT`, `EXEC`, `TRAD`) - a
mass quote's entry one quote holding both its legs - chained by the order the
entry names and split again as its category is. Each split message names its
source in `srcuuids`. A quote is never split: a bid and an offer are the two
legs of one message, stored under side `0`. An acknowledgement of an execution
(`BN`, `Q`) states no fact of its order, so it answers no market leaf.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix } = require('yggdryl')

const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

const fill = '8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|'
const [report, execution] = codec.parseLine(Buffer.from(fill))
assert.deepEqual([report.marketdatakind, report.state], ['ORDR', 'PARTIALLY_FILLED'])
assert.deepEqual([execution.marketdatakind, execution.state], ['EXEC', 'FILLED'])
assert.ok(execution.srcuuids.includes(report.curruuid))
// An order or an execution message stores its cross code under its side; the fill is a chain of its own.
assert.deepEqual([report.crosscode, execution.crosscode], ['10:1:O-9', '8:1:E-1'])

const stated = '8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|15=USD|132=99|134=7|133=101|135=8|10=0|'
const [quote] = codec.parseLine(Buffer.from(stated))
assert.deepEqual([quote.marketdatakind, quote.side, quote.crosscode], ['QUOT', 'BOTH', '14:0:Q1'])
// Both legs on the one message, each in its currency, tagged BOTH; neither is the quote's own price.
assert.equal(quote.price, null)
assert.deepEqual([quote.bidpx, quote.askpx, quote.askqty, quote.askccy], ['99', '101', '8', 'USD'])

// An acknowledgement of an execution states no fact of its order: no market leaf.
const ack = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=BN|52=20260921-10:00:01|17=E-1|37=O-9|1036=2|10=0|'))
assert.deepEqual(ack.marketData(), [])
```

## Turn FIX into market data and books

`marketData` admits orders, quotes, executions and `W`/`X` book messages - a
trade as the executions its parse split off - reads each as its one graph leaf
(a book message one per entry) and sorts them by the instant a book folds them
at; `graph.BookIterator` then walks them, pruning the executions.
`bookArrowReader(messages, snapshotMillis, filter)` folds the same messages
into book rows, one book per book key. Compose `lifecycle` in front when
predecessor state matters. `marketArrowReader` writes the sorted leaves as
`marketdata` rows, and `marketDataArrowReader` is its twin over batches of FIX
rows already in Arrow.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix, graph } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const codec = new fix.FixCodec(registry)
// The update arrives before the snapshot it follows.
const lines = [
  '8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|',
  '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
]
const capture = [...codec.parseLines(lines)]
assert.ok(capture.every((message) => message.marketdatakind === 'BOOK'))

const leaves = [...codec.marketData(codec.lifecycle(capture))]
assert.equal(leaves.length, 4, 'one leaf per entry')
assert.equal(leaves[leaves.length - 1].kind, 'execution_event')
const books = [...new graph.BookIterator(leaves)]
assert.equal(books.length, 2)
assert.equal(books[1].bestPrice('BUYS'), '101')

// The book door does not sort: the same capture out of order is no error - the
// snapshot dated before the book it would fold into is left out, with a warning.
assert.equal(codec.bookArrowReader(capture).intoTable().numRows, 1)
// The sorted leaves as `marketdata` rows.
assert.equal(codec.marketArrowReader(capture).intoTable().numRows, 4)
// The same leaves off the capture's FIX rows.
const fixed = codec.arrowReader(fix.schema(registry, 'fix'), capture)
assert.equal(codec.marketDataArrowReader(fixed).intoTable().numRows, 4)
```

## Build and commit a dictionary

Build fields, components, groups and code sets in memory - a set before the
field naming it - then `commit` writes the shard tree and `fromHandle` reads it
back whole.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Field, fields, fix } = require('yggdryl')

// The counter is a field of the dictionary; no component or message lists it
// beside the group, whose length is its count.
const count = Field.from('NoPartyIDs: int32')
count.fix.tag = 453
const partyId = Field.from('PartyID: utf8')
partyId.fix.tag = 448
const registry = fix.FixRegistry.fromFields([count, partyId])

// A Struct files as a component, a Serie of one as a group.
const member = registry.field(448)
member.fix.fieldRef = 'PartyID'
const party = fields.struct('Party', [member], { nullable: false })
registry.insert(party)
const parties = fields.serie('Parties', party)
parties.fix.counter = 453
parties.fix.component = 'Party'
registry.insert(parties)

// The vocabulary first, then the field that reads by it.
registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }, { value: '2', name: 'Sell' }])
const side = Field.from('Side: utf8')
side.fix.tag = 54
side.fix.codeset = 'sidecodeset'
registry.insert(side)

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-')), 'catalog')
const report = registry.commit(root)
assert.ok(report.written.length > 0)
assert.deepEqual(report.removed, [])
assert.ok(fs.existsSync(path.join(root, 'codesets', 'sidecodeset.json')))
const reloaded = fix.FixRegistry.fromHandle(root)
assert.ok(reloaded.equals(registry))
assert.equal(reloaded.fieldByPath('Parties.PartyID').fix.tag, 448)
fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

## Fold a venue CBlock into a dictionary

`FixRegistry.fromCfbFile` reads one Ullink CBlock (`.cfb`) into a registry and
its declared roots, stamping the dialect on everything it produced. The folds
into a held registry (`add_cfb_file`, `add_cfb_files`, `merge_with`) are not
bound here: fold CBlocks - files, folders of them or globs - with `yggdryl fix
ingest`, a whole dictionary folder with `yggdryl fix sync` ([cli](cli.md)), or
in Rust or Python.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { fix } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const file = path.join(folder, 'alpha.cfb')
fs.writeFileSync(file, `<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4" type="com.ullink.BuySideFIXCPluginCBlock">
  <vocabulary><vocabulary-tag name="20001" alt="VenueFlag" type="string" /></vocabulary>
</cplugin-configuration>
`)
const [venue, roots] = fix.FixRegistry.fromCfbFile(file, 'venue')
assert.deepEqual(venue.field(20001).fix.sources, ['venue'])
assert.deepEqual(venue.dialects(), ['venue'])
// The catalog records the source once: its file and its plugin's role.
assert.deepEqual(venue.sources(), [{ id: 'venue', file: 'alpha.cfb', pluginside: 'BUYS' }])
assert.deepEqual(roots, [])
// No dialect, no stamp: only the folds read a file's stem.
const [bare] = fix.FixRegistry.fromCfbFile(file)
assert.deepEqual(bare.field(20001).fix.sources, [])
assert.deepEqual(bare.sources(), [])
fs.rmSync(folder, { recursive: true, force: true })
```

## Gotchas in JavaScript

- `parseLine`, `parseFixLine` and friends take a `Buffer`; `parseLines` also
  accepts strings. `Buffer.from(text)` keeps SOH and every other byte below
  `0x80` as it is. Use `Buffer.from(text, 'binary')` only when the string holds
  raw bytes `0x80`-`0xFF`, one char per byte; never on real Unicode text, where
  it truncates every character above U+00FF.
- Decimals come back as exact text (`'100'`, `'10.5'`), instants and 64-bit
  hashes as `bigint`: compare with `1_767_348_930_000_000_000n`, never a number.
- `side`, `state` and `marketdatakind` answer the member's stored name (`'BUYS'`,
  `'FILLED'`, `'ORDR'`); an Arrow column stores its code (`Side.BUYS`,
  `MarketDataKind.ORDR`).
- `FixMessages` is a one-shot iterable: spread it once (`[...codec.parseLines(x)]`);
  it throws only for a source failure, where the iteration reaches it. What a
  line states that cannot be read is defaulted or left out, and the addon
  writes a warning to standard error as the core's terminal line
  (`... ! WARNING  [main] yggdryl.fix.build build:<line> › ...`), once per
  kind, unless a handler on `logging.getLogger('yggdryl')` takes them.
- Arrow JS interop is copied IPC with bounded cursors, never zero copy; keep
  bulk work inside `parseTextArrowReader` / `arrowReader` / `writeArrowReader`
  and cross into Arrow JS once at the end (`intoTable()`).
- `handle.readTextLines()` reads under the handle's own text options, row
  header included, and takes other options or a property bag like every
  record read; `options.captureNames` is what `{ captureNames }` wants.
- `snapshotNs` is a `bigint`; `null`, zero or negative disables the grid.
- `sortedLifecycle` is a `boolean`, `false` unless the source is in instant
  order; `withSortedLifecycle(sorted)` answers a copy of the codec.
