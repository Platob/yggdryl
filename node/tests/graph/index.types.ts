import {
  BatchReader,
  Field,
  FieldPath,
  Plan,
  Scalar,
  enums,
  graph,
  type BookEvent,
  type BookLimit,
  type BookSide,
  type ExecutionEvent,
  type Lane,
  type MarketData,
  type MarketItem,
  type Order,
  type OrderEvent,
  type SnapshotPartition,
} from '../..'

// The operation leaves take their named facts as one plain object, or the
// native record; a dated one takes its instant first and its `book` among
// the facts.
const order: Order = new graph.Order({ crosscode: 'O-1', price: '101', ticker: undefined })
const bare: Order = new graph.Order()
const event: OrderEvent = new graph.OrderEvent(1n, {
  crosscode: 'O-1',
  side: 'BUY',
  bid: new graph.Lane({ price: '101' }),
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
const altids: Record<string, string> = event.altids
const bid: Lane | null = event.bid
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
const bidSide: BookSide = book.bid
const partitions: SnapshotPartition[] = book.snapshotPartitions
const books: BookEvent[] = [...new graph.BookIterator(items, 0, false)]
const walked: MarketData[] = [...new graph.EventIterator(items, true, 5n)]
const snapshotNs: bigint | null = new graph.EventIterator([]).snapshotNs
const followedAltids: readonly string[] = graph.FOLLOWED_ALTIDS

// A side reads its limits, best first, and its depth; a book its two bests.
const limits: BookLimit[] = bidSide.limits
const limitPrice: string | null = limits[0].price
const limitQuantity: string = limits[0].quantity
const limitUuids: string[] = limits[0].uuids
const depth: string | null = bidSide.depth(2)
const locked: boolean = book.isLocked
const spread: string | null = book.spread
const imbalance: string | null = book.imbalance(1)

// A named view is one plan, applied to whatever `BatchReader.from` takes.
const viewPlan: Plan = graph.MarketData.plan('orders', ["securityids['ISIN'] as isin", new FieldPath('ticker')])
const lifecyclePlan: Plan = graph.MarketData.plan('lifecycle', [], 'C-1')
const view: BatchReader = graph.MarketData.applyView('trades', reader)
const liftedView: BatchReader = graph.MarketData.applyView('orders', new Uint8Array(), ["securityids['ISIN'] as isin"])
const marketViews: readonly string[] = enums.marketViews
// @ts-expect-error a lift is a path or its text
graph.MarketData.plan('orders', [1])
// @ts-expect-error a limit is read, never written
bidSide.limits = []

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
void altids
void bid
void kind
void followed
void leaf
void kinds
void field
void rows
void marketKinds
void bidSide
void partitions
void books
void walked
void snapshotNs
void followedAltids
void [limitPrice, limitQuantity, limitUuids, depth, locked, spread, imbalance]
void [viewPlan, lifecyclePlan, view, liftedView, marketViews]
