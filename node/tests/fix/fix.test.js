'use strict'

// The FIX boundary: the typed vocabulary on the protocol view, the registry,
// and the message. Every answer here is the core's; what these check is the
// crossing - the key coercion, the tag width, the error class each refusal
// arrives as, the storage locations a JavaScript caller names, and the
// language protocols the loader wires over the native halves.
//
// An identifier crosses as a number - the digest the core derives from a
// tag and a name - so there is no class for it, and a dictionary's membership
// is a list of names on the field it contributed to; every refusal is the
// native one.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { DataType, Field, IOBase, MimeType, Scalar, TextLine, Url, fields, fix, hashing } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')


// A codec that reads every message type. The corpora below are captures, and
// a capture holds the session traffic and the bridge rows stating no type
// that `DEFAULT_REFUSED_MSGTYPES` drop; a case about the refusals says so for
// itself.
function reading(registry, options) {
  return new fix.FixCodec(registry, { excludeMsgtypes: [], ...(options ?? {}) })
}

// The crate's own definitions, which every registry holds from construction
// beside the seeded SendingTime (52) and TransactTime (60) clocks. A
// definition is filed by the shape it has: the columns are scalar fields,
// the `identifiers` and `metadata` Maps are groups, and the parent/source
// UUID lists are registered scalar columns.
const CRATE = fix.crateFields()
// Which category a definition lands in is the core's answer, not a shape a
// test guesses: a snapshot states the three, so the fields it lists are the
// fields, whatever datatype each carries.
const CRATE_SCALAR_NAMES = new Set(new fix.FixRegistry().toJSON().fields.map((field) => field.name))
const CRATE_SCALARS = CRATE.filter((field) => CRATE_SCALAR_NAMES.has(field.name))
const CRATE_GROUPS = CRATE.filter((field) => !CRATE_SCALAR_NAMES.has(field.name))
// The crate's scalar tags, in the registry's own order.
const CRATE_TAGS = CRATE_SCALARS.map((field) => field.fix.tag)
// What a new registry holds before anything is inserted: every crate
// definition and the two seeded clocks.
const SEEDED = 2 + CRATE.length
// The scalar fields the committed dictionary stores.
const STORED = 6241
// The named code sets it stores beside them, one per vocabulary however many
// fields read by it.
const CODESETS = 736

/**
 * The scalar fields a registry holds, and the definitions behind them.
 *
 * The walk answers the scalars first, so the snapshot's own count is where
 * the definitions start.
 */
function scalars(registry) {
  return [...registry].slice(0, registry.toJSON().fields.length)
}

function definitionsOf(registry) {
  return [...registry].slice(registry.toJSON().fields.length)
}
// The one intake clock the Rust suites read undated bytes under
// (`fixed_codec` in `rust/tests/fix.rs`): 2024-01-02T10:15:30Z. Without it an
// undated message reads UTC now, which is deliberately not deterministic.
const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)

function fixedCodec(registry, options = {}) {
  return reading(registry, { ...options, defaultSendingTime: SENDING })
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
}

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

function fixField(name, dtype, tag, { branches, tags, names, description } = {}) {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.tag = tag
  if (branches) field.fix.branches = branches
  if (tags) field.fix.tags = tags
  if (names) field.fix.names = names
  if (description !== undefined) field.fix.description = description
  return field
}

test('the protocol view carries the typed fix vocabulary', () => {
  const field = Field.from('OrderQty: decimal128(20, 8)')
  field.fix.tag = 38
  field.fix.tags = [1088]
  field.fix.names = ['Qty', 'Quantity']
  field.fix.msgcat = 'ORDR'
  field.fix.description = 'Quantity ordered.'

  assert.equal(field.fix.tag, 38)
  assert.deepEqual(field.fix.tags, [1088])
  assert.deepEqual(field.fix.names, ['Qty', 'Quantity'])
  assert.equal(field.fix.msgcat, 'ORDR')
  assert.equal(field.fix.description, 'Quantity ordered.')
  // Ordinary namespaced text, in the one metadata map: a list is the
  // compact JSON array it is.
  assert.equal(field.get('FIX:names'), '["Qty","Quantity"]')
  assert.equal(field.get('FIX:tags'), '[1088]')
  assert.equal(field.fix.get('tag'), '38')
  // Four: a description is a fact about the column rather than a
  // FIX fact, so it lives on the generic key every catalog reads.
  assert.equal(field.get('description'), 'Quantity ordered.')
  assert.equal(field.has('FIX:description'), false)
  assert.equal(field.fix.size, 4)

  const before = field.toJSON()
  assert.throws(() => {
    field.fix.msgcat = 'order'
  })
  assert.deepEqual(field.toJSON(), before, 'a refused category is atomic')
  field.fix.msgcat = null
  assert.equal(field.fix.msgcat, null)

  // An empty array removes a list property; `delete` removes any of them.
  field.fix.tags = []
  assert.deepEqual(field.fix.tags, [])
  assert.equal(field.fix.has('tags'), false)
  field.fix.names = []
  assert.deepEqual(field.fix.names, [])
  assert.equal(field.fix.delete('tag'), true)
  assert.equal(field.fix.tag, null)

  const absent = Field.from('Symbol: utf8')
  assert.equal(absent.fix.tag, null)
  assert.deepEqual(absent.fix.tags, [])
  assert.deepEqual(absent.fix.names, [])
  assert.equal(absent.fix.description, null)
})

test('the typed vocabulary answers only on the fix view', () => {
  const field = Field.from('Symbol: utf8')
  field.fix.tag = 55

  for (const [view, scheme] of [
    [field.http, 'http'],
    [field.iceberg, 'iceberg'],
    [field.protocol('parquet'), 'parquet'],
  ]) {
    assert.throws(() => view.tag, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.tags, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.names, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.identifiers, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.description, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.branches, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.hasBranch('cme'), { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.id, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.directions, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.msgcat, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.codeset, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.codeset = 'sidecodeset'
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.tag = 55
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.directions = [{ code: 'S', patterns: ['^TX '] }]
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.msgcat = 'ORDR'
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.derivation, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.derivation = 'orderqty - cumqty'
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.names = ['Ticker']
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.identifiers = []
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.branches = ['cme']
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.addBranch('cme'), { name: 'TypeError', message: new RegExp(scheme) })
  }
  // The Map-like surface still works on every view, this one included.
  assert.equal(field.protocol('fix').get('tag'), '55')
})

test('identifier declarations resolve aliases and decimal tags into direct member order', () => {
  const client = fixField('clordid', 'utf8', 11, { names: ['ClientOrder'] })
  const order = fixField('orderid', 'utf8', 37)
  const declaration = fields.struct('order', [client, order], { nullable: false })
  const view = declaration.fix
  assert.deepEqual(view.identifiers, [])
  view.identifiers = ['37', 'ClientOrder']
  assert.deepEqual(view.identifiers, ['clordid', 'orderid'])
  assert.equal(declaration.get('FIX:identifiers'), 'clordid,orderid')
  assert.deepEqual(Field.fromJSON(declaration.toJSON()).fix.identifiers, ['clordid', 'orderid'])
  view.identifiers = ['ORDERID']
  assert.deepEqual(view.identifiers, ['orderid'])
  view.identifiers = []
  assert.deepEqual(view.identifiers, [])
  assert.equal(declaration.has('FIX:identifiers'), false)
})

test('identifier declaration refusals leave the entire field unchanged', () => {
  const client = fixField('clordid', 'utf8', 11)
  const nested = fields.struct('nested', [Field.from('child: utf8')])
  const declaration = fields.struct('order', [client, nested], { nullable: false })
  declaration.fix.identifiers = ['clordid']
  const before = declaration.toJSON()
  for (const invalid of [[''], ['absent'], ['nested'], ['nested.child'], ['clordid,orderid'], ['clordid', '11']]) {
    assert.throws(() => { declaration.fix.identifiers = invalid }, /order.fix:identifiers/)
    assert.deepEqual(declaration.toJSON(), before)
  }
  for (const invalid of [
    'clordid', 11, null, ['clordid', 11], new Set(['clordid']),
    (function* () { yield 'clordid' })(), [, 'clordid'],
  ]) {
    assert.throws(() => { declaration.fix.identifiers = invalid })
    assert.deepEqual(declaration.toJSON(), before)
  }
})

test('a derivation crosses as canonical text', () => {
  // One term over the message's fields, stored as its canonical spelling;
  // null removes it, and a text that is not a term throws.
  const field = new Field('leavesqty', 'float64')
  field.fix.tag = 151
  assert.equal(field.fix.derivation, null)

  field.fix.derivation = 'orderqty-cumqty'
  assert.equal(field.fix.derivation, 'orderqty - cumqty')
  assert.equal(field.get('FIX:derivation'), 'orderqty - cumqty')

  assert.throws(() => {
    field.fix.derivation = 'orderqty -'
  })
  assert.equal(field.fix.derivation, 'orderqty - cumqty')

  // An edited derivation is what the reader fills by.
  const registry = fix.FixRegistry.fromHandle(SEED)
  const leaves = registry.getFieldByTag(151)
  leaves.fix.derivation = "case when msgtype in ('8', '9') then orderqty * 2 end"
  registry.update(leaves)
  const codec = fixedCodec(registry)
  const held = codec.parseLine(Buffer.from('8=FIX.4.4|35=8|37=A|38=100|14=0|10=0|')).next().value
  // An exact column renders as its own text, at the one scale this crate
  // keeps a number at.
  assert.equal(held.byTag(151).toJSON(), '200.000000000000000000')

  field.fix.derivation = null
  assert.equal(field.fix.derivation, null)
  assert.equal(field.has('FIX:derivation'), false)
})

test('direction rules cross as a typed list', () => {
  const field = new Field('MsgDirection', 'utf8')
  field.fix.tag = 385
  assert.deepEqual(field.fix.directions, [])

  // One record per code of the set, the patterns decoded, on tag 385.
  const rules = [
    { code: 'S', patterns: ['(?i)^TX\\b'] },
    { code: 'R', patterns: ['(?i)^RX\\b'] },
  ]
  field.fix.directions = rules
  assert.deepEqual(field.fix.directions, rules)
  // The stored text is the canonical document, backslashes escaped.
  assert.equal(
    field.get('FIX:directions'),
    '[{"code":"S","patterns":["(?i)^TX\\\\b"]},' +
      '{"code":"R","patterns":["(?i)^RX\\\\b"]}]',
  )
  assert.deepEqual(JSON.parse(field.get('FIX:directions')), rules)

  // A codec compiles the rules of the dictionary it is built over, once,
  // and the line door fills tag 385 from them; the verb table no longer
  // applies under a stated table.
  const registry = new fix.FixRegistry()
  registry.insert(field)
  const codec = reading(registry)
  const read = (line) => codec.parseLine(Buffer.from(line)).next().value
  assert.equal(read('TX 8=FIX.4.4|35=D|10=0|').byTag(385).asJs(), 'S')
  assert.equal(read('RX 8=FIX.4.4|35=D|10=0|').byTag(385).asJs(), 'R')
  assert.equal(read('sending >> 8=FIX.4.4|35=D|10=0|').getByTag(385), null)

  // A pattern the regex crate refuses is refused whole, the field unchanged.
  assert.throws(() => {
    field.fix.directions = [{ code: 'S', patterns: ['('] }]
  }, /valid byte regex/)
  assert.deepEqual(field.fix.directions, rules)

  // An empty array removes the property.
  field.fix.directions = []
  assert.deepEqual(field.fix.directions, [])
  assert.equal(field.has('FIX:directions'), false)
  assert.deepEqual(new Field('MsgDirection', 'utf8').fix.directions, [])
})

test('a field names the code set it reads by, and the dictionary holds it', () => {
  const registry = new fix.FixRegistry()
  assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])

  // The set is stated first: a dictionary refuses a field naming a
  // vocabulary nothing states, so the members exist before a field points
  // at them.
  registry.setCodeset('sidecodeset', [
    { value: '1', name: 'Buy' },
    { value: '2', name: 'Sell', aliases: ['Sold'], doc: 'Sell side', group: 'Outright' },
  ])
  assert.deepEqual(registry.codesetNames(), ['msgcatcodeset', 'sidecodeset'])

  const side = fixField('Side', 'utf8', 54)
  assert.equal(side.fix.codeset, null)
  side.fix.codeset = 'sidecodeset'
  assert.equal(side.fix.codeset, 'sidecodeset')
  // The name is ordinary namespaced text, in the one metadata map: a field
  // carries the name and never a copy of the members.
  assert.equal(side.get('FIX:codeset'), 'sidecodeset')
  registry.insert(side)

  // One vocabulary, read through the set the field names.
  const set = registry.codesetOf(registry.fieldByTag(54))
  assert.equal(set.name, 'sidecodeset')
  assert.deepEqual(set.codes, [
    { value: '1', name: 'Buy' },
    { value: '2', name: 'Sell', aliases: ['Sold'], doc: 'Sell side', group: 'Outright' },
  ])
  assert.deepEqual(registry.codeset('sidecodeset'), set)
  // The name is folded, so whichever spelling a caller states reaches it.
  assert.deepEqual(registry.getCodeset('SideCodeSet'), set)
  assert.equal(registry.getCodeset('absent'), null)
  assert.throws(() => registry.codeset('absent'), /expected a codesets at "absent", got nothing/)
  // A field drawing on no set answers null rather than a refusal.
  assert.equal(registry.codesetOf(fixField('Symbol', 'utf8', 55)), null)

  // Every spelling of a code reaches its wire value - the value itself, the
  // symbolic name, an alias - and a value answers its name. What the set
  // does not answer to is null: a venue sends codes no dictionary lists.
  assert.equal(registry.codeValue('sidecodeset', '2'), '2')
  assert.equal(registry.codeValue('sidecodeset', 'Buy'), '1')
  assert.equal(registry.codeValue('sidecodeset', 'sold'), '2')
  assert.equal(registry.codeValue('sidecodeset', 'Neither'), null)
  assert.equal(registry.codeName('sidecodeset', '1'), 'Buy')
  assert.equal(registry.codeName('sidecodeset', '9'), null)

  // A set a held field still reads by may not be taken away, by removal or
  // by an empty statement; the field lets go first.
  assert.throws(() => registry.removeCodeset('sidecodeset'), /sidecodeset.*side/i)
  assert.throws(() => registry.setCodeset('sidecodeset', []), /sidecodeset.*side/i)
  const held = registry.fieldByTag(54)
  held.fix.codeset = null
  assert.equal(held.fix.codeset, null)
  assert.equal(held.has('FIX:codeset'), false)
  registry.insert(held)
  assert.deepEqual(registry.removeCodeset('sidecodeset'), set.codes)
  assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])
  assert.equal(registry.removeCodeset('sidecodeset'), null)

  // The committed dictionary is the same shape at scale: one set per
  // vocabulary, and a field states only which one it reads by.
  const shipped = seed()
  assert.equal(shipped.codesetNames().length, CODESETS)
  assert.equal(shipped.fieldByTag(54).fix.codeset, 'sidecodeset')
  assert.equal(shipped.codeValue('sidecodeset', 'Buy'), '1')
  assert.equal(shipped.codeName('sidecodeset', '2'), 'Sell')
  assert.equal(shipped.codesetOf(shipped.fieldByTag(54)).name, 'sidecodeset')
  for (const tag of [447, 525]) {
    const partySource = shipped.codesetOf(shipped.fieldByTag(tag))
    assert.equal(shipped.codeValue(partySource.name, 'proprietary/customcode'), 'D')
  }
})

