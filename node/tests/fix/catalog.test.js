'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { DataType, Field, Scalar, TextLine, fields, fix } = require('yggdryl')

function tagged(name, tag, dtype = 'utf8') {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.tag = tag
  return field
}

function message(name, code, children = []) {
  const field = fields.struct(name, children, { nullable: false })
  field.fix.msgtype = code
  return field
}

function members(field) {
  const held = []
  for (let at = 0; at < field.fieldLen; at += 1) held.push(field.fieldAt(at))
  return held
}

function catalog() {
  const registry = fix.FixRegistry.fromFields([tagged('NoPartyIDs', 453, 'int32'), tagged('PartyID', 448)])
  const member = registry.field(448)
  member.fix.fieldRef = 'PartyID'
  const component = fields.struct('Party', [member], { nullable: false })
  registry.createDefinition('components', component)
  const group = fields.list('Parties', component)
  group.fix.counter = 453
  group.fix.component = 'Party'
  registry.createDefinition('groups', group)
  const occurrence = registry.definition('groups', 'Parties')
  occurrence.fix.group = 'Parties'
  const counter = registry.field(453)
  counter.fix.fieldRef = 'NoPartyIDs'
  registry.createDefinition('messages', message('NewOrderSingle', 'D', [counter, occurrence]))
  return registry
}

test('category CRUD refreshes references and refuses invalid changes atomically', (t) => {
  const registry = catalog()
  const before = registry.intoJson()
  for (const [category, name] of [['fields', 'PartyID'], ['components', 'Party'], ['groups', 'Parties']]) {
    assert.throws(() => registry.removeDefinition(category, name))
    assert.equal(registry.intoJson(), before)
  }
  assert.throws(() => registry.createDefinition('fields', tagged('DifferentName', 448)))
  assert.equal(registry.intoJson(), before)
  for (const [category, name] of [['fields', 'PartyID'], ['components', 'Party'], ['groups', 'Parties'], ['messages', 'NewOrderSingle']]) {
    const original = registry.definition(category, name)
    const changed = original.clone()
    changed.setName(name.toLowerCase())
    changed.fix.description = 'Reviewed'
    assert.ok(registry.updateDefinition(category, changed).equals(original))
    assert.equal(registry.definition(category, name).name, name)
    assert.equal(registry.definition(category, name).fix.description, 'Reviewed')
    assert.throws(() => registry.createDefinition(category, changed))
  }
  assert.equal(registry.fieldByPath('NewOrderSingle.Parties.PartyID').fix.description, 'Reviewed')
  const settled = registry.intoJson()
  assert.throws(() => registry.updateDefinition('fields', tagged('PartyID', 448, 'int32')))
  assert.equal(registry.intoJson(), settled)
  const invalid = registry.field(448)
  invalid.fix.fieldRef = 'PartyID'
  invalid.fix.description = 'Occurrence override'
  assert.throws(() => registry.createDefinition('components', fields.struct('Invalid', [invalid], { nullable: false })))
  assert.equal(registry.intoJson(), settled)
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-node-catalog-'))
  t.after(() => fs.rmSync(folder, { recursive: true, force: true }))
  registry.writeInto(folder)
  assert.ok(fix.FixRegistry.fromHandle(folder).equals(registry))
  for (const [category, name] of [['messages', 'NewOrderSingle'], ['groups', 'Parties'], ['components', 'Party'], ['fields', 'PartyID'], ['fields', 'NoPartyIDs']]) {
    assert.ok(registry.removeDefinition(category, name))
    assert.equal(registry.getDefinition(category, name), null)
    assert.equal(registry.removeDefinition(category, name), null)
  }
  // Only the crate's own fields are left: they seed every registry and
  // are never a definition a caller can remove.
  assert.equal(registry.size, fix.crateFields().length)
})

