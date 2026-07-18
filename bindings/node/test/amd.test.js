'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { detect, AmdDevice, AmdHeap } = yggdryl.amd
const { MemoryInfo } = yggdryl.io

// -------------------------------------------------------------------------------------
// Namespace
// -------------------------------------------------------------------------------------

test('the amd namespace exposes detect, AmdDevice, and AmdHeap', () => {
  assert.equal(typeof detect, 'function')
  assert.equal(typeof AmdDevice, 'function')
  assert.equal(typeof AmdHeap, 'function')
})

// -------------------------------------------------------------------------------------
// MemoryInfo — the capacity snapshot value (io namespace)
// -------------------------------------------------------------------------------------

test('MemoryInfo.system() reports a sane snapshot (total >= available)', () => {
  const sys = MemoryInfo.system()
  assert.ok(sys.total() >= sys.available())
  assert.ok(sys.available() >= 0)
  assert.equal(sys.used(), sys.total() - sys.available())
  assert.ok(sys.usageRatio() >= 0 && sys.usageRatio() <= 1)
})

test('MemoryInfo constructor clamps available to total and derives used/ratio', () => {
  const info = new MemoryInfo(1000, 250)
  assert.equal(info.total(), 1000)
  assert.equal(info.available(), 250)
  assert.equal(info.used(), 750)
  assert.equal(info.usageRatio(), 0.75)
  assert.equal(info.isUnknown(), false)

  const clamped = new MemoryInfo(100, 500) // available is clamped to total
  assert.equal(clamped.available(), 100)
  assert.equal(clamped.used(), 0)
})

test('MemoryInfo.unknown() is the portable sentinel (0/0)', () => {
  const unknown = MemoryInfo.unknown()
  assert.equal(unknown.total(), 0)
  assert.equal(unknown.available(), 0)
  assert.equal(unknown.isUnknown(), true)
  assert.equal(unknown.usageRatio(), 0) // 0 when total is unknown
})

test('MemoryInfo equals + hashCode + toString', () => {
  const a = new MemoryInfo(2048, 1024)
  const b = new MemoryInfo(2048, 1024)
  const c = new MemoryInfo(2048, 512)
  assert.equal(a.equals(b), true)
  assert.equal(a.equals(c), false)
  // Equal values hash equal.
  assert.equal(typeof a.hashCode(), 'number')
  assert.equal(a.hashCode(), b.hashCode())
  assert.equal(a.toString(), 'MemoryInfo(total=2048, available=1024)')
})

// -------------------------------------------------------------------------------------
// detect() + AmdDevice — the hardware probe
// -------------------------------------------------------------------------------------

test('detect() is null or an AmdDevice with isPresent() true', () => {
  const adapter = detect()
  // Adapts to the hardware present: null on a machine with no AMD Radeon adapter.
  assert.ok(adapter === null || adapter instanceof AmdDevice)
  if (adapter !== null) {
    assert.equal(adapter.isPresent(), true)
    assert.equal(typeof adapter.name(), 'string')
    assert.ok(adapter.totalMemory() >= 0)
  }
})

test('AmdHeap().device() is the detected adapter, else the host fallback', () => {
  const adapter = detect()
  const dev = new AmdHeap().device()
  assert.ok(dev instanceof AmdDevice)
  // isPresent() tracks whether a real adapter was detected.
  assert.equal(dev.isPresent(), adapter !== null)

  const info = dev.memoryInfo() // a live capacity snapshot for the device
  assert.ok(info.total() >= info.available())
})

test('AmdDevice equals + hashCode + toString compare by value', () => {
  const a = new AmdHeap().device()
  const b = new AmdHeap().device()
  assert.equal(a.equals(b), true)
  // Equal devices hash equal.
  assert.equal(typeof a.hashCode(), 'number')
  assert.equal(a.hashCode(), b.hashCode())
  assert.equal(a.toString(), `AmdDevice(${a.name()}, present=${a.isPresent()})`)
})

// -------------------------------------------------------------------------------------
// AmdHeap — device memory over the IOBase byte + bulk surface
// -------------------------------------------------------------------------------------

test('AmdHeap upload/download round-trips the payload', () => {
  const buf = new AmdHeap()
  const payload = Buffer.from('radeon payload')
  buf.upload(payload)

  assert.equal(buf.byteSize(), payload.length)
  assert.equal(buf.isEmpty(), false)
  assert.deepEqual(buf.downloadVec(), payload)
  assert.deepEqual(buf.toBytes(), payload)
  // A short download clamps to what is available.
  assert.deepEqual(buf.download(6), Buffer.from('radeon'))
  // An over-long download is short (never over-reads).
  assert.deepEqual(buf.download(1000), payload)
})

