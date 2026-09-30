import { Identifier, Identifiers, Scalar } from '..'

const held: Identifier = new Identifier('fix', 'orderid', 'O-1')
const lineage: Identifier = new Identifier('fix', 'clordid', 'C-2')
const security: Identifier = new Identifier('base', 'isin', 'US0378331005')
const keyed: Identifier | null = Identifier.fromKey('ullink.InstrumentId', 'dbi;X')
const parts: string[] = [held.src, held.type, held.value, held.key]
const matched: boolean = held.isOf('fix', 'orderid')
const ordered: number = held.compare(lineage)
const equal: boolean = held.equals(lineage)
const text: string = held.toString()
const ids: Identifiers = new Identifiers([held, security])
const empty: Identifiers = new Identifiers()
const value: string | null = ids.get('isin')
const from: string | null = ids.getFrom('fix', 'orderid')
const found: Identifier | null = ids.getIdentifier('isin')
const kinds: boolean = ids.containsKind('isin')
const ofKind: Identifier[] = ids.ofKind('isin')
const listed: Identifier[] = ids.toArray()
const count: number = ids.length
const same: boolean = ids.equals(empty)
const native: Scalar = Scalar.from(ids)

// @ts-expect-error parts are read-only
held.value = 'O-2'
// @ts-expect-error the key is read-only
held.key = 'fix:orderid'
// @ts-expect-error an identifier states no parentage
held.parent
// @ts-expect-error an identifier states no parentage
held.orig
// @ts-expect-error an identifier is three texts
new Identifier('fix', 'clordid', 'C-2', 'C-1', 'C-0')
// @ts-expect-error a source, a type and a value are needed
new Identifier('fix', 'orderid')

void [lineage, keyed, parts, matched, ordered, equal, text, empty, value, from, found, kinds, ofKind, listed, count, same, native]
