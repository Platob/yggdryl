'use strict'

// `node/src/instrument.rs`: the instruments, one row per instrument keyed by
// its cross code, its listings nested, learned from and filled into FIX
// messages, bound to the store it is loaded from and committed back to,
// redirected to the core.

const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')
const { BatchReader, IOBase, IOResult, Identifier, Instruments, fix, graph } = require('yggdryl')

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

/** A message stating the EUR/USD pair alone. */
function pair(reader = codec) {
  return reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=F|55=EUR/USD|15=EUR|10=0|'))
}

const ZERO = '00000000-0000-0000-0000-000000000000'

/**
 * One instrument as `merge` reads it: its `facts` alone - a statement is
 * derived, so the element columns and `placeholder` are written again from
 * them and need no stating.
 */
function statement(facts) {
  return { ...facts }
}

/** One listing of a statement: its market and the `facts` beside it. */
function listing(facts) {
  return { miccode: null, ticker: null, currency: null, codes: null, ...facts }
}

/** An identifier map, as a row's `securityids` and a listing's `codes` hold one. */
function ids(entries) {
  return new Map(Object.entries(entries))
}

/**
 * The nanoseconds a row's instant holds: a `Date` where the instant is a
 * whole millisecond, else the datetime `Scalar` that keeps the nanoseconds.
 */
function nanos(instant) {
  return instant instanceof Date ? BigInt(instant.getTime()) * 1_000_000n : instant.count
}

/** A row compared as text, its integers spelled. */
function text(row) {
  return JSON.stringify(row, (_, value) => (typeof value === 'bigint' ? String(value) : value))
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

test('the instruments learn a message and fill a later one named by its ticker', () => {
  const instruments = new Instruments()
  assert.equal(instruments.length, 0)
  assert.equal(instruments.isDirty, false)
  const message = stated()
  assert.equal(message.instcode, HOLCIM, 'a parse writes the code the message alone spells')
  assert.ok(instruments.learn(message))
  assert.equal(instruments.isDirty, true)
  const row = instruments.get(HOLCIM)
  assert.deepEqual(
    [row.crosscode, row.isin, row.cficode, row.placeholder, row.countrycode],
    [HOLCIM, HOLCIM, 'ESVUFR', false, null],
    'a security is keyed by its bare real ISIN; the prefix already says the country',
  )
  assert.equal(row.uuid, row.crossuuid, 'the identity is the code')
  assert.deepEqual(row.securityids, ids({ cfi: 'ESVUFR', isin: HOLCIM, valor: '1221405' }), 'the valor its CH ISIN embeds')
  // The listing facts are the market's: the RIC is a listing code.
  assert.deepEqual(instruments.listings(HOLCIM), [
    { codes: ids({ ric: 'HOLN.S' }), currency: 'CHF', miccode: 'XSWX', ticker: 'HOLN' },
  ])
  assert.deepEqual(row.listings, instruments.listings(HOLCIM), 'the row nests its listings by name')
  assert.equal(ZERO.length, 36)
  assert.equal(instruments.getByTicker('HOLN', 'XSWX').crosscode, HOLCIM)
  assert.equal(instruments.getByTicker('HOLN', 'XLON'), null)
  const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|'))
  assert.deepEqual([later.isincode, later.instcode], [null, null])
  assert.ok(instruments.fill(later))
  assert.equal(later.isincode, HOLCIM)
  assert.equal(later.instcode, HOLCIM, 'the fill states the instrument it resolved')
  assert.ok(later.securityids.isDerived('isin'))
  assert.equal(later.securityids.get('ric'), 'HOLN.S')
  assert.equal(later.ticker, 'HOLN')
  assert.equal(later.currency, 'CHF')
  assert.ok(!instruments.fill(later), 'nothing left to fill')
  const wire = later.intoText('|')
  assert.ok(!wire.includes('461=') && !wire.includes('15=') && !wire.includes('48='), 'never the wire')
  // A RIC is a listing code and, with no ISIN, a lookup key.
  const byRic = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|'))
  assert.ok(instruments.fill(byRic))
  assert.deepEqual([byRic.isincode, byRic.instcode], [HOLCIM, HOLCIM])
  assert.ok(byRic.securityids.isDerived('isin'))
  assert.ok(new Instruments().enrich(stated()))
})

test('an FX pair is an instrument keyed by its class and body, its number minted', () => {
  const message = pair()
  assert.equal(message.instcode, 'IF:EUR/USD', 'the parse spells the pair from the message alone')
  assert.equal(message.isincode, 'QYLTVIRYHNX5', 'the number minted for the code')
  assert.ok(message.securityids.isDerived('isin'))
  const instruments = new Instruments()
  const unknown = instruments.resolve(message)
  assert.deepEqual([unknown.matched, unknown.unmatched, unknown.stated], [false, 'UnknownCode', 'IF:EUR/USD'])
  assert.ok(instruments.learn(message))
  const row = instruments.get('IF:EUR/USD')
  assert.deepEqual(
    [row.crosscode, row.cficode, row.forexcode, row.isin, row.currency, row.countrycode],
    ['IF:EUR/USD', 'IFXXXP', 'EUR/USD', 'QYLTVIRYHNX5', 'USD', null],
    'its currency the quote leg, its country none',
  )
  assert.equal(row.securityids.get('yggdryl:isin'), 'QYLTVIRYHNX5')
  assert.equal(instruments.get('QYLTVIRYHNX5').crosscode, 'IF:EUR/USD', 'the minted number finds it')
  const found = instruments.resolve(pair())
  assert.deepEqual([found.matched, found.tier, found.entry.crosscode], [true, 'crosscode', 'IF:EUR/USD'])
  // A walk learns the pair and states it on what it yields.
  const walked = new Instruments()
  const walking = new fix.FixCodec(codec.registry, { instruments: walked })
  const yielded = [...walking.lifecycle([pair(walking)])]
  assert.deepEqual(yielded.map((held) => [held.instcode, held.isincode]), [['IF:EUR/USD', 'QYLTVIRYHNX5']])
  assert.equal(walked.get('IF:EUR/USD').crosscode, 'IF:EUR/USD')
})

test('a body row nests its characteristics and legs by name and derives its identity', () => {
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: APPLE })), 'a statement of facts alone')
  const apple = instruments.get(APPLE)
  assert.deepEqual([apple.crosscode, apple.uuid === apple.crossuuid, apple.uuid !== ZERO, apple.placeholder], [APPLE, true, true, false])
  assert.ok(
    instruments.merge(statement({ cficode: 'OCXXXX', underlying: APPLE, characteristics: { expiry: '2026-12-18', strikepx: '200' } })),
  )
  const option = instruments.get('OC:US0378331005:2026-12-18:200')
  assert.deepEqual(
    [option.characteristics.expiry, option.characteristics.settle, option.underlying],
    ['2026-12-18', null, APPLE],
    'the characteristics by name',
  )
  assert.ok(option.characteristics.strikepx !== null, 'the strike, an exact decimal, crosses as a Scalar')
  assert.ok(option.isin.startsWith('QY') && option.securityids.get('yggdryl:isin') === option.isin, 'its number minted')
  assert.ok(instruments.merge(statement({ cficode: 'KEXXXX', legs: [{ code: APPLE, ratio: 1 }, { code: HOLCIM, ratio: 2 }] })))
  const spread = instruments.get(`KE:2*${HOLCIM}+${APPLE}`)
  assert.ok(spread, 'the strategy keyed by its legs, in code order')
  assert.deepEqual(spread.legs.map((leg) => [leg.code, leg.ratio]), [[HOLCIM, 2], [APPLE, 1]], 'the legs by name, in code order')
  // A row read back merges as it is, moving nothing.
  assert.ok(!instruments.merge(option))
  assert.ok(!instruments.merge(spread))
})

