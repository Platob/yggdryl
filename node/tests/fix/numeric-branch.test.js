'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const { Field, fields, fix } = require('yggdryl')

function tagged(name, tag, dtype, branch = '') {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.tag = tag
  field.fix.branch = branch
  return field
}

function group(registry, name, counter, member, branch) {
  const component = fields.struct(`${name}Entry`, [member], { nullable: false })
  component.fix.branch = branch
  registry.createDefinition('components', component)
  const definition = fields.list(name, component)
  definition.fix.branch = branch
  definition.fix.counter = counter
  definition.fix.component = component.name
  registry.createDefinition('groups', definition)
  return definition
}

function registry(scoped) {
  const result = fix.FixRegistry.fromFields([
    tagged('MsgType', 35, 'utf8'),
    tagged('Symbol', 55, 'utf8'),
    tagged('CheckSum', 10, 'utf8'),
  ])
  for (const [branch, name, dtype] of [['alpha', 'Alpha', 'int32'], ['beta', 'Beta', 'utf8']]) {
    const counter = tagged(`No${name}Rows`, 6000, 'int32', branch)
    const member = tagged(`${name}ID`, 6001, dtype, branch)
    const tail = tagged(`${name}Value`, 6002, dtype, branch)
    for (const field of [counter, member, tail]) result.insert(field)
    const rows = group(result, `${name}Rows`, 6000, member, branch)
    if (scoped) {
      const message = fields.struct(`${name}Message`, [counter, rows, tail], { nullable: false })
      message.fix.branch = branch
      message.fix.msgtype = 'X'
      result.createDefinition('messages', message)
    }
  }
  const counter = tagged('NoAlphaOnlyRows', 6100, 'int32', 'alpha')
  const member = tagged('AlphaOnlyID', 6101, 'int32', 'alpha')
  result.insert(counter)
  result.insert(member)
  group(result, 'AlphaOnlyRows', 6100, member, 'alpha')
  return result
}

test('pinned numeric fields and groups use their own branch and scalar types', () => {
  const wire = Buffer.from('35=X|6000=1|6001=42|6002=7|55=AAPL|10=0|')
  for (const scoped of [false, true]) {
    const catalog = registry(scoped)
    for (const [branch, name, dtype, member, tail] of [
      ['alpha', 'Alpha', 'int32', 42, 7],
      ['beta', 'Beta', 'utf8', '42', '7'],
    ]) {
      const codec = new fix.FixCodec(catalog, { branch })
      const message = codec.parseFixLine(wire)
      assert.equal(message.branch, branch)
      assert.equal(message.byName(`No${name}Rows`).asJs(), 1)
      assert.equal(message.field.fieldByPath(`No${name}Rows`).dtype.toString(), 'int32')
      assert.equal(message.byPath(`${name}Rows.0.${name}ID`).asJs(), member)
      assert.equal(message.byName(`${name}Value`).asJs(), tail)
      assert.equal(message.field.fieldByPath(`${name}Value`).dtype.toString(), dtype)
      assert.equal(message.byTag(55).asJs(), 'AAPL')
      assert.deepEqual(message.intoBytes(124), wire)
    }
  }
})

test('a pinned numeric row never borrows an unrelated branch counter', () => {
  const codec = new fix.FixCodec(registry(false), { branch: 'beta' })
  const wire = Buffer.from('6100=1|6101=42|55=AAPL|')
  const message = codec.parseFixLine(wire)
  for (const name of ['NoAlphaOnlyRows', 'AlphaOnlyRows', 'AlphaOnlyID']) {
    assert.equal(message.getByName(name), null)
  }
  assert.equal(message.byName('6100').asJs(), '1')
  assert.equal(message.byName('6101').asJs(), '42')
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.deepEqual(message.intoBytes(124), wire)
})
