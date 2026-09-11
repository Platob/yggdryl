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
  type FixLifecycle,
  type FixRegistry,
  type FixMessages,
  type MsgType,
  type UlPlugin,
  type UlPlugins,
  type FixValueInput,
  type LocationInput,
  type TextLine,
} from '../..'

declare const field: Field
declare const handle: IOBase
declare const url: Url
declare const value: Scalar

// The namespace holds native FIX values and iterators.
const standardBranch: string = fix.STANDARD_BRANCH
const userTagMin: number = fix.USER_TAG_MIN
const userTagMax: number = fix.USER_TAG_MAX
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
const byId: Field | null = loaded.getFieldById('5001:cme')
const requiredById: Field = loaded.fieldById('5001:cme')
const byTag: Field | null = loaded.getFieldByTag(55)
const requiredByTag: Field = loaded.fieldByTag(55)
const byName: Field | null = loaded.getFieldByName('Symbol', standardBranch)
const requiredByName: Field = loaded.fieldByName('Symbol', '')
const byPath: Field | null = loaded.getFieldByPath('Parties.PartyID', '')
const requiredByPath: Field = loaded.fieldByPath('Parties.PartyID', '')
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
const removedById: Field | null = loaded.removeById('5001:cme')
const walk: Generator<Field> = loaded.keys()
const drained: Field[] = [...loaded]
const forOf: Field[] = [...loaded.keys()]
const branchName: string = loaded.branchByDigest(0)
const branchOrNull: string | null = loaded.getBranchByDigest(0)
const same: boolean = loaded.equals(built)
const registryHash: bigint = loaded.stableHash()
const copy: FixRegistry = loaded.clone()
const rendered: string = loaded.toString()
const document: unknown = loaded.toJSON()

void size
void userTagMin
void userTagMax
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
// @ts-expect-error a name is required even though its branch is optional
loaded.fieldByName()
// @ts-expect-error an identifier is a string, never a number
loaded.fieldById(5001)
// @ts-expect-error an identifier is a string, never a number
loaded.removeById(5001)

const input: FixValueInput = { Symbol: 'AAPL' }
const message: FixMsg = new fix.FixMsg(field, input)
const explicit: FixMsg = new fix.FixMsg(field, value, loaded)
const nullRegistry: FixMsg = new fix.FixMsg(field, input, null)
const linked: FixRegistry = message.registry
const schema: Field = message.field
const row: Scalar = message.value
const valueCount: number = message.size
const messageBranch: string = message.branch
const valueById: Scalar | null = message.getById('55:')
const requiredValueById: Scalar = message.byId('55:')
const valueByTag: Scalar | null = message.getByTag(55)
const requiredValueByTag: Scalar = message.byTag(55)
const valueByName: Scalar | null = message.getByName('Symbol')
const requiredValueByName: Scalar = message.byName('Symbol')
const valueByPath: Scalar | null = message.getByPath('Parties.0.PartyID')
const requiredValueByPath: Scalar = message.byPath('Parties.0.PartyID')
const valueByKey: Scalar | null = message.get(55)
const requiredValueByKey: Scalar = message.at('Symbol')
const pairs: Generator<[string, Scalar]> = message.entries()
const materialized: [string, Scalar][] = [...message.entries()]
const iterated: [string, Scalar][] = [...message]
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
void messageBranch
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
void pairs
void materialized
void iterated
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
const branch: string = field.fix.branch
field.fix.branch = 'cme'
const identity: string | null = field.fix.id
field.fix.id = '5001:cme'
const tag: number | null = field.fix.tag
field.fix.tag = 55
const tags: number[] = field.fix.tags
field.fix.tags = [1088]
const aliases: string[] = field.fix.aliases
field.fix.aliases = ['Ticker']
const description: string | null = field.fix.description
field.fix.description = 'Ticker symbol.'

void branch
void identity
void tag
void tags
void aliases
void description

// @ts-expect-error a tag crosses as a number, never a bigint
field.fix.tag = 55n
// @ts-expect-error a branch crosses as text, never a number
field.fix.branch = 55
// @ts-expect-error an identifier crosses as text, never a number
field.fix.id = 5001
// @ts-expect-error aliases are strings
field.fix.aliases = [55]

