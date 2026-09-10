'use strict'

// The FIX boundary: the typed vocabulary on the protocol view, the registry,
// and the message. Every answer here is the core's; what these check is the
// crossing - the key coercion, the tag width, the error class each refusal
// arrives as, the storage locations a JavaScript caller names, and the
// language protocols the loader wires over the native halves.
//
// A branch and an identifier cross as strings and are parsed once at the
// boundary, so there is no class for either and every refusal is the native
// one.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { DataType, Field, IOBase, MimeType, Scalar, Url, fields, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')

// What every registry holds before anything is inserted: the crate's own
// twenty fields, standard fields from tag 65000 up, which
// `new fix.FixRegistry()` seeds and `fix.crateFields()` lists.
const CRATED = 20
// The first tag the crate claims; every tag from it up is one of its own.
const CRATE_TAG_MIN = 65000

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
}

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

function fixField(name, dtype, tag, { branch = fix.STANDARD_BRANCH, tags, aliases, description } = {}) {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.id = `${tag}:${branch}`
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
    assert.throws(() => view.description, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.branch, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => view.id, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.tag = 55
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.aliases = ['Ticker']
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.branch = 'cme'
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.id = '5001:cme'
    }, { name: 'TypeError', message: new RegExp(scheme) })
  }
  // The Map-like surface still works on every view, this one included.
  assert.equal(field.protocol('fix').get('tag'), '55')
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

  // The core's own refusals arrive with the full key in the message.
  assert.throws(() => {
    field.fix.tag = -1
  }, /fix:tag/)
  assert.throws(() => {
    field.fix.tags = [55, 55]
  }, /fix:tags/)
  assert.throws(() => {
    field.fix.aliases = ['Sym', 'sym']
  }, /fix:aliases/)
  assert.equal(field.fix.tag, null)
})

test('the branch and the identifier round trip as text', () => {
  const trade = Field.from('TradeID: utf8')
  // An absent property is the standard branch, and there is no identity
  // without a tag.
  assert.equal(trade.fix.branch, fix.STANDARD_BRANCH)
  assert.equal(fix.STANDARD_BRANCH, '')
  assert.equal(trade.fix.id, null)
  assert.equal(trade.has('fix:branch'), false)

  trade.fix.id = '5001:CME'
  assert.equal(trade.fix.id, '5001:cme', 'ASCII case folded once, on the way in')
  assert.equal(trade.fix.branch, 'cme')
  assert.equal(trade.get('fix:branch'), 'cme')
  assert.equal(trade.fix.tag, 5001)

  // Setting the standard branch removes the key rather than storing it.
  trade.fix.branch = ''
  assert.equal(trade.fix.branch, '')
  assert.equal(trade.has('fix:branch'), false)
  assert.equal(trade.fix.id, '5001:')

  // Assigning an identifier moves both halves at once, in either direction.
  trade.fix.id = '5002:cme'
  assert.equal(trade.fix.id, '5002:cme')
  trade.fix.id = '35:'
  assert.equal(trade.fix.id, '35:')
  assert.equal(trade.has('fix:branch'), false)

  // The branch alone still moves a field whose tags allow it.
  const vendor = Field.from('VendorID: utf8')
  vendor.fix.tag = 9001
  vendor.fix.branch = 'cme'
  assert.equal(vendor.fix.id, '9001:cme')
})

test('a malformed branch or identifier is the native parse failure', () => {
  const field = Field.from('TradeID: utf8')

  for (const bad of ['2cme', 'cme:x', 'c,me', 'a'.repeat(24)]) {
    assert.throws(() => {
      field.fix.branch = bad
    }, /fix branch/)
  }
  for (const bad of ['5001', '+5001:cme', '-1:cme', ':cme', '5001:2cme']) {
    assert.throws(() => {
      field.fix.id = bad
    }, /fix identifier|fix branch/i)
  }
  // Nothing was written by any refusal.
  assert.equal(field.fix.branch, '')
  assert.equal(field.fix.id, null)

  // A branch and an identifier are text, never a number.
  assert.throws(() => {
    field.fix.branch = 5001
  }, /into rust type `String`/)
  assert.throws(() => {
    field.fix.id = 5001
  }, /into rust type `String`/)
})

test('a specification tag forces the standard branch at every door', () => {
  assert.equal(fix.USER_TAG_MIN, 5000)
  assert.equal(fix.USER_TAG_MAX, 40_000)

  // A canonical tag: another branch may not claim it.
  const vendor = Field.from('TradeID: utf8')
  vendor.fix.id = '5001:cme'
  assert.throws(() => {
    vendor.fix.tag = 35
  }, /fix:branch/)
  assert.equal(vendor.fix.id, '5001:cme')
  assert.throws(() => {
    vendor.fix.id = '35:cme'
  }, /fix:branch/)
  assert.equal(vendor.fix.id, '5001:cme')

  // An alternate tag resolves with the same power, so it obeys the same rule.
  assert.throws(() => {
    vendor.fix.tags = [35]
  }, /fix:branch/)
  assert.deepEqual(vendor.fix.tags, [])
  assert.equal(vendor.fix.id, '5001:cme')

  // A branch change is refused against the tags the field already holds.
  const msgType = Field.from('MsgType: utf8')
  msgType.fix.tag = 35
  assert.throws(() => {
    msgType.fix.branch = 'cme'
  }, /fix:branch/)
  assert.equal(msgType.fix.branch, '')
  assert.equal(msgType.fix.id, '35:')

  const alternates = Field.from('Wide: utf8')
  alternates.fix.tag = 9001
  alternates.fix.tags = [35]
  assert.throws(() => {
    alternates.fix.branch = 'cme'
  }, /fix:branch/)
  assert.equal(alternates.fix.branch, '')

  // The rule is one-way: the standard branch holds any tag.
  const high = Field.from('Vendorish: utf8')
  high.fix.tag = 10_000
  assert.equal(high.fix.id, '10000:')

  for (const admitted of [fix.USER_TAG_MIN, fix.USER_TAG_MAX - 1]) {
    const field = Field.from('Venue: utf8')
    field.fix.id = `${admitted}:cme`
    assert.equal(field.fix.tag, admitted)
  }
  for (const refused of [fix.USER_TAG_MIN - 1, fix.USER_TAG_MAX]) {
    const field = Field.from('Venue: utf8')
    assert.throws(() => {
      field.fix.id = `${refused}:cme`
    }, /5000.*40000/)
  }
})

