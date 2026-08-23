import {
  AvroSchema,
  Scalar,
  avro,
  type AvroContainer,
  type AvroBlock,
  type AvroBlocks,
  type AvroDecodeLimits,
  type AvroDecodeOptions,
  type AvroSchemaDocument,
  type AvroSchemaInput,
} from '..'
import { Buffer } from 'node:buffer'

interface Trade {
  symbol: string
  qty: number
}

const document: AvroSchemaDocument = {
  type: 'record',
  name: 'trade',
  fields: [
    { name: 'symbol', type: 'string' },
    { name: 'qty', type: 'int' },
  ],
}
const input: AvroSchemaInput = document
const limits: AvroDecodeLimits = { maxDepth: 16, maxInputBytes: 1_000_000, maxNodes: 10_000 }
const options: AvroDecodeOptions = { ...limits, readerSchema: document }
const schema = new AvroSchema(input, limits)
const fromNative: AvroSchema = AvroSchema.from(Scalar.fromJs(document), limits)
const canonical: string = schema.intoCanonicalForm()
const canonicalProperty: string = schema.canonicalForm
const fingerprint: bigint = schema.fingerprint
const kind: string = schema.kind
const original: AvroSchemaDocument = schema.intoJSON()
const cloned: AvroSchema = schema.clone()
const schemaEquals: boolean = schema.equals(cloned)
const schemaOrder: number = schema.compare(cloned)
const schemaHash: bigint = schema.stableHash()

const bytes: Buffer = avro.dumps(
  [{ symbol: 'AAPL', qty: 100 }],
  schema,
  new Map([['source', 'typescript']]),
)
const container: AvroContainer<Trade> = avro.loads<Trade>(bytes)
const resolvedContainer: AvroContainer<Trade> = avro.loads<Trade>(bytes, options)
const blocks: AvroBlocks<Trade> = avro.blocks<Trade>(bytes, options)
const blockResult: IteratorResult<AvroBlock<Trade>> = blocks.next()
if (!blockResult.done) {
  const blockCount: bigint = blockResult.value.count
  const compressedSize: bigint = blockResult.value.size
  const blockRows: Trade[] = blockResult.value.rows()
  void blockCount
  void compressedSize
  void blockRows
}
const trade: Trade = container.rows[0]
const writer: AvroSchema = container.schema
const metadata: Record<string, string> = container.metadata
const single: Buffer = avro.dumpsSingle(trade, schema)
const decoded: Trade = avro.loadsSingle<Trade>(single, schema, limits)
const throughSchema: Trade = schema.fromSingleObject<Trade>(
  schema.intoSingleObject(decoded),
  limits,
)
const exact: Scalar = avro.loadsSingle<Scalar>(
  avro.dumpsSingle(Scalar.d128(18750n, 2), {
    type: 'bytes',
    logicalType: 'decimal',
    precision: 10,
    scale: 2,
  }),
  {
    type: 'bytes',
    logicalType: 'decimal',
    precision: 10,
    scale: 2,
  },
)

// @ts-expect-error encoded Avro input is bytes, not text
avro.loads('container')
// @ts-expect-error reader schema belongs inside the one options object
avro.loads(bytes, schema)
// @ts-expect-error limits are numbers
avro.blocks(bytes, { maxNodes: 1n })
// @ts-expect-error readerSchema is not meaningful for single-object decoding
avro.loadsSingle(single, schema, { readerSchema: schema })
// @ts-expect-error rows must be iterable
avro.dumps({ symbol: 'AAPL', qty: 100 }, schema)
// @ts-expect-error metadata values must be strings
avro.dumps([], schema, { source: 1 })

void fromNative
void resolvedContainer
void blocks
void canonical
void canonicalProperty
void fingerprint
void kind
void original
void cloned
void writer
void metadata
void throughSchema
void exact
