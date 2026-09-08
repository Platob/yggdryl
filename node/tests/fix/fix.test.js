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
// fields, which `new fix.FixRegistry()` seeds and `fix.crateFields()` lists.
const CRATED = 11

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
}

function seed() {
  return fix.FixRegistry.fromHandle(SEED)
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
  assert.equal(registry.size, 6203 + CRATED)

  assert.equal(registry.fieldByTag(55).name, 'symbol')
  assert.equal(registry.getFieldByTag(55).name, 'symbol')
  assert.equal(registry.fieldById('55:').name, 'symbol')
  assert.ok(registry.getFieldById('55:').equals(registry.fieldByTag(55)))
  // The alternate tag 20 reaches ExecType, which claims 150 canonically.
  assert.equal(registry.fieldByTag(150).name, 'exectype')
  // A name answers the canonical spelling whatever case it was asked in.
  assert.equal(registry.fieldByName('symbol', '').name, 'symbol')
  assert.equal(registry.fieldByName('SYMBOL', fix.STANDARD_BRANCH).name, 'symbol')
  assert.equal(registry.fieldByName('clordid', '').name, 'clordid')
  // A path reaches a repeating group and one of its members.
  assert.equal(registry.fieldByPath('NoPartyIDs', '').fix.tag, 453)
  // An occurrence is not a path segment: the walk steps through the list and
  // the member is spelled directly under the counter.
  assert.equal(registry.fieldByPath('nopartyids.partyid', '').fix.tag, 448)
  assert.equal(registry.fieldByPath('nopartyids.partyrole', '').name, 'partyrole')
  assert.equal(registry.getFieldByPath('nopartyids.partyid.partyid', ''), null)

  // The generic pair answers exactly what the specialized one does.
  for (const key of [55, 'Symbol', 'nopartyids', 'nopartyids.partyid']) {
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
    ['8=FIX.4.4|35=UL|#SYMBOL=TTF|10=001|', MimeType.FIXUL, 'UDF'],
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
    const bytes = MimeType.inferBytesMsgtype(Buffer.from(line))
    assert.equal(bytes?.toString() ?? null, msgtype)
    assert.equal(MimeType.inferTextMsgtype(line), msgtype)
  }

  assert.equal(MimeType.inferBytesMsgtype(Buffer.from('35=AE|')).toString(), 'AE')
  assert.equal(MimeType.inferTextMsgtype('MSGTYPE=AE|'), 'AE')

  // A bridge configuration states its own half of the exchange, and the
  // `send` its own payload spells is never read as the marker.
  const answered =
    '{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*",' +
    '"type":"read"},"value":{"name":"send-test-request"},"status":200}'
  assert.ok(MimeType.inferText(answered).equals(MimeType.ULCONFIG))
  assert.equal(MimeType.inferTextDirection(answered), 'RECV')
  assert.equal(MimeType.inferTextMsgtype(answered), 'read')
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
  // own fields close the walk: their tags sit above any a test claims.
  const crated = fix.crateFields().map((field) => field.fix.id)
  assert.deepEqual(crated, Array.from({ length: CRATED }, (_, at) => `${30001 + at}:yggdryl`))
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

test('the seed iterates in canonical-tag order and only the crate branch is named', () => {
  const registry = seed()

  const names = [...registry].map((field) => field.name)
  assert.deepEqual(names.slice(0, 4), ['account', 'advid', 'advrefid', 'advside'])
  assert.equal(names.length, registry.size)

  const tags = [...registry].map((field) => field.fix.tag)
  assert.deepEqual(tags, [...tags].sort((left, right) => left - right))
  // Every stored field is a specification field, so none states a branch;
  // the crate's own eleven, which every registry holds, are the only ones
  // on a branch at all.
  const branches = [...registry].map((field) => field.fix.branch)
  assert.equal(branches.filter((branch) => branch === 'yggdryl').length, CRATED)
  assert.equal(branches.filter((branch) => branch === '').length, registry.size - CRATED)
  assert.ok([...registry].every((field) => field.has('fix:branch') === (field.fix.branch === 'yggdryl')))
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

test('a root left in the retired layout is refused', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const old = path.join(root, 'old')

  fs.mkdirSync(path.join(old, 'records', ''), { recursive: true })
  fs.writeFileSync(path.join(old, 'records', '0.json'), '[]')
  assert.throws(() => fix.FixRegistry.fromHandle(old), /records/)
})

test('a written folder reloads equal through the two trees', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')
  const reference = seed()

  reference.writeInto(dictionary)
  // A shard per hundred tags, over both trees: counted rather than listed,
  // because the committed dictionary is six thousand fields and the listing
  // would be the generator's output restated.
  const primitive = fs.readdirSync(path.join(dictionary, 'primitive', '')).sort()
  const nested = fs.readdirSync(path.join(dictionary, 'nested', '')).sort()
  assert.ok(primitive.includes('0.json') && nested.includes('0.json'))
  assert.equal(primitive.length + nested.length, 128)
  // The crate's own branch is never written: those fields are the crate's
  // rather than the store's, and the reload holds them all the same.
  assert.equal(primitive.includes('yggdryl') || nested.includes('yggdryl'), false)
  // The reload is the assertion; the listing above only says it sharded.
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))

  const reloaded = fix.FixRegistry.fromHandle(new IOBase(dictionary))
  for (const key of [453, 'partyid', 447, 452]) reloaded.remove(key)
  reloaded.writeInto(new IOBase(dictionary))
  // A shard of a six-thousand-field dictionary survives losing four of them;
  // what the removal has to show is the count, not a missing file.
  assert.ok(fs.existsSync(path.join(dictionary, 'primitive')))
  assert.equal(fix.FixRegistry.fromHandle(dictionary).size, reference.size - 4)
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
  assert.ok(fs.existsSync(path.join(dictionary, 'primitive', '0.json')))
  assert.ok(fs.existsSync(path.join(dictionary, 'primitive', 'cme', '50.json')))
  assert.equal(
    fs.existsSync(path.join(dictionary, 'primitive', 'yggdryl')),
    false,
    'the crate branch is never stored',
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
  assert.equal(merged.name, 'SYMBOL')
  assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
  // A datatype disagreement is refused, never widened.
  assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 55)))
  assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

  assert.equal(registry.remove('sym').name, 'SYMBOL')
  assert.equal(registry.getFieldByTag(65), null)
  assert.equal(registry.remove(9999), null)

  // A field with no tag cannot enter at all.
  assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /fix:tag/)
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
  assert.equal(copy.remove(55).name, 'symbol')
  assert.equal(copy.equals(registry), false)
  assert.equal(registry.size, 6203 + CRATED)
})

