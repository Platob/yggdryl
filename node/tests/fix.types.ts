import {
  BatchReader,
  Field,
  IOBase,
  MimeType,
  Scalar,
  Url,
  fix,
  type FixMsg,
  type FixCodec,
  type FixCaptureView,
  type FixDirection,
  type FixEntryView,
  type FixEventView,
  type FixHeaderView,
  type FixRegistry,
  type FixMessages,
  type MsgType,
  type FixValueInput,
  type LocationInput,
  type TextLine,
} from '..'

declare const field: Field
declare const handle: IOBase
declare const url: Url
declare const value: Scalar

// The namespace holds native FIX values and iterators, and no constant: an
// identifier is a number derived from a tag and a name.
const registryClass: typeof FixRegistry = fix.FixRegistry
const seeded: FixRegistry = new fix.FixRegistry()
const built: FixRegistry = fix.FixRegistry.fromFields([field, field])
const location: LocationInput = url
const loaded: FixRegistry = fix.FixRegistry.fromHandle(location)
const fromString: FixRegistry = fix.FixRegistry.fromHandle('file:///lake/fix')
const fromHandle: FixRegistry = fix.FixRegistry.fromHandle(handle)
loaded.writeInto(handle)

void registryClass
void seeded
void built
void fromString
void fromHandle

const size: number = loaded.size
const byId: Field | null = loaded.getFieldById(-1873404312)
const requiredById: Field = loaded.fieldById(-1873404312)
const byTag: Field | null = loaded.getFieldByTag(55)
const requiredByTag: Field = loaded.fieldByTag(55)
const byName: Field | null = loaded.getFieldByName('Symbol')
const requiredByName: Field = loaded.fieldByName('Symbol')
const byPath: Field | null = loaded.getFieldByPath('Parties.PartyID')
const requiredByPath: Field = loaded.fieldByPath('Parties.PartyID')
const bytesProtocol: MimeType = MimeType.inferBytes(Buffer.from('35=D|'))
const textProtocol: MimeType = MimeType.inferText('35=D|')
const bytesMsgtype: Buffer | null = fix.FixCodec.inferMsgtypeBytes(Buffer.from('35=D|'))
const textMsgtype: string | null = fix.FixCodec.inferMsgtypeText('35=D|')
const byKey: Field | null = loaded.getField(55)
const byNameKey: Field | null = loaded.getField('Symbol')
const requiredByKey: Field = loaded.field('Symbol')
const mapLike: Field | null = loaded.get(55)
const present: boolean = loaded.has('Symbol')
const added: boolean = loaded.addField(field)
const inserted: Field | null = loaded.insert(field)
loaded.update(field)
const removed: Field | null = loaded.remove(55)
const removedById: Field | null = loaded.removeById(-1873404312)
const walk: Generator<Field> = loaded.keys()
const drained: Field[] = [...loaded]
const forOf: Field[] = [...loaded.keys()]
const dialects: string[] = loaded.dialects()
const same: boolean = loaded.equals(built)
const registryHash: bigint = loaded.stableHash()
const copy: FixRegistry = loaded.clone()
const rendered: string = loaded.toString()
const document: unknown = loaded.toJSON()

void size
void byId
void requiredById
void byTag
void requiredByTag
void byName
void requiredByName
void byPath
void requiredByPath
void bytesProtocol
void textProtocol
void bytesMsgtype
void textMsgtype
void byKey
void byNameKey
void requiredByKey
void mapLike
void present
void inserted
void removed
void removedById
void walk
void drained
void forOf
void dialects
void same
void registryHash
void copy
void rendered
void document

