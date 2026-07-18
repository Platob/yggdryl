'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { buffer, array, deviceBuffer } = yggdryl
const { Heap } = yggdryl.memory
const { AmdBuffer } = yggdryl.gpu
const { Headers } = yggdryl.headers
const io = yggdryl.io

// -------------------------------------------------------------------------------------
// The top-level builders are registered on the package root (like `open`)
// -------------------------------------------------------------------------------------

test('the package exposes the buffer / array / deviceBuffer builders', () => {
  assert.equal(typeof buffer, 'function')
  assert.equal(typeof array, 'function')
  assert.equal(typeof deviceBuffer, 'function')
})

// -------------------------------------------------------------------------------------
// buffer(data?, options?)
// -------------------------------------------------------------------------------------

test('buffer() with no arguments is an empty Heap', () => {
  const h = buffer()
  assert.ok(h instanceof Heap)
  assert.equal(h.byteSize(), 0)
  assert.ok(h.isEmpty())
})

test('buffer(data) owns a copy of the bytes', () => {
  const src = Buffer.from('hi')
  const h = buffer(src)
  assert.ok(h instanceof Heap)
  assert.equal(h.byteSize(), 2)
  assert.deepEqual(h.toBytes(), Buffer.from('hi'))
  src[0] = 0x5a // mutate the source — the heap holds its own copy
  assert.deepEqual(h.toBytes(), Buffer.from('hi'))
})

test('buffer(undefined, { capacity }) pre-allocates an empty heap', () => {
  const h = buffer(undefined, { capacity: 128 })
  assert.ok(h.isEmpty())
  assert.ok(h.capacity() >= 128)
})

test('buffer(data, { capacity }) copies the bytes and reserves headroom to capacity', () => {
  const h = buffer(Buffer.from('hi'), { capacity: 256 })
  assert.deepEqual(h.toBytes(), Buffer.from('hi')) // the bytes are copied
  assert.ok(h.capacity() >= 256) // capacity is honored even with data
})

test('buffer(data, { headers }) with a plain object sets the header and the bytes', () => {
  const h = buffer(Buffer.from('hi'), { headers: { 'Content-Type': 'text/plain' } })
  assert.deepEqual(h.toBytes(), Buffer.from('hi'))
  assert.equal(h.headers.get('Content-Type'), 'text/plain') // `headers` is a getter
})

test('buffer(data, { headers }) accepts a headers.Headers instance', () => {
  const hdrs = new Headers()
  hdrs.append('X-Test', 'yes')
  const h = buffer(Buffer.from('ab'), { headers: hdrs })
  assert.equal(h.headers.get('X-Test'), 'yes')
  assert.deepEqual(h.toBytes(), Buffer.from('ab'))
})

test('buffer(undefined, { mode }) sets the access mode', () => {
  const h = buffer(undefined, { mode: io.IOMode.Read })
  assert.equal(h.mode, io.IOMode.Read) // `mode` is a getter
})

// -------------------------------------------------------------------------------------
// array(values, dtype?)
// -------------------------------------------------------------------------------------

test('array([1,2,3]) infers i64 and round-trips via preadI64Array', () => {
  const h = array([1, 2, 3])
  assert.ok(h instanceof Heap)
  assert.equal(h.byteSize(), 24) // three i64s
  assert.deepEqual(h.preadI64Array(0, 3), [1, 2, 3])
})

test('array([1.5, 2.5]) infers f64', () => {
  const h = array([1.5, 2.5])
  assert.equal(h.byteSize(), 16) // two f64s
  assert.deepEqual(h.preadF64Array(0, 2), [1.5, 2.5])
})

test('array([1.5, 2.5], "f32") writes f32s round-tripping via preadF32Array', () => {
  const h = array([1.5, 2.5], 'f32')
  assert.equal(h.byteSize(), 8) // two f32s
  assert.deepEqual(h.preadF32Array(0, 2), [1.5, 2.5])
})

test('array([1,2,3], "i32") writes i32s', () => {
  const h = array([1, 2, 3], 'i32')
  assert.equal(h.byteSize(), 12)
  assert.deepEqual(h.preadI32Array(0, 3), [1, 2, 3])
})

test('array([...], "u8") writes bytes', () => {
  const h = array([104, 105], 'u8')
  assert.deepEqual(h.toBytes(), Buffer.from('hi'))
})

test('array(bigints, "i64") accepts a bigint[]', () => {
  const h = array([10n, 20n, 30n], 'i64')
  assert.deepEqual(h.preadI64Array(0, 3), [10, 20, 30])
})

test('array([1], "bogus") throws a guided Error naming the valid dtypes', () => {
  assert.throws(() => array([1], 'bogus'), /unknown dtype 'bogus'.*i8.*f64/s)
})

// -------------------------------------------------------------------------------------
// deviceBuffer(data?, device?)
// -------------------------------------------------------------------------------------

test('deviceBuffer(data) seeds the best available device buffer (byteSize === data length)', () => {
  const dev = deviceBuffer(Buffer.from('x'))
  // The concrete class depends on the hardware present (Heap on a CPU-only host,
  // AmdBuffer when a GPU is detected); either way it carries the uploaded byte.
  assert.ok(dev instanceof Heap || dev instanceof AmdBuffer)
  assert.equal(dev.byteSize(), 1)
})

test('deviceBuffer(undefined, "cpu") returns an empty Heap', () => {
  const dev = deviceBuffer(undefined, 'cpu')
  assert.ok(dev instanceof Heap)
  assert.equal(dev.byteSize(), 0)
})

test('deviceBuffer(data, "amd") returns an AmdBuffer carrying the payload', () => {
  const dev = deviceBuffer(Buffer.from('radeon'), 'amd')
  assert.ok(dev instanceof AmdBuffer)
  assert.equal(dev.byteSize(), 6)
  assert.deepEqual(dev.toBytes(), Buffer.from('radeon'))
})

test('deviceBuffer accepts "gpu" / "cuda" as GPU tokens (mirroring Python)', () => {
  // The AmdBuffer device-memory type falls back to the CPU device when no GPU is present,
  // so these resolve to an AmdBuffer regardless of the hardware.
  assert.ok(deviceBuffer(undefined, 'gpu') instanceof AmdBuffer)
  assert.ok(deviceBuffer(undefined, 'cuda') instanceof AmdBuffer)
})

test('deviceBuffer device tokens are case-insensitive', () => {
  assert.ok(deviceBuffer(undefined, 'AMD') instanceof AmdBuffer)
  assert.ok(deviceBuffer(undefined, 'CPU') instanceof Heap)
})

test('deviceBuffer with an unknown device token throws a guided Error', () => {
  assert.throws(
    () => deviceBuffer(undefined, 'quantum'),
    /unknown device 'quantum'.*cpu.*amd.*gpu.*cuda/s,
  )
})
