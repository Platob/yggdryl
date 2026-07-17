'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../..')
const io = yggdryl.io

// -------------------------------------------------------------------------------------
// Enum values (wire-stable)
// -------------------------------------------------------------------------------------

test('IOKind carries the wire-stable numeric values', () => {
  assert.equal(io.IOKind.Missing, 0)
  assert.equal(io.IOKind.File, 1)
  assert.equal(io.IOKind.Directory, 2)
  assert.equal(io.IOKind.Heap, 3)
})

// -------------------------------------------------------------------------------------
// parseIoKind — the generic entry (name or number)
// -------------------------------------------------------------------------------------

test('parseIoKind infers names (any case, aliases) and numeric values', () => {
  assert.equal(io.parseIoKind('missing'), io.IOKind.Missing)
  assert.equal(io.parseIoKind('file'), io.IOKind.File)
  assert.equal(io.parseIoKind('directory'), io.IOKind.Directory)
  assert.equal(io.parseIoKind('DIR'), io.IOKind.Directory)
  assert.equal(io.parseIoKind('HEAP'), io.IOKind.Heap)

  assert.equal(io.parseIoKind(0), io.IOKind.Missing)
  assert.equal(io.parseIoKind(2), io.IOKind.Directory)
  assert.equal(io.parseIoKind(3), io.IOKind.Heap)
})

test('parseIoKind throws a guided error naming the offending input', () => {
  assert.throws(() => io.parseIoKind('bogus'), /IOKind/)
  assert.throws(() => io.parseIoKind('bogus'), /bogus/)
  assert.throws(() => io.parseIoKind('bogus'), /directory/) // lists the accepted tokens

  assert.throws(() => io.parseIoKind(9), /IOKind/)
  assert.throws(() => io.parseIoKind(9), /9/)
  assert.throws(() => io.parseIoKind(70000), /70000/) // outside u8, still named exactly
})

test('parseIoKind never wraps numbers modulo 2^32 (no ECMAScript ToUint32)', () => {
  // 2^32 + 2 would coerce to 2 (Directory) under ToUint32 — it must throw, naming itself.
  assert.throws(() => io.parseIoKind(4294967298), /IOKind/)
  assert.throws(() => io.parseIoKind(4294967298), /4294967298/)
  // -1 would coerce to 4294967295 under ToUint32 — it must throw, naming itself.
  assert.throws(() => io.parseIoKind(-1), /IOKind/)
  assert.throws(() => io.parseIoKind(-1), /-1/)
})

// -------------------------------------------------------------------------------------
// ioKindName / ioKindExists
// -------------------------------------------------------------------------------------

test('ioKindName is the canonical lowercase name (inverse of parseIoKind)', () => {
  assert.equal(io.ioKindName(io.IOKind.Missing), 'missing')
  assert.equal(io.ioKindName(io.IOKind.Directory), 'directory')
  assert.equal(io.ioKindName(io.IOKind.Heap), 'heap')
  assert.equal(io.parseIoKind(io.ioKindName(io.IOKind.File)), io.IOKind.File)
})

test('ioKindExists is false only for Missing', () => {
  assert.ok(!io.ioKindExists(io.IOKind.Missing))
  assert.ok(io.ioKindExists(io.IOKind.File))
  assert.ok(io.ioKindExists(io.IOKind.Directory))
  assert.ok(io.ioKindExists(io.IOKind.Heap))
})