test('a field naming a code set the dictionary does not hold is refused', () => {
  const stray = fixField('Side', 'utf8', 54)
  stray.fix.codeset = 'sidecodeset'

  // Every door a field arrives through holds the same invariant, and each
  // refusal names the set that is missing rather than the field.
  const refused = /expected a codesets at "sidecodeset", got nothing/
  assert.throws(() => fix.FixRegistry.fromFields([stray]), refused)
  const registry = new fix.FixRegistry()
  assert.throws(() => registry.insert(stray), refused)
  assert.equal(registry.getFieldByTag(54), null)
  assert.deepEqual(registry.codesetNames(), ['msgcatcodeset'])

  // With the set stated the same field arrives, and a held field sent to a
  // set nothing states is refused on update, the dictionary unchanged.
  registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }])
  registry.insert(stray)
  registry.insert(fixField('Symbol', 'utf8', 55))
  const settled = registry.intoJson()
  const moved = registry.fieldByTag(55)
  moved.fix.codeset = 'othercodeset'
  assert.throws(() => registry.update(moved), /expected a codesets at "othercodeset", got nothing/)
  assert.equal(registry.intoJson(), settled)
  assert.equal(registry.fieldByTag(55).fix.codeset, null)

  // A snapshot is read under the same rule: the vocabularies lead it, so a
  // document whose field names one it does not carry is not a dictionary.
  const document = registry.toJSON()
  document.fields.find((field) => field.metadata['FIX:tag'] === '54').metadata['FIX:codeset'] = 'othercodeset'
  assert.throws(
    () => fix.FixRegistry.fromJson(JSON.stringify(document)),
    /expected a codesets at "othercodeset", got nothing/,
  )
})

test('a code set merges by wire value, keeping what the dictionary held', () => {
  const registry = new fix.FixRegistry()

  // A set the dictionary does not hold arrives whole through the fold.
  registry.mergeCodeset('sidecodeset', [
    { value: '1', name: 'Buy', doc: 'Buy side' },
    { value: '2', name: 'Sell' },
  ])
  assert.deepEqual(registry.codeset('sidecodeset').codes, [
    { value: '1', name: 'Buy', doc: 'Buy side' },
    { value: '2', name: 'Sell' },
  ])

  // A venue's statement enriches what is held rather than replacing it: a
  // wire value already held keeps its name, its wording and its order, and
  // the spelling the venue declared becomes another alias; a value nothing
  // held joins the end.
  registry.mergeCodeset('sidecodeset', [
    { value: '2', name: 'Sold' },
    { value: '7', name: 'Undisclosed' },
  ])
  assert.deepEqual(registry.codeset('sidecodeset').codes, [
    { value: '1', name: 'Buy', doc: 'Buy side' },
    { value: '2', name: 'Sell', aliases: ['Sold'] },
    { value: '7', name: 'Undisclosed' },
  ])
  assert.equal(registry.codeValue('sidecodeset', 'Sold'), '2')
  assert.equal(registry.codeValue('sidecodeset', 'Sell'), '2')

  // A statement replaces, which is the other verb: what it does not carry
  // is gone.
  registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }])
  assert.deepEqual(registry.codeset('sidecodeset').codes, [{ value: '1', name: 'Buy' }])
  assert.equal(registry.codeValue('sidecodeset', 'Sold'), null)

  // A field keeps the set it already reads by, and a second statement of
  // that field's vocabulary folds into the held set: the members are the
  // dictionary's to fold, so nothing moves the field to a set holding
  // strictly less than the one it reads by.
  registry.setCodeset('venuesidecodeset', [{ value: '2', name: 'Sell' }])
  const side = fixField('Side', 'utf8', 54)
  side.fix.codeset = 'sidecodeset'
  registry.insert(side)
  const venue = fixField('Side', 'utf8', 54)
  venue.fix.codeset = 'venuesidecodeset'
  registry.update(venue)
  assert.equal(registry.fieldByTag(54).fix.codeset, 'sidecodeset')
  assert.deepEqual(registry.codeset('sidecodeset').codes, [
    { value: '1', name: 'Buy' },
    { value: '2', name: 'Sell' },
  ])
  assert.deepEqual(registry.codesetNames(), ['msgcatcodeset', 'sidecodeset', 'venuesidecodeset'])
})

test('a tag crosses as a number and is never narrowed', () => {
  const field = Field.from('Symbol: utf8')

  for (const value of [2 ** 31, -(2 ** 31) - 1, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => {
      field.fix.tag = value
    }, /signed 32-bit integer/)
    assert.throws(() => {
      field.fix.tags = [55, value]
    }, /signed 32-bit integer/)
  }
  // A refusal leaves the field untouched.
  assert.equal(field.fix.tag, null)
  assert.equal(field.fix.size, 0)

  // The core's own refusals arrive with the full key in the message. A tag
  // is positive: zero is what an unresolved arrival records, never an
  // identity a field can claim (`rust/tests/fix/zero_entries.rs`).
  for (const tag of [0, -1, -(2 ** 31)]) {
    assert.throws(() => {
      field.fix.tag = tag
    }, /FIX:tag.*from 1 to 2147483647/)
    assert.throws(() => {
      field.fix.counter = tag
    }, /FIX:counter.*from 1 to 2147483647/)
    assert.throws(() => {
      field.fix.tags = [4, tag]
    }, /FIX:tags.*from 1 to 2147483647/)
  }
  assert.throws(() => {
    field.fix.tags = [55, 55]
  }, /FIX:tags/)
  assert.throws(() => {
    field.fix.names = ['Sym', 'sym']
  }, /FIX:names/)
  assert.throws(() => {
    field.fix.names = ['Sym"bol']
  }, /FIX:names/)
  assert.equal(field.fix.tag, null)
  assert.equal(field.fix.counter, null)
  assert.deepEqual(field.fix.tags, [])
  assert.equal(field.fix.size, 0)

  // The widest positive tag is one, on every positive tag writer.
  field.fix.tag = 2 ** 31 - 1
  field.fix.counter = 2 ** 31 - 1
  assert.equal(field.fix.tag, 2 ** 31 - 1)
  assert.equal(field.fix.counter, 2 ** 31 - 1)
})

test('an externally stated nonpositive tag is refused where it is read', () => {
  // Text written past the typed setters is read strictly: decimal digits of a
  // positive tag, never signed and never wider than an i32. A registry refuses
  // the field and is left exactly as it was. The alternates are one JSON
  // array, and their elements are held to the same shape.
  for (const [key, property] of [['FIX:tag', 'tag'], ['FIX:counter', 'counter'], ['FIX:tags', 'tags']]) {
    for (const digits of ['0', '000', '-1', '+1', '2147483648']) {
      const text = key === 'FIX:tags' ? `[${digits}]` : digits
      const field = fixField('incoming', 'utf8', 90_001)
      field.set(key, text)
      assert.throws(() => field.fix[property], new RegExp(key), `${key}=${text}`)
      const registry = new fix.FixRegistry()
      const before = registry.clone()
      assert.throws(() => registry.insert(field), `${key}=${text}`)
      assert.ok(registry.equals(before), `${key}=${text}`)
    }
  }
  // Leading zeros are still the tag on the two bare decimals, and never in
  // the array: a JSON number spells none, and the array is JSON.
  const field = fixField('positive', 'utf8', 1)
  for (const key of ['FIX:tag', 'FIX:counter']) field.set(key, '0001')
  field.set('FIX:tags', '[1]')
  assert.equal(field.fix.tag, 1)
  assert.equal(field.fix.counter, 1)
  assert.deepEqual(field.fix.tags, [1])
  field.set('FIX:tags', '[0001]')
  assert.throws(() => field.fix.tags, /FIX:tags/)
})

test('the identifier is a number derived from the tag and the name', () => {
  const trade = Field.from('TradeID: utf8')
  // There is no identity without a tag.
  assert.equal(trade.fix.id, null)

  trade.fix.tag = 5001
  const id = trade.fix.id
  assert.equal(typeof id, 'number')
  assert.ok(Number.isInteger(id))
  assert.ok(id >= -(2 ** 31) && id < 2 ** 31, 'a signed 32-bit digest')
  // Derived on every read, never stored: the view holds the tag alone.
  assert.equal(trade.fix.size, 1)
  assert.equal(trade.has('FIX:id'), false)
  // What the field answers is what the registry answers by it.
  assert.equal(fix.FixRegistry.fromFields([trade]).fieldById(id).name, 'TradeID')

  // The name folds once - ASCII case, `_`, `-` and space dropped - so three
  // spellings of one field under one tag are one identity; another tag or
  // another name is another.
  const ids = ['MsgType', 'msgtype', 'Msg_Type', 'msg-type', 'Msg Type'].map((name) => {
    const field = Field.from(`${name}: utf8`)
    field.fix.tag = 35
    return field.fix.id
  })
  assert.equal(new Set(ids).size, 1)
  const other = Field.from('MsgSeqNum: utf8')
  other.fix.tag = 35
  assert.notEqual(other.fix.id, ids[0])
  const moved = Field.from('MsgType: utf8')
  moved.fix.tag = 36
  assert.notEqual(moved.fix.id, ids[0])
  // A renamed field is another identity, from the same tag.
  trade.fix.tag = 35
  assert.notEqual(trade.fix.id, ids[0])

  // The identity is read, never assigned: the tag and the name are what it
  // is made of, and a property with no setter refuses an assignment.
  assert.throws(() => {
    trade.fix.id = 42
  }, TypeError)
  assert.equal(trade.fix.tag, 35)

  // Any positive tag holds an identity: nothing gates a tag on its
  // dictionary any more. Zero holds none, because no field can claim it: it
  // marks an arrival the dictionary did not resolve.
  for (const tag of [1, 35, 4999, 5000, 39_999, 40_000, 65_000, 2 ** 31 - 1]) {
    const field = Field.from('Any: utf8')
    field.fix.tag = tag
    assert.ok(Number.isInteger(field.fix.id), `${tag}`)
  }
  const zero = Field.from('Any: utf8')
  assert.throws(() => {
    zero.fix.tag = 0
  }, /FIX:tag/)
  assert.equal(zero.fix.id, null)
})

test('membership is a sorted list of dictionary names on the field', () => {
  const trade = Field.from('TradeID: utf8')
  // An absent property is an empty list, and no field the specification
  // alone defines states one.
  assert.deepEqual(trade.fix.branches, [])
  assert.equal(trade.has('FIX:branches'), false)
  assert.equal(trade.fix.hasBranch('cme'), false)

  // Assigning replaces the list: folded once, deduplicated under the fold,
  // sorted, and stored comma-joined under `FIX:branches`.
  trade.fix.branches = ['CME', 'Bloomberg', 'cme']
  assert.deepEqual(trade.fix.branches, ['bloomberg', 'cme'])
  assert.equal(trade.get('FIX:branches'), 'bloomberg,cme')
  assert.equal(trade.fix.hasBranch('CME'), true)
  assert.equal(trade.fix.hasBranch('bloomberg'), true)
  assert.equal(trade.fix.hasBranch('ice'), false)

  // `addBranch` is idempotent under the fold.
  trade.fix.addBranch('ICE')
  trade.fix.addBranch('cme')
  assert.deepEqual(trade.fix.branches, ['bloomberg', 'cme', 'ice'])

  // Membership is provenance: it never touches the identity.
  trade.fix.tag = 5001
  const id = trade.fix.id
  trade.fix.branches = ['venue']
  assert.equal(trade.fix.id, id)

  // An empty array removes the property, as every list property is removed.
  trade.fix.branches = []
  assert.deepEqual(trade.fix.branches, [])
  assert.equal(trade.has('FIX:branches'), false)

  // A name that is empty or carries the separator is the core's refusal,
  // naming the key, and nothing is written by it.
  trade.fix.branches = ['cme']
  for (const bad of [[''], ['c,me'], ['ice', '']]) {
    assert.throws(() => {
      trade.fix.branches = bad
    }, /FIX:branches/)
  }
  assert.throws(() => trade.fix.addBranch(''), /FIX:branches/)
  assert.throws(() => trade.fix.addBranch('a,b'), /FIX:branches/)
  assert.deepEqual(trade.fix.branches, ['cme'])
  // The list is strings, never a bare string or a number.
  assert.throws(() => {
    trade.fix.branches = 'cme'
  })
  assert.throws(() => {
    trade.fix.branches = [5001]
  }, /into rust type `String`/)
  assert.throws(() => trade.fix.addBranch(5001), /into rust type `String`/)
  assert.deepEqual(trade.fix.branches, ['cme'])
})

