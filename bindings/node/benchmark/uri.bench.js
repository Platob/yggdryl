'use strict'

// Fast time + memory benchmark for yggdryl.uri.Uri (runs in ~1-2 s).
//
// Time: Uri.parse is weighed against the built-in WHATWG URL over one URL corpus; fromPath
// and the serializeBytes / deserializeBytes round-trip are yggdryl-only (no built-in single
// call) and reported in Mops/s. Memory: the V8 heapUsed delta per live parsed Uri and per
// serializeBytes buffer, validating the thin wrapper adds no runaway allocation.
//
// Build the addon in RELEASE first (npm run build) — a debug build is meaningless for the
// timings. Run with `--expose-gc` for cleaner memory numbers:
//
//   node --expose-gc bindings/node/benchmark/uri.bench.js

const { uri } = require('..')
const { Uri } = uri

const ITERS = 10_000

const URLS = [
  'https://user:pw@example.com:8080/a/b/c.txt?q=1&x=2#frag',
  'http://example.com/',
  'https://example.com/path/to/archive.tar.gz',
  'ftp://files.example.org:21/pub/readme',
  'http://[::1]:8080/v1/status',
  'postgres://svc:secret@db.internal:5432/app?sslmode=require',
  's3://bucket-name/keys/2026/07/13/object.parquet',
  'mailto:person@example.com',
  'file:///etc/hosts',
  'wss://stream.example.com/socket?token=abcdef#live',
]

const PATHS = [
  'C:\\Users\\alice\\Documents\\report.final.docx',
  'D:\\data\\2026\\input\\records.tar.gz',
  '\\\\server\\share\\team\\notes.txt',
  'src\\bindings\\node\\lib.rs',
  '/usr/local/share/data/set.csv',
  '/var/log/app/service.log.1',
  'E:\\media\\video\\clip.mp4',
  'relative/dir/without/leading/slash',
]

function mopsS(items, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (items * iters) / secs / 1_000_000
}

// Peak V8 heapUsed bytes per object while `count` objects built by `build` stay alive.
function bytesPer(count, build) {
  build() // warm one-time state
  if (global.gc) global.gc()
  const before = process.memoryUsage().heapUsed
  const kept = build()
  const after = process.memoryUsage().heapUsed
  // reference `kept` so V8 cannot collect it before the second sample
  if (kept.length !== count) throw new Error('build size mismatch')
  return (after - before) / count
}

function main() {
  console.log(`yggdryl.uri.Uri — time & memory (${ITERS} iters)\n`)

  // ---- time -----------------------------------------------------------------------
  const yggParse = () => {
    for (const s of URLS) Uri.parse(s)
  }
  const urlParse = () => {
    for (const s of URLS) new URL(s)
  }
  const ygg = mopsS(URLS.length, ITERS, yggParse)
  const std = mopsS(URLS.length, ITERS, urlParse)
  console.log('time (Mops/s):')
  console.log(`  ${'parse (vs URL)'.padEnd(26)} ${ygg.toFixed(2).padStart(7)}   URL ${std.toFixed(2)}   ${(ygg / std).toFixed(2)}x`)

  const uris = URLS.map((s) => Uri.parse(s))
  const encoded = uris.map((u) => u.serializeBytes())
  const ops = [
    ['fromPath', () => PATHS.map((p) => Uri.fromPath(p)), PATHS.length],
    ['serializeBytes', () => uris.map((u) => u.serializeBytes()), uris.length],
    ['deserializeBytes', () => encoded.map((b) => Uri.deserializeBytes(b)), encoded.length],
    ['round-trip', () => uris.map((u) => Uri.deserializeBytes(u.serializeBytes())), uris.length],
  ]
  for (const [name, op, items] of ops) {
    console.log(`  ${name.padEnd(26)} ${mopsS(items, ITERS, op).toFixed(2).padStart(7)}`)
  }

  // ---- memory ---------------------------------------------------------------------
  const n = 20_000
  const perUri = bytesPer(n, () => Array.from({ length: n }, (_, i) => Uri.parse(URLS[i % URLS.length])))
  const perSer = bytesPer(n, () => Array.from({ length: n }, (_, i) => uris[i % uris.length].serializeBytes()))
  console.log('\nmemory (V8 heapUsed delta):')
  console.log(`  ${'bytes / parsed Uri'.padEnd(26)} ${perUri.toFixed(1).padStart(7)}`)
  console.log(`  ${'bytes / serializeBytes'.padEnd(26)} ${perSer.toFixed(1).padStart(7)}`)
}

main()
