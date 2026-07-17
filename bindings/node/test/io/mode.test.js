'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../..')
const io = yggdryl.io

// -------------------------------------------------------------------------------------
// Enum values (wire-stable)
// -------------------------------------------------------------------------------------

test('IOMode carries the wire-stable numeric values', () => {
  assert.equal(io.IOMode.Read, 1)
  assert.equal(io.IOMode.Write, 2)
  assert.equal(io.IOMode.ReadWrite, 3) // = Read | Write
  assert.equal(io.IOMode.Append, 4)
  assert.equal(io.IOMode.Overwrite, 5)
})

// -------------------------------------------------------------------------------------
// parseIoMode — the generic entry (name or number)
// -------------------------------------------------------------------------------------

test('parseIoMode infers names (any case, aliases) and numeric values', () => {
  assert.equal(io.parseIoMode('rw'), io.IOMode.ReadWrite)
  assert.equal(io.parseIoMode('read_write'), io.IOMode.ReadWrite)
  assert.equal(io.parseIoMode('READ'), io.IOMode.Read)
  assert.equal(io.parseIoMode('r'), io.IOMode.Read)
  assert.equal(io.parseIoMode('a'), io.IOMode.Append)
  assert.equal(io.parseIoMode('truncate'), io.IOMode.Overwrite)
  assert.equal(io.parseIoMode('+'), io.IOMode.ReadWrite)

  assert.equal(io.parseIoMode(1), io.IOMode.Read)
  assert.equal(io.parseIoMode(4), io.IOMode.Append)
  assert.equal(io.parseIoMode(5), io.IOMode.Overwrite)
})

test('parseIoMode throws a guided error naming the offending input', () => {
  assert.throws(() => io.parseIoMode('bogus'), /IOMode/)
  assert.throws(() => io.parseIoMode('bogus'), /bogus/)
  assert.throws(() => io.parseIoMode('bogus'), /read_write/) // lists the accepted tokens

  assert.throws(() => io.parseIoMode(9), /IOMode/)
  assert.throws(() => io.parseIoMode(9), /9/)
  assert.throws(() => io.parseIoMode(0), /expected one of/)
  assert.throws(() => io.parseIoMode(70000), /70000/) // outside u8, still named exactly
})

test('parseIoMode never wraps numbers modulo 2^32 (no ECMAScript ToUint32)', () => {
  // 2^32 + 1 would coerce to 1 (Read) under ToUint32 — it must throw, naming itself.
  assert.throws(() => io.parseIoMode(4294967297), /IOMode/)
  assert.throws(() => io.parseIoMode(4294967297), /4294967297/)
  // -1 would coerce to 4294967295 under ToUint32 — it must throw, naming itself.
  assert.throws(() => io.parseIoMode(-1), /IOMode/)
  assert.throws(() => io.parseIoMode(-1), /-1/)
})

// -------------------------------------------------------------------------------------
// ioModeName / ioModeIsReadable / ioModeIsWritable
// -------------------------------------------------------------------------------------

test('ioModeName is the canonical snake_case name (inverse of parseIoMode)', () => {
  assert.equal(io.ioModeName(io.IOMode.Read), 'read')
  assert.equal(io.ioModeName(io.IOMode.ReadWrite), 'read_write')
  assert.equal(io.ioModeName(io.IOMode.Overwrite), 'overwrite')
  assert.equal(io.parseIoMode(io.ioModeName(io.IOMode.Append)), io.IOMode.Append)
})

test('ioModeIsReadable / ioModeIsWritable classify each mode', () => {
  assert.ok(io.ioModeIsReadable(io.IOMode.Read))
  assert.ok(io.ioModeIsReadable(io.IOMode.ReadWrite))
  assert.ok(!io.ioModeIsReadable(io.IOMode.Write))
  assert.ok(!io.ioModeIsReadable(io.IOMode.Append))

  assert.ok(!io.ioModeIsWritable(io.IOMode.Read))
  assert.ok(io.ioModeIsWritable(io.IOMode.Write))
  assert.ok(io.ioModeIsWritable(io.IOMode.ReadWrite))
  assert.ok(io.ioModeIsWritable(io.IOMode.Append))
  assert.ok(io.ioModeIsWritable(io.IOMode.Overwrite))
})