test('the registry resolves every key the way the core does', () => {
  const registry = seed()
  // The store's fields, and the crate's own beside them: a store never
  // writes those, so a loaded dictionary holds the crate's definition.
  assert.equal(registry.size, 6241 + CRATED)

  assert.equal(registry.fieldByTag(55).name, 'symbol')
  assert.equal(registry.getFieldByTag(55).name, 'symbol')
  assert.equal(registry.fieldById('55:').name, 'symbol')
  assert.ok(registry.getFieldById('55:').equals(registry.fieldByTag(55)))
  // The alternate tag 20 reaches ExecType, which claims 150 canonically.
  assert.equal(registry.fieldByTag(150).name, 'exectype')
  // `OrdStatus` and `ExecType` are the order's state, read as one type.
  assert.ok(registry.fieldByTag(39).dtype.equals(DataType.from('state')))
  assert.ok(registry.fieldByTag(150).dtype.equals(DataType.from('state')))
  // A name answers the canonical spelling whatever case it was asked in.
  assert.equal(registry.fieldByName('symbol', '').name, 'symbol')
  assert.equal(registry.fieldByName('SYMBOL', fix.STANDARD_BRANCH).name, 'symbol')
  assert.equal(registry.fieldByName('clordid', '').name, 'clordid')
  // A path reaches a repeating group and one of its members.
  assert.equal(registry.fieldByPath('NoPartyIDs', '').fix.tag, 453)
  // An occurrence is not a path segment: the walk steps through the list and
  // the member is spelled directly under the counter.
  assert.equal(registry.fieldByPath('parties.partyid', '').fix.tag, 448)
  assert.equal(registry.fieldByPath('parties.partyrole', '').name, 'partyrole')
  assert.equal(registry.getFieldByPath('parties.partyid.partyid', ''), null)

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
  assert.equal(registry.getFieldByPath('Symbol.absent', ''), null)
  assert.equal(registry.has('55'), false, 'a tag query never consults names')
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
      '{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:' +
        'name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"}',
      MimeType.ULCONFIG,
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

  // A bridge configuration states its own half of the exchange, and the
  // `send` its own payload spells is never read as the marker.
  const answered =
    '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",' +
    '"type":"read"},"value":{"name":"send-test-request"},"status":200}'
  assert.ok(MimeType.inferText(answered).equals(MimeType.ULCONFIG))
  assert.equal(MimeType.inferTextDirection(answered), 'RECV')
  assert.equal(fix.FixCodec.inferMsgtypeText(answered), 'read')
  const asked = '{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}'
  assert.equal(MimeType.inferTextDirection(asked), 'SENT')
})

test('an explicit branch pins lookup and omission infers the best match', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    // The venue dictionary reuses the name, which is the normal case.
    fixField('Symbol', 'utf8', 5055, { branch: 'cme', aliases: ['VenueTicker'] }),
    fixField('TradeID', 'utf8', 5001, { branch: 'cme' }),
  ])

  // A name is unique per branch, not registry-wide.
  assert.equal(registry.fieldByName('symbol', '').fix.id, '55:')
  assert.equal(registry.fieldByName('SYMBOL', 'cme').fix.id, '5055:cme')
  assert.equal(registry.fieldByName('venueticker', 'CME').name, 'Symbol')
  assert.equal(registry.getFieldByName('venueticker', ''), null)
  assert.equal(registry.getFieldByName('ticker', 'cme'), null)
  assert.equal(registry.getFieldByPath('Symbol', 'cme').fix.id, '5055:cme')

  // A bare tag uses the same deterministic best-match order.
  assert.equal(registry.getFieldByTag(5055).fix.id, '5055:cme')
  assert.equal(registry.getFieldByTag(5001).fix.id, '5001:cme')
  assert.equal(registry.has(5055), true)
  assert.equal(registry.fieldById('5055:cme').fix.id, '5055:cme')

  // A standard canonical name wins; a colon-bearing string is a name, never
  // an identifier.
  assert.equal(registry.getField('symbol').fix.id, '55:')
  assert.equal(registry.getField('5055:cme'), null)
  assert.equal(registry.has('5055:cme'), false)
  assert.equal(registry.get('5001:cme'), null)
  assert.equal(registry.remove('5055:cme'), null)
  assert.equal(registry.size, 3 + CRATED)
})

test('removeById is how a vendor field leaves the dictionary', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('TradeID', 'utf8', 5001, { branch: 'cme', aliases: ['VenueTrade'] }),
    fixField('VenueQty', 'int64', 5002, { branch: 'cme' }),
  ])

  // The generic `remove` cannot reach another branch at all.
  assert.equal(registry.remove('TradeID'), null)
  assert.equal(registry.remove(5001), null)
  assert.equal(registry.size, 3 + CRATED)

  const removed = registry.removeById('5001:CME')
  assert.equal(removed.name, 'TradeID')
  assert.equal(registry.size, 2 + CRATED)
  assert.equal(registry.getFieldById('5001:cme'), null)
  assert.equal(registry.getFieldByName('venuetrade', 'cme'), null)
  // A field that is not there answers null rather than throwing.
  assert.equal(registry.removeById('5001:cme'), null)
  assert.equal(registry.removeById('9999:'), null)
  // And the standard branch is reached by identifier just as well.
  assert.equal(registry.removeById('55:').name, 'Symbol')
  assert.equal(registry.size, 1 + CRATED)

  // A malformed identifier is the native parse failure, never a miss.
  assert.throws(() => registry.removeById('5002'), /fix identifier/)
  assert.throws(() => registry.removeById('35:cme'), /fix:branch/)
  assert.throws(() => registry.removeById(5002), /into rust type `String`/)
  assert.equal(registry.size, 1 + CRATED)
})

test('absence throws with the core message, its get twin answers null', () => {
  const registry = seed()

  assert.throws(
    () => registry.fieldByTag(9999),
    /^Error: expected a fix field at "tag 9999", got nothing$/,
  )
  assert.throws(
    () => registry.fieldById('5001:cme'),
    /^Error: expected a fix field at "identifier 5001:#[0-9a-f]{8}", got nothing$/,
  )
  assert.throws(() => registry.fieldByName('Nope', ''), /name \\"Nope\\"/)
  assert.throws(() => registry.fieldByPath('Symbol.absent', ''), /path \\"Symbol.absent\\"/)
  assert.throws(() => registry.field(9999), /tag 9999/)
  assert.equal(registry.getFieldByName('Nope', ''), null)
  assert.equal(registry.getFieldById('5001:cme'), null)
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
  assert.throws(() => registry.fieldByName('std', 55), /into rust type `String`/)
  assert.throws(() => registry.fieldByTag('55'), /into rust type `f64`/)
})