test('AmdHeap runs a vectorized bulk op on device memory', () => {
  const buf = new AmdHeap()
  buf.upload(Buffer.from('radeon payload'))
  buf.pwriteI32Array(16, [1, -2, 3])
  assert.deepEqual(buf.preadI32Array(16, 3), [1, -2, 3])

  // The uploaded head is untouched by the positioned bulk write.
  assert.deepEqual(buf.download(14), Buffer.from('radeon payload'))

  // i64 bulk op too.
  buf.pwriteI64Array(64, [10, 20])
  assert.deepEqual(buf.preadI64Array(64, 2), [10, 20])
})

test('AmdHeap positioned byte reads/writes', () => {
  const buf = AmdHeap.withCapacity(32)
  assert.equal(buf.byteSize(), 0)
  assert.equal(buf.isEmpty(), true)
  buf.pwriteByteArray(0, Buffer.from('abcdef'))
  assert.deepEqual(buf.preadByteArray(2, 3), Buffer.from('cde'))
})

test('AmdHeap.fromHost seeds the heap and reports its device', () => {
  const buf = AmdHeap.fromHost(Buffer.from('seed'))
  assert.deepEqual(buf.downloadVec(), Buffer.from('seed'))

  const device = buf.device()
  assert.ok(device instanceof AmdDevice)
  assert.equal(device.isPresent(), detect() !== null)

  const info = buf.memoryInfo()
  assert.ok(info.total() >= info.available())
})

test('AmdHeap.dispose resets the heap to empty', () => {
  const buf = AmdHeap.fromHost(Buffer.from('payload'))
  assert.equal(buf.byteSize(), 7)
  buf.dispose()
  assert.equal(buf.byteSize(), 0)
  assert.deepEqual(buf.downloadVec(), Buffer.alloc(0))
})

test('AmdHeap bulk read past the end throws a guided Error', () => {
  const buf = AmdHeap.fromHost(Buffer.from('short'))
  assert.throws(() => buf.preadI32Array(0, 100), /.+/)
})

// -------------------------------------------------------------------------------------
// AmdHeap — auto-dispatched aggregations, filter, and device-aware copy
// -------------------------------------------------------------------------------------

test('AmdHeap i32 aggregations: sum/min/max/mean/countGe', () => {
  const buf = new AmdHeap()
  const values = [4, 8, 15, 16, 23, 42]
  buf.pwriteI32Array(0, values)

  assert.equal(buf.sumI32(0, 6), 108)
  assert.equal(buf.minI32(0, 6), 4)
  assert.equal(buf.maxI32(0, 6), 42)
  assert.equal(buf.meanI32(0, 6), 18)
  // Filter: how many are >= 16.
  assert.equal(buf.countGeI32(0, 6, 16), 3)

  // An empty window yields the null aggregates but a zero filter count.
  assert.equal(buf.minI32(0, 0), null)
  assert.equal(buf.maxI32(0, 0), null)
  assert.equal(buf.meanI32(0, 0), null)
  assert.equal(buf.countGeI32(0, 0, 16), 0)
})

test('AmdHeap std / first / last aggregations (i32 / i64 / f32 / f64)', () => {
  const i32 = new AmdHeap()
  i32.pwriteI32Array(0, [4, 8, 15, 16, 23, 42])
  assert.equal(i32.firstI32(0, 6), 4)
  assert.equal(i32.lastI32(0, 6), 42)
  assert.ok(Math.abs(i32.stdI32(0, 6) - 12.315) < 0.01) // sqrt(910/6)
  assert.equal(i32.firstI32(0, 0), null)
  assert.equal(i32.lastI32(0, 0), null)
  assert.equal(i32.stdI32(0, 0), null)

  const i64 = new AmdHeap()
  i64.pwriteI64Array(0, [10, 20, 30, 40])
  assert.equal(i64.firstI64(0, 4), 10) // a JS number
  assert.equal(i64.lastI64(0, 4), 40)
  assert.ok(i64.stdI64(0, 4) > 0)

  // f64 / f32 first/last widen to JS numbers (seed little-endian bytes).
  const f64 = new AmdHeap()
  const b64 = Buffer.alloc(3 * 8)
  ;[10.0, 20.0, 30.0].forEach((v, i) => b64.writeDoubleLE(v, i * 8))
  f64.pwriteByteArray(0, b64)
  assert.equal(f64.firstF64(0, 3), 10.0)
  assert.equal(f64.lastF64(0, 3), 30.0)
  assert.ok(Math.abs(f64.stdF64(0, 3) - 8.165) < 0.01) // sqrt(200/3)

  const f32 = new AmdHeap()
  const b32 = Buffer.alloc(3 * 4)
  ;[1.5, 2.5, 3.5].forEach((v, i) => b32.writeFloatLE(v, i * 4))
  f32.pwriteByteArray(0, b32)
  assert.equal(f32.firstF32(0, 3), 1.5)
  assert.equal(f32.lastF32(0, 3), 3.5)
  assert.ok(f32.stdF32(0, 3) > 0)
})

