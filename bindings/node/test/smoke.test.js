'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const { version } = require('../index.js')

test('version', () => {
  assert.equal(typeof version(), 'string')
  assert.ok(version().length > 0)
})
