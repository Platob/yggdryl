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

const { DataType, Field, IOBase, MimeType, Scalar, Url, fields, fix, hashing } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')

// The crate's own thirty-four scalar fields, at 65001-65003, 65005-65015,
// 65017-65019, 65021-65035 and 65037-65038 (`rust/tests/fix/digest.rs`); the
// complete `fix.crateFields()` inventory also lists the altids Map group at
// 65020 and the instids Struct at 65036, and the retired 65000, 65004 and
// 65016 are not reused. A loaded dictionary holds these beside its stored
// fields.
const CRATED = 34
// What a new registry holds before anything is inserted: the crate's own
// scalar fields and the seeded SendingTime (52) and TransactTime (60) clocks
// (`seeded_fields()` in `rust/tests/fix.rs`).
const SEEDED = CRATED + 2
// The crate's scalar tags, in order: the altids group's counter sits between.
const CRATE_TAGS = [
  ...Array.from({ length: 3 }, (_, at) => 65001 + at),
  ...Array.from({ length: 11 }, (_, at) => 65005 + at),
  ...Array.from({ length: 3 }, (_, at) => 65017 + at),
  ...Array.from({ length: 15 }, (_, at) => 65021 + at),
  ...Array.from({ length: 2 }, (_, at) => 65037 + at),
]
// The one intake clock the Rust suites read undated bytes under
// (`fixed_codec` in `rust/tests/fix.rs`): 2024-01-02T10:15:30Z. Without it an
// undated message reads UTC now, which is deliberately not deterministic.
const SENDING = Scalar.datetime(1_704_190_530_000_000_000n, 'ns', 'UTC')

function fixedCodec(registry, options = {}) {
  return new fix.FixCodec(registry, { ...options, defaultSendingTime: SENDING })
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
}

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

function fixField(name, dtype, tag, { branches, tags, aliases, description } = {}) {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.tag = tag
  if (branches) field.fix.branches = branches
  if (tags) field.fix.tags = tags
  if (aliases) field.fix.aliases = aliases
  if (description !== undefined) field.fix.description = description
  return field
}

test('the protocol view carries the typed fix vocabulary', () => {
  const field = Field.from('OrderQty: decimal128(20, 8)')
  field.fix.tag = 38
  field.fix.tags = [1088]
  field.fix.aliases = ['Qty', 'Quantity']
  field.fix.description = 'Quantity ordered.'

  assert.equal(field.fix.tag, 38)
  assert.deepEqual(field.fix.tags, [1088])
  assert.deepEqual(field.fix.aliases, ['Qty', 'Quantity'])
  assert.equal(field.fix.description, 'Quantity ordered.')
  // Ordinary namespaced text, in the one metadata map.
  assert.equal(field.get('fix:aliases'), 'Qty,Quantity')
  assert.equal(field.fix.get('tag'), '38')
  // Three, not four: a description is a fact about the column rather than a
  // FIX fact, so it lives on the generic key every catalog reads.
  assert.equal(field.get('description'), 'Quantity ordered.')
  assert.equal(field.has('fix:description'), false)
  assert.equal(field.fix.size, 3)

  // An empty array removes a list property; `delete` removes any of them.
  field.fix.tags = []
  assert.deepEqual(field.fix.tags, [])
  assert.equal(field.fix.has('tags'), false)
  field.fix.aliases = []
  assert.deepEqual(field.fix.aliases, [])
  assert.equal(field.fix.delete('tag'), true)
  assert.equal(field.fix.tag, null)

  const absent = Field.from('Symbol: utf8')
  assert.equal(absent.fix.tag, null)
  assert.deepEqual(absent.fix.tags, [])
  assert.deepEqual(absent.fix.aliases, [])
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
    assert.throws(() => view.aliases, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.identifiers, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.description, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.branches, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.hasBranch('cme'), { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.id, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.directions, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.tag = 55
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.directions = [{ code: 'S', patterns: ['^TX '] }]
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.derivation, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.derivation = 'orderqty - cumqty'
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.aliases = ['Ticker']
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
  const client = fixField('clordid', 'utf8', 11, { aliases: ['ClientOrder'] })
  const order = fixField('orderid', 'utf8', 37)
  const declaration = fields.struct('order', [client, order], { nullable: false })
  const view = declaration.fix
  assert.deepEqual(view.identifiers, [])
  view.identifiers = ['37', 'ClientOrder']
  assert.deepEqual(view.identifiers, ['clordid', 'orderid'])
  assert.equal(declaration.get('fix:identifiers'), 'clordid,orderid')
  assert.deepEqual(Field.fromJSON(declaration.toJSON()).fix.identifiers, ['clordid', 'orderid'])
  view.identifiers = ['ORDERID']
  assert.deepEqual(view.identifiers, ['orderid'])
  view.identifiers = []
  assert.deepEqual(view.identifiers, [])
  assert.equal(declaration.has('fix:identifiers'), false)
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
  assert.equal(field.get('fix:derivation'), 'orderqty - cumqty')

  assert.throws(() => {
    field.fix.derivation = 'orderqty -'
  })
  assert.equal(field.fix.derivation, 'orderqty - cumqty')

  // An edited derivation is what the reader fills by (decision 38).
  const registry = fix.FixRegistry.fromHandle(SEED)
  const leaves = registry.getFieldByTag(151)
  leaves.fix.derivation = "case when msgtype in ('8', '9') then orderqty * 2 end"
  registry.update(leaves)
  const codec = fixedCodec(registry)
  const held = codec.enrichMessage(codec.parseLine(Buffer.from('8=FIX.4.4|35=8|37=A|38=100|14=0|10=0|')).next().value)
  assert.equal(held.byTag(151).toJSON(), 200)

  field.fix.derivation = null
  assert.equal(field.fix.derivation, null)
  assert.equal(field.has('fix:derivation'), false)
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
    field.get('fix:directions'),
    '[{"code":"S","patterns":["(?i)^TX\\\\b"]},' +
      '{"code":"R","patterns":["(?i)^RX\\\\b"]}]',
  )
  assert.deepEqual(JSON.parse(field.get('fix:directions')), rules)

  // A codec compiles the rules of the dictionary it is built over, once,
  // and the line door fills tag 385 from them; the verb table no longer
  // applies under a stated table.
  const registry = new fix.FixRegistry()
  registry.insert(field)
  const codec = new fix.FixCodec(registry)
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
  assert.equal(field.has('fix:directions'), false)
  assert.deepEqual(new Field('MsgDirection', 'utf8').fix.directions, [])
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
    }, /fix:tag.*from 1 to 2147483647/)
    assert.throws(() => {
      field.fix.counter = tag
    }, /fix:counter.*from 1 to 2147483647/)
    assert.throws(() => {
      field.fix.tags = [4, tag]
    }, /fix:tags.*from 1 to 2147483647/)
  }
  assert.throws(() => {
    field.fix.tags = [55, 55]
  }, /fix:tags/)
  assert.throws(() => {
    field.fix.aliases = ['Sym', 'sym']
  }, /fix:aliases/)
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
  // the field and is left exactly as it was.
  for (const [key, property] of [['fix:tag', 'tag'], ['fix:counter', 'counter'], ['fix:tags', 'tags']]) {
    for (const text of ['0', '000', '-1', '+1', '2147483648']) {
      const field = fixField('incoming', 'utf8', 90_001)
      field.set(key, text)
      assert.throws(() => field.fix[property], new RegExp(key), `${key}=${text}`)
      const registry = new fix.FixRegistry()
      const before = registry.clone()
      assert.throws(() => registry.insert(field), `${key}=${text}`)
      assert.ok(registry.equals(before), `${key}=${text}`)
    }
  }
  // Leading zeros of a positive tag are still that tag.
  const field = fixField('positive', 'utf8', 1)
  for (const key of ['fix:tag', 'fix:counter', 'fix:tags']) field.set(key, '0001')
  assert.equal(field.fix.tag, 1)
  assert.equal(field.fix.counter, 1)
  assert.deepEqual(field.fix.tags, [1])
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
  assert.equal(trade.has('fix:id'), false)
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
  }, /fix:tag/)
  assert.equal(zero.fix.id, null)
})

test('membership is a sorted list of dictionary names on the field', () => {
  const trade = Field.from('TradeID: utf8')
  // An absent property is an empty list, and no field the specification
  // alone defines states one.
  assert.deepEqual(trade.fix.branches, [])
  assert.equal(trade.has('fix:branches'), false)
  assert.equal(trade.fix.hasBranch('cme'), false)

  // Assigning replaces the list: folded once, deduplicated under the fold,
  // sorted, and stored comma-joined under `fix:branches`.
  trade.fix.branches = ['CME', 'Bloomberg', 'cme']
  assert.deepEqual(trade.fix.branches, ['bloomberg', 'cme'])
  assert.equal(trade.get('fix:branches'), 'bloomberg,cme')
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
  assert.equal(trade.has('fix:branches'), false)

  // A name that is empty or carries the separator is the core's refusal,
  // naming the key, and nothing is written by it.
  trade.fix.branches = ['cme']
  for (const bad of [[''], ['c,me'], ['ice', '']]) {
    assert.throws(() => {
      trade.fix.branches = bad
    }, /fix:branches/)
  }
  assert.throws(() => trade.fix.addBranch(''), /fix:branches/)
  assert.throws(() => trade.fix.addBranch('a,b'), /fix:branches/)
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
  // The store's fields, and the crate's own beside them: a store never
  // writes those, so a loaded dictionary holds the crate's definition.
  assert.equal(registry.size, 6241 + CRATED)

  assert.equal(registry.fieldByTag(55).name, 'symbol')
  assert.equal(registry.getFieldByTag(55).name, 'symbol')
  const symbol = registry.fieldByTag(55).fix.id
  assert.equal(registry.fieldById(symbol).name, 'symbol')
  assert.ok(registry.getFieldById(symbol).equals(registry.fieldByTag(55)))
  // The alternate tag 20 reaches ExecType, which claims 150 canonically.
  assert.equal(registry.fieldByTag(150).name, 'exectype')
  // `OrdStatus` and `ExecType` are the order's state, read as one type.
  assert.ok(registry.fieldByTag(39).dtype.equals(DataType.from('state')))
  assert.ok(registry.fieldByTag(150).dtype.equals(DataType.from('state')))
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
      // A bridge configuration document is JSON, which is what it is:
      // what makes one *this* reader's is a shape the codec reads rather
      // than a name the scan gives it.
      '{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:' +
        'name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"}',
      MimeType.JSON,
      'Plugin',
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

  // A bridge configuration document states no half of the exchange on its
  // own, and the `send` its own payload spells is never read as the marker:
  // which way it moved is the prose in front of it, read into FIX's own
  // tag 385 by the rules the dictionary carries on that field.
  const answered =
    '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",' +
    '"type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:' +
    'name=Router_TradeCapture,plugin-type=FIX,type=Plugin":' +
    '{"Name":"Router_TradeCapture"}},"status":200}'
  assert.ok(MimeType.inferText(answered).equals(MimeType.JSON))
  // The ObjectName the answer keys its `value` by states the type, and it is
  // the first one the shallow scan reaches: the wildcard the request echoes
  // names none.
  assert.equal(fix.FixCodec.inferMsgtypeText(answered), 'Plugin')
  const codec = new fix.FixCodec(new fix.FixRegistry())
  const read = (line) => codec.parseLine(Buffer.from(line)).next().value
  assert.equal(read(answered).getByTag(385), null)
  assert.equal(read('Response: ' + answered).byTag(385).asJs(), 'R')
  const selected =
    '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:' +
    'name=Router_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},' +
    '"value":{"Name":"Router_TradeCapture"},"status":200}'
  assert.equal(read(selected).getByTag(385), null)
  assert.equal(read('Request: ' + selected).byTag(385).asJs(), 'S')
  // A direction is the line's and a message is the document's, read apart:
  // a read that selected nothing and a request not yet answered both name
  // no plugin, so neither states a message - there is no envelope left to
  // make a row out of - and the prose in front of one makes it no more one.
  const empty =
    '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",' +
    '"type":"read"},"value":{},"status":200}'
  const asked = '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}'
  for (const [body, verb] of [[empty, 'Response'], [asked, 'Request']]) {
    assert.equal(codec.parseLine(Buffer.from(body)).next().done, true)
    const prosed = `[Jolokia] (DEBUG) ${verb}: ${body}`
    assert.equal(codec.parseLine(Buffer.from(prosed)).next().done, true)
  }
})