test('addDefinition folds a definition into the one its name reaches', () => {
  const registry = catalog()
  assert.equal(registry.addField(tagged('PartyNote', 9002)), true)
  const note = registry.field(9002)
  note.fix.fieldRef = 'PartyNote'
  const extended = registry.definition('components', 'Party')
  extended.setDtype(DataType.fromFields([...members(extended), note]))
  assert.equal(registry.addDefinition('components', extended), false)

  // The stored members keep their order and the incoming one is appended; the
  // group and the message that reference the component see it without holding
  // a copy, and the reference resolves again.
  assert.deepEqual(members(registry.definition('components', 'Party')).map(held => held.name), ['PartyID', 'PartyNote'])
  for (const spelled of ['Party.PartyNote', 'Parties.PartyNote', 'NewOrderSingle.Parties.PartyNote']) {
    const member = registry.fieldByPath(spelled)
    assert.equal(member.fix.tag, 9002, spelled)
    assert.equal(member.fix.fieldRef, 'partynote', spelled)
  }
  assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))

  // A message extends the same way and keeps its wire code.
  const order = registry.definition('messages', 'NewOrderSingle')
  order.setDtype(DataType.fromFields([...members(order), Field.from('Text: utf8')]))
  assert.equal(registry.addDefinition('messages', order), false)
  const held = registry.definition('messages', 'NewOrderSingle')
  assert.equal(held.fix.msgtype, 'D')
  assert.deepEqual(members(held).map(member => member.name), ['NoPartyIDs', 'Parties', 'Text'])

  // A name no definition reaches arrives whole, and `'fields'` redirects a
  // scalar to `addField`.
  const hop = fields.struct('Hop', [Field.from('HopID: utf8')], { nullable: false })
  assert.equal(registry.addDefinition('components', hop), true)
  assert.equal(registry.addDefinition('fields', tagged('Symbol', 55)), true)
  assert.equal(registry.field(55).name, 'Symbol')

  // One level deep: a member both sides declare stays the stored one, so an
  // incoming member restating it under another datatype refuses the whole
  // call, and the strict verb still refuses the name outright.
  const before = registry.intoJson()
  const disagreeing = fields.struct('Party', [Field.from('partynote: int32')], { nullable: false })
  assert.throws(() => registry.addDefinition('components', disagreeing), /Party\.PartyNote/)
  assert.equal(registry.intoJson(), before)
  assert.throws(() => registry.createDefinition('components', registry.definition('components', 'Party')))
  assert.equal(registry.intoJson(), before)
})

test('inline codes and the complete native catalog survive snapshots', () => {
  const registry = catalog()
  const coded = registry.field(448)
  coded.set('fix:codes', '{"codes":[{"value":"B","name":"Broker"}]}')
  registry.updateDefinition('fields', coded)
  assert.match(registry.fieldByPath('NewOrderSingle.Parties.PartyID').get('fix:codes'), /Broker/)
  const vendor = tagged('Vendor', 9001)
  vendor.fix.branch = 'venue'
  registry.insert(vendor)
  const document = registry.toJSON()
  assert.deepEqual(Object.keys(document).sort(), ['branches', 'components', 'fields', 'groups', 'messages'])
  const branch = document.branches.find(value => value.name === 'venue')
  branch.version = '5.1.258'
  branch.aliases = ['counterparty']
  const declared = fix.FixRegistry.fromJson(JSON.stringify(document))
  assert.deepEqual(declared.toJSON().branches.find(value => value.name === 'venue').aliases, ['counterparty'])
  for (const copy of [declared.clone(), fix.FixRegistry.fromJson(declared.intoJson())]) {
    assert.ok(copy.equals(declared))
    assert.equal(copy.stableHash(), declared.stableHash())
    assert.equal(copy.groupByCounter('453:').name, 'Parties')
    assert.equal(copy.getGroupByCounter('999:'), null)
  }
  assert.throws(() => registry.definitions('codesets'))
  const changed = registry.clone()
  const definition = changed.definition('messages', 'NewOrderSingle')
  definition.fix.description = 'Different'
  changed.updateDefinition('messages', definition)
  assert.equal(changed.equals(registry), false)
  assert.notEqual(changed.stableHash(), registry.stableHash())
})

test('native category cursors release holds on exhaustion and early close', () => {
  const registry = catalog()
  const cursor = registry.definitions('components')[Symbol.iterator]()
  assert.equal(cursor.next().value.name, 'Party')
  assert.throws(() => registry.insert(tagged('Extra', 9000)), /shared/)
  assert.equal(cursor.next().done, true)
  assert.equal(cursor.next().done, true)
  registry.insert(tagged('Extra', 9000))
  for (const ignored of registry.definitions('groups')) { void ignored; break }
  assert.ok(registry.remove(9000))
  const messages = registry.msgtypes()[Symbol.iterator]()
  messages.return()
  registry.insert(tagged('AfterClose', 9000))
  assert.ok(registry.clone().removeDefinition('messages', 'NewOrderSingle'))
})