test('the registry resolves every key the way the core does', () => {
  const registry = seed()
  // The store's fields, and the crate's own beside them: a loaded dictionary
  // holds the crate's definition over any document it finds, and `size`
  // counts the components and the groups the walk answers behind them.
  assert.equal(scalars(registry).length, STORED + CRATE_SCALARS.length)
  assert.equal(registry.size, [...registry].length)

  assert.equal(registry.fieldByTag(55).name, 'symbol')
  assert.equal(registry.getFieldByTag(55).name, 'symbol')
  const symbol = registry.fieldByTag(55).fix.id
  assert.equal(registry.fieldById(symbol).name, 'symbol')
  assert.ok(registry.getFieldById(symbol).equals(registry.fieldByTag(55)))
  // The alternate tag 20 reaches ExecType, which claims 150 canonically.
  assert.equal(registry.fieldByTag(150).name, 'exectype')
  // `OrdStatus` and `ExecType` keep the text the wire spells: the ranked
  // The two the ranked state is read off are the dictionary's own text.
  assert.ok(registry.fieldByTag(39).dtype.equals(DataType.from('utf8')))
  assert.ok(registry.fieldByTag(150).dtype.equals(DataType.from('utf8')))
  // A name answers the canonical spelling whatever case it was asked in.
  assert.equal(registry.fieldByName('symbol').name, 'symbol')
  assert.equal(registry.fieldByName('SYMBOL').name, 'symbol')
  assert.equal(registry.fieldByName('clordid').name, 'clordid')
  // A path reaches a repeating group and one of its members.
  assert.equal(registry.fieldByPath('NoPartyIDs').fix.tag, 453)
  // An occurrence is not a path segment: the walk steps through the list and
  // the member is spelled directly under the counter.
  assert.equal(registry.fieldByPath('parties.partyid').fix.tag, 448)
  assert.equal(registry.fieldByPath('parties.partyrole').name, 'partyrole')
  assert.equal(registry.getFieldByPath('parties.partyid.partyid'), null)

  // The generic pair answers exactly what the specialized one does.
  for (const key of [55, 'Symbol', 'nopartyids', 'parties.partyid']) {
    const answer = registry.field(key)
    assert.ok(answer.equals(registry.getField(key)))
    assert.ok(answer.equals(registry.get(key)))
    assert.equal(registry.has(key), true)
  }
  assert.equal(registry.has(9999), false)
  assert.equal(registry.has('Nope'), false)
  assert.equal(registry.getField(9999), null)
  assert.equal(registry.get('Nope'), null)
  assert.equal(registry.getFieldByPath('Symbol.absent'), null)
  assert.equal(registry.has('55'), false, 'a tag query never consults names')
  // Membership is provenance and the seed states none.
  assert.deepEqual(registry.dialects(), [])
})

test('protocol and MsgType inference stays native and shallow', () => {
  const registry = seed()
  const cases = [
    ['prefix 8=FIX.4.4|35=D|55=AAPL|10=001| Symbol=suffix', MimeType.FIX, 'D'],
    ['ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL', MimeType.ULLINK, '8'],
    ['8=FIX.4.4|35=UL|#SYMBOL=TTF|10=001|', MimeType.FIXUL, 'UL'],
    [
      '8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|',
      MimeType.FIXUL,
      'D',
    ],
    ['level=INFO message=random', MimeType.KEYVALUE, null],
    [
      // A JSON document is JSON, which is what it is, and it declares no
      // type: the codec reads none and names such a row `unknown`.
      '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}',
      MimeType.JSON,
      null,
    ],
  ]
  for (const [line, protocol, msgtype] of cases) {
    assert.ok(MimeType.inferBytes(Buffer.from(line)).equals(protocol))
    assert.ok(MimeType.inferText(line).equals(protocol))
    const bytes = fix.FixCodec.inferMsgtypeBytes(Buffer.from(line))
    assert.equal(bytes?.toString() ?? null, msgtype)
    assert.equal(fix.FixCodec.inferMsgtypeText(line), msgtype)
  }

  assert.equal(fix.FixCodec.inferMsgtypeBytes(Buffer.from('35=AE|')).toString(), 'AE')
  assert.equal(fix.FixCodec.inferMsgtypeText('MSGTYPE=AE|'), 'AE')

  // A JSON document states no half of the exchange on its own, and a
  // `send` its own payload spells is never read as the marker: which way it
  // moved is the prose in front of it, read into FIX's own tag 385 by the
  // rules the dictionary carries on that field. The document itself is a
  // body the codec does not read: the row is one message named `unknown`,
  // stating no type and no entries, whatever the document says inside.
  const answered =
    '{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},' +
    '"value":{"Name":"Router_TradeCapture","Version":"4.7.0"},"status":200}'
  const asked = '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}'
  assert.ok(MimeType.inferText(answered).equals(MimeType.JSON))
  assert.equal(fix.FixCodec.inferMsgtypeText(answered), null)
  const codec = reading(new fix.FixRegistry())
  const read = (line) => {
    const messages = codec.parseLine(Buffer.from(line))
    const message = messages.next().value
    assert.equal(messages.next().done, true, line)
    assert.equal(message.field.name, 'unknown', line)
    assert.equal(message.header().msgtype, '', line)
    assert.equal(message.getByName('Name'), null, line)
    assert.deepEqual(message.entries(), [], line)
    return message
  }
  // A direction is the line's, read apart from the document: prose in front
  // of it spells one, and a duration behind it is the line's too.
  for (const [body, verb, code] of [[answered, 'Response', 'R'], [asked, 'Request', 'S']]) {
    assert.equal(read(body).header().msgdirection, null)
    assert.equal(read(`${verb}: ${body}`).header().msgdirection, code)
    assert.equal(read(`[Jolokia] (DEBUG) ${verb}: ${body} (12 ms)`).byTag(385).asJs(), code)
  }
})

test('one namespace: a reused name merges and a reused tag stands beside its holder', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
  ])
  assert.deepEqual(registry.dialects(), ['cme'])

  // A venue reusing a name under another tag is the same field spelled with
  // another number: `addField` folds it into the holder, which gains the tag
  // as an alternate, the alias, and the membership; `insert` refuses it.
  const venueSymbol = fixField('Symbol', 'utf8', 5055, { branches: ['cme'], names: ['VenueTicker'] })
  assert.throws(() => registry.insert(venueSymbol), /held by Symbol/)
  assert.equal(registry.addField(venueSymbol), false)
  assert.equal(registry.size, 2 + SEEDED)
  const symbol = registry.fieldByTag(55)
  assert.equal(registry.fieldByTag(5055).name, 'Symbol')
  assert.deepEqual(symbol.fix.tags, [5055])
  assert.deepEqual(symbol.fix.names, ['Ticker', 'VenueTicker'])
  assert.deepEqual(symbol.fix.branches, ['cme'])
  assert.equal(registry.fieldByName('venueticker').fix.id, symbol.fix.id)
  assert.equal(registry.getFieldByPath('Symbol').fix.id, symbol.fix.id)

  // A venue reusing a tag under another name is a new thing it defined over
  // that tag: it is registered under its own identity beside the holder,
  // the holder gains the name as an alias, and the bare tag keeps answering
  // the holder while the newcomer is reached by its name or its identity.
  const venueId = fixField('VenueSymbol', 'utf8', 55, { branches: ['cme'] })
  assert.equal(registry.insert(venueId), null)
  assert.equal(registry.size, 3 + SEEDED)
  assert.equal(registry.fieldByTag(55).name, 'Symbol')
  assert.deepEqual(registry.fieldByTag(55).fix.names, ['Ticker', 'VenueTicker', 'VenueSymbol'])
  const newcomer = registry.fieldByName('venuesymbol')
  assert.equal(newcomer.name, 'VenueSymbol')
  assert.notEqual(newcomer.fix.id, symbol.fix.id)
  assert.equal(newcomer.fix.id, venueId.fix.id)
  assert.equal(registry.fieldById(newcomer.fix.id).name, 'VenueSymbol')
  assert.equal(registry.fieldById(symbol.fix.id).name, 'Symbol')
  assert.equal(registry.has(55), true)
  assert.deepEqual(registry.dialects(), ['cme'])

  // A bare string is a name, never an identifier, and a bare number is a
  // tag: an identifier is only ever spelled through the `ById` doors.
  assert.equal(registry.getField('symbol').fix.id, symbol.fix.id)
  assert.equal(registry.getField(`${symbol.fix.id}`), null)
  assert.equal(registry.has(`${symbol.fix.id}`), false)
  assert.equal(registry.get(55).name, 'Symbol')
  assert.equal(registry.remove(`${symbol.fix.id}`), null)
  assert.equal(registry.size, 3 + SEEDED)
})

test('removeById reaches one of two fields on a tag by its own identity', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'], names: ['VenueTrade'] }),
  ])
  registry.insert(fixField('VenueSymbol', 'utf8', 55, { branches: ['cme'] }))
  assert.equal(registry.size, 3 + SEEDED)
  const symbol = registry.fieldByTag(55).fix.id
  const venue = registry.fieldByName('VenueSymbol').fix.id

  // The generic `remove` reads a number as the tag, which the holder answers;
  // the identity is what reaches the newcomer.
  const removed = registry.removeById(venue)
  assert.equal(removed.name, 'VenueSymbol')
  assert.equal(registry.size, 2 + SEEDED)
  assert.equal(registry.getFieldById(venue), null)
  // The alias the holder gained when the newcomer arrived is the holder's
  // to keep: the name still answers, now to the holder alone.
  assert.equal(registry.getFieldByName('venuesymbol').name, 'Symbol')
  assert.equal(registry.fieldByTag(55).name, 'Symbol')
  // A field that is not there answers null rather than throwing.
  assert.equal(registry.removeById(venue), null)
  // And the holder is reached by identifier just as well.
  assert.equal(registry.removeById(symbol).name, 'Symbol')
  assert.equal(registry.getFieldByName('ticker'), null)
  assert.equal(registry.size, 1 + SEEDED)
  assert.equal(registry.removeById(registry.fieldByTag(5001).fix.id).name, 'TradeID')
  assert.equal(registry.size, SEEDED)

  // An identifier is an exact number, never text.
  assert.throws(() => registry.removeById(1.5), /id must be a signed 32-bit integer/)
  assert.throws(() => registry.removeById('55'), /into rust type `f64`/)
  assert.equal(registry.size, SEEDED)
})

test('absence throws with the core message, its get twin answers null', () => {
  const registry = seed()

  assert.throws(
    () => registry.fieldByTag(9999),
    /^Error: expected a fix field at "tag 9999", got nothing$/,
  )
  // An identifier no field stands behind: one derived for a tag the seed
  // does not hold.
  const stray = fixField('TradeID', 'utf8', 5001)
  assert.throws(
    () => registry.fieldById(stray.fix.id),
    new RegExp(`^Error: expected a fix field at "identifier ${stray.fix.id}", got nothing$`),
  )
  assert.throws(() => registry.fieldByName('Nope'), /name \\"Nope\\"/)
  assert.throws(() => registry.fieldByPath('Symbol.absent'), /path Symbol\.absent/)
  assert.throws(() => registry.field(9999), /tag 9999/)
  assert.equal(registry.getFieldByName('Nope'), null)
  assert.equal(registry.getFieldById(stray.fix.id), null)
})

test('a key is a number tag or a string name, and nothing else', () => {
  const registry = seed()

  for (const key of [3.5, 2 ** 31, -(2 ** 31) - 1, Number.NaN]) {
    assert.throws(() => registry.get(key), /key must be a signed 32-bit integer/)
  }
  assert.throws(() => registry.getFieldByTag(2 ** 31), /tag must be a signed 32-bit integer/)
  assert.throws(() => registry.fieldByTag(1.5), /tag must be a signed 32-bit integer/)

  for (const [key, named] of [
    [55n, 'BigInt'],
    [{ tag: 55 }, 'Object'],
    [null, 'Null'],
    [true, 'Boolean'],
    [undefined, 'Undefined'],
  ]) {
    assert.throws(() => registry.get(key), {
      name: 'TypeError',
      message: `key must be a number tag or a string name, got ${named}`,
    })
  }
  // The specialized halves take exactly one shape, checked by Node-API.
  assert.throws(() => registry.fieldByName(55), /into rust type `String`/)
  assert.throws(() => registry.fieldByTag('55'), /into rust type `f64`/)
})