test('one namespace: a reused name merges and a reused tag stands beside its holder', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'] }),
  ])
  assert.deepEqual(registry.dialects(), ['cme'])

  // A venue reusing a name under another tag is the same field spelled with
  // another number: `addField` folds it into the holder, which gains the tag
  // as an alternate, the alias, and the membership; `insert` refuses it.
  const venueSymbol = fixField('Symbol', 'utf8', 5055, { branches: ['cme'], aliases: ['VenueTicker'] })
  assert.throws(() => registry.insert(venueSymbol), /held by Symbol/)
  assert.equal(registry.addField(venueSymbol), false)
  assert.equal(registry.size, 2 + SEEDED)
  const symbol = registry.fieldByTag(55)
  assert.equal(registry.fieldByTag(5055).name, 'Symbol')
  assert.deepEqual(symbol.fix.tags, [5055])
  assert.deepEqual(symbol.fix.aliases, ['Ticker', 'VenueTicker'])
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
  assert.deepEqual(registry.fieldByTag(55).fix.aliases, ['Ticker', 'VenueTicker', 'VenueSymbol'])
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
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'], aliases: ['VenueTrade'] }),
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
  // in their tag places, and the crate's own scalar fields close the walk:
  // their tags sit above any a test claims, and a definition is filed by the
  // shape it has - the altids Map at 65020 is a group and the instids Struct
  // at 65036 a component, so neither is a field.
  const crated = fix.crateFields()
    .filter((field) => field.fix.counter === null && field.fieldLen === 0)
    .map((field) => field.fix.tag)
  assert.deepEqual(crated, CRATE_TAGS)
  assert.deepEqual(
    [...registry].map((field) => field.fix.tag),
    [1, 44, 52, 55, 55, 60, 5001, 5002, 9001, ...crated],
  )
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
    [...registry].map((field) => field.fix.tag),
    [44, 52, 55, 55, 60, 5001, 5002, 9001, ...crated],
  )
})

test('the seed iterates in canonical-tag order and every field is standard', () => {
  const registry = seed()

  const names = [...registry].map((field) => field.name)
  assert.deepEqual(names.slice(0, 4), ['account', 'advid', 'advrefid', 'advside'])
  assert.equal(names.length, registry.size)

  const tags = [...registry].map((field) => field.fix.tag)
  assert.deepEqual(tags, [...tags].sort((left, right) => left - right))
  // Every stored field is a specification field, and the crate's own
  // thirty-four are standard fields above every published tag, so no field
  // states a membership at all - and the crate's close the walk, since
  // nothing the seed stores sits in their block from 65000. The store's own
  // SendingTime and TransactTime are what 52 and 60 answer: a loaded
  // definition supplies its metadata rather than colliding with a seed.
  assert.ok([...registry].every((field) => field.fix.branches.length === 0))
  assert.ok([...registry].every((field) => !field.has('fix:branches')))
  assert.deepEqual(registry.dialects(), [])
  assert.deepEqual(tags.filter((tag) => tag >= 65000), CRATE_TAGS)
  assert.equal(registry.size, 6241 + CRATED)
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

test('a written catalog reloads all three categories', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')
  const reference = seed()
  reference.writeInto(dictionary)
  assert.deepEqual(fs.readdirSync(dictionary).sort(), ['components', 'fields', 'groups'])
  const shards = fs.readdirSync(path.join(dictionary, 'fields'))
  assert.equal(shards.length, 66)
  // The crate's own fields are written too - a store states the whole row -
  // and their block from 65000 is one shard of its own; the reload takes the
  // definition every registry holds over the document it finds there.
  assert.equal(shards.includes('650.json'), true)
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
    '0.json',
    '50.json',
    '650.json',
  ])
  assert.equal(fs.existsSync(path.join(dictionary, 'branches.json')), false)
  assert.equal(
    fs.existsSync(path.join(dictionary, 'fields', '650.json')),
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
  // and nowhere else.
  const document = registry.toJSON()
  assert.deepEqual(Object.keys(document).sort(), ['components', 'fields', 'groups'])
  assert.equal(document.fields.find((field) => field.name === 'TradeID').metadata['fix:branches'], 'cme')
})

test('insert, update and remove carry the core rules across', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('Price', 'decimal128(20, 8)', 44, { aliases: ['Px'] }),
  ])
  assert.equal(registry.size, 2 + SEEDED)
  assert.equal(registry.insert(fixField('Side', 'utf8', 54)), null)
  assert.equal(registry.fieldByTag(54).name, 'Side')

  // A key another field holds is refused, naming both; nothing changes.
  assert.throws(
    () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { aliases: ['ticker'] })),
    /alias \\"ticker\\" of SymbolSfx, held by Symbol/,
  )
  assert.equal(registry.size, 3 + SEEDED)

  // One namespace: the same alias under a venue's membership is the same
  // conflict.
  assert.throws(
    () => registry.insert(fixField('VenueSym', 'utf8', 5055, { branches: ['cme'], aliases: ['ticker'] })),
    /held by Symbol/,
  )
  assert.equal(registry.size, 3 + SEEDED)
  assert.equal(registry.fieldByName('TICKER').name, 'Symbol')

  // A merge concatenates the two list properties, incoming first.
  registry.update(fixField('SYMBOL', 'utf8', 55, { tags: [65], aliases: ['Sym'] }))
  const merged = registry.fieldByTag(65)
  assert.equal(merged.name, 'Symbol')
  assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
  // A datatype disagreement is refused, never widened.
  assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 55)))
  assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

  assert.equal(registry.remove('sym').name, 'Symbol')
  assert.equal(registry.getFieldByTag(65), null)
  assert.equal(registry.remove(9999), null)

  // A field with no tag cannot enter at all.
  assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /fix:tag/)
})

test('addField answers whether the field arrived or folded into a stored one', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { tags: [65], aliases: ['Ticker'], description: 'stored' }),
    fixField('Price', 'float64', 44),
  ])

  // A name that folds to a stored name merges into that field: the stored
  // identity, spelling and nullability stand, the alternate tags and the
  // aliases are the union - stored order first, the incoming canonical tag
  // last - and the incoming metadata wins a shared key.
  const incoming = fixField('symbol', 'utf8', 9001, {
    tags: [66],
    aliases: ['Sym', 'TICKER'],
    description: 'incoming',
  })
  assert.equal(registry.addField(incoming), false)
  assert.equal(registry.size, 2 + SEEDED)
  const stored = registry.fieldByTag(55)
  assert.equal(stored.name, 'Symbol')
  assert.equal(stored.fix.tag, 55)
  assert.deepEqual(stored.fix.tags, [65, 66, 9001])
  assert.deepEqual(stored.fix.aliases, ['Ticker', 'Sym'])
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
  assert.equal(registry.size, 6241 + CRATED)
})

// A hand-built order states its own SendingTime, as every hand-built message
// in the Rust suites does, so two builds of it settle the same clocks.
function order(registry) {
  return fields.struct(
    'NewOrderSingle',
    [
      registry.fieldByTag(55),
      registry.fieldByTag(38),
      registry.fieldByName('nopartyids'),
      registry.definition('groups', 'Parties'),
      Field.from('9999: utf8'),
      registry.fieldByTag(52),
    ],
    { nullable: false },
  )
}

const ORDER_VALUE = {
  symbol: 'AAPL',
  orderqty: Scalar.float(100),
  nopartyids: 1,
  parties: [{ partyid: 'BROKER', partyidsource: 'D', partyrole: 1 }],
  9999: 'custom',
  sendingtime: SENDING,
}

// The replay fields a message root lacks, appended in the core's one order
// when the message is built (`Role::ALL` in `rust/src/fix/identity.rs`, and
// `rust/tests/fix/codec.rs`): `sendingtime` closes them when it is appended.
const REPLAY = ['updatedat', 'createdat', 'msghash', 'msgphash', 'code', 'snapshotat', 'sendingtime']

