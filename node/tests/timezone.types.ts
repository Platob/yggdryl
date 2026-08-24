import { Timezone, type TimezoneAlias, type TimezoneInput } from '..'

const input: TimezoneInput = 'US/Eastern'
const zone = new Timezone(input)
const inferred: Timezone = Timezone.from(zone)
const parsed: Timezone = Timezone.fromString('Europe/Paris')
const offsetZone: Timezone = Timezone.fromOffset(5 * 3600 + 1800)
const utc: Timezone = Timezone.UTC

const registered: Timezone[] = Timezone.registered()
const aliases: TimezoneAlias[] = Timezone.aliases()
const alias: string = aliases[0].alias
const canonical: string = aliases[0].canonical

const key: string = zone.key
const isUtc: boolean = zone.isUtc()
const isKnown: boolean = zone.isKnown()
const isFixed: boolean = zone.isFixed()
const observesSaving: boolean = zone.observesSaving()
const offset: number | null = zone.offsetAt(1_720_000_000)
const standardOffset: number | null = zone.standardOffset
const isSaving: boolean | null = zone.isSavingAt(1_720_000_000)
const abbreviation: string | null = zone.abbreviationAt(1_720_000_000)
const local: number = zone.intoLocal(1_720_000_000)
const instant: number = zone.intoUtc(local)
const jsOffset: number | null = zone.getTimezoneOffset(1_720_000_000)

const equals: boolean = zone.equals(parsed)
const compared: number = zone.compare(parsed)
const hash: bigint = zone.stableHash()
const cloned: Timezone = zone.clone()
const printed: string = zone.toString()
const serialized: string = zone.toJSON()

void inferred
void parsed
void offsetZone
void utc
void registered
void alias
void canonical
void key
void isUtc
void isKnown
void isFixed
void observesSaving
void offset
void standardOffset
void isSaving
void abbreviation
void instant
void jsOffset
void equals
void compared
void hash
void cloned
void printed
void serialized
