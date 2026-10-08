'use strict'

// `node/src/pluginside.rs`: `PluginSide`, the role of a FIX plugin - the side
// of the session a dialect's plugin stands on - stored as the `uint8` code of
// its member, and the reading of a CBlock's plugin class.

const test = require('node:test')
const assert = require('node:assert/strict')

const {
  DataType,
  Field,
  PluginSide,
  Serie,
  Side,
  enums,
  fields,
  fix,
  pluginSideFromPluginType,
} = require('yggdryl')

test('PluginSide is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(PluginSide))
  assert.deepEqual(PluginSide, { UKNW: 0, BUYS: 1, SELL: 2 })
  assert.equal('_pluginSideMembersNative' in require('yggdryl'), false)
  assert.ok(enums.dataTypeIds.includes('pluginside'))
  // A separate enum from `Side`, though two names are spelled alike.
  assert.notEqual(PluginSide, Side)
  assert.equal(Side.SELL, PluginSide.SELL)
  assert.equal('SSHT' in PluginSide, false)
})

test('pluginside is an enum datatype, no registered code', () => {
  const dtype = new DataType('pluginside')
  assert.equal(dtype.id, 'pluginside')
  assert.equal(dtype.kind, 'enum')
  assert.equal(dtype.codeWidth, null)
  assert.equal(dtype.toString(), 'pluginside')
  assert.equal(Field.from('role: pluginside').dtype.id, 'pluginside')
})

test('a pluginside column stores the uint8 code and reads back the member name', () => {
  const field = fields.pluginside('role', { nullable: true })
  assert.equal(field.dtype.id, 'pluginside')
  // The stored name in any case, the role's own name folded, and the stored
  // code all name one member.
  for (const given of ['SELL', 'sell', 'SellSide', 'sell-side', 'sell_side', PluginSide.SELL]) {
    assert.equal(new DataType('pluginside').scalar(given).asJs(), 'SELL', String(given))
  }
  for (const given of ['BUYS', 'buys', 'BuySide', 'buy-side', PluginSide.BUYS]) {
    assert.equal(new DataType('pluginside').scalar(given).asJs(), 'BUYS', String(given))
  }
  assert.equal(new DataType('pluginside').scalar('unknown').asJs(), 'UKNW')
  // A `Side` member is no plugin side, however alike the two enums spell.
  assert.throws(() => new DataType('pluginside').scalar('SSHT'))
  const serie = Serie.fromScalars(field, ['BUYS', 'SELL', null, 'UKNW'])
  const array = serie.intoArrowArray()
  assert.equal(String(array.type), 'Uint8')
  assert.deepEqual([...array], [PluginSide.BUYS, PluginSide.SELL, null, PluginSide.UKNW])
  // An enum column casts to text as its names and to any integer as its codes.
  assert.equal(serie.cast(new Field('t', 'utf8')).scalar(1).asJs(), 'SELL')
  assert.equal(serie.cast(new Field('t', 'int64')).scalar(0).asJs(), PluginSide.BUYS)
})

test('a plugin class names its role on its last segment, never throwing', () => {
  const cblock = 'com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock'
  assert.equal(pluginSideFromPluginType(`${cblock}.BuySideFIXCPluginCBlock`), 'BUYS')
  assert.equal(pluginSideFromPluginType(`${cblock}.SellSideFIXCPluginCBlock`), 'SELL')
  assert.equal(pluginSideFromPluginType('x.Buy_Side_FIXCPluginCBlock'), 'BUYS')
  assert.equal(pluginSideFromPluginType('x.FIXCPluginCBlock'), 'UKNW')
  // Only the last segment is read: a package naming a side names no role.
  assert.equal(pluginSideFromPluginType('buyside.FIXCPluginCBlock'), 'UKNW')
  assert.equal(pluginSideFromPluginType(''), 'UKNW')
})

test('the dictionary renders the members as its intrinsic plugin side set', () => {
  const registry = new fix.FixRegistry()
  assert.deepEqual(
    registry.codeset('msgpluginsidecodeset').codes.map(({ value, name }) => [value, name]),
    Object.entries(PluginSide).map(([name, code]) => [String(code), name]),
  )
  const field = fix.crateFields().find((held) => held.name === 'msgpluginside')
  // StrikePx keeps 65036; plugin ID is 65042 and plugin side follows at 65043.
  assert.equal(field.fix.tag, 65043)
  assert.equal(field.dtype.id, 'pluginside')
  assert.equal(field.nullable, false)
  assert.equal(field.fix.codeset, 'msgpluginsidecodeset')
})