test('an instrument merges by the update rule', () => {
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: HOLCIM, updunix: 10n, securityids: ids({ cusip: '037833100' }) })))
  assert.ok(
    !instruments.merge(statement({ isin: HOLCIM, securityids: ids({ cusip: '037833100' }) })),
    'a statement saying nothing new moves nothing',
  )
  assert.ok(
    !instruments.merge(statement({ isin: HOLCIM, securityids: ids({ cusip: '037833101' }) })),
    'a typo under a checked code moves nothing',
  )
  assert.ok(instruments.merge(statement({ isin: HOLCIM, countrycode: 'LI', currency: 'CHF', forexcode: 'EUR/CHF' })))
  const row = instruments.get(HOLCIM)
  assert.deepEqual([row.countrycode, row.currency, row.forexcode], ['LI', 'CHF', 'EUR/CHF'])
  assert.equal(row.securityids.get('cusip'), '037833100')
  assert.equal(row.underlying, null)
  assert.ok(instruments.merge(statement({ isin: HOLCIM, underlying: APPLE })), 'an instrument fact fills')
  assert.ok(!instruments.merge(statement({ isin: HOLCIM, underlying: HOLCIM })), 'its own code states nothing')
  assert.equal(instruments.get(HOLCIM).underlying, APPLE)
  assert.throws(
    () => instruments.merge(statement({ securityids: ids({ ric: 'HOLN.S' }) })),
    /crosscode: expected a real ISIN or a class/,
  )
  // `remove` answers the instrument, then none.
  assert.equal(instruments.remove(HOLCIM).crosscode, HOLCIM)
  assert.equal(instruments.remove(HOLCIM), null)
  instruments.merge(statement({ isin: HOLCIM }))
  instruments.clear()
  assert.equal(instruments.length, 0)
  const bounded = new Instruments(1)
  assert.equal(bounded.maxInstruments, 1)
  assert.ok(bounded.merge(statement({ isin: HOLCIM })))
  assert.throws(() => bounded.merge(statement({ isin: APPLE })), /at most 1 instruments/)
  assert.equal(bounded.toString(), 'Instruments(len=1, maxInstruments=1, dirty=true)')
})

test('an instrument takes the defaults its ISIN and its market imply', () => {
  // The national number a closed ISIN embeds fills the listing of its
  // country's market, and a listing's empty currency is its market's
  // country's legal tender; a stated value is never replaced by either.
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: DIAGEO, listings: [listing({ miccode: 'XLON' })] })))
  assert.deepEqual(instruments.listings(DIAGEO), [
    { codes: ids({ sedol: '0237400' }), currency: 'GBP', miccode: 'XLON', ticker: null },
  ], "a GB '00' ISIN embeds its SEDOL, and XLON is in GB")
  assert.ok(instruments.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XETR', currency: 'USD' })] })))
  assert.equal(instruments.getListing(APPLE, 'XETR').currency, 'USD', 'a stated currency stands')
  assert.equal(instruments.get(APPLE).securityids.get('cusip'), '037833100')
})

