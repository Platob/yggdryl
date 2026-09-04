import {
  Field,
  IOBase,
  Scalar,
  Url,
  fix,
  type FixMsg,
  type FixRegistry,
  type FixValueInput,
  type LocationInput,
} from '..'

declare const field: Field
declare const handle: IOBase
declare const url: Url
declare const value: Scalar

// The namespace holds two classes and two functions, and nothing else.
const registryClass: typeof FixRegistry = fix.FixRegistry
const empty: FixRegistry = new fix.FixRegistry()
const built: FixRegistry = fix.FixRegistry.fromFields([field, field])
const location: LocationInput = url
const loaded: FixRegistry = fix.FixRegistry.fromHandle(location)
const fromString: FixRegistry = fix.FixRegistry.fromHandle('file:///lake/fix')
const fromHandle: FixRegistry = fix.FixRegistry.fromHandle(handle)
loaded.writeInto(handle)

void registryClass
void empty
void built
void fromString
void fromHandle

const size: number = loaded.size
const byTag: Field | null = loaded.getFieldByTag(55)
const requiredByTag: Field = loaded.fieldByTag(55)
const byName: Field | null = loaded.getFieldByName('Symbol')
const requiredByName: Field = loaded.fieldByName('Symbol')
const byPath: Field | null = loaded.getFieldByPath('NoPartyIDs.PartyID')
const requiredByPath: Field = loaded.fieldByPath('NoPartyIDs.PartyID')
const byKey: Field | null = loaded.getField(55)
const byNameKey: Field | null = loaded.getField('Symbol')
const requiredByKey: Field = loaded.field('Symbol')
const mapLike: Field | null = loaded.get(55)
const present: boolean = loaded.has('Symbol')
const inserted: Field | null = loaded.insert(field)
loaded.update(field)
const removed: Field | null = loaded.remove(55)
const walk: Generator<Field> = loaded.keys()
const drained: Field[] = [...loaded]
const forOf: Field[] = [...loaded.keys()]
const same: boolean = loaded.equals(built)
const registryHash: bigint = loaded.stableHash()
const copy: FixRegistry = loaded.clone()
const rendered: string = loaded.toString()
const document: unknown[] = loaded.toJSON()

void size
void byTag
void requiredByTag
void byName
void requiredByName
void byPath
void requiredByPath
void byKey
void byNameKey
void requiredByKey
void mapLike
void present
void inserted
void removed
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
loaded.fieldByName(55)

const input: FixValueInput = { Symbol: 'AAPL' }
const message: FixMsg = new fix.FixMsg(field, input)
const explicit: FixMsg = new fix.FixMsg(field, value, loaded)
const nullRegistry: FixMsg = new fix.FixMsg(field, input, null)
const linked: FixRegistry = message.registry
const schema: Field = message.field
const row: Scalar = message.value
const valueCount: number = message.size
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
const tag: number | null = field.fix.tag
field.fix.tag = 55
const tags: number[] = field.fix.tags
field.fix.tags = [1088]
const aliases: string[] = field.fix.aliases
field.fix.aliases = ['Ticker']
const description: string | null = field.fix.description
field.fix.description = 'Ticker symbol.'

void tag
void tags
void aliases
void description

// @ts-expect-error a tag crosses as a number, never a bigint
field.fix.tag = 55n
// @ts-expect-error aliases are strings
field.fix.aliases = [55]
