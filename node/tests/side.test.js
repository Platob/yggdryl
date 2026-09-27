'use strict'

// `node/src/side.rs`: `Side`, FIX's Side(54) as the enum a `side` column
// stores as the `int32` code of its member.

const test = require('node:test')
const assert = require('node:assert/strict')

const { DataType, Field, Side, fields } = require('yggdryl')

test('Side is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(Side))
  const codes = Object.values(Side)
  assert.deepEqual(codes, Array.from({ length: 18 }, (_, index) => index))
  assert.equal(Side.UNKNOWN, 0)
  assert.equal(Side.BUY, 1)
  assert.equal(Side.SELL, 2)
  assert.equal(Side.SSHORT, 5)
  assert.equal(Side.SELLUND, 17)
  assert.equal('_sideMembersNative' in require('yggdryl'), false)
})

test('a side column stores the code and reads back the member name', () => {
  const field = fields.side('side', { nullable: false })
  assert.equal(field.dtype.id, 'side')
  assert.equal(field.dtype.kind, 'enum')
  assert.equal(field.dtype.codeWidth, null)
  // The stored name, the FIX wire code and the folded specification name.
  for (const given of ['SELL', '2', 'Sell']) {
    assert.equal(new DataType('side').scalar(given).asJs(), 'SELL', given)
  }
  assert.throws(() => new DataType('side').scalar('not a side'))
  assert.equal(Field.from('side: side').dtype.id, 'side')
})
