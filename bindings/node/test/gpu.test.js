'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { availableDevices, defaultDevice, GpuDevice, AmdBuffer } = yggdryl.gpu
const { MemoryInfo } = yggdryl.io

// -------------------------------------------------------------------------------------
// Namespace
// -------------------------------------------------------------------------------------

test('the gpu namespace exposes availableDevices, defaultDevice, GpuDevice, and AmdBuffer', () => {
  assert.equal(typeof availableDevices, 'function')
  assert.equal(typeof defaultDevice, 'function')
  assert.equal(typeof GpuDevice, 'function')
  assert.equal(typeof AmdBuffer, 'function')
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
// GpuDevice + the device probe
// -------------------------------------------------------------------------------------

test('availableDevices() is non-empty and always includes a cpu device', () => {
  const devices = availableDevices()
  assert.ok(Array.isArray(devices))
  assert.ok(devices.length >= 1)
  const cpu = devices.find((d) => d.isCpu())
  assert.ok(cpu, 'the CPU device is always present')
  assert.equal(cpu.backend(), 'cpu')
  assert.equal(typeof cpu.name(), 'string')
  assert.ok(cpu.totalMemory() >= 0)
})

test('defaultDevice() is a GpuDevice whose backend token is amd or cpu', () => {
  const device = defaultDevice()
  assert.ok(device instanceof GpuDevice)
  assert.ok(['amd', 'cpu'].includes(device.backend()))
  const info = device.memoryInfo()
  assert.ok(info.total() >= info.available()) // total >= available within a device
  assert.equal(device.toString(), `GpuDevice(${device.backend()}, ${device.name()})`)
})

test('GpuDevice.equals + hashCode compare by value', () => {
  const devices = availableDevices()
  const cpuA = devices.find((d) => d.isCpu())
  const cpuB = availableDevices().find((d) => d.isCpu())
  assert.equal(cpuA.equals(cpuB), true)
  // Equal devices hash equal.
  assert.equal(typeof cpuA.hashCode(), 'number')
  assert.equal(cpuA.hashCode(), cpuB.hashCode())
})

// -------------------------------------------------------------------------------------
// AmdBuffer — device memory over the IOBase byte + bulk surface
// -------------------------------------------------------------------------------------

test('AmdBuffer upload/download round-trips the payload', () => {
  const buf = new AmdBuffer()
  const payload = Buffer.from('radeon payload')
  buf.upload(payload)

  assert.equal(buf.byteSize(), payload.length)
  assert.deepEqual(buf.downloadVec(), payload)
  assert.deepEqual(buf.toBytes(), payload)
  // A short download clamps to what is available.
  assert.deepEqual(buf.download(6), Buffer.from('radeon'))
  // An over-long download is short (never over-reads).
  assert.deepEqual(buf.download(1000), payload)
})

test('AmdBuffer runs a vectorized bulk op on device memory', () => {
  const buf = new AmdBuffer()
  buf.upload(Buffer.from('radeon payload'))
  buf.pwriteI32Array(16, [1, -2, 3])
  assert.deepEqual(buf.preadI32Array(16, 3), [1, -2, 3])

  // The uploaded head is untouched by the positioned bulk write.
  assert.deepEqual(buf.download(14), Buffer.from('radeon payload'))

  // i64 bulk op too.
  buf.pwriteI64Array(64, [10, 20])
  assert.deepEqual(buf.preadI64Array(64, 2), [10, 20])
})

test('AmdBuffer positioned byte reads/writes', () => {
  const buf = AmdBuffer.withCapacity(32)
  assert.equal(buf.byteSize(), 0)
  buf.pwriteByteArray(0, Buffer.from('abcdef'))
  assert.deepEqual(buf.preadByteArray(2, 3), Buffer.from('cde'))
})

test('AmdBuffer.fromHost seeds the buffer, and its device backend is amd or cpu', () => {
  const buf = AmdBuffer.fromHost(Buffer.from('seed'))
  assert.deepEqual(buf.downloadVec(), Buffer.from('seed'))

  const device = buf.device()
  assert.ok(device instanceof GpuDevice)
  assert.ok(['amd', 'cpu'].includes(device.backend()))

  const info = buf.memoryInfo()
  assert.ok(info.total() >= info.available())
})

test('AmdBuffer.dispose resets the buffer to empty', () => {
  const buf = AmdBuffer.fromHost(Buffer.from('payload'))
  assert.equal(buf.byteSize(), 7)
  buf.dispose()
  assert.equal(buf.byteSize(), 0)
  assert.deepEqual(buf.downloadVec(), Buffer.alloc(0))
})

test('AmdBuffer bulk read past the end throws a guided Error', () => {
  const buf = AmdBuffer.fromHost(Buffer.from('short'))
  assert.throws(() => buf.preadI32Array(0, 100), /.+/)
})

// -------------------------------------------------------------------------------------
// AmdBuffer — Compute: auto-dispatched aggregations, filter, and device-aware copy
// -------------------------------------------------------------------------------------

test('AmdBuffer i32 aggregations: sum/min/max/mean/countGe', () => {
  const buf = new AmdBuffer()
  const values = [4, 8, 15, 16, 23, 42]
  buf.pwriteI32Array(0, values)

  assert.equal(buf.sumI32(0, 6), 108)
  assert.equal(buf.minI32(0, 6), 4)
  assert.equal(buf.maxI32(0, 6), 42)
  assert.equal(buf.meanI32(0, 6), 18)
  // Filter: how many are >= 16.
  assert.equal(buf.countGeI32(0, 6, 16), 3)

  // An empty window yields the null/None aggregates but a zero filter count.
  assert.equal(buf.minI32(0, 0), null)
  assert.equal(buf.maxI32(0, 0), null)
  assert.equal(buf.meanI32(0, 0), null)
  assert.equal(buf.countGeI32(0, 0, 16), 0)
})

test('AmdBuffer std / first / last aggregations (i32 / i64 / f32 / f64)', () => {
  const i32 = new AmdBuffer()
  i32.pwriteI32Array(0, [4, 8, 15, 16, 23, 42])
  assert.equal(i32.firstI32(0, 6), 4)
  assert.equal(i32.lastI32(0, 6), 42)
  assert.ok(Math.abs(i32.stdI32(0, 6) - 12.315) < 0.01) // sqrt(910/6)
  assert.equal(i32.firstI32(0, 0), null)
  assert.equal(i32.lastI32(0, 0), null)
  assert.equal(i32.stdI32(0, 0), null)

  const i64 = new AmdBuffer()
  i64.pwriteI64Array(0, [10, 20, 30, 40])
  assert.equal(i64.firstI64(0, 4), 10) // a JS number
  assert.equal(i64.lastI64(0, 4), 40)
  assert.ok(i64.stdI64(0, 4) > 0)

  // f64 / f32 first/last widen to JS numbers (seed little-endian bytes).
  const f64 = new AmdBuffer()
  const b64 = Buffer.alloc(3 * 8)
  ;[10.0, 20.0, 30.0].forEach((v, i) => b64.writeDoubleLE(v, i * 8))
  f64.pwriteByteArray(0, b64)
  assert.equal(f64.firstF64(0, 3), 10.0)
  assert.equal(f64.lastF64(0, 3), 30.0)
  assert.ok(Math.abs(f64.stdF64(0, 3) - 8.165) < 0.01) // sqrt(200/3)

  const f32 = new AmdBuffer()
  const b32 = Buffer.alloc(3 * 4)
  ;[1.5, 2.5, 3.5].forEach((v, i) => b32.writeFloatLE(v, i * 4))
  f32.pwriteByteArray(0, b32)
  assert.equal(f32.firstF32(0, 3), 1.5)
  assert.equal(f32.lastF32(0, 3), 3.5)
  assert.ok(f32.stdF32(0, 3) > 0)
})

test('AmdBuffer i64 aggregations cross 64-bit values (BigInt sum, BigInt threshold)', () => {
  const buf = new AmdBuffer()
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

test('AmdBuffer f64 aggregations stream a large (>1024) array through the stack chunk', () => {
  const buf = new AmdBuffer()
  const n = 5000 // exceeds the 1024-element compute chunk, exercising the streaming loop
  // No f64 bulk-array method on AmdBuffer, so seed the device bytes as little-endian f64s.
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

test('AmdBuffer f32 aggregations widen to JS numbers', () => {
  const buf = new AmdBuffer()
  // Seed little-endian f32s (no f32 bulk-array method on AmdBuffer).
  const bytes = Buffer.alloc(3 * 4)
  ;[1.5, 2.5, 3.5].forEach((v, i) => bytes.writeFloatLE(v, i * 4))
  buf.pwriteByteArray(0, bytes)

  assert.equal(buf.sumF32(0, 3), 7.5)
  assert.equal(buf.minF32(0, 3), 1.5)
  assert.equal(buf.maxF32(0, 3), 3.5)
  assert.equal(buf.meanF32(0, 3), 2.5)
  assert.equal(buf.countGeF32(0, 3, 2.5), 2)
})

test('AmdBuffer computeCopyInto round-trips the whole buffer into another buffer', () => {
  const src = new AmdBuffer()
  src.pwriteI32Array(0, [7, 11, 13, 17])
  const dst = new AmdBuffer()

  const copied = src.computeCopyInto(dst)
  assert.equal(copied, src.byteSize())
  assert.equal(copied, 16) // four i32s
  assert.deepEqual(dst.downloadVec(), src.downloadVec())
  assert.deepEqual(dst.preadI32Array(0, 4), [7, 11, 13, 17])
})

test('AmdBuffer computeBackend reports the cpu token for small workloads', () => {
  const buf = new AmdBuffer()
  assert.equal(buf.computeBackend(8), 'cpu')
})
