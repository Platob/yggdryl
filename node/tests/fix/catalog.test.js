'use strict'

// Components, groups and messages through the one set of field doors: a
// definition is filed by the shape it has, reached by name, counter or path,
// and mutated by `insert`, `update` and `remove` like any field.
//
// Every rule is the core's; what these check is the crossing - the shape a
// JavaScript caller builds, the refusals that arrive as the native ones, and
// the message definition a registry answers as a copy of its own.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { DataType, Field, Scalar, TextLine, fields, fix } = require('yggdryl')

// A codec that reads every message type. The corpora below are captures and
// bridge rows, which state no type and which `DEFAULT_REFUSED_MSGTYPES` drop;
// a case about the refusals says so for itself.
function reading(registry, options) {
  return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
}

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

/** A registry holding one order message over one repeating group. */
function catalog() {
  const registry = fix.FixRegistry.fromFields([tagged('NoPartyIDs', 453, 'int32'), tagged('PartyID', 448)])
  const member = registry.field(448)
  member.fix.fieldRef = 'PartyID'
  const component = fields.struct('Party', [member], { nullable: false })
  registry.insert(component)
  const group = fields.list('Parties', component)
  group.fix.counter = 453
  group.fix.component = 'Party'
  registry.insert(group)
  const occurrence = registry.fieldByName('Parties')
  occurrence.fix.group = 'Parties'
  const counter = registry.field(453)
  counter.fix.fieldRef = 'NoPartyIDs'
  registry.insert(message('NewOrderSingle', 'D', [counter, occurrence]))
  return registry
}

test('a definition is filed by the shape it has and reached through the field doors', () => {
  const registry = catalog()

  // A Struct is a component, a List of Structs a group, and a Struct
  // carrying `FIX:msgtype` a message; each is reached by its name, and a
  // group by the counter it opens.
  assert.equal(registry.fieldByName('Party').dtype.id, 'struct')
  assert.equal(registry.fieldByName('Parties').dtype.id, 'list')
  assert.equal(registry.fieldByCounter(453).name, 'Parties')
  assert.ok(registry.getFieldByCounter(453).equals(registry.fieldByName('Parties')))
  assert.equal(registry.getFieldByCounter(999), null)
  assert.throws(() => registry.fieldByCounter(999), /got nothing/)
  // The counter itself is the scalar the tag holds: the group is a
  // definition of its own beside it.
  assert.equal(registry.fieldByTag(453).name, 'NoPartyIDs')
  assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
  // A path walks through the group into its member.
  assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
  assert.equal(registry.fieldByPath('NewOrderSingle.Parties.PartyID').fix.tag, 448)
  // A counter is a tag: an exact number, never text.
  assert.throws(() => registry.fieldByCounter(1.5), /tag must be a signed 32-bit integer/)
  assert.throws(() => registry.getFieldByCounter('453'), /into rust type `f64`/)
  // A named definition carries a derived tag of its own, from the block
  // above every published tag.
  const parties = registry.fieldByName('Parties').fix.tag
  assert.ok(parties >= 100_000 && parties < 1_100_000, `derived definition tag, got ${parties}`)
  // And every definition is in the walk, behind the scalars.
  const names = [...registry].map((field) => field.name)
  for (const name of ['Party', 'Parties', 'NewOrderSingle']) assert.ok(names.includes(name), name)
  assert.equal(names.length, registry.size)
})

