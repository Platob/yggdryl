'use strict'

// `node/src/side.rs`: `Side`, FIX's Side(54) as the enum a `side` column
// stores as the `uint8` code of its member.

const test = require('node:test')
const assert = require('node:assert/strict')

const { DataType, Field, Side, fields } = require('yggdryl')

test('Side is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(Side))
  const codes = Object.values(Side)
  assert.deepEqual(codes, Array.from({ length: 18 }, (_, index) => index))
  assert.equal(Side.UKNW, 0)
  assert.equal(Side.BUYS, 1)
  assert.equal(Side.SELL, 2)
  assert.equal(Side.SSHT, 5)
  assert.equal(Side.SELU, 17)
  assert.deepEqual(Object.keys(Side).slice(0, 3), ['UKNW', 'BUYS', 'SELL'])
  assert.equal('_sideMembersNative' in require('yggdryl'), false)
})

test("a side column stores the code and reads back the member's four-letter code", () => {
  const field = fields.side('side', { nullable: false })
  assert.equal(field.dtype.id, 'side')
  assert.equal(field.dtype.kind, 'enum')
  assert.equal(field.dtype.codeWidth, null)
  // The four-letter code, the FIX wire code and the folded specification
  // name; a stored name from before the codes is read and never written.
  for (const given of ['SELL', '2', 'Sell']) {
    assert.equal(new DataType('side').scalar(given).asJs(), 'SELL', given)
  }
  for (const given of ['BUYS', '1', 'Buy', 'BUY']) {
    assert.equal(new DataType('side').scalar(given).asJs(), 'BUYS', given)
  }
  for (const given of ['SSHT', '5', 'SellShort', 'SSHORT']) {
    assert.equal(new DataType('side').scalar(given).asJs(), 'SSHT', given)
  }
  assert.throws(() => new DataType('side').scalar('not a side'))
  assert.equal(Field.from('side: side').dtype.id, 'side')
})