test('every identifier argument is an exact number at the boundary', () => {
  const registry = seed()

  // A fractional or out-of-range number is refused rather than narrowed into
  // another identity, and text is not a number at all.
  for (const bad of [1.5, 2 ** 31, -(2 ** 31) - 1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => registry.fieldById(bad), /id must be a signed 32-bit integer/)
    assert.throws(() => registry.getFieldById(bad), /id must be a signed 32-bit integer/)
    assert.throws(() => registry.removeById(bad), /id must be a signed 32-bit integer/)
  }
  for (const wrong of ['55', null, 55n, { id: 55 }]) {
    assert.throws(() => registry.fieldById(wrong), /into rust type `f64`/)
    assert.throws(() => registry.getFieldById(wrong), /into rust type `f64`/)
  }
  for (const wrong of [55, 3.5]) {
    assert.throws(() => registry.getFieldByName(wrong), /into rust type `String`/)
    assert.throws(() => registry.fieldByPath(wrong), /none of these types `String`, `JsFieldPath`/)
  }
})

test('the registry iterates lazily in ascending identifier order', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
    fixField('Price', 'decimal128(20, 8)', 44),
    fixField('VenueQty', 'int64', 5002, { branches: ['cme'] }),
    fixField('Account', 'utf8', 1),
    fixField('Tail', 'utf8', 9001),
  ])
  // Two fields on one tag: the second stands beside the holder.
  registry.insert(fixField('VenueSymbol', 'utf8', 55))

  // Tag-major, the tag's holder first, then by identifier - the core's own
  // order. The venue fields therefore precede the later standard tag, the
  // seeded SendingTime (52) and TransactTime (60) clocks are ordinary fields
  // in their tag places, and the crate's own scalar fields close the
  // scalars: their tags sit above any a test claims. The definitions walk
  // behind them, in name order - here the crate's two Map groups, filed as
  // groups by the shape they have.
  assert.deepEqual(
    scalars(registry).map((field) => field.fix.tag),
    [1, 44, 52, 55, 55, 60, 5001, 5002, 9001, ...CRATE_TAGS],
  )
  // The crate's own definitions close the walk.
  assert.deepEqual(
    definitionsOf(registry).map((field) => field.name).sort(),
    CRATE_GROUPS.map((field) => field.name).sort(),
  )
  assert.equal([...registry].length, registry.size)
  assert.deepEqual(
    [...registry].filter((field) => [52, 60].includes(field.fix.tag)).map((field) => field.name),
    ['sendingtime', 'transacttime'],
  )
  const pair = [...registry].filter((field) => field.fix.tag === 55).map((field) => field.name)
  assert.deepEqual(pair, ['Symbol', 'VenueSymbol'])
  assert.deepEqual(
    [...registry.keys()].map((field) => field.fix.id),
    [...registry].map((field) => field.fix.id),
  )
  assert.ok([...registry].every((field) => Number.isInteger(field.fix.id)))

  // An unfinished walk shares the registry, so a mutation refuses until the
  // walk ends - by exhaustion or by the `return` a `break` sends.
  const walk = registry.keys()
  assert.equal(walk.next().value.name, 'Account')
  assert.equal(walk.next().value.name, 'Price')
  assert.throws(() => registry.remove(1), /shared with a message/)
  assert.throws(() => registry.removeById(registry.fieldByTag(5001).fix.id), /shared with a message/)
  walk.return()
  assert.equal(registry.remove(1).name, 'Account')
  assert.deepEqual(
    scalars(registry).map((field) => field.fix.tag),
    [44, 52, 55, 55, 60, 5001, 5002, 9001, ...CRATE_TAGS],
  )
  // A walk that stops among the definitions releases the registry as
  // promptly as one that stops among the scalars.
  const late = registry.keys()
  for (let at = 0; at < 8 + CRATE_TAGS.length + 1; at += 1) late.next()
  assert.throws(() => registry.remove(44), /shared with a message/)
  late.return()
  assert.equal(registry.remove(44).name, 'Price')
})

test('the seed iterates in canonical-tag order and every field is standard', () => {
  const registry = seed()

  const names = [...registry].map((field) => field.name)
  assert.deepEqual(names.slice(0, 4), ['account', 'advid', 'advrefid', 'advside'])
  assert.equal(names.length, registry.size)

  const tags = scalars(registry).map((field) => field.fix.tag)
  assert.deepEqual(tags, [...tags].sort((left, right) => left - right))
  // Every stored field is a specification field, and the crate's own are
  // standard fields above every published tag, so no field states a
  // membership at all - and the crate's close the scalars, since nothing
  // the seed stores sits in their block from 65000. The store's own
  // SendingTime and TransactTime are what 52 and 60 answer: a loaded
  // definition supplies its metadata rather than colliding with a seed.
  assert.ok([...registry].every((field) => field.fix.branches.length === 0))
  assert.ok([...registry].every((field) => !field.has('FIX:branches')))
  assert.deepEqual(registry.dialects(), [])
  assert.deepEqual(tags.filter((tag) => tag >= 65000), CRATE_TAGS)
  assert.equal(tags.length, STORED + CRATE_SCALARS.length)
  // The definitions walk behind the scalars: the components and the groups
  // the dictionary declares, the crate's two Map groups among them, each a
  // nested datatype and none of them a scalar the tag doors answer.
  const definitions = definitionsOf(registry)
  assert.ok(definitions.every((field) => field.dtype.kind === 'nested'))
  assert.equal(definitions.length + tags.length, registry.size)
  assert.ok(definitions.some((field) => field.name === 'parties'))
  assert.ok(definitions.some((field) => field.name === 'identifiers'))
})

test('the registry takes every storage location', () => {
  const reference = seed()
  const url = Url.fromPath(SEED)

  for (const location of [SEED, path.resolve(SEED), url.toString(), url, new IOBase(SEED)]) {
    assert.ok(fix.FixRegistry.fromHandle(location).equals(reference))
  }

  // A folder that is not there loads as a new registry - the crate's own
  // fields and the two seeded clocks, nothing else - and is not created.
  const root = scratch()
  try {
    const missing = path.join(root, 'missing')
    assert.ok(fix.FixRegistry.fromHandle(missing).equals(new fix.FixRegistry()))
    assert.equal(fix.FixRegistry.fromHandle(missing).size, SEEDED)
    assert.equal(fs.existsSync(missing), false)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('a malformed native shard names its location', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.mkdirSync(path.join(root, 'fields'))
  fs.writeFileSync(path.join(root, 'fields', '0.json'), '[{"name":"broken"}]')
  assert.throws(() => fix.FixRegistry.fromHandle(root), /0.json/)
})

test('a written catalog reloads its three categories and the sets they read by', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')
  const reference = seed()
  reference.writeInto(dictionary)
  // Four folders: the three categories, and the vocabularies the fields name.
  assert.deepEqual(fs.readdirSync(dictionary).sort(), ['codesets', 'components', 'fields', 'groups'])
  // One document per set, filed under the name a field states.
  assert.deepEqual(
    fs.readdirSync(path.join(dictionary, 'codesets')).sort(),
    reference.codesetNames().map((name) => `${name}.json`).sort(),
  )
  const shards = fs.readdirSync(path.join(dictionary, 'fields'))
  // A shard is named by its tag block, nine digits with leading zeros. The
  // crate's own fields are written too - a store states the whole row - and
  // their block from 65000 is one shard of its own, the fixed row is the
  // `fixmsg` component and the crate's two Maps are two groups; the reload
  // takes the definition every registry holds over the document it finds.
  assert.ok(shards.every((shard) => /^\d{9}\.json$/.test(shard)), shards.join(', '))
  assert.equal(shards.includes('000000000.json'), true)
  assert.equal(shards.includes('000000650.json'), true)
  assert.equal(fs.existsSync(path.join(dictionary, 'components', 'fixmsg.json')), true)
  assert.equal(fs.existsSync(path.join(dictionary, 'groups', 'identifiers.json')), true)
  assert.equal(fs.existsSync(path.join(dictionary, 'groups', 'metadata.json')), true)
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
  const reloaded = fix.FixRegistry.fromHandle(new IOBase(dictionary))
  for (const key of [453, 'partyid', 447, 452]) assert.equal(reloaded.remove(key), null)
  reloaded.insert(fixField('Unreferenced', 'utf8', 39999))
  assert.equal(reloaded.remove(39999).name, 'Unreferenced')
  reloaded.writeInto(new IOBase(dictionary))
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
})

test('membership is stored on the field, in the one shard tree', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')

  const registry = fix.FixRegistry.fromFields([
    fixField('MsgType', 'utf8', 35),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
  ])
  registry.writeInto(dictionary)

  // One shard arithmetic for every field: 5001 / 100 is 50, whatever the
  // field's membership, and nothing is keyed by a dictionary name.
  assert.deepEqual(fs.readdirSync(path.join(dictionary, 'fields')).sort(), [
    '000000000.json',
    '000000050.json',
    '000000650.json',
  ])
  assert.equal(fs.existsSync(path.join(dictionary, 'branches.json')), false)
  assert.equal(
    fs.existsSync(path.join(dictionary, 'fields', '000000650.json')),
    true,
    "the crate's own fields are stored like every other",
  )

  const reloaded = fix.FixRegistry.fromHandle(dictionary)
  assert.ok(reloaded.equals(registry))
  assert.equal(reloaded.stableHash(), registry.stableHash())
  assert.equal(reloaded.fieldById(registry.fieldByTag(5001).fix.id).name, 'TradeID')
  assert.equal(reloaded.fieldByName('tradeid').name, 'TradeID')
  assert.deepEqual(reloaded.fieldByTag(5001).fix.branches, ['cme'])
  assert.deepEqual(reloaded.fieldByTag(35).fix.branches, [])
  assert.deepEqual(reloaded.dialects(), ['cme'])
  // Membership is metadata like any other: it is in the snapshot's fields
  // and nowhere else. The vocabularies are the fourth key, beside the three
  // categories, because a code set is the dictionary's and not a field's.
  const document = registry.toJSON()
  assert.deepEqual(Object.keys(document).sort(), ['codesets', 'components', 'fields', 'groups'])
  assert.deepEqual(document.codesets.map((codeset) => codeset.name), ['msgcatcodeset'])
  assert.equal(document.fields.find((field) => field.name === 'TradeID').metadata['FIX:branches'], 'cme')
})

test('insert, update and remove carry the core rules across', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
    fixField('Price', 'decimal128(20, 8)', 44, { names: ['Px'] }),
  ])
  assert.equal(registry.size, 2 + SEEDED)
  assert.equal(registry.insert(fixField('Side', 'utf8', 54)), null)
  assert.equal(registry.fieldByTag(54).name, 'Side')

  // A key another field holds is refused, naming both; nothing changes.
  assert.throws(
    () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { names: ['ticker'] })),
    /alias \\"ticker\\" of SymbolSfx, held by Symbol/,
  )
  assert.equal(registry.size, 3 + SEEDED)

  // One namespace: the same alias under a venue's membership is the same
  // conflict.
  assert.throws(
    () => registry.insert(fixField('VenueSym', 'utf8', 5055, { branches: ['cme'], names: ['ticker'] })),
    /held by Symbol/,
  )
  assert.equal(registry.size, 3 + SEEDED)
  assert.equal(registry.fieldByName('TICKER').name, 'Symbol')

  // A merge concatenates the two list properties, incoming first.
  registry.update(fixField('SYMBOL', 'utf8', 55, { tags: [65], names: ['Sym'] }))
  const merged = registry.fieldByTag(65)
  assert.equal(merged.name, 'Symbol')
  assert.deepEqual(merged.fix.names, ['Sym', 'Ticker'])
  // A datatype disagreement is refused, never widened.
  assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 55)))
  assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

  assert.equal(registry.remove('sym').name, 'Symbol')
  assert.equal(registry.getFieldByTag(65), null)
  assert.equal(registry.remove(9999), null)

  // A field with no tag cannot enter at all.
  assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /FIX:tag/)
})

test('addField answers whether the field arrived or folded into a stored one', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { tags: [65], names: ['Ticker'], description: 'stored' }),
    fixField('Price', 'float64', 44),
  ])

  // A name that folds to a stored name merges into that field: the stored
  // identity, spelling and nullability stand, the alternate tags and the
  // aliases are the union - stored order first, the incoming canonical tag
  // last - and the incoming metadata wins a shared key.
  const incoming = fixField('symbol', 'utf8', 9001, {
    tags: [66],
    names: ['Sym', 'TICKER'],
    description: 'incoming',
  })
  assert.equal(registry.addField(incoming), false)
  assert.equal(registry.size, 2 + SEEDED)
  const stored = registry.fieldByTag(55)
  assert.equal(stored.name, 'Symbol')
  assert.equal(stored.fix.tag, 55)
  assert.deepEqual(stored.fix.tags, [65, 66, 9001])
  assert.deepEqual(stored.fix.names, ['Ticker', 'Sym'])
  assert.equal(stored.fix.description, 'incoming')

  // Every spelling the incoming field carried now reaches the stored one.
  for (const key of [9001, 66, 65, 'sym', 'TICKER']) {
    assert.equal(registry.field(key).name, 'Symbol', `${key}`)
  }

  // Folding it again changes nothing, and a field nothing answers to arrives
  // whole. `TransactTime` is no longer such a field: every registry seeds it
  // at 60 as a nanosecond UTC clock, so a text spelling of it folds into the
  // seed and its datatype is refused like any other disagreement.
  const folded = registry.intoJson()
  assert.equal(registry.addField(incoming), false)
  assert.equal(registry.intoJson(), folded)
  assert.equal(registry.addField(fixField('Text', 'utf8', 58)), true)
  assert.equal(registry.size, 3 + SEEDED)
  const arrived = registry.intoJson()
  assert.throws(() => registry.addField(fixField('TransactTime', 'utf8', 60)))
  assert.equal(registry.intoJson(), arrived)
  assert.equal(registry.fieldByTag(60).name, 'transacttime')

  // A datatype that disagrees with the stored field is refused, and the
  // refusal writes nothing: merging metadata never redeclares a datatype.
  const settled = registry.intoJson()
  assert.throws(() => registry.addField(fixField('SYMBOL', 'int32', 9002)), /utf8/)
  assert.equal(registry.intoJson(), settled)
  assert.equal(registry.getFieldByTag(9002), null)

  // One of this crate's own tags is every dictionary's already: neither added
  // nor merged.
  assert.equal(registry.addField(fix.crateFields()[0]), false)
  assert.equal(registry.intoJson(), settled)
})