test('every branch and identifier argument is coerced at the boundary', () => {
  const registry = seed()

  for (const bad of ['2cme', 'c:me']) {
    assert.throws(() => registry.fieldByName('Symbol', bad), /fix branch/)
    assert.throws(() => registry.getFieldByName('Symbol', bad), /fix branch/)
    assert.throws(() => registry.fieldByPath('Symbol', bad), /fix branch/)
    assert.throws(() => registry.getFieldByPath('Symbol', bad), /fix branch/)
  }
  for (const bad of ['55', 'cme:', 'cme:x']) {
    assert.throws(() => registry.fieldById(bad), /fix identifier/)
    assert.throws(() => registry.getFieldById(bad), /fix identifier/)
    assert.throws(() => registry.removeById(bad), /fix identifier/)
  }
  // The standard-tag rule reaches the boundary through that same parse.
  assert.throws(() => registry.fieldById('35:cme'), /fix:branch/)

  for (const wrong of [55, null, 3.5]) {
    assert.throws(() => registry.fieldById(wrong), /into rust type `String`/)
  }
  for (const wrong of [55, 3.5]) {
    assert.throws(() => registry.getFieldByName('Symbol', wrong), /into rust type `String`/)
    assert.throws(() => registry.fieldByPath('std', wrong), /into rust type `String`/)
  }
  assert.equal(registry.getFieldByName('Symbol', null).name, 'symbol')
})

test('the registry iterates lazily in ascending identifier order', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55),
    fixField('TradeID', 'utf8', 5001, { branch: 'cme' }),
    fixField('Price', 'decimal128(20, 8)', 44),
    fixField('VenueQty', 'int64', 5002, { branch: 'cme' }),
    fixField('Account', 'utf8', 1),
    fixField('Tail', 'utf8', 9001),
  ])

  // Tag-major, then by branch digest - the identifier's own order. The
  // vendor fields therefore precede the later standard tag, and the crate's
  // own fields close the walk: standard fields whose tags sit above any a
  // test claims.
  const crated = fix.crateFields().map((field) => field.fix.id)
  assert.deepEqual(crated, Array.from({ length: CRATED }, (_, at) => `${CRATE_TAG_MIN + at}:`))
  assert.deepEqual(
    [...registry].map((field) => field.fix.id),
    ['1:', '44:', '55:', '5001:cme', '5002:cme', '9001:', ...crated],
  )
  assert.deepEqual(
    [...registry.keys()].map((field) => field.fix.id),
    [...registry].map((field) => field.fix.id),
  )

  // An unfinished walk shares the registry, so a mutation refuses until the
  // walk ends - by exhaustion or by the `return` a `break` sends.
  const walk = registry.keys()
  assert.equal(walk.next().value.name, 'Account')
  assert.equal(walk.next().value.name, 'Price')
  assert.throws(() => registry.remove(1), /shared with a message/)
  assert.throws(() => registry.removeById('5001:cme'), /shared with a message/)
  walk.return()
  assert.equal(registry.remove(1).name, 'Account')
  assert.deepEqual(
    [...registry].map((field) => field.fix.id),
    ['44:', '55:', '5001:cme', '5002:cme', '9001:', ...crated],
  )
})

test('the seed iterates in canonical-tag order and every field is standard', () => {
  const registry = seed()

  const names = [...registry].map((field) => field.name)
  assert.deepEqual(names.slice(0, 4), ['account', 'advid', 'advrefid', 'advside'])
  assert.equal(names.length, registry.size)

  const tags = [...registry].map((field) => field.fix.tag)
  assert.deepEqual(tags, [...tags].sort((left, right) => left - right))
  // Every stored field is a specification field, and the crate's own twenty
  // are standard fields above every published tag, so no field states a
  // branch at all - and the crate's close the walk, since nothing the seed
  // stores sits at or above their first tag.
  assert.ok([...registry].every((field) => field.fix.branch === fix.STANDARD_BRANCH))
  assert.ok([...registry].every((field) => !field.has('fix:branch')))
  assert.deepEqual(
    tags.filter((tag) => tag >= CRATE_TAG_MIN),
    fix.crateFields().map((field) => field.fix.tag),
  )
})

test('the registry takes every storage location', () => {
  const reference = seed()
  const url = Url.fromPath(SEED)

  for (const location of [SEED, path.resolve(SEED), url.toString(), url, new IOBase(SEED)]) {
    assert.ok(fix.FixRegistry.fromHandle(location).equals(reference))
  }

  // A folder that is not there loads as a new registry - the crate's own
  // fields and nothing else - and is not created.
  const root = scratch()
  try {
    const missing = path.join(root, 'missing')
    assert.ok(fix.FixRegistry.fromHandle(missing).equals(new fix.FixRegistry()))
    assert.equal(fix.FixRegistry.fromHandle(missing).size, CRATED)
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

test('a written catalog reloads all four categories', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')
  const reference = seed()
  reference.writeInto(dictionary)
  assert.deepEqual(fs.readdirSync(dictionary).sort(), ['components', 'fields', 'groups', 'messages'])
  const shards = fs.readdirSync(path.join(dictionary, 'fields'))
  assert.equal(shards.length, 65)
  // The crate's own fields are never written - a standard field from 65000
  // up is the crate's rather than the store's - so the shard their block
  // would take is absent, and the reload holds them all the same.
  assert.equal(shards.includes('650.json'), false)
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
  const reloaded = fix.FixRegistry.fromHandle(new IOBase(dictionary))
  for (const key of [453, 'partyid', 447, 452]) assert.equal(reloaded.remove(key), null)
  reloaded.insert(fixField('Unreferenced', 'utf8', 39999))
  assert.equal(reloaded.remove(39999).name, 'Unreferenced')
  reloaded.writeInto(new IOBase(dictionary))
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))
})

