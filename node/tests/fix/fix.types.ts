import {
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
  type FixValueInput,
  type LocationInput,
} from '../..'

declare const field: Field
declare const handle: IOBase
declare const url: Url
declare const value: Scalar

// The namespace holds two classes, two functions and the three constants.
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
const byPath: Field | null = loaded.getFieldByPath('NoPartyIDs.PartyID', '')
const requiredByPath: Field = loaded.fieldByPath('NoPartyIDs.PartyID', '')
const bytesProtocol: MimeType = MimeType.inferBytes(Buffer.from('35=D|'))
const textProtocol: MimeType = MimeType.inferText('35=D|')
const bytesMsgtype: Buffer | null = MimeType.inferBytesMsgtype(Buffer.from('35=D|'))
const textMsgtype: string | null = MimeType.inferTextMsgtype('35=D|')
const byKey: Field | null = loaded.getField(55)
const byNameKey: Field | null = loaded.getField('Symbol')
const requiredByKey: Field = loaded.field('Symbol')
const mapLike: Field | null = loaded.get(55)
const present: boolean = loaded.has('Symbol')
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
const document: unknown[] = loaded.toJSON()

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
const valueByPath: Scalar | null = message.getByPath('NoPartyIDs.0.PartyID')
const requiredValueByPath: Scalar = message.byPath('NoPartyIDs.0.PartyID')
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

// @ts-expect-error a message value is not a bare string
new fix.FixMsg(field, 'AAPL')
// @ts-expect-error a message key is a tag or a name
message.at(55n)

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

// The reader is a class over one dictionary, with every pin optional.
const readerClass: typeof FixCodec = fix.FixCodec
const reader: FixCodec = new fix.FixCodec(loaded)
const pinned: FixCodec = new fix.FixCodec(loaded, {
  branch: 'cme',
  version: '4.4',
  nullValues: ['<none>'],
})
// @ts-expect-error the source and target pins are one `version`
const stalePin: FixCodec = new fix.FixCodec(loaded, { sourceVersion: '4.2' })
void stalePin
const readRegistry: FixRegistry = reader.registry
const fromText: FixMsg = reader.transformLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromBytes: FixMsg = reader.transformLine(Buffer.from('8=FIX.4.4|35=D|10=0|'))
const fromFrame: FixMsg = reader.transformFixLine(Buffer.from('8=FIX.4.4'), 1)
const fromBridge: FixMsg = reader.transformUllinkLine(Buffer.from('#SYMBOL=TTF'))
const fromPairs: FixMsg = reader.transformPairs([['55', 'AAPL']])
const enriched: FixMsg = reader.enrichFixmsg(fromText)
const readerCopy: FixCodec = reader.clone()

// The lifecycle is a class over one dictionary, or over the process default,
// and the reader runs one over an array.
const lifeClass: typeof FixLifecycle = fix.FixLifecycle
const life: FixLifecycle = new fix.FixLifecycle(loaded)
const defaultLife: FixLifecycle = new fix.FixLifecycle()
const stamped: FixMsg = life.fill(fromText)
const alive: number = life.alive
const lifeRendered: string = life.toString()
life.clear()
const stream: FixMsg[] = reader.lifecycle([fromText, stamped])

void enriched
void lifeClass
void defaultLife
void alive
void lifeRendered
void stream

// @ts-expect-error a lifecycle stamps messages, never lines
life.fill(Buffer.from('8=FIX.4.4|35=0|10=0|'))
// @ts-expect-error the stream is an array of messages
reader.lifecycle(fromText)
// @ts-expect-error alive is read, never set
life.alive = 0

// The fixed row is a schema, and a column is the folded name of its field.
const fixedSchema: Field = fix.schema(loaded, 'FixMessage')
const carried: Field = fix.schemaCarrying(field, fixedSchema)
const at: number | null = fixedSchema.indexOf('msgtype')
const fixedRow: Scalar = fromText.toRow(fixedSchema)
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
const arrivals: Array<[number, number, string, string]> = fromText.arrivals()
const wire: Buffer = fromText.toBytes(124)

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