test('an instrument holds one listing per market', () => {
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: HOLCIM, updunix: 10n, listings: [listing({ miccode: 'XSWX', ticker: 'HOLN' })] })))
  assert.ok(
    instruments.merge(statement({ isin: HOLCIM, listings: [listing({ miccode: 'XLON', ticker: '0QKY' })] })),
    'a new market is a new listing',
  )
  assert.equal(instruments.length, 1, 'one instrument')
  assert.equal(instruments.rows, 1, 'one row, its listings nested')
  assert.deepEqual(instruments.listings(HOLCIM).map((held) => [held.miccode, held.ticker, held.currency]), [
    ['XLON', '0QKY', 'GBP'],
    ['XSWX', 'HOLN', 'CHF'],
  ], 'in MIC order, each with its own market default')
  assert.deepEqual(instruments.listings(APPLE), [])
  assert.equal(instruments.getListing(HOLCIM, 'XSWX').ticker, 'HOLN')
  assert.equal(instruments.getListing(HOLCIM, 'XNAS'), null)
  assert.throws(() => instruments.getListing(HOLCIM, 'TOOLONG'))
  assert.equal(instruments.getByTicker('0QKY', 'XLON').crosscode, HOLCIM)
  assert.equal(instruments.getByTicker('HOLN').crosscode, HOLCIM, 'the one instrument listing the ticker on any market')
  assert.ok(instruments.merge(statement({ isin: HOLCIM, cficode: 'ESVUFR' })), 'an instrument fact')
  assert.equal(instruments.get(HOLCIM).cficode, 'ESVUFR')
  assert.ok(
    instruments.merge(statement({ isin: HOLCIM, listings: [listing({ miccode: 'XSWX', codes: ids({ ric: 'HOLN.S' }) })] })),
  )
  assert.deepEqual(
    instruments.listings(HOLCIM).map((held) => held.codes),
    [null, ids({ ric: 'HOLN.S' })],
    'a listing code on the listing it names',
  )
  assert.equal(instruments.removeListing(HOLCIM, 'XLON').miccode, 'XLON')
  assert.equal(instruments.removeListing(HOLCIM, 'XLON'), null)
  assert.equal(instruments.listings(HOLCIM).length, 1)
  assert.equal(instruments.removeListing(HOLCIM, 'XSWX').ticker, 'HOLN')
  assert.deepEqual([instruments.length, instruments.listings(HOLCIM)], [1, []], 'the instrument stays')
})

test('a learn moves lastunix to the latest instant, firstunix to the earliest and updunix only where a fact moved', () => {
  const at = (stamp) => codec.parseFixLine(Buffer.from(
    `8=FIX.4.4|35=D|52=${stamp}|11=A|22=4|48=${HOLCIM}|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|`,
  ))
  const first = at('20260102-10:00:00')
  const later = at('20260102-11:00:00')
  const earlier = at('20260102-09:00:00')
  const instruments = new Instruments()
  assert.ok(instruments.learn(first))
  const learned = instruments.get(HOLCIM)
  assert.equal(nanos(learned.lastunix), first.transunix, 'lastunix is the instant of the event that stated the ISIN')
  assert.equal(nanos(learned.firstunix), first.transunix, 'the first learn sets firstunix too')
  assert.equal(nanos(learned.updunix), first.transunix)
  const clean = Instruments.fromArrowReader(instruments.intoArrowReader())
  assert.equal(clean.isDirty, false)
  assert.ok(clean.learn(later), 'meeting a known instrument later moves the collection')
  assert.equal(clean.isDirty, true)
  const met = clean.get(HOLCIM)
  assert.equal(nanos(met.lastunix), later.transunix)
  assert.equal(nanos(met.firstunix), first.transunix, 'a later instant leaves firstunix')
  assert.equal(nanos(met.updunix), first.transunix, 'no fact moved, so updunix stays')
  const replayed = Instruments.fromArrowReader(clean.intoArrowReader())
  assert.ok(!replayed.learn(first), 'between the two instants nothing moves')
  assert.equal(replayed.isDirty, false)
  assert.ok(replayed.learn(earlier), 'an earlier instant moves firstunix back')
  assert.equal(replayed.isDirty, true)
  const back = replayed.get(HOLCIM)
  assert.equal(nanos(back.firstunix), earlier.transunix)
  assert.equal(nanos(back.lastunix), later.transunix, 'lastunix unmoved')
  assert.equal(nanos(back.updunix), first.transunix, 'updunix unmoved')
  // A stated one merges earlier-wins.
  assert.ok(!replayed.merge(statement({ isin: HOLCIM, firstunix: back.firstunix })))
  assert.ok(replayed.merge(statement({ isin: HOLCIM, firstunix: 1n })))
  assert.equal(nanos(replayed.get(HOLCIM).firstunix), 1n)
})

