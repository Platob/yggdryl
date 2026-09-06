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

const { Field, Scalar, fields, fix } = require('yggdryl')

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
  const all = [...fix.FixRegistry.fromHandle(SEED)]
  for (let offset = 0; offset < VENDOR_FIELDS; offset += 1) {
    const field = Field.from(`Venue${offset}: utf8`)
    field.fix.id = `${5000 + offset}:${VENDOR_BRANCH}`
    field.fix.aliases = [`VenueAlias${offset}`]
    all.push(field)
  }
  return fix.FixRegistry.fromFields(all)
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
  ],
  { nullable: false },
)
const message = new fix.FixMsg(
  order,
  {
    Symbol: 'AAPL',
    OrderQty: Scalar.decimal(100n),
    NoPartyIDs: [{ PartyID: 'BROKER', PartyIDSource: 'D', PartyRole: 1 }],
  },
  registry,
)

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
    registry.fieldByPath('NoPartyIDs.PartyID', fix.STANDARD_BRANCH),
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
  benchmark('fix/message_get_by_path', () => message.getByPath('NoPartyIDs.0.PartyID'))
  benchmark('fix/message_branch', () => message.branch)
  benchmark('fix/infer_fixml_protocol', () => registry.inferBytesProtocol(FIXML_LINE))
  benchmark('fix/infer_ullink_msgtype', () => registry.inferTextMsgtype(ULLINK_LINE))
  benchmarkLoad('fix/from_handle_seed', () => fix.FixRegistry.fromHandle(SEED))
  benchmarkLoad(`fix/from_handle_${WIDE_FIELDS}_fields`, () =>
    fix.FixRegistry.fromHandle(generated),
  )
} finally {
  fs.rmSync(workspace, { recursive: true, force: true })
}