test('a message resolves through the registry it carries', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)

  // The root states SendingTime, so the six other replay fields it lacks are
  // appended behind its own children and the message's field is that root.
  assert.equal(message.field.equals(root), false)
  assert.equal(message.field.fieldLen, root.fieldLen + 6)
  assert.ok(message.registry.equals(registry))
  assert.equal([...message].length, 12)
  // `size` is what Python spells `len(message)`, and it agrees with the walk.
  assert.equal(message.size, 12)
  assert.equal(message.size, [...message.entries()].length)
  for (let at = 0; at < root.fieldLen; at += 1) {
    assert.ok(message.field.fieldAt(at).dtype.equals(root.fieldAt(at).dtype), root.fieldAt(at).name)
  }
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.equal(message.byId(registry.fieldByTag(55).fix.id).asJs(), 'AAPL')
  assert.equal(message.byName('SYMBOL').asJs(), 'AAPL')
  assert.equal(message.byTag(38).toString(), '100.0')
  assert.equal(message.byPath('parties[0].partyid').asJs(), 'BROKER')
  // An unknown tag is retained under its rendered name, never dropped.
  assert.equal(message.byTag(9999).asJs(), 'custom')
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

  // The plain object became the ordered row the root declares, and the
  // replay fields it lacked follow in the core's order.
  const pairs = [...message]
  assert.deepEqual(
    pairs.map(([name]) => name),
    [...root.dtype.keys(), ...REPLAY.filter((name) => name !== 'sendingtime')],
  )
  assert.deepEqual([...message.entries()].map(([name]) => name), pairs.map(([name]) => name))
  assert.equal(pairs[0][1].asJs(), 'AAPL')
  assert.equal(message.value.kind, 'sequence')
  // The stated SendingTime settles every clock the message was built without,
  // no code is an empty code, and the readers answer the row's own values.
  assert.ok(message.byTag(52).equals(SENDING))
  assert.ok(message.updatedat().equals(SENDING))
  assert.ok(message.createdat().equals(SENDING))
  assert.equal(message.byTag(65025).kind, 'null')
  assert.equal(message.byTag(65024).asJs(), '')
  assert.ok(message.updatedat().equals(message.byTag(65003)))
  assert.ok(message.createdat().equals(message.byTag(65023)))
  assert.ok(message.msghash().equals(message.byTag(65017)))
  assert.ok(message.msgphash().equals(message.byTag(65018)))

  // A native Scalar names the same row under the field the message settled:
  // it states the replay fields, and the identities it states are the ones
  // its content computes.
  assert.ok(new fix.FixMsg(message.field, message.value, registry).equals(message))
})

test("a venue's field and MsgType are both reachable from a venue message", () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('MsgType', 'utf8', 35),
    fixField('TradeID', 'utf8', 5001, { branches: ['cme'], aliases: ['VenueTrade'] }),
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
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
  // contributed them - membership is provenance, never a tier.
  assert.equal(message.byTag(5001).asJs(), 'T-1')
  assert.equal(message.byName('venuetrade').asJs(), 'T-1')
  assert.equal(message.byTag(35).asJs(), 'D')
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

test('a message is a value: equality, hash, clone and JSON', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)
  const same = new fix.FixMsg(root, ORDER_VALUE, registry)

  assert.ok(message.equals(same))
  assert.equal(message.stableHash(), same.stableHash())
  assert.equal(typeof message.stableHash(), 'bigint')
  assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)), false)

  // Two builds under one stated SendingTime settle one identity, and other
  // content is another identity under the same code.
  assert.ok(message.msghash().equals(same.msghash()))
  const other = new fix.FixMsg(root, { ...ORDER_VALUE, symbol: 'MSFT' }, registry)
  assert.equal(other.msghash().equals(message.msghash()), false)
  assert.ok(other.msgphash().equals(message.msgphash()))

  const copy = message.clone()
  assert.ok(copy.equals(message))
  assert.ok(copy.registry.equals(registry))
  assert.equal(message.toString(), 'FixMsg("NewOrderSingle", 12 values)')

  const document = message.toJSON()
  assert.deepEqual(Object.keys(document), ['field', 'value'])
  assert.equal(document.field.metadata['fix:tag'], undefined, 'the root carries no tag')
  // The value document is the ordered row, not the object it was written as.
  assert.equal(document.value[0], 'AAPL')
  assert.equal(document.value.length, 12)
  assert.ok(JSON.stringify(message).includes('"NewOrderSingle"'))
})

test('a registry is a value: equality, hash, clone, JSON and text', () => {
  const registry = seed()

  assert.ok(registry.equals(seed()))
  assert.equal(registry.stableHash(), seed().stableHash())
  assert.equal(typeof registry.stableHash(), 'bigint')
  assert.equal(registry.equals(new fix.FixRegistry()), false)
  assert.equal(registry.toString(), `FixRegistry(${6241 + CRATED} fields)`)
  // A new registry is never empty: it holds the crate's own fields and the
  // seeded SendingTime and TransactTime clocks.
  assert.equal(new fix.FixRegistry().toString(), `FixRegistry(${SEEDED} fields)`)

  const document = registry.toJSON()
  // The crate's own are seeded and stored alike, so the snapshot states the
  // store's fields and the crate's thirty-four beside them.
  assert.equal(document.fields.length, 6241 + CRATED)
  const stored = document.fields[0]
  const held = JSON.parse(JSON.stringify(registry.fieldByTag(1)))
  assert.equal(stored.name, held.name)
  assert.deepEqual(stored.dtype, held.dtype)
  assert.equal(stored.metadata['fix:tag'], '1')
  // A snapshot is the store's shape, so a document property is the JSON it
  // is; a `Field`'s own JSON is the core shape, where metadata is text.
  const coded = document.fields.find((field) => field.metadata['fix:codes'] !== undefined)
  const codedHeld = JSON.parse(JSON.stringify(registry.fieldByTag(Number(coded.metadata['fix:tag']))))
  assert.ok(Array.isArray(coded.metadata['fix:codes']))
  assert.equal(typeof codedHeld.metadata['fix:codes'], 'string')
  assert.deepEqual(coded.metadata['fix:codes'], JSON.parse(codedHeld.metadata['fix:codes']))
  assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
})

test('the fix namespace is frozen and the raw exports are gone', () => {
  const yggdryl = require('yggdryl')

  assert.ok(Object.isFrozen(fix))
  assert.deepEqual(
    Object.keys(fix).sort(),
    [
      'FixCodec',
      'FixLifecycle',
      'FixMessages',
      'FixMsg',
      'FixRegistry',
      'MsgType',
      'Plugin',
      'Plugins',
      'crateFields',
      'globalRegistry',
      'installGlobalRegistry',
      'pluginFields',
      'pluginMessage',
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

  // What every built message has, whatever its line carried: it opens with
  // `beginstring` - the wire's own, else the version it was read at - states
  // the `version` the read used, and closes with the replay fields the line
  // did not state, in the core's order: `updatedat`, `createdat`, `msghash`,
  // `msgphash`, `code`, `snapshotat`, `sendingtime` (`rust/tests/fix/codec.rs`).
  // None is an entry unless the line carried it, so the wire re-emits byte
  // for byte.
  const pairs = fixedCodec(registry).parsePairs([['55', 'AAPL']])
  assert.deepEqual([...pairs].map(([name]) => name), ['beginstring', 'symbol', 'version', ...REPLAY])
  // Pairs state no frame, so the read is at the dictionary's newest.
  const newest = '5.0.2'
  assert.equal(pairs.byTag(65001).toJSON(), newest)
  assert.equal(pairs.byTag(8).toJSON(), `FIX.${newest}`)
  assert.deepEqual(pairs.arrivals().map(([tag]) => tag), [55])
  assert.equal(pairs.intoBytes(124).toString(), '55=AAPL|')
  // An undated message takes the codec's default SendingTime, and its event,
  // update and creation instants are that one instant.
  assert.ok(pairs.byTag(52).equals(SENDING))
  assert.equal(pairs.byTag(65025).kind, 'null')
  assert.ok(pairs.updatedat().equals(SENDING))
  assert.ok(pairs.createdat().equals(SENDING))

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
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|',
      'binary',
    ),
  ).next().value
  assert.ok(inferred.equals(bridge))
  assert.equal(bridge.byTag(55).toJSON(), 'TTF')
  assert.equal(bridge.byTag(38).toJSON(), 1200)
  assert.equal(bridge.byTag(44).toJSON(), 41.25)
  assert.equal(bridge.party('1')[0].toJSON(), 'BUYSIDE')
  assert.equal(bridge.digest().length, 16)
  // The counter says two occurrences and one arrived: reported, not repaired.
  assert.equal(bridge.anomalies().length, 1)
  assert.match(bridge.anomalies()[0], /453/)
})

test('a reader takes the pins the core takes', () => {
  const registry = seed()

  // Tag 32 is `lastshares` at 4.2 and `lastqty` from 4.3 on. A version
  // settles how a value is read, never what a field is called: the column is
  // the dictionary's own whatever version read the row, and the 4.2 spelling
  // still reaches it as an alias.
  const dated = new fix.FixCodec(registry)
  const named = dated.parseLine(Buffer.from('8=FIX.4.2|35=8|32=100|10=0|')).next().value
  assert.ok(named.field.indexOf('lastqty') !== null)
  assert.ok(named.getByName('lastshares') !== null)
  assert.ok(named.getByName('lastqty') !== null)

  // A codec pins no version: a row states one, or the line implies it.
  assert.equal(dated.version, undefined)

  // A stated absence produces no field at all.
  const silent = new fix.FixCodec(registry, { nullValues: ['<none>'] })
  assert.equal(silent.parseLine(Buffer.from('8=FIX.4.4|35=D|55=<none>|10=0|')).next().value.getByTag(55), null)
})

test('a reader fills what the line implied and leaves the wire alone', () => {
  const reader = new fix.FixCodec(seed())

  // Enrichment is a call; the rules are the core's. A `SecurityID` an ISIN's
  // check digit closes has stated its source, and under that source the
  // crate's `isincode` column and the country its prefix names.
  const line = '8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|'
  const filled = reader.enrichMessage(reader.parseLine(Buffer.from(line)).next().value)
  assert.equal(filled.byTag(22).toJSON(), '4')
  assert.equal(filled.byTag(65013).toJSON(), 'US0378331005')
  assert.equal(filled.byTag(470).toJSON(), 'US')
  assert.equal(filled.byTag(59).toJSON(), '0', 'an order stating no time in force is a day order')

  // Unfilled, the line states none of them.
  const bare = reader.parseLine(Buffer.from(line)).next().value
  assert.equal(bare.getByTag(22), null)
  assert.equal(bare.getByTag(65013), null)
  assert.equal(bare.getByTag(470), null)

  // Only the row was filled: the wire comes back byte for byte, and a second
  // pass changes nothing.
  assert.equal(filled.intoBytes('|'.charCodeAt(0)).toString(), line)
  assert.ok(reader.enrichMessage(filled).equals(filled))

  // A value no standard closes answers nothing rather than a guess.
  const opaque = reader.enrichMessage(
    reader.parseLine(Buffer.from('8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|')).next().value,
  )
  assert.equal(opaque.getByTag(22), null)
  assert.equal(opaque.getByTag(65013), null)
})

// A Jolokia answer as a bridge log line writes it: a timestamp and a reader
// in front of the document, the duration the call took behind it.
const CONFIGURATION = Buffer.from(
  '2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":' +
    '"com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_OrderRouting,' +
    'plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"Router_OrderRouting",' +
    '"Version":"4.7.0","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD",' +
    '"BeginString":"FIX.4.4","CurrentPort":9726,"State":"logged"},"status":200} (12 ms)',
)