test('the seed holds the common instruments, clean and bound to no store', () => {
  const seeded = Instruments.seeded()
  assert.equal(seeded.length, 208)
  assert.equal(seeded.rows, 208, 'one row per instrument')
  assert.deepEqual(seeded.listings(HSBC).map((held) => held.miccode), ['XHKG', 'XLON'], 'HSBC is listed twice')
  assert.equal(seeded.isDirty, false)
  assert.equal(seeded.maxInstruments, 16384)
  assert.throws(() => seeded.commit(), /holder/, 'bound to no store')
  const apple = seeded.getByTicker('AAPL', 'XNAS')
  assert.deepEqual([apple.crosscode, apple.fisn, apple.cficode], [APPLE, 'APPLE INC/SH SH', 'ESVUFR'])
  assert.equal(apple.securityids.get('cusip'), '037833100', 'the CUSIP its ISIN embeds')
  assert.deepEqual(seeded.listings(APPLE), [{ codes: null, currency: 'USD', miccode: 'XNAS', ticker: 'AAPL' }])
  assert.equal(seeded.getListing(DIAGEO, 'XLON').codes.get('sedol'), '0237400')
  assert.equal(new Instruments().length, 0, 'new holds none of it')
  assert.ok(seeded.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XNAS', codes: ids({ ric: 'AAPL.OQ' }) })] })))
  assert.equal(seeded.isDirty, true)
  assert.equal(Instruments.seeded().getListing(APPLE, 'XNAS').codes, null, 'each seeded collection is its own')
  const field = Instruments.field()
  assert.equal(field.fieldLen, 25)
  assert.equal(field.indexOf('crosscode'), 2)
  assert.equal(field.indexOf('metadata'), field.indexOf('listings') + 1)
  assert.equal(field.indexOf('origccy'), field.indexOf('currency') + 1)
  // The seed states an origin currency where the issue's is not the
  // trading one, and derives none.
  assert.equal(seeded.get(CSPX).origccy, 'USD')
  assert.equal(apple.origccy, null)
})

test('the origin currency is an instrument fact, stated and never derived', () => {
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: CSPX, listings: [listing({ miccode: 'XLON', currency: 'USD' })] })))
  assert.equal(instruments.get(CSPX).origccy, null, "neither the prefix's EUR nor the listing's USD is derived")
  assert.ok(instruments.merge(statement({ isin: CSPX, origccy: 'USD', listings: [listing({ miccode: 'XETR', currency: 'EUR' })] })))
  assert.equal(instruments.get(CSPX).origccy, 'USD')
  assert.deepEqual(instruments.listings(CSPX).map((held) => [held.miccode, held.currency]), [['XETR', 'EUR'], ['XLON', 'USD']])
  assert.ok(!instruments.merge(statement({ isin: CSPX, origccy: 'USD' })), 'restated, it moves nothing')

  // A EUR listing of the USD class, learned from a message stating only
  // its currency: the origin stays USD; a fill lands it where the message
  // holds none.
  const seeded = Instruments.seeded()
  const sxr8 = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=A|22=4|48=${CSPX}|55=SXR8|207=XETR|15=EUR|10=0|`))
  assert.equal(sxr8.origccy, null)
  assert.equal(sxr8.originCurrency, 'EUR', 'the currency where none is held')
  assert.ok(seeded.learn(sxr8))
  assert.equal(seeded.get(CSPX).origccy, 'USD', 'never EUR')
  assert.equal(seeded.getListing(CSPX, 'XETR').ticker, 'SXR8')
  const filled = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=B|22=4|48=${CSPX}|207=XETR|10=0|`))
  assert.ok(seeded.fill(filled))
  assert.deepEqual([filled.origccy, filled.currency, filled.originCurrency], ['USD', 'EUR', 'USD'])
  // An instrument stating none fills none.
  const holcim = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=C|22=4|48=${HOLCIM}|207=XSWX|10=0|`))
  assert.ok(seeded.fill(holcim))
  assert.equal(holcim.origccy, null)
  assert.equal(holcim.originCurrency, 'CHF')
})

test('the cascade takes the ISIN, then a code, then the ticker on its market', () => {
  const instruments = new Instruments()
  instruments.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XNAS', ticker: 'AAPL' })] }))
  instruments.merge(statement({ isin: DIAGEO, listings: [listing({ miccode: 'XLON', ticker: 'DGE' })] }))
  instruments.merge(statement({ isin: SAP, listings: [listing({ miccode: 'XETR' })] }))
  // The ISIN wins over a CUSIP naming Apple.
  const byIsin = instruments.resolve(element([['isin', SAP], ['cusip', '037833100']]))
  assert.equal(byIsin.matched, true)
  assert.deepEqual(
    [byIsin.entry.crosscode, byIsin.tier, byIsin.kind, byIsin.derived, byIsin.listing, byIsin.unmatched],
    [SAP, 'isin', null, false, true, null],
  )
  // A CUSIP wins over a ticker naming Diageo; Apple is not listed on XLON.
  const byCode = instruments.resolve(element([['cusip', '037833100']], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [byCode.entry.crosscode, byCode.tier, byCode.kind, byCode.derived, byCode.listing],
    [APPLE, 'code', 'cusip', true, false],
  )
  // The ticker on its market last.
  const byTicker = instruments.resolve(element([], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [byTicker.entry.crosscode, byTicker.tier, byTicker.derived, byTicker.listing],
    [DIAGEO, 'symbology', true, true],
  )
  // The same through the public lookups.
  assert.equal(instruments.getByCode('cusip', '037833100').crosscode, APPLE)
  assert.equal(instruments.getByCode('sedol', '0237400').crosscode, DIAGEO, 'the SEDOL its ISIN embeds')
  assert.equal(instruments.getByCode('cusip', 'not a cusip'), null)
  assert.equal(instruments.getByCode('isoccy', 'USD'), null, 'a currency is no key')
  assert.throws(() => instruments.getByCode('', '037833100'), /identifier type/)
  // Nothing stated: no key; a ticker nothing lists: no candidate.
  const none = instruments.resolve(element([]))
  assert.deepEqual([none.matched, none.entry, none.tier, none.unmatched], [false, null, null, 'NoKey'])
  assert.equal(instruments.resolve(element([], { ticker: 'ZZZZ' })).unmatched, 'NoCandidate')
  // Any market value resolves, a FixMsg and a MarketData included.
  const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|55=DGE|207=XLON|10=0|'))
  assert.equal(instruments.resolve(message).entry.crosscode, DIAGEO)
  assert.equal(instruments.resolve(new graph.MarketData(element([['cusip', '037833100']]))).entry.crosscode, APPLE)
  assert.throws(() => instruments.resolve({ isin: APPLE }), /expected MarketData, a market leaf or a FixMsg/)
  // The keys a lookup reads, in the order it reads them.
  const codes = Instruments.lookupCodes()
  assert.equal(codes.length, 20)
  assert.deepEqual(codes.slice(0, 4), ['cusip', 'sedol', 'wkn', 'valor'])
  for (const shared of ['isoccy', 'isoctry', 'index', 'synthetic', 'isin']) {
    assert.ok(!codes.includes(shared), shared)
  }
})

test('an unknown stated ISIN ends the cascade and fills nothing', () => {
  const instruments = new Instruments()
  instruments.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XNAS' })] }))
  const unknown = instruments.resolve(element([['isin', NOVARTIS], ['cusip', '037833100']]))
  assert.deepEqual([unknown.matched, unknown.unmatched, unknown.stated], [false, 'UnknownIsin', NOVARTIS])
  const message = codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|11=A|22=4|48=${NOVARTIS}|10=0|`))
  assert.ok(!instruments.fill(message))
  assert.deepEqual([message.isincode, message.instcode], [NOVARTIS, NOVARTIS], 'the parse spelled the code already')
})

