import {
  DataType,
  Field,
  fields,
  type AsciiField,
  type BytesDataTypeId,
  type BytesField,
  type BigDecimalField,
  type BbgField,
  type CcyField,
  type DecimalField,
  type DecimalWidthField,
  type FigiField,
  type FixedAsciiField,
  type FixedCp1252Field,
  type FixedUtf8Field,
  type LargeCp1252ViewField,
  type SizedAsciiField,
  type SizedUtf8Field,
  type StringDataTypeId,
  type StringField,
  type GeographyField,
  type GeometryField,
  type Duration32Field,
  type Duration64Field,
  type Int32Field,
  type SerieField,
  type MapField,
  type MediaTypeField,
  type MimeTypeField,
  type RicField,
  type TimezoneField,
  type TimeField,
  type DateTime64Field,
  type UrlField,
  type VariantField,
  type VersionField,
} from '..'

const id: Int32Field = fields.int32('id', { nullable: false })
// `kind` is the coarse family a variant belongs to; `id` is the variant itself.
const idKind: 'integer' = id.dtype.kind
const idId: 'int32' = id.dtype.id
// The exported aliases describe non-null fields, so a factory call that wants
// one has to say so now that the factories default to nullable.
const ids: SerieField<number> = fields.serie('ids', id, { nullable: false })
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
const release: VersionField = fields.version('release', { nullable: false })
// Bare `decimal` is the fixed leaf; a precision names a width.
const price: DecimalField = fields.decimal('price', { nullable: false })
const priceId: 'decimal' = price.dtype.id
const notional: BigDecimalField = fields.bigdecimal('notional', { nullable: false })
const notionalId: 'bigdecimal' = notional.dtype.id
const amount: DecimalWidthField = fields.decimal('amount', 18, 2, { nullable: false })
// @ts-expect-error a precision answers a width, never the fixed leaf
const amountLeaf: DecimalField = fields.decimal('amount', 18, 2, { nullable: false })
// A location column is text, and its values are the canonical URL spelling.
const source: UrlField = fields.url('source', { nullable: false })
const sourceId: 'url' = source.dtype.id
const sourceKind: 'text' = source.dtype.kind
const sourceValue: string = source.defaultJSValue()
const nullableSource: string | null = fields.url('source').defaultJSValue()
// A zone, a MIME type and a media type are canonical text the same way: the
// declared value is the spelling the crate parsed it to.
const zone: TimezoneField = fields.timezone('zone', { nullable: false })
const zoneId: 'timezone' = zone.dtype.id
const zoneValue: string = zone.defaultJSValue()
const mime: MimeTypeField = fields.mimetype('mime', { nullable: false })
const mimeKind: 'text' = mime.dtype.kind
const media: MediaTypeField = fields.mediatype('media', { nullable: false })
const mediaId: 'mediatype' = media.dtype.id
const payloadId: 'variant' = payload.dtype.id
const shape: GeometryField = fields.geometry('shape', { nullable: false })
const shapeKind: 'geospatial' = shape.dtype.kind
const projectedShape: GeometryField = fields.geometry('shape', 'EPSG:3857', {
  nullable: false,
})
const region: GeographyField = fields.geography('region', 'OGC:CRS84', 'vincenty', {
  nullable: false,
})
const ccy: CcyField = fields.ccy('ccy', { nullable: false })
const ccyId: 'ccy' = ccy.dtype.id
const ccyKind: 'code' = ccy.dtype.kind
const ccyValue: string = ccy.defaultJSValue()
const figi: FigiField = fields.figi('figi', { nullable: false })
const figiId: 'figi' = figi.dtype.id
const figiValue: string = figi.defaultJSValue()
const bbg: BbgField = fields.bbg('bbg', { nullable: false })
const bbgId: 'bbg' = bbg.dtype.id
const bbgKind: 'code' = bbg.dtype.kind
const ric: RicField = fields.ric('ric', { nullable: false })
const ricId: 'ric' = ric.dtype.id
const ricKind: 'code' = ric.dtype.kind
// A RIC has no neutral member, so a default is what a nullable column holds.
const nullableRic: string | null = fields.ric('ric').defaultJSValue()
const note: AsciiField = fields.ascii('note', { nullable: false })
const noteId: 'ascii' = note.dtype.id
const noteKind: 'text' = note.dtype.kind
const sized: FixedAsciiField = fields.fixedAscii('code', 12, { nullable: false })
const sizedId: 'fixed_ascii' = sized.dtype.id
const nullableCode: string | null = fields.fixedAscii('code', 3).defaultJSValue()
const padded: FixedUtf8Field = fields.fixedUtf8('label', 8, { nullable: false })
// One factory per leaf: the number is the leaf's own.
const bounded: SizedUtf8Field = fields.sizedUtf8('label', 32, { nullable: false })
const boundedId: 'sized_utf8' = bounded.dtype.id
const ticker: SizedAsciiField = fields.sizedAscii('ticker', 12, { nullable: false })
const legacy: FixedCp1252Field = fields.fixedCp1252('legacy', 8, { nullable: false })
const wide: LargeCp1252ViewField = fields.largeCp1252View('wide', { nullable: false })
const wideId: 'large_cp1252_view' = wide.dtype.id
// The whole family: the parameters ride the options beside the field's own,
// and a charset beside a charset-free spelling lands on that charset's leaf.
const latin: StringField = fields.string('latin', {
  charset: 'windows-1252',
  max: 32,
  nullable: false,
})
const latinId: StringDataTypeId = latin.dtype.id
const latinKind: 'text' = latin.dtype.kind
const latinValue: string = latin.defaultJSValue()
const nullableLatin: string | null = fields.string('latin').defaultJSValue()
const blob: BytesField = fields.bytes('blob', { layout: 'large_binary', nullable: false })
const blobId: BytesDataTypeId = blob.dtype.id
const blobValue: Uint8Array = blob.defaultJSValue()
void ccyId
void ccyKind
void ccyValue
void figi
void figiId
void figiValue
void bbg
void bbgId
void bbgKind
void ric
void ricId
void ricKind
void nullableRic
void note
void noteId
void noteKind
void sized
void sizedId
void nullableCode
void padded
void bounded
void boundedId
void ticker
void legacy
void wide
void wideId
void latin
void latinId
void latinKind
void latinValue
void nullableLatin
void blob
void blobId
void blobValue
void payloadId
void release
void priceId
void notionalId
void amount
void amountLeaf
void source
void sourceId
void sourceKind
void sourceValue
void nullableSource
void zoneId
void zoneValue
void mimeKind
void mediaId
void shapeKind
void projectedShape
void region