test('a shared registry refuses mutation and a clone is independent', () => {
  const registry = seed()
  const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })
  const message = new fix.FixMsg(root, { symbol: 'AAPL' }, registry)

  for (const mutation of [
    () => registry.insert(fixField('side', 'utf8', 54)),
    () => registry.update(fixField('symbol', 'utf8', 55)),
    () => registry.remove(55),
    () => registry.removeById(registry.fieldByTag(55).fix.id),
  ]) {
    assert.throws(mutation, /shared with a message or installed as the process default/)
  }
  assert.ok(message.registry.equals(registry))
  // The shared dictionary stays readable, and a deep copy is writable.
  assert.equal(registry.fieldByTag(55).name, 'symbol')
  const copy = registry.clone()
  assert.ok(copy.equals(registry))
  copy.insert(fixField('Unreferenced', 'utf8', 39999))
  assert.equal(copy.equals(registry), false)
  assert.equal(copy.size, registry.size + 1)
})

// A hand-built order states its own SendingTime, as every hand-built message
// in the Rust suites does, so two builds of it settle the same clocks. It
// declares typed facts - the type, the side, the clock - among its content,
// which is where a plain row states them.
function order(registry) {
  return fields.struct(
    'NewOrderSingle',
    [
      registry.fieldByTag(35),
      registry.fieldByTag(11),
      registry.fieldByTag(55),
      registry.fieldByTag(38),
      registry.fieldByTag(54),
      registry.fieldByName('nopartyids'),
      registry.fieldByName('Parties'),
      Field.from('9999: utf8'),
      registry.fieldByTag(52),
    ],
    { nullable: false },
  )
}

const ORDER_VALUE = {
  msgtype: 'D',
  clordid: 'C-1',
  symbol: 'AAPL',
  orderqty: Scalar.decimal(100n * 10n ** 18n, 18),
  side: '1',
  nopartyids: 1,
  parties: [{ partyid: 'BROKER', partyidsource: 'D', partyrole: 1 }],
  9999: 'custom',
  sendingtime: SENDING,
}

// The content row a built order keeps: every child that is not a typed fact,
// in the root's order. `ClOrdID(11)` and `OrderQty(38)` are facts the message
// lifts and holds; the side is an ordinary child and stays.
const CONTENT = ['symbol', 'side', 'nopartyids', 'parties', '9999']

// The nanosecond instant `SENDING` is, as the typed facts state it.
const SENDING_NS = 1_704_190_530_000_000_000n

/** The entries of a message flattened pre-order into `[tag, name, value]`. */
function flat(message) {
  const held = []
  const walk = (entries) => {
    for (const entry of entries) {
      held.push([entry.tag, entry.name, entry.value])
      walk(entry.entries)
    }
  }
  walk(message.entries())
  return held
}

test('a message holds its typed facts beside the content row it resolves through the registry', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)

  // A child stating a typed fact - the type, the identifier, the quantity,
  // the sending clock - fills the holder that owns it and leaves the row, so
  // the row is what the root declared beyond them, in its order.
  assert.ok(message.registry.equals(registry))
  assert.equal(message.field.equals(root), false)
  assert.deepEqual([...message.field.dtype.keys()], CONTENT)
  assert.equal(message.size, CONTENT.length)
  assert.equal(message.value.kind, 'sequence')
  assert.equal(message.value.length, CONTENT.length)
  // The header states what the root stated: no version, the type, the
  // clock, and none of the trailer the frame never carried.
  assert.deepEqual(message.header(), {
    beginstring: '',
    msgtype: 'D',
    sendercompid: null,
    targetcompid: null,
    msgseqnum: null,
    sendingtime: SENDING_NS,
    possdupflag: null,
    msgdirection: null,
    signaturelength: null,
    signature: null,
    checksum: null,
  })
  // The side is the event's; the event happened at the sending clock, which
  // is where its chain was created too; the cross code is the client's
  // order identifier, the first stated of the tags a chain is named by.
  assert.equal(message.side, 'BUY')
  assert.equal(message.crosscode, 'C-1')
  assert.equal(message.currunix, SENDING_NS)
  assert.equal(message.event().creaunix, SENDING_NS)
  assert.equal(message.state, '00UNKNOWN')
  assert.equal(message.seqnum, 0)
  assert.equal(message.prevuuid, null)
  assert.deepEqual(message.parentuuids, [])
  assert.deepEqual(message.srcuuids, [])
  assert.equal(message.px, '0')
  // `OrderQty(38)` is the quantity the event is about, so the root's child
  // filled it rather than staying a column.
  assert.equal(message.qty, '100')
  assert.equal(message.currency, 'XXX')
  assert.equal(message.text, null)
  assert.deepEqual(message.metadata, {})
  // Nothing was captured: no plugin, no context, no session. Where the line
  // came from is the reader's statement, on the row, and never here.
  assert.deepEqual(message.capture(), {
    msgpluginid: null,
    msgctxid: null,
    msgsessionid: null,
  })

  // A lookup reaches the row and the holders alike, always as a Scalar: a
  // typed tag answers its holder's fact as the Scalar its column types.
  assert.equal(message.byTag(35).asJs(), 'D')
  assert.equal(message.byId(registry.fieldByTag(35).fix.id).asJs(), 'D')
  assert.equal(message.byName('MsgType').asJs(), 'D')
  assert.equal(message.byTag(54).asJs(), 'BUY')
  assert.ok(message.byTag(52).equals(SENDING))
  assert.ok(message.byTag(65003).equals(SENDING), 'unix, as the clock its column holds')
  assert.equal(message.byTag(65039).asJs(), message.curruuid)
  assert.equal(message.byTag(65017).asJs(), message.currhashcode)
  assert.equal(message.byTag(65048).asJs(), 'C-1')
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.equal(message.byId(registry.fieldByTag(55).fix.id).asJs(), 'AAPL')
  assert.equal(message.byName('SYMBOL').asJs(), 'AAPL')
  assert.equal(message.byTag(38).toString(), '"100.000000000000000000"')
  assert.equal(message.byPath('parties[0].partyid').asJs(), 'BROKER')
  // An unknown tag is retained under its rendered name, never dropped.
  assert.equal(message.byTag(9999).asJs(), 'custom')
  // A fact the message states nothing for is absent, not null.
  assert.equal(message.getByTag(44), null, 'no price')
  assert.equal(message.getByTag(58), null, 'no text')
  assert.equal(message.getByTag(65049), null, 'no metadata')
  // An identifier is exact: one the dictionary holds no field under misses,
  // and so does one whose field the root does not declare.
  const stray = fixField('TradeID', 'utf8', 5001)
  assert.equal(message.getById(stray.fix.id), null)
  assert.equal(message.getById(registry.fieldByTag(44).fix.id), null)

  assert.ok(message.get(55).equals(message.byTag(55)))
  assert.ok(message.at('symbol').equals(message.byTag(55)))
  assert.equal(message.get(1234), null)
  assert.equal(message.getByName('nope'), null)
  assert.equal(message.getByPath('parties.partyid'), null)
  assert.throws(
    () => message.byTag(1234),
    /^Error: expected a fix value at "tag 1234", got nothing$/,
  )
  assert.throws(
    () => message.byId(stray.fix.id),
    new RegExp(`^Error: expected a fix value at "identifier ${stray.fix.id}", got nothing$`),
  )
  assert.throws(() => message.byName('nope'), /name \\"nope\\"/)
  assert.throws(() => message.byPath('parties.partyid'), /path parties\.partyid/)
  assert.throws(() => message.at(55n), {
    name: 'TypeError',
    message: 'key must be a number tag or a string name, got BigInt',
  })
  assert.throws(() => message.byTag(2 ** 31), /tag must be a signed 32-bit integer/)
  // An identifier that is not an exact number is refused, never a miss.
  assert.throws(() => message.byId(1.5), /id must be a signed 32-bit integer/)
  assert.throws(() => message.getById(2 ** 31), /id must be a signed 32-bit integer/)
  assert.throws(() => message.getById('55'), /into rust type `f64`/)
})

test('the entries are the content row read as a tree, and the wire is the header before them', () => {
  const registry = seed()
  const message = new fix.FixMsg(order(registry), ORDER_VALUE, registry)

  // One entry per child the row states, a group as its counter entry with
  // the occurrence's members nested under an entry of their own; a key no
  // dictionary explains carries tag 0 and its own spelling.
  const entries = message.entries()
  assert.deepEqual(entries.map((entry) => [entry.tag, entry.name, entry.value]), [
    [55, 'symbol', 'AAPL'],
    [54, 'side', '1'],
    [453, 'parties', '1'],
    [0, '9999', 'custom'],
  ])
  assert.deepEqual(entries[2].entries, [
    {
      tag: 0,
      name: 'party',
      value: null,
      entries: [
        { tag: 448, name: 'partyid', value: 'BROKER', entries: [] },
        { tag: 447, name: 'partyidsource', value: 'D', entries: [] },
        { tag: 452, name: 'partyrole', value: '1', entries: [] },
      ],
    },
  ])
  assert.ok(entries.every((entry) => Array.isArray(entry.entries)))
  // Iterating the message walks the entries.
  assert.deepEqual([...message], entries)
  // The typed facts are not entries: the wire puts the header and the
  // fields the message lifted in front of them, coded facts as their wire
  // code, and `SendingTime` only because the root stated it. What the
  // message merely *implies* about its market - the bid lane a buy of a
  // hundred fills - reaches no byte of it.
  const wire =
    '35=D|52=20240102-10:15:30|11=C-1|38=100|55=AAPL|54=1|453=1|448=BROKER|447=D|452=1|9999=custom|'
  assert.equal(message.intoBytes(124).toString(), wire)
  assert.equal(message.intoText('|'), wire)
  assert.equal(message.intoText(), wire.replaceAll('|', '\x01'))
  assert.deepEqual(message.intoBytes(), Buffer.from(wire.replaceAll('|', '\x01')))
  assert.throws(() => message.intoText('||'), /one character/)
  assert.throws(() => message.intoBytes(256), /one byte/)
  // The digest is over the wire entries: sixteen bytes, the same for two
  // builds and another for other content.
  assert.equal(message.digest().length, 16)
  assert.deepEqual(message.digest(), new fix.FixMsg(order(registry), ORDER_VALUE, registry).digest())
  assert.notDeepEqual(
    message.digest(),
    new fix.FixMsg(order(registry), { ...ORDER_VALUE, symbol: 'MSFT' }, registry).digest(),
  )
})

test('the identity is settled on every message and follows what it says', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)
  const same = new fix.FixMsg(root, ORDER_VALUE, registry)
  const other = new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)

  // A UUID crosses as its hyphenated text, a hash as a bigint.
  assert.match(message.curruuid, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/)
  assert.match(message.crossuuid, /^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/)
  assert.equal(typeof message.currhashcode, 'bigint')
  assert.equal(typeof message.crosshashcode, 'bigint')
  assert.notEqual(message.currhashcode, 0n)
  assert.notEqual(message.crosshashcode, 0n)
  // Two builds of one statement are one identity; other content is another
  // message of the same chain.
  assert.equal(message.currhashcode, same.currhashcode)
  assert.equal(message.curruuid, same.curruuid)
  assert.notEqual(other.currhashcode, message.currhashcode)
  assert.notEqual(other.curruuid, message.curruuid)
  assert.equal(other.crossuuid, message.crossuuid)
  assert.equal(other.crosshashcode, message.crosshashcode)
  // The event states the same facts the getters do.
  const event = message.event()
  assert.equal(event.curruuid, message.curruuid)
  assert.equal(event.crossuuid, message.crossuuid)
  assert.equal(event.currhashcode, message.currhashcode)
  assert.equal(event.crosshashcode, message.crosshashcode)
  assert.equal(event.crosscode, message.crosscode)
  assert.deepEqual(event.identifiers, message.identifiers)
  assert.equal(event.side, message.side)
  assert.equal(event.currunix, message.currunix)
  assert.equal(event.state, message.state)
  assert.equal(event.seqnum, message.seqnum)
  assert.equal(event.prevuuid, message.prevuuid)
  // A buy of a hundred at no price fills the bid lane's size and nothing
  // else; the other lane is the other party's.
  assert.equal(event.bidqty, '100')
  for (const lane of ['bidpx', 'bidcurrency', 'bidunit', 'askpx', 'askqty', 'askcurrency', 'askunit']) {
    assert.equal(event[lane], null, lane)
  }
  for (const code of ['isincode', 'cusipcode', 'sedolcode', 'bloombergcode', 'figicode', 'cficode', 'miccode']) {
    assert.equal(event[code], null, code)
  }
  assert.equal(event.unit, '')
  assert.equal(event.exprtime, null)
  assert.equal(event.prevunix, null)
  assert.equal(event.snapunix, null)
  // No cross code names no chain: the chain identity is the message's own.
  const heartbeat = new fix.FixMsg(
    fields.struct('Heartbeat', [registry.fieldByTag(35), registry.fieldByTag(52)], { nullable: false }),
    { msgtype: '0', sendingtime: SENDING },
    registry,
  )
  assert.equal(heartbeat.crosscode, '')
  assert.equal(heartbeat.crosshashcode, 0n)
  assert.equal(heartbeat.crossuuid, heartbeat.curruuid)
  assert.equal(heartbeat.size, 0)
  assert.deepEqual(heartbeat.entries(), [])
  assert.equal(heartbeat.intoText('|'), '35=0|52=20240102-10:15:30|')
})