test('a code two instruments hold is ambiguous and stops the cascade', () => {
  const instruments = new Instruments()
  instruments.merge(statement({ isin: APPLE, securityids: ids({ common: 'C-1' }) }))
  instruments.merge(statement({ isin: SAP, securityids: ids({ common: 'C-1' }) }))
  instruments.merge(statement({ isin: DIAGEO, listings: [listing({ miccode: 'XLON', ticker: 'DGE' })] }))
  const ambiguous = instruments.resolve(element([['common', 'C-1']], { ticker: 'DGE', miccode: 'XLON' }))
  assert.deepEqual(
    [ambiguous.matched, ambiguous.unmatched, ambiguous.tier, ambiguous.kind, ambiguous.codes],
    [false, 'Ambiguous', 'code', 'common', [SAP, APPLE]],
    'the two cross codes, in code order',
  )
  assert.equal(instruments.getByCode('common', 'C-1'), null)
})

test('the listings of one instrument are one match', () => {
  const seeded = Instruments.seeded()
  const sedol = seeded.resolve(element([['sedol', '0540528']]))
  assert.deepEqual([sedol.entry.crosscode, sedol.tier, sedol.kind, sedol.derived], [HSBC, 'code', 'sedol', true])
  assert.equal(seeded.getByCode('sedol', '0540528').crosscode, HSBC, 'two listings, one instrument')

  const instruments = new Instruments()
  for (const market of ['XNAS', 'XNYS']) {
    instruments.merge(statement({ isin: APPLE, fisn: 'APPLE INC/SH', listings: [listing({ miccode: market })] }))
  }
  assert.deepEqual([instruments.length, instruments.listings(APPLE).length], [1, 2])
  assert.equal(instruments.resolve(element([['cusip', '037833100']])).tier, 'code')
  const byName = instruments.resolve(element([['fisn', 'APPLE INC/SH']], { currency: 'USD' }))
  assert.deepEqual([byName.entry.crosscode, byName.tier, byName.similarity, byName.derived], [APPLE, 'economic', 1, true])

  // On a market the instrument is not listed on, its instrument facts and
  // none of the listing's.
  const london = new Instruments()
  london.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XLON', ticker: '0R2V', codes: ids({ ric: 'AAPL.L' }) })] }))
  const resolved = london.resolve(element([['cusip', '037833100']], { miccode: 'XSWX' }))
  assert.deepEqual([resolved.tier, resolved.derived, resolved.listing], ['code', true, false])
})

