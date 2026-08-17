import {
  Value,
  codec,
  json,
  toml,
  yaml,
  type CodecOptions,
  type CodecTimeUnit,
  type SingleDocumentCodec,
  type TomlCodecFormat,
} from '..'
import { Buffer } from 'node:buffer'
import { Readable, Writable } from 'node:stream'
import { ReadableStream, WritableStream } from 'node:stream/web'
import { pathToFileURL } from 'node:url'

class Order {
  id = 0
}

const options: CodecOptions = {
  format: 'yaml',
  maxDepth: 32,
}

const unit: CodecTimeUnit = 'us'
const at: Value = Value.timestamp(1700000000000000n, unit, 'UTC')
const naive: Value = Value.timestamp(1700000000000n, 'ms')
const on: Value = Value.date(19723)
const sinceMidnight: Value = Value.time(45296000000n, unit)
const took: Value = Value.duration(90n, 's')
const price: Value = Value.decimal(-1050n, 2)
const kind: string = at.kind
const count: bigint | null = at.count
const zone: string | null = at.zone
const unscaled: bigint | null = price.unscaled
const scale: number | null = price.scale
const same: boolean = took.equals(Value.duration(90000n, 'ms'))
const pivot: Value = Value.fromJs(new Set([1, 2]), { maxDepth: 8 })
const lowered: unknown = pivot.asJs()

const bytes: Buffer = yaml.dumps(new Order(), options)
const tomlFormat: TomlCodecFormat = 'toml'
const tomlFacade: SingleDocumentCodec = toml
const tomlBytes: Buffer = toml.dumps(new Order(), { format: tomlFormat })
const tomlOrder: Order = toml.loads<Order>(tomlBytes)
const order: Order = yaml.loads<Order>(bytes, options)
const inferred: Order = codec.from<Order>('order.yml', options)
const inferredToml: Order = codec.from<Order>('order.toml', { format: 'toml' })
const rows: AsyncIterable<Order> = json.loadAllStream<Order>(
  (async function* () {
    yield bytes
  })(),
  options,
)
const file = pathToFileURL('order.yml')
const fromUrl: Order = codec.from<Order>(file, options)
const bufferedRows: Order[] = codec.from<Order>(Buffer.from('{}\n'), {
  format: 'jsonl',
})
const dashedRows: Order[] = codec.from<Order>(Buffer.from('{}\n'), {
  format: 'json-lines',
})
const jsonLines: Buffer = codec.into([order], { format: 'json_lines' })
const streamedRows: AsyncIterable<Order> = codec.fromStream<Order>(
  (async function* () {
    yield Buffer.from('{}\n')
  })(),
  { format: 'ndjson' },
)
yaml.dump(order, file, options)
codec.into(order, file, options)
codec.into([order], file, { format: 'json-lines' })

const loadedFromStream: Promise<Order> = json.load<Order>(
  Readable.from(['{}']),
  options,
)
const loadedTomlFromStream: Promise<Order> = toml.load<Order>(
  Readable.from(['id = 1\n']),
)
const loadedTomlFromWebStream: Promise<Order> = toml.load<Order>(
  new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(Buffer.from('id = 1\n'))
      controller.close()
    },
  }),
)
const loadedRows: AsyncIterable<Order> = json.loadAll<Order>(
  Readable.from(['{}\n']),
  options,
)
const inferredStream: Promise<Order> = codec.from<Order>(
  Readable.from(['id: 1\n']),
  options,
)
const inferredRowStream: AsyncIterable<Order> = codec.from<Order>(
  Readable.from(['{}\n']),
  { format: 'jsonl' },
)
const nodeWrite: Promise<void> = yaml.dump(
  order,
  new Writable({ write(_chunk, _encoding, done) { done() } }),
  options,
)
const webWrite: Promise<void> = json.dump(
  order,
  new WritableStream<Uint8Array>({ write() {} }),
)
const genericWrite: Promise<void> = codec.into(
  [order],
  new Writable({ write(_chunk, _encoding, done) { done() } }),
  { format: 'jsonl' },
)
const tomlWrite: Promise<void> = toml.dump(
  order,
  new Writable({ write(_chunk, _encoding, done) { done() } }),
)
const asyncRowsWrite: Promise<void> = json.dumpAll(
  (async function* () { yield order })(),
  new Writable({ write(_chunk, _encoding, done) { done() } }),
)
const dataViewOrder: Order = json.loads<Order>(new DataView(new ArrayBuffer(2)))
const sharedOrder: Order = yaml.loads<Order>(new SharedArrayBuffer(2))

// TOML is deliberately single-document at the JavaScript boundary.
// @ts-expect-error no TOML multi-document decode API
toml.loadsAll(tomlBytes)
// @ts-expect-error no TOML multi-document encode API
toml.dumpAll([order])

void order
void inferred
void inferredToml
void at
void naive
void on
void sinceMidnight
void took
void price
void kind
void count
void zone
void unscaled
void scale
void same
void pivot
void lowered
void rows
void fromUrl
void bufferedRows
void dashedRows
void jsonLines
void streamedRows
void loadedFromStream
void loadedTomlFromStream
void loadedTomlFromWebStream
void loadedRows
void inferredStream
void inferredRowStream
void nodeWrite
void webWrite
void genericWrite
void tomlWrite
void asyncRowsWrite
void dataViewOrder
void sharedOrder
void tomlFacade
void tomlOrder
