'use strict'

const { performance } = require('node:perf_hooks')
const arrow = require('apache-arrow')
const {
  ChunkedSerie,
  DataType,
  Expression,
  Field,
  MarketDataKind,
  MediaType,
  MimeType,
  Scalar,
  Serie,
  SerieReader,
  Side,
  Plan,
  Term,
  StringEnum,
  Version,
  fields,
  iceberg: icebergApi,
  intoField,
} = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '100000', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 1_000); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const id = fields.int32('id', { nullable: false, metadata: { source: 'event' } })
const digestBits = fields.int64('digest', { nullable: false })
const unsignedDigest = arrow.vectorFromArray([2n ** 64n - 1n], new arrow.Uint64())
const name = fields.utf8('name')
const struct = DataType.fromFields([id, name])
const structuralJson = struct.toJSON()
const wideLeft = DataType.fromFields(
  Array.from({ length: 1_024 }, (_, index) =>
    fields.int32(`left_${index.toString().padStart(4, '0')}`),
  ),
)
const wideRight = DataType.fromFields(
  Array.from({ length: 1_024 }, (_, index) =>
    fields.int32(`right_${index.toString().padStart(4, '0')}`),
  ),
)
// The protocol view crosses the boundary once and then answers by bare name,
// so both halves are measured: taking the view, and reading or writing through
// one already held next to the two-argument property call it replaces.
const property = fields.decimal128('price', 18, 6, {
  metadata: {
    'ICEBERG:doc': 'closing price',
    'ICEBERG:field-id': '7',
    'ICEBERG:schema-id': '3',
    'POSTGRES:type': 'numeric(18,6)',
  },
})
const iceberg = property.iceberg
const partitioned = Field.from(
  'row: struct<year: int32 not null, price: float64 not null> not null',
).withPartitionFields(['year'])
const rowField = fields.struct('BenchRow', [id, name], { nullable: false })
const term = new Term('id + 1 > 2')
const expression = new Expression('select id where id > 0')
const plan = new Plan('select id from t where id > 0 limit 10')
const icebergSchema = icebergApi.assignFieldIds(rowField)
const icebergSpec = icebergApi.PartitionSpec.identity(icebergSchema, ['id'], 1)
const icebergSpecDocument = icebergSpec.intoJSON()
class BenchRow {
  static get intoStructField() {
    return rowField
  }
}
// Resolve before timing so this measures the steady cached class accessor.
intoField(BenchRow)

// The prebuilt ISO 4217 listing: what a schema pays once when it declares a
// currency column, and the members every reader of that schema computes.
const ccys = StringEnum.fromLogicalName('ccy')
// The one string datatype with everything declared, and a fixed width the
// datatype answers for.
const latin = DataType.string({ charset: 'windows-1252', max: 32 })
const tenor = DataType.fixedAscii(8)

const knownMime = 'application/json'
const customMime = 'application/vnd.benchmark+json'
const compoundMedia = 'text/csv;encodings=application/gzip,application/zstd'
const contentType = 'text/csv; charset=utf-8'
const contentEncoding = 'gzip, zstd'
const release = new Version(5, 0, 300)
const earlierRelease = new Version(5, 0, 10)
const candidate = new Version(1, 0, '-rc2')
const laterCandidate = new Version(1, 0, '-rc10')
const releaseScalar = Scalar.from(release)
const releaseField = fields.version('release', { nullable: false })
const figi = DataType.from('figi')
const figiField = fields.figi('figi', { nullable: false })
const figiScalar = figi.scalar('BBG000BLNQ16')
// An enum member crosses as its stored name in both directions: the frozen
// objects map a name to its code, a scalar reads back the name.
const side = DataType.from('side')
const sideScalar = side.scalar('BUYS')
const kind = DataType.from('marketdatakind')
const kindScalar = kind.scalar('ORDR')