test('AmdHeap i64 aggregations cross 64-bit values (BigInt sum, BigInt threshold)', () => {
  const buf = new AmdHeap()
  buf.pwriteI64Array(0, [10, 20, 30, 40])

  // The i64 sum accumulates to i128 and crosses as a BigInt.
  assert.equal(buf.sumI64(0, 4), 100n)
  // min/max cross as JS numbers (i64, exact to 2^53).
  assert.equal(buf.minI64(0, 4), 10)
  assert.equal(buf.maxI64(0, 4), 40)
  assert.equal(buf.meanI64(0, 4), 25)
  // The i64 filter threshold is a BigInt.
  assert.equal(buf.countGeI64(0, 4, 25n), 2)
})

test('AmdHeap f64 aggregations stream a large (>1024) array through the stack chunk', () => {
  const buf = new AmdHeap()
  const n = 5000 // exceeds the compute chunk, exercising the streaming loop
  // Seed the device bytes as little-endian f64s.
  const bytes = Buffer.alloc(n * 8)
  let expectedSum = 0
  for (let i = 0; i < n; i++) {
    const v = i + 0.5
    bytes.writeDoubleLE(v, i * 8)
    expectedSum += v
  }
  buf.pwriteByteArray(0, bytes)

  assert.ok(Math.abs(buf.sumF64(0, n) - expectedSum) < 1e-6)
  assert.equal(buf.minF64(0, n), 0.5)
  assert.equal(buf.maxF64(0, n), n - 1 + 0.5)
  assert.ok(Math.abs(buf.meanF64(0, n) - expectedSum / n) < 1e-9)
  // How many of the n values are >= (n / 2): the upper half.
  assert.equal(buf.countGeF64(0, n, n / 2), n - n / 2)
})

test('AmdHeap f32 aggregations widen to JS numbers', () => {
  const buf = new AmdHeap()
  // Seed little-endian f32s.
  const bytes = Buffer.alloc(3 * 4)
  ;[1.5, 2.5, 3.5].forEach((v, i) => bytes.writeFloatLE(v, i * 4))
  buf.pwriteByteArray(0, bytes)

  assert.equal(buf.sumF32(0, 3), 7.5)
  assert.equal(buf.minF32(0, 3), 1.5)
  assert.equal(buf.maxF32(0, 3), 3.5)
  assert.equal(buf.meanF32(0, 3), 2.5)
  assert.equal(buf.countGeF32(0, 3, 2.5), 2)
})

test('AmdHeap min/max ignore NaN independent of its position', () => {
  // Seed the finite values with a NaN at the start, middle, and end — min/max skip NaN,
  // so the finite min/max is the same wherever NaN sits (order-independent).
  const layouts = [
    [NaN, 1.5, 4.5, 2.5],
    [1.5, NaN, 4.5, 2.5],
    [1.5, 4.5, 2.5, NaN],
  ]
  for (const values of layouts) {
    const buf = new AmdHeap()
    const bytes = Buffer.alloc(values.length * 8)
    values.forEach((v, i) => bytes.writeDoubleLE(v, i * 8))
    buf.pwriteByteArray(0, bytes)

    assert.equal(buf.minF64(0, values.length), 1.5)
    assert.equal(buf.maxF64(0, values.length), 4.5)
  }
})

test('AmdHeap computeCopyInto round-trips the whole heap into another heap', () => {
  const src = new AmdHeap()
  src.pwriteI32Array(0, [7, 11, 13, 17])
  const dst = new AmdHeap()

  const copied = src.computeCopyInto(dst)
  assert.equal(copied, src.byteSize())
  assert.equal(copied, 16) // four i32s
  assert.deepEqual(dst.downloadVec(), src.downloadVec())
  assert.deepEqual(dst.preadI32Array(0, 4), [7, 11, 13, 17])
})

test('AmdHeap computeBackend reports the cpu token for small workloads', () => {
  const buf = new AmdHeap()
  assert.equal(buf.computeBackend(8), 'cpu')
})