test('a vendor branch gets its own folder', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')

  const registry = fix.FixRegistry.fromFields([
    fixField('MsgType', 'utf8', 35),
    fixField('TradeID', 'utf8', 5001, { branch: 'cme' }),
  ])
  registry.writeInto(dictionary)

  // Each branch owns its own shard arithmetic: 5001 / 100 is 50.
  assert.ok(fs.existsSync(path.join(dictionary, 'fields', '0.json')))
  assert.ok(fs.existsSync(path.join(dictionary, 'fields', 'cme', '50.json')))
  assert.equal(
    fs.existsSync(path.join(dictionary, 'fields', '650.json')),
    false,
    "the crate's own fields are never stored",
  )

  const reloaded = fix.FixRegistry.fromHandle(dictionary)
  assert.ok(reloaded.equals(registry))
  assert.equal(reloaded.fieldById('5001:cme').name, 'TradeID')
  assert.equal(reloaded.fieldByName('tradeid', 'cme').name, 'TradeID')
  assert.equal(reloaded.getFieldByTag(5001).name, 'TradeID')
})

test('insert, update and remove carry the core rules across', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('Price', 'decimal128(20, 8)', 44, { aliases: ['Px'] }),
  ])
  assert.equal(registry.size, 2 + CRATED)
  assert.equal(registry.insert(fixField('Side', 'utf8', 54)), null)
  assert.equal(registry.fieldByTag(54).name, 'Side')

  // A key another field holds is refused, naming both and the branch; nothing
  // changes.
  assert.throws(
    () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { aliases: ['ticker'] })),
    /held by Symbol/,
  )
  assert.throws(
    () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { aliases: ['ticker'] })),
    /branch \\"\\"/,
  )
  assert.equal(registry.size, 3 + CRATED)

  // The same alias in another branch is not a conflict at all.
  assert.equal(
    registry.insert(fixField('VenueSym', 'utf8', 5055, { branch: 'cme', aliases: ['ticker'] })),
    null,
  )
  assert.equal(registry.fieldByName('TICKER', 'cme').name, 'VenueSym')

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
  assert.equal(registry.size, 2 + CRATED)
  const stored = registry.fieldByTag(55)
  assert.equal(stored.name, 'Symbol')
  assert.equal(stored.fix.id, '55:')
  assert.deepEqual(stored.fix.tags, [65, 66, 9001])
  assert.deepEqual(stored.fix.aliases, ['Ticker', 'Sym'])
  assert.equal(stored.fix.description, 'incoming')

  // Every spelling the incoming field carried now reaches the stored one.
  for (const key of [9001, 66, 65, 'sym', 'TICKER']) {
    assert.equal(registry.field(key).name, 'Symbol', `${key}`)
  }

  // Folding it again changes nothing, and a field nothing answers to arrives
  // whole.
  const folded = registry.intoJson()
  assert.equal(registry.addField(incoming), false)
  assert.equal(registry.intoJson(), folded)
  assert.equal(registry.addField(fixField('TransactTime', 'utf8', 60)), true)
  assert.equal(registry.size, 3 + CRATED)

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
    () => registry.removeById('55:'),
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

function order(registry) {
  return fields.struct(
    'NewOrderSingle',
    [
      registry.fieldByTag(55),
      registry.fieldByTag(38),
      registry.fieldByName('nopartyids', ''),
      registry.definition('groups', 'Parties'),
      Field.from('9999: utf8'),
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
}

test('a message resolves through the registry it carries', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)

  assert.ok(message.field.equals(root))
  assert.ok(message.registry.equals(registry))
  assert.equal(message.branch, fix.STANDARD_BRANCH)
  assert.equal([...message].length, 5)
  // `size` is what Python spells `len(message)`, and it agrees with the walk.
  assert.equal(message.size, 5)
  assert.equal(message.size, [...message.entries()].length)
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.equal(message.byId('55:').asJs(), 'AAPL')
  assert.equal(message.byName('SYMBOL').asJs(), 'AAPL')
  assert.equal(message.byTag(38).toString(), '100.0')
  assert.equal(message.byPath('parties.0.partyid').asJs(), 'BROKER')
  // An unknown tag is retained under its rendered name, never dropped.
  assert.equal(message.byTag(9999).asJs(), 'custom')
  // An identifier is exact: a dictionary this message does not speak misses.
  assert.equal(message.getById('5001:cme'), null)

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
    () => message.byId('5001:cme'),
    /^Error: expected a fix value at "identifier 5001:#[0-9a-f]{8}", got nothing$/,
  )
  assert.throws(() => message.byName('nope'), /name \\"nope\\"/)
  assert.throws(() => message.byPath('parties.partyid'), /path \\"parties.partyid\\"/)
  assert.throws(() => message.at(55n), {
    name: 'TypeError',
    message: 'key must be a number tag or a string name, got BigInt',
  })
  assert.throws(() => message.byTag(2 ** 31), /tag must be a signed 32-bit integer/)
  // A malformed identifier is the native parse failure, never a miss.
  assert.throws(() => message.byId('55'), /fix identifier/)
  assert.throws(() => message.getById('cme:'), /fix identifier/)
  assert.throws(() => message.getById(55), /into rust type `String`/)

  // The plain object became the ordered row the root declares.
  const pairs = [...message]
  assert.deepEqual(pairs.map(([name]) => name), root.dtype.keys())
  assert.deepEqual([...message.entries()].map(([name]) => name), pairs.map(([name]) => name))
  assert.equal(pairs[0][1].asJs(), 'AAPL')
  assert.equal(message.value.kind, 'sequence')

  // A native Scalar names the same row.
  assert.ok(new fix.FixMsg(root, message.value, registry).equals(message))
})

