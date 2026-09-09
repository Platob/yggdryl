import { Scalar, Version, fields, type VersionField } from '../..'

const value: Version = new Version(5, 0, 300)
const parsed: Version = Version.fromStr('5.0.300')
const parts: number[] = [value.major, value.minor, value.patch]
const ordered: number = value.compare(parsed)
const equal: boolean = value.equals(parsed)
const hash: bigint = value.stableHash()
const clone: Version = value.clone()
const text: string = value.toString()
const json: string = value.toJSON()
const field: VersionField = fields.version('release', { nullable: false })
const defaultValue: Version = field.defaultJSValue()
const optionalValue: Version | null = fields.version('release').defaultJSValue()
const native: Scalar = Scalar.fromJs(value)

// @ts-expect-error numeric parts are read-only
value.patch = 7
// @ts-expect-error native versions have no tag
value.tag
// @ts-expect-error constructor components are numeric
new Version('5')

void [parts, ordered, equal, hash, clone, text, json, defaultValue, optionalValue, native]
