import { Identifier, Identifiers, Scalar } from '..'

const held: Identifier = new Identifier('orderid', 'O-1')
const lineage: Identifier = new Identifier('oms:clordid', 'C-2')
const security: Identifier = new Identifier('isin', 'US0378331005')
const keyed: Identifier | null = Identifier.fromKey('ullink.InstrumentId', 'dbi;X')
const parts: string[] = [held.src, held.type, held.value, held.key]
const ordered: number = held.compare(lineage)
const equal: boolean = held.equals(lineage)
const text: string = held.toString()
const ids: Identifiers = new Identifiers([held, security])
const empty: Identifiers = new Identifiers()
const value: string | null = ids.get('isin')
const from: string | null = ids.getFrom('orderid')
const derived: boolean = ids.isDerived('cusip')
const kinds: boolean = ids.containsKind('isin')
const ofKind: Identifier[] = ids.ofKind('isin')
const listed: Identifier[] = ids.toArray()
const count: number = ids.length
const same: boolean = ids.equals(empty)
const object: Record<string, string> = ids.intoObject()
const read: Identifiers = Identifiers.fromObject({ 'ullink:isin': 'US0378331005' })
const native: Scalar = Scalar.from(ids)

// @ts-expect-error parts are read-only
held.value = 'O-2'
// @ts-expect-error the key is read-only
held.key = 'orderid'
// @ts-expect-error an identifier states no parentage
held.parent
// @ts-expect-error no key reading is a method of an identifier
held.isOf('fix', 'orderid')
// @ts-expect-error an identifier is a key and a value
new Identifier('fix', 'clordid', 'C-2')
// @ts-expect-error a key and a value are needed
new Identifier('orderid')
// @ts-expect-error a key reads as text
ids.getFrom('fix', 'orderid')

void [lineage, keyed, parts, ordered, equal, text, empty, value, from, derived, kinds, ofKind, listed, count, same, object, read, native]
