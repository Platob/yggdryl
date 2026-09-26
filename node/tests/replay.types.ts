import { graph, type BookEvent, type MarketData } from '..'
import {
  DEFAULT_STATE_DIR,
  bookAt,
  bookJson,
  booksBetween,
  booksJson,
  createReplayServer,
  eventJson,
  indexBooks,
  leafFromJson,
  leafJson,
  leavesJson,
  loadSource,
  main,
  merged,
  openScenarios,
  parseArgs,
  refusalText,
  rerun,
  rowsOf,
  synthetic,
  toJson,
  walk,
  type BookJson,
  type EventJson,
  type FieldAnswer,
  type LeafJson,
  type Pairs,
  type ReplayServer,
  type Rows,
  type Scenario,
  type ScenariosAnswer,
  type Source,
} from 'yggdryl/replay'

// A source's operations are native values; its symbols and refusals text.
const source: Source = loadSource('synthetic')
const operations: MarketData[] = source.operations
const refusals: string[] = loadSource('capture.log', { sendingTime: 1_704_190_530_000_000_000n, registry: null }).refusals
loadSource('capture.log', { sendingTime: '2024-01-02T10:15:30Z', rowheader: '^(?P<x>\\d+) ' })
// @ts-expect-error a source is named by text
loadSource(42)

// The walk answers native books; the index reads bigint instants.
const walked: BookEvent[] = walk(operations, { snapshotMillis: 2, global: false })
const index = indexBooks(walked)
const window: BookEvent[] = booksBetween(index, 'ALPHA', synthetic.T0, synthetic.T0 + 1n)
const standing: BookEvent | null = bookAt(index, 'ALPHA')
const stream = merged(operations, [new graph.OrderEvent(synthetic.T0, { crosscode: 'X' })])
// @ts-expect-error an instant is a bigint, never a number
booksBetween(index, 'ALPHA', 1, 2)

// A served book: text for every instant, decimal and hash; its sides carry
// their limits, depth and length.
const book: BookJson = bookJson(walked[0])
const at: string = book.currunix
const hash: string = book.stableHash
const depth: string | null = book.bid.depth['1']
const price: string | null = book.ask.limits[0].price
const tick: boolean = book.isTick
const imbalance: string | null = book.imbalance['10']
const served: BookJson[] = booksJson(synthetic.books())
const rows: Rows = rowsOf(graph.MarketData.arrowReader(operations))
const firstColumn: string = rows.columns[0].name

// An inserted event is built by the native constructor; a scenario holds its JSON.
const leaf = leafFromJson({ kind: 'order_event', currunix: '1700000000000000000', facts: { price: '82.5' } })
const event: EventJson = eventJson(leaf)
const native: string = event.native
const kindOf: string = leafJson(leaf).kind
const leaves: LeafJson[] = leavesJson(operations)
// A map crosses as its [key, value] pairs, both ways.
const pairs: Pairs = [['7117', 'b'], ['__proto__', 'p']]
leafFromJson({ kind: 'order_event', currunix: '1700000000000000000', facts: { metadata: pairs } })
// @ts-expect-error a kind is one of the six leaves a form can state
leafFromJson({ kind: 'book_event' })

async function scenarios(): Promise<void> {
  const store = await openScenarios('state')
  const scenario: Scenario = store.save({ name: 'what-if', events: [event] })
  const names: string[] = store.list()
  const loaded = store.load('what-if')
  if (loaded !== null) {
    const again: MarketData[] = loaded.operations
    // What the load decoded is handed on, never decoded again.
    store.save(loaded.scenario, again)
    await rerun(operations, loaded.scenario, {}, again)
  }
  const held: Scenario | null = store.read('what-if')
  // The routes' answers: the insert form's vocabulary, and every scenario whole.
  const field: FieldAnswer = { columns: rows.columns, kinds: ['order_event', 'execution'] }
  // @ts-expect-error a kind is one of the six an inserted event may name
  const unknownKind: FieldAnswer = { columns: [], kinds: ['book_event'] }
  const listed: ScenariosAnswer = { scenarios: [scenario] }
  void [field, unknownKind, listed]
  const stored: EventJson = store.insert(held ?? { name: 'what-if', events: [] }, leaf)
  const removed: boolean = store.remove('what-if')
  const { books, from } = await rerun(operations, scenario, { global: true })
  const first: BookEvent | undefined = books[0]
  const since: bigint | null = from
}

async function serve(): Promise<void> {
  const service: ReplayServer = createReplayServer({ sources: [source], stateDir: 'state', snapshotMillis: 0 })
  const byId: ReplayServer = createReplayServer({ sources: new Map([['synthetic', source]]) })
  const url: string = await service.listen(0)
  const kept: string = service.stateDir
  await service.close()
  await byId.listen()
  const cli = await main(['synthetic', '--port', '0'])
  const cliUrl: string = cli.url
  await cli.close()
}

const args = parseArgs(['synthetic', '--global'])
const port: number = args.port
const state: string = DEFAULT_STATE_DIR
const text: string = toJson({ at: 1n }) + refusalText(new Error('x'))
const synthetic2: MarketData[] = synthetic()
const symbols: readonly string[] = synthetic.SYMBOLS

void [refusals, window, standing, stream, at, hash, depth, price, tick, imbalance, served, firstColumn, native, kindOf, leaves]
void [scenarios, serve, port, state, text, synthetic2, symbols]
