'use strict'

// Boundary cost of the FIX dictionary, against the native numbers.
//
// Every case here is one crossing over a registry the core resolves: what is
// measured is the coercion of the key - a tag, a branch, an identifier - the
// wrapper the answer is put in, and - for the two loads - the shard read the
// boundary only names. The generated registries are written to a temporary
// folder and removed on the way out, so the only tracked input is the seed
// dictionary at `config/fix`.

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')

const arrow = require('apache-arrow')

const { BatchReader, Field, MimeType, Scalar, fields, fix } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '5000', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 1_000); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

// A whole stream costs milliseconds, so it runs a fiftieth of the hit count.
function benchmarkStreams(name, rounds, operation) {
  for (let index = 0; index < Math.min(rounds, 5); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < rounds; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((rounds * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

// A load costs milliseconds, so it runs a hundredth of the hit count.
function benchmarkLoad(name, operation) {
  const rounds = Math.max(1, Math.round(iterations / 1_000))
  for (let index = 0; index < Math.min(rounds, 5); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < rounds; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((rounds * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const SEED = path.join(__dirname, '..', '..', 'config', 'fix')
const WIDE_FIELDS = 200
const VENDOR_BRANCH = 'cme'
const VENDOR_FIELDS = 200
const FIXML_LINE = Buffer.from(
  '8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|',
)
const ULLINK_LINE = 'ACCOUNT=A1|MSGTYPE=D|SYMBOL=AAPL'

const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-bench-fix-'))
const generated = path.join(workspace, 'generated')
{
  const wide = []
  for (let tag = 1; tag <= WIDE_FIELDS; tag += 1) {
    const field = Field.from(`Field${tag}: utf8`)
    field.fix.tag = tag
    field.fix.aliases = [`Alias${tag}`]
    wide.push(field)
  }
  fix.FixRegistry.fromFields(wide).writeInto(generated)
}

const registry = fix.FixRegistry.fromHandle(SEED)

// The seed beside a vendor dictionary, for the cross-branch rows.
const twoBranches = (() => {
  const all = []
  for (let offset = 0; offset < VENDOR_FIELDS; offset += 1) {
    const field = Field.from(`Venue${offset}: utf8`)
    field.fix.id = `${5000 + offset}:${VENDOR_BRANCH}`
    field.fix.aliases = [`VenueAlias${offset}`]
    all.push(field)
  }
  const vendor = fix.FixRegistry.fromFields(all).toJSON()
  const document = registry.toJSON()
  document.fields.push(...vendor.fields)
  document.branches.push(...vendor.branches.filter(branch => branch.name !== ''))
  return fix.FixRegistry.fromJson(JSON.stringify(document))
})()

// A field carrying a vendor identity, for the two identity-property rows.
const tagged = Field.from('TradeID: utf8')
tagged.fix.id = `5001:${VENDOR_BRANCH}`

const order = fields.struct(
  'NewOrderSingle',
  [
    registry.fieldByTag(55),
    registry.fieldByTag(38),
    registry.fieldByName('NoPartyIDs', fix.STANDARD_BRANCH),
    registry.definition('groups', 'Parties'),
  ],
  { nullable: false },
)
const message = new fix.FixMsg(
  order,
  {
    symbol: 'AAPL',
    orderqty: Scalar.float(100),
    nopartyids: 1,
    parties: [{ partyid: 'BROKER', partyidsource: 'D', partyrole: 1 }],
  },
  registry,
)

const catalog = new fix.FixRegistry()
const counter = Field.from('NoPartyIDs: int32')
counter.fix.tag = 453
const partyId = Field.from('PartyID: utf8')
partyId.fix.tag = 448
catalog.insert(counter)
catalog.insert(partyId)
partyId.fix.fieldRef = 'PartyID'
const party = fields.struct('Party', [partyId], { nullable: false })
catalog.createDefinition('components', party)
const parties = fields.list('Parties', party)
parties.fix.counter = 453
parties.fix.component = 'Party'
catalog.createDefinition('groups', parties)
const occurrence = catalog.definition('groups', 'Parties')
occurrence.fix.group = 'Parties'
counter.fix.fieldRef = 'NoPartyIDs'
const definition = fields.struct('NewOrderSingle', [counter, occurrence], { nullable: false })
definition.fix.msgtype = 'D'
catalog.createDefinition('messages', definition)
const snapshot = catalog.intoJson()
const singleton = catalog.msgtype('D')
const codec = new fix.FixCodec(catalog)
const ulregistry = new fix.FixRegistry()
ulregistry.withUlbridgeFields()
const ulcodec = new fix.FixCodec(ulregistry, { branch: 'ulbridge' })

function drain(values) {
  let count = 0
  for (const value of values) { void value; count += 1 }
  return count
}

// The stream and Arrow doors, over a capture of wide orders: what is measured
// is the crossing - one pull per line, one batch per pull - beside the parse.
const seedCodec = new fix.FixCodec(registry)
const fixedSchema = fix.schema(registry)
const encoder = new TextEncoder()
const LINES = Array.from({ length: 256 }, (_, index) =>
  encoder.encode(`8=FIX.4.4|35=D|11=ORDER-${String(index).padStart(6, '0')}|55=AAPL|54=1|38=100|58=${'x'.repeat(200)}|10=0|`),
)
const capture = new arrow.Table({ body: arrow.vectorFromArray(LINES, new arrow.Binary()) })
const parsed = seedCodec.parseLine(Buffer.from(LINES[0])).next().value
const parsedRow = parsed.intoRow(fixedSchema)
const parsedIpc = seedCodec.parseTextArrowReader(capture).intoIpc()
if (drain(seedCodec.parseLines(LINES)) !== LINES.length) throw new Error('line cardinality mismatch')
const sink = { write() {} }

function wildcard(size) {
  return Buffer.from(JSON.stringify({
    request: { mbean: 'com.ullink.ulbridge.sessioninterfaces.plugins:*', type: 'read' },
    value: Object.fromEntries(Array.from({ length: size }, (_, index) => [
      `com.ullink.ulbridge.sessioninterfaces.plugins:name=Item${index},plugin-type=FIX,type=Plugin`,
      { Name: `Item${index}`, CurrentPort: 9000 + index },
    ])),
    status: 200,
  }))
}

try {
  benchmark('fix/tag_hit', () => registry.getFieldByTag(55))
  benchmark('fix/alternate_tag_hit', () => registry.getFieldByTag(20))
  benchmark('fix/id_hit', () => registry.getFieldById('55:'))
  benchmark('fix/name_hit', () => registry.getFieldByName('Symbol', fix.STANDARD_BRANCH))
  benchmark('fix/folded_name_hit', () => registry.getFieldByName('symbol', fix.STANDARD_BRANCH))
  benchmark('fix/alias_hit', () => registry.getFieldByName('ticker', fix.STANDARD_BRANCH))
  benchmark('fix/tag_miss', () => registry.getFieldByTag(9999))
  benchmark('fix/name_miss', () => registry.getFieldByName('Nope', fix.STANDARD_BRANCH))
  benchmark('fix/id_miss', () => registry.getFieldById('5001:cme'))
  benchmark('fix/generic_tag_hit', () => registry.getField(55))
  benchmark('fix/generic_name_hit', () => registry.getField('Symbol'))
  benchmark('fix/field_by_path_one_segment', () =>
    registry.fieldByPath('NoPartyIDs', fix.STANDARD_BRANCH),
  )
  benchmark('fix/field_by_path_two_segments', () =>
    registry.fieldByPath('Parties.PartyID', fix.STANDARD_BRANCH),
  )
  benchmark('fix/vendor_id_hit_two_branches', () => twoBranches.getFieldById('5001:cme'))
  benchmark('fix/vendor_name_hit_two_branches', () =>
    twoBranches.getFieldByName('Venue1', VENDOR_BRANCH),
  )
  benchmark('fix/vendor_alias_hit_two_branches', () =>
    twoBranches.getFieldByName('venuealias1', VENDOR_BRANCH),
  )
  benchmark('fix/vendor_tag_hit_inferred', () => twoBranches.getFieldByTag(5001))
  benchmark('fix/standard_tag_hit_two_branches', () => twoBranches.getFieldByTag(55))
  // A removal that finds nothing: the coercion and the probe, with no field
  // wrapped and no dictionary changed, so the loop stays repeatable.
  benchmark('fix/remove_by_id_miss', () => twoBranches.removeById('9999:cme'))
  benchmark('fix/field_branch', () => tagged.fix.branch)
  benchmark('fix/field_id', () => tagged.fix.id)
  benchmark('fix/message_get_by_tag', () => message.getByTag(55))
  benchmark('fix/message_get_by_id', () => message.getById('55:'))
  benchmark('fix/message_get_by_name', () => message.getByName('ticker'))
  benchmark('fix/message_get_by_path', () => message.getByPath('Parties.0.PartyID'))
  benchmark('fix/message_branch', () => message.branch)
  benchmark('fix/message_into_latest', () => message.intoLatest())
  benchmark('fix/infer_fixml_protocol', () => MimeType.inferBytes(FIXML_LINE))
  benchmark('fix/infer_ullink_msgtype', () => fix.FixCodec.inferMsgtypeText(ULLINK_LINE))
  for (const category of ['fields', 'components', 'groups', 'messages']) {
    benchmark(`fix/${category}_first`, () => {
      const iterator = catalog.definitions(category)[Symbol.iterator]()
      const first = iterator.next().value
      iterator.return()
      return first
    })
    benchmark(`fix/${category}_drain`, () => drain(catalog.definitions(category)))
  }
  benchmark('fix/singleton_lookup', () => catalog.msgtype('D'))
  benchmark('fix/singleton_first', () => {
    const iterator = catalog.msgtypes()[Symbol.iterator]()
    const first = iterator.next().value
    iterator.return()
    return first
  })
  benchmark('fix/singleton_drain', () => drain(catalog.msgtypes()))
  benchmark('fix/singleton_field', () => singleton.asField())
  benchmark('fix/singleton_hash', () => singleton.stableHash())
  benchmark('fix/singleton_compare', () => singleton.compare(singleton))
  benchmark('fix/group_lookup', () => catalog.groupByCounter('453:'))
  benchmark('fix/catalog_hash', () => catalog.stableHash())
  benchmark('fix/catalog_snapshot_write', () => catalog.intoJson())
  benchmark('fix/catalog_snapshot_read', () => fix.FixRegistry.fromJson(snapshot))
  benchmark('fix/catalog_clone', () => catalog.clone())
  benchmark('fix/catalog_update', () => {
    const copy = catalog.clone()
    const changed = copy.definition('components', 'Party')
    changed.fix.description = 'Reviewed'
    return copy.updateDefinition('components', changed)
  })
  benchmark('fix/catalog_create_remove', () => {
    const copy = catalog.clone()
    const empty = fields.struct('NewMessage', [], { nullable: false })
    empty.fix.msgtype = 'New Message Code'
    copy.createDefinition('messages', empty)
    return copy.removeDefinition('messages', 'NewMessage')
  })
  benchmark('fix/numeric_group_parse', () => drain(codec.parseLine(Buffer.from('35=D|453=2|448=ONE|448=TWO|'))))
  benchmark('fix/message_into_row', () => message.intoRow(order))
  benchmark('fix/message_into_bytes', () => message.intoBytes(124))
  benchmark('fix/message_set', () => parsed.clone().set(55, 'MSFT'))
  benchmark('fix/message_remove', () => parsed.clone().remove(55))
  benchmark('fix/message_from_row', () => fix.FixMsg.fromRow(fixedSchema, parsedRow, registry))
  const streams = Math.max(1, Math.round(iterations / 50))
  benchmarkStreams(`fix/parse_lines_drain/${LINES.length}`, streams, () => drain(seedCodec.parseLines(LINES)))
  benchmarkStreams(`fix/parse_text_records_drain/${LINES.length}`, streams, () =>
    drain(seedCodec.parseTextRecords(LINES.map((body) => ({ body })))),
  )
  benchmarkStreams(`fix/parse_text_arrow_reader/${LINES.length}`, streams, () =>
    seedCodec.parseTextArrowReader(capture).intoTable().numRows,
  )
  benchmarkStreams(`fix/enrich_messages_arrow_reader/${LINES.length}`, streams, () =>
    seedCodec.enrichMessagesArrowReader(BatchReader.fromIpc(parsedIpc)).intoTable().numRows,
  )
  benchmarkStreams(`fix/messages_drain/${LINES.length}`, streams, () =>
    drain(seedCodec.messages(BatchReader.fromIpc(parsedIpc))),
  )
  benchmarkStreams(`fix/arrow_reader_over_parse_lines/${LINES.length}`, streams, () =>
    seedCodec.arrowReader(fixedSchema, seedCodec.parseLines(LINES)).intoTable().numRows,
  )
  benchmarkStreams(`fix/write_arrow_reader/${LINES.length}`, streams, () =>
    seedCodec.writeArrowReader(BatchReader.fromIpc(parsedIpc), sink),
  )
  for (const size of [1, 32, 64]) {
    const body = wildcard(size)
    if (drain(ulcodec.parseLine(body)) !== size) throw new Error('bulk cardinality mismatch')
    const selected = fix.UlPlugin.fromJsonBytes(body)[Symbol.iterator]().next().value
    benchmark(`fix/ulconfigs_first/${size}`, () => fix.UlPlugin.fromJsonBytes(body)[Symbol.iterator]().next().value)
    benchmark(`fix/ulconfigs_drain/${size}`, () => drain(fix.UlPlugin.fromJsonBytes(body)))
    benchmark(`fix/messages_first/${size}`, () => ulcodec.parseLine(body).next().value)
    benchmark(`fix/messages_drain/${size}`, () => drain(ulcodec.parseLine(body)))
    benchmark(`fix/records_drain/${size}`, () => drain(ulcodec.parseTextRecord({ url: 'capture.log', body })))
    benchmark(`fix/ulconfig_hash/${size}`, () => selected.stableHash())
  }
  benchmark('fix/register_ulbridge_fields', () => new fix.FixRegistry().withUlbridgeFields())
  benchmarkLoad('fix/from_handle_seed', () => fix.FixRegistry.fromHandle(SEED))
  benchmarkLoad(`fix/from_handle_${WIDE_FIELDS}_fields`, () =>
    fix.FixRegistry.fromHandle(generated),
  )
} finally {
  fs.rmSync(workspace, { recursive: true, force: true })
}
