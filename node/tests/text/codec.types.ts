import {
  DataType,
  Field,
  Timezone,
  Scalar,
  Serie,
  codec,
  json,
  toml,
  xml,
  yaml,
  type CodecOptions,
  type CodecTimeUnit,
  type SingleDocumentCodec,
  type TimezoneInput,
  type TomlCodecFormat,
  type XmlCodecFormat,
} from '../..'
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
// A unit or zone held in a variable reaches the type through the spelling, so
// these pin that a computed datatype expression still type-checks.
const at: Scalar = new DataType(`datetime64(${unit},"UTC")`).scalar(
  1700000000000000n,
)
const naive: Scalar = new DataType('datetime64(ms)').scalar(1700000000000n)
const on: Scalar = new DataType('date32').scalar(19723)
const wideDate: Scalar = new DataType('date64').scalar(1700000000000n)
const sinceMidnight: Scalar = new DataType(`time64(${unit})`).scalar(45296000000n)
const shortTime: Scalar = new DataType('time32(ms)').scalar(1000)
const took: Scalar = Scalar.duration(90, 's')
const longTook: Scalar = Scalar.duration(90n, 's')
const price: Scalar = Scalar.decimal(-1050n, 2)
const widePrice: Scalar = Scalar.decimal(-(2n ** 200n), 2)
const half: Scalar = Scalar.float(1.5, 16)
const single: Scalar = Scalar.float(1.5, 32)
const double: Scalar = Scalar.float(1.5)
const enumScalar: Scalar = Scalar.fromEnum('IOMode', 'append')
const enumText: string | null = enumScalar.asStr()
const truthy: boolean = enumScalar.isTruthy()
const typedInstant: Scalar = new Field(
  'at',
  new DataType('datetime64(ns,"UTC")'),
  true,
).scalar(1n)
const typedWidth: Scalar = Scalar.float(1.5, 16)
const kind: string = at.kind
const scalarId: string = at.id
const scalarFamily: string = at.family
const count: bigint | null = at.count
const zone: string | null = at.zone
const unscaled: bigint | null = price.unscaled
const scale: number | null = price.scale
const same: boolean = took.equals(Scalar.duration(90000n, 'ms'))
const hash: bigint = widePrice.stableHash()
const valueClone: Scalar = widePrice.clone()
const valueOrder: number = widePrice.compare(valueClone)
const valueSum: Scalar = price.add(Scalar.decimal(50n, 2))
const inferredSum: Scalar = Scalar.from(40).add(2)
const valueDifference: Scalar = price.subtract(1)
const valueProduct: Scalar = price.multiply(2)
const valueQuotient: Scalar = price.divide(2)
const valueRemainder: Scalar = Scalar.from(5).remainder(2)
const negativeValue: Scalar = price.negate()
const absoluteValue: Scalar = negativeValue.absolute()
const dtype = widePrice.dtype
const rawBytes: Buffer | null = Scalar.from(Buffer.from('x')).asBytes()
const rawText: string | null = Scalar.from('x').asStr()
const jsonBytes: Buffer = widePrice.intoJsonBytes()
const jsonUtf8: string = widePrice.intoJson()
const pivot: Scalar = Scalar.from(new Set([1, 2]), { maxDepth: 8 })
const lowered: unknown = pivot.asJs()
const scalarField: Field = Scalar.from(1).intoField()
const arrayField: Field = Scalar.from([1]).intoArrayField()
const inferredStructField: Field = Scalar.from([{ id: 1 }]).intoStructField()
const nestedValues = Scalar.from({ rows: [new DataType('datetime64(ns)').scalar(1n)] })
const childCount: number = nestedValues.length
const emptyContainer: boolean = nestedValues.isEmpty()
const childAt: Scalar | null = Scalar.from([1]).at(0)
const childByKey: Scalar | null = nestedValues.get('rows')
const childByPath: Scalar | null = nestedValues.path('rows.0')
const hasChild: boolean = nestedValues.has('rows')
const replacedValue: Scalar = nestedValues.set('rows', [2])
const removedValue: Scalar = nestedValues.remove('rows')
const iteratedValues: Scalar[] = [...nestedValues]
void childCount
void scalarId
void scalarFamily
void emptyContainer
void childAt
void childByKey
void childByPath
void hasChild
void replacedValue
void removedValue
void iteratedValues
void enumText
void typedInstant
void typedWidth
void truthy

// Arrow crosses as a Serie, and a column is one value; the Scalar Arrow
// doors are retired with no alias.
const arrowVector = vectorFromArray([1, 2], new Int32())
const arrowValue: Scalar = Serie.fromArrowArray(arrowVector).intoScalar()
const arrowScalar: Scalar = Serie.fromArrowArray(vectorFromArray([1], new Int32())).scalar(0)
const arrowTable = tableFromArrays({ id: Int32Array.from([1, 2]) })
const rowField = new Field('row', 'struct<id:int32 not null>', false)
const tableValue: Scalar = Serie.fromArrowBatch(arrowTable, rowField).intoScalar()
// @ts-expect-error retired: `Serie.fromArrowArray(vector).scalar(0)`
Scalar.fromArrowScalar(arrowVector)
// @ts-expect-error retired: `Serie.fromArrowArray(vector).intoScalar()`
Scalar.fromArrowArray(arrowVector)
// @ts-expect-error retired: `Serie.fromArrowBatch(batch).intoScalar()`
Scalar.fromArrowBatch(arrowTable.batches[0])
// @ts-expect-error retired: `Serie.fromArrowBatch(table).intoScalar()`
Scalar.fromArrowTable(arrowTable)
// @ts-expect-error retired: `Serie.fromScalars(field, [value]).intoArrowScalar()`
arrowScalar.intoArrowScalar()
// @ts-expect-error retired: `Serie.fromScalars(field, rows).intoArrowArray()`
arrowValue.intoArrowArray()
// @ts-expect-error retired: `Serie.fromScalars(root, rows).intoArrowBatch()`
tableValue.intoArrowBatch(rowField)
// @ts-expect-error retired, with the table door: a batch is the one record shape
tableValue.intoArrowTable(rowField)
class TypedOrder {
  static get intoStructField(): Field {
    return rowField
  }
}
const classFieldOptions: CodecOptions = { field: TypedOrder }
const instanceFieldOptions: CodecOptions = { field: new TypedOrder() }
const classTypedOrder: TypedOrder = json.loads<TypedOrder>('{}', classFieldOptions)
const instanceTypedOrder: TypedOrder = json.loads<TypedOrder>('{}', instanceFieldOptions)
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
const xmlFormat: XmlCodecFormat = 'xml'
const xmlFacade: SingleDocumentCodec = xml
const xmlBytes: Buffer = xml.dumps({ order: new Order() }, { format: xmlFormat })
const xmlOrder: Order = xml.loads<Order>(xmlBytes, { field: new Field('order', 'struct<id: int64>', false) })
const inferredXml: Order = codec.from<Order>('order.xml', { format: 'xml' })
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
// So is XML: a document is one root element.
// @ts-expect-error no XML multi-document decode API
xml.loadsAll(xmlBytes)
// @ts-expect-error no XML multi-document encode API
xml.dumpAll([order])
void xmlFacade
void xmlOrder
void inferredXml

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
void dtype
void rawBytes
void rawText
void jsonBytes
void jsonUtf8
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
