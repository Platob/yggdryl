'use strict'

// What wrapping a caller's own file system costs, beside the calls it wraps.
//
// Every row here is a *wrapper overhead* measurement. The same payload lands
// in the same place twice: once through a Yggdryl handle over an Arrow file
// system handler written against `node:fs`, and once through those same
// `node:fs` calls directly. The direct row is the trusted baseline because it
// is what the handler itself calls - so what the pair isolates is the complete
// filesystem/stream boundary, not the speed of the filesystem underneath.
//
// The ranged-read pair is the one worth reading twice. An Arrow file system
// serves a range without fetching the whole object, and the wrapper is
// supposed to keep that property rather than materialize a value to hand back
// a footer - so a ranged read should cost about what one `fs.readSync` costs,
// not what reading the whole file costs.
//
// The record rows compare the wrapper against the local backend rather than
// against `node:fs`: reading Parquet rows has no `node:fs` spelling, and the
// local handle is the same core code over a mapped file.

const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase } = require('yggdryl')

// A byte round trip is far more work than a path lookup, so this target counts
// in thousands rather than tens of thousands by default.
const iterations = Number.parseInt(
  process.env.YGGDRYL_BENCH_ITERATIONS ?? '2000',
  10,
)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError(
    'YGGDRYL_BENCH_ITERATIONS must be a positive safe integer',
  )
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 100); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