test('message singleton indices distinguish names from another wire code', () => {
  const registry = new fix.FixRegistry()
  registry.createDefinition('messages', message('D', 'X'))
  registry.createDefinition('messages', message('NewOrderSingle', 'D'))
  registry.createDefinition('messages', message('BridgeReport', 'P Report Ack'))
  const values = [...registry.msgtypes()]
  assert.deepEqual(values.map(value => [value.name, value.asStr()]), [['BridgeReport', 'P Report Ack'], ['D', 'X'], ['NewOrderSingle', 'D']])
  assert.equal(registry.msgtype('D').name, 'NewOrderSingle')
  assert.equal(registry.msgtype('bridgereport').asStr(), 'P Report Ack')
  assert.equal(registry.getMsgtype('p report ack'), null)
  assert.equal(values[1].clone().asStr(), 'X')
  assert.equal(values[1].compare(values[1].clone()), 0)
  assert.equal(values[0].compare(values[1]), -1)
  assert.equal(values[1].compare(values[0]), 1)
  assert.equal(values[1].stableHash(), values[1].clone().stableHash())
  const projected = values[1].asField()
  projected.setName('Changed')
  projected.fix.msgtype = 'Changed'
  assert.equal(values[1].name, 'D')
  assert.equal(values[1].asStr(), 'X')
  assert.equal(registry.definition('messages', 'D').fix.msgtype, 'X')
  assert.throws(() => new fix.MsgType())
  assert.throws(() => registry.createDefinition('messages', message('AnotherOrder', 'D')), /shared/)
  const ambiguous = registry.clone()
  ambiguous.createDefinition('messages', message('AnotherOrder', 'D'))
  assert.equal(ambiguous.getMsgtype('D'), null)
  assert.throws(() => ambiguous.msgtype('D'))
  const held = catalog().msgtype('D')
  assert.equal(held.getGroupByCounter('453:').name, 'Parties')
  assert.equal(held.getGroupByCounter('999:'), null)
})

test('numeric counters remain int32 beside message-scoped occurrence lists', () => {
  const registry = catalog()
  const codec = new fix.FixCodec(registry)
  const values = [...codec.parseLine(Buffer.from('35=D|453=2|448=ONE|448=TWO|'))]
  assert.equal(values.length, 1)
  const value = values[0]
  assert.equal(registry.field(453).dtype.toString(), 'int32')
  // A named definition carries a derived tag of its own now, taken from the
  // block above every published tag (FixId::DEFINITION_TAG_MIN..MAX).
  const parties = registry.definition('groups', 'Parties').fix.tag
  assert.ok(parties >= 100_000 && parties < 1_100_000, `derived definition tag, got ${parties}`)
  assert.equal(value.byTag(453).asJs(), 2)
  assert.equal(value.byPath('Parties.0.PartyID').asJs(), 'ONE')
  assert.equal(value.byPath('Parties.1.PartyID').asJs(), 'TWO')
  assert.deepEqual(value.anomalies(), [])
  assert.match(value.intoBytes(124).toString(), /453=2\|448=ONE\|448=TWO/)
})

function wildcard(size = 2) {
  return {
    request: { mbean: 'com.ullink.ulbridge.sessioninterfaces.plugins:*', type: 'read' },
    value: Object.fromEntries(Array.from({ length: size }, (_, index) => [
      `com.ullink.ulbridge.sessioninterfaces.plugins:name=Item${index},plugin-type=FIX,type=Plugin`,
      { Name: `Item${index}`, CurrentPort: 9000 + index },
    ])),
    status: 200,
  }
}

test('UlPlugin iterators own selected values and exchange identity', () => {
  const document = wildcard()
  const cursor = fix.UlPlugin.fromJsonScalar(document)[Symbol.iterator]()
  const first = cursor.next().value
  document.value = {}
  assert.equal(cursor.next().value.name, 'Item1')
  assert.equal(cursor.next().done, true)
  assert.equal(cursor.next().done, true)
  const sibling = wildcard()
  Object.values(sibling.value)[1].CurrentPort = 9999
  const same = fix.UlPlugin.fromJsonScalar(sibling)[Symbol.iterator]().next().value
  assert.ok(same.equals(first))
  assert.equal(same.stableHash(), first.stableHash())
  sibling.status = 503
  const changed = fix.UlPlugin.fromJsonScalar(sibling)[Symbol.iterator]().next().value
  assert.equal(changed.equals(first), false)
  assert.notEqual(changed.stableHash(), first.stableHash())
  const rebuilt = new fix.UlPlugin(first.mbean, first.asAttributes(), first.asEnvelope())
  assert.ok(rebuilt.equals(first))
  assert.ok(first.clone().equals(first))
  assert.equal(rebuilt.stableHash(), first.stableHash())
  assert.throws(() => fix.UlPlugin.fromJsonScalar([wildcard(), null]), /ulconfig\[1\]/)
  assert.equal([...fix.UlPlugin.fromJsonBytes(Buffer.from(JSON.stringify(wildcard())))].length, 2)
})

