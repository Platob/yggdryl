'use strict'

// `node/src/isin_registry.rs`: the instrument registry, one row per ISIN and
// market of every fact it is known by, learned from and filled into FIX messages,
// bound to the store it is loaded from and committed back to, redirected to
// the core.

const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')
const { BatchReader, IOBase, IOResult, Identifier, IsinRegistry, fix, graph } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', 'config', 'fix')
const HOLCIM = 'CH0012214059'
const APPLE = 'US0378331005'
const NOVARTIS = 'CH0012005267'
const DIAGEO = 'GB0002374006'
const SAP = 'DE0007164600'
const HSBC = 'GB0005405286'
const MICROSOFT = 'US5949181045'
const CSPX = 'IE00B5BMR087'
const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(SEED))

/** A message stating Holcim's ISIN, its RIC, its CFI code, its ticker, its market and its currency. */
function stated(reader = codec) {
  return reader.parseFixLine(Buffer.from(
    `8=FIX.4.4|35=D|11=A|22=4|48=${HOLCIM}|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|`,
  ))
}

/**
 * The nanoseconds a row's instant holds: a `Date` where the instant is a
 * whole millisecond, else the datetime `Scalar` that keeps the nanoseconds.
 */
function nanos(instant) {
  return instant instanceof Date ? BigInt(instant.getTime()) * 1_000_000n : instant.count
}

/** The columns a row states, its three instants aside. */
function statedColumns(row) {
  const stamps = ['updunix', 'firstunix', 'lastunix']
  return Object.fromEntries(Object.entries(row).filter(([key, value]) => value !== null && !stamps.includes(key)))
}

/**
 * An order event dated `1` stating `codes` - `[type, value]` pairs - and
 * the named `facts` beside them.
 */
function element(codes, facts = {}) {
  return new graph.OrderEvent(1, {
    securityids: codes.map(([type, value]) => new Identifier(type, value)),
    ...facts,
  })
}

test('a registry learns a message and fills a later one named by its ticker', () => {
  const registry = new IsinRegistry()
  assert.equal(registry.length, 0)
  assert.equal(registry.isDirty, false)
  assert.ok(registry.learn(stated()))
  assert.equal(registry.isDirty, true)
  const row = registry.get(HOLCIM)
  assert.deepEqual(statedColumns(row), {
    cficode: 'ESVUFR', currency: 'CHF', isin: HOLCIM, miccode: 'XSWX', ric: 'HOLN.S', ticker: 'HOLN', valor: '1221405',
  })
  // The message's derived valor is never learned; the row's valor is the
  // default the fold reads off the closed CH ISIN itself.
  assert.equal(row.valor, '1221405', 'the valor a CH ISIN embeds is the row default')
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
  // A RIC is a listing code and, with no ISIN, a lookup key.
  const byRic = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|'))
  assert.ok(registry.fill(byRic))
  assert.equal(byRic.isincode, HOLCIM)
  assert.ok(byRic.securityids.isDerived('isin'))
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
  assert.equal(row.underlyingisin, null)
  assert.ok(registry.merge({ isin: HOLCIM, underlyingisin: APPLE }), 'an instrument fact fills')
  assert.ok(!registry.merge({ isin: HOLCIM, underlyingisin: HOLCIM }), "the row's own ISIN states nothing")
  assert.ok(!registry.merge({ isin: HOLCIM, underlyingisin: 'US0378331006' }), 'a typo is dropped')
  assert.equal(registry.get(HOLCIM).underlyingisin, APPLE)
  assert.throws(() => registry.merge({ ric: 'HOLN.S' }), /isin/)
  // `remove` answers every listing of the ISIN, in MIC order.
  assert.deepEqual(registry.remove(HOLCIM).map((held) => held.isin), [HOLCIM])
  assert.deepEqual(registry.remove(HOLCIM), [])
  registry.merge({ isin: HOLCIM })
  registry.clear()
  assert.equal(registry.length, 0)
  const bounded = new IsinRegistry(1)
  assert.equal(bounded.maxInstruments, 1)
  assert.ok(bounded.merge({ isin: HOLCIM }))
  assert.throws(() => bounded.merge({ isin: APPLE }), /1/)
  assert.equal(bounded.toString(), 'IsinRegistry(len=1, maxInstruments=1, dirty=true)')
})

