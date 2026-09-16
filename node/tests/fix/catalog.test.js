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
  registry.createDefinition('components', message('NewOrderSingle', 'D', [counter, occurrence]))
  return registry
}

test('category CRUD refreshes references and refuses invalid changes atomically', (t) => {
  const registry = catalog()
  const before = registry.intoJson()
  for (const [category, name] of [['fields', 'PartyID'], ['components', 'Party'], ['groups', 'Parties']]) {
    assert.throws(() => registry.removeDefinition(category, name))
    assert.equal(registry.intoJson(), before)
  }
  // One namespace: a held tag under another name is a new field beside the
  // holder, which gains the name as an alias while the bare tag keeps
  // answering it; a held name under another tag is what is refused.
  {
    const beside = registry.clone()
    beside.createDefinition('fields', tagged('DifferentName', 448))
    assert.equal(beside.field(448).name, 'PartyID')
    assert.ok(beside.field(448).fix.names.includes('DifferentName'))
    assert.equal(beside.fieldByName('differentname').fix.tag, 448)
    assert.notEqual(beside.fieldByName('differentname').fix.id, beside.field(448).fix.id)
  }
  assert.throws(() => registry.createDefinition('fields', tagged('PartyID', 9448)), /existing FIX definition/)
  assert.equal(registry.intoJson(), before)
  for (const [category, name] of [['fields', 'PartyID'], ['components', 'Party'], ['groups', 'Parties'], ['components', 'NewOrderSingle']]) {
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
  for (const [category, name] of [['components', 'NewOrderSingle'], ['groups', 'Parties'], ['components', 'Party'], ['fields', 'PartyID'], ['fields', 'NoPartyIDs']]) {
    assert.ok(registry.removeDefinition(category, name))
    assert.equal(registry.getDefinition(category, name), null)
    assert.equal(registry.removeDefinition(category, name), null)
  }
  // Only what seeds every registry is left: the crate's own thirty-four
  // scalar fields and the standard SendingTime (52) and TransactTime (60)
  // clocks (`seeded_fields()` in `rust/tests/fix.rs`), all of them in the
  // fields category. The crate's thirty-six definitions add the altids
  // group and the instids struct, which are filed by the shapes they have.
  assert.equal(registry.size, 36)
  assert.equal([...registry.definitions('fields')].length, 36)
  assert.equal(registry.fieldByTag(52).name, 'sendingtime')
  assert.equal(registry.fieldByTag(60).name, 'transacttime')
  assert.equal(fix.crateFields().length, 36)
  assert.equal(registry.groupByTag(65020).name, 'altids')
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
  const order = registry.definition('components', 'NewOrderSingle')
  order.setDtype(DataType.fromFields([...members(order), Field.from('Text: utf8')]))
  assert.equal(registry.addDefinition('components', order), false)
  const held = registry.definition('components', 'NewOrderSingle')
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
  coded.set('fix:codes', '[{"value":"B","name":"Broker"}]')
  registry.updateDefinition('fields', coded)
  assert.match(registry.fieldByPath('NewOrderSingle.Parties.PartyID').get('fix:codes'), /Broker/)
  const vendor = tagged('Vendor', 9001)
  vendor.fix.branches = ['venue']
  registry.insert(vendor)
  const document = registry.toJSON()
  // Three categories and nothing else: a dictionary's membership is metadata
  // on the field it contributed to, so it travels inside `fields`.
  assert.deepEqual(Object.keys(document).sort(), ['components', 'fields', 'groups'])
  assert.equal(document.fields.find(value => value.name === 'Vendor').metadata['fix:branches'], 'venue')
  const declared = fix.FixRegistry.fromJson(JSON.stringify(document))
  assert.deepEqual(declared.fieldByTag(9001).fix.branches, ['venue'])
  assert.deepEqual(declared.dialects(), ['venue'])
  for (const copy of [declared.clone(), fix.FixRegistry.fromJson(declared.intoJson())]) {
    assert.ok(copy.equals(declared))
    assert.equal(copy.stableHash(), declared.stableHash())
    assert.equal(copy.groupByTag(453).name, 'Parties')
    assert.equal(copy.getGroupByTag(999), null)
  }
  // A counter is a tag: an exact number, never text.
  assert.throws(() => declared.groupByTag(1.5), /tag must be a signed 32-bit integer/)
  assert.throws(() => declared.getGroupByTag('453'), /into rust type `f64`/)
  assert.throws(() => registry.definitions('codesets'))
  const changed = registry.clone()
  const definition = changed.definition('components', 'NewOrderSingle')
  definition.fix.description = 'Different'
  changed.updateDefinition('components', definition)
  assert.equal(changed.equals(registry), false)
  assert.notEqual(changed.stableHash(), registry.stableHash())
})

test('native category cursors release holds on exhaustion and early close', () => {
  const registry = catalog()
  const cursor = registry.definitions('components')[Symbol.iterator]()
  // A message is a component: the two iterate together, in
  // name order.
  assert.equal(cursor.next().value.name, 'NewOrderSingle')
  assert.throws(() => registry.insert(tagged('Extra', 9000)), /shared/)
  assert.equal(cursor.next().value.name, 'Party')
  // The crate's own is behind them: `instids`, the Struct that joins an
  // instrument's identifiers, which every registry carries as it carries
  // the crate's own fields.
  assert.equal(cursor.next().value.name, 'instids')
  assert.equal(cursor.next().done, true)
  assert.equal(cursor.next().done, true)
  registry.insert(tagged('Extra', 9000))
  for (const ignored of registry.definitions('groups')) { void ignored; break }
  assert.ok(registry.remove(9000))
  const messages = registry.msgtypes()[Symbol.iterator]()
  messages.return()
  registry.insert(tagged('AfterClose', 9000))
  assert.ok(registry.clone().removeDefinition('components', 'NewOrderSingle'))
})

test('message singleton indices distinguish names from another wire code', () => {
  const registry = new fix.FixRegistry()
  registry.createDefinition('components', message('D', 'X'))
  registry.createDefinition('components', message('NewOrderSingle', 'D'))
  registry.createDefinition('components', message('BridgeReport', 'P Report Ack'))
  // In name order, and none of the crate's own among them: a registry
  // declares no message type before a caller creates one.
  assert.deepEqual([...new fix.FixRegistry().msgtypes()], [])
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
  assert.equal(registry.definition('components', 'D').fix.msgtype, 'X')
  assert.throws(() => new fix.MsgType())
  assert.throws(() => registry.createDefinition('components', message('AnotherOrder', 'D')), /shared/)
  // One message-code namespace: a second message declaring a held code under
  // another name is a second message reached by its name, and the bare code
  // answers the message tag 35's code set names - none here, so the first
  // in name order - whichever arrived first.
  const second = registry.clone()
  second.createDefinition('components', message('AnotherOrder', 'D'))
  assert.equal(second.msgtype('D').name, 'AnotherOrder')
  assert.equal(second.msgtype('newordersingle').asStr(), 'D')
  assert.equal(second.msgtype('anotherorder').asStr(), 'D')
  assert.equal([...second.msgtypes()].length, 4)
  const held = catalog().msgtype('D')
  assert.equal(held.getGroupByTag(453).name, 'Parties')
  assert.equal(held.getGroupByTag(999), null)
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
  assert.equal(value.byPath('Parties[0].PartyID').asJs(), 'ONE')
  assert.equal(value.byPath('Parties[1].PartyID').asJs(), 'TWO')
  assert.deepEqual(value.anomalies(), [])
  assert.match(value.intoBytes(124).toString(), /453=2\|448=ONE\|448=TWO/)
})

test('identifier declarations replace whole on merge and reload in final member order', () => {
  const client = tagged('clordid', 11)
  const order = tagged('orderid', 37)
  const registry = fix.FixRegistry.fromFields([client, order])
  client.fix.fieldRef = 'clordid'
  order.fix.fieldRef = 'orderid'
  const initial = message('order', 'D', [client, order])
  initial.fix.identifiers = ['clordid']
  registry.createDefinition('components', initial)
  const incoming = message('order', 'D', [order, client])
  incoming.fix.identifiers = ['orderid']
  assert.equal(registry.addDefinition('components', incoming), false)
  assert.deepEqual(registry.definition('components', 'order').fix.identifiers, ['orderid'])
  incoming.fix.identifiers = ['37', '11']
  assert.deepEqual(incoming.fix.identifiers, ['orderid', 'clordid'])
  registry.addDefinition('components', incoming)
  assert.deepEqual(registry.definition('components', 'order').fix.identifiers, ['clordid', 'orderid'])
  const restored = fix.FixRegistry.fromJson(registry.intoJson())
  assert.ok(restored.equals(registry))
  assert.deepEqual(restored.definition('components', 'order').fix.identifiers, ['clordid', 'orderid'])
  const malformed = incoming.clone()
  malformed.set('fix:identifiers', 'clordid,,orderid')
  const before = registry.intoJson()
  assert.throws(() => registry.addDefinition('components', malformed), /fix:identifiers/)
  assert.equal(registry.intoJson(), before)
})

test('compiled identifier selection returns independent declarations and native BigInt scalars', () => {
  const client = tagged('clordid', 11)
  const order = tagged('orderid', 37)
  const numeric = tagged('numericid', 9001, 'int64')
  const declaration = message('order', 'D', [client, order, numeric])
  declaration.fix.identifiers = ['9001', '37', '11']
  const registry = fix.FixRegistry.fromFields([client, order, numeric])
  registry.createDefinition('components', declaration)
  const registered = registry.definition('components', 'order')
  const row = fields.struct('row', [tagged('venueorder', 37), client, numeric], { nullable: false })
  const numericValue = 9007199254740993n
  const value = new fix.FixMsg(row, ['O-1', 'C-1', numericValue], registry)
  const singleton = registry.msgtype('D')
  const selected = singleton.identifierValues(value)
  assert.deepEqual(selected.map(([field, scalar]) => [field.name, scalar.asJs()]), [
    ['clordid', 'C-1'], ['orderid', 'O-1'], ['numericid', numericValue],
  ])
  assert.ok(selected[0][0].equals(declaration.fieldAt(0)))
  assert.ok(selected[0][1] instanceof Scalar)
  assert.ok(selected[0][1].equals(value.byName('clordid')))
  assert.ok(selected[2][1].equals(value.byName('numericid')))
  assert.equal(typeof selected[2][1].asJs(), 'bigint')
  selected[0][0].setName('changed')
  selected[0][0].fix.names = ['ChangedAlias']
  assert.equal(singleton.identifierValues(value)[0][0].name, 'clordid')
  assert.deepEqual(singleton.identifierValues(value)[0][0].fix.names, [])
  assert.ok(registry.definition('components', 'order').equals(registered))
  const absent = new fix.FixMsg(row, [null, 'C-1', null], registry)
  assert.deepEqual(singleton.identifierValues(absent).map(([field, scalar]) => [field.name, scalar.asJs()]), [['clordid', 'C-1']])
  assert.throws(() => singleton.identifierValues('not a message'))
})

test('binary identifiers stay native and name nothing when they will not spell text', () => {
  const code = tagged('msgtype', 35)
  const identifier = tagged('customid', 9001, 'binary')
  const declaration = message('custom', 'Z9', [code, identifier])
  declaration.fix.identifiers = ['customid']
  const registry = fix.FixRegistry.fromFields([code, identifier])
  registry.createDefinition('components', declaration)
  const raw = Buffer.from([0x41, 0xff])
  const value = new fix.FixMsg(declaration, ['Z9', raw], registry)
  const before = value.clone()
  const selected = registry.msgtype('Z9').identifierValues(value)
  assert.equal(selected.length, 1)
  assert.equal(selected[0][0].name, 'customid')
  assert.ok(selected[0][1] instanceof Scalar)
  assert.ok(selected[0][1].equals(value.byTag(9001)))
  assert.ok(selected[0][1].asJs() instanceof Uint8Array)
  assert.deepEqual(Buffer.from(selected[0][1].asJs()), raw)
  // An identifier whose value will not spell text names nothing, so it is
  // left out of the map rather than refusing the message around it.
  const codec = new fix.FixCodec(registry)
  const enriched = codec.enrichMessage(value)
  assert.ok(enriched.byTag(9001).equals(value.byTag(9001)))
  assert.ok(value.equals(before))
})

test('compiled identifiers never assign one ambiguous tag to another direct member', () => {
  const left = tagged('leftid', 9001)
  const right = tagged('rightid', 9001)
  const declaration = message('paired', 'PAIR', [left, right])
  declaration.fix.identifiers = ['rightid', 'leftid']
  const registry = new fix.FixRegistry()
  registry.createDefinition('components', declaration)
  const singleton = registry.msgtype('PAIR')
  const exact = fields.struct('row', [right, left], { nullable: false })
  const value = new fix.FixMsg(exact, ['R-1', 'L-1'], registry)
  assert.deepEqual(singleton.identifierValues(value).map(([field, scalar]) => [field.name, scalar.asJs()]), [
    ['leftid', 'L-1'], ['rightid', 'R-1'],
  ])
  const renamed = fields.struct('row', [tagged('venueleft', 9001), tagged('venueright', 9001)], { nullable: false })
  assert.deepEqual(singleton.identifierValues(new fix.FixMsg(renamed, ['L-1', 'R-1'], registry)), [])
})

test('a builtin altids group reference reloads over the persisted builtin definition', (t) => {
  const registry = new fix.FixRegistry()
  const mapping = registry.groupByTag(65020)
  mapping.fix.group = 'altids'
  registry.createDefinition('components', message('identified', 'ID', [mapping]))
  // A dump states the crate's own group, and reading one back takes the held
  // declaration over the document's.
  assert.deepEqual(
    registry.toJSON().groups.map((group) => group.name),
    ['altids'],
  )
  assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-node-altids-'))
  t.after(() => fs.rmSync(folder, { recursive: true, force: true }))
  registry.writeInto(folder)
  assert.equal(fs.existsSync(path.join(folder, 'groups', 'altids.json')), true)
  assert.ok(fix.FixRegistry.fromHandle(folder).equals(registry))
})

test('a JSON document is one unknown message through every door', () => {
  const registry = new fix.FixRegistry()
  const codec = new fix.FixCodec(registry)
  // A body that is JSON says nothing FIX can read, and being unable to read
  // a body is not an error in the codec: the row is one message named
  // `unknown` with no entries, from the line door and the text-line door
  // alike, and the cursor fuses behind it.
  const body = Buffer.from('{"a":1}')
  const schema = fix.schema(registry)
  for (const cursor of [codec.parseLine(body), codec.parseTextLine(new TextLine(17, body))]) {
    assert.ok(cursor instanceof fix.FixMessages)
    assert.equal(cursor[Symbol.iterator](), cursor)
    const message = cursor.next().value
    assert.equal(message.field.name, 'unknown')
    assert.deepEqual(message.arrivals(), [])
    assert.equal(message.intoBytes(124).length, 0)
    assert.equal(cursor.next().done, true)
    assert.equal(cursor.next().done, true)
    assert.equal(message.intoRow(schema).asJs().length, schema.fieldLen)
    assert.equal(typeof message.stableHash(), 'bigint')
  }
})

for (const [property, key, value] of [['counter', 'counter', 453], ['component', 'component', 'Party'], ['fieldRef', 'field', 'PartyID'], ['group', 'group', 'Parties'], ['msgtype', 'msgtype', 'P Report Ack']]) {
  test(`typed FIX ${property} shares native metadata and rejects invalid writes atomically`, () => {
    const field = Field.from('Definition: utf8')
    field.fix[property] = value
    const expected = ['component', 'fieldRef', 'group'].includes(property) ? value.toLowerCase() : value
    assert.equal(field.fix[property], expected)
    assert.equal(field.get(`fix:${key}`), String(expected))
    const before = JSON.stringify(field)
    // A counter is a positive tag: zero marks an unresolved arrival and is
    // refused like any other nonpositive number (`rust/tests/fix/zero_entries.rs`).
    for (const invalid of property === 'counter' ? [0, -1, 1.5, 2 ** 31] : ['', '\u0000']) {
      assert.throws(() => { field.fix[property] = invalid })
      assert.equal(JSON.stringify(field), before)
    }
    assert.throws(() => field.http[property], /http/)
    assert.throws(() => { field.http[property] = value }, /http/)
    assert.equal(field.fix.delete(key), true)
    assert.equal(field.fix[property], null)
  })
}