test('the economic tier matches a similar short name in the same currency', () => {
  const named = (name, facts = {}) => element([['fisn', name]], { currency: 'USD', ...facts })
  const instrumentsOf = (name, cficode, facts = {}) => {
    const instruments = new Instruments()
    instruments.merge(statement({ isin: APPLE, fisn: name, cficode, listings: [listing({ miccode: 'XNAS' })], ...facts }))
    return instruments
  }
  const close = instrumentsOf('APPLE INC./SH', 'ESVUFR').resolve(named('APPLE INC/SH'))
  assert.deepEqual(
    [close.matched, close.entry.crosscode, close.tier, close.derived, close.listing],
    [true, APPLE, 'economic', true, true],
  )
  assert.ok(Math.abs(close.similarity - 12 / 13) < 1e-12, String(close.similarity))

  const plain = instrumentsOf('APPLE INC/SH', 'ESVUFR')
  const below = plain.resolve(named('APPLE INC/SH USD'))
  assert.deepEqual([below.unmatched, below.best, below.code, below.entry], ['BelowThreshold', 0.75, APPLE, null])

  const conflict = instrumentsOf('APPLE INC/SH', 'DBFTFR').resolve(named('APPLE INC/SH', { cficode: 'ESVUFR' }))
  assert.deepEqual(
    [conflict.unmatched, conflict.stated, conflict.held, conflict.code],
    ['CfiConflict', 'E', 'D', APPLE],
  )
  assert.equal(plain.resolve(named('APPLE INC/SH', { cficode: 'XXXXXX' })).tier, 'economic', 'XXXXXX conflicts with nothing')
  assert.equal(plain.resolve(named('APPLE INC/SH', { currency: 'EUR' })).unmatched, 'NoCandidate', 'another currency')

  const origin = instrumentsOf('APPLE INC/SH', 'ESVUFR', { origccy: 'USD' }).resolve(named('APPLE INC/SH', { origccy: 'EUR' }))
  assert.deepEqual(
    [origin.unmatched, origin.stated, origin.held, origin.code],
    ['CurrencyConflict', 'EUR', 'USD', APPLE],
  )

  const twins = instrumentsOf('APPLE INC/SH', 'ESVUFR')
  twins.merge(statement({ isin: MICROSOFT, fisn: 'APPLE INC/SH', listings: [listing({ miccode: 'XNYS' })] }))
  const ambiguous = twins.resolve(named('APPLE INC/SH'))
  assert.deepEqual(
    [ambiguous.unmatched, ambiguous.tier, ambiguous.similarity, ambiguous.codes],
    ['Ambiguous', 'economic', 1, [APPLE, MICROSOFT]],
  )
  assert.equal(plain.resolve(element([], { ticker: 'ZZZZ', currency: 'USD' })).unmatched, 'NoCandidate', 'no short name')
  assert.equal(plain.resolve(named('APPLE INC/SH', { currency: 'XXX' })).unmatched, 'NoCandidate', 'XXX states no currency')
})

test('a fill takes the economic match only where it is enabled', () => {
  const instruments = new Instruments()
  instruments.merge(statement({ isin: APPLE, fisn: 'APPLE INC./SH', listings: [listing({ miccode: 'XNAS' })] }))
  assert.equal(instruments.isEconomicMatch, false)
  assert.equal(instruments.economicThreshold, 0.85)
  const message = () => codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|'))
  assert.equal(instruments.resolve(message()).tier, 'economic', 'resolve always weighs it')
  const off = message()
  assert.ok(!instruments.fill(off), 'a judgement no fill takes unasked')
  assert.equal(off.isincode, null)
  instruments.setEconomicMatch(true)
  assert.equal(instruments.isEconomicMatch, true)
  const on = message()
  assert.ok(instruments.fill(on))
  assert.deepEqual([on.isincode, on.instcode], [APPLE, APPLE])
  assert.ok(on.securityids.isDerived('isin'))
  // A stricter threshold refuses what it took; a refusal moves nothing.
  instruments.setEconomicThreshold(0.95)
  assert.equal(instruments.economicThreshold, 0.95)
  assert.ok(!instruments.fill(message()))
  for (const refused of [0, 1.5, Number.NaN, -0.5]) {
    assert.throws(() => instruments.setEconomicThreshold(refused), new RegExp(String(refused).replace('.', '\\.')))
  }
  assert.equal(instruments.economicThreshold, 0.95)
  instruments.setEconomicThreshold(1)
  // The settings are the collection's, never the store's: a clear keeps them.
  instruments.clear()
  assert.deepEqual([instruments.economicThreshold, instruments.isEconomicMatch], [1, true])
})

test('a store bound seeded is laid over the seed', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-instruments-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'instruments.arrows')
  const store = Instruments.fromUrl(target)
  assert.ok(store.merge(statement({ isin: APPLE, listings: [listing({ miccode: 'XNAS', ticker: 'AAPL', currency: 'CHF' })] })))
  const bae = 'GB0002634946'
  assert.ok(store.merge(statement({ isin: bae, listings: [listing({ miccode: 'XLON' })] })))
  store.commit()
  const seed = Instruments.seeded()
  assert.equal(seed.get(bae), null)
  const instruments = Instruments.seededFromUrl(target, 1024)
  assert.equal(instruments.length, seed.length + 1)
  assert.equal(instruments.isDirty, false)
  assert.equal(instruments.maxInstruments, 1024)
  assert.equal(instruments.getListing(APPLE, 'XNAS').currency, 'CHF', "the store's value wins")
  assert.equal(instruments.get(APPLE).fisn, seed.get(APPLE).fisn, "the seed's fact stands")
  assert.equal(text(instruments.get(bae)), text(store.get(bae)), 'an instrument only the store holds')
  assert.equal(instruments.commit().writtenRows, 0, 'clean after the load')
  assert.equal(Instruments.fromUrl(target).length, 2, "unseeded: the store's rows alone")
  assert.ok(instruments.merge(statement({ isin: HOLCIM, securityids: ids({ wkn: '869898' }) })))
  assert.equal(instruments.commit().writtenRows, instruments.rows, "every instrument of the seed's with the store's")
  assert.equal(Instruments.fromUrl(target).length, instruments.length)
  assert.equal(Instruments.seededFromUrl(new IOBase(target)).length, instruments.length, 'a handle names the store too')
  assert.throws(() => Instruments.seededFromUrl(new IOBase(target), undefined, { media_type: 'x' }), /properties/)
  const first = Instruments.seededFromUrl(path.join(root, 'instruments') + path.sep)
  assert.equal(first.length, seed.length, 'a first run: the seed bound to the store')
  assert.equal(first.commit().writtenRows, 0)
  assert.ok(!fs.existsSync(path.join(root, 'instruments')))
  const field = Instruments.field()
  assert.equal(field.get('PARTITION:by'), '["truncate(crosscode, 2)"]')
  assert.equal(field.get('SORT:by'), '["crosscode"]', 'an instrument is keyed by its cross code')
})