test('a row takes the defaults its ISIN and its market imply', () => {
  // The national number a closed ISIN embeds fills its empty column, and an
  // empty currency is the listing market's country's legal tender; a stated
  // value is never replaced by either.
  const diageo = 'GB0002374006'
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: diageo, miccode: 'XLON' }))
  const row = registry.get(diageo)
  assert.equal(row.sedol, '0237400', "a GB '00' ISIN embeds its SEDOL")
  assert.equal(row.currency, 'GBP', 'XLON is in GB')
  assert.ok(registry.merge({ isin: APPLE, cusip: '037833100', currency: 'EUR' }))
  assert.equal(registry.get(APPLE).currency, 'EUR', 'a stated currency stands')
  assert.equal(registry.get(APPLE).cusip, '037833100')
})

test('an ISIN holds one listing per market', () => {
  // Instrument facts are every listing's; listing facts belong to the
  // market a statement names, and a statement naming none lands them on no
  // listing of an ISIN listed on two.
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XSWX', ticker: 'HOLN', updunix: 10n }))
  assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XLON', ticker: '0QKY' }), 'a new market is a new listing')
  assert.equal(registry.length, 1, 'one instrument')
  assert.equal(registry.rows, 2, 'two listings')
  assert.deepEqual(registry.listings(HOLCIM).map((row) => [row.miccode, row.ticker, row.currency]), [
    ['XLON', '0QKY', 'GBP'],
    ['XSWX', 'HOLN', 'CHF'],
  ], 'in MIC order, each with its own market default')
  assert.deepEqual(registry.listings(APPLE), [])
  assert.deepEqual(registry.get(HOLCIM), registry.listings(HOLCIM)[0], 'get is the first listing')
  assert.equal(registry.getListing(HOLCIM, 'XSWX').ticker, 'HOLN')
  assert.equal(registry.getListing(HOLCIM, 'XNAS'), null)
  assert.throws(() => registry.getListing(HOLCIM, 'TOOLONG'))
  assert.equal(registry.getByTicker('0QKY', 'XLON').miccode, 'XLON')
  assert.equal(registry.getByTicker('HOLN').miccode, 'XSWX', 'the one listing of the ticker on any market')
  assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVUFR' }), 'an instrument fact')
  assert.deepEqual(registry.listings(HOLCIM).map((row) => row.cficode), ['ESVUFR', 'ESVUFR'], 'on every listing')
  assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.S' }), 'a listing fact naming no market on two listings')
  assert.deepEqual(registry.listings(HOLCIM).map((row) => row.ric), [null, null], 'lands on none')
  assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XSWX', ric: 'HOLN.S' }))
  assert.deepEqual(registry.listings(HOLCIM).map((row) => row.ric), [null, 'HOLN.S'], 'on the listing it names')
  const [london, zurich] = registry.listings(HOLCIM)
  assert.equal(nanos(london.updunix), nanos(zurich.updunix), 'updunix is an instrument fact')
  assert.equal(registry.removeListing(HOLCIM, 'XLON').miccode, 'XLON')
  assert.equal(registry.removeListing(HOLCIM, 'XLON'), null)
  assert.deepEqual([registry.length, registry.rows], [1, 1])
  assert.equal(registry.removeListing(HOLCIM, 'XSWX').ticker, 'HOLN')
  assert.deepEqual([registry.length, registry.rows], [0, 0], 'the instrument goes with its last listing')
  assert.equal(registry.get(HOLCIM), null)
})

