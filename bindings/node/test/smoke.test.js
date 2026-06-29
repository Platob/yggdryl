'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')

test('core.version', () => {
  assert.equal(typeof yggdryl.core.version(), 'string')
  assert.ok(yggdryl.core.version().length > 0)
})

test('schema.DataTypeId', () => {
  assert.equal(yggdryl.schema.DataTypeId.Int32, 0x04)
  assert.notEqual(yggdryl.schema.DataTypeId.Int32, yggdryl.schema.DataTypeId.Utf8)
})
