import {
  Field,
  RecordOptions,
  Scalar,
  xml,
  type XmlDecodeLimits,
  type XmlSchemaOptions,
  type XmlWriteOptions,
} from '../..'
import { Buffer } from 'node:buffer'

interface Order {
  id: string
  symbol: string
  qty: string
}

const DOCUMENT = '<Order id="7"><symbol>AAPL</symbol><qty>100</qty></Order>'
const SCHEMA =
  '<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">' +
  '<xs:element name="note" type="xs:string"/>' +
  '</xs:schema>'

const limits: XmlDecodeLimits = {
  maxDepth: 16,
  maxInputBytes: 1_000_000,
  maxNodes: 10_000,
}
const schemaOptions: XmlSchemaOptions = { ...limits, root: 'note' }
const layout: XmlWriteOptions = { indent: 2 }

const order: Order = xml.loads<Order>(DOCUMENT, limits)
const fromBytes: Order = xml.loads<Order>(Buffer.from(DOCUMENT), limits)
const declared: Field = xml.schema(SCHEMA, schemaOptions)
const typed: Scalar = xml.loadsWithField(DOCUMENT, declared, limits)
const document: Buffer = xml.dumps(order, 'Order', layout)
const compact: Buffer = xml.dumps(order, 'Order', { indent: null })
const tabbed: Buffer = xml.dumps(order, 'Order', { indent: '\t' })
const schema: Buffer = xml.schemaDumps(declared, layout)

const options: RecordOptions = RecordOptions.from('feed.xml')
options.document = 'channel'
options.rowElement = 'item'
options.maxNodes = 1_000
const wrapper: string | null = options.document
const named: string | null = options.rowElement
const nodes: number | null = options.maxNodes

// @ts-expect-error the document element name is required
xml.dumps(order, undefined)
// @ts-expect-error indent belongs to a write, never to a decode
xml.loads(DOCUMENT, { indent: 2 })
// @ts-expect-error a root names one global element declaration
xml.schema(SCHEMA, { root: 7 })
// @ts-expect-error limits are numbers
xml.loads(DOCUMENT, { maxNodes: 1n })
// @ts-expect-error a layout is spaces, a tab, or none at all
xml.dumps(order, 'Order', { indent: 'wide' })
// @ts-expect-error only a Field types a document's leaves
xml.loadsWithField(DOCUMENT, 'note')
// @ts-expect-error a schema is written from a Field, not from its name
xml.schemaDumps('note')

void order
void fromBytes
void typed
void document
void compact
void tabbed
void schema
void wrapper
void named
void nodes
