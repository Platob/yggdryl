'use strict'

// `node/src/marketdatakind.rs`: `MarketDataKind`, FIX's MsgCat code set as
// the enum a `marketdatakind` column stores as the `int32` code of its member.

const test = require('node:test')
const assert = require('node:assert/strict')

const { DataType, Field, MarketDataKind, fields } = require('yggdryl')

test('MarketDataKind is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(MarketDataKind))
  const codes = Object.values(MarketDataKind)
  assert.deepEqual(codes, Array.from({ length: 22 }, (_, index) => index))
  assert.equal(MarketDataKind.UNKN, 0)
  assert.equal(MarketDataKind.BOOK, 3)
  assert.equal(MarketDataKind.EXEC, 8)
  assert.equal(MarketDataKind.ORDR, 10)
  assert.equal(MarketDataKind.QUOT, 14)
  assert.equal(MarketDataKind.TRAD, 21)
  assert.equal('_marketDataKindMembersNative' in require('yggdryl'), false)
})

test('a marketdatakind column stores the code and reads back the member name', () => {
  const field = fields.marketdatakind('kind', { nullable: false })
  assert.equal(field.dtype.id, 'marketdatakind')
  assert.equal(field.dtype.kind, 'enum')
  assert.equal(field.dtype.codeWidth, null)
  for (const given of ['ORDR', 'ordr', 'order']) {
    assert.equal(new DataType('marketdatakind').scalar(given).asJs(), 'ORDR', given)
  }
  // A stored code is an integer, never text.
  assert.throws(() => new DataType('marketdatakind').scalar('10'), /marketdatakind/)
  assert.throws(() => new DataType('marketdatakind').scalar('not a kind'))
  assert.equal(Field.from('kind: marketdatakind').dtype.id, 'marketdatakind')
})