const clockType: DataType = DataType.time('milliseconds')
const generic: Field = ids
const genericItem = new Field('item', 'int32', false)
const genericItems: SerieField<unknown> = fields.serie(
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
// @ts-expect-error a Field casts nothing: a column casts, through Serie
id.castArrowArray
// @ts-expect-error the native diff bridge is hidden behind showDiffs
id._showDiffs(fields.int32('native_other'))
// @ts-expect-error metadata values are never string-coerced
id.update(new Map([['attempts', 3]]))
// @ts-expect-error generic time selection requires an explicit unit
fields.time('clock')
// @ts-expect-error a string field's options carry only the family's parameters
fields.string('bad', { width: 4 })
// @ts-expect-error generic time selection requires an explicit unit
DataType.time()
// @ts-expect-error a defaulted factory field is nullable, so its default is not a bare number
const nonNullDefault: number = fields.int32('defaulted').defaultJSValue()
// @ts-expect-error a defaulted factory field does not satisfy the non-null alias
const nonNullAlias: Int32Field = fields.int32('defaulted')
// @ts-expect-error the CurrencyField export was retired with the datatype spelling
const retiredCurrencyField: import('..').CurrencyField = ccy
// @ts-expect-error the currency field factory was retired with the datatype spelling
fields.currency('ccy')

void retiredCurrencyField
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
