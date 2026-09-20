import {
  BatchReader,
  Field,
  Scalar,
  fix,
  market,
  type Book,
  type BookIterator,
  type Books,
  type Execution,
  type Executions,
  type FixCodec,
  type FixEventView,
  type FixMsg,
  type FixValueInput,
  type Market,
  type MarketLaneView,
  type MarketLevelView,
  type MarketPartyView,
  type MarketSymbol,
  type Order,
  type Orders,
  type Quote,
  type Quotes,
  type Statements,
  type Trade,
  type Trades,
} from '../..'

declare const codec: FixCodec
declare const message: FixMsg
declare const row: Scalar
declare const field: Field

// The namespace holds the five products and their streams, and nothing a
// caller builds: a product is read out of messages or out of a row.
const namespace: Market = market
const orderClass: typeof market.Order = market.Order
void namespace
void orderClass

// @ts-expect-error a product has no constructor
new market.Order()
// @ts-expect-error a stream is what a codec door answers
new market.Orders()
// @ts-expect-error a product has no constructor
new market.Book()

// Every door takes an iterable of messages and answers a lazy stream of its
// product; the book door declares its depth and its grid step.
const orders: Orders = codec.orders([message])
const executions: Executions = codec.executions([message])
const trades: Trades = codec.trades([message])
const quotes: Quotes = codec.quotes([message])
const books: Books = codec.books([message], 5, 1_000_000_000n)
const booksByNumber: Books = codec.books(codec.lifecycle([message]), 5, 1_000_000_000)
const booksPerInstant: Books = codec.books([message], 5)
const statements: Statements = codec.statements([message])
const walkedTwice: Orders = codec.orders(codec.lifecycle([message]))

void booksByNumber
void booksPerInstant
void walkedTwice

// @ts-expect-error a door takes an iterable of messages, never one message
codec.orders(message)
// @ts-expect-error a door takes messages, never lines
codec.trades([Buffer.from('8=FIX.4.4|35=8|10=0|')])
// @ts-expect-error a book declares its depth
codec.books([message])
// @ts-expect-error a depth is a number
codec.books([message], 5n, 1_000_000_000n)
// @ts-expect-error the statements door takes messages, never products
codec.statements([order])

// A stream is its own iterator, and each step is the product.
const firstOrder: IteratorResult<Order> = orders.next()
const everyOrder: Order[] = [...orders]
const firstExecution: IteratorResult<Execution> = executions.next()
const everyTrade: Trade[] = [...trades]
const everyQuote: Quote[] = [...quotes]
const everyBook: Book[] = [...books]
const sameStream: Orders = orders[Symbol.iterator]()
const firstStatement: IteratorResult<Order | Quote | Execution> = statements.next()
const everyStatement: Array<Order | Quote | Execution> = [...statements]
const sameStatements: Statements = statements[Symbol.iterator]()

void firstOrder
void firstExecution
void sameStream
void firstStatement
void sameStatements
// @ts-expect-error a statement is never a trade or a book
const noTrade: Trade = everyStatement[0]
void noTrade

declare const order: Order
declare const execution: Execution
declare const trade: Trade
declare const quote: Quote
declare const book: Book

// The sixteen event facts cross as a message's do.
const curruuid: string = order.curruuid
const crossuuid: string = order.crossuuid
const crosscode: string = order.crosscode
const currhashcode: bigint = order.currhashcode
const crosshashcode: bigint = order.crosshashcode
const identifiers: Record<string, string> = order.identifiers
const parentuuids: string[] = order.parentuuids
const srcuuids: string[] = order.srcuuids
const currunix: bigint = order.currunix
const state: string = order.state
const seqnum: number = order.seqnum
const creaunix: bigint | null = order.creaunix
const expirunix: bigint | null = order.expirunix
const prevunix: bigint | null = order.prevunix
const prevuuid: string | null = order.prevuuid
const snapunix: bigint | null = order.snapunix
const event: FixEventView = order.event()
// The one reading of liveness, on every product.
const alive: boolean[] = [order.isAlive, execution.isAlive, trade.isAlive, quote.isAlive, book.isAlive]