test('insert, update and remove carry a definition as they carry a field', () => {
  const registry = catalog()
  const before = registry.intoJson()

  // A definition another one references does not leave.
  for (const name of ['PartyID', 'Party', 'Parties']) {
    assert.equal(registry.remove(name), null, name)
    assert.equal(registry.intoJson(), before, name)
  }

  // `update` merges into the definition of the same folded name, and the
  // stored spelling stands.
  const changed = registry.fieldByName('Party')
  changed.setName('party')
  changed.fix.description = 'Reviewed'
  registry.update(changed)
  assert.equal(registry.fieldByName('Party').name, 'Party')
  assert.equal(registry.fieldByName('Party').fix.description, 'Reviewed')

  // A component another definition references is the one that definition
  // holds: restating its members is refused, naming the reference it would
  // break, and the dictionary is left exactly as it was.
  assert.equal(registry.insert(tagged('PartyNote', 9002)), null)
  const note = registry.field(9002)
  note.fix.fieldRef = 'PartyNote'
  const extended = registry.fieldByName('Party')
  extended.setDtype(DataType.fromFields([...members(extended), note]))
  const settled = registry.intoJson()
  assert.throws(() => registry.update(extended), /Parties/)
  assert.equal(registry.intoJson(), settled)
  assert.throws(() => registry.insert(extended), /Parties/)
  assert.equal(registry.intoJson(), settled)
  assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))

  // A definition nothing references leaves, and leaving again is null.
  const removed = registry.remove('NewOrderSingle')
  assert.equal(removed.name, 'NewOrderSingle')
  assert.equal(registry.remove('NewOrderSingle'), null)
  assert.equal(registry.getFieldByName('NewOrderSingle'), null)
  assert.equal(registry.getMsgtype('D'), null)
  assert.equal(registry.remove('Parties').name, 'Parties')
  assert.equal(registry.getFieldByCounter(453), null)
})

test('the complete native catalog survives a snapshot and a store', (t) => {
  const registry = catalog()
  const coded = registry.field(448)
  coded.set('FIX:codes', '[{"value":"B","name":"Broker"}]')
  registry.update(coded)
  assert.match(registry.fieldByPath('NewOrderSingle.Parties.PartyID').get('FIX:codes'), /Broker/)
  const vendor = tagged('Vendor', 9001)
  vendor.fix.branches = ['venue']
  registry.insert(vendor)

  // Three categories and nothing else: a dictionary's membership is metadata
  // on the field it contributed to, so it travels inside `fields`.
  const document = registry.toJSON()
  assert.deepEqual(Object.keys(document).sort(), ['components', 'fields', 'groups'])
  assert.equal(document.fields.find((value) => value.name === 'Vendor').metadata['FIX:branches'], 'venue')
  assert.ok(document.components.some((value) => value.name === 'Party'))
  assert.ok(document.groups.some((value) => value.name === 'Parties'))

  const declared = fix.FixRegistry.fromJson(JSON.stringify(document))
  assert.deepEqual(declared.fieldByTag(9001).fix.branches, ['venue'])
  assert.deepEqual(declared.dialects(), ['venue'])
  for (const copy of [declared.clone(), fix.FixRegistry.fromJson(declared.intoJson())]) {
    assert.ok(copy.equals(declared))
    assert.equal(copy.stableHash(), declared.stableHash())
    assert.equal(copy.fieldByCounter(453).name, 'Parties')
  }

  // A store writes the whole dictionary, the crate's own definitions
  // included, and reads it back as itself.
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-node-catalog-'))
  t.after(() => fs.rmSync(folder, { recursive: true, force: true }))
  registry.writeInto(folder)
  assert.deepEqual(fs.readdirSync(folder).sort(), ['components', 'fields', 'groups'])
  // A definition is one document under its category, and the crate's own
  // are written like every other: the fixed row is `components/fixmsg.json`
  // and its two Map groups are two documents.
  const documents = (category) => fs.readdirSync(path.join(folder, category)).sort()
  assert.ok(documents('components').includes('fixmsg.json'))
  assert.ok(documents('components').some((name) => name.toLowerCase() === 'party.json'))
  assert.deepEqual(
    documents('groups').map((name) => name.toLowerCase()).sort(),
    ['identifiers.json', 'metadata.json', 'parties.json'],
  )
  assert.ok(documents('fields').every((name) => /^\d{9}\.json$/.test(name)))
  assert.ok(fix.FixRegistry.fromHandle(folder).equals(registry))

  // A change to any category changes the value.
  const changed = registry.clone()
  const definition = changed.fieldByName('NewOrderSingle')
  definition.fix.description = 'Different'
  changed.update(definition)
  assert.equal(changed.equals(registry), false)
  assert.notEqual(changed.stableHash(), registry.stableHash())
})