test('a learn moves lastunix to the latest instant, firstunix to the earliest and updunix only where a fact moved', () => {
  const at = (stamp) => codec.parseFixLine(Buffer.from(
    `8=FIX.4.4|35=D|52=${stamp}|11=A|22=4|48=${HOLCIM}|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|`,
  ))
  const first = at('20260102-10:00:00')
  const later = at('20260102-11:00:00')
  const earlier = at('20260102-09:00:00')
  const registry = new IsinRegistry()
  assert.ok(registry.learn(first))
  const learned = registry.get(HOLCIM)
  assert.equal(nanos(learned.lastunix), first.transunix, 'lastunix is the instant of the event that stated the ISIN')
  assert.equal(nanos(learned.firstunix), first.transunix, 'the first learn sets firstunix too')
  assert.equal(nanos(learned.updunix), first.transunix)
  const clean = IsinRegistry.fromArrowReader(registry.intoArrowReader())
  assert.equal(clean.isDirty, false)
  assert.ok(clean.learn(later), 'meeting a known instrument later moves the registry')
  assert.equal(clean.isDirty, true)
  const met = clean.get(HOLCIM)
  assert.equal(nanos(met.lastunix), later.transunix)
  assert.equal(nanos(met.firstunix), first.transunix, 'a later instant leaves firstunix')
  assert.equal(nanos(met.updunix), first.transunix, 'no fact moved, so updunix stays')
  const replayed = IsinRegistry.fromArrowReader(clean.intoArrowReader())
  assert.ok(!replayed.learn(first), 'between the two instants nothing moves')
  assert.equal(replayed.isDirty, false)
  assert.ok(replayed.learn(earlier), 'an earlier instant moves firstunix back')
  assert.equal(replayed.isDirty, true)
  const back = replayed.get(HOLCIM)
  assert.equal(nanos(back.firstunix), earlier.transunix)
  assert.equal(nanos(back.lastunix), later.transunix, 'lastunix unmoved')
  assert.equal(nanos(back.updunix), first.transunix, 'updunix unmoved')
  // A stated one merges earlier-wins, on every listing.
  assert.ok(!replayed.merge({ isin: HOLCIM, firstunix: back.firstunix }))
  assert.ok(replayed.merge({ isin: HOLCIM, firstunix: 1n }))
  assert.equal(nanos(replayed.get(HOLCIM).firstunix), 1n)
})

test('the seed holds the common instruments, clean and bound to no store', () => {
  const seeded = IsinRegistry.seeded()
  assert.equal(seeded.length, 208)
  assert.equal(seeded.rows, 209, 'HSBC is listed on XHKG and XLON')
  assert.equal(seeded.isDirty, false)
  assert.equal(seeded.maxInstruments, 16384)
  assert.throws(() => seeded.commit(), /holder/, 'bound to no store')
  const apple = seeded.getByTicker('AAPL', 'XNAS')
  assert.equal(apple.isin, APPLE)
  assert.deepEqual(
    [apple.ticker, apple.miccode, apple.currency, apple.fisn, apple.cficode],
    ['AAPL', 'XNAS', 'USD', 'APPLE INC/SH SH', 'ESVUFR'],
  )
  assert.equal(apple.cusip, '037833100', 'the CUSIP its ISIN embeds')
  assert.equal(seeded.get('GB0002374006').sedol, '0237400')
  assert.equal(new IsinRegistry().length, 0, 'new holds none of it')
  assert.ok(seeded.merge({ isin: APPLE, ric: 'AAPL.OQ' }))
  assert.equal(seeded.isDirty, true)
  assert.equal(IsinRegistry.seeded().get(APPLE).ric, null, 'each seeded registry is its own')
  const field = IsinRegistry.field()
  // `firstunix` then `lastunix` after `updunix`, and `origccy` after
  // `currency`: forty-six columns.
  assert.equal(field.fieldLen, 46)
  assert.equal(field.indexOf('firstunix'), field.indexOf('updunix') + 1)
  assert.equal(field.indexOf('lastunix'), field.indexOf('updunix') + 2)
  assert.equal(field.indexOf('fisn'), field.indexOf('ticker') + 1)
  assert.equal(field.indexOf('origccy'), field.indexOf('currency') + 1)
  // The seed states an origin currency where the issue's is not the
  // trading one, and derives none.
  assert.equal(seeded.get(CSPX).origccy, 'USD')
  assert.equal(apple.origccy, null)
})