test('a venue message resolves in two steps', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('MsgType', 'utf8', 35),
    fixField('TradeID', 'utf8', 5001, { branch: 'cme', aliases: ['VenueTrade'] }),
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('Symbol', 'utf8', 5055, { branch: 'cme', aliases: ['VenueTicker'] }),
  ])
  const root = fields.struct(
    'VenueOrder',
    [Field.from('MsgType: utf8'), Field.from('TradeID: utf8'), Field.from('Symbol: utf8')],
    { nullable: false },
  )
  root.fix.branch = 'cme'
  const message = new fix.FixMsg(
    root,
    { MsgType: 'D', TradeID: 'T-1', Symbol: 'AAPL' },
    registry,
  )

  // The branch is the root's own, derived and never declared.
  assert.equal(message.branch, 'cme')
  // Step one: the message's own dictionary.
  assert.equal(message.byTag(5001).asJs(), 'T-1')
  assert.equal(message.byName('venuetrade').asJs(), 'T-1')
  assert.equal(message.byName('venueticker').asJs(), 'AAPL')
  // Step two: the standard branch, which every FIX message still carries.
  assert.equal(message.byTag(35).asJs(), 'D')
  // And no third step: a standard alias the venue does not define still
  // resolves, because the standard branch is the second tier.
  assert.equal(message.byName('ticker').asJs(), 'AAPL')

  // An identifier names one dictionary exactly and does not tier.
  assert.equal(message.byId('5001:cme').asJs(), 'T-1')
  assert.equal(message.byId('35:').asJs(), 'D')
  assert.equal(message.getById('5001:'), null)

  // A standard message is one step: it never reads a venue dictionary.
  const plain = fields.struct(
    'Order',
    [Field.from('MsgType: utf8'), Field.from('TradeID: utf8')],
    { nullable: false },
  )
  const standard = new fix.FixMsg(plain, { MsgType: 'D', TradeID: 'T-1' }, registry)
  assert.equal(standard.branch, '')
  assert.equal(standard.byTag(35).asJs(), 'D')
  assert.equal(standard.getByTag(5001), null)
  assert.equal(standard.getByName('venuetrade'), null)
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

  // A root whose stored branch is malformed fails at construction.
  const broken = fields.struct('row', [Field.from('Symbol: utf8')], { nullable: false })
  broken.set('fix:branch', '2cme')
  assert.throws(() => new fix.FixMsg(broken, { Symbol: 'AAPL' }, registry), /fix:branch/)
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

  const copy = message.clone()
  assert.ok(copy.equals(message))
  assert.ok(copy.registry.equals(registry))
  assert.equal(message.toString(), 'FixMsg("NewOrderSingle", 5 values)')

  const document = message.toJSON()
  assert.deepEqual(Object.keys(document), ['field', 'value'])
  assert.equal(document.field.metadata['fix:tag'], undefined, 'the root carries no tag')
  // The value document is the ordered row, not the object it was written as.
  assert.equal(document.value[0], 'AAPL')
  assert.equal(document.value.length, 5)
  assert.ok(JSON.stringify(message).includes('"NewOrderSingle"'))
})