// A key is a tag or a name; anything else is refused before it runs.
// @ts-expect-error a bigint is not a FIX key
loaded.getField(55n)
// @ts-expect-error an object is not a FIX key
loaded.get({ tag: 55 })
// @ts-expect-error a tag is a number, never a string
loaded.getFieldByTag('55')
// @ts-expect-error a name is a string, never a number
loaded.fieldByName('std', 55)
// @ts-expect-error a name is required
loaded.fieldByName()
// @ts-expect-error an identifier is a number, never a string
loaded.fieldById('5001:cme')
// @ts-expect-error an identifier is a number, never a string
loaded.removeById('55:')
// @ts-expect-error a name lookup takes no dialect: the registry is one namespace
loaded.getFieldByName('Symbol', 'cme')
// @ts-expect-error a path lookup takes no dialect
loaded.fieldByPath('Parties.PartyID', '')

const input: FixValueInput = { Symbol: 'AAPL' }
const message: FixMsg = new fix.FixMsg(field, input)
const explicit: FixMsg = new fix.FixMsg(field, value, loaded)
const nullRegistry: FixMsg = new fix.FixMsg(field, input, null)
const linked: FixRegistry = message.registry
const schema: Field = message.field
const row: Scalar = message.value
const valueCount: number = message.size
const valueById: Scalar | null = message.getById(-1873404312)
const requiredValueById: Scalar = message.byId(-1873404312)
const valueByTag: Scalar | null = message.getByTag(55)
const requiredValueByTag: Scalar = message.byTag(55)
const valueByName: Scalar | null = message.getByName('Symbol')
const requiredValueByName: Scalar = message.byName('Symbol')
const valueByPath: Scalar | null = message.getByPath('Parties.0.PartyID')
const requiredValueByPath: Scalar = message.byPath('Parties.0.PartyID')
const valueByKey: Scalar | null = message.get(55)
const requiredValueByKey: Scalar = message.at('Symbol')
const entries: FixEntryView[] = message.entries()
const nested: FixEntryView[] = entries[0].entries
const entryTag: number = entries[0].tag
const entryName: string = entries[0].name
const entryValue: string | null = entries[0].value
const walked: FixEntryView[] = [...message]
// The typed holders answer one plain object each.
const event: FixEventView = message.event()
const header: FixHeaderView = message.header()
const capture: FixCaptureView = message.capture()
const text: string | null = message.text
const metadata: Record<string, string> = message.metadata
// The graph facts a message answers directly.
const curruuid: string = message.curruuid
const crossuuid: string = message.crossuuid
const crosscode: string = message.crosscode
const currhashcode: bigint = message.currhashcode
const crosshashcode: bigint = message.crosshashcode
const currunix: bigint = message.currunix
const state: string = message.state
const seqnum: number = message.seqnum
const prevuuid: string | null = message.prevuuid
const parentuuids: string[] = message.parentuuids
const srcuuids: string[] = message.srcuuids
const messageIdentifiers: Record<string, string> = message.identifiers
const price: string = message.price
const quantity: string = message.quantity
const side: string = message.side
const currency: string = message.currency
const figicode: string | null = message.figicode
const messageCategory: number | null = message.msgcat
const marketOperationId: number | null = message.marketoperationid
// And the same facts on the event, with the instants and the lanes.
const eventCurrunix: bigint = event.currunix
const eventCreated: bigint | null = event.creaunix
const eventExecuted: bigint | null = event.execunix
const eventRecorded: bigint | null = event.recdunix
const eventReferenceRecorded: bigint | null = event.refrecdunix
const eventPrevUnix: bigint | null = event.prevunix
const eventSnap: bigint | null = event.snapunix
const eventExpiry: bigint | null = event.exprtime
const eventMarketOperationId: number | null = event.marketoperationid
const eventPrice: string = event.price
const eventQuantity: string = event.quantity
const eventLastPx: string | null = event.lastpx
const eventLastQty: string | null = event.lastqty
const eventAvgPx: string | null = event.avgpx
const eventCumQty: string | null = event.cumqty
const eventLeavesQty: string | null = event.leavesqty
const eventPrevPx: string | null = event.prevpx
const eventPrevQty: string | null = event.prevqty
const eventTif: string | null = event.tif
const eventTradable: boolean | null = event.tradable
const eventSymbolTicker: string | null = event.symbolticker
const eventIsin: string | null = event.isincode
const eventFigi: string | null = event.figicode
const eventBidPx: string | null = event.bidpx
const eventAskCurrency: string | null = event.askcurrency
const eventIdentifiers: Record<string, string> = event.identifiers
const eventParents: string[] = event.parentuuids
const eventSources: string[] = event.srcuuids
const beginstring: string = header.beginstring
const msgtype: string = header.msgtype
const sendercompid: string | null = header.sendercompid
const msgseqnum: number | null = header.msgseqnum
const sendingtime: bigint = header.sendingtime
const possdupflag: boolean | null = header.possdupflag
const msgdirection: string | null = header.msgdirection
const msgpluginid: string | null = capture.msgpluginid
const msgctxid: string | null = capture.msgctxid
const msgsessionid: string | null = capture.msgsessionid