test('the origin currency is an instrument fact, stated and never derived', () => {
  const registry = new IsinRegistry()
  assert.ok(registry.merge({ isin: CSPX, miccode: 'XLON', currency: 'USD' }))
  assert.equal(registry.get(CSPX).origccy, null, "neither the prefix's EUR nor the listing's USD is derived")
  assert.ok(registry.merge({ isin: CSPX, miccode: 'XETR', currency: 'EUR', origccy: 'USD' }))
  assert.deepEqual(registry.listings(CSPX).map((row) => [row.miccode, row.currency, row.origccy]), [
    ['XETR', 'EUR', 'USD'],
    ['XLON', 'USD', 'USD'],
  ], 'every listing holds it')
  assert.ok(!registry.merge({ isin: CSPX, origccy: 'USD' }), 'restated, it moves nothing')

  // A EUR listing of the USD class, learned from a message stating only
  // its currency: the origin stays USD; a fill lands it where the message
  // holds none.
  const seeded = IsinRegistry.seeded()
  const sxr8 = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=A|22=4|48=${CSPX}|55=SXR8|207=XETR|15=EUR|10=0|`))
  assert.equal(sxr8.origccy, null)
  assert.equal(sxr8.originCurrency, 'EUR', 'the currency where none is held')
  assert.ok(seeded.learn(sxr8))
  assert.deepEqual(seeded.listings(CSPX).map((row) => row.origccy), seeded.listings(CSPX).map(() => 'USD'), 'never EUR')
  const filled = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=B|22=4|48=${CSPX}|207=XETR|10=0|`))
  assert.ok(seeded.fill(filled))
  assert.equal(filled.origccy, 'USD')
  assert.equal(filled.currency, 'EUR')
  assert.equal(filled.originCurrency, 'USD')
  // An instrument stating none fills none.
  const holcim = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=C|22=4|48=${HOLCIM}|207=XSWX|10=0|`))
  assert.ok(seeded.fill(holcim))
  assert.equal(holcim.origccy, null)
  assert.equal(holcim.originCurrency, 'CHF')
})

test('the cascade takes the ISIN, then a code, then the ticker on its market', () => {
  const registry = new IsinRegistry()
  registry.merge({ isin: APPLE, miccode: 'XNAS', ticker: 'AAPL' })
  registry.merge({ isin: DIAGEO, miccode: 'XLON', ticker: 'DGE' })
  registry.merge({ isin: SAP, miccode: 'XETR' })
  // The ISIN wins over a CUSIP naming Apple.
  const byIsin = registry.resolve(element([['isin', SAP], ['cusip', '037833100']]))
  assert.equal(byIsin.matched, true)
  assert.deepEqual(
    [byIsin.entry.isin, byIsin.tier, byIsin.kind, byIsin.derived, byIsin.listing, byIsin.unmatched],
    [SAP, 'isin', null, false, true, null],
  )
  // A CUSIP wins over a ticker naming Diageo; Apple is not listed on XLON.
  const byCode = registry.resolve(element([['cusip', '037833100']], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [byCode.entry.isin, byCode.tier, byCode.kind, byCode.derived, byCode.listing],
    [APPLE, 'code', 'cusip', true, false],
  )
  // The ticker on its market last.
  const byTicker = registry.resolve(element([], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [byTicker.entry.isin, byTicker.tier, byTicker.derived, byTicker.listing],
    [DIAGEO, 'symbology', true, true],
  )
  // The same through the public lookups.
  assert.equal(registry.getByCode('cusip', '037833100').isin, APPLE)
  assert.equal(registry.getByCode('sedol', '0237400', 'XLON').isin, DIAGEO, 'the SEDOL its ISIN embeds')
  assert.equal(registry.getByCode('cusip', 'not a cusip'), null)
  assert.equal(registry.getByCode('isoccy', 'USD'), null, 'a currency is no key')
  assert.throws(() => registry.getByCode('cusip', '037833100', 'TOOLONG'))
  // Nothing stated: no key; a ticker nothing lists: no candidate.
  const none = registry.resolve(element([]))
  assert.deepEqual([none.matched, none.entry, none.tier, none.unmatched], [false, null, null, 'NoKey'])
  assert.equal(registry.resolve(element([], { ticker: 'ZZZZ' })).unmatched, 'NoCandidate')
  // Any market value resolves, a FixMsg and a MarketData included.
  const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|55=DGE|207=XLON|10=0|'))
  assert.equal(registry.resolve(message).entry.isin, DIAGEO)
  assert.equal(registry.resolve(new graph.MarketData(element([['cusip', '037833100']]))).entry.isin, APPLE)
  assert.throws(() => registry.resolve({ isin: APPLE }), /expected MarketData, a market leaf or a FixMsg/)
  // The keys a lookup reads, in the order it reads them.
  const codes = IsinRegistry.lookupCodes()
  assert.equal(codes.length, 20)
  assert.deepEqual(codes.slice(0, 4), ['cusip', 'sedol', 'wkn', 'valor'])
  for (const shared of ['isoccy', 'isoctry', 'index', 'synthetic', 'isin']) {
    assert.ok(!codes.includes(shared), shared)
  }
})

test('an unknown stated ISIN ends the cascade and fills nothing', () => {
  const registry = new IsinRegistry()
  registry.merge({ isin: APPLE, miccode: 'XNAS' })
  const unknown = registry.resolve(element([['isin', NOVARTIS], ['cusip', '037833100']]))
  assert.deepEqual([unknown.matched, unknown.unmatched, unknown.stated], [false, 'UnknownIsin', NOVARTIS])
  const stated = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=A|22=4|48=${NOVARTIS}|10=0|`))
  assert.ok(!registry.fill(stated))
  assert.equal(stated.isincode, NOVARTIS)
})