// The codec is a class over one dictionary, with every pin optional and
// read back as it was given.
const readerClass: typeof FixCodec = fix.FixCodec
const reader: FixCodec = new fix.FixCodec(loaded)
const pinned: FixCodec = new fix.FixCodec(loaded, {
  branch: 'cme',
  version: '4.4',
  separator: 124,
  payloadColumn: 'line',
  nullValues: ['<none>'],
  direction: 'recv',
  batchByteSize: 1 << 20,
})
// @ts-expect-error the source and target pins are one `version`
const stalePin: FixCodec = new fix.FixCodec(loaded, { sourceVersion: '4.2' })
void stalePin
const readRegistry: FixRegistry = reader.registry
const pinnedBranch: string | null = pinned.branch
const pinnedVersion: string | null = pinned.version
const pinnedSeparator: number | null = pinned.separator
const pinnedPayloadColumn: string = pinned.payloadColumn
const pinnedNullValues: string[] = pinned.nullValues
const pinnedDirection: string = pinned.direction
const pinnedBatchByteSize: number = pinned.batchByteSize
const fromText: FixMsg = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromBytes: FixMessages = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromLines: FixMessages = reader.parseLines([Buffer.from('8=FIX.4.4|35=D|10=0|'), '8=FIX.4.4|35=D|10=0|'])
const fromFrame: FixMsg = reader.parseFixLine(Buffer.from('8=FIX.4.4'))
const fromBridge: FixMsg = reader.parseUllinkLine(Buffer.from('#SYMBOL=TTF'))
const fromFixml: FixMsg = reader.parseFixmlLine(Buffer.from("<Order ClOrdID='A'/>"))
const fromPairs: FixMsg = reader.parsePairs([['55', 'AAPL']])
const enriched: FixMsg = reader.enrichMessage(fromText)
const enrichedStream: FixMessages = reader.enrichMessages([fromText, enriched])
const restated: FixMsg = fromText.intoLatest()
const readerCopy: FixCodec = reader.clone()

// The lifecycle is a class over one dictionary, or over the process default,
// and the codec runs one over any iterable, lazily.
const lifeClass: typeof FixLifecycle = fix.FixLifecycle
const life: FixLifecycle = new fix.FixLifecycle(loaded)
const defaultLife: FixLifecycle = new fix.FixLifecycle()
const stamped: FixMsg = life.fill(fromText)
const alive: number = life.alive
const lifeRendered: string = life.toString()
life.clear()
const stream: FixMessages = reader.lifecycle([fromText, stamped])
const stampedAgain: FixMessages = reader.lifecycle(stream)

// The Arrow twins take and answer batch readers, and a stream crosses back.
const parsedBatches: BatchReader = reader.parseTextArrowReader(BatchReader.fromIpc(new Uint8Array()))
const filledBatches: BatchReader = reader.enrichMessagesArrowReader(parsedBatches)
const readBackStream: FixMessages = reader.messages(filledBatches)
const rows: BatchReader = reader.arrowReader(field, readBackStream)
const written: number = reader.writeArrowReader(rows, { write(chunk: Uint8Array) { void chunk } })

void pinnedBranch
void pinnedVersion
void pinnedSeparator
void pinnedPayloadColumn
void pinnedNullValues
void pinnedDirection
void pinnedBatchByteSize
void fromLines
void fromFixml
void enriched
void enrichedStream
void restated
void lifeClass
void defaultLife
void alive
void lifeRendered
void stream
void stampedAgain
void written

// @ts-expect-error a lifecycle stamps messages, never lines
life.fill(Buffer.from('8=FIX.4.4|35=0|10=0|'))
// @ts-expect-error the stream is an iterable of messages
reader.lifecycle(fromText)
// @ts-expect-error alive is read, never set
life.alive = 0
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

// Everything the core derives about a message.
const digest: Buffer = fromText.digest()
const ticker: Scalar | null = fromText.symbolTicker()
const clock: Scalar | null = fromText.marketTimestamp()
const partition: Scalar | null = fromText.unixPartition(3600)
const lifted: Scalar | null = fromText.lifted('bidpx')
const liftSource: string | null = fromText.liftSource('bidpx')
const lift: Array<[string, Scalar]> = fromText.lift()
const party: Array<Scalar | null> | null = fromText.party('1')
const regulatory: Scalar | null = fromText.trdRegTimestamp('1')
const anomalies: string[] = fromText.anomalies()
const arrivals: Array<[number, string, string]> = fromText.arrivals()
const wire: Buffer = fromText.intoBytes(124)