function order(registry) {
  return fields.struct(
    'NewOrderSingle',
    [
      registry.fieldByTag(55),
      registry.fieldByTag(38),
      registry.fieldByName('nopartyids', ''),
      Field.from('9999: utf8'),
    ],
    { nullable: false },
  )
}

const ORDER_VALUE = {
  symbol: 'AAPL',
  orderqty: Scalar.float(100),
  nopartyids: [{ partyid: 'BROKER', partyidsource: 'D', partyrole: 1 }],
  9999: 'custom',
}

test('a message resolves through the registry it carries', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)

  assert.ok(message.field.equals(root))
  assert.ok(message.registry.equals(registry))
  assert.equal(message.branch, fix.STANDARD_BRANCH)
  assert.equal([...message].length, 4)
  // `size` is what Python spells `len(message)`, and it agrees with the walk.
  assert.equal(message.size, 4)
  assert.equal(message.size, [...message.entries()].length)
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.equal(message.byId('55:').asJs(), 'AAPL')
  assert.equal(message.byName('SYMBOL').asJs(), 'AAPL')
  assert.equal(message.byTag(38).toString(), '100.0')
  assert.equal(message.byPath('nopartyids.0.partyid').asJs(), 'BROKER')
  // An unknown tag is retained under its rendered name, never dropped.
  assert.equal(message.byTag(9999).asJs(), 'custom')
  // An identifier is exact: a dictionary this message does not speak misses.
  assert.equal(message.getById('5001:cme'), null)

  assert.ok(message.get(55).equals(message.byTag(55)))
  assert.ok(message.at('symbol').equals(message.byTag(55)))
  assert.equal(message.get(1234), null)
  assert.equal(message.getByName('nope'), null)
  assert.equal(message.getByPath('nopartyids.partyid'), null)
  assert.throws(
    () => message.byTag(1234),
    /^Error: expected a fix value at "tag 1234", got nothing$/,
  )
  assert.throws(
    () => message.byId('5001:cme'),
    /^Error: expected a fix value at "identifier 5001:#[0-9a-f]{8}", got nothing$/,
  )
  assert.throws(() => message.byName('nope'), /name \\"nope\\"/)
  assert.throws(() => message.byPath('nopartyids.partyid'), /path \\"nopartyids.partyid\\"/)
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
  assert.equal(message.toString(), 'FixMsg("NewOrderSingle", 4 values)')

  const document = message.toJSON()
  assert.deepEqual(Object.keys(document), ['field', 'value'])
  assert.equal(document.field.metadata['fix:tag'], undefined, 'the root carries no tag')
  // The value document is the ordered row, not the object it was written as.
  assert.equal(document.value[0], 'AAPL')
  assert.equal(document.value.length, 4)
  assert.ok(JSON.stringify(message).includes('"NewOrderSingle"'))
})