test('bulk message streams preserve flat configuration rows and fuse', () => {
  const registry = new fix.FixRegistry()
  registry.withUlbridgeFields()
  const codec = new fix.FixCodec(registry, { branch: 'ulbridge' })
  const error = { request: { mbean: 'com.ullink.ulbridge:type=Bridge', type: 'read' }, status: 404, error: 'missing' }
  const request = { mbean: 'com.ullink.ulbridge:type=Bridge', type: 'read' }
  const body = Buffer.from(JSON.stringify([wildcard(), error, request]))
  const cursor = codec.parseLine(body)
  assert.ok(cursor instanceof fix.FixMessages)
  assert.equal(cursor[Symbol.iterator](), cursor)
  const values = [...cursor]
  assert.equal(values.length, 4)
  assert.deepEqual(values.slice(0, 2).map(value => value.byName('Name').asJs()), ['Item0', 'Item1'])
  assert.equal(values[2].byName('Status').asJs(), 404)
  assert.equal(values[2].byName('Error').asJs(), 'missing')
  assert.equal(values[3].byName('Operation').asJs(), 'read')
  assert.ok(values.every(value => value.getByName('SessionInterfaces') === null))
  assert.equal(cursor.next().done, true)
  assert.equal(cursor.next().done, true)
  assert.equal([...codec.parseUlconfigLine(body)].length, 4)
  assert.equal([...codec.parseTextLine(new TextLine(17, body))].length, 4)
  const selected = fix.UlPlugin.fromFixmsg(values[0])
  assert.equal(selected.name, 'Item0')
  assert.equal(selected.intoFixmsg(codec).byName('Name').asJs(), 'Item0')
  const invalid = new fix.UlPlugin(null, { CurrentPort: NaN }, {})
  assert.throws(() => invalid.intoFixmsg(codec), /non-finite/)
  const schema = fix.schema(registry)
  assert.equal(values[0].intoRow(schema).asJs().length, schema.fieldLen)
  assert.equal(typeof values[0].stableHash(), 'bigint')
})

// Only `ulbridge` is registered on request: the crate's own fields seed every
// registry, so there is no `withCrateFields` left to refuse.
for (const [method, vocabulary] of [['withUlbridgeFields', fix.ulbridgeFields]]) {
  test(`${method} refuses atomically with a named catalog present`, () => {
    const registry = catalog()
    const conflict = vocabulary().at(-1)
    conflict.setName('ConflictingName')
    registry.insert(conflict)
    const before = registry.intoJson()
    assert.throws(() => registry[method]())
    assert.equal(registry.intoJson(), before)
  })
}

for (const [property, key, value] of [['counter', 'counter', 453], ['component', 'component', 'Party'], ['fieldRef', 'field', 'PartyID'], ['group', 'group', 'Parties'], ['msgtype', 'msgtype', 'P Report Ack']]) {
  test(`typed FIX ${property} shares native metadata and rejects invalid writes atomically`, () => {
    const field = Field.from('Definition: utf8')
    field.fix[property] = value
    const expected = ['component', 'fieldRef', 'group'].includes(property) ? value.toLowerCase() : value
    assert.equal(field.fix[property], expected)
    assert.equal(field.get(`fix:${key}`), String(expected))
    const before = JSON.stringify(field)
    for (const invalid of property === 'counter' ? [-1, 1.5, 2 ** 31] : ['', '\u0000']) {
      assert.throws(() => { field.fix[property] = invalid })
      assert.equal(JSON.stringify(field), before)
    }
    assert.throws(() => field.http[property], /http/)
    assert.throws(() => { field.http[property] = value }, /http/)
    assert.equal(field.fix.delete(key), true)
    assert.equal(field.fix[property], null)
  })
}
