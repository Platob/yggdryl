'use strict'

const { performance } = require('node:perf_hooks')
const {
  AsciiDictionary,
  DataType,
  Expression,
  Field,
  MediaType,
  MimeType,
  Statement,
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
    'iceberg:doc': 'closing price',
    'iceberg:field-id': '7',
    'iceberg:schema-id': '3',
    'postgres:type': 'numeric(18,6)',
  },
})
const iceberg = property.iceberg
const partitioned = Field.from(
  'row: struct<year: int32 not null, price: float64 not null> not null',
).withPartitionFields(['year'])
const rowField = fields.struct('BenchRow', [id, name], { nullable: false })
const expression = new Expression('id + 1 > 2')
const statement = new Statement('select id where id > 0')
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

const currencies = AsciiDictionary.fromValues('ascii32', ['USD', 'EUR', 'JPY', 'GBP'])
// One bounded column per call: this measures the encode plus the copied Arrow
// IPC crossing, not Arrow JS materialization of a large table.
const currencyRows = Array.from({ length: 64 }, (_, index) =>
  currencies.values()[index % currencies.length],
)

const knownMime = 'application/json'
const customMime = 'application/vnd.benchmark+json'
const compoundMedia = 'text/csv;encodings=application/gzip,application/zstd'
const contentType = 'text/csv; charset=utf-8'
const contentEncoding = 'gzip, zstd'

benchmark('schema/from_fields', () => DataType.fromFields([id, name]))
benchmark('schema/map_of', () => fields.mapOf('labels', 'utf8', 'int32'))
benchmark('schema/time_infer_time32', () => DataType.time('ms'))
benchmark('schema/time_infer_time64', () => DataType.time('ns'))
benchmark('schema/variant_dense_2', () => DataType.variant([id, name]))
benchmark('schema/ascii_infer', () => DataType.ascii(3))
benchmark('schema/currency', () => DataType.from('currency'))
benchmark('schema/ascii_field', () => fields.ascii32('ccy'))
benchmark('schema/time_field_infer_time32', () => fields.time('clock', 'ms'))
benchmark('schema/time_field_infer_time64', () => fields.time('clock', 'ns'))
benchmark('schema/from_json', () => DataType.fromJSON(structuralJson))
benchmark('schema/into_field_native', () => intoField(rowField))
benchmark('schema/into_field_class_cached', () => intoField(BenchRow))
benchmark('schema/into_field_renamed', () => intoField(BenchRow, 'row'))
benchmark('schema/metadata_ignored_equals', () => struct.equals(struct, false))
benchmark('expression/equals', () => expression.equals(expression))
benchmark('expression/compare', () => expression.compare(expression))
benchmark('expression/stable_hash', () => expression.stableHash())
benchmark('expression/clone', () => expression.clone())
benchmark('statement/equals', () => statement.equals(statement))
benchmark('statement/compare', () => statement.compare(statement))
benchmark('statement/stable_hash', () => statement.stableHash())
benchmark('statement/clone', () => statement.clone())
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
benchmark('schema/without_partition_fields', () =>
  partitioned.withoutPartitionFields(),
)
benchmark('schema/ascii_dictionary_push_hit', () => currencies.push('EUR'))
benchmark('schema/ascii_dictionary_enum', () => currencies.intoEnum('Currency'))
benchmark('schema/ascii_dictionary_arrow_column_64', () =>
  currencies.intoArrowArray(currencyRows),
)
benchmark('schema/mime_known_parse', () => MimeType.fromString(knownMime))
benchmark('schema/mime_custom_parse', () => MimeType.fromString(customMime))
benchmark('schema/media_compound_parse', () => MediaType.fromString(compoundMedia))
benchmark('schema/media_header_inference', () =>
  MediaType.fromContentHeaders(contentType, contentEncoding),
)