void entries
void nested
void entryTag
void entryName
void entryValue
void walked
void text
void metadata
void curruuid
void crossuuid
void crosscode
void currhashcode
void crosshashcode
void currunix
void state
void seqnum
void prevuuid
void parentuuids
void srcuuids
void eventSources
void messageIdentifiers
void price
void quantity
void side
void currency
void figicode
void messageCategory
void marketOperationId
void eventCurrunix
void eventCreated
void eventExecuted
void eventRecorded
void eventReferenceRecorded
void eventPrevUnix
void eventSnap
void eventExpiry
void eventMarketOperationId
void eventPrice
void eventQuantity
void eventLastPx
void eventLastQty
void eventAvgPx
void eventCumQty
void eventLeavesQty
void eventPrevPx
void eventPrevQty
void eventTif
void eventTradable
void eventSymbolTicker
void eventIsin
void eventFigi
void eventBidPx
void eventAskCurrency
void eventIdentifiers
void eventParents
void beginstring
void msgtype
void sendercompid
void msgseqnum
void sendingtime
void possdupflag
void msgdirection
void msgpluginid
void msgctxid
void msgsessionid

// @ts-expect-error a graph fact is read, never assigned
message.curruuid = 'other'
// @ts-expect-error the entries are derived from the row
message.entries = []

const equalMessages: boolean = message.equals(explicit)
const messageHash: bigint = message.stableHash()
const clonedMessage: FixMsg = message.clone()
const messageText: string = message.toString()
const messageDocument: unknown = message.toJSON()
message.set(55, 'MSFT')
message.set('Symbol', null)
const removedValue: Scalar | null = message.remove(55)
const readBack: FixMsg = fix.FixMsg.fromRow(field, input)
const readBackExplicit: FixMsg = fix.FixMsg.fromRow(field, value, loaded)

void explicit
void nullRegistry
void linked
void schema
void row
void valueCount
void valueById
void requiredValueById
void valueByTag
void requiredValueByTag
void valueByName
void requiredValueByName
void valueByPath
void requiredValueByPath
void valueByKey
void requiredValueByKey
void equalMessages
void messageHash
void clonedMessage
void messageText
void messageDocument
void removedValue
void readBack
void readBackExplicit

// @ts-expect-error a message value is not a bare string
new fix.FixMsg(field, 'AAPL')
// @ts-expect-error a message key is a tag or a name
message.at(55n)
// @ts-expect-error a write's key is a tag or a name
message.set(55n, 'MSFT')
// @ts-expect-error a row is read under a Field, never a number
fix.FixMsg.fromRow(55, input)

const global: FixRegistry = fix.globalRegistry()
fix.installGlobalRegistry(global)