test('a table that is no instrument row is refused, and a column no field reads lands in metadata', () => {
  const golden = new arrow.Table({
    ISIN: arrow.vectorFromArray([HOLCIM, APPLE], new arrow.Utf8()),
    RIC: arrow.vectorFromArray(['HOLN.S', 'AAPL.OQ'], new arrow.Utf8()),
  })
  assert.throws(() => Instruments.fromArrowReader(BatchReader.from(golden)), /lacks the crosscode column/)
  const instruments = new Instruments()
  instruments.merge(statement({ isin: HOLCIM, cficode: 'ESVUFR' }))
  instruments.merge(statement({ isin: APPLE }))
  const stored = instruments.intoArrowReader().intoTable()
  assert.equal(stored.schema.fields.length, 25)
  const desk = stored.assign(new arrow.Table({ desk: arrow.vectorFromArray(['EQ', null], new arrow.Utf8()) }))
  const loaded = Instruments.fromArrowReader(BatchReader.from(desk))
  assert.equal(loaded.isDirty, false, 'a load leaves the collection clean')
  assert.deepEqual(loaded.get(HOLCIM).metadata, ids({ desk: 'EQ' }))
  assert.equal(loaded.get(APPLE).metadata, null, 'a null cell states nothing')
  assert.equal(loaded.extendFromArrowReader(BatchReader.from(desk)), 2, 'a known row folds again')
})

test('the instruments round trip through a holder, their stream a snapshot', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-instruments-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const instruments = new Instruments()
  instruments.merge(statement({ isin: HOLCIM, cficode: 'ESVUFR', updunix: 7n, listings: [listing({ miccode: 'XSWX', codes: ids({ ric: 'HOLN.S' }) })] }))
  instruments.merge(statement({ isin: APPLE }))
  const snapshot = instruments.intoArrowReader()
  instruments.clear()
  const target = path.join(root, 'instruments.arrow')
  new IOBase(target).overwriteArrowReader(snapshot)
  const loaded = Instruments.fromUrl(target)
  assert.equal(loaded.length, 2, 'the stream is a snapshot a later write does not move')
  assert.equal(loaded.get(HOLCIM).cficode, 'ESVUFR')
  assert.equal(loaded.getListing(HOLCIM, 'XSWX').codes.get('ric'), 'HOLN.S')
  assert.equal(loaded.extendFromHandle(new IOBase(target)), 2)
  assert.equal(Instruments.fromUrl(new IOBase(target)).length, 2, 'a handle names the store too')
  assert.equal(Instruments.fromUrl(path.join(root, 'missing.arrow')).length, 0, 'a missing store is empty')
  assert.throws(() => Instruments.fromUrl(new IOBase(target), undefined, { media_type: 'x' }), /properties/)
})

test('the instruments commit to their store only where they moved', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-instruments-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const store = path.join(root, 'instruments') + path.sep
  const instruments = Instruments.fromUrl(store, 8)
  assert.equal(instruments.length, 0)
  assert.equal(instruments.isDirty, false)
  assert.equal(instruments.maxInstruments, 8)
  assert.ok(!fs.existsSync(path.join(root, 'instruments')), 'nothing is laid out before a commit')
  assert.ok(instruments.commit() instanceof IOResult)
  assert.equal(instruments.commit().writtenRows, 0, 'a clean collection writes nothing')
  assert.ok(instruments.merge(statement({ isin: HOLCIM })))
  assert.equal(instruments.isDirty, true)
  assert.equal(instruments.commit().writtenRows, 1)
  assert.equal(instruments.isDirty, false)
  assert.ok(fs.existsSync(path.join(root, 'instruments', 'part-0.arrows')))
  assert.equal(instruments.commit().writtenRows, 0, 'a second commit writes nothing')
  const reloaded = Instruments.fromUrl(store)
  assert.equal(text(reloaded.get(HOLCIM)), text(instruments.get(HOLCIM)))
  assert.equal(reloaded.isDirty, false)
  instruments.clear()
  instruments.commit()
  assert.equal(Instruments.fromUrl(store).length, 0, 'emptied on the next commit')
  assert.throws(() => new Instruments().commit(), /holder/)
})

test('a codec shares the caller\'s instruments with every lifecycle and every parse', () => {
  const instruments = new Instruments()
  const shared = new fix.FixCodec(codec.registry, { instruments })
  assert.ok(shared.instruments.equals(instruments))
  assert.equal(codec.instruments, null)
  assert.ok(!instruments.equals(new Instruments()), 'equal only to itself')
  for (const _ of shared.lifecycle([stated(shared)])) void _
  assert.equal(instruments.length, 1)
  assert.notEqual(instruments.get(HOLCIM), null)
  for (const _ of codec.lifecycle([stated()])) void _
  assert.equal(instruments.length, 1, 'a codec without one learns into its own')
  // A walk fills the instrument's code onto a later message naming it by
  // its ticker alone.
  const later = shared.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=Q|55=HOLN|207=XSWX|10=0|'))
  assert.deepEqual([...shared.lifecycle([later])].map((held) => [held.instcode, held.isincode]), [[HOLCIM, HOLCIM]])
  // A parse through the sharing codec fills derived identifiers from the
  // table, learns nothing, and leaves the identity the bare parse gives -
  // the two read under one pinned clock, so an undated line dates alike.
  const pinned = new Date(Date.UTC(2026, 0, 2, 10, 15, 30))
  const line = Buffer.from('8=FIX.4.4|35=D|11=P|55=HOLN|207=XSWX|10=0|')
  const filled = new fix.FixCodec(codec.registry, { instruments, defaultSendingTime: pinned }).parseFixLine(line)
  const bare = new fix.FixCodec(codec.registry, { defaultSendingTime: pinned }).parseFixLine(line)
  assert.equal(filled.isincode, HOLCIM)
  assert.ok(filled.securityids.isDerived('isin'))
  assert.equal(filled.securityids.get('ric'), 'HOLN.S')
  assert.equal(bare.isincode, null)
  assert.equal(filled.uuid, bare.uuid)
  assert.equal(filled.hashcode, bare.hashcode)
  assert.equal(filled.intoText('|'), bare.intoText('|'))
  assert.equal(filled.cficode, null, 'a parse fills identifiers only')
  assert.equal(filled.instcode, null, 'and writes no code the table alone answers')
  assert.ok(!instruments.learn(filled), 'nothing a parse derived is learned back')
})

