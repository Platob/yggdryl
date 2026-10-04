'use strict'

// `node/src/isin_registry.rs`: the instrument registry, one row per ISIN of
// every fact it is known by, learned from and filled into FIX messages,
// bound to the store it is loaded from and committed back to, redirected to
// the core.

const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')
const { BatchReader, IOBase, IOResult, IsinRegistry, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', 'config', 'fix')
const HOLCIM = 'CH0012214059'
const APPLE = 'US0378331005'
const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(SEED))

/** A message stating Holcim's ISIN, its RIC, its CFI code, its ticker, its market and its currency. */
function stated(reader = codec) {
  return reader.parseFixLine(Buffer.from(
    `8=FIX.4.4|35=D|11=A|22=4|48=${HOLCIM}|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|`,
  ))
}

/** The columns a row states. */
function statedColumns(row) {
  return Object.fromEntries(Object.entries(row).filter(([key, value]) => value !== null && key !== 'updunix'))
}

test('a registry learns a message and fills a later one named by its ticker', () => {
  const registry = new IsinRegistry()
  assert.equal(registry.length, 0)
  assert.equal(registry.isDirty, false)
  assert.ok(registry.learn(stated()))
  assert.equal(registry.isDirty, true)
  const row = registry.get(HOLCIM)
  assert.deepEqual(statedColumns(row), {
    cficode: 'ESVUFR', currency: 'CHF', isin: HOLCIM, miccode: 'XSWX', ric: 'HOLN.S', ticker: 'HOLN',
  })
  assert.equal(row.valor, null, 'a code the message only derived is never learned')
  assert.equal(row.countrycode, null, 'the prefix already says the country')
  assert.deepEqual(registry.getByTicker('HOLN', 'XSWX'), row)
  assert.equal(registry.getByTicker('HOLN', 'XLON'), null)
  const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|'))
  assert.equal(later.isincode, null)
  assert.ok(registry.fill(later))
  assert.equal(later.isincode, HOLCIM)
  assert.ok(later.securityids.isDerived('isin'))
  assert.equal(later.securityids.get('ric'), 'HOLN.S')
  assert.equal(later.ticker, 'HOLN')
  assert.equal(later.currency, 'CHF')
  assert.ok(!registry.fill(later), 'nothing left to fill')
  const wire = later.intoText('|')
  assert.ok(!wire.includes('461=') && !wire.includes('15=') && !wire.includes('48='), 'never the wire')
  // A RIC is a listing code, never a key.
  const byRic = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|'))
  assert.ok(!registry.fill(byRic))
  assert.equal(byRic.isincode, null)
  assert.ok(new IsinRegistry().enrich(stated()))
})

test('a row merges by the update rule', () => {
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.S', updunix: 10n }))
  assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.S' }), 'a row stating nothing new moves nothing')
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 5n }), 'a valid value replaces whatever the time')
  assert.equal(registry.get(HOLCIM).ric, 'HOLN.VX')
  assert.ok(!registry.merge({ isin: HOLCIM, cusip: '037833101' }), 'a typo under a checked code moves nothing')
  assert.ok(registry.merge({ isin: HOLCIM, cusip: '037833100', countrycode: 'LI', currency: 'CHF', forexcode: 'EUR/CHF' }))
  const row = registry.get(HOLCIM)
  assert.deepEqual([row.cusip, row.countrycode, row.currency, row.forexcode], ['037833100', 'LI', 'CHF', 'EUR/CHF'])
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
  assert.equal(bounded.toString(), 'IsinRegistry(len=1, maxInstruments=1, dirty=true)')
})

test('a golden table loads by any spelling of its columns', () => {
  const golden = new arrow.Table({
    ISIN: arrow.vectorFromArray([HOLCIM, APPLE], new arrow.Utf8()),
    RIC: arrow.vectorFromArray(['HOLN.S', 'AAPL.OQ'], new arrow.Utf8()),
    BloombergSymbol: arrow.vectorFromArray(['HOLN SW Equity', 'AAPL US Equity'], new arrow.Utf8()),
    MIC: arrow.vectorFromArray(['XSWX', 'XNAS'], new arrow.Utf8()),
    Country: arrow.vectorFromArray(['LI', null], new arrow.Utf8()),
    Currency: arrow.vectorFromArray(['CHF', 'USD'], new arrow.Utf8()),
  })
  const registry = IsinRegistry.fromArrowReader(BatchReader.from(golden))
  assert.equal(registry.length, 2)
  assert.equal(registry.isDirty, false, 'a load leaves a registry clean')
  const row = registry.get(APPLE)
  assert.deepEqual([row.isin, row.bloomberg, row.miccode, row.currency], [APPLE, 'AAPL US Equity', 'XNAS', 'USD'])
  assert.equal(registry.get(HOLCIM).countrycode, 'LI')
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
  const loaded = IsinRegistry.fromUrl(target)
  assert.equal(loaded.length, 2, 'the stream is a snapshot a later write does not move')
  assert.equal(loaded.get(HOLCIM).cficode, 'ESVUFR')
  assert.equal(loaded.extendFromHandle(new IOBase(target)), 2)
  assert.equal(IsinRegistry.fromUrl(new IOBase(target)).length, 2, 'a handle names the store too')
  assert.equal(IsinRegistry.fromUrl(path.join(root, 'missing.arrow')).length, 0, 'a missing store is empty')
  assert.throws(() => IsinRegistry.fromUrl(new IOBase(target), undefined, { media_type: 'x' }), /properties/)
})