benchmark('version/native_parts', () => new Version(5, 0, 300))
benchmark('version/native_parse', () => Version.fromStr('5.0.300'))
benchmark('version/native_parts_text_patch', () => new Version(1, 0, '-rc1'))
benchmark('version/native_parse_text_patch', () => Version.fromStr('1.0-rc1'))
benchmark('version/patch', () => release.patch)
benchmark('version/compare', () => earlierRelease.compare(release))
benchmark('version/compare_text_patch', () => candidate.compare(laterCandidate))
benchmark('version/stable_hash', () => release.stableHash())
benchmark('version/clone', () => release.clone())
benchmark('version/into_scalar', () => Scalar.from(release))
benchmark('version/field_into_scalar', () => Scalar.from('5.0.300', { field: releaseField }))
benchmark('version/scalar_as_js', () => releaseScalar.asJs())
benchmark('figi/into_scalar', () => figi.scalar('BBG000BLNQ16'))
benchmark('figi/field_into_scalar', () => Scalar.from('BBG000BLNQ16', { field: figiField }))
benchmark('figi/scalar_as_js', () => figiScalar.asJs())
benchmark('side/member_lookup', () => Side.SELL)
benchmark('side/into_scalar', () => side.scalar('SELL'))
benchmark('side/scalar_as_js', () => sideScalar.asJs())
benchmark('marketdatakind/member_lookup', () => MarketDataKind.QUOT)
benchmark('marketdatakind/into_scalar', () => kind.scalar('QUOT'))
benchmark('marketdatakind/scalar_as_js', () => kindScalar.asJs())