// A bridge line on one plugin, with no comp ids of its own: what ten million
// lines behind one configuration look like.
function pluginRow(plugin) {
  return Buffer.from(`MSGTYPE=8|ACCOUNT=ACCT-000117|PLUGINID=${plugin}|`)
}

function bridge() {
  const registry = seed()
  registry.withPluginFields()
  return registry
}

test('a configuration is a message the crate registered', () => {
  const message = new fix.FixCodec(bridge()).parseLine(CONFIGURATION).next().value
  // The name the crate registered the code under, and the code itself.
  assert.equal(message.field.name, 'pluginconfig')
  assert.equal(message.byTag(35).asJs(), 'UCFG')

  // Registering it is nobody's choice: a registry that never asked for the
  // plugin fields still holds the message, because a component holds its
  // members by value and the code is the crate's own (decision 19).
  const plain = new fix.FixRegistry()
  assert.equal(plain.msgtype('UCFG').name, 'pluginconfig')
  const held = plain.definition('components', 'pluginconfig')
  assert.equal(held.name, fix.pluginMessage().name)
  assert.ok(held.dtype.equals(fix.pluginMessage().dtype))
  assert.equal(held.fix.msgtype, 'UCFG')
  assert.equal(plain.getFieldByTag(20010), null)
  assert.deepEqual(plain.dialects(), [])
  // FIX's own `MsgType` opens the component, the plugin attributes follow,
  // and the three FIX fields a configuration also states close it.
  const component = fix.pluginMessage()
  assert.equal(component.fix.msgtype, 'UCFG')
  const named = []
  for (let at = 0; at < component.fieldLen; at += 1) named.push(component.fieldAt(at).name)
  assert.equal(named[0], 'MsgType')
  assert.deepEqual(named.slice(1, -3), fix.pluginFields().map(field => field.name))
  assert.deepEqual(named.slice(-3), ['BeginString', 'SenderCompID', 'TargetCompID'])

  // A built child, not a pair: the document sent no `35=`, so the arrival
  // record holds none and the wire re-emits exactly as it did before the
  // type existed (decision 17 still holds).
  const wire = message.intoBytes('|'.charCodeAt(0)).toString()
  assert.equal(wire.includes('35='), false)
  assert.equal(wire.includes('MsgType'), false)
  assert.ok(wire.startsWith('SessionInterface=com.ullink.ulbridge'), wire)
})

test('the enriching stream fills a row from the configuration that named its plugin', () => {
  const codec = new fix.FixCodec(bridge())
  const one = body => codec.parseLine(body).next().value
  const named = 'Router_OrderRouting'
  const config = one(CONFIGURATION)

  // Alone, a row naming a plugin states no comp ids and gains none: there is
  // nothing yet to fill them from.
  const [bare] = [...codec.enrichMessages([one(pluginRow(named))])]
  assert.equal(bare.getByTag(49), null)
  assert.equal(bare.getByTag(56), null)

  // Behind the configuration that named it, the same row takes the session's
  // two ends (decision 19).
  const filled = [...codec.enrichMessages([config, one(pluginRow(named))])]
  assert.equal(filled[0].field.name, 'pluginconfig')
  assert.equal(filled[1].byTag(49).asJs(), 'CLI.PROD.TRD')
  assert.equal(filled[1].byTag(56).asJs(), 'ST.PROD')
  // Not the begin string: every built message already fills tag 8 from the
  // version its row was read at, so there is never one absent to fill.
  assert.equal(filled[1].byTag(8).asJs(), bare.byTag(8).asJs())

  // A row that stated its own 49 keeps it, which is what makes the pass
  // idempotent; the one it did not state is still filled.
  const stated = Buffer.from(`MSGTYPE=8|PLUGINID=${named}|SENDERCOMPID=ITS.OWN|`)
  const held = [...codec.enrichMessages([config, one(stated)])][1]
  assert.equal(held.byTag(49).asJs(), 'ITS.OWN')
  assert.equal(held.byTag(56).asJs(), 'ST.PROD')

  // A row naming a plugin no configuration named gains nothing, and so does
  // one naming no plugin at all.
  for (const untouched of [pluginRow('Someone_Else'), Buffer.from('MSGTYPE=8|ACCOUNT=ACCT-000117|')]) {
    const last = [...codec.enrichMessages([config, one(untouched)])][1]
    assert.equal(last.getByTag(49), null)
    assert.equal(last.getByTag(56), null)
  }

  // The memory is the stream's: one message is not a stream, so the door
  // that takes one remembers nothing and fills nothing.
  assert.equal(codec.enrichMessage(one(pluginRow(named))).getByTag(49), null)
})

test('the enriching pass restates before it fills', () => {
  // A FIX 4.2 execution report: a transaction type, a partial fill, a Rule80A
  // capacity and two identities the specification later moved into `Parties`.
  const line =
    '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
  const codec = new fix.FixCodec(seed())
  const read = codec.parseLine(Buffer.from(line)).next().value
  assert.equal(read.byTag(65001).toJSON(), '4.2')
  assert.equal(read.byTag(150).toJSON(), '40PARTFILL')
  assert.equal(read.getByTag(528), null)
  assert.equal(read.getByTag(453), null)

  // Restatement is the pass's first step, not a door of its own: the rules
  // are the dictionary's, and the row is canonical before anything fills it.
  const latest = codec.enrichMessage(read)
  // ExecTransType Cancel wrote ExecType TradeCancel over the retired
  // PartiallyFilled, and the source stays.
  assert.equal(latest.byTag(150).toJSON(), '40TRDCXL')
  assert.equal(latest.byTag(20).toJSON(), '1')
  // Rule80A A is an agency order.
  assert.equal(latest.byTag(528).toJSON(), 'A')
  assert.equal(latest.byTag(47).toJSON(), 'A')
  // ExecBroker and ClientID are two parties, in tag order, counted.
  assert.equal(latest.byTag(453).asJs(), 2)
  assert.equal(latest.byPath('parties[0].partyid').asJs(), 'BRKR')
  assert.equal(latest.byPath('parties[0].partyrole').asJs(), 1)
  assert.equal(latest.byPath('parties[1].partyid').asJs(), 'CLIENT1')
  assert.equal(latest.byPath('parties[1].partyrole').asJs(), 3)
  // The fill under its newest spelling, reachable by the old one too, and
  // reachable is all it is: the registry answers a field for any alias it
  // holds, so an alias is a way of asking rather than a child to store.
  assert.equal(latest.byTag(32).asJs(), 100)
  assert.equal(latest.byName('LastShares').asJs(), 100)
  const names = [...latest].map(([name]) => name)
  assert.equal(names.filter((name) => name === 'lastqty').length, 1)
  assert.ok(!names.includes('lastshares'))
  // The row speaks the dictionary's newest version; the wire still says 4.2.
  assert.equal(latest.byTag(65001).toJSON(), '5.0.2')
  assert.equal(latest.byTag(8).toJSON(), 'FIX.4.2')

  // One pass, and the filling read the restated row: a report stating no time
  // in force is a day order, one fill's average is that fill's price, and what
  // it was worth is the quantity times the price.
  assert.equal(latest.byTag(59).toJSON(), '0')
  assert.equal(latest.byTag(6).asJs(), 10.5)
  assert.equal(latest.byTag(381).asJs(), 1050)

  // Only the row was touched: the wire comes back byte for byte, the arrival
  // record and the anomalies are the same, and a second pass changes nothing.
  assert.equal(latest.intoBytes('|'.charCodeAt(0)).toString(), line)
  assert.deepEqual(latest.arrivals(), read.arrivals())
  assert.deepEqual(latest.anomalies(), read.anomalies())
  assert.ok(codec.enrichMessage(latest).equals(latest))
})

// The messages of one order's life, as a venue and its client tell it.
const LIFE = [
  // The order, sent under the client's own identifier.
  '8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|',
  // Acknowledged under the venue's, which now names the same chain.
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|',
  // Half of it done.
  '8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|',
  // Replaced: the new client identifier names the old one, and joins.
  '8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|44=12.6|60=20260102-10:15:32.000|10=0|',
  '8=FIX.4.4|35=8|41=A1|11=A2|37=O1|17=E3|150=5|39=5|55=AAPL|207=XNAS|15=USD|38=120|14=50|151=70|60=20260102-10:15:32.100|10=0|',
  // Filled under the new identifier alone: the chain ends here.
  '8=FIX.4.4|35=8|11=A2|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=120|14=120|151=0|32=70|31=12.6|60=20260102-10:15:33.000|10=0|',
]
// The lifecycle's identity columns and the chain facts it stamps, by tag.
const UUID = 65017
const PUUID = 65018
const PREVUPDATEDAT = 65021
const PREVUUID = 65022
const CODE = 65024
const PIPE = '|'.charCodeAt(0)

/**
 * The sixteen bytes one identity column holds as lowercase hex, or null where
 * it holds nothing.
 *
 * Every identity is `fixedbinary(16)` - plain bytes with no version or
 * variant bit, which is what the Rust suite's `bytes` helper checks - and a
 * `Buffer` is what JavaScript gets. Hex keeps the bytes' own order, so a
 * string comparison here is a byte comparison.
 */
function identity(message, tag) {
  const held = message.getByTag(tag)
  if (held === null || held.kind === 'null') return null
  assert.equal(held.id, 'fixed_size_binary', `tag ${tag} holds sixteen fixed bytes`)
  const bytes = held.asJs()
  assert.equal(bytes.length, 16, `tag ${tag} is sixteen bytes wide`)
  return Buffer.from(bytes).toString('hex')
}

/**
 * The msgphash the core computes for a message whose code is `code`: XXH3-128 of
 * the code bytes alone, so any message naming the code answers it.
 */
function persistentOf(code) {
  const registry = new fix.FixRegistry()
  const root = fields.struct('coded', [registry.fieldByTag(CODE), registry.fieldByTag(52)], { nullable: false })
  return new fix.FixMsg(root, { code, sendingtime: SENDING }, registry).msgphash()
}

