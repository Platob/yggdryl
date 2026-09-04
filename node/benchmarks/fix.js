'use strict'

// Boundary cost of the FIX dictionary, against the native numbers.
//
// Every case here is one crossing over a registry the core resolves: what is
// measured is the coercion of the key, the wrapper the answer is put in, and -
// for the two loads - the shard read the boundary only names. The generated
// registry is written to a temporary folder and removed on the way out, so the
// only tracked input is the seed dictionary at `config/fix`.

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')

const { Field, Scalar, fields, fix } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '100000', 10)
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
const WIDE_FIELDS = 1_000

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
const order = fields.struct(
  'NewOrderSingle',
  [registry.fieldByTag(55), registry.fieldByTag(38), registry.fieldByName('NoPartyIDs')],
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
  benchmark('fix/name_hit', () => registry.getFieldByName('Symbol'))
  benchmark('fix/folded_name_hit', () => registry.getFieldByName('symbol'))
  benchmark('fix/alias_hit', () => registry.getFieldByName('ticker'))
  benchmark('fix/tag_miss', () => registry.getFieldByTag(9999))
  benchmark('fix/name_miss', () => registry.getFieldByName('Nope'))
  benchmark('fix/generic_tag_hit', () => registry.getField(55))
  benchmark('fix/generic_name_hit', () => registry.getField('Symbol'))
  benchmark('fix/field_by_path_one_segment', () => registry.fieldByPath('NoPartyIDs'))
  benchmark('fix/field_by_path_two_segments', () =>
    registry.fieldByPath('NoPartyIDs.PartyID'),
  )
  benchmark('fix/message_get_by_tag', () => message.getByTag(55))
  benchmark('fix/message_get_by_name', () => message.getByName('ticker'))
  benchmark('fix/message_get_by_path', () => message.getByPath('NoPartyIDs.0.PartyID'))
  benchmarkLoad('fix/from_handle_seed', () => fix.FixRegistry.fromHandle(SEED))
  benchmarkLoad(`fix/from_handle_${WIDE_FIELDS}_fields`, () =>
    fix.FixRegistry.fromHandle(generated),
  )
} finally {
  fs.rmSync(workspace, { recursive: true, force: true })
}
