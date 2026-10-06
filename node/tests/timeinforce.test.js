'use strict'

// `node/src/timeinforce.rs`: `TimeInForce`, how long an order stands - FIX's
// `TimeInForce(59)` code set - stored as the `uint8` code of its member, and
// its FIX reading.

const test = require('node:test')
const assert = require('node:assert/strict')

const {
  DataType,
  Field,
  Serie,
  enums,
  fix,
  graph,
  TimeInForce,
  fields,
  timeInForceFixCode,
  timeInForceFromFix,
} = require('yggdryl')

test('TimeInForce is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(TimeInForce))
  assert.deepEqual(TimeInForce, {
    UKNW: 0,
    DAY: 1,
    GTC: 2,
    OPG: 3,
    IOC: 4,
    FOK: 5,
    GTX: 6,
    GTD: 7,
    ATC: 8,
    GTHX: 9,
    ATX: 10,
    GFT: 11,
    GFA: 12,
    GFM: 13,
    OTHER: 99,
  })
  assert.equal('_timeInForceMembersNative' in require('yggdryl'), false)
  assert.ok(enums.dataTypeIds.includes('timeinforce'))
})

test('timeinforce is an enum datatype, no registered code', () => {
  const dtype = new DataType('timeinforce')
  assert.equal(dtype.id, 'timeinforce')
  assert.equal(dtype.kind, 'enum')
  assert.equal(dtype.codeWidth, null)
  assert.equal(dtype.toString(), 'timeinforce')
  assert.equal(Field.from('tif: timeinforce').dtype.id, 'timeinforce')
})

test('a timeinforce column stores the uint8 code and reads back the member name', () => {
  const field = fields.timeinforce('tif', { nullable: true })
  assert.equal(field.dtype.id, 'timeinforce')
  // The stored name in any case, the FIX name folded, the wire value, and
  // the stored code all name one member.
  for (const given of ['GTC', 'gtc', 'GoodTillCancel', 'good_till_cancel', '1', TimeInForce.GTC]) {
    assert.equal(new DataType('timeinforce').scalar(given).asJs(), 'GTC', String(given))
  }
  assert.throws(() => new DataType('timeinforce').scalar('not a time in force'))
  const serie = Serie.fromScalars(field, ['DAY', 'IOC', null])
  const array = serie.intoArrowArray()
  assert.equal(String(array.type), 'Uint8')
  assert.deepEqual([...array], [TimeInForce.DAY, TimeInForce.IOC, null])
  // An enum column casts to text as its names and to any integer as its codes.
  assert.equal(serie.cast(new Field('t', 'utf8')).scalar(1).asJs(), 'IOC')
  assert.equal(serie.cast(new Field('t', 'int64')).scalar(1).asJs(), TimeInForce.IOC)
})

test('the FIX reading redirects to the core', () => {
  assert.equal(timeInForceFromFix('0'), 'DAY')
  assert.equal(timeInForceFromFix('1'), 'GTC')
  assert.equal(timeInForceFromFix('A'), 'GFT')
  // A venue's own value reads as the catch-all.
  assert.equal(timeInForceFromFix('Z'), 'OTHER')
  assert.equal(timeInForceFixCode('GTC'), '1')
  assert.equal(timeInForceFixCode('GFM'), 'C')
  assert.equal(timeInForceFixCode('UKNW'), null)
  assert.equal(timeInForceFixCode('OTHER'), null)
  assert.throws(() => timeInForceFixCode('nope'), /timeinforce/)
})

test('a FIX message and a graph leaf answer the member name', () => {
  const codec = new fix.FixCodec(new fix.FixRegistry(), { excludeMsgtypes: [] })
  const line = (extra) => Buffer.from(`8=FIX.4.4|35=D|11=A|55=X|54=1|38=1|${extra}10=0|`)
  assert.equal(codec.parseFixLine(line('59=3|')).timeinforce, 'IOC')
  assert.equal(codec.parseFixLine(line('59=Q|')).timeinforce, 'OTHER')
  for (const given of ['GTC', 'gtc', '1', TimeInForce.GTC]) {
    assert.equal(new graph.OrderEvent(1, { crosscode: 'O', timeinforce: given }).timeinforce, 'GTC', String(given))
  }
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O' }).timeinforce, null)
  assert.ok(enums.operationColumns.includes('timeinforce'))
})
