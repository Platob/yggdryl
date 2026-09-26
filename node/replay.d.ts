// `yggdryl/replay`: the order book replay service over the native package,
// and the one JSON renderer between them. Every market fact it serves is the
// native package's; instants cross as the decimal text of nanoseconds and
// decimals as their exact text.

import type { Server } from 'node:http'

import type { BatchReader, BookEvent, FixRegistry, MarketData, MarketItem, Scalar } from './binding'

/**
 * A value as the service serves it. A map column is an array of
 * `[key, value]` pairs in the native key order (`Pairs`), never an object:
 * an object would move an integer-like key first and lose a `__proto__` one.
 */
export type Json = string | number | boolean | null | Json[] | { [key: string]: Json }
/** A map column's value: its `[key, value]` pairs, in the native key order. */
export type Pairs = [Json, Json][]
/** One row, keyed by column name in column order. */
export type Row = { [column: string]: Json }
/** One column as the native field describes it. */
export type Column = { name: string; dtype: string; nullable: boolean }
/** A stream's rows: the columns its field states, then one object per row. */
export type Rows = { columns: Column[]; rows: Row[] }

/** One reading per served level count. */
export type PerDepth = { '1': string | null; '5': string | null; '10': string | null }
/** One limit of a side, as `BookSide.limits` answers it. */
export type LimitJson = { price: string | null; quantity: string; uuids: string[] }
/** A book side's row, beside its native depth per level count and its length. */
export type SideJson = Row & { limits: LimitJson[]; live: Row[]; deltas: Row[]; depth: PerDepth; length: number }
/**
 * A book's `marketdata` row, its sides in place of the two lanes, and the
 * native readings no column holds.
 */
export type BookJson = Row & {
  kind: 'book_event'
  currunix: string
  snapunix: string | null
  curruuid: string
  crosscode: string
  spread: string | null
  crossed: boolean | null
  locked: boolean | null
  executions: Row[] | null
  bid: SideJson
  ask: SideJson
  imbalance: PerDepth
  bboMidpoint: string | null
  medianQuantity: string | null
  stableHash: string
  isTick: boolean
}
/** `GET /api/sources`: the walk a request that names none reads, and every loaded source. */
export interface SourcesAnswer {
  snapshotMillis: number
  global: boolean
  sources: { id: string; kind: Source['kind']; name: string; operations: number; symbols: string[]; refusals: string[] }[]
}

/** A loaded source: its operations as the core answered them. */
export interface Source {
  id: string
  kind: 'synthetic' | 'arrow' | 'parquet' | 'fix'
  name: string
  operations: MarketData[]
  /** The symbols the per-symbol walk emits books for, sorted. */
  symbols: string[]
  /** Every message a capture refused, verbatim, in order. */
  refusals: string[]
}

export interface LoadOptions {
  /** A FIX capture's row header; the ULBridge one by default. */
  rowheader?: string
  /** The clock an undated FIX message takes; 2024-01-02T10:15:30Z by default, `null` for UTC now. */
  sendingTime?: bigint | string | Date | Scalar | null
  /** The FIX dictionary, or the folder it is stored in; the process default when `null`. */
  registry?: FixRegistry | string | null
  /** The zone an offset-free row-header instant is read in; UTC by default. */
  timezone?: string
  /** The source's identifier; its file name by default. */
  id?: string
}

/** Load `'synthetic'`, a `.arrow`/`.ipc`/`.feather`/`.parquet` file, or a FIX capture. */
export declare function loadSource(spec: string, options?: LoadOptions): Source

export interface WalkOptions {
  snapshotMillis?: number
  global?: boolean
}
/** Books by symbol and instant, as `indexBooks` builds them. */
export interface BookIndex {
  all: { books: BookEvent[]; instants: (bigint | null)[] }
  bySymbol: Map<string, { books: BookEvent[]; instants: (bigint | null)[] }>
}

/** Every book the native walk answers, in walk order. */
export declare function walk(operations: Iterable<MarketItem>, options?: WalkOptions): BookEvent[]
export declare function indexBooks(books: readonly BookEvent[]): BookIndex
/** The books of `symbol` (every symbol when omitted) in `[from, to]`. */
export declare function booksBetween(index: BookIndex, symbol?: string, from?: bigint, to?: bigint): BookEvent[]
/** The book of `symbol` standing at `at`: the latest at or before it; the last when omitted. */
export declare function bookAt(index: BookIndex, symbol: string, at?: bigint): BookEvent | null
/** A native stream's rows, each column read by its Arrow type. Drains the reader. */
export declare function rowsOf(reader: BatchReader): Rows
export declare function bookJson(book: BookEvent): BookJson
export declare function booksJson(books: readonly BookEvent[]): BookJson[]
/** A refusal's message, verbatim. */
export declare function refusalText(error: unknown): string
/** `JSON.stringify` with every bigint as its decimal text. */
export declare function toJson(value: unknown, space?: string | number): string

export interface ReplayServerOptions {
  /** Loaded sources: a `Map` keyed by id, or any iterable of them. */
  sources?: ReadonlyMap<string, Source> | Iterable<Source>
  /** The component and the page; the package's own `web/` by default. */
  webDir?: string
  /** The grid a request that names none walks at; 0 by default. */
  snapshotMillis?: number
  /** Whether a request that says nothing walks one consolidated book; `false` by default. */
  global?: boolean
}
export interface ReplayServer {
  readonly server: Server
  /** Serve on `port` (an ephemeral one by default) and `host` (127.0.0.1), answering the base URL. */
  listen(port?: number, host?: string): Promise<string>
  /** Stop serving, ending every open connection and stream. */
  close(): Promise<void>
}
/**
 * The replay service: `GET /api/sources` and the book stream
 * `GET /api/sources/:source/books` (server-sent `book` events, then `end`
 * with `{ count }`), the component under `/web/` and the page at
 * `/web/app/`, which names its files relative to
 * itself; `/`, `/app`, `/app/` and `/web/app` answer 308 to `/web/app/`, the
 * query kept.
 */
export declare function createReplayServer(options?: ReplayServerOptions): ReplayServer

/** The deterministic synthetic scenario over `ALPHA` and `BETA`. */
export interface Synthetic {
  (): MarketData[]
  /** The books the native walk answers over it. */
  books(snapshotMillis?: number, global?: boolean): BookEvent[]
  readonly T0: bigint
  readonly SYMBOLS: readonly string[]
}
export declare const synthetic: Synthetic

/** The command line's arguments, every default applied. */
export interface Args {
  source: string
  port: number
  snapshotMillis: number
  global: boolean
  rowheader: string | undefined
  sendingTime: string | undefined
  registry: string | undefined
  web: string | undefined
}
export declare function parseArgs(argv: readonly string[]): Args
/** Run the command line: load, serve, print the URL. */
export declare function main(argv?: readonly string[]): Promise<{ url: string; close(): Promise<void> }>
