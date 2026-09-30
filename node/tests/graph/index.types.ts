import {
  BatchReader,
  Field,
  FieldPath,
  Plan,
  Scalar,
  Timezone,
  enums,
  graph,
  type BookEvent,
  type BookLimit,
  type Candle,
  type CandleOptions,
  type CandleReading,
  type ExecutionEvent,
  type Identifiers,
  type MarketData,
  type MarketItem,
  type Order,
  type OrderEvent,
} from '../..'

// The operation leaves take their named facts as one plain object, or the
// native record; a dated one takes its instant first and its `book` among
// the facts.
const order: Order = new graph.Order({ crosscode: 'O-1', price: '101', ticker: undefined })
const bare: Order = new graph.Order()
const event: OrderEvent = new graph.OrderEvent(1n, {
  crosscode: 'O-1',
  side: 'BUYS',
  bidpx: '101',
  book: new graph.BookRef({ action: '0' }),
})
const fromRecord: OrderEvent = new graph.OrderEvent(1, Scalar.from({ crosscode: 'O-1' }))
const dated: OrderEvent = order.at(1n)
const undated: Order = event.intoElement()
const execution: ExecutionEvent = new graph.ExecutionEvent(2n, { crosscode: 'O-1', lastqty: 1 })
const restored: OrderEvent = graph.OrderEvent.fromJSON(event.toJSON())

// Every fact is typed as the addon answers it.
const curruuid: string = event.curruuid
const currunix: bigint = event.currunix
const seqnum: number = event.seqnum
const creaunix: bigint | null = event.creaunix
const price: string | null = event.price
const side: string = event.side
const identifiers: Identifiers = event.identifiers
const partyids: Identifiers = event.partyids
const securityids: Identifiers = event.securityids
const bidpx: string | null = event.bidpx
const bidccy: string | null = event.bidccy
const isincode: string | null = event.isincode
const fxrates: Record<string, string> = event.fxrates
const marketdatakind: string = event.marketdatakind
const kind: string = event.kind
const followed: OrderEvent | null = event.withPrevious(dated)

// `MarketData` wraps any leaf, and streams through the lifted doors.
const data: MarketData = new graph.MarketData(event)
const leaf: OrderEvent | null = data.asOrderEvent()
const kinds: string[] = graph.MarketData.kinds()
const field: Field = graph.MarketData.field()
const items: MarketItem[] = [order, event, data, execution]
const reader: BatchReader = graph.MarketData.arrowReader(items, 2)
const rows: MarketData[] = [...graph.MarketData.fromArrowReader(reader)]
const marketKinds: readonly string[] = enums.marketKinds

// A book folds any iterable of items, and the two walks pull theirs lazily.
const book: BookEvent = new graph.BookEvent(1n, 'IBM').withOperations(new Set(items))
const alive: MarketData[] = book.alive()
const deltas: MarketData[] = book.deltas()
const executions: ExecutionEvent[] = book.executions()
const books: BookEvent[] = [...new graph.BookIterator(items, 0)]
const walked: MarketData[] = [...new graph.EventIterator(items, true, 5n)]
const snapshotNs: bigint | null = new graph.EventIterator([]).snapshotNs

// A book reads each side's limits, best first, and its depth; and its two
// bests.
const limits: BookLimit[] = book.limits('BUYS')
const limitPrice: string | null = limits[0].price
const limitQuantity: string = limits[0].quantity
const limitUuids: string[] = limits[0].uuids
const limitTradable: boolean = limits[0].tradable
const depth: string | null = book.depth('SELL', 2)
const bestPrice: string | null = book.bestPrice(1)
const bestQuantity: string | null = book.bestQuantity('SELL')
const locked: boolean = book.isLocked
const spread: string | null = book.spread
const imbalance: string | null = book.imbalance(1)

// Candles bucket a sorted stream of books by an interval - a spelling, a
// `CandleOptions` or a count of nanoseconds - in a zone; each reading is
// four decimals as text or null, an instant a bigint, a count a number.
const candleOptions: CandleOptions = new graph.CandleOptions('1m', 'Europe/Zurich')
const byNanos: CandleOptions = new graph.CandleOptions(60_000_000_000n)
const byZone: CandleOptions = candleOptions.withTimezone(new Timezone('UTC'))
const interval: bigint = candleOptions.interval
const zone: Timezone = candleOptions.timezone
const spelling: string = candleOptions.spelling
const candles: Candle[] = graph.candles(books, '1m')
const walkedCandles: Candle[] = [...new graph.CandleIterator(books, candleOptions)]
const walkOptions: CandleOptions = new graph.CandleIterator(new graph.BookIterator(items), 60_000_000_000).options
const heldCandles: Candle[] = graph.candles(walked, '1m')
const candle: Candle = candles[0]
const bid: CandleReading | null = candle.bid
const open: string | undefined = bid?.open
const start: bigint = candle.start
const ticker: string | null = candle.ticker
const bidqty: string | null = candle.bidqty
const volume: string = candle.volume
const bookCount: number = candle.books
const candleField: Field = graph.Candle.field()
const candleScalar: Scalar = candle.intoScalar()
const restoredCandle: Candle = graph.Candle.fromScalar(candleScalar)
const fromObject: Candle = graph.Candle.fromScalar({ crosscode: 'ACME', start: 0n, end: 1n })
const fromJson: Candle = graph.Candle.fromJSON(candle.toJSON())
const sameCandle: boolean = candle.equals(restoredCandle)
// @ts-expect-error the candle walk needs its options
void new graph.CandleIterator(books)
// @ts-expect-error the candle walk folds books, not items
void new graph.CandleIterator(items, '1m')

// A named view is one plan, applied to whatever `BatchReader.from` takes.
const viewPlan: Plan = graph.MarketData.plan('orders', ["identifiers['fix:clordid'].value as clordid", new FieldPath('ticker')])
const lifecyclePlan: Plan = graph.MarketData.plan('lifecycle', [], '10:1:C-1')
const view: BatchReader = graph.MarketData.applyView('trades', reader)
const liftedView: BatchReader = graph.MarketData.applyView('orders', new Uint8Array(), ["securityids['base:isin'].value as isin"])
const marketViews: readonly string[] = enums.marketViews
// @ts-expect-error a lift is a path or its text
graph.MarketData.plan('orders', [1])
// @ts-expect-error the book walk takes no third argument
void new graph.BookIterator(items, 0, false)

// @ts-expect-error the namespace is frozen
graph.Order = graph.Quote
// @ts-expect-error an event needs its instant
void new graph.OrderEvent()

void bare
void fromRecord
void undated
void restored
void curruuid
void currunix
void seqnum
void creaunix
void price
void side
void identifiers
void partyids
void securityids
void [bidpx, bidccy, isincode, fxrates, marketdatakind]
void kind
void followed
void leaf
void kinds
void field
void rows
void marketKinds
void [alive, deltas, executions]
void books
void walked
void snapshotNs
void [limitPrice, limitQuantity, limitUuids, limitTradable, depth, bestPrice, bestQuantity, locked, spread, imbalance]
void [byNanos, byZone, interval, zone, spelling, walkedCandles, walkOptions, heldCandles]
void [open, start, ticker, bidqty, volume, bookCount, candleField, fromObject, fromJson, sameCandle]
void [viewPlan, lifecyclePlan, view, liftedView, marketViews]