test('a code two instruments hold is ambiguous and stops the cascade', () => {
  const registry = new IsinRegistry()
  registry.merge({ isin: APPLE, common: 'C-1' })
  registry.merge({ isin: SAP, common: 'C-1' })
  registry.merge({ isin: DIAGEO, miccode: 'XLON', ticker: 'DGE' })
  const ambiguous = registry.resolve(element([['common', 'C-1']], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [ambiguous.matched, ambiguous.unmatched, ambiguous.tier, ambiguous.kind, ambiguous.isins],
    [false, 'Ambiguous', 'code', 'common', [SAP, APPLE]],
  )
  assert.equal(registry.getByCode('common', 'C-1'), null)
})

test('the listings of one instrument are one match', () => {
  const seeded = IsinRegistry.seeded()
  const sedol = seeded.resolve(element([['sedol', '0540528']]))
  assert.deepEqual([sedol.entry.isin, sedol.tier, sedol.kind, sedol.derived], [HSBC, 'code', 'sedol', true])
  assert.equal(seeded.listings(HSBC).length, 2)
  assert.equal(seeded.getByCode('sedol', '0540528').isin, HSBC, 'two listings, one instrument')
  assert.equal(seeded.getByCode('sedol', '0540528', 'XLON').ticker, 'HSBA')

  const registry = new IsinRegistry()
  for (const market of ['XNAS', 'XNYS']) registry.merge({ isin: APPLE, miccode: market, fisn: 'APPLE INC/SH' })
  assert.equal(registry.rows, 2)
  assert.equal(registry.resolve(element([['cusip', '037833100']])).tier, 'code')
  const byName = registry.resolve(element([['fisn', 'APPLE INC/SH']], { currency: 'USD' }))
  assert.deepEqual([byName.entry.isin, byName.tier, byName.similarity, byName.derived], [APPLE, 'economic', 1, true])

  // On a market the instrument is not listed on, its instrument facts and
  // none of the listing's.
  const london = new IsinRegistry()
  london.merge({ isin: APPLE, miccode: 'XLON', ric: 'AAPL.L', ticker: '0R2V' })
  const swiss = element([['cusip', '037833100']], { miccode: 'XSWX' })
  const resolved = london.resolve(swiss)
  assert.deepEqual([resolved.tier, resolved.derived, resolved.listing], ['code', true, false])
})

test('the economic tier matches a similar short name in the same currency', () => {
  const named = (name, facts = {}) => element([['fisn', name]], { currency: 'USD', ...facts })
  const registryOf = (name, cficode, facts = {}) => {
    const registry = new IsinRegistry()
    registry.merge({ isin: APPLE, miccode: 'XNAS', fisn: name, cficode, ...facts })
    return registry
  }
  const close = registryOf('APPLE INC./SH', 'ESVUFR').resolve(named('APPLE INC/SH'))
  assert.deepEqual(
    [close.matched, close.entry.isin, close.tier, close.derived, close.listing],
    [true, APPLE, 'economic', true, true],
  )
  assert.ok(Math.abs(close.similarity - 12 / 13) < 1e-12, String(close.similarity))

  const plain = registryOf('APPLE INC/SH', 'ESVUFR')
  const below = plain.resolve(named('APPLE INC/SH USD'))
  assert.deepEqual([below.unmatched, below.best, below.isin, below.entry], ['BelowThreshold', 0.75, APPLE, null])

  const conflict = registryOf('APPLE INC/SH', 'DBFTFR').resolve(named('APPLE INC/SH', { cficode: 'ESVUFR' }))
  assert.deepEqual(
    [conflict.unmatched, conflict.stated, conflict.held, conflict.isin],
    ['CfiConflict', 'E', 'D', APPLE],
  )
  assert.equal(plain.resolve(named('APPLE INC/SH', { cficode: 'XXXXXX' })).tier, 'economic', 'XXXXXX conflicts with nothing')
  assert.equal(plain.resolve(named('APPLE INC/SH', { currency: 'EUR' })).unmatched, 'NoCandidate', 'another currency')

  const origin = registryOf('APPLE INC/SH', 'ESVUFR', { origccy: 'USD' }).resolve(named('APPLE INC/SH', { origccy: 'EUR' }))
  assert.deepEqual(
    [origin.unmatched, origin.stated, origin.held, origin.isin],
    ['CurrencyConflict', 'EUR', 'USD', APPLE],
  )

  const twins = registryOf('APPLE INC/SH', 'ESVUFR')
  twins.merge({ isin: MICROSOFT, miccode: 'XNYS', fisn: 'APPLE INC/SH' })
  const ambiguous = twins.resolve(named('APPLE INC/SH'))
  assert.deepEqual(
    [ambiguous.unmatched, ambiguous.tier, ambiguous.similarity, ambiguous.isins],
    ['Ambiguous', 'economic', 1, [APPLE, MICROSOFT]],
  )
  assert.equal(plain.resolve(element([], { ticker: 'ZZZZ', currency: 'USD' })).unmatched, 'NoCandidate', 'no short name')
  assert.equal(plain.resolve(named('APPLE INC/SH', { currency: 'XXX' })).unmatched, 'NoCandidate', 'XXX states no currency')
})

test('a fill takes the economic match only where it is enabled', () => {
  const registry = new IsinRegistry()
  registry.merge({ isin: APPLE, miccode: 'XNAS', fisn: 'APPLE INC./SH' })
  assert.equal(registry.isEconomicMatch, false)
  assert.equal(registry.economicThreshold, 0.85)
  const message = () => codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|'))
  assert.equal(registry.resolve(message()).tier, 'economic', 'resolve always weighs it')
  const off = message()
  assert.ok(!registry.fill(off), 'a judgement no fill takes unasked')
  assert.equal(off.isincode, null)
  registry.setEconomicMatch(true)
  assert.equal(registry.isEconomicMatch, true)
  const on = message()
  assert.ok(registry.fill(on))
  assert.equal(on.isincode, APPLE)
  assert.ok(on.securityids.isDerived('isin'))
  // A stricter threshold refuses what it took; a refusal moves nothing.
  registry.setEconomicThreshold(0.95)
  assert.equal(registry.economicThreshold, 0.95)
  assert.ok(!registry.fill(message()))
  for (const refused of [0, 1.5, Number.NaN, -0.5]) {
    assert.throws(() => registry.setEconomicThreshold(refused), new RegExp(String(refused).replace('.', '\\.')))
  }
  assert.equal(registry.economicThreshold, 0.95)
  registry.setEconomicThreshold(1)
  // The settings are the registry's, never the store's: a clear keeps them.
  registry.clear()
  assert.deepEqual([registry.economicThreshold, registry.isEconomicMatch], [1, true])
})

test('a store bound seeded is laid over the seed', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-isin-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'instruments.arrows')
  const store = IsinRegistry.fromUrl(target)
  assert.ok(store.merge({ isin: APPLE, miccode: 'XNAS', ticker: 'AAPL', currency: 'CHF' }))
  const bae = 'GB0002634946'
  assert.ok(store.merge({ isin: bae, miccode: 'XLON' }))
  store.commit()
  const seed = IsinRegistry.seeded()
  assert.equal(seed.get(bae), null)
  const registry = IsinRegistry.seededFromUrl(target, 1024)
  assert.equal(registry.length, seed.length + 1)
  assert.equal(registry.isDirty, false)
  assert.equal(registry.maxInstruments, 1024)
  assert.equal(registry.get(APPLE).currency, 'CHF', "the store's value wins")
  assert.equal(registry.get(APPLE).fisn, seed.get(APPLE).fisn, "the seed's fact stands")
  assert.deepEqual(registry.get(bae), store.get(bae), 'a row only the store holds')
  assert.equal(registry.commit().writtenRows, 0, 'clean after the load')
  assert.equal(IsinRegistry.fromUrl(target).length, 2, "unseeded: the store's rows alone")
  assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.S' }))
  assert.equal(registry.commit().writtenRows, registry.rows, "every listing of the seed's with the store's")
  assert.equal(IsinRegistry.fromUrl(target).length, registry.length)
  assert.equal(IsinRegistry.seededFromUrl(new IOBase(target)).length, registry.length, 'a handle names the store too')
  assert.throws(() => IsinRegistry.seededFromUrl(new IOBase(target), undefined, { media_type: 'x' }), /properties/)
  const first = IsinRegistry.seededFromUrl(path.join(root, 'isin') + path.sep)
  assert.equal(first.length, seed.length, 'a first run: the seed bound to the store')
  assert.equal(first.commit().writtenRows, 0)
  assert.ok(!fs.existsSync(path.join(root, 'isin')))
  const field = IsinRegistry.field()
  assert.equal(field.get('PARTITION:by'), '["truncate(isin, 2)"]')
  assert.equal(field.get('SORT:by'), '["isin","miccode"]', 'a listing row is keyed by its ISIN and its market')
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
  assert.equal(filled.uuid, bare.uuid)
  assert.equal(filled.hashcode, bare.hashcode)
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
  // Two instruments listing one ticker on two markets: a market resolves
  // to its listing, none resolves to neither.
  assert.ok(registry.merge({ isin: novartis, ticker: 'HOLN', miccode: 'XLON' }))
  assert.deepEqual([isin('HOLN', 'XLON'), isin('HOLN', 'XSWX')], [novartis, HOLCIM])
  assert.equal(isin('HOLN'), null, 'ambiguous')
  assert.equal(registry.remove(novartis).length, 1)
  assert.equal(isin('HOLN'), HOLCIM)
  registry.clear()
  assert.equal(isin('HOLN'), null)
})

