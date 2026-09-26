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
fix.installGlobalRegistry(registry)
assert.ok(fix.globalRegistry().equals(registry))
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
field.fix.branches = ['Venue', 'desk']

assert.equal(field.fix.tag, 38)
assert.equal(field.get('FIX:names'), '["Qty","Quantity"]')
assert.deepEqual(field.fix.branches, ['desk', 'venue'])
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
assert.equal(message.byTag(453).asJs(), 1)
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
const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|10=0|'))

assert.deepEqual([message.header().beginstring, message.header().msgtype], ['FIX.4.4', 'D'])
// A coded value reads as its name; the wire keeps its code.
assert.equal(message.byTag(54).asJs(), 'BUY')
assert.equal(message.side, 'BUY')
assert.equal(message.quantity, '100')
assert.equal(message.byName('symbol').asJs(), 'AAPL')
// The first stated OrderID, ClOrdID, ... names the order's chain.
assert.equal(message.crosscode, 'A1')
assert.deepEqual(message.altids, { CLORDID: 'A1' })
// Instants are bigint nanoseconds since the epoch, UTC.
assert.equal(message.currunix, 1_767_348_930_000_000_000n)
// The entries are the content row as a tree of { tag, name, value, entries }.
assert.deepEqual(message.entries().map((entry) => entry.name), ['symbol', 'side', 'timeinforce'])
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
assert.equal(message.crosscode, 'A1')

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
capture's own columns leading; parsing is pooled across `threads`.

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
// The schema is decided before a row is read: the capture leads, `fixentries` closes.
assert.equal(read.field.fieldAt(0).name, 'url')
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

// Line by line: pass the options again, and tell the codec what the captures are called.
const lines = [...handle.readTextLines(options)]
assert.equal(lines.length, 3)
const codec = new fix.FixCodec(registry, { captureNames: ['mtime', 'level'] })
const messages = [...codec.parseTextLines(lines)]
assert.deepEqual(messages.map((message) => message.recdunix), [1_767_348_930_250_000_000n, 1_767_348_930_500_000_000n])

fs.rmSync(directory, { recursive: true, force: true })
```

## Land messages in the fixed row and back

`fix.schema` is the one row every message answers as; `arrowReader` and
`messages` cross between messages and batches, any record medium stores the
batches, and `writeArrowReader` writes them back as wire lines to any
`{ write(chunk) }` sink.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, fix } = require('yggdryl')

const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
const codec = new fix.FixCodec(registry, { separator: 124 })
const schema = fix.schema(registry, 'fix')
const lines = ['8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|', '8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|']
const parsed = [...codec.parseLines(lines)]

// One message as one row, and back; a column is found by name.
const row = parsed[0].intoRow(schema)
assert.equal(row.toJSON()[schema.indexOf('msgtype')], 'D')
assert.ok(fix.FixMsg.fromRow(schema, row, registry).intoRow(schema).equals(row))

// A stream of messages as batches, landed in Parquet without a per-row detour.
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const stored = new IOBase(path.join(directory, 'capture.parquet'))
stored.overwriteArrowReader(codec.arrowReader(schema, parsed))
const again = [...codec.messages(stored.readArrowReader())]
assert.deepEqual(again.map((message) => message.currhashcode), parsed.map((message) => message.currhashcode))

// And out to the wire, one line per row.
const chunks = []
assert.equal(codec.writeArrowReader(stored.readArrowReader(), { write: (chunk) => chunks.push(Buffer.from(chunk)) }), 2)
assert.ok(Buffer.concat(chunks).toString().startsWith('8=FIX.4.4|35=D|11=ORDER-1|9999=x|'))
fs.rmSync(directory, { recursive: true, force: true })
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order under one `crossuuid`.

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
assert.ok(parsed.every((held) => held.seqnum === 0 && held.prevuuid === null))

const [order, ack, fill] = codec.lifecycle(parsed)
// Sorted by event time, joined by the identifiers each message went by.
assert.deepEqual([order.seqnum, ack.seqnum, fill.seqnum], [0, 1, 2])
assert.equal(ack.prevuuid, order.curruuid)
assert.equal(fill.prevuuid, ack.curruuid)
assert.ok([ack, fill].every((held) => held.crossuuid === order.crossuuid))
assert.equal(fill.state, '80FILLED')

// Rows already in Arrow chain in place, under the schema they were read with.
const rows = codec.arrowReader(fix.schema(registry), codec.parseLines(lines))
const chained = codec.lifecycleArrowReader(rows).intoTable()
assert.equal(new Set([...chained.getChild('crossuuid')].map(String)).size, 1)
```

## Turn FIX into market operations and books

`marketOperations` admits what a book folds, expands each message into graph
leaves and sorts them by the instant a book folds them at; `graph.BookIterator`
then walks them. Compose `lifecycle` in front when predecessor state matters.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix, graph } = require('yggdryl')

const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
// The update arrives before the snapshot it follows.
const lines = [
  '8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|',
  '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
]
const capture = [...codec.parseLines(lines)]

const operations = [...codec.marketOperations(codec.lifecycle(capture))]
assert.equal(operations[operations.length - 1].kind, 'execution_event')
const books = [...new graph.BookIterator(operations)]
assert.equal(books.length, 2)
assert.equal(books[1].bid.bestPrice, '101')

// The book door is strict: the same capture out of order is refused.
assert.throws(() => codec.bookArrowReader(capture).intoTable(), /nondecreasing/)
// The sorted operations as `marketdata` rows.
assert.equal(codec.marketArrowReader(capture).intoTable().numRows, 4)
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

## Gotchas in JavaScript

- `parseLine`, `parseFixLine` and friends take a `Buffer`; `parseLines` also
  accepts strings. A `Buffer` built from a string with control bytes needs the
  `'binary'` encoding to keep them.
- Decimals come back as exact text (`'100'`, `'10.5'`), instants and 64-bit
  hashes as `bigint`: compare with `1_767_348_930_000_000_000n`, never a number.
- `FixMessages` is a one-shot iterable: spread it once (`[...codec.parseLines(x)]`);
  a refused line throws where the iteration reaches it.
- Arrow JS interop is copied IPC with bounded cursors, never zero copy; keep
  bulk work inside `parseTextArrowReader` / `arrowReader` / `writeArrowReader`
  and cross into Arrow JS once at the end (`intoTable()`).
- `TextOptions` has no `captureNames` getter here: pass the capture names you
  wrote in the row header to `{ captureNames }`, or read them off
  `options.sourceField()` - the children after `body`.
- `handle.readTextLines()` with no argument reads under default text options
  even on an `intoText(options)` handle - no row header, no captures; pass the
  options (`readTextLines(options)`). `readArrowReader()` does apply them.
- `snapshotNs` is a `bigint`; `null`, zero or negative disables the grid.
