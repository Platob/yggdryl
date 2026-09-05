import {
  DataType,
  Field,
  fields,
  type AsciiField,
  type CurrencyField,
  type FixedAsciiField,
  type GeographyField,
  type GeometryField,
  type Duration32Field,
  type Duration64Field,
  type Int32Field,
  type ListField,
  type MapField,
  type TimeField,
  type DateTime64Field,
  type VariantField,
} from '../..'

const id: Int32Field = fields.int32('id', { nullable: false })
// `kind` is the coarse family a variant belongs to; `id` is the variant itself.
const idKind: 'integer' = id.dtype.kind
const idId: 'int32' = id.dtype.id
// The exported aliases describe non-null fields, so a factory call that wants
// one has to say so now that the factories default to nullable.
const ids: ListField<number> = fields.list('ids', id, { nullable: false })
const eventTime: DateTime64Field = fields.datetime64(
  'event_time',
  'us',
  'Europe/Paris',
  { nullable: false, metadata: new Map([['unit', 'event']]) },
)
const labels: MapField = fields.mapOf('labels', 'utf8', 'int32', true, {
  nullable: false,
})
const clock: TimeField = fields.time('clock', 'us', { nullable: false })
const shortDuration: Duration32Field = fields.duration32('short', 'ms', {
  nullable: false,
})
const longDuration: Duration64Field = fields.duration64('long', 'us', {
  nullable: false,
})
const payload: VariantField = fields.variant('payload', { nullable: false })
const payloadId: 'variant' = payload.dtype.id
const shape: GeometryField = fields.geometry('shape', { nullable: false })
const shapeKind: 'geospatial' = shape.dtype.kind
const projectedShape: GeometryField = fields.geometry('shape', 'EPSG:3857', {
  nullable: false,
})
const region: GeographyField = fields.geography('region', 'OGC:CRS84', 'vincenty', {
  nullable: false,
})
const currency: CurrencyField = fields.currency('ccy', { nullable: false })
const currencyId: 'currency' = currency.dtype.id
const currencyKind: 'ascii' = currency.dtype.kind
const currencyValue: string = currency.defaultJSValue()
const note: AsciiField = fields.ascii('note', { nullable: false })
const noteId: 'ascii' = note.dtype.id
const sized: FixedAsciiField = fields.fixedAscii('code', 12, { nullable: false })
const nullableCode: string | null = fields.fixedAscii('code', 3).defaultJSValue()
void currencyId
void currencyKind
void currencyValue
void note
void noteId
void sized
void nullableCode
void payloadId
void shapeKind
void projectedShape
void region
const clockType: DataType = DataType.time('milliseconds')
const generic: Field = ids
const genericItem = new Field('item', 'int32', false)
const genericItems: ListField<unknown> = fields.list(
  'generic_items',
  genericItem,
  { nullable: false },
)
const structType: DataType = DataType.fromFields(
  (function* children() { yield id })(),
)
const differences: IterableIterator<string> = id.showDiffs(
  fields.int32('other'),
  false,
)

// A factory call that says nothing about nullability yields a nullable field,
// matching the Python factories, so its default value includes the null case.
const defaulted = fields.int32('defaulted')
const defaultedValue: number | null = defaulted.defaultJSValue()

// @ts-expect-error internal factory bridges are not part of the package API
DataType._simple('int32')
// @ts-expect-error the native diff bridge is hidden behind showDiffs
id._showDiffs(fields.int32('native_other'))
// @ts-expect-error metadata values are never string-coerced
id.update(new Map([['attempts', 3]]))
// @ts-expect-error generic time selection requires an explicit unit
fields.time('clock')
// @ts-expect-error generic time selection requires an explicit unit
DataType.time()
// @ts-expect-error a defaulted factory field is nullable, so its default is not a bare number
const nonNullDefault: number = fields.int32('defaulted').defaultJSValue()
// @ts-expect-error a defaulted factory field does not satisfy the non-null alias
const nonNullAlias: Int32Field = fields.int32('defaulted')

void idKind
void idId
void eventTime
void labels
void clock
void shortDuration
void longDuration
void clockType
void generic
void genericItems
void structType
void differences
void defaultedValue
void nonNullDefault
void nonNullAlias
