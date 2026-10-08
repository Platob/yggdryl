import { BatchReader, IsinRegistry, type FixCodec, type FixMsg, type IOResult, type IsinResolution, type OrderEvent } from '..'

const registry: IsinRegistry = new IsinRegistry(8)
const unbounded: IsinRegistry = new IsinRegistry()
const merged: boolean = registry.merge({ isin: 'CH0012214059', ric: 'HOLN.S' })
const row: Record<string, unknown> | null = registry.get('CH0012214059')
const dirty: boolean = registry.isDirty
const listings: Record<string, unknown>[] = registry.listings('CH0012214059')
const listing: Record<string, unknown> | null = registry.getListing('CH0012214059', 'XSWX')
const removedListing: Record<string, unknown> | null = registry.removeListing('CH0012214059', 'XSWX')
const removed: Record<string, unknown>[] = registry.remove('CH0012214059')
const count: number = registry.length
const rowCount: number = registry.rows
const bound: number = registry.maxInstruments
const stream: BatchReader = registry.intoArrowReader()
const loaded: IsinRegistry = IsinRegistry.fromArrowReader(stream)
const read: number = unbounded.extendFromArrowReader(loaded.intoArrowReader())
const fromUrl: IsinRegistry = IsinRegistry.fromUrl('instruments.arrows', 16)
const withProperties: IsinRegistry = IsinRegistry.fromUrl('instruments/', undefined, { media_type: 'application/vnd.apache.arrow.stream' })
const committed: IOResult = fromUrl.commit()
const fromEnv: IsinRegistry = IsinRegistry.fromEnv()
const seeded: IsinRegistry = IsinRegistry.seeded()
const seededFromUrl: IsinRegistry = IsinRegistry.seededFromUrl('instruments.arrows', 16, { media_type: 'application/vnd.apache.arrow.stream' })
IsinRegistry.installEnv(registry)
const same: boolean = registry.equals(loaded)
const text: string = registry.toString()
declare const codec: FixCodec
const shared: IsinRegistry | null = codec.isinRegistry
declare const message: FixMsg
const learned: boolean = registry.learn(message)
const filled: boolean = registry.fill(message)
const enriched: boolean = registry.enrich(message)
registry.clear()
const byCode: Record<string, unknown> | null = registry.getByCode('cusip', '037833100', 'XNAS')
const byCodeAnywhere: Record<string, unknown> | null = registry.getByCode('sedol', '0540528')
const lookupCodes: string[] = IsinRegistry.lookupCodes()
declare const order: OrderEvent
const resolution: IsinResolution = registry.resolve(order)
const fromMessage: IsinResolution = registry.resolve(message)
const matched: boolean = resolution.matched
const entry: Record<string, unknown> | null = resolution.entry
const tier: 'isin' | 'code' | 'symbology' | 'economic' | null = resolution.tier
const similarity: number | null = resolution.similarity
const why: string | null = resolution.unmatched
const isins: string[] | null = resolution.isins
const threshold: number = registry.economicThreshold
registry.setEconomicThreshold(0.9)
const economic: boolean = registry.isEconomicMatch
registry.setEconomicMatch(true)
const origccy: string | null = order.origccy
const originCurrency: string = message.originCurrency

// @ts-expect-error the count is read-only
registry.length = 2
// @ts-expect-error an ISIN is text
registry.get(1)
// @ts-expect-error the listing count is read-only
registry.rows = 2
// @ts-expect-error a listing names its market
registry.getListing('CH0012214059')
// @ts-expect-error a code names its type
registry.getByCode('037833100')
// @ts-expect-error the threshold is set through its setter
registry.economicThreshold = 0.9
// @ts-expect-error a resolution reads a market value
registry.resolve({ isin: 'CH0012214059' })

void [merged, row, dirty, listings, listing, removedListing, removed, count, rowCount, bound, read, fromUrl, withProperties, committed, fromEnv, seeded, seededFromUrl, same, text, shared, learned, filled, enriched, byCode, byCodeAnywhere, lookupCodes, fromMessage, matched, entry, tier, similarity, why, isins, threshold, economic, origccy, originCurrency]
