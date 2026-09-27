'use strict'

// Pins node/src/media/options.rs: the record settings every encoding shares.

const assert = require('node:assert/strict')
const test = require('node:test')

const { RecordOptions } = require('yggdryl')

test('a row offset is a setting of its own, set, carried and cleared', () => {
  const options = RecordOptions.from('trades.parquet')
  assert.equal(options.rowOffset, null)

  options.rowOffset = 5
  assert.equal(options.rowOffset, 5)
  const copy = options.withRowOffset(7)
  assert.equal(copy.rowOffset, 7)
  // A `with` is a copy: the options it was taken from keep their offset.
  assert.equal(options.rowOffset, 5)
  assert.equal(options.equals(copy), false)

  for (const value of [-1, 1.5, Number.NaN]) {
    assert.throws(() => {
      options.rowOffset = value
    }, /rowOffset/)
    assert.equal(options.rowOffset, 5)
  }
  options.rowOffset = null
  assert.equal(options.rowOffset, null)
})

test("a plan's offset is the row offset, and the offset is the plan's", () => {
  const options = RecordOptions.from('trades.parquet').withPlan(
    'select id limit 3 offset 2',
  )
  assert.equal(options.rowOffset, 2)
  assert.equal(options.maxRowSize, 3)
  assert.equal(options.plan.offset, 2)
  assert.equal(options.plan.limit, 3)

  const fresh = RecordOptions.from('trades.parquet')
  fresh.plan = options.plan.toString()
  assert.ok(fresh.equals(options))
})

test('null is a value, and clears every section it names', () => {
  const options = RecordOptions.from('trades.parquet')
    .withSelect(['id'])
    .withFilter('id > 1')
    .withMergeBy(['id'])
  assert.equal(options.select.toString(), 'id')
  assert.equal(options.mergeBy.toString(), 'id')

  options.select = null
  options.filter = null
  options.mergeBy = null
  assert.ok(options.equals(RecordOptions.from('trades.parquet')))

  // The plan with no section: every section it spells is cleared.
  const planned = RecordOptions.from('trades.parquet').withPlan("select id where id > 1")
  planned.plan = null
  assert.equal(planned.select.toString(), '*')
  assert.ok(planned.equals(RecordOptions.from('trades.parquet')))
})