// The typed FIX vocabulary lives on the protocol view a field already answers.
const branches: string[] = field.fix.branches
field.fix.branches = ['cme', 'ice']
field.fix.addBranch('bloomberg')
const member: boolean = field.fix.hasBranch('cme')
const identity: number | null = field.fix.id
const tag: number | null = field.fix.tag
field.fix.tag = 55
const tags: number[] = field.fix.tags
field.fix.tags = [1088]
const names: string[] = field.fix.names
field.fix.names = ['Ticker']
const identifiers: string[] = field.fix.identifiers
field.fix.identifiers = ['11', 'OrderIdentifier']
field.fix.identifiers = []
const description: string | null = field.fix.description
field.fix.description = 'Ticker symbol.'
const nulls: string[] = field.fix.nulls
field.fix.nulls = ['<none>']
// A direction rule is one record per code of tag 385's set, its patterns decoded.
const directions: FixDirection[] = field.fix.directions
field.fix.directions = [{ code: 'S', patterns: ['(?i)^TX\\b'] }, { code: 'R', patterns: ['(?i)^RX\\b'] }]
field.fix.directions = []
// A derivation is one term's canonical text, or null where nothing derives the field.
const derivation: string | null = field.fix.derivation
field.fix.derivation = 'orderqty - cumqty'
field.fix.derivation = null

void branches
void member
void identity
void tag
void tags
void names
void identifiers
void description
void nulls
void directions
void derivation

// @ts-expect-error a tag crosses as a number, never a bigint
field.fix.tag = 55n
// @ts-expect-error membership is a list of names, never one name
field.fix.branches = 'cme'
// @ts-expect-error membership is a list of names, never numbers
field.fix.branches = [55]
// @ts-expect-error the identifier is derived from the tag and the name, never assigned
field.fix.id = 5001
// @ts-expect-error aliases are strings
field.fix.names = [55]
// @ts-expect-error identifiers are an array of spellings, never one spelling
field.fix.identifiers = '11'
// @ts-expect-error decimal tags are spelled as strings
field.fix.identifiers = [11]
// @ts-expect-error native list intake accepts arrays, not arbitrary iterables
field.fix.identifiers = new Set(['11'])
// @ts-expect-error native list intake accepts arrays, not generators
field.fix.identifiers = (function* () { yield '11' })()
// @ts-expect-error a sparse selection contains an absent spelling
field.fix.identifiers = [, '11']
// @ts-expect-error a rule states a code and its patterns
field.fix.directions = [{ code: 'S' }]
// @ts-expect-error the patterns are a list, never one pattern
field.fix.directions = [{ code: 'S', patterns: '^TX ' }]
// @ts-expect-error a derivation is text, never a parsed term object
field.fix.derivation = { term: 'orderqty - cumqty' }

// The codec is a class over one dictionary, with every pin optional and
// read back as it was given.
const readerClass: typeof FixCodec = fix.FixCodec
const reader: FixCodec = new fix.FixCodec(loaded)
const pinned: FixCodec = new fix.FixCodec(loaded, {
  separator: 124,
  payloadColumn: 'line',
  nullValues: ['<none>'],
  direction: 'recv',
  batchByteSize: 1 << 20,
})
// @ts-expect-error a codec pins no version: a row states one, or the line implies it
const stalePin: FixCodec = new fix.FixCodec(loaded, { version: '4.2' })
void stalePin
// @ts-expect-error no pin names a dialect: the dictionary is one namespace
const dialectPin: FixCodec = new fix.FixCodec(loaded, { branch: 'cme' })
void dialectPin
// The SendingTime an undated message takes is a `Scalar` or a `Date`, read
// back as the native clock or `null` where each new message reads now.
const dated: FixCodec = new fix.FixCodec(loaded, { defaultSendingTime: value })
const datedFromDate: FixCodec = new fix.FixCodec(loaded, { defaultSendingTime: new Date(0) })
const undated: FixCodec = new fix.FixCodec(loaded, { defaultSendingTime: null })
const defaultSendingTime: Scalar | null = dated.defaultSendingTime
void datedFromDate
void undated
void defaultSendingTime
// @ts-expect-error the default sending time is a Scalar or a Date, never text
new fix.FixCodec(loaded, { defaultSendingTime: '2024-01-02T10:15:30Z' })
// @ts-expect-error the default sending time is fixed at construction
dated.defaultSendingTime = value
const readRegistry: FixRegistry = reader.registry
const pinnedSeparator: number | null = pinned.separator
const pinnedPayloadColumn: string = pinned.payloadColumn
const pinnedNullValues: string[] = pinned.nullValues
const pinnedDirection: string | null = pinned.direction
const pinnedBatchByteSize: number = pinned.batchByteSize
const fromText: FixMsg = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromBytes: FixMessages = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromLines: FixMessages = reader.parseLines([Buffer.from('8=FIX.4.4|35=D|10=0|'), '8=FIX.4.4|35=D|10=0|'])
const fromFrame: FixMsg = reader.parseFixLine(Buffer.from('8=FIX.4.4'))
const fromBridge: FixMsg = reader.parseUllinkLine(Buffer.from('#SYMBOL=TTF'))
const fromFixml: FixMsg = reader.parseFixmlLine(Buffer.from("<Order ClOrdID='A'/>"))
const fromPairs: FixMsg = reader.parsePairs([['55', 'AAPL']])
const readerCopy: FixCodec = reader.clone()