benchmark('schema/from_fields', () => DataType.fromFields([id, name]))
benchmark('serie/from_arrow_array_bits', () =>
  Serie.fromArrowArray(unsignedDigest, digestBits, { representation: 'bits' }),
)
// A chunked column crosses as copied IPC, one batch per Data or per table
// batch; the join, the cast and the way out are measured beside the doors.
const chunkedInt64 = arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64())
const chunkedVector = chunkedInt64.concat(arrow.vectorFromArray([4n, 5n], new arrow.Int64()))
const chunkedTable = new arrow.Table([
  new arrow.Table({ id: chunkedInt64 }).batches[0],
  new arrow.Table({ id: arrow.vectorFromArray([4n, 5n], new arrow.Int64()) }).batches[0],
])
const heldChunked = ChunkedSerie.fromArrowArray(chunkedVector)
const heldChunkedTable = ChunkedSerie.fromArrowBatch(chunkedTable)
const chunkedWide = fields.float64('item', { nullable: false })
benchmark('chunked_serie/from_arrow_array', () => ChunkedSerie.fromArrowArray(chunkedVector))
benchmark('chunked_serie/from_arrow_batch_table', () =>
  ChunkedSerie.fromArrowBatch(chunkedTable),
)
benchmark('chunked_serie/cast', () => heldChunked.cast(chunkedWide))
benchmark('chunked_serie/into_serie', () => heldChunked.intoSerie())
benchmark('chunked_serie/into_arrow_array', () => heldChunked.intoArrowArray())
benchmark('chunked_serie/into_arrow_table', () => heldChunkedTable.intoArrowTable())
benchmark('chunked_serie/child', () => heldChunkedTable.child('id'))
// A held record column streams with no plan compiled; a cast is one plan
// over every record the reader holds, applied as the cast is asked for.
const heldRecords = Serie.fromArrowBatch(chunkedTable)
const heldRecordsWide = fields.struct('row', [fields.float64('id')], { nullable: false })
benchmark('serie_reader/from_serie', () => SerieReader.fromSerie(heldRecords))
benchmark('serie_reader/cast', () => SerieReader.fromSerie(heldRecords).cast(heldRecordsWide))
// Ordering, uniqueness and grouping over a 64-row primitive column, a
// 64-row text column and a record of both; every argument serie is built
// once, so a row measures the verb and its crossing alone. An `as*` write
// runs on a serie of its own, which it leaves in a state the next call
// accepts.
const orderRows = 64
const orderPrices = Serie.fromScalars(
  fields.int64('price'),
  Array.from({ length: orderRows }, (_, index) => (index * 37) % 23),
)
const orderVenues = Serie.fromScalars(
  fields.utf8('venue'),
  Array.from({ length: orderRows }, (_, index) => ['XNAS', 'XNYS', 'XPAR'][index % 3]),
)
const orderQuotes = Serie.fromScalars(
  fields.struct('quote', [fields.utf8('venue'), fields.int64('price')], { nullable: false }),
  Array.from({ length: orderRows }, (_, index) => ({
    venue: ['XNAS', 'XNYS', 'XPAR'][index % 3],
    price: index % 5,
  })),
)
const orderIndices = Serie.fromScalars(
  fields.uint32('index', { nullable: false }),
  Array.from({ length: orderRows }, (_, index) => orderRows - 1 - index),
)
const orderMask = Serie.fromScalars(
  fields.boolean('keep', { nullable: false }),
  Array.from({ length: orderRows }, () => true),
)
const descending = { descending: true }
benchmark('serie/sort_indices', () => orderPrices.sortIndices())
benchmark('serie/sort_indices_utf8', () => orderVenues.sortIndices(descending))
benchmark('serie/is_sorted', () => orderPrices.isSorted())
benchmark('serie/is_unique', () => orderVenues.isUnique())
benchmark('serie/unique_count', () => orderVenues.uniqueCount())
benchmark('serie/memory_size', () => orderPrices.memorySize())
benchmark('serie/into_sorted', () => orderPrices.intoSorted())
benchmark('serie/into_unique', () => orderVenues.intoUnique())
benchmark('serie/into_reversed', () => orderPrices.intoReversed())
benchmark('serie/into_taken', () => orderPrices.intoTaken(orderIndices))
benchmark('serie/into_filtered', () => orderPrices.intoFiltered(orderMask))
benchmark('serie/partition_by', () => orderPrices.partitionBy(orderVenues))
benchmark('serie/partition_by_paths', () => orderQuotes.partitionByPaths(['venue', 'price']))
const sortedInPlace = orderPrices.intoSorted()
benchmark('serie/as_sorted', () => sortedInPlace.asSorted())
const uniqueInPlace = orderVenues.intoUnique()
benchmark('serie/as_unique', () => uniqueInPlace.asUnique())
const reversedInPlace = orderPrices.intoSorted()
benchmark('serie/as_reversed', () => reversedInPlace.asReversed())
const takenInPlace = orderPrices.intoSorted()
benchmark('serie/as_taken', () => takenInPlace.asTaken(orderIndices))
const filteredInPlace = orderPrices.intoSorted()
benchmark('serie/as_filtered', () => filteredInPlace.asFiltered(orderMask))
benchmark('serie/window', () => orderPrices.window(8, 32))
// A window reads and writes through the serie it holds at each call.
const windowed = orderPrices.intoSorted()
const window = windowed.window(8, 32)
const windowIndices = Serie.fromScalars(
  fields.uint32('index', { nullable: false }),
  Array.from({ length: 32 }, (_, index) => 31 - index),
)
const windowMask = Serie.fromScalars(
  fields.boolean('keep', { nullable: false }),
  Array.from({ length: 32 }, () => true),
)
const windowKeys = orderVenues.slice(8, 32)
const windowSource = orderPrices.window(0, 32)
const windowRows = Array.from({ length: 4 }, (_, index) => Scalar.from(index))
const windowValue = Scalar.from(7)
benchmark('serie_slice/null_count', () => window.nullCount())
benchmark('serie_slice/scalar', () => window.scalar(3))
benchmark('serie_slice/rows', () => window.rows())
benchmark('serie_slice/memory_size', () => window.memorySize())
benchmark('serie_slice/is_sorted', () => window.isSorted())
benchmark('serie_slice/is_unique', () => window.isUnique())
benchmark('serie_slice/unique_count', () => window.uniqueCount())
benchmark('serie_slice/sort_indices', () => window.sortIndices())
benchmark('serie_slice/window', () => window.window(4, 8))
benchmark('serie_slice/into_serie', () => window.intoSerie())
benchmark('serie_slice/into_sorted', () => window.intoSorted(descending))
benchmark('serie_slice/into_unique', () => window.intoUnique())
benchmark('serie_slice/into_reversed', () => window.intoReversed())
benchmark('serie_slice/into_taken', () => window.intoTaken(windowIndices))
benchmark('serie_slice/into_filtered', () => window.intoFiltered(windowMask))
benchmark('serie_slice/partition_by', () => window.partitionBy(windowKeys))
benchmark('serie_slice/set', () => window.set(3, windowValue))
benchmark('serie_slice/fill', () => window.fill(windowValue))
benchmark('serie_slice/swap', () => window.swap(0, 31))
benchmark('serie_slice/copy_from', () => window.copyFrom(windowSource))
benchmark('serie_slice/splice', () => window.splice(4, 8, windowRows))
benchmark('serie_slice/as_sorted', () => window.asSorted())
benchmark('serie_slice/as_reversed', () => window.asReversed())
benchmark('serie_slice/as_taken', () => window.asTaken(windowIndices))
// Across the chunks: chunk by chunk where a chunk can answer alone, through
// the one join where the rows must be seen together.
const orderChunked = ChunkedSerie.fromSeries(
  [orderPrices.slice(0, 32), orderPrices.slice(32, 32)],
  orderPrices.field,
)
benchmark('chunked_serie/sort_indices', () => orderChunked.sortIndices())
benchmark('chunked_serie/is_sorted', () => orderChunked.isSorted())
benchmark('chunked_serie/is_unique', () => orderChunked.isUnique())
benchmark('chunked_serie/unique_count', () => orderChunked.uniqueCount())
benchmark('chunked_serie/memory_size', () => orderChunked.memorySize())
benchmark('chunked_serie/into_sorted', () => orderChunked.intoSorted())
benchmark('chunked_serie/into_unique', () => orderChunked.intoUnique())
benchmark('chunked_serie/into_reversed', () => orderChunked.intoReversed())
benchmark('chunked_serie/into_taken', () => orderChunked.intoTaken(orderIndices))
benchmark('chunked_serie/into_filtered', () => orderChunked.intoFiltered(orderMask))
benchmark('chunked_serie/partition_by', () => orderChunked.partitionBy(orderVenues))
const orderChunkedVenues = ChunkedSerie.fromSeries(
  [orderVenues.slice(0, 32), orderVenues.slice(32, 32)],
  orderVenues.field,
)
benchmark('chunked_serie/partition_by_chunked_keys', () =>
  orderChunked.partitionBy(orderChunkedVenues),
)
const chunkedInPlace = orderChunked.intoSorted()
benchmark('chunked_serie/as_sorted', () => chunkedInPlace.asSorted())
benchmark('chunked_serie/as_unique', () => chunkedInPlace.asUnique())
benchmark('chunked_serie/as_reversed', () => chunkedInPlace.asReversed())
const chunkedTakenInPlace = orderChunked.intoSorted()
benchmark('chunked_serie/as_taken', () => chunkedTakenInPlace.asTaken(orderIndices))
const chunkedFilteredInPlace = orderChunked.intoSorted()
benchmark('chunked_serie/as_filtered', () => chunkedFilteredInPlace.asFiltered(orderMask))
benchmark('schema/map_of', () => fields.mapOf('labels', 'utf8', 'int32'))
benchmark('schema/time_infer_time32', () => DataType.time('ms'))
benchmark('schema/time_infer_time64', () => DataType.time('ns'))
benchmark('schema/variant_dense_2', () => DataType.variant([id, name]))
benchmark('schema/fixed_ascii', () => DataType.fixedAscii(3))
benchmark('schema/string_parameters', () =>
  DataType.string({ charset: 'windows-1252', max: 32 }),
)
benchmark('schema/bytes_parameters', () => DataType.bytes({ max: 16 }))
benchmark('schema/string_parameters_get', () => latin.stringParameters)
benchmark('schema/fixed_byte_width', () => tenor.fixedByteWidth)
benchmark('schema/ccy', () => DataType.from('ccy'))
benchmark('schema/ccy_field', () => fields.ccy('ccy'))
benchmark('schema/fixed_ascii_field', () => fields.fixedAscii('tenor', 8))
benchmark('schema/string_field', () =>
  fields.string('note', { charset: 'windows-1252', max: 32 }),
)
benchmark('schema/bytes_field', () => fields.bytes('payload', { max: 16 }))
benchmark('schema/time_field_infer_time32', () => fields.time('clock', 'ms'))
benchmark('schema/time_field_infer_time64', () => fields.time('clock', 'ns'))
benchmark('schema/from_json', () => DataType.fromJSON(structuralJson))
benchmark('schema/into_field_native', () => intoField(rowField))
benchmark('schema/into_field_class_cached', () => intoField(BenchRow))
benchmark('schema/into_field_renamed', () => intoField(BenchRow, 'row'))
benchmark('schema/metadata_ignored_equals', () => struct.equals(struct, false))
benchmark('term/equals', () => term.equals(term))
benchmark('term/compare', () => term.compare(term))
benchmark('term/stable_hash', () => term.stableHash())
benchmark('term/clone', () => term.clone())
benchmark('expression/equals', () => expression.equals(expression))
benchmark('expression/compare', () => expression.compare(expression))
benchmark('expression/stable_hash', () => expression.stableHash())
benchmark('expression/clone', () => expression.clone())
benchmark('plan/equals', () => plan.equals(plan))
benchmark('plan/compare', () => plan.compare(plan))
benchmark('plan/stable_hash', () => plan.stableHash())
benchmark('plan/clone', () => plan.clone())
benchmark('iceberg/partition_spec_equals', () => icebergSpec.equals(icebergSpec))
benchmark('iceberg/partition_spec_compare', () => icebergSpec.compare(icebergSpec))
benchmark('iceberg/partition_spec_stable_hash', () => icebergSpec.stableHash())
benchmark('iceberg/partition_spec_clone', () => icebergSpec.clone())
benchmark('iceberg/partition_spec_from_json', () =>
  icebergApi.PartitionSpec.fromJSON(icebergSpecDocument),
)
benchmark('iceberg/partition_spec_into_json', () => icebergSpec.intoJSON())
benchmark('schema/diff_first_wide_struct_1024', () =>
  wideLeft.showDiffs(wideRight, false).next(),
)
benchmark('schema/protocol_view', () => property.iceberg)
benchmark('schema/protocol_view_get', () => iceberg.get('doc'))
benchmark('schema/protocol_property_get', () =>
  property.getProperty('iceberg', 'doc'),
)
benchmark('schema/protocol_view_set', () => iceberg.set('doc', 'closing price'))
benchmark('schema/protocol_view_entries', () => iceberg.entries())
benchmark('schema/partition_field_names', () => partitioned.partitionFieldNames())
// The `by` declarations: a partition declaration materialized on a struct
// root, read back, and the typed list on a protocol view.
const declaring = Field.from('row: struct<venue: utf8 not null, ts: timestamp(us) not null> not null')
const declared = declaring.withPartitionBy(['venue', 'days(ts)'])
const sortView = declaring.clone().sort
sortView.by = ['venue', 'ts desc']
benchmark('schema/with_partition_by', () => declaring.withPartitionBy(['venue', 'days(ts)']))
benchmark('schema/partition_by', () => declared.partitionBy())
benchmark('schema/protocol_by_get', () => sortView.by)
benchmark('schema/protocol_by_set', () => {
  sortView.by = ['venue', 'ts desc']
})
benchmark('schema/protocol_remove_by', () => declared.clone().partition.removeBy())
benchmark('schema/without_partition_fields', () =>
  partitioned.withoutPartitionFields(),
)
benchmark('schema/string_vocabulary_prebuilt', () =>
  StringEnum.fromLogicalName('ccy'),
)
benchmark('schema/string_vocabulary_enum', () => ccys.intoEnum('ccy'))
benchmark('schema/mime_known_parse', () => MimeType.fromString(knownMime))
benchmark('schema/mime_custom_parse', () => MimeType.fromString(customMime))
benchmark('schema/media_compound_parse', () => MediaType.fromString(compoundMedia))
benchmark('schema/media_header_inference', () =>
  MediaType.fromContentHeaders(contentType, contentEncoding),
)