test('every message of one order carries the chain identity until it ends', () => {
  const registry = seed()
  const reader = fixedCodec(registry)
  const life = new fix.FixLifecycle(registry)
  const stamped = []
  for (const line of LIFE) {
    stamped.push(life.fill(reader.parseLine(Buffer.from(line)).next().value))
    // Alive from the first message to the fill that ends it.
    assert.equal(life.alive, stamped.length < LIFE.length ? 1 : 0)
  }

  // One instrument, one chain, six messages. The instrument is the scope the
  // chain code opens with - thirty-two hex digits - and no column holds it.
  const codes = stamped.map((held) => held.byTag(CODE).asJs())
  const scope = codes[0].split('/')[0]
  assert.equal(scope.length, 32)
  assert.ok(codes.every((code) => code.startsWith(scope)))
  const chains = stamped.map((held) => identity(held, PUUID))
  assert.ok(
    chains.every((held) => held === chains[0]),
    "the replace's new identifier joined the chain the old one opened",
  )
  // The first creation instant survives the replacement and the terminal
  // fill: it is the order's own transaction time.
  assert.ok(
    stamped.every((held) => held.createdat().equals(stamped[0].createdat())),
    'the first creation instant survives replacement and terminal fill',
  )
  assert.ok(stamped[0].createdat().equals(stamped[0].byTag(60)))
  assert.equal(hashing.txhash.unixOf(stamped[0].byTag(60)), 1_767_348_930_000_000n)
  const ids = stamped.map((held) => identity(held, UUID))
  for (let at = 1; at < ids.length; at += 1) {
    assert.notEqual(ids[at - 1], ids[at], 'different finalized message content')
    if (stamped[at - 1].updatedat().compare(stamped[at].updatedat()) < 0) {
      assert.ok(ids[at - 1] < ids[at], 'identities sort by the full grid instant')
    }
  }
  // A chain carries only its previous message's clock and identity: none before
  // the first message, then each message's predecessor.
  for (const tag of [PREVUPDATEDAT, PREVUUID]) assert.equal(stamped[0].byTag(tag).kind, 'null')
  for (let at = 1; at < stamped.length; at += 1) {
    assert.ok(stamped[at].byTag(PREVUPDATEDAT).equals(stamped[at - 1].updatedat()), `message ${at}`)
    assert.ok(stamped[at].byTag(PREVUUID).equals(stamped[at - 1].msghash()), `message ${at}`)
  }
  // The chain is named by the instrument scope and the first identifier, and
  // its msgphash is the hash of that code alone.
  const code = codes[0]
  assert.equal(code, `${scope}/A1`)
  assert.ok(stamped[0].msgphash().equals(persistentOf(code)))
  assert.ok(stamped[0].msgphash().equals(stamped[0].byTag(PUUID)))
  // A state a row holds is the ranked spelling, never the wire's code.
  assert.equal(stamped[1].byTag(39).toJSON(), '20NEW')
  assert.equal(stamped[5].byTag(39).toJSON(), '80FILLED')

  // Nothing here is an entry: the wire re-emits byte for byte.
  for (const [at, line] of LIFE.entries()) {
    assert.equal(stamped[at].intoBytes(PIPE).toString(), line)
  }

  // Reusing the code tomorrow is the same chain identity, but a fresh live
  // incarnation: its creation is its own clock and it has no predecessor.
  const tomorrow = LIFE[0].replaceAll('20260102', '20260103')
  const again = life.fill(reader.parseLine(Buffer.from(tomorrow)).next().value)
  assert.equal(identity(again, PUUID), chains[0])
  assert.ok(again.createdat().equals(again.byTag(60)))
  assert.equal(again.createdat().equals(stamped[0].createdat()), false)
  assert.equal(again.byTag(PREVUUID).kind, 'null')
  assert.equal(life.alive, 1)
  assert.equal(life.toString(), 'FixLifecycle(1 alive)')
  life.clear()
  assert.equal(life.alive, 0)
  assert.equal(String(life), 'FixLifecycle(0 alive)')
  // The same line at the same instant is the same chain and message identity,
  // which is what makes two reads of one capture agree.
  const replayed = life.fill(reader.parseLine(Buffer.from(LIFE[0])).next().value)
  assert.equal(identity(replayed, PUUID), chains[0])
  assert.equal(identity(replayed, UUID), ids[0])
  assert.ok(replayed.createdat().equals(stamped[0].createdat()))

  // Over any iterable, the reader runs one lifecycle for the whole stream,
  // lazily, answering what a fresh lifecycle answers, and a stamped stream
  // read again keeps what it carries.
  const stream = reader.lifecycle(reader.parseLines(LIFE.map((line) => Buffer.from(line))))
  assert.ok(stream instanceof fix.FixMessages)
  const once = [...stream]
  const twice = [...reader.lifecycle(once)]
  assert.equal(once.length, LIFE.length)
  assert.equal(twice.length, LIFE.length)
  for (const [at, first] of once.entries()) {
    for (const tag of [UUID, PUUID]) {
      assert.equal(identity(first, tag), identity(twice[at], tag), `tag ${tag}`)
    }
    assert.equal(first.arrivals().length, twice[at].arrivals().length)
    assert.ok(first.equals(twice[at]), `message ${at}`)
    assert.ok(first.equals(stamped[at]), `message ${at}`)
  }
  assert.equal(identity(once[5], PUUID), chains[0])
  assert.deepEqual([...reader.lifecycle([])], [])
})

test('a message naming no order has an id and no chain', () => {
  const reader = fixedCodec(seed())
  const [heartbeat] = reader.lifecycle([
    reader.parseLine(Buffer.from('8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|')).next().value,
  ])
  // A stream over anything that is not iterable, or holding what is not a
  // message, is refused: before anything is pulled, or where the item is met.
  assert.throws(() => reader.lifecycle(42), TypeError)
  const mixed = reader.lifecycle([heartbeat, '8=FIX.4.4|35=0|10=0|'])
  assert.equal(mixed.next().done, false)
  assert.throws(() => mixed.next(), TypeError)
  assert.equal(mixed.next().done, true)
  const sent = identity(heartbeat, UUID)
  assert.notEqual(sent, null, 'every message has an id')
  // No identifier names no chain: the code stays the empty name, whose msgphash
  // is the one deterministic hash of no bytes.
  assert.equal(heartbeat.byTag(CODE).asJs(), '')
  assert.ok(heartbeat.msgphash().equals(persistentOf('')))
  assert.notEqual(identity(heartbeat, PUUID), null)
  for (const tag of [PREVUPDATEDAT, PREVUUID]) assert.equal(heartbeat.byTag(tag).kind, 'null')
  // The event is the stated sending time, already on the one-second grid.
  assert.ok(heartbeat.updatedat().equals(heartbeat.byTag(52)))
  // An undated message takes the configured intake clock, settled once.
  const [undated] = reader.lifecycle([reader.parseLine(Buffer.from('8=FIX.4.4|35=0|10=0|')).next().value])
  assert.ok(undated.byTag(52).equals(SENDING), 'the configured intake clock is settled once')
  assert.ok(undated.updatedat().equals(undated.byTag(52)))

  // A lifecycle over the process default is the same pass: the columns are
  // the crate's own, which every registry holds. It stamps messages, never
  // lines, and replaying a stamped message answers its own identity.
  const life = new fix.FixLifecycle()
  assert.equal(life.alive, 0)
  assert.equal(identity(life.fill(heartbeat), UUID), sent)
  assert.throws(() => life.fill('8=FIX.4.4|35=0|10=0|'))
})

test('the instrument identity is the same across spellings and venues', () => {
  const registry = seed()
  const reader = fixedCodec(registry)
  const life = new fix.FixLifecycle(registry)
  const filled = (line) => life.fill(reader.parseLine(Buffer.from(line)).next().value)
  // The instrument is a scope, not a column: the chain code a message opens
  // is `<scope hex>/<identifier>`, so the text before the slash is the
  // identity, and `-` is a message that named no instrument.
  const instrument = (line) => {
    const scope = filled(line).byTag(CODE).asJs().split('/')[0]
    return scope === '-' ? null : scope
  }
  // An ISIN outranks a symbol, so the same security under two symbols is one
  // instrument, and case is not a difference.
  const byIsin = instrument('8=FIX.4.4|35=D|11=B1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|10=0|')
  assert.notEqual(byIsin, null)
  assert.equal(instrument('8=FIX.4.4|35=D|11=B2|48=us0378331005|22=4|55=APPLE|207=xnas|15=usd|10=0|'), byIsin)
  // Another market is another instrument identity.
  assert.notEqual(instrument('8=FIX.4.4|35=D|11=B3|48=US0378331005|22=4|55=AAPL|207=XLON|15=USD|10=0|'), byIsin)
  // Without an ISIN the symbol stands in, and a stated one wins over a
  // symbol that would say otherwise.
  const bySymbol = instrument('8=FIX.4.4|35=D|11=B4|55=AAPL|207=XNAS|15=USD|10=0|')
  assert.notEqual(bySymbol, null)
  assert.notEqual(bySymbol, byIsin)
  // A bridge row names the same facts under its own keys, and the chain a
  // typed one opens is scoped by the instrument those keys named.
  assert.equal(
    instrument('MSGTYPE=D|#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B5|'),
    byIsin,
  )
  // Five typed orders keep their chains alive. An untyped bridge row states
  // no MsgType, so no declared identifier selection names a chain for it: its
  // code stays empty and it opens none.
  const bridged = filled('#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B6|')
  assert.equal(bridged.getByTag(35), null)
  assert.equal(bridged.byTag(CODE).asJs(), '')
  assert.equal(life.alive, 5)
})

