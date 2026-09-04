'use strict'

// The FIX boundary: the typed vocabulary on the protocol view, the registry,
// and the message. Every answer here is the core's; what these check is the
// crossing - the key coercion, the tag width, the error class each refusal
// arrives as, the storage locations a JavaScript caller names, and the
// language protocols the loader wires over the native halves.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { DataType, Field, IOBase, Scalar, Url, fields, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', 'config', 'fix')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fix-'))
}

function seed() {
  return fix.FixRegistry.fromHandle(SEED)
}

function fixField(name, dtype, tag, { tags, aliases, description } = {}) {
  const field = Field.from(`${name}: ${dtype}`)
  field.fix.tag = tag
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
  assert.equal(field.fix.size, 4)

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
    assert.throws(() => {
      view.tag = 55
    }, { name: 'TypeError', message: new RegExp(scheme) })
    assert.throws(() => {
      view.aliases = ['Ticker']
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

test('the registry resolves every key the way the core does', () => {
  const registry = seed()
  assert.equal(registry.size, 34)

  assert.equal(registry.fieldByTag(55).name, 'Symbol')
  assert.equal(registry.getFieldByTag(55).name, 'Symbol')
  // The alternate tag 20 reaches ExecType, which claims 150 canonically.
  assert.equal(registry.fieldByTag(20).name, 'ExecType')
  assert.equal(registry.fieldByTag(150).name, 'ExecType')
  // A name answers the canonical spelling whatever case it was asked in.
  assert.equal(registry.fieldByName('symbol').name, 'Symbol')
  assert.equal(registry.fieldByName('SYMBOL').name, 'Symbol')
  assert.equal(registry.fieldByName('ticker').name, 'Symbol')
  assert.equal(registry.fieldByName('clientorderid').name, 'ClOrdID')
  // A path reaches a repeating group and one of its members.
  assert.equal(registry.fieldByPath('NoPartyIDs').fix.tag, 453)
  assert.equal(registry.fieldByPath('NoPartyIDs.PartyID').fix.tag, 448)
  assert.equal(registry.fieldByPath('nopartyids.item.PartyRole').name, 'PartyRole')

  // The generic pair answers exactly what the specialized one does.
  for (const key of [55, 'Symbol', 'ticker', 'NoPartyIDs.PartyID', 20]) {
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
})

test('absence throws with the core message, its get twin answers null', () => {
  const registry = seed()

  assert.throws(
    () => registry.fieldByTag(9999),
    /^Error: expected a fix field at "tag 9999", got nothing$/,
  )
  assert.throws(() => registry.fieldByName('Nope'), /name \\"Nope\\"/)
  assert.throws(() => registry.fieldByPath('Symbol.absent'), /path \\"Symbol.absent\\"/)
  assert.throws(() => registry.field(9999), /tag 9999/)
  assert.equal(registry.getFieldByName('Nope'), null)
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

test('the registry iterates lazily in canonical-tag order', () => {
  const registry = seed()

  const names = [...registry].map((field) => field.name)
  assert.deepEqual(names.slice(0, 4), ['Account', 'AvgPx', 'BeginString', 'BodyLength'])
  assert.equal(names.length, registry.size)
  assert.deepEqual([...registry.keys()].map((field) => field.name), names)

  const tags = [...registry].map((field) => field.fix.tag)
  assert.deepEqual(tags, [...tags].sort((left, right) => left - right))

  // An unfinished walk shares the registry, so a mutation refuses until the
  // walk ends - by exhaustion or by the `return` a `break` sends.
  const walk = registry.keys()
  assert.equal(walk.next().value.name, 'Account')
  assert.throws(() => registry.remove(1), /shared with a message/)
  walk.return()
  assert.equal(registry.remove(1).name, 'Account')
  for (const field of registry) {
    assert.equal(field.name, 'AvgPx')
    break
  }
  assert.equal(registry.remove(6).name, 'AvgPx')
  assert.equal(registry.size, 32)
})

test('the registry takes every storage location', () => {
  const reference = seed()
  const url = Url.fromPath(SEED)

  for (const location of [SEED, path.resolve(SEED), url.toString(), url, new IOBase(SEED)]) {
    assert.ok(fix.FixRegistry.fromHandle(location).equals(reference))
  }

  // A folder that is not there loads as empty and is not created.
  const root = scratch()
  try {
    const missing = path.join(root, 'missing')
    assert.equal(fix.FixRegistry.fromHandle(missing).size, 0)
    assert.equal(fs.existsSync(missing), false)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('a written folder reloads equal, and empty shards are removed', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const dictionary = path.join(root, 'dictionary')
  const reference = seed()

  reference.writeInto(dictionary)
  assert.deepEqual(
    fs.readdirSync(path.join(dictionary, 'records')).sort(),
    ['0.json', '1.json', '4.json'],
  )
  assert.ok(fix.FixRegistry.fromHandle(dictionary).equals(reference))

  const reloaded = fix.FixRegistry.fromHandle(new IOBase(dictionary))
  for (const key of [453, 'PartyID', 447, 452]) reloaded.remove(key)
  reloaded.writeInto(new IOBase(dictionary))
  assert.equal(fs.existsSync(path.join(dictionary, 'records', '4.json')), false)
  assert.equal(fix.FixRegistry.fromHandle(dictionary).size, reference.size - 4)
})

test('insert, update and remove carry the core rules across', () => {
  const registry = fix.FixRegistry.fromFields([
    fixField('Symbol', 'utf8', 55, { aliases: ['Ticker'] }),
    fixField('Price', 'decimal128(20, 8)', 44, { aliases: ['Px'] }),
  ])
  assert.equal(registry.size, 2)
  assert.equal(registry.insert(fixField('Side', 'utf8', 54)), null)
  assert.equal(registry.fieldByTag(54).name, 'Side')

  // A key another field holds is refused, naming both; nothing changes.
  assert.throws(
    () => registry.insert(fixField('SymbolSfx', 'utf8', 65, { aliases: ['ticker'] })),
    /held by Symbol/,
  )
  assert.equal(registry.size, 3)

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
  const message = new fix.FixMsg(root, { Symbol: 'AAPL' }, registry)

  for (const mutation of [
    () => registry.insert(fixField('Side', 'utf8', 54)),
    () => registry.update(fixField('Symbol', 'utf8', 55)),
    () => registry.remove(55),
  ]) {
    assert.throws(mutation, /shared with a message or installed as the process default/)
  }
  assert.ok(message.registry.equals(registry))
  // The shared dictionary stays readable, and a deep copy is writable.
  assert.equal(registry.fieldByTag(55).name, 'Symbol')
  const copy = registry.clone()
  assert.ok(copy.equals(registry))
  assert.equal(copy.remove(55).name, 'Symbol')
  assert.equal(copy.equals(registry), false)
  assert.equal(registry.size, 34)
})

function order(registry) {
  return fields.struct(
    'NewOrderSingle',
    [
      registry.fieldByTag(55),
      registry.fieldByTag(38),
      registry.fieldByName('NoPartyIDs'),
      Field.from('9999: utf8'),
    ],
    { nullable: false },
  )
}

const ORDER_VALUE = {
  Symbol: 'AAPL',
  OrderQty: Scalar.decimal(100n),
  NoPartyIDs: [{ PartyID: 'BROKER', PartyIDSource: 'D', PartyRole: 1 }],
  9999: 'custom',
}

test('a message resolves through the registry it carries', () => {
  const registry = seed()
  const root = order(registry)
  const message = new fix.FixMsg(root, ORDER_VALUE, registry)

  assert.ok(message.field.equals(root))
  assert.ok(message.registry.equals(registry))
  assert.equal([...message].length, 4)
  // `size` is what Python spells `len(message)`, and it agrees with the walk.
  assert.equal(message.size, 4)
  assert.equal(message.size, [...message.entries()].length)
  assert.equal(message.byTag(55).asJs(), 'AAPL')
  assert.equal(message.byName('ticker').asJs(), 'AAPL')
  assert.equal(message.byTag(38).toString(), '"100"')
  assert.equal(message.byPath('NoPartyIDs.0.PartyID').asJs(), 'BROKER')
  // An unknown tag is retained under its rendered name, never dropped.
  assert.equal(message.byTag(9999).asJs(), 'custom')

  assert.ok(message.get(55).equals(message.byTag(55)))
  assert.ok(message.at('ticker').equals(message.byTag(55)))
  assert.equal(message.get(1234), null)
  assert.equal(message.getByName('nope'), null)
  assert.equal(message.getByPath('NoPartyIDs.PartyID'), null)
  assert.throws(
    () => message.byTag(1234),
    /^Error: expected a fix value at "tag 1234", got nothing$/,
  )
  assert.throws(() => message.byName('nope'), /name \\"nope\\"/)
  assert.throws(() => message.byPath('NoPartyIDs.PartyID'), /path \\"NoPartyIDs.PartyID\\"/)
  assert.throws(() => message.at(55n), {
    name: 'TypeError',
    message: 'key must be a number tag or a string name, got BigInt',
  })
  assert.throws(() => message.byTag(2 ** 31), /tag must be a signed 32-bit integer/)

  // The plain object became the ordered row the root declares.
  const pairs = [...message]
  assert.deepEqual(pairs.map(([name]) => name), root.dtype.keys())
  assert.deepEqual([...message.entries()].map(([name]) => name), pairs.map(([name]) => name))
  assert.equal(pairs[0][1].asJs(), 'AAPL')
  assert.equal(message.value.kind, 'sequence')

  // A native Scalar names the same row.
  assert.ok(new fix.FixMsg(root, message.value, registry).equals(message))
})

test('a message refuses a value its field refuses', () => {
  const registry = seed()
  const root = fields.struct('row', [registry.fieldByTag(55)], { nullable: false })

  assert.throws(() => new fix.FixMsg(root, { Symbol: 5 }, registry), /Symbol/)
  assert.throws(() => new fix.FixMsg(Field.from('scalar: utf8'), { Symbol: 'AAPL' }, registry))
  assert.throws(() => fix.FixMsg(root, { Symbol: 'AAPL' }, registry), /without 'new'/)
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
  assert.equal(message.equals(new fix.FixMsg(root, { ...ORDER_VALUE, Symbol: 'MSFT' }, registry)), false)

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
  assert.equal(registry.toString(), 'FixRegistry(34 fields)')
  assert.equal(new fix.FixRegistry().toString(), 'FixRegistry(0 fields)')

  const document = registry.toJSON()
  assert.equal(document.length, 34)
  assert.deepEqual(document[0], JSON.parse(JSON.stringify(registry.fieldByTag(1))))
  assert.equal(document[0].metadata['fix:tag'], '1')
})

test('the fix namespace is frozen and the raw exports are gone', () => {
  const yggdryl = require('yggdryl')

  assert.ok(Object.isFrozen(fix))
  assert.deepEqual(
    Object.keys(fix).sort(),
    ['FixMsg', 'FixRegistry', 'globalRegistry', 'installGlobalRegistry'],
  )
  for (const name of [
    'FixFieldIterator',
    'FixMsg',
    'FixMsgEntries',
    'FixRegistry',
    'JsFixMsg',
    'JsFixRegistry',
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
    assert.equal(fix.globalRegistry().fieldByTag(55).name, 'Symbol')
    const root = fields.struct('row', [fix.globalRegistry().fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }).registry.equals(seed))
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
