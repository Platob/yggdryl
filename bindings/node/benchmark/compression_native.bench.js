'use strict'

// yggdryl.compression vs Node's built-in zlib (runs in ~2-4 s).
//
// The point: yggdryl's gzip/zlib run on flate2's `zlib-rs` backend (a pure-Rust port of the
// SIMD-tuned zlib-ng); Node's `node:zlib` links the C `zlib`. At a matched level the streams
// are byte-compatible, so this measures only throughput — and with the napi boundary already
// zero-copy (`Buffer` derefs to `&[u8]`), yggdryl's deflate out-compresses C zlib. Its
// pure-Rust inflate still trails C zlib on decompress — reported honestly. `zstd` is compared
// only when this Node exposes `zlib.zstdCompressSync` (Node >= 22.15 / 23.8).
//
// Build the addon in RELEASE first (npm run build). Run with:
//
//   node bindings/node/benchmark/compression_native.bench.js

const zlibNative = require('node:zlib')
const { compression } = require('..')
const { Gzip, Zlib, Zstd } = compression

// A realistic, semi-compressible corpus: repeated JSON-ish records with varying fields.
const records = []
for (let i = 0; i < 24_000; i++) {
  records.push(
    `{"id":${i},"ts":"2026-07-18T09:${String(i % 60).padStart(2, '0')}:` +
      `${String((i * 7) % 60).padStart(2, '0')}Z","user":"user_${i % 997}",` +
      `"event":"checkout","amount":${i % 500}.${String(i % 100).padStart(2, '0')},` +
      `"currency":"EUR","ok":true,"tags":["retail","eu","priority"],` +
      `"note":"the quick brown fox jumps over the lazy dog"}\n`,
  )
}
const CORPUS = Buffer.from(records.join(''))
const MIB = CORPUS.length / (1024 * 1024)

function timed(op, iters) {
  op() // warm up
  let best = Infinity
  for (let i = 0; i < iters; i++) {
    const start = process.hrtime.bigint()
    op()
    const ns = Number(process.hrtime.bigint() - start)
    if (ns < best) best = ns
  }
  return MIB / (best / 1e9) // MiB/s
}

function report(label, nativeMibs, yggMibs) {
  const speedup = yggMibs / nativeMibs
  const flag = speedup >= 1.0 ? 'faster' : 'SLOWER'
  console.log(
    `  ${label.padEnd(22)} native ${nativeMibs.toFixed(1).padStart(8)}   ` +
      `yggdryl ${yggMibs.toFixed(1).padStart(8)} MiB/s   ->  ${speedup.toFixed(2)}x  ${flag}`,
  )
  return speedup
}

function benchDeflate(name, nativeC, nativeD, yggCodec, iters) {
  const nativeCompressed = nativeC(CORPUS)
  const yggCompressed = yggCodec.compress(CORPUS)
  // Streams are interchange-compatible: each side decompresses the other's output.
  if (!nativeD(yggCompressed).equals(CORPUS)) throw new Error(`${name}: native cannot read yggdryl`)
  if (!Buffer.from(yggCodec.decompress(nativeCompressed)).equals(CORPUS)) {
    throw new Error(`${name}: yggdryl cannot read native`)
  }
  console.log(
    `${name} (level 6) — ${MIB.toFixed(2)} MiB, ratio ${(yggCompressed.length / CORPUS.length).toFixed(3)}`,
  )
  const up = report(
    'compress',
    timed(() => nativeC(CORPUS), iters),
    timed(() => yggCodec.compress(CORPUS), iters),
  )
  const down = report(
    'decompress',
    timed(() => nativeD(nativeCompressed), iters),
    timed(() => yggCodec.decompress(nativeCompressed), iters),
  )
  return [up, down]
}

function main() {
  console.log(`compression: yggdryl (flate2/zlib-rs) vs node:zlib — corpus ${MIB.toFixed(2)} MiB\n`)

  const opts = { level: 6 }
  const comp = []
  const decomp = []
  for (const [name, nc, nd, codec] of [
    ['gzip', (b) => zlibNative.gzipSync(b, opts), (b) => zlibNative.gunzipSync(b), new Gzip(6)],
    ['zlib', (b) => zlibNative.deflateSync(b, opts), (b) => zlibNative.inflateSync(b), new Zlib(6)],
  ]) {
    const [up, down] = benchDeflate(name, nc, nd, codec, 40)
    comp.push(up)
    decomp.push(down)
    console.log()
  }

  // zstd only when this Node exposes it (>= 22.15 / 23.8); ours is the same C libzstd.
  if (typeof zlibNative.zstdCompressSync === 'function') {
    const yz = new Zstd(3)
    const zc = zlibNative.zstdCompressSync(CORPUS)
    console.log(`zstd (level 3) — ratio ${(zc.length / CORPUS.length).toFixed(3)}`)
    report(
      'compress',
      timed(() => zlibNative.zstdCompressSync(CORPUS), 40),
      timed(() => yz.compress(CORPUS), 40),
    )
    report(
      'decompress',
      timed(() => zlibNative.zstdDecompressSync(zc), 60),
      timed(() => yz.decompress(zc), 60),
    )
    console.log()
  } else {
    console.log('zstd: this Node has no zlib.zstdCompressSync — skipped\n')
  }

  console.log('deflate (gzip+zlib) vs node:zlib:')
  console.log(`  compress   mean ${(comp.reduce((a, b) => a + b) / comp.length).toFixed(2)}x   (>1 = yggdryl faster)`)
  console.log(`  decompress mean ${(decomp.reduce((a, b) => a + b) / decomp.length).toFixed(2)}x`)
}

main()
