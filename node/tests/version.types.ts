import { Scalar, Version, fields, type VersionField } from '..'

const value: Version = new Version(5, 0, 300)
const parsed: Version = Version.fromStr('5.0.300')
const parts: number[] = [value.major, value.minor]
const patch: string | null = value.patch
const textual: Version = new Version(1, 0, '-rc1')
const ordered: number = value.compare(parsed)
const equal: boolean = value.equals(parsed)
const hash: bigint = value.stableHash()
const clone: Version = value.clone()
const text: string = value.toString()
const json: string = value.toJSON()
const field: VersionField = fields.version('release', { nullable: false })
const defaultValue: Version = field.defaultJSValue()
const optionalValue: Version | null = fields.version('release').defaultJSValue()
const native: Scalar = Scalar.from(value)

// @ts-expect-error parts are read-only
value.patch = '7'
// @ts-expect-error native versions have no tag
value.tag
// @ts-expect-error the major is numeric
new Version('5')
// @ts-expect-error a patch is a number or text
new Version(5, 0, 1n)

void [parts, patch, textual, ordered, equal, hash, clone, text, json, defaultValue, optionalValue, native]
