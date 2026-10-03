'use strict'

// `node/src/isin_registry.rs`: the instrument registry, one row per ISIN of
// its equivalents, learned from and filled into FIX messages and read from
// and written to any holder, redirected to the core.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')
const { BatchReader, IOBase, IsinRegistry, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', 'config', 'fix')
const HOLCIM = 'CH0012214059'
const APPLE = 'US0378331005'
const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(SEED))

/** A message stating Holcim's ISIN, its RIC, its CFI code and its ticker. */
function stated(reader = codec) {
  return reader.parseFixLine(Buffer.from(
    `8=FIX.4.4|35=D|11=A|22=4|48=${HOLCIM}|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|10=0|`,
  ))
}

/** The columns a row states. */
function statedColumns(row) {
  return Object.fromEntries(Object.entries(row).filter(([key, value]) => value !== null && key !== 'updunix'))
}

test('a registry learns a message and fills a later one named by its RIC', () => {
  const registry = new IsinRegistry()
  assert.equal(registry.length, 0)
  assert.ok(registry.learn(stated()))
  const row = registry.get(HOLCIM)
  assert.deepEqual(statedColumns(row), { cficode: 'ESVUFR', isin: HOLCIM, ric: 'HOLN.S', ticker: 'HOLN' })
  assert.equal(row.valor, null, 'a code the message only derived is never learned')
  assert.deepEqual(registry.getByRic('HOLN.S'), row)
  assert.equal(registry.getByRic('XXXX.S'), null)
  const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=B|22=5|48=HOLN.S|10=0|'))
  assert.equal(later.isincode, null)
  assert.ok(registry.fill(later))
  assert.equal(later.isincode, HOLCIM)
  assert.ok(later.securityids.isDerived('isin'))
  assert.equal(later.ticker, 'HOLN')
  assert.ok(!registry.fill(later), 'nothing left to fill')
  assert.ok(!later.intoText('|').includes('461='), 'never the wire')
  assert.ok(new IsinRegistry().enrich(stated()))
})

test('a row merges by the update rule', () => {
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.S', updunix: 10n }))
  assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.S' }), 'a row stating nothing new moves nothing')
  assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 5n }), 'an older statement only fills')
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 20n }), 'a newer one replaces')
  assert.notEqual(registry.getByRic('HOLN.VX'), null)
  assert.equal(registry.getByRic('HOLN.S'), null)
  assert.throws(() => registry.merge({ ric: 'HOLN.S' }), /isin/)
  assert.equal(registry.remove(HOLCIM).isin, HOLCIM)
  assert.equal(registry.remove(HOLCIM), null)
  registry.merge({ isin: HOLCIM })
  registry.clear()
  assert.equal(registry.length, 0)
  const bounded = new IsinRegistry(1)
  assert.equal(bounded.maxInstruments, 1)
  assert.ok(bounded.merge({ isin: HOLCIM }))
  assert.throws(() => bounded.merge({ isin: APPLE }), /1/)
  assert.equal(bounded.toString(), 'IsinRegistry(len=1, maxInstruments=1)')
})

test('a golden table loads by any spelling of its columns', () => {
  const golden = new arrow.Table({
    ISIN: arrow.vectorFromArray([HOLCIM, APPLE], new arrow.Utf8()),
    RIC: arrow.vectorFromArray(['HOLN.S', 'AAPL.OQ'], new arrow.Utf8()),
    BloombergSymbol: arrow.vectorFromArray(['HOLN SW Equity', 'AAPL US Equity'], new arrow.Utf8()),
    MIC: arrow.vectorFromArray(['XSWX', 'XNAS'], new arrow.Utf8()),
  })
  const registry = IsinRegistry.fromArrowReader(BatchReader.from(golden))
  assert.equal(registry.length, 2)
  const row = registry.getByRic('AAPL.OQ')
  assert.deepEqual([row.isin, row.bloomberg, row.miccode], [APPLE, 'AAPL US Equity', 'XNAS'])
  assert.equal(registry.extendFromArrowReader(BatchReader.from(golden)), 2, 'a known row folds again')
  assert.throws(() => IsinRegistry.fromArrowReader(BatchReader.from(new arrow.Table({
    RIC: arrow.vectorFromArray(['HOLN.S'], new arrow.Utf8()),
  }))), /isin/)
})

test('a registry round trips through a holder, its stream a snapshot', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-isin-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const registry = new IsinRegistry()
  registry.merge({ isin: HOLCIM, ric: 'HOLN.S', cficode: 'ESVUFR', updunix: 7n })
  registry.merge({ isin: APPLE, ric: 'AAPL.OQ' })
  const snapshot = registry.intoArrowReader()
  registry.clear()
  const target = path.join(root, 'instruments.arrow')
  new IOBase(target).overwriteArrowReader(snapshot)
  const loaded = IsinRegistry.fromHandle(target)
  assert.equal(loaded.length, 2, 'the stream is a snapshot a later write does not move')
  assert.equal(loaded.getByRic('HOLN.S').cficode, 'ESVUFR')
  assert.equal(loaded.extendFromHandle(new IOBase(target)), 2)
  assert.equal(IsinRegistry.fromHandle(path.join(root, 'missing.arrow')).length, 0, 'a missing store is empty')
})

test('a codec shares the caller\'s registry with every lifecycle', () => {
  const registry = new IsinRegistry()
  const shared = new fix.FixCodec(codec.registry, { isinRegistry: registry })
  assert.ok(shared.isinRegistry.equals(registry))
  assert.equal(codec.isinRegistry, null)
  assert.ok(!registry.equals(new IsinRegistry()), 'equal only to itself')
  for (const _ of shared.lifecycle([stated(shared)])) void _
  assert.equal(registry.length, 1)
  assert.notEqual(registry.get(HOLCIM), null)
  for (const _ of codec.lifecycle([stated()])) void _
  assert.equal(registry.length, 1, 'a codec without one learns into its own')
})

test('a ticker leads back to its ISIN on the same market', () => {
  // Through the inverse index, gated by the market: the one row listing the
  // ticker whose market is the one asked, or whose market or the one asked
  // is unstated; two rows answering is ambiguous, and answers none.
  const novartis = 'CH0012005267'
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.S', ticker: 'HOLN', miccode: 'XSWX' }))
  const isin = (ticker, market) => {
    const row = registry.getByTicker(ticker, market)
    return row === null ? null : String(row.isin)
  }

  assert.equal(isin('HOLN'), HOLCIM)
  assert.equal(isin('HOLN', 'XSWX'), HOLCIM)
  assert.equal(isin('HOLN', null), HOLCIM)
  assert.equal(isin('HOLN', 'XXXX'), HOLCIM, 'XXXX states no market')
  assert.equal(isin('HOLN', 'XLON'), null)
  assert.equal(isin('ABBN'), null)
  assert.throws(() => registry.getByTicker('HOLN', 'TOOLONG'))
  // Two rows listing one ticker on two markets: a market resolves to its
  // listing, none resolves to neither.
  assert.ok(registry.merge({ isin: novartis, ticker: 'HOLN', miccode: 'XLON' }))
  assert.deepEqual([isin('HOLN', 'XLON'), isin('HOLN', 'XSWX')], [novartis, HOLCIM])
  assert.equal(isin('HOLN'), null, 'ambiguous')
  assert.notEqual(registry.remove(novartis), null)
  assert.equal(isin('HOLN'), HOLCIM)
  registry.clear()
  assert.equal(isin('HOLN'), null)
})
