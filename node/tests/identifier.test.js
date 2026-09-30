'use strict'

// `node/src/identifier.rs`: the identifier and the sorted map an element
// states them in, keyed `src:type`, redirected to the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Identifier, Identifiers, Scalar, graph } = require('yggdryl')

/** What `Identifier.fromKey` reads a key as, displayed. */
function read(key, value) {
  const keyed = Identifier.fromKey(key, value)
  assert.notEqual(keyed, null, `${key} names an identifier`)
  return keyed.toString()
}

test('an identifier folds its words and trims its value', () => {
  const held = new Identifier('bic', 'executing_trader', ' T-1 ')
  assert.deepEqual([held.src, held.type, held.value], ['bic', 'executingtrader', 'T-1'])
  assert.equal(held.toString(), 'bic:executingtrader=T-1')
  assert.ok(held.isOf('BIC', 'Executing Trader'))
  assert.ok(!held.isOf('fix', 'executingtrader'))
  const folded = new Identifier('FIX', 'CLORDID', 'C-2')
  assert.deepEqual([folded.src, folded.type], ['fix', 'clordid'])
})

test('an identifier is three texts and states no parentage', () => {
  const held = new Identifier('fix', 'clordid', 'C-2', 'C-1', 'C-0')
  assert.equal(held.toString(), 'fix:clordid=C-2', 'an argument past the value is not read')
  assert.equal('parent' in held, false)
  assert.equal('orig' in held, false)
  assert.equal(held.parent, undefined)
  assert.equal(held.orig, undefined)
})

test('an identifier names its unique key src:type', () => {
  const held = new Identifier('Fix', 'ClOrdID', 'C-1')
  assert.equal(held.key, 'fix:clordid')
  assert.equal(held.key, `${held.src}:${held.type}`)
  assert.equal(new Identifier('firm.x', 'house code', 'hc-1').key, 'firm.x:housecode')
  assert.equal(new Identifier('base', 'isin', 'US0378331005').key, 'base:isin')
  assert.equal(new Identifier('fix', 'orderid', 'A').key, new Identifier('fix', 'orderid', 'B').key, 'a key names, never a value')
  assert.notEqual(new Identifier('fix', 'orderid', 'A').key, new Identifier('venue', 'orderid', 'A').key)
  assert.notEqual(new Identifier('fix', 'orderid', 'A').key, new Identifier('fix', 'clordid', 'A').key)
  assert.ok(Identifier.fromKey(held.key, held.value).equals(held), 'the key is what fromKey reads back')
  assert.throws(() => { held.key = 'x:y' }, TypeError, 'a key is read-only')
})

test('a source and a type are lower-case words and a value is left as given', () => {
  const held = new Identifier('BASE', 'ISIN', 'us0378331005')
  assert.deepEqual([held.src, held.type], ['base', 'isin'])
  assert.equal(held.value, 'US0378331005', 'an ISIN is upper-cased as its type stores it')
  assert.equal(held.toString(), 'base:isin=US0378331005')
  assert.equal(new Identifier('Fix', 'OrderID', 'O-1').value, 'O-1')
  assert.equal(new Identifier('fix', 'isinnumber', 'US0378331005').type, 'isin')
})

test('a retired source is a word like any other', () => {
  // `oms` and `ullink` are no source of the crate's own: they parse as the words they are.
  const held = new Identifier('OMS', 'orderid', 'O-1')
  assert.deepEqual([held.src, held.key], ['oms', 'oms:orderid'])
  assert.equal(new Identifier('ullink', 'instrumentid', 'dbi;X').src, 'ullink')
  // A retired type is a word too, and no parent of anything.
  assert.equal(new Identifier('fix', 'transversalkey', 'K-1').type, 'transversalkey')
})

test('an identifier refuses what is no word and a value that states nothing', () => {
  assert.throws(() => new Identifier('fix', 'café', 'X'), /identifier type/)
  assert.throws(() => new Identifier('fix', '', 'X'), /identifier type/)
  assert.throws(() => new Identifier('café', 'orderid', 'X'), /identifier source/)
  assert.throws(() => new Identifier('fix', 'orderid', 'n/a'), /identifier value/)
  assert.throws(() => new Identifier('fix', 'orderid', ''), /identifier value/)
})

