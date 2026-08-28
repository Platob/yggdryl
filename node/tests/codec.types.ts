import {
  Field,
  Timezone,
  Scalar,
  codec,
  json,
  toml,
  yaml,
  type CodecOptions,
  type CodecTimeUnit,
  type SingleDocumentCodec,
  type TimezoneInput,
  type TomlCodecFormat,
} from '..'
import { Buffer } from 'node:buffer'
import { Int32, vectorFromArray, tableFromArrays } from 'apache-arrow'
import { Readable, Writable } from 'node:stream'
import { ReadableStream, WritableStream } from 'node:stream/web'
import { pathToFileURL } from 'node:url'

class Order {
  id = 0
}

const options: CodecOptions = {
  format: 'yaml',
  maxDepth: 32,
  maxInputBytes: 1 << 20,
  maxNodes: 10_000,
  maxDocuments: 16,
  indent: 2,
}
const compactOptions: CodecOptions = { indent: null, maxDepth: null }
const tabbedOptions: CodecOptions = { indent: '\t', maxInputBytes: null }
void compactOptions
void tabbedOptions

const unit: CodecTimeUnit = 'us'
const utc: TimezoneInput = Timezone.UTC
const at: Scalar = Scalar.datetime(1700000000000000n, unit, utc)
const naive: Scalar = Scalar.datetime(1700000000000n, 'ms')
const on: Scalar = Scalar.date(19723, 'd', 'NAIVE')
const wideDate: Scalar = Scalar.date(1700000000000n, 'ms', Timezone.from('NAIVE'))
const sinceMidnight: Scalar = Scalar.time(45296000000n, unit, 'NAIVE')
const shortTime: Scalar = Scalar.time(1000, 'ms', Timezone.from('NAIVE'))
const took: Scalar = Scalar.duration(90, 's', 'NAIVE')
const longTook: Scalar = Scalar.duration(90n, 's', Timezone.from('NAIVE'))
const price: Scalar = Scalar.decimal(-1050n, 2)
const widePrice: Scalar = Scalar.decimal(-(2n ** 200n), 2)
const half: Scalar = Scalar.float(1.5, 16)
const single: Scalar = Scalar.float(1.5, 32)
const double: Scalar = Scalar.float(1.5)
const enumScalar: Scalar = Scalar.fromEnum('io_mode', 'append')
const enumKind: string | null = enumScalar.enumKind
const enumValue: string | null = enumScalar.enumValue
const enumOrdinal: number | null = enumScalar.enumOrdinal
const kind: string = at.kind
const count: bigint | null = at.count
const zone: string | null = at.zone
const unscaled: bigint | null = price.unscaled
const scale: number | null = price.scale
const same: boolean = took.equals(Scalar.duration(90000n, 'ms'))
const hash: bigint = widePrice.stableHash()
const valueClone: Scalar = widePrice.clone()
const valueOrder: number = widePrice.compare(valueClone)
const valueSum: Scalar = price.add(Scalar.decimal(50n, 2))
const inferredSum: Scalar = Scalar.fromJs(40).add(2)
const valueDifference: Scalar = price.subtract(1)
const valueProduct: Scalar = price.multiply(2)
const valueQuotient: Scalar = price.divide(2)
const valueRemainder: Scalar = Scalar.fromJs(5).remainder(2)
const negativeValue: Scalar = price.negate()
const absoluteValue: Scalar = negativeValue.absolute()
const dataType = widePrice.dataType
const rawBytes: Buffer | null = Scalar.fromJs(Buffer.from('x')).asBytes()
const rawUtf8: string | null = Scalar.fromJs('x').asUtf8()
const jsonBytes: Buffer = widePrice.asJsonBytes()
const jsonUtf8: string = widePrice.asJsonUtf8()
const pivot: Scalar = Scalar.fromJs(new Set([1, 2]), { maxDepth: 8 })
const lowered: unknown = pivot.asJs()
const scalarField: Field = Scalar.fromJs(1).intoField()
const arrayField: Field = Scalar.fromJs([1]).intoArrayField()
const inferredStructField: Field = Scalar.fromJs([{ id: 1 }]).intoStructField()
const nestedValues = Scalar.fromJs({ rows: [Scalar.datetime(1n, 'ns')] })
const childCount: number = nestedValues.length
const emptyContainer: boolean = nestedValues.isEmpty()
const childAt: Scalar | null = Scalar.fromJs([1]).at(0)
const childByKey: Scalar | null = nestedValues.get('rows')
const childByPath: Scalar | null = nestedValues.path('rows.0')
const hasChild: boolean = nestedValues.has('rows')
const replacedValue: Scalar = nestedValues.set('rows', [2])
const removedValue: Scalar = nestedValues.remove('rows')
const iteratedValues: Scalar[] = [...nestedValues]
void childCount
void emptyContainer
void childAt
void childByKey
void childByPath
void hasChild
void replacedValue
void removedValue
void iteratedValues
void enumKind
void enumValue
void enumOrdinal

const arrowVector = vectorFromArray([1, 2], new Int32())
const arrowValue: Scalar = Scalar.fromArrowArray(arrowVector)
const nativeVector = arrowValue.intoArrowArray()
const arrowScalar: Scalar = Scalar.fromArrowScalar(vectorFromArray([1], new Int32()))
const nativeScalar: unknown = arrowScalar.intoArrowScalar()
const arrowTable = tableFromArrays({ id: Int32Array.from([1, 2]) })
const tableValue: Scalar = Scalar.fromArrowTable(arrowTable)
const rowField = new Field('row', 'struct<id:int32 not null>', false)
class TypedOrder {
  static get intoStructField(): Field {
    return rowField
  }
}
const classFieldOptions: CodecOptions = { field: TypedOrder }
const instanceFieldOptions: CodecOptions = { field: new TypedOrder() }
const classTypedOrder: TypedOrder = json.loads<TypedOrder>('{}', classFieldOptions)
const instanceTypedOrder: TypedOrder = json.loads<TypedOrder>('{}', instanceFieldOptions)
const nativeTable = tableValue.intoArrowTable(rowField)
const batchValue: Scalar = Scalar.fromArrowBatch(arrowTable.batches[0], rowField)
const nativeBatch = batchValue.intoArrowBatch(rowField)
const narrowNative: Scalar = json.loads('7', {
  field: new Field('value', 'int16', false),
  scalar: true,
})
void narrowNative

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
void wideDate
void sinceMidnight
void shortTime
void took
void longTook
void price
void widePrice
void half
void single
void double
void kind
void count
void zone
void unscaled
void scale
void same
void hash
void valueClone
void valueOrder
void valueSum
void inferredSum
void valueDifference
void valueProduct
void valueQuotient
void valueRemainder
void negativeValue
void absoluteValue
void dataType
void rawBytes
void rawUtf8
void jsonBytes
void jsonUtf8
void pivot
void lowered
void nativeVector
void nativeScalar
void nativeTable
void nativeBatch
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