// The walk is a codec stage: any iterable in, a lazy `FixMessages` out.
const stream: FixMessages = reader.lifecycle([fromText])
const stampedAgain: FixMessages = reader.lifecycle(stream)

void stream
void stampedAgain

// @ts-expect-error the walk takes an iterable of messages, never one message
reader.lifecycle(fromText)
// @ts-expect-error the walk takes messages, never lines
reader.lifecycle([Buffer.from('8=FIX.4.4|35=0|10=0|')])

// The Arrow twins take and answer batch readers, and a stream crosses back.
const parsedBatches: BatchReader = reader.parseTextArrowReader(BatchReader.fromIpc(new Uint8Array()))
const walkedBatches: BatchReader = reader.lifecycleArrowReader(parsedBatches)
const readBackStream: FixMessages = reader.messages(walkedBatches)
const rows: BatchReader = reader.arrowReader(field, readBackStream)
const books: BatchReader = reader.bookArrowReader([fromText], 1000, false)
const written: number = reader.writeArrowReader(rows, { write(chunk: Uint8Array) { void chunk } })

void pinnedSeparator
void pinnedPayloadColumn
void pinnedNullValues
void pinnedDirection
void pinnedBatchByteSize
void fromLines
void fromFixml
void written
void books

// @ts-expect-error a stage is a call, never a flag
reader.parseLine(Buffer.from('35=D|'), true)
// @ts-expect-error a sink writes chunks
reader.writeArrowReader(rows, {})

// The fixed row is a schema, and a column is the folded name of its field.
const fixedSchema: Field = fix.schema(loaded, 'FixMessage')
const carried: Field = fix.schemaCarrying(field, fixedSchema)
const at: number | null = fixedSchema.indexOf('msgtype')
const fixedRow: Scalar = fromText.intoRow(fixedSchema)
const fixedSchemaTags: number[] = fix.schemaTags()
const crateFields: Field[] = fix.crateFields()

// What a message emits and digests.
const digest: Buffer = fromText.digest()
const wire: Buffer = fromText.intoBytes(124)
const defaultWire: Buffer = fromText.intoBytes()
const wireText: string = fromText.intoText('|')
const defaultWireText: string = fromText.intoText()

void digest
void wire
void defaultWire
void wireText
void defaultWireText

// @ts-expect-error the retired readers are gone: the facts are the holders'
fromText.updatedat()
// @ts-expect-error a lifted facet is the event's own lane
fromText.lifted('bidpx')
// @ts-expect-error the arrival record is the entries
fromText.arrivals()
// @ts-expect-error a separator is one byte
fromText.intoBytes('|')

void readerClass
void pinned
void readRegistry
void fromBytes
void fromFrame
void fromBridge
void fromPairs
void readerCopy
void carried
void at
void fixedRow
void fixedSchema
void fixedSchemaTags
void crateFields
void digest

// @ts-expect-error a fixed schema is built from a registry, never from a number
fix.schema(55)