void [curruuid, crossuuid, crosscode, currhashcode, crosshashcode, identifiers, parentuuids,
  srcuuids, currunix, state, seqnum, creaunix, expirunix, prevunix, prevuuid, snapunix, event, alive]

// @ts-expect-error a fact is read, never written
order.curruuid = 'x'

// The market facts each row publishes, as the event view crosses them, and
// each product's own.
const px: string = order.px
const avgpx: string | null = order.avgpx
const qty: string = order.qty
const cumqty: string | null = order.cumqty
const leavesqty: string | null = order.leavesqty
const side: string = order.side
const currency: string = order.currency
const unit: string = order.unit
const tif: string | null = order.tif
const tradable: boolean | null = order.tradable
const symbolticker: string | null = order.symbolticker
const isincode: string | null = order.isincode
const cusipcode: string | null = order.cusipcode
const sedolcode: string | null = order.sedolcode
const bloombergcode: string | null = order.bloombergcode
const cficode: string | null = order.cficode
const miccode: string | null = order.miccode
const stoppx: string | null = order.stoppx
const ordtype: string | null = order.ordtype
// What the stated facts imply: a bare noun is a getter.
const pricing: string | null = order.pricing
const remaining: string = order.remaining
const filled: string = order.filled
const filledRatio: string | null = order.filledRatio
const isResting: boolean = order.isResting
const orderNotional: string | null = order.notional

void [px, avgpx, qty, cumqty, leavesqty, side, currency, unit, tif, tradable, symbolticker,
  isincode, cusipcode, sedolcode, bloombergcode, cficode, miccode, stoppx, ordtype, pricing,
  remaining, filled, filledRatio, isResting, orderNotional]
// @ts-expect-error a reading is read, never written
order.pricing = 'limit'
// @ts-expect-error a bare-noun reading is a getter, not a method
order.remaining()

const executionPx: string = execution.px
const executionSide: string = execution.side
const executionMic: string | null = execution.miccode
const isPartial: boolean = execution.isPartial
const completes: boolean = execution.completes
const executionNotional: string | null = execution.notional
// @ts-expect-error an execution publishes no average
execution.avgpx
// @ts-expect-error an execution publishes no stop price
execution.stoppx

const tradePx: string = trade.px
const tradedate: number | null = trade.tradedate
const settldate: number | null = trade.settldate
const parties: MarketPartyView[] = trade.parties
const party: { role: string; id: string; source: string } = parties[0]
const byRole: MarketPartyView | null = trade.partyByRole('1')
const settlementDays: number | null = trade.settlementDays
const tradeNotional: string | null = trade.notional
// @ts-expect-error a trade publishes no lanes
trade.bidpx
// @ts-expect-error a role is text
trade.partyByRole(1)

const bidpx: string | null = quote.bidpx
const bidqty: string | null = quote.bidqty
const bidcurrency: string | null = quote.bidcurrency
const bidunit: string | null = quote.bidunit
const askpx: string | null = quote.askpx
const askqty: string | null = quote.askqty
const askcurrency: string | null = quote.askcurrency
const askunit: string | null = quote.askunit
const quoteTicker: string | null = quote.symbolticker
// The lanes as lanes, and what they imply.
const bid: MarketLaneView | null = quote.bid
const ask: { px: string; qty: string } | null = quote.ask
const laneOf: MarketLaneView | null = quote.lane('Buy')
const twoSided: boolean = quote.isTwoSided
const quoteMid: string | null = quote.mid
const quoteSpread: string | null = quote.spread
// @ts-expect-error a quote publishes no one price
quote.px
// @ts-expect-error a quote publishes no side
quote.side
// @ts-expect-error a lane is taken by a side spelling
quote.lane(1)
// @ts-expect-error a quote is worth nothing at one price
quote.notional

