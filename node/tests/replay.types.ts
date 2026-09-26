import { graph, type BookEvent, type MarketData } from '..'
import {
  bookAt,
  bookJson,
  booksBetween,
  booksJson,
  createReplayServer,
  indexBooks,
  loadSource,
  main,
  parseArgs,
  refusalText,
  rowsOf,
  synthetic,
  toJson,
  walk,
  type BookJson,
  type LimitJson,
  type Pairs,
  type ReplayServer,
  type Rows,
  type Source,
  type SourcesAnswer,
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
// @ts-expect-error an instant is a bigint, never a number
booksBetween(index, 'ALPHA', 1, 2)

// A served book: text for every instant, decimal and hash; its sides carry
// their limits, depth and length.
const book: BookJson = bookJson(walked[0])
const at: string = book.currunix
const hash: string = book.stableHash
const depth: string | null = book.bid.depth['1']
const limit: LimitJson = book.ask.limits[0]
const price: string | null = limit.price
const uuids: string[] = limit.uuids
const tick: boolean = book.isTick
const imbalance: string | null = book.imbalance['10']
const served: BookJson[] = booksJson(synthetic.books())
const rows: Rows = rowsOf(graph.MarketData.arrowReader(operations))
const firstColumn: string = rows.columns[0].name
// A map crosses as its [key, value] pairs.
const pairs: Pairs = [['7117', 'b'], ['__proto__', 'p']]
const listed: SourcesAnswer = {
  snapshotMillis: 0,
  global: false,
  sources: [{ id: 'synthetic', kind: 'synthetic', name: 'synthetic', operations: 23, symbols: ['ALPHA'], refusals: [] }],
}
// @ts-expect-error a source's kind is one of the four the loader reads
const unknownKind: SourcesAnswer['sources'][number]['kind'] = 'csv'

async function serve(): Promise<void> {
  const service: ReplayServer = createReplayServer({ sources: [source], snapshotMillis: 0 })
  const byId: ReplayServer = createReplayServer({ sources: new Map([['synthetic', source]]) })
  const url: string = await service.listen(0)
  await service.close()
  await byId.listen()
  const cli = await main(['synthetic', '--port', '0'])
  const cliUrl: string = cli.url
  await cli.close()
  void [url, cliUrl]
}

const args = parseArgs(['synthetic', '--global'])
const port: number = args.port
const text: string = toJson({ at: 1n }) + refusalText(new Error('x'))
const synthetic2: MarketData[] = synthetic()
const symbols: readonly string[] = synthetic.SYMBOLS

void [refusals, window, standing, at, hash, depth, price, uuids, tick, imbalance, served, firstColumn, pairs, listed, unknownKind]
void [serve, port, text, synthetic2, symbols]