test('bridge capture context is an identifier but never the crosscode or content', () => {
  const registry = seed()
  const message = fixedCodec(registry).parseUllinkLine(Buffer.from(
    'MSGTYPE=8|#ORDERID=ORDER-1|#CLORDID=CLIENT-1|#MSGSESSIONID=SESSION-1|' +
    '#MSGCTXID=CONTEXT-1|#SYMBOL=n/A|#VENUEOWNTHING=n/A|',
  ))

  assert.equal(message.crosscode, 'ORDER-1')
  assert.deepEqual(message.identifiers, {
    clordid: 'CLIENT-1',
    msgsectxid: 'SESSION-1:CONTEXT-1',
    orderid: 'ORDER-1',
  })
  assert.equal(message.getByTag(55), null)
  assert.equal(message.getByName('venueownthing'), null)
  const contentHash = message.currhashcode
  const contentUuid = message.curruuid

  message.set('msgsessionid', 'SESSION-2')
  assert.equal(message.capture().msgsessionid, 'SESSION-2')
  assert.deepEqual(message.identifiers, {
    clordid: 'CLIENT-1',
    msgsectxid: 'SESSION-2:CONTEXT-1',
    orderid: 'ORDER-1',
  })
  assert.equal(message.currhashcode, contentHash)
  assert.equal(message.curruuid, contentUuid)

  message.set('msgctxid', null)
  assert.equal(message.capture().msgctxid, null)
  assert.deepEqual(message.identifiers, {
    clordid: 'CLIENT-1',
    orderid: 'ORDER-1',
  })
  assert.equal(message.currhashcode, contentHash)

  message.set('msgctxid', 'CONTEXT-2')
  assert.equal(message.identifiers.msgsectxid, 'SESSION-2:CONTEXT-2')
  assert.equal(message.currhashcode, contentHash)
})

test('a message is a value: equality, hash, clone and JSON', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)
  const same = new fix.FixMsg(root, ORDER_VALUE, registry)

  assert.ok(message.equals(same))
  assert.equal(message.stableHash(), same.stableHash())
  assert.equal(typeof message.stableHash(), 'bigint')
  assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)), false)
  // A typed fact is part of the value: another side is another message.
  assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, side: '2' }, registry)), false)

  const copy = message.clone()
  assert.ok(copy.equals(message))
  assert.ok(copy.registry.equals(registry))
  assert.equal(copy.curruuid, message.curruuid)
  assert.equal(message.toString(), 'FixMsg("NewOrderSingle", 5 values)')

  // The JSON is the content row's two documents; the typed facts are the
  // holders' to answer.
  const document = message.toJSON()
  assert.deepEqual(Object.keys(document), ['field', 'value'])
  assert.equal(document.field.metadata['FIX:tag'], undefined, 'the root carries no tag')
  assert.equal(document.value[0], 'AAPL')
  assert.equal(document.value.length, CONTENT.length)
  assert.ok(JSON.stringify(message).includes('"NewOrderSingle"'))
})

test("a venue's field and MsgType are both reachable from a venue message", () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('MsgType', 'utf8', 35),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'], names: ['VenueTrade'] }),
    fixField('Symbol', 'utf8', 55, { names: ['Ticker'] }),
  ])
  const root = fields.struct(
    'VenueOrder',
    [Field.from('MsgType: utf8'), Field.from('TradeID: utf8'), Field.from('Symbol: utf8')],
    { nullable: false },
  )
  const message = new fix.FixMsg(
    root,
    { MsgType: 'D', TradeID: 'T-1', Symbol: 'AAPL' },
    registry,
  )

  // One namespace, one step: the venue's own field, the specification's,
  // and every alias either declares resolve alike, whatever dictionary
  // contributed them - membership is provenance, never a tier. The type is
  // the header's, whatever the root spelled it as.
  assert.equal(message.byTag(5001).asJs(), 'T-1')
  assert.equal(message.byName('venuetrade').asJs(), 'T-1')
  assert.equal(message.byTag(35).asJs(), 'D')
  assert.equal(message.header().msgtype, 'D')
  assert.equal(message.size, 2)
  assert.equal(message.byName('ticker').asJs(), 'AAPL')
  assert.equal(message.byId(registry.fieldByTag(5001).fix.id).asJs(), 'T-1')
  assert.equal(message.byId(registry.fieldByTag(35).fix.id).asJs(), 'D')
  assert.ok(registry.fieldByTag(5001).fix.hasBranch('cme'))
  assert.equal(registry.fieldByTag(35).fix.hasBranch('cme'), false)
  // A message root is not a dictionary member: it carries no membership.
  assert.deepEqual(message.field.fix.branches, [])

  // A root that does not declare the child misses it, whatever the
  // dictionary holds.
  const plain = fields.struct(
    'Order',
    [Field.from('MsgType: utf8'), Field.from('TradeID: utf8')],
    { nullable: false },
  )
  const standard = new fix.FixMsg(plain, { MsgType: 'D', TradeID: 'T-1' }, registry)
  assert.equal(standard.byTag(35).asJs(), 'D')
  assert.equal(standard.byTag(5001).asJs(), 'T-1')
  assert.equal(standard.getByTag(55), null)
  assert.equal(standard.getByName('ticker'), null)
})

test('a message refuses a value its field refuses', () => {
  const registry = seed()
  const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })

  // A text field reads any value that spells text, a number included, so what
  // it refuses is a value with no spelling at all. The dictionary folds its
  // names, so the field the refusal names is `symbol`.
  assert.throws(() => new fix.FixMsg(root, { symbol: [1] }, registry), /symbol/)
  assert.throws(() => new fix.FixMsg(Field.from('scalar: utf8'), { symbol: 'AAPL' }, registry))
  assert.throws(() => fix.FixMsg(root, { symbol: 'AAPL' }, registry), /without 'new'/)
})

test('a message links the process default when none is named', () => {
  const registry = fix.FixRegistry.fromFields([fixField('Symbol', 'utf8', 55)])
  const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })

  const global = fix.globalRegistry()
  assert.ok(global instanceof fix.FixRegistry)
  // Whatever this machine has installed, the two calls answer one dictionary.
  assert.ok(global.equals(fix.globalRegistry()))
  assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }).registry.equals(global))
  // An explicit registry is kept instead.
  assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }, registry).registry.equals(registry))
})

test('a registry is a value: equality, hash, clone, JSON and text', () => {
  const registry = seed()

  assert.ok(registry.equals(seed()))
  assert.equal(registry.stableHash(), seed().stableHash())
  assert.equal(typeof registry.stableHash(), 'bigint')
  assert.equal(registry.equals(new fix.FixRegistry()), false)
  assert.equal(registry.toString(), `FixRegistry(${registry.size} fields)`)
  // A new registry is never empty: it holds the crate's own fields and the
  // seeded SendingTime and TransactTime clocks.
  assert.equal(new fix.FixRegistry().size, SEEDED)
  assert.equal(new fix.FixRegistry().toString(), `FixRegistry(${SEEDED} fields)`)

  const document = registry.toJSON()
  // The crate's own are seeded and stored alike, so the snapshot states the
  // store's fields and the crate's scalars beside them.
  assert.equal(document.fields.length, STORED + CRATE_SCALARS.length)
  const stored = document.fields[0]
  const held = JSON.parse(JSON.stringify(registry.fieldByTag(1)))
  assert.equal(stored.name, held.name)
  assert.deepEqual(stored.dtype, held.dtype)
  assert.equal(stored.metadata['FIX:tag'], '1')
  // A field states the name of the vocabulary it reads by and nothing more,
  // in the snapshot exactly as on the field: the members are the
  // dictionary's, under `codesets`, which leads the document because a
  // reader has the sets before it meets a field naming one.
  const coded = document.fields.find((field) => field.metadata['FIX:codeset'] !== undefined)
  const codedHeld = JSON.parse(JSON.stringify(registry.fieldByTag(Number(coded.metadata['FIX:tag']))))
  assert.equal(typeof coded.metadata['FIX:codeset'], 'string')
  assert.equal(codedHeld.metadata['FIX:codeset'], coded.metadata['FIX:codeset'])
  assert.equal(Object.keys(document)[0], 'codesets')
  const set = document.codesets.find((held) => held.name === coded.metadata['FIX:codeset'])
  assert.deepEqual(set.codes, registry.codeset(set.name).codes)
  assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
})

test('the fix namespace is frozen and the raw exports are gone', () => {
  const yggdryl = require('yggdryl')

  assert.ok(Object.isFrozen(fix))
  assert.deepEqual(
    Object.keys(fix).sort(),
    [
      'FixCodec',
      'FixMessages',
      'FixMsg',
      'FixRegistry',
      'MsgType',
      'crateFields',
      'globalRegistry',
      'installGlobalRegistry',
      'schema',
      'schemaCarrying',
      'schemaTags',
    ],
  )
  for (const name of [
    'FixFieldIterator',
    'FixMsg',
    'FixMsgEntries',
    'FixCodec',
    'FixLifecycle',
    'FixDefinitionIterator',
    'MsgTypeIterator',
    'FixRegistry',
    'JsFixMsg',
    'JsFixCodec',
    'JsFixLifecycle',
    'JsFixRegistry',
    'fixCrateFields',
    'fixSchema',
    'fixSchemaCarrying',
    'fixSchemaTags',
    'fixGlobalRegistryNative',
    'fixInstallGlobalRegistryNative',
  ]) {
    assert.equal(name in yggdryl, false, name)
  }
  assert.equal(typeof fix.installGlobalRegistry, 'function')
})

test('installing the process default wins before anything resolves it', () => {
  // Process-wide state, so it is driven in a process of its own.
  const script = `
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require(process.argv[1])
    const seed = fix.FixRegistry.fromHandle(process.argv[2])
    fix.installGlobalRegistry(seed)
    assert.ok(fix.globalRegistry().equals(seed))
    assert.equal(fix.globalRegistry().fieldByTag(55).name, 'symbol')
    assert.equal(fix.globalRegistry().fieldByName('SYMBOL').name, 'symbol')
    const root = fields.struct('row', [fix.globalRegistry().fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { symbol: 'AAPL' }).registry.equals(seed))
    assert.throws(() => fix.installGlobalRegistry(new fix.FixRegistry()), /already resolved/)
    console.log('ok')
  `
  const { execFileSync } = require('node:child_process')
  const output = execFileSync(
    process.execPath,
    ['-e', script, require.resolve('yggdryl'), SEED],
    { encoding: 'utf8' },
  )
  assert.equal(output.trim(), 'ok')
})

test('a reader parses every frame shape the core reads', () => {
  const registry = seed()
  // Two reads of undated bytes agree only under one stated intake clock.
  const reader = fixedCodec(registry)

  assert.equal(reader.parseLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
  assert.equal(reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
  assert.equal(
    reader.parseFixLine(Buffer.from('8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01')).byTag(55).toJSON(),
    'AAPL',
  )
  assert.equal(reader.parsePairs([['55', 'AAPL']]).byTag(55).toJSON(), 'AAPL')
  assert.ok(reader.registry.equals(registry))

  // What every built message has, whatever its line carried: a header
  // opening with `beginstring` - the wire's own, else the version it was read
  // at - and the sending clock the codec settles for an undated one, which
  // stamps the event and its creation alike and never goes back on the wire.
  const pairs = reader.parsePairs([['55', 'AAPL']])
  assert.deepEqual([...pairs.field.dtype.keys()], ['symbol'])
  assert.equal(pairs.header().beginstring, 'FIX.4.4')
  assert.equal(pairs.header().msgtype, '')
  assert.equal(pairs.byTag(8).toJSON(), 'FIX.4.4')
  assert.equal(pairs.header().sendingtime, SENDING_NS)
  assert.ok(pairs.byTag(52).equals(SENDING))
  assert.equal(pairs.currunix, SENDING_NS)
  assert.equal(pairs.event().creaunix, SENDING_NS)
  assert.deepEqual(flat(pairs), [[55, 'symbol', 'AAPL']])
  assert.equal(pairs.intoText('|'), '8=FIX.4.4|55=AAPL|')
  // The stated clock is the one that goes back out and the reference the
  // event is dated against; this transaction stands two years in front of
  // it, far outside the codec's default one-second delay, so the sending
  // clock dates the event.
  const dated = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|60=20260102-10:15:31.5|11=A|10=0|'))
  assert.equal(dated.header().sendingtime, SENDING_NS)
  assert.equal(dated.currunix, SENDING_NS)
  assert.ok(dated.byTag(60).equals(new DataType('datetime64(ns,"UTC")').scalar(1_767_348_931_500_000_000n)))
  assert.equal(dated.intoText('|').startsWith('8=FIX.4.4|35=D|52=20240102-10:15:30|'), true)
  // A transaction time stating only a day is no different: the sending
  // clock stands.
  const day = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|60=20260814|11=A|10=0|'))
  assert.equal(day.currunix, SENDING_NS)

  // A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
  // whose value packs its members behind the two control bytes ULLINK uses.
  const bridge = reader.parseUllinkLine(
    Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    ),
  )
  const inferred = reader.parseLine(
    Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    ),
  ).next().value
  assert.ok(inferred.equals(bridge))
  assert.equal(bridge.byTag(55).toJSON(), 'TTF')
  assert.equal(bridge.byTag(38).toJSON(), '1200.000000000000000000')
  assert.equal(bridge.byTag(44).toJSON(), '41.250000000000000000')
  assert.equal(bridge.side, 'BUY')
  assert.equal(bridge.byPath('parties[0].partyid').asJs(), 'BUYSIDE')
  // The counter said two occurrences and one arrived: the group entry
  // counts what is there.
  const parties = bridge.entries().find((entry) => entry.tag === 453)
  assert.equal(parties.value, '1')
  assert.equal(parties.entries.length, 1)
  assert.equal(bridge.digest().length, 16)
})