void branchName
void branchOrNull
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
void ticker
void clock
void partition
void lifted
void liftSource
void lift
void party
void regulatory
void anomalies
void arrivals
void wire

// @ts-expect-error a fixed schema is built from a registry, never from a number
fix.schema(55)

// Categories contain native Fields; a counter remains scalar beside its list.
const group: Field = loaded.definition('groups', 'parties')
const counter: Field = loaded.definition('fields', 'nopartyids')
const component: Field | null = loaded.getDefinition('components', 'party')
const definitions: IterableIterator<Field> = loaded.definitions('messages')
const definitionAdded: boolean = loaded.addDefinition('components', field)
const previous: Field | null = loaded.insertDefinition('components', field)
loaded.createDefinition('components', field)
const replaced: Field = loaded.updateDefinition('components', field)
const deleted: Field | null = loaded.removeDefinition('components', 'party')
const groupByCounter: Field | null = loaded.getGroupByCounter('453:')
const requiredGroup: Field = loaded.groupByCounter('453:')
const snapshot: string = loaded.intoJson()
const restored: FixRegistry = fix.FixRegistry.fromJson(snapshot)
loaded.withUlbridgeFields()

const order: MsgType = loaded.msgtype('D')
const optionalOrder: MsgType | null = loaded.getMsgtype('newordersingle', '')
const messageTypes: IterableIterator<MsgType> = loaded.msgtypes()
const registered: MsgType = loaded.registerMsgtype('ConfigurationPlugin', 'configurationplugin')
const wireCode: string = order.asStr()
const messageDefinition: Field = order.asField()
const scoped: Field | null = order.getGroupByCounter('453:')
const singletonHash: bigint = order.stableHash()
const singletonEqual: boolean = order.equals(order.clone())
const singletonOrder: number = order.compare(order)

field.fix.counter = 453
const counterTag: number | null = field.fix.counter
field.fix.component = 'party'
field.fix.group = 'parties'
field.fix.fieldRef = 'partyid'
field.fix.msgtype = 'ConfigurationPlugin'
const componentRef: string | null = field.fix.component
const groupRef: string | null = field.fix.group
const fieldRef: string | null = field.fix.fieldRef
const messageCode: string | null = field.fix.msgtype

const selected: UlPlugin = new fix.UlPlugin('d:name=A,type=ConfigurationPlugin', { Name: 'A' }, {})
const configurations: UlPlugins = fix.UlPlugin.fromJsonScalar({ value: { Name: 'A' } })
const parsedConfigurations: UlPlugins = fix.UlPlugin.fromJsonBytes(new Uint8Array())
const nativeConfigurations: UlPlugin[] = [...configurations]
const configAttributes: Scalar = selected.asAttributes()
const configEnvelope: Scalar = selected.asEnvelope()
const configMessage: FixMsg = selected.intoFixmsg(reader)
const recovered: UlPlugin = fix.UlPlugin.fromFixmsg(configMessage)
const configHash: bigint = selected.stableHash()
const bulk: FixMessages = reader.parseUlconfigLine(Buffer.from('{}'))
const decoded: TextLine = handle.readTextLines().next().value
const records: FixMessages = reader.parseTextLine(decoded)
const lineStream: FixMessages = reader.parseTextLines([decoded])
const nextMessage: IteratorResult<FixMsg> = bulk.next()
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
// @ts-expect-error message cursors are constructed by the native codec
new fix.FixMessages()
// @ts-expect-error counter metadata requires a number
field.fix.counter = '453'

void [group, counter, component, definitions, previous, replaced, deleted, groupByCounter,
  requiredGroup, restored, order, optionalOrder, messageTypes, registered, wireCode,
  messageDefinition, scoped, singletonHash, singletonEqual, singletonOrder, counterTag,
  componentRef, groupRef, fieldRef, messageCode, parsedConfigurations, nativeConfigurations,
  configAttributes, configEnvelope, recovered, configHash, nextMessage, allMessages, single]