test('a security type checks its code', () => {
  const apple = new Identifier('fix', 'isin', ' us0378331005 ')
  assert.deepEqual([apple.src, apple.type, apple.value], ['fix', 'isin', 'US0378331005'])
  assert.throws(() => new Identifier('fix', 'isin', 'US0378331006'))
  assert.equal(new Identifier('base', 'forex', 'eur/usd').value, 'EUR/USD')
})

test('a key names its source and its type', () => {
  const stated = Identifier.fromKey('fix:ClOrdID', 'C-1')
  assert.deepEqual([stated.src, stated.type], ['fix', 'clordid'])
  assert.equal(read('firm.x:house code', 'hc-1'), 'firm.x:housecode=hc-1')
  const bare = Identifier.fromKey('ClOrdID', 'C-1')
  assert.deepEqual([bare.src, bare.type], ['base', 'clordid'])
  assert.equal(Identifier.fromKey('ClOrdID', 'null'), null)
})

test('a key is read for the identifier name it ends with and the source before it', () => {
  assert.equal(read('firm.x.ParentOrderID', 'P-1'), 'firm.x:parentorderid=P-1')
  assert.equal(read('OMS_InstrumentID', 'dbi;X'), 'oms:instrumentid=dbi;X')
  assert.equal(read('marketorderid', 'O-1'), 'market:orderid=O-1')
  assert.equal(read('OMSDealerParentOrderID', 'P-1'), 'omsdealer:parentorderid=P-1')
  assert.equal(read('OMSUserID', 'U-1'), 'oms:userid=U-1')
  assert.equal(read('OrderID', 'O-1'), 'base:orderid=O-1')
  assert.equal(read('firm..x_OrderID', 'O-1'), 'firm..x:orderid=O-1', 'a dot inside is kept')
  assert.equal(read('.OrderID', 'O-1'), 'base:orderid=O-1', 'a dot at the end is trimmed')
  assert.equal(read('oms#orderid', 'O-1'), 'oms:orderid=O-1', 'folding drops #')
  assert.equal(read('OrigClOrdID', 'C-0'), 'base:origclordid=C-0')
  assert.equal(read('MyOrigClOrdID', 'C-0'), 'my:origclordid=C-0', 'the longest name the key ends with answers')
  assert.equal(read('oms.Account', 'ACC-1'), 'oms:account=ACC-1')
  assert.equal(
    read('ullink.InstrumentId', 'dbi;CH0012214059_XSWX_CHF'),
    'ullink:instrumentid=dbi;CH0012214059_XSWX_CHF',
  )
})

test('a whole security name is that type from base', () => {
  assert.equal(read('ISINCode', 'US0378331005'), 'base:isin=US0378331005')
  assert.equal(read('security_cusip', '037833100'), 'base:cusip=037833100')
  assert.equal(read('firm.isin', 'us0378331005'), 'firm:isin=US0378331005')
})

test('a key that names no identifier or another instrument\'s security is null', () => {
  for (const [key, value] of [
    ['underlyingisin', 'US0378331005'],
    ['legisin', 'US0378331005'],
    ['contra.isin', 'US0378331005'],
    ['relatedcusip', '037833100'],
    ['benchmarkisin', 'US0378331005'],
  ]) assert.equal(Identifier.fromKey(key, value), null, key)
  for (const key of ['transversalkey', 'symbol', 'ticker', 'securityid', 'id', '', '   ', '#']) {
    assert.equal(Identifier.fromKey(key, 'X-1'), null, JSON.stringify(key))
  }
  assert.equal(read('contraorderid', 'O-1'), 'contra:orderid=O-1', 'only a security type is refused there')
  assert.equal(Identifier.fromKey('ISINCode', 'US0378331006'), null, 'a bad check digit')
  assert.equal(Identifier.fromKey('firm.x.ParentOrderID', ''), null)
})