test('a ticker leads back to its instrument on the same market', () => {
  // Through the inverse index, gated by the market: the one instrument
  // listing the ticker on the market asked, or on any where none is asked;
  // two answering is ambiguous, and answers none.
  const instruments = new Instruments()
  assert.ok(instruments.merge(statement({ isin: HOLCIM, listings: [listing({ miccode: 'XSWX', ticker: 'HOLN' })] })))
  const code = (ticker, market) => {
    const row = instruments.getByTicker(ticker, market)
    return row === null ? null : row.crosscode
  }

  assert.equal(code('HOLN'), HOLCIM)
  assert.equal(code(' HOLN '), HOLCIM, 'trimmed')
  assert.equal(code('HOLN', 'XSWX'), HOLCIM)
  assert.equal(code('HOLN', null), HOLCIM)
  assert.equal(code('HOLN', 'XXXX'), HOLCIM, 'XXXX states no market')
  assert.equal(code('HOLN', 'XLON'), null)
  assert.equal(code('ABBN'), null)
  assert.throws(() => instruments.getByTicker('HOLN', 'TOOLONG'))
  // Two instruments listing one ticker on two markets: a market resolves
  // to its listing, none resolves to neither.
  assert.ok(instruments.merge(statement({ isin: NOVARTIS, listings: [listing({ miccode: 'XLON', ticker: 'HOLN' })] })))
  assert.deepEqual([code('HOLN', 'XLON'), code('HOLN', 'XSWX')], [NOVARTIS, HOLCIM])
  assert.equal(code('HOLN'), null, 'ambiguous')
  assert.equal(instruments.remove(NOVARTIS).crosscode, NOVARTIS)
  assert.equal(code('HOLN'), HOLCIM)
  instruments.clear()
  assert.equal(code('HOLN'), null)
})

test('the process instruments are the store the environment names, shared with the codec the environment names', (t) => {
  // Process-wide state, so it is driven in a process of its own whose
  // environment names a scratch store, never the real home.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-instruments-env-'))
  t.after(() => fs.rmSync(home, { recursive: true, force: true }))
  const store = path.join(home, 'instruments') + path.sep
  const script = `
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const path = require('node:path')
    const { Instruments, fix } = require(process.argv[1])
    const instruments = Instruments.fromEnv()
    // The seed lies under the store, so an empty first run holds it, clean.
    assert.equal(instruments.length, 208, 'an empty first run is the seed')
    assert.equal(instruments.isDirty, false)
    assert.ok(Instruments.fromEnv().equals(instruments), 'resolved once')
    assert.throws(() => Instruments.installEnv(new Instruments()), /already resolved/)
    const ZERO = '00000000-0000-0000-0000-000000000000'
    assert.ok(instruments.merge({
      uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false,
      isin: '${HOLCIM}', securityids: new Map([['wkn', '869898']]),
    }))
    assert.equal(instruments.commit().writtenRows, 208, "the seed's instruments ride the first commit")
    assert.ok(fs.existsSync(path.join(process.argv[2], 'part-0.arrows')))
    const codec = fix.FixCodec.fromEnv()
    assert.ok(codec.instruments.equals(instruments))
    const own = new Instruments()
    assert.ok(fix.FixCodec.fromEnv({ instruments: own }).instruments.equals(own), 'a stated pin stands')
    assert.equal(new fix.FixCodec(fix.FixRegistry.fromEnv()).instruments, null, 'a codec built by hand attaches none')
    console.log('ok')
  `
  const output = execFileSync(
    process.execPath,
    ['-e', script, require.resolve('yggdryl'), store],
    { encoding: 'utf8', env: { ...process.env, HOME: home, USERPROFILE: home, YGGDRYL_INSTRUMENTS_URI: store } },
  )
  assert.equal(output.trim(), 'ok')
  // Installed first, the caller's own table is the one every later
  // `fromEnv` answers.
  const installed = `
    const assert = require('node:assert/strict')
    const { Instruments } = require(process.argv[1])
    const own = new Instruments(4)
    Instruments.installEnv(own)
    assert.ok(Instruments.fromEnv().equals(own))
    assert.throws(() => Instruments.installEnv(new Instruments()), /already resolved/)
    console.log('ok')
  `
  const second = execFileSync(
    process.execPath,
    ['-e', installed, require.resolve('yggdryl')],
    { encoding: 'utf8', env: { ...process.env, HOME: home, USERPROFILE: home, YGGDRYL_INSTRUMENTS_URI: store } },
  )
  assert.equal(second.trim(), 'ok')
})
