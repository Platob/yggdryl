'use strict'

const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const { IOBase, Timezone, Url } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '20000', 10)
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

// One small Hive-partitioned lake, prepared outside every measured loop so the
// numbers report the boundary crossing rather than the fixture.
const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-bench-')), 'lake')
for (const year of ['2024', '2025']) {
  for (const month of ['01', '02']) {
    const leaf = path.join(root, `year=${year}`, `month=${month}`)
    fs.mkdirSync(leaf, { recursive: true })
    fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
  }
}

const lake = new IOBase(root)
const leaf = lake.joinpath('year=2024', 'month=01', 'part-0.parquet')
const memory = IOBase.fromBytes(Buffer.alloc(64 * 1_024))
const payload = Buffer.from('symbol,price\n')
const url = Url.fromString('file:///lake/year=2024/month=01/part-0.parquet')
const zone = Timezone.fromString('America/New_York')
const epoch = 1_720_000_000

benchmark('io/handle_from_path', () => new IOBase(root))
benchmark('io/handle_from_url', () => IOBase.from(url))
benchmark('io/join_child', () => lake.joinpath('year=2024', 'month=01'))
benchmark('io/list_children', () => lake.iterdir())
benchmark('io/glob_fixed_prefix', () => lake.glob('year=2024/**/*.parquet'))
benchmark('io/children_where', () => lake.childrenWhere({ year: '2024' }))
benchmark('io/partitions', () => leaf.partitions)
benchmark('io/read_leaf_bytes', () => leaf.readBytes())
benchmark('io/memory_pread_4k', () => memory.pread(0, 4_096))
benchmark('io/memory_pwrite', () => memory.pwrite(0, payload))
benchmark('uri/url_parts', () => url.parts)
benchmark('uri/url_parents', () => url.parents)
benchmark('uri/url_match', () => url.match('lake/**/part-?.parquet'))
benchmark('uri/url_partitions', () => url.partitions)
benchmark('uri/url_relative_to', () => url.relativeTo('file:///lake'))
benchmark('timezone/parse_alias', () => Timezone.fromString('US/Eastern'))
benchmark('timezone/offset_at', () => zone.offsetAt(epoch))
benchmark('timezone/abbreviation_at', () => zone.abbreviationAt(epoch))

fs.rmSync(path.dirname(root), { recursive: true, force: true })