test('identifiers order by their key as spelled, then by value', () => {
  const first = new Identifier('fix', 'a', '1')
  const second = new Identifier('fix', 'b', '1')
  assert.equal(first.compare(second), -1)
  assert.equal(new Identifier('alpha', 'z', '1').compare(first), -1, 'a source orders before a type')
  // The order is the one of the text `src:type`, never of the (src, type)
  // pair: a '.' or a digit sorts below ':', so a longer source sorts first.
  assert.equal(new Identifier('a.b', 'c', '1').compare(new Identifier('a', 'z', '1')), -1, "'a.b:c' < 'a:z'")
  assert.equal(new Identifier('a1', 'x', '1').compare(new Identifier('a', 'x', '1')), -1, "'a1:x' < 'a:x'")
  assert.equal(new Identifier('a', 'z', '1').compare(new Identifier('ab', 'a', '1')), -1, "':' sorts below a letter")
  assert.equal(new Identifier('fix', 'orderid', 'A').compare(new Identifier('fix', 'orderid', 'B')), -1)
  assert.equal(new Identifier('fix', 'orderid', 'Z').compare(new Identifier('oms', 'clordid', 'A')), -1, 'a key decides before a value')
  const loose = [
    new Identifier('a', 'z', '1'),
    new Identifier('a1', 'x', '1'),
    new Identifier('a', 'x', '2'),
    new Identifier('a', 'x', '1'),
    new Identifier('a.b', 'c', '1'),
    new Identifier('ab', 'a', '1'),
  ]
  assert.deepEqual(
    loose.sort((left, right) => left.compare(right)).map(String),
    ['a.b:c=1', 'a1:x=1', 'a:x=1', 'a:x=2', 'a:z=1', 'ab:a=1'],
  )
})

test('identifiers compare as values', () => {
  const first = new Identifier('fix', 'a', '1')
  assert.ok(first.equals(new Identifier('FIX', 'A', '1')))
  assert.ok(!first.equals(new Identifier('fix', 'a', '2')))
})

test('a map holds one identifier per key, sorted by that key', () => {
  const ids = new Identifiers([
    new Identifier('base', 'isin', 'US0378331005'),
    new Identifier('BASE', 'ISIN', 'CH0012214059'),
    new Identifier('derived', 'cusip', '037833100'),
    new Identifier('ullink', 'instrumentid', 'dbi;X'),
  ])
  assert.equal(ids.length, 3)
  assert.deepEqual(ids.toArray().map(String), [
    'base:isin=US0378331005',
    'derived:cusip=037833100',
    'ullink:instrumentid=dbi;X',
  ])
  assert.deepEqual(ids.toArray().map((id) => id.key), ['base:isin', 'derived:cusip', 'ullink:instrumentid'], 'key order')
  assert.equal(ids.get('isin'), 'US0378331005')
  assert.equal(ids.getFrom('ullink', 'instrument_id'), 'dbi;X')
  assert.equal(ids.getFrom('fix', 'ISIN'), null)
  assert.ok(ids.containsKind('cusip') && !ids.containsKind('SEDOL'))
  assert.equal(ids.getIdentifier('ISIN').src, 'base')
  assert.deepEqual(ids.ofKind('ISIN').map((id) => id.src), ['base'])
  assert.equal(ids.toString(), '[base:isin=US0378331005, derived:cusip=037833100, ullink:instrumentid=dbi;X]')
  assert.equal(new Identifiers().length, 0)
})

test('a map is held in the order of its spelled keys', () => {
  const ids = new Identifiers([
    new Identifier('a', 'z', '1'),
    new Identifier('a1', 'x', '1'),
    new Identifier('a.b', 'c', '1'),
    new Identifier('a', 'x', '1'),
    new Identifier('ab', 'a', '1'),
  ])
  assert.deepEqual(ids.toArray().map((id) => id.key), ['a.b:c', 'a1:x', 'a:x', 'a:z', 'ab:a'])
  assert.equal(ids.getFrom('a.b', 'c'), '1')
  assert.equal(ids.getFrom('a1', 'x'), '1')
  assert.ok(ids.equals(new Identifiers(ids.toArray().reverse())), 'the order a map was filled in never shows')
})

