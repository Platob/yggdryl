import { BatchReader, Instruments, type FixCodec, type FixMsg, type IOResult, type OrderEvent, type Resolution } from '..'

const ZERO = '00000000-0000-0000-0000-000000000000'
const instruments: Instruments = new Instruments(8)
const unbounded: Instruments = new Instruments()
const merged: boolean = instruments.merge({
  uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, isin: 'CH0012214059',
})
const row: Record<string, unknown> | null = instruments.get('CH0012214059')
const fx: Record<string, unknown> | null = instruments.get('IF:EUR/USD')
const dirty: boolean = instruments.isDirty
const listings: Record<string, unknown>[] = instruments.listings('CH0012214059')
const listing: Record<string, unknown> | null = instruments.getListing('CH0012214059', 'XSWX')
const removedListing: Record<string, unknown> | null = instruments.removeListing('CH0012214059', 'XSWX')
const removed: Record<string, unknown> | null = instruments.remove('CH0012214059')
const count: number = instruments.length
const rowCount: number = instruments.rows
const bound: number = instruments.maxInstruments
const stream: BatchReader = instruments.intoArrowReader()
const loaded: Instruments = Instruments.fromArrowReader(stream)
const read: number = unbounded.extendFromArrowReader(loaded.intoArrowReader())
const fromUrl: Instruments = Instruments.fromUrl('instruments.arrows', 16)
const withProperties: Instruments = Instruments.fromUrl('instruments/', undefined, { media_type: 'application/vnd.apache.arrow.stream' })
const committed: IOResult = fromUrl.commit()
const fromEnv: Instruments = Instruments.fromEnv()
const seeded: Instruments = Instruments.seeded()
const seededFromUrl: Instruments = Instruments.seededFromUrl('instruments.arrows', 16, { media_type: 'application/vnd.apache.arrow.stream' })
Instruments.installEnv(instruments)
const same: boolean = instruments.equals(loaded)
const text: string = instruments.toString()
declare const codec: FixCodec
const shared: Instruments | null = codec.instruments
declare const message: FixMsg
const learned: boolean = instruments.learn(message)
const filled: boolean = instruments.fill(message)
const enriched: boolean = instruments.enrich(message)
const instcode: string | null = message.instcode
instruments.clear()
const byCode: Record<string, unknown> | null = instruments.getByCode('cusip', '037833100')
const byTicker: Record<string, unknown> | null = instruments.getByTicker('HOLN', 'XSWX')
const lookupCodes: string[] = Instruments.lookupCodes()
declare const order: OrderEvent
const resolution: Resolution = instruments.resolve(order)
const fromMessage: Resolution = instruments.resolve(message)
const matched: boolean = resolution.matched
const entry: Record<string, unknown> | null = resolution.entry
const tier: 'isin' | 'crosscode' | 'code' | 'symbology' | 'economic' | null = resolution.tier
const similarity: number | null = resolution.similarity
const why: string | null = resolution.unmatched
const codes: string[] | null = resolution.codes
const code: string | null = resolution.code
const threshold: number = instruments.economicThreshold
instruments.setEconomicThreshold(0.9)
const economic: boolean = instruments.isEconomicMatch
instruments.setEconomicMatch(true)
const origccy: string | null = order.origccy
const orderInstcode: string | null = order.instcode
const originCurrency: string = message.originCurrency

// @ts-expect-error the count is read-only
instruments.length = 2
// @ts-expect-error a key is text
instruments.get(1)
// @ts-expect-error the row count is read-only
instruments.rows = 2
// @ts-expect-error a listing names its market
instruments.getListing('CH0012214059')
// @ts-expect-error a code names its type
instruments.getByCode('037833100')
// @ts-expect-error a code names no market
instruments.getByCode('cusip', '037833100', 'XNAS')
// @ts-expect-error the threshold is set through its setter
instruments.economicThreshold = 0.9
// @ts-expect-error a resolution reads a market value
instruments.resolve({ isin: 'CH0012214059' })

void [merged, row, fx, dirty, listings, listing, removedListing, removed, count, rowCount, bound, read, fromUrl, withProperties, committed, fromEnv, seeded, seededFromUrl, same, text, shared, learned, filled, enriched, instcode, byCode, byTicker, lookupCodes, fromMessage, matched, entry, tier, similarity, why, codes, code, threshold, economic, origccy, orderInstcode, originCurrency]
