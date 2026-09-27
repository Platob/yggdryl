'use strict'

// Pins node/src/text/options.rs: flat plain-text record options.

const assert = require('node:assert/strict')
const test = require('node:test')

const { TextOptions } = require('yggdryl')

test('the capture names are the row header groups, in column order', () => {
  const options = new TextOptions()
  assert.deepEqual(options.captureNames, [])

  options.rowheader = '^\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+) '
  assert.deepEqual(options.captureNames, ['level', 'id'])

  options.rowheader = null
  assert.deepEqual(options.captureNames, [])
})

test('a text row offset is set, copied, and planned like any other', () => {
  const options = new TextOptions()
  assert.equal(options.rowOffset, null)

  options.rowOffset = 3
  assert.equal(options.rowOffset, 3)
  assert.equal(options.withRowOffset(4).rowOffset, 4)
  assert.equal(options.rowOffset, 3)
  assert.throws(() => {
    options.rowOffset = -1
  }, /rowOffset/)
  assert.equal(options.rowOffset, 3)

  const planned = new TextOptions().withPlan('select body limit 2 offset 1')
  assert.equal(planned.rowOffset, 1)
  assert.equal(planned.maxRowSize, 2)
})

test('null clears the sections a TextOptions carries', () => {
  const { TextOptions } = require('yggdryl')
  const options = new TextOptions().withSelect(['body']).withFilter("body = 'x'")
  options.select = null
  options.filter = null
  options.mergeBy = null
  options.plan = null
  assert.equal(options.select.toString(), '*')
  assert.ok(options.equals(new TextOptions()))
})
