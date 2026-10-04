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
})

test('an absent count is zero, and what was read and not written is skipped unless stated', () => {
  for (const [args, expected] of [
    [[], [0, 0, 0]],
    [[undefined, undefined], [0, 0, 0]],
    [[null, null], [0, 0, 0]],
    [[5], [5, 0, 5]],
    [[undefined, 3], [0, 3, 0]],
    [[10, 8], [10, 8, 2]],
    [[10, 8, undefined], [10, 8, 2]],
    [[10, 8, null], [10, 8, 2]],
    // A `select` that unnests writes more rows than it read: none skipped.
    [[2, 6], [2, 6, 0]],
    // Stated, the three counts are taken as they are: what a sum holds.
    [[6, 6, 4], [6, 6, 4]],
  ]) {
    const answered = counts(new IOResult(...args))
    assert.deepEqual(answered, expected, String(args))
    for (const count of answered) assert.equal(typeof count, 'number')
  }
  assert.equal(new IOResult().isEmpty(), true)
  // A source whose every row was kept out was read, and a write that
  // produced rows from none wrote: neither is empty.
  for (const args of [[10, 8], [3, 0], [0, 2]]) {
    assert.equal(new IOResult(...args).isEmpty(), false, String(args))
  }
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
    [[1, 1, -1], 'skippedRows'],
  ]) {
    assert.throws(
      () => new IOResult(...args),
      new RegExp(`${name} must be a non-negative whole number of at most 2\\^53`),
      String(args),
    )
  }
  // A count is a number: text and a bigint are refused, never read.
  for (const value of ['10', 1n]) {
    assert.throws(() => new IOResult(value), /into rust type `f64`/)
  }
})

test('results add count by count, each stating its own skipped rows', () => {
  assert.ok(new IOResult(10, 8).add(new IOResult(5, 5)).equals(new IOResult(15, 13)))
  // A sum keeps the skipped rows each part stated, which the difference of
  // the summed counts need not be.
  const summed = new IOResult(2, 6).add(new IOResult(4, 0))
  assert.deepEqual(counts(summed), [6, 6, 4])
  assert.ok(summed.equals(new IOResult(6, 6, 4)))
  assert.equal(summed.equals(new IOResult(6, 6)), false)
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

test('a result is immutable, clones apart and renders the core text', () => {
  const result = new IOResult(10, 8)
  assert.throws(() => {
    'use strict'
    result.readRows = 1
  }, TypeError)
  assert.deepEqual(counts(result), [10, 8, 2])

  const copy = result.clone()
  assert.notEqual(copy, result)
  assert.ok(copy.equals(result))

  assert.equal(result.toString(), 'read 10 rows, wrote 8, skipped 2')
  assert.equal(`${new IOResult(2, 6)}`, 'read 2 rows, wrote 6, skipped 0')
})