const depth: number = book.depth
const bids: MarketLevelView[] = book.bids
const asks: MarketLevelView[] = book.asks
const level: { px: string; qty: string; count: number } = bids[0]
const bookCurrency: string = book.currency
const bookUnit: string = book.unit
// A book is a market event: the mid, what rests, the tops and the prints.
const bookPx: string = book.px
const bookQty: string = book.qty
const bookTops: Array<string | null> = [book.bidpx, book.bidqty, book.askpx, book.askqty]
const bookPrints: Array<string | null> = [book.lastpx, book.lastqty, book.avgpx, book.cumqty]
const bookTradable: boolean | null = book.tradable
const updates: number = book.updates
const bestBid: MarketLevelView | null = book.bestBid
const bestAsk: MarketLevelView | null = book.bestAsk
const levelAt: MarketLevelView | null = book.level('Sell', 0)
const bookFlags: boolean[] = [book.isTwoSided, book.isLocked, book.isCrossed]
const bookReadings: Array<string | null> = [book.mid, book.spread, book.spreadBps, book.microprice, book.imbalance]
const toDepth: string | null = book.imbalanceToDepth(3)
const sizes: string[] = [book.bidSize, book.askSize]
const counts: number[] = [book.bidCount, book.askCount]
// @ts-expect-error a book publishes no side
book.side
// @ts-expect-error a level is taken by a side spelling and an index
book.level(0, 'Buy')
// @ts-expect-error a book is worth nothing at one price
book.notional

void [executionPx, executionSide, executionMic, isPartial, completes, executionNotional, tradePx,
  tradedate, settldate, parties, party, byRole, settlementDays, tradeNotional, bidpx, bidqty,
  bidcurrency, bidunit, askpx, askqty, askcurrency, askunit, quoteTicker, bid, ask, laneOf,
  twoSided, quoteMid, quoteSpread, depth, bids, asks, level, bookCurrency, bookUnit, bookPx,
  bookQty, bookTops, bookPrints, bookTradable, updates, bestBid, bestAsk, levelAt, bookFlags,
  bookReadings, toDepth, sizes, counts]

// The symbol a book is read under: built from text, read off a product,
// or the global one.
const symbol: MarketSymbol = new market.Symbol('AAPL')
const global: MarketSymbol = market.Symbol.GLOBAL
const ofOrder: MarketSymbol = market.Symbol.of(order)
const ofBook: MarketSymbol = market.Symbol.of(book)
const symbolText: string = symbol.text
const isGlobal: boolean = symbol.isGlobal
const symbolEquals: boolean = symbol.equals(global)
const symbolString: string = symbol.toString()
void [symbol, global, ofOrder, ofBook, symbolText, isGlobal, symbolEquals, symbolString]
// @ts-expect-error a symbol is spelled as text
new market.Symbol(1)
// @ts-expect-error a symbol is read off a product, never a message
market.Symbol.of(message)
// @ts-expect-error the global symbol is one value
market.Symbol.GLOBAL = symbol

// The book iterator over any iterable of statements: a door's stream, the
// products it read, or rows read back, the depth declared, the step and the
// symbol optional.
const iterator: BookIterator = new market.BookIterator(codec.statements([message]), 5)
const gridded: BookIterator = new market.BookIterator([order, quote, execution, trade], 5, 1_000_000_000n)
const keyed: BookIterator = new market.BookIterator(codec.orders([message]), 5, 0, market.Symbol.GLOBAL)
const unkeyed: BookIterator = new market.BookIterator(everyStatement, 5, 0n, null)
const firstBook: IteratorResult<Book> = iterator.next()
const everyGridded: Book[] = [...gridded]
const sameIterator: BookIterator = iterator[Symbol.iterator]()
const iteratorDepth: number = keyed.depth
const iteratorStep: bigint = keyed.snapshotNs
const iteratorSymbol: MarketSymbol | null = keyed.symbol
void [unkeyed, firstBook, everyGridded, sameIterator, iteratorDepth, iteratorStep, iteratorSymbol]
// @ts-expect-error a book is no statement
new market.BookIterator([book], 5)
// @ts-expect-error a message is no statement
new market.BookIterator([message], 5)
// @ts-expect-error a depth is declared
new market.BookIterator([order])
// @ts-expect-error a symbol is a market.Symbol
new market.BookIterator([order], 5, 0n, 'AAPL')
// @ts-expect-error an iterator is what new answers
market.BookIterator([order], 5)