test('a registry is a value: equality, hash, clone, JSON and text', () => {
  const registry = seed()

  assert.ok(registry.equals(seed()))
  assert.equal(registry.stableHash(), seed().stableHash())
  assert.equal(typeof registry.stableHash(), 'bigint')
  assert.equal(registry.equals(new fix.FixRegistry()), false)
  assert.equal(registry.toString(), `FixRegistry(${6203 + CRATED} fields)`)
  // A new registry is never empty: it holds the crate's own fields.
  assert.equal(new fix.FixRegistry().toString(), `FixRegistry(${CRATED} fields)`)

  const document = registry.toJSON()
  assert.equal(document.length, 6203 + CRATED)
  assert.deepEqual(document[0], JSON.parse(JSON.stringify(registry.fieldByTag(1))))
  assert.equal(document[0].metadata['fix:tag'], '1')
})

test('the fix namespace is frozen and the raw exports are gone', () => {
  const yggdryl = require('yggdryl')

  assert.ok(Object.isFrozen(fix))
  assert.deepEqual(
    Object.keys(fix).sort(),
    [
      'FixCodec',
      'FixMsg',
      'FixRegistry',
      'STANDARD_BRANCH',
      'USER_TAG_MAX',
      'USER_TAG_MIN',
      'crateFields',
      'globalRegistry',
      'installGlobalRegistry',
      'schema',
      'schemaCarrying',
      'schemaTags',
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
    'FixRegistry',
    'JsFixMsg',
    'JsFixCodec',
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

  assert.equal(reader.transformLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|')).byTag(55).toJSON(), 'AAPL')
  assert.equal(reader.transformLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|10=0|')).byTag(55).toJSON(), 'AAPL')
  assert.equal(
    reader.transformFixLine(Buffer.from('8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01'), 1).byTag(55).toJSON(),
    'AAPL',
  )
  assert.equal(reader.transformPairs([['55', 'AAPL']]).byTag(55).toJSON(), 'AAPL')
  assert.ok(reader.registry.equals(registry))

  // Two children every built message has, whatever its line carried: it
  // opens with `beginstring` - the wire's own, else the version it was read
  // at - and closes with the crate's `timestamp`. Neither is an entry unless
  // the line carried it, so the wire re-emits byte for byte.
  const pairs = new fix.FixCodec(registry, { version: '4.2' }).transformPairs([['55', 'AAPL']])
  assert.deepEqual([...pairs].map(([name]) => name), ['beginstring', 'symbol', 'timestamp'])
  assert.equal(pairs.byTag(8).toJSON(), 'FIX.4.2')
  assert.deepEqual(pairs.arrivals().map(([tag]) => tag), [55])
  assert.equal(pairs.toBytes(124).toString(), '55=AAPL|')

  // A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
  // whose value packs its members behind the two control bytes ULLINK uses.
  const bridge = reader.transformUllinkLine(
    Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    ),
  )
  const inferred = reader.transformLine(
    Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|',
      'binary',
    ),
  )
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
  const named = dated.transformLine(Buffer.from('8=FIX.4.4|35=8|32=100|10=0|'))
  assert.ok(named.field.indexOf('lastqty') !== null)
  assert.ok(named.getByName('lastshares') !== null)
  assert.ok(named.getByName('lastqty') !== null)

  // A stated absence produces no field at all.
  const silent = new fix.FixCodec(registry, { nullValues: ['<none>'] })
  assert.equal(silent.transformLine(Buffer.from('8=FIX.4.4|35=D|55=<none>|10=0|')).getByTag(55), null)

  assert.throws(() => new fix.FixCodec(registry, { branch: 'not a branch' }))
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
  assert.deepEqual(
    fix.schemaTags().slice(-12),
    [30001, 30002, 30003, 30004, 30005, 30006, 30007, 30008, 30009, 30010, 30011, 385],
  )

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
  const message = reader.transformLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|9999=x|10=0|'))
  const row = message.toRow(schema).toJSON()
  assert.equal(row.length, schema.fieldLen)
  assert.equal(row[schema.indexOf('beginstring')], 'FIX.4.4')
  assert.equal(row[schema.indexOf('msgtype')], 'D')
  assert.equal(row[schema.indexOf('symbol')], 'AAPL')
  assert.equal(row[schema.indexOf('version')], '4.4')
  assert.equal(row[schema.indexOf('sendercompid')], null, 'no sender, not a shift')
  // A message with no clock is stamped with the epoch, which sorts first and
  // visibly, and the partition is the epoch's own.
  assert.notEqual(row[schema.indexOf('timestamp')], null)
  assert.ok(message.getByTag(30004).equals(Scalar.datetime(0n, 'ns', 'UTC')))
  assert.ok(message.marketTimestamp().equals(message.getByTag(30004)))
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
  const row = new fix.FixCodec(registry).transformLine(Buffer.from('8=FIX.4.4|35=D|10=0|')).toRow(carried).toJSON()
  assert.equal(row[0], null)
  assert.equal(row[carried.indexOf('msgtype')], 'D')

  // A capture column whose folded name a FIX column takes is not carried in
  // front: `sessionId` and `sessionid` are one name, and the FIX column is
  // the one a reader spelling it means.
  const stamped = fields.struct(
    'line',
    [
      fields.utf8('url', { nullable: false }),
      fields.binary('body', { nullable: false }),
      fields.utf8('sessionId'),
    ],
    { nullable: false },
  )
  const folded = fix.schemaCarrying(stamped, plain)
  assert.equal(folded.fieldLen, plain.fieldLen + 2)
  assert.equal(folded.indexOf('sessionId'), null)
  assert.equal(folded.indexOf('sessionid'), plain.indexOf('sessionid') + 2)
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
      'sessionid',
      'msgctxid',
      'pluginid',
      'prevpluginid',
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
      'SessionId',
      'MsgCtxId',
      'PluginId',
      'PrevPluginId',
    ],
  )
  // In tag order, on the crate's own branch: the same tag as a venue's own
  // 30001 would be, and a different identity.
  assert.deepEqual(
    held.map((field) => field.fix.id),
    held.map((_, at) => `${30001 + at}:yggdryl`),
  )

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

test("the bridge's four facts are crate fields, and every registry holds them", () => {
  // What a bridge's own log states about a line: the session and the message
  // context it was handled under, and the plugins it moved between.
  const held = fix.crateFields().slice(7)
  assert.deepEqual(
    held.map((field) => [field.name, field.display, field.fix.id]),
    [
      ['sessionid', 'SessionId', '30008:yggdryl'],
      ['msgctxid', 'MsgCtxId', '30009:yggdryl'],
      ['pluginid', 'PluginId', '30010:yggdryl'],
      ['prevpluginid', 'PrevPluginId', '30011:yggdryl'],
    ],
  )
  assert.ok(held.every((field) => field.dtype.equals(DataType.from('utf8'))))
  assert.ok(held.every((field) => field.nullable))
  assert.ok(held.every((field) => field.description))

  // A new registry, a loaded one and a built one answer them alike, by
  // identifier, by name on the branch, and by the bare tag or name - which
  // reach past the standard branch, since it claims neither.
  for (const registry of [
    new fix.FixRegistry(),
    seed(),
    fix.FixRegistry.fromFields([fixField('Symbol', 'utf8', 55)]),
  ]) {
    assert.equal(registry.fieldById('30008:yggdryl').name, 'sessionid')
    assert.equal(registry.fieldByName('SessionId', 'yggdryl').fix.tag, 30008)
    assert.equal(registry.fieldByTag(30011).name, 'prevpluginid')
    assert.equal(registry.field('msgctxid').fix.id, '30009:yggdryl')
    assert.equal(registry.has('pluginid'), true)
  }

  // And each is a column of the fixed row, typed by the crate's own
  // definition and spelled by its folded name.
  const schema = fix.schema(seed(), 'FixMessage')
  for (const field of held) {
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
  const message = reader.transformLine(Buffer.from('8=FIX.4.4|35=D|55=AAPL|207=XNAS|54=1|44=10.5|38=100|60=20240201-12:34:56|10=0|'))

  assert.equal(message.symbolTicker().toJSON(), 'AAPL@XNAS')
  assert.ok(message.marketTimestamp() !== null)
  assert.ok(message.unixPartition(3600) !== null)
  // The clock is the message's own last child, stamped when it was built,
  // and the partition floors it to the hour.
  assert.equal([...message].at(-1)[0], 'timestamp')
  assert.ok(message.marketTimestamp().equals(message.getByTag(30004)))
  assert.ok(message.unixPartition(3600).equals(Scalar.fromJs(1706788800n)))
  // A buy order at a price is a party willing to pay it, so the bid lane it
  // never wrote is still true of it.
  assert.equal(message.lifted('bidpx').toJSON(), 10.5)
  assert.match(message.liftSource('bidpx'), /^44/)
  assert.ok(message.lift().length > 0)
  assert.equal(message.digest().length, 16)
  assert.equal(message.arrivals()[0][0], 8)
  // Every arrival states its dialect as the branch digest, and a message read
  // under no dialect is on the standard branch, whose digest is zero.
  assert.ok(message.arrivals().every(([, bid]) => bid === 0))
  assert.equal(message.toBytes(124).toString().split('|')[0], '8=FIX.4.4')
})

test('a dialect crosses as its digest and the registry resolves it back', () => {
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

  // The dialect is declared by the file, so a capture read under it carries a
  // digest the registry turns back into the branch. Without that table an
  // outside reader would have to reproduce the hash to join the two.
  const reader = new fix.FixCodec(registry, { branch: 'bloomberg' })
  const message = reader.transformLine(Buffer.from('8=FIX.4.4|35=D|10001=NONE|10=0|'))
  const [, bid] = message.arrivals()[0]
  assert.notEqual(bid, 0)
  assert.equal(registry.branchByDigest(bid), 'bloomberg')
  assert.equal(registry.getBranchByDigest(bid), 'bloomberg')

  // Only a declared branch resolves, and a value no digest can hold names
  // nothing at all.
  assert.equal(registry.getBranchByDigest(0), null)
  assert.equal(registry.getBranchByDigest(-1), null)
  assert.throws(() => registry.branchByDigest(-1))
})

test('a message type registers under the name and wording it is given', () => {
  const registry = new fix.FixRegistry()
  registry.insert(fixField('msgtype', 'utf8', 35))

  // What fits the column is itself; a bridge's composite key is hashed into
  // what fits, under the name and the wording the caller gives it.
  assert.equal(registry.registerMsgtype('D'), 'D')
  const value = registry.registerMsgtype(
    'P Report Ack',
    'AllocationReportAck',
    'Allocation Report ACK',
  )
  assert.match(value, /^~/)
  const codes = registry.fieldByTag(35).get('fix:codes')
  assert.match(codes, /"name":"AllocationReportAck"/)
  assert.match(codes, /P Report Ack/)
  assert.match(codes, /Allocation Report ACK/)

  // Idempotent: registering it again answers the same value.
  assert.equal(registry.registerMsgtype('P Report Ack'), value)
})

test('a CBlock refusal crosses with the byte, the content and the element', () => {
  const root = scratch()
  const file = path.join(root, 'broken.cfb')
  fs.writeFileSync(
    file,
    [
      '<?xml version="1.0"?>',
      '<cplugin-configuration fix-version="4.4">',
      '<vocabulary><vocabulary-tag name="35" alt="MsgType" type="decimal" /></vocabulary>',
      '</cplugin-configuration>',
    ].join('\n'),
    'utf8',
  )

  // The native sentence crosses whole rather than as a bare "invalid file":
  // the byte the reader stopped at, the eight types it wanted, the ninth it
  // got, and the element the file declared it in.
  assert.throws(
    () => fix.FixRegistry.fromCfbFile(file, 'bloomberg'),
    (error) => {
      assert.match(error.message, /invalid cfb expression at byte \d+/)
      assert.match(error.message, /utc-time-only/)
      assert.match(error.message, /"decimal"/)
      assert.match(error.message, /<vocabulary-tag name=\\"35\\"/)
      return true
    },
  )
})