test('a message definition is a copy of the registry-owned one', () => {
  const registry = new fix.FixRegistry()
  registry.insert(message('D', 'X'))
  registry.insert(message('NewOrderSingle', 'D'))
  registry.insert(message('BridgeReport', 'P Report Ack'))

  // A code, a canonical name and tag 35's alias all reach it.
  assert.equal(registry.msgtype('D').name, 'NewOrderSingle')
  assert.equal(registry.msgtype('bridgereport').asStr(), 'P Report Ack')
  assert.equal(registry.getMsgtype('p report ack'), null)
  assert.equal(registry.getMsgtype('nothing'), null)
  assert.throws(() => registry.msgtype('nothing'), /nothing/)
  // A registry declares no message before a caller builds one.
  assert.equal(new fix.FixRegistry().getMsgtype('D'), null)

  const held = registry.msgtype('D')
  assert.equal(held.asStr(), 'D')
  assert.equal(held.compare(registry.msgtype('D')), 0)
  assert.equal(held.equals(registry.msgtype('D')), true)
  assert.equal(held.equals(registry.msgtype('bridgereport')), false)
  assert.equal(typeof held.stableHash(), 'bigint')
  assert.equal(held.stableHash(), held.clone().stableHash())
  assert.equal(String(held), 'D')
  assert.equal(held.toJSON().name, 'NewOrderSingle')
  assert.throws(() => new fix.MsgType())

  // It is a copy: the definition it projects is independently mutable, and
  // a later mutation of the dictionary leaves the answered value alone.
  const projected = held.asField()
  projected.setName('Changed')
  projected.fix.msgtype = 'Changed'
  assert.equal(held.name, 'NewOrderSingle')
  assert.equal(held.asStr(), 'D')
  assert.equal(registry.fieldByName('NewOrderSingle').fix.msgtype, 'D')
  registry.remove('NewOrderSingle')
  assert.equal(held.asStr(), 'D', 'the copy outlives the definition')
  // The bare code now reaches the other message the dictionary holds under
  // that name - the one whose own code is `X`.
  assert.equal(registry.msgtype('D').name, 'D')
  assert.equal(registry.msgtype('D').asStr(), 'X')
  assert.equal(registry.getMsgtype('newordersingle'), null)
})

test('a message reaches its group by the counter that opens it', () => {
  const registry = catalog()
  const held = registry.msgtype('D')
  assert.equal(held.getGroupByTag(453).name, 'Parties')
  assert.equal(held.getGroupByTag(999), null)
  assert.throws(() => held.getGroupByTag(1.5), /tag must be a signed 32-bit integer/)

  // And a parse under that registry fills the group beside its counter.
  const codec = reading(registry)
  const values = [...codec.parseLine(Buffer.from('35=D|453=2|448=ONE|448=TWO|'))]
  assert.equal(values.length, 1)
  const value = values[0]
  assert.equal(value.byTag(453).asJs(), 2)
  assert.equal(value.byPath('Parties[0].PartyID').asJs(), 'ONE')
  assert.equal(value.byPath('Parties[1].PartyID').asJs(), 'TWO')
  assert.match(value.intoText('|'), /453=2\|448=ONE\|448=TWO/)
  // The group states its count once, on the entry that heads it.
  const parties = value.entries().find((entry) => entry.tag === 453)
  assert.equal(parties.value, '2')
  assert.equal(parties.entries.length, 2)
  assert.deepEqual(parties.entries.map((entry) => entry.value), [null, null])
})

