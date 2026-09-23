'use strict'

const { performance } = require('node:perf_hooks')
const { Serie, fields } = require('yggdryl')

const iterations = Number.parseInt(
  process.env.YGGDRYL_BENCH_ITERATIONS ?? '100000',
  10,
)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

let sink = 0
function observe(value) {
  if (typeof value === 'number') return value
  if (typeof value === 'bigint') return Number(value & 0xffffn)
  if (value === null) return 1
  return value?.length ?? value?.size ?? value?.constructor?.name?.length ?? 0
}

function benchmark(name, count, operation) {
  for (let index = 0; index < Math.min(count, 1_000); index += 1) {
    sink ^= observe(operation())
  }
  const started = performance.now()
  for (let index = 0; index < count; index += 1) sink ^= observe(operation())
  const elapsed = performance.now() - started
  const rate = Math.round((count * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const int64 = fields.int64('id').dtype
const required = fields.int64('id', { nullable: false })
const nullable = fields.utf8('note', { nullable: true })
// Required all the way down, so the nested default is materialized rather
// than the logical null a nullable factory field defaults to.
const child = fields.struct(
  'child',
  [fields.int32('quantity', { nullable: false }), fields.utf8('sku')],
  { nullable: false },
)
const nested = fields.fixedSizeList('children', child, 4, { nullable: false })
const sparkSource = fields.struct('payload', [
  fields.uint8('small'),
  fields.largeUtf8('text'),
  fields.listView('items', fields.float16('item')),
])

// Resolve Apache Arrow JS and its schema support before timing scalar IPC
// projection. Each measured call still owns its documented one-row IPC copy.
Serie.fromDefault(required).intoArrowScalar()

benchmark('defaults/datatype_scalar_js', iterations, () => int64.defaultJSValue())
benchmark('defaults/field_nullable_js', iterations, () => nullable.defaultJSValue())
benchmark('defaults/nested_struct_js', Math.max(1, Math.floor(iterations / 10)), () =>
  nested.defaultJSValue()[3][0],
)
benchmark('defaults/cached_hint', iterations, () => nested.defaultJSHint())
benchmark('defaults/arrow_compat_clone', iterations, () =>
  sparkSource.dtype.intoSchemeCompat('arrow'),
)
benchmark('defaults/spark_nested_normalize', Math.max(1, Math.floor(iterations / 10)), () =>
  sparkSource.intoSchemeCompat('spark'),
)
benchmark('defaults/arrow_scalar_ipc', Math.max(1, Math.floor(iterations / 1_000)), () =>
  Serie.fromDefault(required).intoArrowScalar(),
)

globalThis.__yggdrylDefaultsBenchSink = sink
