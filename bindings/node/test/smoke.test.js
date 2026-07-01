'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')

test('core.version', () => {
  assert.equal(typeof yggdryl.core.version(), 'string')
  assert.ok(yggdryl.core.version().length > 0)
})

test('schema.DataTypeId', () => {
  assert.equal(yggdryl.schema.DataTypeId.Binary, 0x0d)
  assert.equal(yggdryl.schema.DataTypeId.Decimal128, 0x40)
  assert.notEqual(yggdryl.schema.DataTypeId.Binary, yggdryl.schema.DataTypeId.Decimal128)
})
