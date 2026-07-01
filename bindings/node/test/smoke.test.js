'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')

test('core.version', () => {
  assert.equal(typeof yggdryl.core.version(), 'string')
  assert.ok(yggdryl.core.version().length > 0)
})

test('core.hello', () => {
  assert.doesNotThrow(() => yggdryl.core.hello())
})

test('core.Whence', () => {
  assert.equal(yggdryl.core.Whence.Start, 0)
  assert.equal(yggdryl.core.Whence.End, 2)
  assert.notEqual(yggdryl.core.Whence.Start, yggdryl.core.Whence.End)
})