test('the fixed row is spelled by name, filled by tag and never shifts', () => {
  const registry = seed()
  const schema = fix.schema(registry, 'FixMessage')
  // A column is the dictionary's folded name, never the tag's digits; the
  // tag stays on the column as its identity. The crate's own lead the row -
  // a table is read by time and joined by identity - and the protocol's own
  // follow them.
  assert.equal(schema.fieldAt(0).name, 'updatedat')
  const header = schema.indexOf('beginstring')
  assert.equal(schema.fieldAt(header).fix.tag, 8)
  assert.equal(schema.fieldAt(header + 2).name, 'msgtype')
  assert.equal(schema.fieldAt(header + 2).fix.tag, 35)
  // One group closes the row: `fixentries`, the whole arrival record, under
  // the `nofixentries` that counts it, with FIX's own `MsgDirection`, where
  // the line was read from, and the settled chain facts before them.
  const tail = []
  for (let at = schema.fieldLen - 9; at < schema.fieldLen; at += 1) tail.push(schema.fieldAt(at).name)
  assert.deepEqual(tail, [
    'secaltidgrp',
    'notrdregtimestamps',
    'trdregtimestamps',
    'signaturelength',
    'signature',
    'checksum',
    'msgdirection',
    'nofixentries',
    'fixentries',
  ])
  assert.equal(schema.indexOf('nounmappedfixentries'), null)
  assert.equal(schema.indexOf('timestamp'), null)
  assert.equal(schema.indexOf('uuid'), null)
  assert.equal(schema.indexOf('instuuid'), null)
  assert.deepEqual(fix.schemaTags().slice(header, header + 3), [8, 9, 35])
  // The crate's own facts open the tagged columns, in three groups - the
  // clocks, then the identities, then everything else the crate knows - and
  // only FIX's own `MsgDirection`, read off the line where the wire states
  // none, and the arrival record's counter close the row.
  assert.equal(fix.schemaTags().length, 116)
  assert.deepEqual(fix.schemaTags().slice(0, 10), [
    65003, 65021, 65023, 65025, 65028, 65029,
    65017, 65018, 65022, 65024,
  ])
  assert.deepEqual(fix.schemaTags().slice(-2), [385, 65027])

  // A column is found by its folded name, and nothing else is needed.
  assert.equal(schema.indexOf('msgtype'), header + 2)
  assert.equal(schema.indexOf('35'), null)
  assert.equal(schema.indexOf('999999'), null)
  assert.equal(schema.name, 'FixMessage')
  const mapping = schema.fieldAt(schema.indexOf('altids'))
  assert.equal(mapping.fix.counter, 65020)
  assert.equal(mapping.fix.tag, 65020)
  assert.equal(mapping.nullable, true)
  assert.equal([...Array(schema.fieldLen).keys()].filter((at) => schema.fieldAt(at).name === 'altids').length, 1)

  // BeginString and the replay fields whose values every message settles are
  // declared so; every other is nullable - `snapshotat` among them, because
  // only a snapshot stamps it - because a message that carried nothing there
  // must answer null rather than shift its neighbours.
  const required = []
  for (let at = 0; at < schema.fieldLen; at += 1) {
    const column = schema.fieldAt(at)
    if (!column.nullable) required.push(column.name)
  }
  assert.deepEqual(required, [
    'updatedat', 'createdat', 'msghash', 'msgphash', 'code', 'beginstring', 'sendingtime',
  ])

  const reader = fixedCodec(registry)
  const message = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|9999=x|10=0|')).next().value
  const native = message.intoRow(schema)
  const row = native.toJSON()
  assert.equal(row.length, schema.fieldLen)
  assert.equal(row[schema.indexOf('beginstring')], 'FIX.4.4')
  assert.equal(row[schema.indexOf('msgtype')], 'D')
  assert.equal(row[schema.indexOf('symbol')], 'AAPL')
  assert.equal(row[schema.indexOf('version')], '4.4')
  assert.equal(row[schema.indexOf('sendercompid')], null, 'no sender, not a shift')
  // A message with no clock is never stamped with the epoch: it settles the
  // codec's default SendingTime and every replay clock follows that one
  // instant. A read is not a snapshot, so that column stays empty.
  for (const name of ['sendingtime', 'updatedat', 'createdat']) {
    assert.ok(native.at(schema.indexOf(name)).equals(SENDING), name)
  }
  assert.equal(native.at(schema.indexOf('snapshotat')).kind, 'null')
  assert.ok(message.updatedat().equals(message.getByTag(65003)))
  assert.ok(message.updatedat().equals(SENDING))
  assert.equal(native.at(schema.indexOf('msghash')).id, 'fixed_size_binary')
  // The row's msghash names the row's own content: padding and derived columns
  // may move it (decision 26), the row read back verifies it, and projection
  // leaves the message's own msghash alone. The chain name, msgphash, keeps its code.
  const before = message.msghash()
  const replayed = fix.FixMsg.fromRow(schema, native, registry)
  assert.ok(native.at(schema.indexOf('msghash')).equals(replayed.msghash()))
  assert.ok(message.msghash().equals(before))
  assert.ok(native.at(schema.indexOf('msgphash')).equals(message.msgphash()))
  assert.equal(native.at(schema.indexOf('code')).asJs(), '')
  // The arrival record closes the row, all of it in arrival order, and a key
  // no dictionary explains is still there, recorded at tag 0 under its raw key.
  const entries = native.at(schema.fieldLen - 1)
  assert.equal(entries.length, 5)
  const unresolved = []
  for (let at = 0; at < entries.length; at += 1) {
    const entry = entries.at(at)
    if (entry.at(0).asJs() === 0) unresolved.push(entry.at(1).asJs())
  }
  assert.deepEqual(unresolved, ['9999'])
  assert.deepEqual(message.arrivals().map(([tag, key]) => [tag, key]), [[8, '8'], [35, '35'], [55, '55'], [0, '9999'], [10, '10']])
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
  const row = new fix.FixCodec(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10=0|')).next().value.intoRow(carried).toJSON()
  assert.equal(row[0], null)
  assert.equal(row[carried.indexOf('msgtype')], 'D')

  // A capture column whose folded name a FIX column takes is not carried in
  // front: `senderSessionId` and `sendersessionid` are one name, and the FIX
  // column is the one a reader spelling it means - so the bracket's session
  // instance reaches that column instead of riding in front. `threadId` names
  // no FIX column, so it is carried and leads the row.
  const stamped = fields.struct(
    'line',
    [
      fields.utf8('url', { nullable: false }),
      fields.binary('body', { nullable: false }),
      fields.utf8('threadId'),
      fields.utf8('senderSessionId'),
    ],
    { nullable: false },
  )
  const folded = fix.schemaCarrying(stamped, plain)
  assert.equal(folded.fieldLen, plain.fieldLen + 3)
  assert.equal(folded.indexOf('senderSessionId'), null)
  assert.equal(folded.indexOf('threadId'), 2)
  assert.equal(folded.indexOf('sendersessionid'), plain.indexOf('sendersessionid') + 3)
})

test('the crate fields declare their own protocols', () => {
  const held = fix.crateFields()
  // Thirty-four scalar fields, the altids group and the instids struct
  // (`rust/tests/fix/digest.rs`).
  assert.equal(held.length, CRATED + 2)
  assert.equal(held.filter((field) => field.fix.counter === null).length, CRATED + 1)
  assert.deepEqual(
    held.map((field) => field.name),
    [
      'version',
      'symbolticker',
      'updatedat',
      'parentclordid',
      'parentorderid',
      'sendersessionid',
      'msgctxid',
      'pluginid',
      'prevpluginid',
      'sendersessionname',
      'targetsessionname',
      'isincode',
      'miccode',
      'state',
      'msghash',
      'msgphash',
      'targetsessionid',
      'altids',
      'prevupdatedat',
      'prevmsghash',
      'createdat',
      'code',
      'snapshotat',
      'sourceurl',
      'nofixentries',
      'recordedat',
      'expiredat',
      'bidcurrency',
      'offercurrency',
      'bridgesessionid',
      'bloombergcode',
      'cusipcode',
      'sedolcode',
      'instids',
      'sessionmsgid',
      'sessionmsgseqid',
    ],
  )
  assert.deepEqual(
    held.map((field) => field.display),
    [
      'Version',
      'SymbolTicker',
      'UpdatedAt',
      'ParentClOrdID',
      'ParentOrderID',
      'SenderSessionId',
      'MsgCtxId',
      'PluginId',
      'PrevPluginId',
      'SenderSessionName',
      'TargetSessionName',
      'ISINCode',
      'MICCode',
      'State',
      'MsgHash',
      'MsgPHash',
      'TargetSessionId',
      'AltIds',
      'PrevUpdatedAt',
      'PrevMsgHash',
      'CreatedAt',
      'Code',
      'SnapshotAt',
      'SourceUrl',
      'NoFixEntries',
      'RecordedAt',
      'ExpiredAt',
      'BidCurrency',
      'OfferCurrency',
      'BridgeSessionId',
      'BloombergCode',
      'CUSIPCode',
      'SEDOLCode',
      'InstIds',
      'SessionMsgId',
      'SessionMsgSeqId',
    ],
  )
  // In tag order from 65001 up: above every tag FIX or a venue publishes, so
  // they collide with nothing a dictionary declares and belong to none. The
  // retired 65000, 65004 and 65016 are not reused.
  assert.deepEqual(
    held.map((field) => field.fix.tag),
    [
      ...Array.from({ length: 3 }, (_, at) => 65001 + at),
      ...Array.from({ length: 11 }, (_, at) => 65005 + at),
      ...Array.from({ length: 22 }, (_, at) => 65017 + at),
    ],
  )
  assert.ok(
    held.every((field) => ![65000, 65004, 65016].includes(field.fix.tag)),
  )
  assert.ok(held.every((field) => field.fix.branches.length === 0))
  assert.ok(held.every((field) => Number.isInteger(field.fix.id)))
  const mapping = held[held.findIndex((field) => field.name === 'altids')]
  assert.equal(mapping.name, 'altids')
  assert.equal(mapping.fix.counter, 65020)
  assert.equal(mapping.nullable, true)
  assert.match(mapping.dtype.toString(), /keys_sorted=true/)
  for (const registry of [new fix.FixRegistry(), seed()]) {
    assert.ok(registry.definition('groups', 'altids').equals(mapping))
    assert.ok(registry.groupByTag(65020).equals(mapping))
    assert.equal(registry.getFieldByTag(65020), null)
    assert.equal(registry.getFieldByName('altids'), null)
    assert.equal(registry.getFieldById(mapping.fix.id), null)
    // The retired definitions are gone rather than aliased. `msghash` is a
    // live name again - on 65017, not the 65000 it once had.
    for (const retired of ['instid', 'id', 'persistentid', 'timestamp', 'uuid', 'puuid', 'prevuuid', 'instuuid']) {
      assert.equal(registry.getFieldByName(retired), null, retired)
    }
    assert.equal(registry.getFieldByTag(65000), null)
  }

  // The settled message facts are never null: the update, creation and
  // event clocks, the two identities and the code. Everything else is.
  const required = held.filter((field) => !field.nullable).map((field) => field.name)
  assert.deepEqual(required, ['updatedat', 'msghash', 'msgphash', 'createdat', 'code'])
  const named = (name) => held[held.findIndex((field) => field.name === name)]
  // `snapshotat` is the one the bundle holds without requiring: only a
  // snapshot stamps it, so a row no snapshot was taken of leaves it empty.
  assert.equal(named('snapshotat').nullable, true)
  const clock = named('updatedat').dtype
  for (const name of ['prevupdatedat', 'createdat', 'snapshotat', 'recordedat', 'expiredat']) {
    assert.ok(named(name).dtype.equals(clock), name)
  }
  assert.ok(named('prevmsghash').dtype.equals(DataType.fixedSizeBinary(16)))
  assert.ok(named('code').dtype.equals(DataType.from('utf8')))
  assert.ok(held.every((field) => field.description))
  // No crate field holds a digest any more: msghash is the one stored message
  // identity, and the arrival digest stays the message's own `digest()`.
  assert.ok(held.every((field) => field.getProperty('digest', 'role') === null))

  // No partition column: how a layout is cut is the target's to decide, and
  // an Iceberg table takes an `hour` transform over `updatedat`.
  assert.ok(held.every((field) => field.name !== 'timepartition'))
  assert.ok(held.every((field) => !field.isPartition))
  // Every identifier the instrument is known by is one Struct beside the
  // columns it reads.
  const instids = named('instids')
  assert.deepEqual(
    Array.from({ length: instids.fieldLen }, (_, at) => instids.getFieldAt(at).name),
    ['cficode', 'isincode', 'bloombergcode', 'cusipcode', 'sedolcode'],
  )
})

test("the bridge's six facts are crate fields, and every registry holds them", () => {
  // What a bridge's own log states about a line: the session the message
  // itself belongs to, the message context it was handled under, the plugin
  // that logged it and the one it came through before that, and the two
  // session names the line spells.
  const names = fix.crateFields().map((field) => field.name)
  const from = names.indexOf('sendersessionid')
  const held = fix.crateFields().slice(from, from + 6)
  assert.deepEqual(
    held.map((field) => [field.name, field.display, field.fix.tag]),
    [
      ['sendersessionid', 'SenderSessionId', 65007],
      ['msgctxid', 'MsgCtxId', 65008],
      ['pluginid', 'PluginId', 65009],
      ['prevpluginid', 'PrevPluginId', 65010],
      ['sendersessionname', 'SenderSessionName', 65011],
      ['targetsessionname', 'TargetSessionName', 65012],
    ],
  )
  assert.ok(held.every((field) => field.dtype.equals(DataType.from('utf8'))))
  assert.ok(held.every((field) => field.nullable))
  assert.ok(held.every((field) => field.description))
  // The session names answer to the spellings a bridge row writes them
  // under, so `ULFROMSESSIONNAME=` lands on the sender's session by name.
  assert.deepEqual(
    held.slice(4).map((field) => field.fix.aliases),
    [['ULFromSessionName'], ['ULToSessionName']],
  )

  // And the three facts a row derives from what the message said, typed as
  // the thing they hold rather than as the text a venue spelled it in: the
  // state is ten bytes, two digits of rank then the name, as `40PARTFILL`.
  const at = (name) => names.indexOf(name)
  const derived = fix.crateFields().slice(at('isincode'), at('isincode') + 3)
  assert.deepEqual(
    derived.map((field) => [field.name, field.display, field.fix.tag, field.dtype.toString()]),
    [
      ['isincode', 'ISINCode', 65013, 'isin'],
      ['miccode', 'MICCode', 65014, 'mic'],
      ['state', 'State', 65015, 'state'],
    ],
  )
  assert.equal(derived[2].dtype.codeWidth, 10)

  // And the two identities that sit together - the message's time/content
  // identity and the event chain's - sixteen plain bytes, which is what every
  // lake engine reads as `fixed[16]`. Both are settled on every message, so
  // neither is ever null; the chain's `prevmsghash` beside them may be.
  const identities = fix.crateFields().slice(at('msghash'), at('msghash') + 2)
  assert.deepEqual(
    identities.map((field) => [field.name, field.display, field.fix.tag, field.dtype.toString(), field.nullable]),
    [
      ['msghash', 'MsgHash', 65017, 'fixed_size_binary(16)', false],
      ['msgphash', 'MsgPHash', 65018, 'fixed_size_binary(16)', false],
    ],
  )
  assert.ok(identities.every((field) => field.description))
  assert.ok(identities.every((field) => field.fix.aliases.length === 0))

  // A new registry, a loaded one and a built one answer them alike, by the
  // identifier the listed field derives on its own, by name, and by the bare
  // tag or name.
  for (const registry of [
    new fix.FixRegistry(),
    seed(),
    fix.FixRegistry.fromFields([fixField('Symbol', 'utf8', 55)]),
  ]) {
    assert.equal(registry.fieldById(held[0].fix.id).name, 'sendersessionid')
    assert.equal(registry.fieldByName('SenderSessionId').fix.tag, 65007)
    assert.equal(registry.fieldByTag(65012).name, 'targetsessionname')
    assert.equal(registry.field('msgctxid').fix.id, held[1].fix.id)
    assert.equal(registry.has('pluginid'), true)
    assert.equal(registry.fieldByName('ULToSessionName').name, 'targetsessionname')
  }

  // And each is a column of the fixed row, typed by the crate's own
  // definition and spelled by its folded name; only the settled identities
  // are required there.
  const schema = fix.schema(seed(), 'FixMessage')
  for (const field of [...held, ...derived, ...identities]) {
    const at = schema.indexOf(field.name)
    assert.notEqual(at, null, field.name)
    assert.equal(schema.fieldAt(at).fix.id, field.fix.id)
    assert.equal(schema.fieldAt(at).display, field.display)
    assert.equal(schema.fieldAt(at).nullable, !['msghash', 'msgphash'].includes(field.name), field.name)
  }
})

test('a message says everything the core derives about it', () => {
  const registry = seed()
  const reader = new fix.FixCodec(registry)
  const message = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|207=XNAS|54=1|44=10.5|38=100|60=20240201-12:34:56|10=0|')).next().value

  assert.equal(message.symbolTicker().toJSON(), 'AAPL@XNAS')
  assert.equal('marketTimestamp' in message, false, 'the retired reader is gone')
  assert.equal('timePartition' in message, false, 'how a layout is cut is the target\'s')
  // The settled clocks are never null. TransactTime is the event, so the
  // update and creation instants are that event. The SendingTime the line
  // did not state closes the root.
  assert.equal([...message].at(-1)[0], 'sendingtime')
  for (const held of [message.updatedat(), message.createdat(), message.msghash(), message.msgphash()]) {
    assert.ok(held instanceof Scalar)
    assert.notEqual(held.kind, 'null')
  }
  assert.ok(message.updatedat().equals(message.getByTag(65003)))
  assert.ok(message.updatedat().equals(message.byTag(60)))
  assert.ok(message.createdat().equals(message.byTag(60)))
  // A read is not a snapshot, so that column stays empty.
  assert.equal(message.byTag(65025).kind, 'null')
  assert.ok(message.msghash().equals(message.byTag(65017)))
  assert.ok(message.msgphash().equals(message.byTag(65018)))
  assert.equal(message.msghash().id, 'fixed_size_binary')
  assert.equal(message.msgphash().id, 'fixed_size_binary')
  // Sixteen bytes reach JavaScript as a Buffer, not as hyphenated text.
  assert.equal(Buffer.from(message.msghash().asJs()).length, 16)
  // A row derives the market from the first MIC the message names, and
  // leaves the ISIN and the state null when it stated no source for either.
  const schema = fix.schema(registry, 'FixMessage')
  const row = message.intoRow(schema).toJSON()
  assert.equal(row[schema.indexOf('miccode')], 'XNAS')
  assert.equal(row[schema.indexOf('isincode')], null)
  assert.equal(row[schema.indexOf('state')], null)
  // A buy order at a price is a party willing to pay it, so the bid lane it
  // never wrote is still true of it.
  assert.equal(message.lifted('bidpx').toJSON(), 10.5)
  // A lift source is the tag the facet was read from.
  assert.equal(message.liftSource('bidpx'), 44)
  assert.equal(message.liftSource('nope'), null)
  assert.ok(message.lift().length > 0)
  assert.equal(message.digest().length, 16)
  // An arrival states the tag its key named.
  assert.equal(message.arrivals()[0][0], 8)
  assert.equal(message.intoBytes(124).toString().split('|')[0], '8=FIX.4.4')
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
  // A definition is filed by the shape it has, so each is looked up through
  // the door its shape put it behind.
  assert.ok(fix.crateFields().every((field) => {
    let declared
    if (field.fix.counter !== null) declared = registry.groupByTag(field.fix.counter)
    else if (field.fieldLen > 0) declared = registry.definition('components', field.name)
    else declared = registry.fieldByTag(field.fix.tag)
    return !declared.fix.hasBranch('bloomberg')
  }))

  // Membership is provenance: the codec reads the one namespace with no pin
  // and the venue's field resolves like any other.
  const message = new fix.FixCodec(registry).parseLine(Buffer.from('8=FIX.4.4|35=D|10001=NONE|10=0|')).next().value
  assert.equal(message.byName('ExcludedDealers').asJs(), 'NONE')
  assert.equal(message.byTag(10001).asJs(), 'NONE')
  assert.ok(message.arrivals().every(([tag]) => Number.isInteger(tag)))

  // With no dialect named nothing is stamped, and a name that is empty or
  // carries the separator is refused before anything is read.
  const [unstamped] = fix.FixRegistry.fromCfbFile(file)
  assert.deepEqual(unstamped.fieldByTag(10001).fix.branches, [])
  assert.deepEqual(unstamped.dialects(), [])
  assert.equal(fix.FixRegistry.fromCfbFile(file, null)[0].dialects().length, 0)
  assert.throws(() => fix.FixRegistry.fromCfbFile(file, 'a,b'), /fix:branches/)
  assert.throws(() => fix.FixRegistry.fromCfbFile(file, ''), /fix:branches/)
})

test('generated identifier membership fills native sorted altids without changing the record', () => {
  const registry = seed()
  assert.deepEqual(registry.msgtype('D').asField().fix.identifiers, [
    'clordid', 'secondaryclordid', 'allocid', 'quoteid', 'reforderid', 'refclordid',
  ])
  assert.deepEqual(registry.msgtype('8').asField().fix.identifiers, [
    'orderid', 'secondaryorderid', 'secondaryclordid', 'secondaryexecid',
    'clordid', 'origclordid', 'quoterespid', 'listid', 'execid', 'execrefid',
    'allocid', 'reforderid', 'refclordid',
  ])
  const codec = new fix.FixCodec(registry)
  const wire = Buffer.from('8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|')
  const original = codec.parseFixLine(wire)
  const filled = codec.enrichMessage(original)
  const expected = new Map([['clordid', 'C-001'], ['execid', 'E-09'], ['orderid', 'O-01']])
  assert.equal(filled.byTag(65020).kind, 'mapping')
  assert.ok(filled.byName('AltIds').asJs() instanceof Map)
  assert.deepEqual(filled.byTag(65020).asJs(), expected)
  assert.deepEqual([...filled.byName('AltIds').asJs().keys()], [...expected.keys()])
  assert.deepEqual(filled.arrivals(), original.arrivals())
  assert.deepEqual(filled.intoBytes(124), wire)
  assert.deepEqual(filled.digest(), original.digest())
  assert.ok(codec.enrichMessage(filled).equals(filled))
  const schema = fix.schema(registry)
  const restored = fix.FixMsg.fromRow(schema, filled.intoRow(schema), registry)
  assert.ok(restored.byName('altids').equals(filled.byName('altids')))
  assert.deepEqual(restored.arrivals(), filled.arrivals())
  const stamped = [...codec.lifecycle([filled])]
  assert.equal(stamped.length, 1)
  assert.deepEqual(stamped[0].byName('altids').asJs(), expected)
  assert.deepEqual(stamped[0].arrivals(), filled.arrivals())
  assert.deepEqual(stamped[0].digest(), filled.digest())
})

test('a stated altids Map is preserved even when it is empty', () => {
  const codec = new fix.FixCodec(seed())
  for (const stated of [new Map(), new Map([['venue', '001']])]) {
    const value = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C-1|10=0|'))
    const before = [value.arrivals(), value.intoBytes(124), value.digest()]
    value.set('altids', stated)
    const filled = codec.enrichMessage(value)
    assert.deepEqual(filled.byTag(65020).asJs(), stated)
    assert.deepEqual([filled.arrivals(), filled.intoBytes(124), filled.digest()], before)
    assert.ok(codec.enrichMessage(filled).equals(filled))
  }
})

test('altids distinguishes known empty messages from unknown messages without flattening groups', () => {
  const codec = new fix.FixCodec(seed())
  for (const code of ['0', 'D']) {
    const filled = codec.enrichMessage(codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=${code}|10=0|`)))
    assert.deepEqual(filled.byTag(65020).asJs(), new Map())
  }
  const unknown = codec.enrichMessage(codec.parseFixLine(Buffer.from('8=FIX.4.4|35=ZZ|11=C-1|10=0|')))
  assert.equal(unknown.getByTag(65020), null)
  const nested = codec.enrichMessage(codec.parseLine(Buffer.from(
    'MSGTYPE=E|#LISTID=L-1|#NOORDERS=1|#NOORDERS[0]=CLORDID=C-nested\x04\x03SYMBOL=EXAMPLE',
  )).next().value)
  assert.deepEqual(nested.byTag(65020).asJs(), new Map([['listid', 'L-1']]))
})

test('a message type keeps its complete wire code and immutable schema', () => {
  const registry = new fix.FixRegistry()
  registry.insert(fixField('msgtype', 'utf8', 35))
  const value = registry.registerMsgtype('P Report Ack', 'AllocationReportAck', 'Allocation Report ACK')
  assert.equal(value.asStr(), 'P Report Ack')
  assert.equal(value.name, 'allocationreportack')
  assert.match(registry.fieldByTag(35).get('fix:codes'), /"name":"AllocationReportAck"/)
  assert.match(registry.fieldByTag(35).get('fix:codes'), /P Report Ack/)
  assert.equal(value.asField().fix.msgtype, 'P Report Ack')
  assert.throws(() => registry.registerMsgtype('X'), /shared/)
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

// Decision 28: one arrival record, with tag 0 reserved for what the dictionary
// did not resolve. Parity with `rust/tests/fix/zero_entries.rs`.

test('an unresolved arrival keeps tag 0, its raw key, its value and its place', () => {
  const codec = fixedCodec(seed())
  const wire = '35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|'
  const messages = codec.parseLine(Buffer.from(wire))
  const message = messages.next().value
  assert.equal(messages.next().done, true)
  // A name and a number the dictionary does not define are both unresolved,
  // and so are a zero and a number too wide to be a tag; the known keys keep
  // their canonical positive tags.
  const arrivals = message.arrivals()
  assert.equal(arrivals.length, 8)
  assert.deepEqual(arrivals.map(([tag]) => tag), [35, 0, 0, 0, 0, 0, 55, 10])
  const unresolved = [
    ['999999', 'one'],
    ['0999999', 'two'],
    ['OwnThing', 'three'],
    ['0', 'zero'],
    ['2147483648', 'wide'],
  ]
  assert.deepEqual(arrivals.slice(1, 6).map(([, key, value]) => [key, value]), unresolved)
  // Each is still a dynamic column under its raw name, and the numeric one
  // is reached by its number too.
  for (const [key, value] of unresolved) assert.equal(message.getByName(key).asJs(), value, key)
  assert.equal(message.getByTag(999_999).asJs(), 'one')
  assert.equal(message.intoBytes(124).toString(), wire)

  // A value that is not text is still recorded at tag 0 under its raw key,
  // reported as the lossy decode it is, and re-emitted byte for byte.
  const raw = Buffer.concat([Buffer.from('35=D|999999='), Buffer.from([0xff]), Buffer.from('|10=0|')])
  const lossy = codec.parseLine(raw).next().value
  assert.deepEqual(lossy.arrivals().map(([tag, key]) => [tag, key]), [[35, '35'], [0, '999999'], [10, '10']])
  assert.equal(lossy.anomalies()[0], '999999 (0) reaches the row as a lossy decode')
  assert.deepEqual(lossy.intoBytes(124), raw)

  // Signed keys are not tags either: native pairs reach the builder directly,
  // and neither sign becomes a numeric tag.
  const signed = codec.parsePairs([['-1', 'negative'], ['+35', 'signed']])
  assert.deepEqual(signed.arrivals(), [[0, '-1', 'negative'], [0, '+35', 'signed']])
  assert.equal(signed.getByName('-1').asJs(), 'negative')
  assert.equal(signed.getByName('+35').asJs(), 'signed')
  assert.equal(signed.intoBytes(124).toString(), '-1=negative|+35=signed|')

  // Indexed unknowns keep each arrival and their existing value shape.
  const indexed = codec.parsePairs([['999999[0]', 'first'], ['999999[2]', 'third']])
  assert.deepEqual(indexed.arrivals(), [[0, '999999[0]', 'first'], [0, '999999[2]', 'third']])
  assert.deepEqual(indexed.getByName('999999').asJs(), ['first', null, 'third'])

  // The intrinsic altids Map has no numeric scalar wire grammar.
  const opaque = codec.parseLine(Buffer.from('35=D|65020=opaque|10=0|')).next().value
  assert.deepEqual(opaque.arrivals()[1], [0, '65020', 'opaque'])
  assert.equal(opaque.getByName('65020').asJs(), 'opaque')
  assert.equal(opaque.getByName('altids'), null)
  assert.equal(opaque.intoBytes(124).toString(), '35=D|65020=opaque|10=0|')

  // Every unresolved key is hashed under its own raw key in the zero-tag
  // frame, so two of them never share an arrival digest.
  const digests = ['999999', '0999999', '999998', 'OwnThing'].map((key) => {
    const one = codec.parsePairs([[key, 'x']])
    assert.deepEqual(one.arrivals(), [[0, key, 'x']])
    return one.digest().toString('hex')
  })
  assert.equal(new Set(digests).size, 4)
})

test('unresolved counters keep their members in arrival order under tag 0', () => {
  const codec = fixedCodec(seed())
  // At the top: an unregistered numeric counter heads what arrived under it.
  const top = codec.parsePairs([
    ['999999', '2'],
    ['999999[0].OwnThing', 'a'],
    ['999999[1].999998', 'b'],
  ])
  assert.deepEqual(top.arrivals(), [
    [0, '999999', '2'],
    [0, '999999[0].OwnThing', 'a'],
    [0, '999999[1].999998', 'b'],
  ])
  assert.equal(top.intoBytes(124).toString(), '999999=2|999999[0].OwnThing=a|999999[1].999998=b|')

  // Inside a resolved group: the unresolved sub-counter keeps its member, and
  // the resolved member after it still follows it on the wire.
  const nested = codec.parsePairs([
    ['NoPartyIDs', '1'],
    ['NoPartyIDs[0].999999', '1'],
    ['NoPartyIDs[0].999999[0].999998', 'A'],
    ['NoPartyIDs[0].PartyRole', '3'],
  ])
  assert.deepEqual(nested.arrivals(), [
    [453, 'NoPartyIDs', '1'],
    [0, 'NoPartyIDs[0].999999', '1'],
    [0, 'NoPartyIDs[0].999999[0].999998', 'A'],
    [452, 'NoPartyIDs[0].PartyRole', '3'],
  ])
  assert.equal(
    nested.intoBytes(124).toString(),
    'NoPartyIDs=1|NoPartyIDs[0].999999=1|NoPartyIDs[0].999999[0].999998=A|NoPartyIDs[0].PartyRole=3|',
  )

  // A declared group keeps its resolved member and the unknown children
  // beside it under the counter it stated.
  const registry = new fix.FixRegistry()
  registry.insert(fixField('norows', 'int32', 90_001))
  const group = fields.list('rows', fields.struct('row', [fixField('scopedvalue', 'utf8', 90_002)], { nullable: false }))
  group.fix.counter = 90_001
  registry.createDefinition('groups', group)
  assert.equal(registry.getFieldByTag(90_002), null)
  const rows = fixedCodec(registry).parsePairs([
    ['NoRows', '1'],
    ['Rows[0].ScopedValue', 'known'],
    ['Rows[0].999999', 'numeric'],
    ['Rows[0].OwnThing', 'named'],
  ])
  assert.deepEqual(rows.arrivals(), [
    [90_001, 'NoRows', '1'],
    [90_002, 'Rows[0].ScopedValue', 'known'],
    [0, 'Rows[0].999999', 'numeric'],
    [0, 'Rows[0].OwnThing', 'named'],
  ])
  assert.equal(rows.byPath('rows[0].scopedvalue').asJs(), 'known')
  assert.equal(rows.byPath('rows[0]."999999"').asJs(), 'numeric')
  assert.equal(rows.byPath('rows[0].ownthing').asJs(), 'named')
  assert.equal(
    rows.intoBytes(124).toString(),
    'NoRows=1|Rows[0].ScopedValue=known|Rows[0].999999=numeric|Rows[0].OwnThing=named|',
  )
})

test('numeric and named aliases keep the canonical positive arrival tag', () => {
  const registry = seed()
  const symbol = registry.fieldByTag(55)
  symbol.fix.tags = [9_000_001]
  symbol.fix.aliases = ['SyntheticSymbol']
  registry.insert(symbol)
  const codec = fixedCodec(registry)
  const canonical = codec.parseLine(Buffer.from('35=D|55=SYNTH|10=0|')).next().value
  for (const key of ['55', '00055', '9000001', 'SyntheticSymbol']) {
    const wire = `35=D|${key}=SYNTH|10=0|`
    const message = codec.parseLine(Buffer.from(wire)).next().value
    assert.deepEqual(message.arrivals()[1], [55, key, 'SYNTH'], key)
    assert.deepEqual(message.digest(), canonical.digest(), key)
    assert.equal(message.intoBytes(124).toString(), wire)
  }

  // The envelope exclusion reads resolved tags only: a dictionary defining
  // MsgSeqNum leaves 34 out of the digest, a bare registry hashes it as an
  // unresolved arrival under its raw key.
  const digest = (dictionary, sequence) =>
    fixedCodec(dictionary).parsePairs([['34', sequence], ['11', 'A']]).digest()
  const committed = seed()
  assert.deepEqual(digest(committed, '7'), digest(committed, '8'))
  const bare = new fix.FixRegistry()
  assert.notDeepEqual(digest(bare, '7'), digest(bare, '8'))
})