// The row doors: the row a product publishes, and the product a row states,
// the row widened as `FixMsg.fromRow` widens one.
const orderField: Field = market.Order.field()
const executionField: Field = market.Execution.field()
const tradeField: Field = market.Trade.field()
const quoteField: Field = market.Quote.field()
const bookField: Field = market.Book.field(5)
const orderRow: Scalar = order.intoRow()
const orderBack: Order = market.Order.fromRow(orderField, orderRow)
const orderFromObject: Order = market.Order.fromRow(orderField, { currunix: 0n })
const orderFromArray: Order = market.Order.fromRow(orderField, [0n])
const rowInput: FixValueInput = row
const tradeBack: Trade = market.Trade.fromRow(tradeField, rowInput)
const bookBack: Book = market.Book.fromRow(bookField, row)

void [executionField, quoteField, orderBack, orderFromObject, orderFromArray, tradeBack, bookBack]

// @ts-expect-error a book's row is declared to a depth
market.Book.field()
// @ts-expect-error a depth is a number
market.Book.field('5')
// @ts-expect-error the other rows take no depth
market.Order.field(5)
// @ts-expect-error a row is stated under a field
market.Order.fromRow(orderRow)
// @ts-expect-error a product publishes its own row: no field is named
order.intoRow(field)

// The protocol every product shares.
const same: boolean = order.equals(orderBack)
const rendered: string = order.toString()
const document: unknown = order.toJSON()
void [same, rendered, document]
// @ts-expect-error equality is between products of one kind
order.equals(trade)

// The Arrow twins take whatever `BatchReader.from` accepts and answer a
// reader of product rows.
const source: BatchReader = codec.arrowReader(fix.schema(), [message])
const orderRows: BatchReader = codec.ordersArrowReader(source)
const executionRows: BatchReader = codec.executionsArrowReader(orderRows.intoTable())
const tradeRows: BatchReader = codec.tradesArrowReader(source)
const quoteRows: BatchReader = codec.quotesArrowReader(source)
const bookRows: BatchReader = codec.booksArrowReader(source, 5, 1_000_000_000n)
const bookRowsByNumber: BatchReader = codec.booksArrowReader(source, 5, 1_000_000_000)
const bookRowsPerInstant: BatchReader = codec.booksArrowReader(source, 5)

void [orderRows, executionRows, tradeRows, quoteRows, bookRows, bookRowsByNumber, bookRowsPerInstant]

// @ts-expect-error the book twin declares its depth
codec.booksArrowReader(source)

// The writers: the two products one message states exactly, and the three
// the crate does not guess.
const single: FixMsg = fix.FixMsg.fromOrder(codec, order)
const report: FixMsg = fix.FixMsg.fromExecution(codec, execution)
const refusedQuote: never = fix.FixMsg.fromQuote(codec, quote)
const refusedTrade: never = fix.FixMsg.fromTrade(codec, trade)
const refusedBook: never = fix.FixMsg.fromBook(codec, book)

void [single, report, refusedQuote, refusedTrade, refusedBook]

// @ts-expect-error a writer takes the product it names
fix.FixMsg.fromOrder(codec, execution)
// @ts-expect-error a writer takes a codec first
fix.FixMsg.fromOrder(order, codec)

void everyOrder
void everyTrade
void everyQuote
void everyBook