test('a stated source answers before a derived one', () => {
  const both = new Identifiers([new Identifier('derived', 'cusip', '037833100'), new Identifier('fix', 'cusip', '594918104')])
  assert.equal(both.get('cusip'), '594918104')
})

test('a map compares as a value', () => {
  const ids = new Identifiers([new Identifier('fix', 'orderid', 'O-1')])
  assert.ok(ids.equals(new Identifiers([new Identifier('fix', 'orderid', 'O-1')])))
  assert.equal(ids.equals(new Identifiers([new Identifier('fix', 'orderid', 'O-2')])), false, 'the value is part of the identity')
})

test('identifiers cross the scalar boundary as an identifier column holds them', () => {
  const row = Scalar.from(new Identifier('fix', 'orderid', 'O-1'))
  assert.deepEqual(row.asJs(), ['fix', 'orderid', 'O-1'])
  // A map is the sorted map from each key `src:type` to its row.
  const ids = new Identifiers([new Identifier('fix', 'orderid', 'O-1'), new Identifier('base', 'isin', 'US0378331005')])
  const map = Scalar.from(ids).asJs()
  assert.deepEqual(
    [...map].map(([key, held]) => [key, held]),
    [['base:isin', ['base', 'isin', 'US0378331005']], ['fix:orderid', ['fix', 'orderid', 'O-1']]],
  )
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    identifiers: new Identifiers([new Identifier('fix', 'orderid', 'O-1')]),
    securityids: [new Identifier('base', 'isin', 'US0378331005')],
  })
  // A first-seen identifier states no parent: the chain gives it its parents.
  assert.deepEqual(order.identifiers.toArray().map((id) => [id.key, id.value]), [['fix:orderid', 'O-1']])
  assert.equal(order.securityids.toString(), '[base:isin=US0378331005, derived:cusip=037833100]')
  assert.equal(order.partyids.length, 0)
})

test('an array of identifiers states the map it makes, one value under a key', () => {
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    partyids: [new Identifier('fix', 'account', 'ACC-1'), new Identifier('FIX', 'ACCOUNT', 'ACC-1')],
    identifiers: [],
  })
  assert.deepEqual(order.partyids.toArray().map(String), ['fix:account=ACC-1'])
  assert.equal(order.identifiers.length, 0)
  // Two values under one key are two readings, refused at the second.
  assert.throws(
    () =>
      new graph.OrderEvent(1n, {
        crosscode: 'O-1',
        partyids: [new Identifier('fix', 'account', 'ACC-1'), new Identifier('FIX', 'ACCOUNT', 'ACC-2')],
      }),
    /\$\[1\]: expected one value under fix:account, got "ACC-1" and "ACC-2"/,
  )
})

test('a map of identifier rows states the identifiers its keys name', () => {
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    identifiers: new Map([['fix:orderid', ['fix', 'orderid', 'O-1']], ['fix:clordid', ['fix', 'clordid', 'C-1']]]),
  })
  assert.deepEqual(order.identifiers.toArray().map((id) => id.key), ['fix:clordid', 'fix:orderid'])
})

test('a fact that is no identifier map is refused where it is stated, by the core', () => {
  const stated = (facts) => () => new graph.OrderEvent(1n, { crosscode: 'O-1', ...facts })
  // A key that is not its row's `src:type` is refused, expected and actual
  // located on the key - never read as an empty map.
  assert.throws(
    stated({ identifiers: new Map([['fix:clordid', ['fix', 'orderid', 'O-1']]]) }),
    /\['fix:clordid'\].*expected the key fix:orderid, got "fix:clordid"/s,
  )
  // A row that is no identifier, in an array or under a key.
  assert.throws(stated({ securityids: [['fix', 'isin']] }), /identifier row/)
  assert.throws(stated({ securityids: new Map([['fix:isin', ['fix', 'isin']]]) }), /identifier row/)
  // Anything that is no map and no array is refused by the core's reading.
  assert.throws(stated({ securityids: 'x' }), /expected a map or a sequence of identifiers, got string/)
})