test('a reader takes the pins the core takes', () => {
  const registry = seed()

  // Tag 32 is `lastshares` at 4.2 and `lastqty` from 4.3 on. A version
  // settles how a value is read, never what a field is called: what the 4.2
  // spelling restates to is the quantity the event last traded, which the
  // holder answers under either name.
  const dated = reading(registry)
  const named = dated.parseLine(Buffer.from('8=FIX.4.2|35=8|32=100|10=0|')).next().value
  assert.equal(named.field.indexOf('lastqty'), null, 'the event holds it')
  assert.equal(named.lastqty, '100')
  assert.ok(named.getByName('lastshares') !== null)
  assert.ok(named.getByName('lastqty') !== null)
  assert.equal(named.header().beginstring, 'FIX.4.2')

  // A codec pins no version: a row states one, or the line implies it.
  assert.equal(dated.version, undefined)

  // A stated absence produces no field at all.
  const silent = reading(registry, { nullValues: ['<none>'] })
  assert.equal(silent.parseLine(Buffer.from('8=FIX.4.4|35=D|55=<none>|10=0|')).next().value.getByTag(55), null)
})

test('a parse derives what the dictionary derives and states it on the wire', () => {
  const reader = fixedCodec(seed())

  // There is no enriching pass: the parse runs the dictionary's own
  // `FIX:derivation` rules. A `SecurityID` an ISIN's check digit closes has
  // stated its source, and under that source the event's ISIN and the
  // country its prefix names; an order stating no time in force is a day
  // order.
  const line = '8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|'
  const filled = reader.parseLine(Buffer.from(line)).next().value
  assert.equal(filled.byTag(22).toJSON(), '4')
  assert.equal(filled.isincode, 'US0378331005')
  assert.equal(filled.byTag(470).toJSON(), 'US')
  assert.equal(filled.byTag(59).toJSON(), '0', 'an order stating no time in force is a day order')
  // What the message now states is what it emits: the frame leads, the
  // fields it lifted follow, then the row with the day order among it.
  assert.equal(
    filled.intoText('|'),
    '8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|59=0|470=US|10=0|',
  )
  // `TimeInForce(59)` is an ordinary column, so what the dictionary derived
  // for it is an entry like any other, and the trait reads the standing off
  // it.
  assert.ok(flat(filled).some(([tag]) => tag === 59))
  assert.equal(filled.tif, '0')

  // A value no standard closes answers nothing rather than a guess.
  const opaque = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|')).next().value
  assert.equal(opaque.getByTag(22), null)
  assert.equal(opaque.event().isincode, null)
})

test('the official time delay bounds which clock dates the message', () => {
  const registry = seed()
  assert.equal(new fix.FixCodec(registry).officialTimeDelayMs, 1_000)
  assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: undefined }).officialTimeDelayMs, 1_000)
  assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: 0 }).officialTimeDelayMs, 0)
  assert.equal(new fix.FixCodec(registry, { officialTimeDelayMs: -1 }).officialTimeDelayMs, -1)
  assert.throws(() => new fix.FixCodec(registry, { officialTimeDelayMs: 1.5 }), /whole number/i)

  const codec = reading(registry)
  const SENT = 1_787_308_200_415_000_000n
  // A transaction half a second in front of the sending clock is the same
  // event said twice, so the more exact saying of it dates the message.
  const near = codec.parseFixLine(
    Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|'),
  )
  assert.equal(near.currunix, 1_787_308_199_900_000_000n)
  assert.equal(near.event().creaunix, near.currunix)
  // Five seconds out is a different event of the session's day.
  const apart = codec.parseFixLine(
    Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|'),
  )
  assert.equal(apart.currunix, SENT)

  // A message stating no transaction is dated by the regulatory stamp its
  // `TrdRegTimestampType(770)` says is about the event; the nearer stamp is
  // when the report reached a repository, which is not that.
  const stamped = codec.parseFixLine(
    Buffer.from(
      '8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|' +
        '769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|',
    ),
  )
  assert.equal(stamped.currunix, 1_787_308_199_900_000_000n)
  const unranked = codec.parseFixLine(
    Buffer.from('8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|769=20260821-10:30:00.400|770=23|10=0|'),
  )
  assert.equal(unranked.currunix, SENT)

  // The pin is the caller's to widen and to close.
  const wide = reading(registry, { officialTimeDelayMs: 10_000 })
  assert.equal(
    wide.parseFixLine(
      Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|'),
    ).currunix,
    1_787_308_195_000_000_000n,
  )
  const shut = reading(registry, { officialTimeDelayMs: 0 })
  assert.equal(
    shut.parseFixLine(
      Buffer.from('8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|'),
    ).currunix,
    SENT,
  )
})

test('the lifecycle redirects categories snapshots dedup and normalized rows', () => {
  const registry = seed()
  assert.equal(new fix.FixCodec(registry).snapshotNs, null)
  assert.equal(new fix.FixCodec(registry, { snapshotNs: undefined }).snapshotNs, null)
  assert.equal(new fix.FixCodec(registry, { snapshotNs: null }).snapshotNs, null)
  assert.equal(new fix.FixCodec(registry, { snapshotNs: 0n }).snapshotNs, null)
  assert.equal(new fix.FixCodec(registry, { snapshotNs: -1n }).snapshotNs, null)
  assert.throws(() => new fix.FixCodec(registry, { snapshotNs: 1 }), /bigint/i)
  assert.throws(() => new fix.FixCodec(registry, { snapshotNs: 9_223_372_036_854_775_808n }), /signed 64-bit/i)
  const codec = fixedCodec(registry)
  const snapshots = fixedCodec(registry, { snapshotNs: 1_000_000_000n })
  assert.equal(snapshots.snapshotNs, 1_000_000_000n)
  assert.equal(registry.msgtype('D').msgcat, 'ORDR')

  const original = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|'))
  const replay = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|'))
  const distinct = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=8|52=20260102-10:15:32|11=REPLAY-1|55=AAPL|10=0|'))
  assert.equal(original.msgcat, 'ORDR')
  const deduplicated = [...codec.lifecycle([original, original.clone(), replay, distinct])]
  assert.deepEqual(deduplicated.map((message) => message.header().msgseqnum), [7, 8])
  assert.deepEqual(deduplicated.map((message) => message.seqnum), [0, 1])

  const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|11=LATER|isincode=US0378331005|10=0|'))
  const earlier = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=EARLIER|isincode=US0378331005|bloombergcode=AAPL US Equity|10=0|'))
  const learned = [...codec.lifecycle([later, earlier])]
  assert.equal(learned[1].bloombergcode, 'AAPL US Equity')
  assert.equal(later.bloombergcode, null, 'the input is never enriched in place')
  const schema = fix.schema(registry)
  const rebuilt = fix.FixMsg.fromRow(schema, learned[1].intoRow(schema), registry)
  assert.equal(rebuilt.bloombergcode, 'AAPL US Equity')

  const expiring = snapshots.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|'))
  const entries = expiring.entries()
  const deadline = expiring.event().exprtime
  const walked = [...snapshots.lifecycle([expiring])]
  assert.deepEqual(expiring.entries(), entries)
  assert.equal(expiring.event().snapunix, null)
  assert.ok(walked.some((message) => message.currunix === deadline && message.state === '95EXPIRED'))
  const emittedSnapshots = walked.filter((message) => message.event().snapunix !== null)
  assert.ok(emittedSnapshots.length > 0)
  assert.ok(emittedSnapshots.every((message) => message.currunix <= message.event().snapunix && message.event().snapunix < deadline))
})

test('the fixed schema places category beside message type and normalized codes once', () => {
  const registry = seed()
  const schema = fix.schema(registry)
  assert.equal(CRATE.length, 29)
  assert.equal(CRATE_SCALARS.length, 27)
  assert.equal(new fix.FixRegistry().size, 31)
  assert.equal(scalars(new fix.FixRegistry()).length, 29)
  assert.equal(schema.fieldLen, 123)
  assert.equal(fix.schemaTags().length, 119)
  const at = schema.indexOf('msgtype')
  assert.deepEqual(
    [schema.fieldAt(at - 1).name, schema.fieldAt(at).name, schema.fieldAt(at + 1).name, schema.fieldAt(at + 2).name],
    ['beginstring', 'msgtype', 'msgcat', 'msgseqnum'],
  )
  for (const [name, tag] of [['isincode', 65055], ['cusipcode', 65057], ['sedolcode', 65058], ['bloombergcode', 65059], ['miccode', 65060], ['figicode', 65061]]) {
    assert.equal(schema.fieldAt(schema.indexOf(name)).fix.tag, tag, name)
  }
  assert.equal(schema.fieldAt(schema.indexOf('cficode')).fix.tag, 461)
  assert.equal(fix.schemaTags().includes(65056), false)
})

test('security source S identifies FIGI while A remains Bloomberg', () => {
  const registry = seed()
  const codec = fixedCodec(registry)
  const figi = codec.parseFixLine(Buffer.from(
    '8=FIX.4.4|35=D|52=20240102-10:15:30|22=S|48=BBG000BLNQ16|454=1|455=BBG000BLNQ16|456=S|10=0|',
  ))
  assert.equal(figi.figicode, 'BBG000BLNQ16')
  assert.equal(figi.event().figicode, 'BBG000BLNQ16')
  assert.equal(figi.bloombergcode, null)

  const schema = fix.schema(registry)
  const rebuilt = fix.FixMsg.fromRow(schema, figi.intoRow(schema), registry)
  assert.equal(rebuilt.figicode, 'BBG000BLNQ16')
  assert.ok(rebuilt.intoRow(schema).equals(figi.intoRow(schema)))

  const bloomberg = codec.parseFixLine(Buffer.from(
    '8=FIX.4.4|35=D|52=20240102-10:15:30|22=A|48=AAPL US Equity|10=0|',
  ))
  assert.equal(bloomberg.bloombergcode, 'AAPL US Equity')
  assert.equal(bloomberg.figicode, null)

  const stated = figi.clone()
  stated.set(65061, 'BBG000BLNQ16')
  assert.equal(stated.figicode, 'BBG000BLNQ16')
})

test('a parse restates deprecated fields to their latest aliases', () => {
  // A FIX 4.2 execution report: a transaction type, a partial fill, a Rule80A
  // capacity and two identities the specification later moved into `Parties`.
  const line =
    '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
  const latest = fixedCodec(seed()).parseLine(Buffer.from(line)).next().value
  assert.equal(latest.header().beginstring, 'FIX.4.2')
  // The state the report reached is the event's, ranked; the row keeps the
  // code the wire spelled, restated to the latest alias where one applies.
  assert.equal(latest.state, '40PARTFILL')
  assert.equal(latest.byTag(150).asJs(), 'F')
  assert.equal(latest.byTag(39).asJs(), '1')

  // Rule80A A is an agency order.
  assert.equal(latest.byTag(528).toJSON(), 'A')
  // ExecBroker and ClientID are two parties, in tag order, counted.
  assert.equal(latest.byTag(453).asJs(), 2)
  assert.equal(latest.byPath('parties[0].partyid').asJs(), 'BRKR')
  assert.equal(latest.byPath('parties[0].partyrole').asJs(), 1)
  assert.equal(latest.byPath('parties[1].partyid').asJs(), 'CLIENT1')
  assert.equal(latest.byPath('parties[1].partyrole').asJs(), 3)
  // The fill under its newest spelling, reachable by the old one too, and
  // reachable is all it is: an alias is a way of asking rather than a child
  // to store.
  assert.ok(latest.byTag(32).equals(Scalar.decimal(100n)))
  assert.ok(latest.byName('LastShares').equals(Scalar.decimal(100n)))
  assert.equal(latest.lastqty, '100')
  const names = [...latest.field.dtype.keys()]
  assert.equal(names.filter((name) => name === 'lastqty').length, 0, 'the event holds it')
  assert.ok(!names.includes('lastshares'))
  // The event reads the report: the last price, the venue's order
  // identifier as the cross code, the identifiers the message component
  // declares.
  assert.equal(latest.px, '10.5')
  assert.equal(latest.crosscode, 'O1')
  assert.deepEqual(latest.identifiers, { execid: 'E1', orderid: 'O1' })
  // One pass, and the filling read the restated row: a report stating no time
  // in force is a day order, one fill's average is that fill's price, and what
  // it was worth is the quantity times the price.
  assert.equal(latest.byTag(59).toJSON(), '0')
  assert.ok(latest.byTag(6).equals(Scalar.decimal(105n, 1)))
  assert.ok(latest.byTag(381).equals(Scalar.decimal(1050n)))
})