test('the process registry is the store the environment names, shared with the codec the environment names', (t) => {
  // Process-wide state, so it is driven in a process of its own whose
  // environment names a scratch store, never the real home.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-isin-env-'))
  t.after(() => fs.rmSync(home, { recursive: true, force: true }))
  const store = path.join(home, 'instruments') + path.sep
  const script = `
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const path = require('node:path')
    const { IsinRegistry, fix } = require(process.argv[1])
    const registry = IsinRegistry.fromEnv()
    // The seed lies under the store, so an empty first run holds it, clean.
    assert.equal(registry.length, 208, 'an empty first run is the seed')
    assert.equal(registry.isDirty, false)
    assert.ok(IsinRegistry.fromEnv().equals(registry), 'resolved once')
    assert.throws(() => IsinRegistry.installEnv(new IsinRegistry()), /already resolved/)
    assert.ok(registry.merge({ isin: '${HOLCIM}', ric: 'HOLN.S' }))
    assert.equal(registry.commit().writtenRows, 209, "the seed's listings ride the first commit")
    assert.ok(fs.existsSync(path.join(process.argv[2], 'part-0.arrows')))
    const codec = fix.FixCodec.fromEnv()
    assert.ok(codec.isinRegistry.equals(registry))
    const own = new IsinRegistry()
    assert.ok(fix.FixCodec.fromEnv({ isinRegistry: own }).isinRegistry.equals(own), 'a stated pin stands')
    assert.equal(new fix.FixCodec(fix.FixRegistry.fromEnv()).isinRegistry, null, 'a codec built by hand attaches none')
    console.log('ok')
  `
  const output = execFileSync(
    process.execPath,
    ['-e', script, require.resolve('yggdryl'), store],
    { encoding: 'utf8', env: { ...process.env, HOME: home, USERPROFILE: home, YGGDRYL_ISIN_REGISTRY_URI: store } },
  )
  assert.equal(output.trim(), 'ok')
  // Installed first, the caller's own table is the one every later
  // `fromEnv` answers.
  const installed = `
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require(process.argv[1])
    const own = new IsinRegistry(4)
    IsinRegistry.installEnv(own)
    assert.ok(IsinRegistry.fromEnv().equals(own))
    assert.throws(() => IsinRegistry.installEnv(new IsinRegistry()), /already resolved/)
    console.log('ok')
  `
  const second = execFileSync(
    process.execPath,
    ['-e', installed, require.resolve('yggdryl')],
    { encoding: 'utf8', env: { ...process.env, HOME: home, USERPROFILE: home, YGGDRYL_ISIN_REGISTRY_URI: store } },
  )
  assert.equal(second.trim(), 'ok')
})
