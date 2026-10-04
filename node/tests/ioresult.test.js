'use strict'

// `node/src/ioresult.rs`: the rows one record write read, wrote and skipped.

const assert = require('node:assert/strict')
const test = require('node:test')

const { IOResult } = require('yggdryl')
const { IOResult: NativeIOResult } = require('../index.js')

function counts(result) {
  assert.ok(result instanceof IOResult)
  return [result.readRows, result.writtenRows, result.skippedRows]
}

test('IOResult is the native class, exported at the root', () => {
  assert.equal(IOResult, NativeIOResult)
  assert.equal(typeof IOResult, 'function')
})

test('what was read and not written is skipped', () => {
  const result = new IOResult(10, 8)
  assert.deepEqual(counts(result), [10, 8, 2])
  for (const count of counts(result)) assert.equal(typeof count, 'number')
  assert.equal(result.isEmpty(), false)
})

test('a write of more rows than it read skips none', () => {
  // A `select` that unnests writes more rows than its source held, and the
  // skipped count never goes below zero.
  assert.deepEqual(counts(new IOResult(2, 6)), [2, 6, 0])
})

test('an absent count is zero, and the default is the write of an empty source', () => {
  const empty = new IOResult()
  assert.deepEqual(counts(empty), [0, 0, 0])
  assert.equal(empty.isEmpty(), true)
  assert.ok(new IOResult(undefined, undefined).equals(empty))
  assert.ok(new IOResult(null, null).equals(empty))
  assert.deepEqual(counts(new IOResult(5)), [5, 0, 5])
  assert.deepEqual(counts(new IOResult(undefined, 3)), [0, 3, 0])

  // The three counts stated are taken as they are: what a sum holds, which
  // the read rows less the written need not be.
  const summed = new IOResult(2, 6).add(new IOResult(4, 0))
  assert.deepEqual(counts(summed), [6, 6, 4])
  assert.ok(new IOResult(6, 6, 4).equals(summed))
  assert.equal(new IOResult(6, 6, 4).equals(new IOResult(6, 6)), false)
  assert.ok(new IOResult(10, 8, undefined).equals(new IOResult(10, 8)))
  assert.ok(new IOResult(10, 8, null).equals(new IOResult(10, 8)))
  assert.throws(() => new IOResult(1, 1, -1), /skippedRows/)
  // A source whose every row was kept out was read: it is not empty, and
  // neither is a write that produced rows from none.
  assert.equal(new IOResult(3, 0).isEmpty(), false)
  assert.equal(new IOResult(0, 2).isEmpty(), false)
})

test('a count is a non-negative whole number, refused by name', () => {
  for (const [args, name] of [
    [[-1], 'readRows'],
    [[1.5], 'readRows'],
    [[Number.NaN], 'readRows'],
    [[Number.POSITIVE_INFINITY], 'readRows'],
    [[2 ** 53 + 2], 'readRows'],
    [[1, -1], 'writtenRows'],
    [[1, 0.5], 'writtenRows'],
  ]) {
    assert.throws(
      () => new IOResult(...args),
      new RegExp(`${name} must be a non-negative whole number of at most 2\\^53`),
      String(args),
    )
  }
  // A count is a number: text and a bigint are refused, never read.
  assert.throws(() => new IOResult('10'), /into rust type `f64`/)
  assert.throws(() => new IOResult(1n), /into rust type `f64`/)
})

test('results add count by count, each stating its own skipped rows', () => {
  const total = new IOResult(10, 8).add(new IOResult(5, 5))
  assert.ok(total.equals(new IOResult(15, 13)))
  assert.deepEqual(counts(total.add(new IOResult(1, 0))), [16, 13, 3])
  // A sum keeps the skipped rows each part stated, which the difference of
  // the summed counts need not be: 3 read, 6 written, 1 skipped.
  assert.deepEqual(counts(new IOResult(2, 6).add(new IOResult(1, 0))), [3, 6, 1])
  // Neither operand moves.
  const left = new IOResult(1, 1)
  left.add(new IOResult(4, 4))
  assert.deepEqual(counts(left), [1, 1, 0])
  assert.throws(() => left.add({ readRows: 1, writtenRows: 1, skippedRows: 0 }))
})

test('a result compares, orders and hashes as its three counts', () => {
  const result = new IOResult(10, 8)
  assert.equal(result.equals(new IOResult(10, 8)), true)
  assert.equal(result.equals(new IOResult(10, 9)), false)
  assert.equal(result.compare(new IOResult(10, 8)), 0)
  // The counts order in turn: what was read, then what was written.
  assert.equal(new IOResult(1, 1).compare(new IOResult(2, 0)), -1)
  assert.equal(new IOResult(2, 0).compare(new IOResult(1, 1)), 1)
  assert.equal(new IOResult(3, 1).compare(new IOResult(3, 2)), -1)

  const hash = result.stableHash()
  assert.equal(typeof hash, 'bigint')
  assert.equal(hash, new IOResult(10, 8).stableHash())
  assert.notEqual(hash, new IOResult(10, 9).stableHash())
  assert.notEqual(hash, new IOResult(8, 10).stableHash())
})

test('a result clones apart and renders the core text', () => {
  const result = new IOResult(10, 8)
  const copy = result.clone()
  assert.notEqual(copy, result)
  assert.ok(copy.equals(result))
  assert.equal(copy.stableHash(), result.stableHash())

  assert.equal(result.toString(), 'read 10 rows, wrote 8, skipped 2')
  assert.equal(`${new IOResult()}`, 'read 0 rows, wrote 0, skipped 0')
  assert.equal(String(new IOResult(2, 6)), 'read 2 rows, wrote 6, skipped 0')
})

test('a result is immutable', () => {
  const result = new IOResult(10, 8)
  assert.throws(() => {
    'use strict'
    result.readRows = 1
  }, TypeError)
  assert.deepEqual(counts(result), [10, 8, 2])
})
