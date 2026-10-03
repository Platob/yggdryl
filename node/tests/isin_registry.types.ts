import { BatchReader, IsinRegistry, type FixCodec, type FixMsg } from '..'

const registry: IsinRegistry = new IsinRegistry(8)
const unbounded: IsinRegistry = new IsinRegistry()
const merged: boolean = registry.merge({ isin: 'CH0012214059', ric: 'HOLN.S' })
const row: Record<string, unknown> | null = registry.get('CH0012214059')
const byRic: Record<string, unknown> | null = registry.getByRic('HOLN.S')
const removed: Record<string, unknown> | null = registry.remove('CH0012214059')
const count: number = registry.length
const bound: number = registry.maxInstruments
const stream: BatchReader = registry.intoArrowReader()
const loaded: IsinRegistry = IsinRegistry.fromArrowReader(stream)
const read: number = unbounded.extendFromArrowReader(loaded.intoArrowReader())
const fromHandle: IsinRegistry = IsinRegistry.fromHandle('instruments.arrow', 16)
const same: boolean = registry.equals(loaded)
const text: string = registry.toString()
declare const codec: FixCodec
const shared: IsinRegistry | null = codec.isinRegistry
declare const message: FixMsg
const learned: boolean = registry.learn(message)
const filled: boolean = registry.fill(message)
const enriched: boolean = registry.enrich(message)
registry.clear()

// @ts-expect-error the count is read-only
registry.length = 2
// @ts-expect-error an ISIN is text
registry.get(1)

void [merged, row, byRic, removed, count, bound, read, fromHandle, same, text, shared, learned, filled, enriched]