function input(location) {
  const descriptor = fs.openSync(location, 'r')
  let position = 0n
  let closed = false
  const readAt = (offset, length) => {
    const bytes = Buffer.alloc(Number(length))
    return bytes.subarray(
      0,
      fs.readSync(descriptor, bytes, 0, bytes.length, Number(offset)),
    )
  }
  return {
    read(length) {
      const bytes = readAt(position, length)
      position += BigInt(bytes.length)
      return bytes
    },
    readAt,
    seek(offset, whence) {
      const base =
        whence === 'current'
          ? position
          : whence === 'end'
            ? fs.fstatSync(descriptor, { bigint: true }).size
            : 0n
      position = base + offset
      return position
    },
    tell: () => position,
    close() {
      if (!closed) fs.closeSync(descriptor)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

function output(location, append) {
  const descriptor = fs.openSync(location, append ? 'a' : 'w')
  let position = append ? fs.fstatSync(descriptor, { bigint: true }).size : 0n
  let closed = false
  return {
    write(bytes) {
      const written = fs.writeSync(descriptor, bytes)
      position += BigInt(written)
      return BigInt(written)
    },
    tell: () => position,
    flush: () => fs.fsyncSync(descriptor),
    close() {
      if (!closed) fs.closeSync(descriptor)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

const domain = {}

// The complete synchronous handler under measurement, directly over node:fs.
const handler = {
  typeName: 'local',
  domain,
  equals(other) {
    return other?.domain === domain
  },
  normalizePath: path.normalize,
  fileInfo(location) {
    const stats = fs.statSync(location, {
      throwIfNoEntry: false,
      bigint: true,
    })
    if (stats === undefined) return { path: location, kind: 'not-found' }
    if (stats.isDirectory())
      return { path: location, kind: 'directory', mtimeNs: stats.mtimeNs }
    return {
      path: location,
      kind: 'file',
      size: stats.size,
      mtimeNs: stats.mtimeNs,
    }
  },
  list(selector) {
    return fs
      .readdirSync(selector.baseDir, {
        recursive: selector.recursive,
        withFileTypes: true,
      })
      .map((entry) => {
        const full = path.join(entry.parentPath ?? entry.path, entry.name)
        return this.fileInfo(full)
      })
  },
  createDir(location, recursive) {
    fs.mkdirSync(location, { recursive })
  },
  deleteDir: fs.rmdirSync,
  deleteDirContents(location, missingDirOk) {
    if (!fs.existsSync(location) && missingDirOk) return
    for (const name of fs.readdirSync(location)) {
      fs.rmSync(path.join(location, name), { recursive: true })
    }
  },
  deleteRootDirContents() {
    throw new Error('Unsupported: local root deletion')
  },
  deleteFile: fs.unlinkSync,
  copyFile: fs.copyFileSync,
  move: fs.renameSync,
  openInputFile: input,
  openInputStream: input,
  openOutputStream(location) {
    return output(location, false)
  },
  openAppendStream(location) {
    return output(location, true)
  },
}

// Every fixture is built once, outside the measured loops, so the numbers
// report the boundary crossing rather than the corpus.
const PAYLOAD_BYTES = 64 * 1_024
const RANGE_BYTES = 4_096

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fs-bench-'))
const lake = path.join(root, 'lake')
for (const year of ['2024', '2025']) {
  for (const month of ['01', '02']) {
    const leaf = path.join(lake, `year=${year}`, `month=${month}`)
    fs.mkdirSync(leaf, { recursive: true })
    fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
  }
}

const payload = Buffer.alloc(PAYLOAD_BYTES, 'symbol,price\n')
const source = path.join(root, 'source.bin')
fs.writeFileSync(source, payload)
const wrapperSink = path.join(root, 'sink-wrapper.bin')
const directSink = path.join(root, 'sink-direct.bin')

const rows = 10_000
const ids = new Array(rows)
const symbols = new Array(rows)
for (let index = 0; index < rows; index += 1) {
  ids[index] = BigInt(index)
  symbols[index] = index % 2 === 0 ? 'AAPL' : 'MSFT'
}
const table = new arrow.Table({
  id: arrow.vectorFromArray(ids, new arrow.Int64()),
  symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
})
const wrapperRecords = path.join(root, 'wrapper.arrows')
const directRecords = path.join(root, 'direct.arrows')
// Settle the fixture before measuring reads.
const records = IOBase.fromFs(handler, wrapperRecords)
records.overwriteArrowTable(table)
records.flush()
new IOBase(directRecords).overwriteArrowTable(table)

const handle = IOBase.fromFs(handler, source)
const folder = IOBase.fromFs(handler, lake)
const localFolder = new IOBase(lake)

// Construction: what holding a handler costs beside naming a local path.
benchmark('fs/handle_from_path', () => IOBase.fromFs(handler, source))
benchmark('local/handle_from_path', () => new IOBase(source))

// Output bytes are forwarded to the handler's writer as they arrive.
benchmark('fs/write_bytes', () => {
  const sink = IOBase.fromFs(handler, wrapperSink)
  sink.writeBytes(payload)
  sink.flush()
})
benchmark('node:fs/write_bytes', () => fs.writeFileSync(directSink, payload))

benchmark('fs/read_bytes', () => IOBase.fromFs(handler, source).readBytes())
benchmark('node:fs/read_bytes', () => fs.readFileSync(source))

// The row that says whether a range stayed a range.
benchmark('fs/read_range_4k', () => handle.readRangeBytes(0, RANGE_BYTES))
benchmark('node:fs/read_range_4k', () => {
  const stream = input(source)
  try {
    return stream.readAt(0n, BigInt(RANGE_BYTES))
  } finally {
    stream.close()
  }
})

benchmark('fs/size', () => IOBase.fromFs(handler, source).size)
benchmark('node:fs/size', () => fs.statSync(source).size)

benchmark('fs/list_children', () => folder.iterdir())
benchmark('local/list_children', () => localFolder.iterdir())
benchmark('node:fs/list_children', () =>
  fs.readdirSync(lake, { withFileTypes: true }),
)

benchmark('fs/glob_parquet', () => folder.rglob('*.parquet'))
benchmark('node:fs/glob_parquet', () =>
  fs
    .readdirSync(lake, { recursive: true })
    .filter((name) => name.endsWith('.parquet')),
)

// Records: the wrapper against the same core code over a mapped local file.
// A record read is thousands of times the work of a path lookup, so it counts
// in hundreds rather than thousands.
const recordIterations = Math.max(1, Math.floor(iterations / 20))

function recordBenchmark(name, operation) {
  for (let index = 0; index < Math.min(recordIterations, 10); index += 1)
    operation()
  const started = performance.now()
  for (let index = 0; index < recordIterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((recordIterations * rows * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} rows/second`)
}

recordBenchmark('fs/read_records', () =>
  IOBase.fromFs(handler, wrapperRecords).readArrowReader().intoTable(),
)
recordBenchmark('local/read_records', () =>
  new IOBase(directRecords).readArrowReader().intoTable(),
)
recordBenchmark('fs/write_records', () => {
  const sink = IOBase.fromFs(handler, wrapperRecords)
  sink.overwriteArrowTable(table)
  sink.flush()
})
recordBenchmark('local/write_records', () =>
  new IOBase(directRecords).overwriteArrowTable(table),
)

fs.rmSync(root, { recursive: true, force: true })
