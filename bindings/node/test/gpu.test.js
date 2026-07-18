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

test('MemoryInfo equals + toString', () => {
  const a = new MemoryInfo(2048, 1024)
  const b = new MemoryInfo(2048, 1024)
  const c = new MemoryInfo(2048, 512)
  assert.equal(a.equals(b), true)
  assert.equal(a.equals(c), false)
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

test('GpuDevice.equals compares by value', () => {
  const devices = availableDevices()
  const cpuA = devices.find((d) => d.isCpu())
  const cpuB = availableDevices().find((d) => d.isCpu())
  assert.equal(cpuA.equals(cpuB), true)
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