// A Jolokia answer as a bridge log line writes it: a timestamp and a reader
// in front of the document, the duration the call took behind it.
const DOCUMENT = Buffer.from(
  '2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":' +
    '"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting",' +
    '"Version":"4.7.0","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD",' +
    '"BeginString":"FIX.4.4","CurrentPort":9726,"State":"logged"},"status":200} (12 ms)',
)

test('a JSON document is one unknown message carrying only what the row stated', () => {
  const codec = fixedCodec(new fix.FixRegistry(), { captureNames: ['msgpluginid'] })
  const messages = codec.parseTextLine(new TextLine(0, DOCUMENT, ['Router_OrderRouting']))
  const message = messages.next().value
  assert.equal(messages.next().done, true)
  // The codec reads no document: the row is a message that stated no type
  // and no content, so nothing inside the document reaches a fact - the
  // comp ids it spells included - and the wire re-emits the header alone.
  assert.equal(message.field.name, 'unknown')
  assert.deepEqual(message.entries(), [])
  assert.equal(message.size, 0)
  assert.equal(message.header().msgtype, '')
  assert.equal(message.header().sendercompid, null)
  assert.equal(message.header().targetcompid, null)
  assert.equal(message.intoText('|'), '8=FIX.4.4|')
  assert.equal(message.getByName('Name'), null)
  // What the row stated beside the document is carried: the direction the
  // prose in front of it spells, and the capture its header declared.
  assert.equal(message.header().msgdirection, 'R')
  assert.equal(message.byTag(385).asJs(), 'R')
  assert.equal(message.capture().msgpluginid, 'Router_OrderRouting')
  assert.equal(message.byTag(65009).asJs(), 'Router_OrderRouting')
})

test('a row reads back into a message stating the same facts', () => {
  const registry = seed()
  const schema = fix.schema(registry)
  const reader = fixedCodec(registry)
  const message = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|54=1|11=A1|55=AAPL|9999=x|10=0|'))
  const row = message.intoRow(schema)
  const held = fix.FixMsg.fromRow(schema, row, registry)

  // Projected columns and residual entries are one semantic message. Their
  // internal child order may differ, while the row and every named fact stay
  // fixed and the row-provided identity remains exact.
  assert.ok(held.field.equals(schema) === false, 'the root is the content, not the schema')
  assert.equal(held.currhashcode, message.currhashcode)
  assert.equal(held.curruuid, message.curruuid)
  assert.equal(held.crossuuid, message.crossuuid)
  assert.deepEqual(held.header(), message.header())
  assert.equal(held.side, 'BUY')
  for (const tag of [8, 35, 11, 55, 54, 52]) assert.ok(held.byTag(tag).equals(message.byTag(tag)), `tag ${tag}`)
  assert.ok(held.intoRow(schema).equals(row))
  // The process default is the registry when none is named.
  assert.notEqual(fix.FixMsg.fromRow(schema, row).registry, null)
  // A row that does not fit the schema is refused.
  assert.throws(() => fix.FixMsg.fromRow(schema, { nosuchcolumn: 1 }, registry))
})

test("a capture's own columns lead the row", () => {
  const registry = seed()
  // Nullable, because a message parsed on its own states none of them and a
  // row cell is typed by its column (`rust/tests/fix/message.rs`).
  const carrier = fields.struct(
    'line',
    [fields.utf8('url', { nullable: true }), fields.binary('body', { nullable: true })],
    { nullable: false },
  )
  const plain = fix.schema(registry, 'FixMessage')
  const carried = fix.schemaCarrying(carrier, plain)

  assert.equal(carried.fieldAt(0).name, 'url')
  assert.equal(carried.fieldAt(1).name, 'body')
  assert.equal(carried.fieldLen, plain.fieldLen + 2)
  assert.equal(carried.indexOf('msgtype'), plain.indexOf('msgtype') + 2)

  // A column no tag names is the capture's, so a row answers null there: the
  // capture fills it, and nothing in the message says what it held.
  const row = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10=0|')).next().value.intoRow(carried).toJSON()
  assert.equal(row[0], null)
  assert.equal(row[carried.indexOf('msgtype')], 'D')

  // A capture column whose folded name a FIX column takes is not carried in
  // front: `msgSessionId` and `msgsessionid` are one name, and the FIX
  // column is the one a reader spelling it means - so the bridge's session
  // instance reaches that column instead of riding in front. `msgthreadid`
  // names no FIX column, so it is carried and leads the row.
  const stamped = fields.struct(
    'line',
    [
      fields.utf8('url', { nullable: false }),
      fields.binary('body', { nullable: false }),
      fields.utf8('msgthreadid'),
      fields.utf8('msgSessionId'),
    ],
    { nullable: false },
  )
  const folded = fix.schemaCarrying(stamped, plain)
  assert.equal(folded.fieldLen, plain.fieldLen + 3)
  assert.equal(folded.indexOf('msgSessionId'), null)
  assert.equal(folded.indexOf('msgthreadid'), 2)
  assert.equal(folded.indexOf('msgsessionid'), plain.indexOf('msgsessionid') + 3)
})

test('a CBlock read under a dialect stamps membership on everything it produced', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const file = path.join(root, 'bloomberg.cfb')
  fs.writeFileSync(
    file,
    [
      '<?xml version="1.0"?>',
      '<cplugin-configuration fix-version="4.4">',
      '<vocabulary>',
      '<vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />',
      '<vocabulary-tag name="55" alt="Symbol" type="string" />',
      '</vocabulary>',
      '</cplugin-configuration>',
    ].join('\n'),
    'utf8',
  )
  const [registry, roots] = fix.FixRegistry.fromCfbFile(file, 'Bloomberg')
  assert.equal(roots.length, 0)
  assert.equal(registry.size, 2 + SEEDED)

  // Every field the file produced - a standard tag included, since membership
  // means the dictionary speaks it - carries the dialect, folded once.
  assert.deepEqual(registry.fieldByTag(10001).fix.branches, ['bloomberg'])
  assert.deepEqual(registry.fieldByTag(55).fix.branches, ['bloomberg'])
  assert.ok(registry.fieldByTag(10001).fix.hasBranch('BLOOMBERG'))
  assert.deepEqual(registry.dialects(), ['bloomberg'])
  // The crate's own are nobody's: a definition is filed by the shape it
  // has, so each is reached through the door its shape put it behind.
  assert.ok(fix.crateFields().every((field) => {
    const declared = field.fix.counter !== null
      ? registry.fieldByCounter(field.fix.counter)
      : registry.getFieldByTag(field.fix.tag)
    return declared === null || !declared.fix.hasBranch('bloomberg')
  }))

  // Membership is provenance: the codec reads the one namespace with no pin
  // and the venue's field resolves like any other.
  const message = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10001=DEALER-A|10=0|')).next().value
  assert.equal(message.byName('ExcludedDealers').asJs(), 'DEALER-A')
  assert.equal(message.byTag(10001).asJs(), 'DEALER-A')
  assert.ok(flat(message).every(([tag]) => Number.isInteger(tag)))
  const excluded = reading(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10001=NONE|10=0|')).next().value
  assert.equal(excluded.getByName('ExcludedDealers'), null)
  assert.equal(excluded.getByTag(10001), null)

  // With no dialect named nothing is stamped, and a name that is empty or
  // carries the separator is refused before anything is read.
  const [unstamped] = fix.FixRegistry.fromCfbFile(file)
  assert.deepEqual(unstamped.fieldByTag(10001).fix.branches, [])
  assert.deepEqual(unstamped.dialects(), [])
  assert.equal(fix.FixRegistry.fromCfbFile(file, null)[0].dialects().length, 0)
  assert.throws(() => fix.FixRegistry.fromCfbFile(file, 'a,b'), /FIX:branches/)
  assert.throws(() => fix.FixRegistry.fromCfbFile(file, ''), /FIX:branches/)
})

test('a message type keeps its complete wire code and immutable schema', () => {
  const registry = new fix.FixRegistry()
  registry.insert(fixField('msgtype', 'utf8', 35))
  const value = registry.registerMsgtype('P Report Ack', 'AllocationReportAck', 'Allocation Report ACK')
  assert.equal(value.asStr(), 'P Report Ack')
  assert.equal(value.name, 'allocationreportack')
  // The vocabulary is the dictionary's, never a copy on the field: tag 35
  // names the set derived from its own name, and the code registered is a
  // member of that set, its full wire spelling kept as an alias.
  assert.equal(registry.fieldByTag(35).fix.codeset, 'msgtypecodeset')
  assert.deepEqual(registry.codeset('msgtypecodeset').codes, [
    {
      value: 'P Report Ack',
      name: 'AllocationReportAck',
      aliases: ['P Report Ack'],
      doc: 'Allocation Report ACK',
    },
  ])
  assert.equal(registry.codeValue('msgtypecodeset', 'AllocationReportAck'), 'P Report Ack')
  assert.equal(registry.codeName('msgtypecodeset', 'P Report Ack'), 'AllocationReportAck')
  assert.equal(value.asField().fix.msgtype, 'P Report Ack')
  // The registered definition is a component of the registry, reached
  // through the field doors like every definition.
  assert.equal(registry.fieldByName('allocationreportack').fix.msgtype, 'P Report Ack')
  assert.equal(registry.size, SEEDED + 2)
  // A message type is a copy: the dictionary is not shared with it.
  registry.insert(fixField('Symbol', 'utf8', 55))
  assert.ok(registry.getMsgtype('P Report Ack').equals(value))
  const copy = registry.clone()
  assert.ok(copy.registerMsgtype('P Report Ack').equals(value))
})

test('a CBlock is read for what it says, and a truncated one is refused', () => {
  const root = scratch()

  // A declaration this reader cannot make a field of is dropped and the rest
  // of the file is still a dictionary: what went is a warning the host reads
  // through a logger, never a failed read.
  const dropped = path.join(root, 'dropped.cfb')
  fs.writeFileSync(
    dropped,
    [
      '<?xml version="1.0"?>',
      '<cplugin-configuration fix-version="4.4">',
      '<vocabulary>',
      '<vocabulary-tag name="35" alt="MsgType" type="decimal" />',
      '<vocabulary-tag name="55" alt="Symbol" type="string" />',
      '</vocabulary>',
      '</cplugin-configuration>',
    ].join('\n'),
    'utf8',
  )
  const [registry] = fix.FixRegistry.fromCfbFile(dropped, 'bloomberg')
  assert.equal(registry.fieldByTag(55).name, 'symbol')
  assert.throws(() => registry.fieldByTag(35))

  // A document that stops with an element open leaves nothing to keep, and
  // the native sentence crosses whole rather than as a bare "invalid file":
  // the byte the reader stopped at, and the element it left open.
  const truncated = path.join(root, 'broken.cfb')
  fs.writeFileSync(
    truncated,
    [
      '<?xml version="1.0"?>',
      '<cplugin-configuration fix-version="4.4">',
      '<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" />',
      '</cplugin-configuration>',
    ].join('\n'),
    'utf8',
  )
  assert.throws(
    () => fix.FixRegistry.fromCfbFile(truncated, 'bloomberg'),
    (error) => {
      assert.match(error.message, /invalid cfb expression at byte \d+/)
      assert.match(error.message, /vocabulary/)
      return true
    },
  )
})

// One content record, with tag 0 reserved for what the dictionary did not
// resolve. Parity with `rust/tests/fix/zero_entries.rs`.

test('unresolved counters keep their members in arrival order under tag 0', () => {
  const codec = fixedCodec(seed())
  // At the top: an unregistered numeric counter heads what arrived under it.
  const top = codec.parsePairs([
    ['999999', '2'],
    ['999999[0].OwnThing', 'a'],
    ['999999[1].999998', 'b'],
  ])
  assert.deepEqual(flat(top), [
    [0, '999999', null],
    [0, '999999', null],
    [0, '999998', 'b'],
    [0, '999999', null],
    [0, 'ownthing', 'a'],
  ])
  assert.equal(top.intoText('|'), '8=FIX.4.4|999998=b|ownthing=a|')

  // A declared group keeps its resolved member and the unknown children
  // beside it under the counter it stated.
  const registry = new fix.FixRegistry()
  registry.insert(fixField('norows', 'int32', 90_001))
  const group = fields.list('rows', fields.struct('row', [fixField('scopedvalue', 'utf8', 90_002)], { nullable: false }))
  group.fix.counter = 90_001
  assert.equal(registry.insert(group), null)
  assert.equal(registry.getFieldByTag(90_002), null)
  assert.equal(registry.fieldByCounter(90_001).name, 'rows')
  const rows = fixedCodec(registry).parsePairs([
    ['NoRows', '1'],
    ['Rows[0].ScopedValue', 'known'],
    ['Rows[0].999999', 'numeric'],
    ['Rows[0].OwnThing', 'named'],
  ])
  assert.deepEqual(flat(rows), [
    [90_001, 'rows', '1'],
    [0, 'row', null],
    [90_002, 'scopedvalue', 'known'],
    [0, '999999', 'numeric'],
    [0, 'ownthing', 'named'],
  ])
  assert.equal(rows.byPath('rows[0].scopedvalue').asJs(), 'known')
  assert.equal(rows.byPath('rows[0]."999999"').asJs(), 'numeric')
  assert.equal(rows.byPath('rows[0].ownthing').asJs(), 'named')
  assert.equal(rows.intoText('|'), '8=FIX.4.4|90001=1|90002=known|999999=numeric|ownthing=named|')
})