test('a registry is a value: equality, hash, clone, JSON and text', () => {
  const registry = seed()

  assert.ok(registry.equals(seed()))
  assert.equal(registry.stableHash(), seed().stableHash())
  assert.equal(typeof registry.stableHash(), 'bigint')
  assert.equal(registry.equals(new fix.FixRegistry()), false)
  assert.equal(registry.toString(), `FixRegistry(${6241 + CRATED} fields)`)
  // A new registry is never empty: it holds the crate's own fields.
  assert.equal(new fix.FixRegistry().toString(), `FixRegistry(${CRATED} fields)`)

  const document = registry.toJSON()
  // The crate's own are seeded, never stored, so only the store's own are written.
  assert.equal(document.fields.length, 6241)
  assert.deepEqual(document.fields[0], JSON.parse(JSON.stringify(registry.fieldByTag(1))))
  assert.equal(document.fields[0].metadata['fix:tag'], '1')
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
      'STANDARD_BRANCH',
      'USER_TAG_MAX',
      'USER_TAG_MIN',
      'UlPlugin',
      'UlPlugins',
      'crateFields',
      'globalRegistry',
      'installGlobalRegistry',
      'schema',
      'schemaCarrying',
      'schemaTags',
      'ulbridgeFields',
    ],
  )
  assert.equal(fix.STANDARD_BRANCH, '')
  assert.equal(fix.USER_TAG_MIN, 5000)
  assert.equal(fix.USER_TAG_MAX, 40_000)
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
    '_fixStandardBranchNative',
    '_fixStandardTagLimitNative',
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
    assert.equal(fix.globalRegistry().fieldByName('SYMBOL', '').name, 'symbol')
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
  const reader = new fix.FixCodec(registry)

  assert.equal(reader.parseLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
  assert.equal(reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|10=0|')).next().value.byTag(55).toJSON(), 'AAPL')
  assert.equal(
    reader.parseFixLine(Buffer.from('8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01')).byTag(55).toJSON(),
    'AAPL',
  )
  assert.equal(reader.parsePairs([['55', 'AAPL']]).byTag(55).toJSON(), 'AAPL')
  assert.ok(reader.registry.equals(registry))

  // Three children every built message has, whatever its line carried: it
  // opens with `beginstring` - the wire's own, else the version it was read
  // at - states the `version` the read used, and closes with the crate's
  // `timestamp`. None is an entry unless the line carried it, so the wire
  // re-emits byte for byte.
  const pairs = new fix.FixCodec(registry, { version: '4.2' }).parsePairs([['55', 'AAPL']])
  assert.deepEqual([...pairs].map(([name]) => name), ['beginstring', 'symbol', 'version', 'timestamp'])
  assert.equal(pairs.byTag(65001).toJSON(), '4.2')
  assert.equal(pairs.byTag(8).toJSON(), 'FIX.4.2')
  assert.deepEqual(pairs.arrivals().map(([tag]) => tag), [55])
  assert.equal(pairs.intoBytes(124).toString(), '55=AAPL|')

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

  // Tag 32 is `lastshares` at 4.2 and `lastqty` from 4.3 on. A pin settles how
  // a value is read, never what a field is called: the column is the
  // dictionary's own whatever version read the row, and the 4.2 spelling still
  // reaches it as an alias.
  const dated = new fix.FixCodec(registry, { version: '4.2' })
  const named = dated.parseLine(Buffer.from('8=FIX.4.4|35=8|32=100|10=0|')).next().value
  assert.ok(named.field.indexOf('lastqty') !== null)
  assert.ok(named.getByName('lastshares') !== null)
  assert.ok(named.getByName('lastqty') !== null)

  // A stated absence produces no field at all.
  const silent = new fix.FixCodec(registry, { nullValues: ['<none>'] })
  assert.equal(silent.parseLine(Buffer.from('8=FIX.4.4|35=D|55=<none>|10=0|')).next().value.getByTag(55), null)

  assert.throws(() => new fix.FixCodec(registry, { branch: 'not a branch' }))
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

test('a message restates at the dictionary\'s newest version', () => {
  // A FIX 4.2 execution report: a transaction type, a partial fill, a Rule80A
  // capacity and two identities the specification later moved into `Parties`.
  const line =
    '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
  const read = new fix.FixCodec(seed()).parseLine(Buffer.from(line)).next().value
  assert.equal(read.byTag(65001).toJSON(), '4.2')
  assert.equal(read.byTag(150).toJSON(), '40PARTFILL')
  assert.equal(read.getByTag(528), null)
  assert.equal(read.getByTag(453), null)

  // Restatement is a method; the rules are the dictionary's.
  const latest = read.intoLatest()
  // ExecTransType Cancel wrote ExecType TradeCancel over the retired
  // PartiallyFilled, and the source stays.
  assert.equal(latest.byTag(150).toJSON(), '40TRDCXL')
  assert.equal(latest.byTag(20).toJSON(), '1')
  // Rule80A A is an agency order.
  assert.equal(latest.byTag(528).toJSON(), 'A')
  assert.equal(latest.byTag(47).toJSON(), 'A')
  // ExecBroker and ClientID are two parties, in tag order, counted.
  assert.equal(latest.byTag(453).asJs(), 2)
  assert.equal(latest.byPath('parties.0.partyid').asJs(), 'BRKR')
  assert.equal(latest.byPath('parties.0.partyrole').asJs(), 1)
  assert.equal(latest.byPath('parties.1.partyid').asJs(), 'CLIENT1')
  assert.equal(latest.byPath('parties.1.partyrole').asJs(), 3)
  // The fill under its newest spelling, reachable by the old one too.
  assert.equal(latest.byTag(32).asJs(), 100)
  assert.equal(latest.byName('LastShares').asJs(), 100)
  // The row speaks the dictionary's newest version; the wire still says 4.2.
  assert.equal(latest.byTag(65001).toJSON(), '5.0.2')
  assert.equal(latest.byTag(8).toJSON(), 'FIX.4.2')

  // Only the row was restated: the wire comes back byte for byte, the
  // arrival record and the anomalies are the same, and a second pass
  // changes nothing.
  assert.equal(latest.intoBytes('|'.charCodeAt(0)).toString(), line)
  assert.deepEqual(latest.arrivals(), read.arrivals())
  assert.deepEqual(latest.anomalies(), read.anomalies())
  assert.ok(latest.intoLatest().equals(latest))
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
// The three identity columns, by tag.
const INSTID = 65016
const ID = 65017
const PERSISTENTID = 65018
const PIPE = '|'.charCodeAt(0)

/** The bytes one identity column holds, or null where it holds nothing. */
function identity(message, tag) {
  const held = message.getByTag(tag)
  return held === null ? null : Buffer.from(held.asJs())
}

test('every message of one order carries the chain identity until it ends', () => {
  const registry = seed()
  const reader = new fix.FixCodec(registry)
  const life = new fix.FixLifecycle(registry)
  const stamped = []
  for (const line of LIFE) {
    stamped.push(life.fill(reader.parseLine(Buffer.from(line)).next().value))
    // Alive from the first message to the fill that ends it.
    assert.equal(life.alive, stamped.length < LIFE.length ? 1 : 0)
  }

  // One instrument, one chain, six messages.
  const instruments = stamped.map((held) => identity(held, INSTID))
  assert.equal(instruments[0].length, 16)
  assert.ok(instruments.every((held) => held.equals(instruments[0])))
  const chains = stamped.map((held) => identity(held, PERSISTENTID))
  assert.ok(
    chains.every((held) => held.equals(chains[0])),
    "the replace's new identifier joined the chain the old one opened",
  )
  const ids = stamped.map((held) => identity(held, ID))
  for (let at = 1; at < ids.length; at += 1) {
    assert.ok(Buffer.compare(ids[at - 1], ids[at]) < 0, 'ids sort by the impact clock')
  }
  // The chain is dated by the order's own transaction time, in microseconds,
  // and every id after it opens with a later instant.
  assert.equal(chains[0].readBigInt64BE(0), 1_767_348_930_000_000n)
  assert.ok(ids[0].subarray(0, 8).equals(chains[0].subarray(0, 8)))
  // A state a row holds is the ranked spelling, never the wire's code.
  assert.equal(stamped[1].byTag(39).toJSON(), '20NEW')
  assert.equal(stamped[5].byTag(39).toJSON(), '80FILLED')

  // Nothing here is an entry: the wire re-emits byte for byte.
  for (const [at, line] of LIFE.entries()) {
    assert.equal(stamped[at].intoBytes(PIPE).toString(), line)
  }

  // The identifier a venue reuses tomorrow opens a new chain rather than
  // joining yesterday's, which ended: dated by its own clock, it is another
  // identity.
  const tomorrow = LIFE[0].replaceAll('20260102', '20260103')
  const again = life.fill(reader.parseLine(Buffer.from(tomorrow)).next().value)
  assert.equal(identity(again, PERSISTENTID).equals(chains[0]), false)
  assert.equal(life.alive, 1)
  assert.equal(life.toString(), 'FixLifecycle(1 alive)')
  life.clear()
  assert.equal(life.alive, 0)
  assert.equal(String(life), 'FixLifecycle(0 alive)')
  // The same line at the same instant is the same chain identity, which is
  // what makes two reads of one capture agree.
  const replayed = life.fill(reader.parseLine(Buffer.from(LIFE[0])).next().value)
  assert.ok(identity(replayed, PERSISTENTID).equals(chains[0]))
  assert.ok(identity(replayed, ID).equals(ids[0]))

  // Over any iterable, the reader runs one lifecycle for the whole stream,
  // lazily, and a stamped stream read again keeps what it carries.
  const stream = reader.lifecycle(reader.parseLines(LIFE.map((line) => Buffer.from(line))))
  assert.ok(stream instanceof fix.FixMessages)
  const once = [...stream]
  const twice = [...reader.lifecycle(once)]
  assert.equal(once.length, LIFE.length)
  for (const [at, first] of once.entries()) {
    for (const tag of [INSTID, ID, PERSISTENTID]) {
      assert.deepEqual(identity(first, tag), identity(twice[at], tag), `tag ${tag}`)
    }
    assert.equal(first.arrivals().length, twice[at].arrivals().length)
  }
  assert.ok(identity(once[5], PERSISTENTID).equals(chains[0]))
  assert.deepEqual([...reader.lifecycle([])], [])
})

test('a message naming no order has an id and no chain', () => {
  const reader = new fix.FixCodec(seed())
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
  const sent = identity(heartbeat, ID)
  assert.notEqual(sent, null, 'every message has an id')
  assert.equal(heartbeat.getByTag(PERSISTENTID), null, 'no identifier, no chain')
  assert.equal(heartbeat.getByTag(INSTID), null, 'no instrument, no identity')
  // The impact clock is the sending time where no transaction time is
  // stated, and the epoch where the message states no clock at all.
  assert.equal(sent.readBigInt64BE(0), 1_767_348_930_000_000n)
  const [undated] = reader.lifecycle([reader.parseLine(Buffer.from('8=FIX.4.4|35=0|10=0|')).next().value])
  assert.ok(identity(undated, ID).subarray(0, 8).equals(Buffer.alloc(8)))

  // A lifecycle over the process default is the same pass: the columns are
  // the crate's own, which every registry holds. It stamps messages, never
  // lines.
  const life = new fix.FixLifecycle()
  assert.equal(life.alive, 0)
  assert.ok(identity(life.fill(heartbeat), ID).equals(sent))
  assert.throws(() => life.fill('8=FIX.4.4|35=0|10=0|'))
})

test('the instrument identity is the same across spellings and venues', () => {
  const registry = seed()
  const reader = new fix.FixCodec(registry)
  const life = new fix.FixLifecycle(registry)
  const instrument = (line) => identity(life.fill(reader.parseLine(Buffer.from(line)).next().value), INSTID)
  // An ISIN outranks a symbol, so the same security under two symbols is one
  // instrument, and case is not a difference.
  const byIsin = instrument('8=FIX.4.4|35=D|11=B1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|10=0|')
  assert.ok(byIsin.equals(instrument('8=FIX.4.4|35=D|11=B2|48=us0378331005|22=4|55=APPLE|207=xnas|15=usd|10=0|')))
  // Another market is another instrument identity.
  assert.equal(byIsin.equals(instrument('8=FIX.4.4|35=D|11=B3|48=US0378331005|22=4|55=AAPL|207=XLON|15=USD|10=0|')), false)
  // Without an ISIN the symbol stands in, and a stated one wins over a
  // symbol that would say otherwise.
  const bySymbol = instrument('8=FIX.4.4|35=D|11=B4|55=AAPL|207=XNAS|15=USD|10=0|')
  assert.notEqual(bySymbol, null)
  assert.equal(bySymbol.equals(byIsin), false)
  // A bridge row names the same facts under its own keys.
  assert.ok(byIsin.equals(instrument('#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B5|')))
  // Five orders, none of them ended.
  assert.equal(life.alive, 5)
})

test('the fixed row is spelled by name, filled by tag and never shifts', () => {
  const registry = seed()
  const schema = fix.schema(registry, 'FixMessage')
  // A column is the dictionary's folded name, never the tag's digits; the
  // tag stays on the column as its identity.
  assert.equal(schema.fieldAt(0).name, 'beginstring')
  assert.equal(schema.fieldAt(0).fix.tag, 8)
  assert.equal(schema.fieldAt(2).name, 'msgtype')
  assert.equal(schema.fieldAt(2).fix.tag, 35)
  assert.equal(schema.fieldAt(schema.fieldLen - 2).name, 'nofixentries')
  assert.equal(schema.fieldAt(schema.fieldLen - 1).name, 'nounmappedfixentries')
  assert.deepEqual(fix.schemaTags().slice(0, 3), [8, 9, 35])
  // The crate's own facts close the columns, and FIX's own `MsgDirection`
  // after them, because no message carries it on the wire.
  assert.deepEqual(fix.schemaTags().slice(-21), [
    65000, 65001, 65002, 65003, 65004, 65005, 65006, 65007, 65008, 65009,
    65010, 65011, 65012, 65013, 65014, 65015, 65016, 65017, 65018, 65019,
    385,
  ])

  // A column is found by its folded name, and nothing else is needed.
  assert.equal(schema.indexOf('msgtype'), 2)
  assert.equal(schema.indexOf('35'), null)
  assert.equal(schema.indexOf('999999'), null)
  assert.equal(schema.name, 'FixMessage')

  // The four columns every message fills are declared so; every other is
  // nullable, because a message that carried nothing there must answer null
  // rather than shift its neighbours.
  const required = []
  for (let at = 0; at < schema.fieldLen; at += 1) {
    const column = schema.fieldAt(at)
    if (!column.nullable) required.push(column.name)
  }
  assert.deepEqual(required, ['beginstring', 'msghash', 'timestamp', 'unixpartition'])

  const reader = new fix.FixCodec(registry)
  const message = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|9999=x|10=0|')).next().value
  const row = message.intoRow(schema).toJSON()
  assert.equal(row.length, schema.fieldLen)
  assert.equal(row[schema.indexOf('beginstring')], 'FIX.4.4')
  assert.equal(row[schema.indexOf('msgtype')], 'D')
  assert.equal(row[schema.indexOf('symbol')], 'AAPL')
  assert.equal(row[schema.indexOf('version')], '4.4')
  assert.equal(row[schema.indexOf('sendercompid')], null, 'no sender, not a shift')
  // A message with no clock is stamped with the epoch, which sorts first and
  // visibly, and the partition is the epoch's own.
  assert.notEqual(row[schema.indexOf('timestamp')], null)
  assert.ok(message.getByTag(65003).equals(Scalar.datetime(0n, 'ns', 'UTC')))
  assert.ok(message.marketTimestamp().equals(message.getByTag(65003)))
  assert.ok(message.unixPartition(3600).equals(Scalar.fromJs(0n)))
  // A tag no dictionary explains is still there, in its own column.
  assert.equal(row[row.length - 1].length, 1)
})

test("a capture's own columns lead the row", () => {
  const registry = seed()
  const carrier = fields.struct(
    'line',
    [fields.utf8('url', { nullable: false }), fields.binary('body', { nullable: false })],
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
  assert.equal(held.length, CRATED)
  assert.deepEqual(
    held.map((field) => field.name),
    [
      'msghash',
      'version',
      'symbolticker',
      'timestamp',
      'unixpartition',
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
      'instid',
      'id',
      'persistentid',
      'targetsessionid',
    ],
  )
  assert.deepEqual(
    held.map((field) => field.display),
    [
      'MsgHash',
      'Version',
      'SymbolTicker',
      'Timestamp',
      'UnixPartition',
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
      'InstId',
      'Id',
      'PersistentId',
      'TargetSessionId',
    ],
  )
  // In tag order, on the standard branch, from 65000 up: above every tag FIX
  // or a venue publishes, so they collide with nothing a dictionary declares
  // and need no branch of their own.
  assert.deepEqual(
    held.map((field) => field.fix.id),
    held.map((_, at) => `${CRATE_TAG_MIN + at}:`),
  )
  assert.ok(held.every((field) => field.fix.branch === fix.STANDARD_BRANCH))

  const digest = held[0]
  assert.equal(digest.getProperty('digest', 'role'), 'holder')
  assert.equal(digest.getProperty('digest', 'algorithm'), 'xxh3-128')
  assert.equal(digest.getProperty('digest', 'sources'), '["nofixentries"]')
  assert.ok(digest.description)

  // The partition names the column it reads, which is the clock's own name.
  const partition = held[4]
  assert.equal(partition.getProperty('partition', 'sources'), '["timestamp"]')
  assert.equal(partition.getProperty('iceberg', 'transform'), 'truncate[3600]')
})

test("the bridge's six facts are crate fields, and every registry holds them", () => {
  // What a bridge's own log states about a line: the session the message
  // itself belongs to, the message context it was handled under, the plugin
  // that logged it and the one it came through before that, and the two
  // session names the line spells.
  const held = fix.crateFields().slice(7, 13)
  assert.deepEqual(
    held.map((field) => [field.name, field.display, field.fix.id]),
    [
      ['sendersessionid', 'SenderSessionId', '65007:'],
      ['msgctxid', 'MsgCtxId', '65008:'],
      ['pluginid', 'PluginId', '65009:'],
      ['prevpluginid', 'PrevPluginId', '65010:'],
      ['sendersessionname', 'SenderSessionName', '65011:'],
      ['targetsessionname', 'TargetSessionName', '65012:'],
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
  const derived = fix.crateFields().slice(13, 16)
  assert.deepEqual(
    derived.map((field) => [field.name, field.display, field.fix.id, field.dtype.toString()]),
    [
      ['isincode', 'ISINCode', '65013:', 'isin'],
      ['miccode', 'MICCode', '65014:', 'mic'],
      ['state', 'State', '65015:', 'state'],
    ],
  )
  assert.equal(derived[2].dtype.asciiWidth, 10)

  // And the three identities a lifecycle pass stamps - the instrument, the
  // message and the order chain - sixteen bytes each, so a monitor joins on
  // them as it joins on the digest.
  const identities = fix.crateFields().slice(16, 19)
  assert.deepEqual(
    identities.map((field) => [field.name, field.display, field.fix.id, field.dtype.toString()]),
    [
      ['instid', 'InstId', '65016:', 'fixed_size_binary(16)'],
      ['id', 'Id', '65017:', 'fixed_size_binary(16)'],
      ['persistentid', 'PersistentId', '65018:', 'fixed_size_binary(16)'],
    ],
  )
  assert.ok(identities.every((field) => field.description))

  // A new registry, a loaded one and a built one answer them alike, by
  // identifier, by name on the standard branch, and by the bare tag or name,
  // which is that branch's.
  for (const registry of [
    new fix.FixRegistry(),
    seed(),
    fix.FixRegistry.fromFields([fixField('Symbol', 'utf8', 55)]),
  ]) {
    assert.equal(registry.fieldById('65007:').name, 'sendersessionid')
    assert.equal(registry.fieldByName('SenderSessionId', fix.STANDARD_BRANCH).fix.tag, 65007)
    assert.equal(registry.fieldByTag(65012).name, 'targetsessionname')
    assert.equal(registry.field('msgctxid').fix.id, '65008:')
    assert.equal(registry.has('pluginid'), true)
    assert.equal(registry.fieldByName('ULToSessionName', fix.STANDARD_BRANCH).name, 'targetsessionname')
  }

  // And each is a column of the fixed row, typed by the crate's own
  // definition and spelled by its folded name.
  const schema = fix.schema(seed(), 'FixMessage')
  for (const field of [...held, ...derived, ...identities]) {
    const at = schema.indexOf(field.name)
    assert.notEqual(at, null, field.name)
    assert.equal(schema.fieldAt(at).fix.id, field.fix.id)
    assert.equal(schema.fieldAt(at).display, field.display)
    assert.equal(schema.fieldAt(at).nullable, true)
  }
})

test('a message says everything the core derives about it', () => {
  const registry = seed()
  const reader = new fix.FixCodec(registry)
  const message = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|207=XNAS|54=1|44=10.5|38=100|60=20240201-12:34:56|10=0|')).next().value

  assert.equal(message.symbolTicker().toJSON(), 'AAPL@XNAS')
  assert.ok(message.marketTimestamp() !== null)
  assert.ok(message.unixPartition(3600) !== null)
  // The clock is the message's own last child, stamped when it was built,
  // and the partition floors it to the hour.
  assert.equal([...message].at(-1)[0], 'timestamp')
  assert.ok(message.marketTimestamp().equals(message.getByTag(65003)))
  assert.ok(message.unixPartition(3600).equals(Scalar.fromJs(1706788800n)))
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
  assert.match(message.liftSource('bidpx'), /^44/)
  assert.ok(message.lift().length > 0)
  assert.equal(message.digest().length, 16)
  assert.equal(message.arrivals()[0][0], 8)
  // An arrival states the tag its key named; the dialect is the message's own,
  // and a message read under none is on the standard branch.
  assert.equal(message.branch, fix.STANDARD_BRANCH)
  assert.equal(message.intoBytes(124).toString().split('|')[0], '8=FIX.4.4')
})

test('a dialect crosses as its name and the registry resolves a digest back', () => {
  const root = scratch()
  const file = path.join(root, 'bloomberg.cfb')
  fs.writeFileSync(
    file,
    [
      '<CBlock>',
      '<Fields>',
      '<Field name="10001" alt="ExcludedDealers" type="string" desc="Dealers excluded."/>',
      '</Fields>',
      '</CBlock>',
    ].join('\n'),
    'utf8',
  )
  const [registry] = fix.FixRegistry.fromCfbFile(file, 'bloomberg')

  // The dialect is declared by the file, and a capture read under it carries
  // that dialect once, on the message. A pair does not repeat it: what a
  // dictionary decided is one value for every field of one message.
  const reader = new fix.FixCodec(registry, { branch: 'bloomberg' })
  const message = reader.parseLine(Buffer.from('8=FIX.4.4|35=D|10001=NONE|10=0|')).next().value
  assert.equal(message.branch, 'bloomberg')
  assert.ok(message.arrivals().every(([tag]) => Number.isInteger(tag)))

  // The digest table stays what it is: a capture written by a reader that
  // stores digests joins to a declaration through it, without reproducing the
  // hash. Zero is the standard branch, which every registry holds through the
  // crate's own fields; a value no declared branch digests to names nothing at
  // all.
  assert.equal(registry.getBranchByDigest(0), fix.STANDARD_BRANCH)
  assert.equal(registry.getBranchByDigest(-1), null)
  assert.throws(() => registry.branchByDigest(-1))
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
