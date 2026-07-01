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
