'use strict'

// `node/src/marketdatatype.rs`: `MarketDataType`, the type of its kind a market
// element is, stored as the `uint16` code of its member, and its FIX reading.

const test = require('node:test')
const assert = require('node:assert/strict')

const {
  DataType,
  Field,
  enums,
  fix,
  graph,
  MarketDataType,
  fields,
  marketDataTypeFixCode,
  marketDataTypeFromFix,
} = require('yggdryl')

test('MarketDataType is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(MarketDataType))
  const codes = Object.values(MarketDataType)
  assert.deepEqual(
    codes,
    [...codes].sort((a, b) => a - b),
  )
  assert.equal(MarketDataType.UNKN, 0)
  assert.equal(MarketDataType.ORDMKT, 101)
  assert.equal(MarketDataType.ORDLIMIT, 102)
  assert.equal(MarketDataType.ORDOTHER, 199)
  assert.equal(MarketDataType.QUOOTHER, 299)
  assert.equal(MarketDataType.TRDOTHER, 399)
  assert.equal(MarketDataType.BOOKBID, 400)
  assert.equal(MarketDataType.BOOKOTHER, 499)
  // The trade report, quote request, mass cancel and market data request
  // sets, each closed by its catch-all.
  assert.equal(MarketDataType.TRPTSUBMIT, 500)
  assert.equal(MarketDataType.TRPTOTHER, 599)
  assert.equal(MarketDataType.QRQMANUAL, 601)
  assert.equal(MarketDataType.QRQOTHER, 699)
  assert.equal(MarketDataType.MCXSECURITY, 701)
  assert.equal(MarketDataType.MCXOTHER, 799)
  assert.equal(MarketDataType.MDRSNAPSHOT, 800)
  assert.equal(MarketDataType.MDROTHER, 899)
  assert.equal('_marketDataTypeMembersNative' in require('yggdryl'), false)
})

test('a marketdatatype column stores the code and reads back the member name', () => {
  const field = fields.marketdatatype('type', { nullable: false })
  assert.equal(field.dtype.id, 'marketdatatype')
  assert.equal(field.dtype.kind, 'enum')
  for (const given of ['ORDLIMIT', 'ordlimit', 'limit']) {
    assert.equal(new DataType('marketdatatype').scalar(given).asJs(), 'ORDLIMIT', given)
  }
  assert.throws(() => new DataType('marketdatatype').scalar('102'), /marketdatatype/)
  assert.throws(() => new DataType('marketdatatype').scalar('not a type'))
  assert.equal(Field.from('type: marketdatatype').dtype.id, 'marketdatatype')
})

test('the FIX reading redirects to the core', () => {
  assert.equal(marketDataTypeFromFix(40, '2'), 'ORDLIMIT')
  assert.equal(marketDataTypeFromFix(40, 'nope'), 'ORDOTHER')
  assert.equal(marketDataTypeFromFix(828, 'nope'), 'TRDOTHER')
  assert.equal(marketDataTypeFromFix(54, '1'), null)
  // The four sets read off TradeReportType(856), QuoteRequestType(303),
  // MassCancelRequestType(530) and SubscriptionRequestType(263).
  for (const [tag, wire, member, other] of [
    [856, '0', 'TRPTSUBMIT', 'TRPTOTHER'],
    [303, '1', 'QRQMANUAL', 'QRQOTHER'],
    [530, '1', 'MCXSECURITY', 'MCXOTHER'],
    [263, '0', 'MDRSNAPSHOT', 'MDROTHER'],
  ]) {
    assert.equal(marketDataTypeFromFix(tag, wire), member)
    assert.equal(marketDataTypeFromFix(tag, 'nope'), other)
    assert.deepEqual(marketDataTypeFixCode(member), { tag, wire })
    assert.equal(marketDataTypeFixCode(other), null)
  }
  assert.deepEqual(marketDataTypeFixCode('ORDLIMIT'), { tag: 40, wire: '2' })
  assert.equal(marketDataTypeFixCode('UNKN'), null)
  assert.throws(() => marketDataTypeFixCode('nope'), /marketdatatype/)
})

test('a registry reads a type through its dictionary, then the core table', () => {
  const registry = new fix.FixRegistry()
  for (const [tag, wire] of [
    [40, '2'],
    [40, 'Z'],
    [54, '1'],
  ]) {
    assert.equal(registry.marketdatatypeOf(tag, wire), marketDataTypeFromFix(tag, wire))
  }
})

test('a field states its own wire values under FIX:marketdatatype', () => {
  const field = new Field('OrdType', 'utf8')
  field.fix.tag = 40
  assert.deepEqual(field.fix.marketdatatypes, [])
  field.fix.marketdatatypes = [{ wire: 'Z', marketdatatype: 'ORDPEGGED' }]
  assert.deepEqual(field.fix.marketdatatypes, [{ wire: 'Z', marketdatatype: 'ORDPEGGED' }])
  assert.equal(field.get('FIX:marketdatatype'), '["Z=ORDPEGGED"]')
  assert.throws(() => {
    field.fix.marketdatatypes = [{ wire: 'Z', marketdatatype: 'nope' }]
  }, /marketdatatype/)
  assert.deepEqual(field.fix.marketdatatypes, [{ wire: 'Z', marketdatatype: 'ORDPEGGED' }])
  field.fix.marketdatatypes = []
  assert.equal(field.has('FIX:marketdatatype'), false)
})

test('a FIX message and a graph leaf answer the type of their kind', () => {
  const codec = new fix.FixCodec(new fix.FixRegistry(), { excludeMsgtypes: [] })
  const line = (extra) => Buffer.from(`8=FIX.4.4|35=D|55=X|54=1|38=1|${extra}10=0|`)
  assert.equal(codec.parseFixLine(line('40=2|')).marketdatatype, 'ORDLIMIT')
  assert.equal(codec.parseFixLine(line('')).marketdatatype, 'UNKN')
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O' }).marketdatatype, 'UNKN')
  assert.equal('ordtype' in new graph.OrderEvent(1, { crosscode: 'O' }), false)
  assert.ok(enums.marketColumns.includes('marketdatatype'))
})