test('identifier declarations replace whole on update and reload in member order', () => {
  const client = tagged('clordid', 11)
  const order = tagged('orderid', 37)
  const registry = fix.FixRegistry.fromFields([client, order])
  client.fix.fieldRef = 'clordid'
  order.fix.fieldRef = 'orderid'
  const initial = message('order', 'D', [client, order])
  initial.fix.identifiers = ['clordid']
  registry.insert(initial)
  assert.deepEqual(registry.fieldByName('order').fix.identifiers, ['clordid'])

  const incoming = message('order', 'D', [order, client])
  incoming.fix.identifiers = ['37', '11']
  assert.deepEqual(incoming.fix.identifiers, ['orderid', 'clordid'])
  registry.update(incoming)
  assert.deepEqual(registry.fieldByName('order').fix.identifiers, ['orderid', 'clordid'])
  const restored = fix.FixRegistry.fromJson(registry.intoJson())
  assert.ok(restored.equals(registry))
  assert.deepEqual(restored.fieldByName('order').fix.identifiers, ['orderid', 'clordid'])

  const malformed = incoming.clone()
  malformed.set('FIX:identifiers', 'clordid,,orderid')
  const before = registry.intoJson()
  assert.throws(() => registry.update(malformed), /FIX:identifiers/)
  assert.equal(registry.intoJson(), before)
})

test('compiled identifier selection returns independent declarations and native scalars', () => {
  const client = tagged('clordid', 11)
  const order = tagged('orderid', 37)
  const numeric = tagged('numericid', 9001, 'int64')
  const declaration = message('order', 'D', [client, order, numeric])
  declaration.fix.identifiers = ['9001', '37', '11']
  const registry = fix.FixRegistry.fromFields([client, order, numeric])
  registry.insert(declaration)
  const row = fields.struct('row', [tagged('venueorder', 37), client, numeric], { nullable: false })
  const numericValue = 9007199254740993n
  const value = new fix.FixMsg(row, ['O-1', 'C-1', numericValue], registry)
  const singleton = registry.msgtype('D')
  const selected = singleton.identifierValues(value)
  assert.deepEqual(selected.map(([field, scalar]) => [field.name, scalar.asJs()]), [
    ['clordid', 'C-1'], ['orderid', 'O-1'], ['numericid', numericValue],
  ])
  assert.ok(selected[0][1] instanceof Scalar)
  assert.ok(selected[0][1].equals(value.byName('clordid')))
  assert.equal(typeof selected[2][1].asJs(), 'bigint')
  // The Fields are independent copies.
  selected[0][0].setName('changed')
  assert.equal(singleton.identifierValues(value)[0][0].name, 'clordid')
  const absent = new fix.FixMsg(row, [null, 'C-1', null], registry)
  assert.deepEqual(singleton.identifierValues(absent).map(([field]) => field.name), ['clordid'])
  assert.throws(() => singleton.identifierValues('not a message'))
})

test('a JSON document is one unknown message through every door', () => {
  const registry = new fix.FixRegistry()
  const codec = reading(registry)
  // A body that is JSON says nothing FIX can read, and being unable to read
  // a body is not an error in the codec: the row is one message named
  // `unknown` with no content, from the line door and the text-line door
  // alike, and the cursor fuses behind it.
  const body = Buffer.from('{"a":1}')
  const schema = fix.schema(registry)
  for (const cursor of [codec.parseLine(body), codec.parseTextLine(new TextLine(17, body))]) {
    assert.ok(cursor instanceof fix.FixMessages)
    assert.equal(cursor[Symbol.iterator](), cursor)
    const held = cursor.next().value
    assert.equal(held.field.name, 'unknown')
    assert.deepEqual(held.entries(), [])
    assert.equal(held.size, 0)
    assert.equal(cursor.next().done, true)
    assert.equal(cursor.next().done, true)
    assert.equal(held.intoRow(schema).asJs().length, schema.fieldLen)
    assert.equal(typeof held.stableHash(), 'bigint')
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
    // A counter is a positive tag: zero marks an unresolved entry and is
    // refused like any other nonpositive number.
    const refused = property === 'counter' ? [0, -1, 1.5, 2 ** 31] : ['', ' ']
    for (const invalid of refused) {
      assert.throws(() => { field.fix[property] = invalid })
      assert.equal(JSON.stringify(field), before)
    }
    assert.throws(() => field.http[property], /http/)
    assert.throws(() => { field.http[property] = value }, /http/)
    assert.equal(field.fix.delete(key), true)
    assert.equal(field.fix[property], null)
  })
}