test('a registry commits to its store only where it moved', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-isin-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const store = path.join(root, 'isin') + path.sep
  const registry = IsinRegistry.fromUrl(store, 8)
  assert.equal(registry.length, 0)
  assert.equal(registry.isDirty, false)
  assert.equal(registry.maxInstruments, 8)
  assert.ok(!fs.existsSync(path.join(root, 'isin')), 'nothing is laid out before a commit')
  assert.ok(registry.commit() instanceof IOResult)
  assert.equal(registry.commit().writtenRows, 0, 'a clean registry writes nothing')
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.S' }))
  assert.equal(registry.isDirty, true)
  assert.equal(registry.commit().writtenRows, 1)
  assert.equal(registry.isDirty, false)
  assert.ok(fs.existsSync(path.join(root, 'isin', 'part-0.arrows')))
  assert.equal(registry.commit().writtenRows, 0, 'a second commit writes nothing')
  const reloaded = IsinRegistry.fromUrl(store)
  assert.deepEqual(reloaded.get(HOLCIM), registry.get(HOLCIM))
  assert.equal(reloaded.isDirty, false)
  registry.clear()
  registry.commit()
  assert.equal(IsinRegistry.fromUrl(store).length, 0, 'emptied on the next commit')
  assert.throws(() => new IsinRegistry().commit(), /holder/)
})

test('a codec shares the caller\'s registry with every lifecycle and every parse', () => {
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
  // A parse through the sharing codec fills derived identifiers from the
  // table, learns nothing, and leaves the identity the bare parse gives -
  // the two read under one pinned clock, so an undated line dates alike.
  const pinned = new Date(Date.UTC(2026, 0, 2, 10, 15, 30))
  const line = Buffer.from('8=FIX.4.4|35=D|11=P|55=HOLN|207=XSWX|10=0|')
  const filled = new fix.FixCodec(codec.registry, { isinRegistry: registry, defaultSendingTime: pinned }).parseFixLine(line)
  const bare = new fix.FixCodec(codec.registry, { defaultSendingTime: pinned }).parseFixLine(line)
  assert.equal(filled.isincode, HOLCIM)
  assert.ok(filled.securityids.isDerived('isin'))
  assert.equal(filled.securityids.get('ric'), 'HOLN.S')
  assert.equal(bare.isincode, null)
  assert.equal(filled.curruuid, bare.curruuid)
  assert.equal(filled.currhashcode, bare.currhashcode)
  assert.equal(filled.intoText('|'), bare.intoText('|'))
  assert.equal(filled.cficode, null, 'a parse fills identifiers only')
  assert.ok(!registry.learn(filled), 'nothing a parse derived is learned back')
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

test('the process registry is the store the environment names, shared with the codec the environment names', (t) => {
  // Process-wide state, so each case is driven in a process of its own whose
  // environment names a scratch store, never the real home.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-isin-env-'))
  t.after(() => fs.rmSync(home, { recursive: true, force: true }))
  const store = path.join(home, 'instruments') + path.sep
  const inProcess = (body) => {
    const script = `
      const assert = require('node:assert/strict')
      const fs = require('node:fs')
      const path = require('node:path')
      const { IsinRegistry, fix } = require(process.argv[1])
      const store = process.argv[2]
      ${body}
      console.log('ok')
    `
    const output = execFileSync(
      process.execPath,
      ['-e', script, require.resolve('yggdryl'), store],
      { encoding: 'utf8', env: { ...process.env, HOME: home, USERPROFILE: home, YGGDRYL_ISIN_REGISTRY_URI: store } },
    )
    assert.equal(output.trim(), 'ok')
  }

  inProcess(`
    const registry = IsinRegistry.fromEnv()
    assert.equal(registry.length, 0, 'an empty first run')
    assert.ok(IsinRegistry.fromEnv().equals(registry), 'resolved once')
    assert.throws(() => IsinRegistry.installEnv(new IsinRegistry()), /already resolved/)
    assert.ok(registry.merge({ isin: '${HOLCIM}', ric: 'HOLN.S' }))
    assert.equal(registry.commit().writtenRows, 1)
    assert.ok(fs.existsSync(path.join(store, 'part-0.arrows')))
    assert.ok(fix.FixCodec.fromEnv().isinRegistry.equals(registry))
    const own = new IsinRegistry()
    assert.ok(fix.FixCodec.fromEnv({ isinRegistry: own }).isinRegistry.equals(own), 'a stated pin stands')
    assert.equal(new fix.FixCodec(fix.FixRegistry.fromEnv()).isinRegistry, null, 'a codec built by hand attaches none')
  `)
  // Installed first, the caller's own table is the one every later
  // `fromEnv` answers.
  inProcess(`
    const own = new IsinRegistry(4)
    IsinRegistry.installEnv(own)
    assert.ok(IsinRegistry.fromEnv().equals(own))
    assert.throws(() => IsinRegistry.installEnv(new IsinRegistry()), /already resolved/)
  `)
})