// A definition is reached through the field doors: by name, by the counter
// it opens, or by the path grammar.
const group: Field = loaded.fieldByName('parties')
const groupByCounter: Field | null = loaded.getFieldByCounter(453)
const requiredByCounter: Field = loaded.fieldByCounter(453)
const nestedMember: Field = loaded.fieldByPath('Parties.PartyID')
const definitionInserted: Field | null = loaded.insert(field)
loaded.update(field)
const definitionRemoved: Field | null = loaded.remove('parties')
const snapshot: string = loaded.intoJson()
const restored: FixRegistry = fix.FixRegistry.fromJson(snapshot)

void group
void groupByCounter
void requiredByCounter
void nestedMember
void definitionInserted
void definitionRemoved
void restored

// @ts-expect-error the category doors are gone
loaded.getDefinition('components', 'party')
// @ts-expect-error a counter is a number, never text
loaded.fieldByCounter('453')

const order: MsgType = loaded.msgtype('D')
const optionalOrder: MsgType | null = loaded.getMsgtype('newordersingle')
const registered: MsgType = loaded.registerMsgtype('BridgeReport', 'bridgereport')
const wireCode: string = order.asStr()
const orderCategory: string | null = order.msgcat
const messageDefinition: Field = order.asField()
const identifierValues: Array<[Field, Scalar]> = order.identifierValues(fromText)
for (const [identifierField, identifierValue] of identifierValues) {
  const declaration: Field = identifierField
  const nativeValue: Scalar = identifierValue
  declaration.setName('independent')
  void nativeValue
}
const scoped: Field | null = order.getGroupByTag(453)
const singletonHash: bigint = order.stableHash()
const singletonEqual: boolean = order.equals(order.clone())
const singletonOrder: number = order.compare(order)

const snapshotCodec = new fix.FixCodec(loaded, { snapshotNs: 1_000_000_000n })
const snapshotNs: bigint | null = snapshotCodec.snapshotNs
const snapshotsDisabled: FixCodec = new fix.FixCodec(loaded, { snapshotNs: null })

const officialDelayCodec = new fix.FixCodec(loaded, { officialTimeDelayMs: 250 })
const officialTimeDelayMs: number = officialDelayCodec.officialTimeDelayMs

field.fix.counter = 453
const counterTag: number | null = field.fix.counter
field.fix.component = 'party'
field.fix.group = 'parties'
field.fix.fieldRef = 'partyid'
field.fix.msgtype = 'BridgeReport'
const componentRef: string | null = field.fix.component
const groupRef: string | null = field.fix.group
const fieldRef: string | null = field.fix.fieldRef
const messageCode: string | null = field.fix.msgtype
const fieldCategory: string | null = field.fix.msgcat
field.fix.msgcat = 'ORDR'
field.fix.msgcat = null

const decoded: TextLine = handle.readTextLines().next().value
const records: FixMessages = reader.parseTextLine(decoded)
const lineStream: FixMessages = reader.parseTextLines([decoded])
const nextMessage: IteratorResult<FixMsg> = records.next()
const allMessages: FixMsg[] = [...records, ...lineStream]

// @ts-expect-error the generic parse returns a cursor
const single: FixMsg = reader.parseLine(Buffer.from('35=D|'))
// @ts-expect-error retired projection spelling
message.toRow(field)
// @ts-expect-error retired projection spelling
message.toBytes()
// @ts-expect-error message type is a registry singleton, not a datatype constructor
new fix.MsgType('D')
// @ts-expect-error singleton classes have no public constructor
new fix.MsgType()
// @ts-expect-error compiled identifier selection takes a native message, not a Field
order.identifierValues(field)
// @ts-expect-error compiled identifier selection takes a native message, not a row object
order.identifierValues({ clordid: 'ORDER-1' })
// @ts-expect-error message cursors are constructed by the native codec
new fix.FixMessages()
// @ts-expect-error counter metadata requires a number
field.fix.counter = '453'

void [order, optionalOrder, registered, wireCode, messageDefinition, identifierValues,
  scoped, singletonHash, singletonEqual, singletonOrder, counterTag, componentRef,
  groupRef, fieldRef, messageCode, nextMessage, allMessages, single]
