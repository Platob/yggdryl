'use strict'

// `node/src/identifier.rs`: a value under a key `src:type` - the type alone
// for the base source - and the sorted map an element states them in,
// redirected to the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Identifier, Identifiers, Scalar, graph } = require('yggdryl')

/** What `Identifier.fromKey` reads a name as, displayed. */
function read(key, value) {
  const keyed = Identifier.fromKey(key, value)
  assert.notEqual(keyed, null, `${key} names an identifier`)
  return keyed.toString()
}

test('an identifier reads its key exactly and trims its value', () => {
  const held = new Identifier('bic:executing_trader', ' DEUTDEFF ')
  assert.deepEqual([held.src, held.type, held.value], ['bic', 'executingtrader', 'DEUTDEFF'])
  assert.equal(held.key, 'bic:executingtrader')
  assert.equal(held.toString(), 'bic:executingtrader=DEUTDEFF')
  for (const spelled of ['clordid', 'ClOrdID', 'base:clordid', 'BASE:CLORDID', 'fix:clordid', 'FIX:ClOrdID']) {
    const bare = new Identifier(spelled, 'C-2')
    assert.deepEqual([bare.src, bare.type, bare.key], ['base', 'clordid', 'clordid'], spelled)
  }
  assert.equal(new Identifier('isin', 'US0378331005').toString(), 'isin=US0378331005', 'a base key is its type alone')
  assert.equal(new Identifier('marketorderid', 'O-1').key, 'marketorderid', 'a bare word is read, never inferred')
  assert.equal(new Identifier('isinnumber', ' us0378331005 ').value, 'US0378331005')
})

test('a key or a value that reads as nothing is refused', () => {
  for (const key of ['fix:', ':isin', 'a:b:c', 'café', '']) {
    assert.throws(() => new Identifier(key, 'X'), /identifier key/, key)
  }
  assert.throws(() => new Identifier('orderid', 'n/a'), /identifier value/)
  // A check digit that does not close is a rank, never a refusal: the shape
  // is what a type holds to.
  assert.equal(new Identifier('isin', 'US0378331006').toString(), 'isin=US0378331006')
  assert.throws(() => new Identifier('isin', 'US037833100'), /expected twelve characters/)
  assert.equal(new Identifier('forex', 'eur/usd').value, 'EUR/USD')
})

test('a name no key spells is read for the identifier name it ends with', () => {
  assert.equal(read('firm.x.ParentOrderID', 'P-1'), 'firm.x:parentorderid=P-1')
  assert.equal(read('OMS_InstrumentID', 'dbi;X'), 'oms:instrumentid=dbi;X')
  assert.equal(read('marketorderid', 'O-1'), 'market:orderid=O-1')
  assert.equal(read('OrderID', 'O-1'), 'orderid=O-1')
  assert.equal(read('fix:ClOrdID', 'C-1'), 'clordid=C-1')
  assert.equal(read('Derived_ISIN', 'US0378331005'), 'isin=US0378331005', 'a reserved source names no namespace')
  assert.equal(read('ISINCode', 'US0378331005'), 'isin=US0378331005')
  assert.equal(read('X-SWX-VALOR', '1221405'), 'valor=1221405', "a vendor's source spelling")
  assert.equal(read('OMS_SIXSymbol', 'HOLN'), 'oms:exchsymb=HOLN')
  for (const key of ['underlyingisin', 'legisin', 'transversalkey', 'symbol', '']) {
    assert.equal(Identifier.fromKey(key, 'US0378331005'), null, key)
  }
})

test('identifiers order by their key as spelled, then by value, and compare as values', () => {
  const cusip = '037833100'
  const isin = 'US0378331005'
  assert.equal(new Identifier('cusip', cusip).compare(new Identifier('derived:cusip', cusip)), -1)
  assert.equal(new Identifier('derived:cusip', cusip).compare(new Identifier('isin', isin)), -1)
  assert.equal(new Identifier('isin', isin).compare(new Identifier('ullink:isin', isin)), -1)
  assert.equal(new Identifier('a.b:c', '1').compare(new Identifier('a:z', '1')), -1, "'.' sorts below ':'")
  assert.equal(new Identifier('orderid', 'A').compare(new Identifier('orderid', 'B')), -1)
  const first = new Identifier('oms:clordid', '1')
  assert.ok(first.equals(new Identifier('OMS:ClOrdID', '1')))
  assert.ok(!first.equals(new Identifier('oms:clordid', '2')))
})

test('a named source fills the base key of its type', () => {
  const ids = new Identifiers([
    new Identifier('ullink:isin', 'US0378331005'),
    new Identifier('isin', 'CH0012214059'),
    new Identifier('derived:cusip', '037833100'),
    new Identifier('oms:instrumentid', 'dbi;X'),
  ])
  assert.deepEqual(ids.toArray().map(String), [
    'cusip=037833100',
    'derived:cusip=037833100',
    'instrumentid=dbi;X',
    'isin=US0378331005',
    'oms:instrumentid=dbi;X',
    'ullink:isin=US0378331005',
  ])
  assert.equal(ids.length, 6)
  assert.equal(ids.get('isin'), 'US0378331005', 'the first statement fills; a later base key is dropped')
  assert.equal(ids.getFrom('ullink:isin'), 'US0378331005')
  assert.equal(ids.getFrom('oms:isin'), null)
  assert.equal(ids.getFrom('BASE:ISIN'), ids.get('ISIN'))
  assert.ok(ids.containsKind('cusip') && !ids.containsKind('SEDOL'))
  assert.deepEqual(ids.ofKind('isin').map((id) => id.key), ['isin', 'ullink:isin'])
  assert.ok(ids.isDerived('cusip') && !ids.isDerived('isin'))
  assert.equal(new Identifiers().length, 0)
})

test('a statement takes back the derivation of its type', () => {
  const ids = new Identifiers([new Identifier('derived:cusip', '037833100'), new Identifier('ullink:cusip', '594918104')])
  assert.deepEqual(ids.toArray().map(String), ['cusip=594918104', 'ullink:cusip=594918104'])
  assert.ok(!ids.isDerived('cusip'))
})

test('an object reads exactly and closes by the base rule', () => {
  const ids = Identifiers.fromObject({ 'ullink:isin': 'US0378331005', 'OMS:InstrumentID': 'dbi;X' })
  assert.deepEqual(ids.intoObject(), {
    instrumentid: 'dbi;X',
    isin: 'US0378331005',
    'oms:instrumentid': 'dbi;X',
    'ullink:isin': 'US0378331005',
  })
  assert.deepEqual(Object.keys(ids.intoObject()), Object.keys(ids.intoObject()).sort(), 'an object is in key order')
  assert.ok(Identifiers.fromObject(ids.intoObject()).equals(ids), 'a written map comes back unchanged')
  const derived = Identifiers.fromObject({ 'derived:cusip': '037833100' })
  assert.deepEqual(derived.intoObject(), { cusip: '037833100', 'derived:cusip': '037833100' })
  assert.ok(derived.isDerived('cusip'), 'a derivation read back stays one')
  assert.ok(Identifiers.fromObject({}).equals(new Identifiers()))
  assert.throws(() => Identifiers.fromObject({ 'fix:': 'X' }), /fix:/)
  assert.throws(() => Identifiers.fromObject({ isin: 'US037833100' }), /\$\['isin'\]: expected twelve characters/)
  // A typo is a value of the lowest rank: a map read raw closes each type on
  // its highest-ranked named source, whatever order the entries came in.
  assert.deepEqual(Identifiers.fromObject({ isin: 'US0378331006' }).intoObject(), { isin: 'US0378331006' })
  for (const entries of [
    { 'abc:isin': 'US0378331006', 'venue:isin': 'US0378331005' },
    { 'zzz:isin': 'US0378331005', 'abc:isin': 'US0378331006' },
  ]) {
    assert.equal(Identifiers.fromObject(entries).get('isin'), 'US0378331005')
  }
  assert.throws(() => Identifiers.fromObject({ isin: 'US0378331005', 'BASE:ISIN': 'CH0012214059' }), /one value under isin/)
})

test('identifiers cross the scalar boundary as an identifier column holds them', () => {
  const entry = Scalar.from(new Identifier('ullink:isin', 'US0378331005')).asJs()
  assert.deepEqual([...entry], [['ullink:isin', 'US0378331005']], 'an identifier is the one-entry map of its key')
  const ids = new Identifiers([new Identifier('orderid', 'O-1'), new Identifier('isin', 'US0378331005')])
  assert.deepEqual([...Scalar.from(ids).asJs()], Object.entries(ids.intoObject()))
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    identifiers: new Identifiers([new Identifier('orderid', 'O-1')]),
    securityids: [new Identifier('isin', 'US0378331005')],
  })
  assert.deepEqual(order.identifiers.toArray().map((id) => [id.key, id.value]), [['orderid', 'O-1']])
  assert.equal(order.securityids.toString(), '[cusip=037833100, derived:cusip=037833100, isin=US0378331005]')
  assert.equal(order.partyids.length, 0)
})

test('an array of identifiers states the map it makes, one value under a key', () => {
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    partyids: [new Identifier('bic:account', 'DEUTDEFF'), new Identifier('BIC:ACCOUNT', 'DEUTDEFF')],
    identifiers: [],
  })
  assert.deepEqual(order.partyids.toArray().map(String), ['account=DEUTDEFF', 'bic:account=DEUTDEFF'])
  assert.equal(order.identifiers.length, 0)
  // Two values under one key are two readings, refused at the second.
  assert.throws(
    () =>
      new graph.OrderEvent(1n, {
        crosscode: 'O-1',
        partyids: [new Identifier('account', 'ACC-1'), new Identifier('fix:account', 'ACC-2')],
      }),
    /\$\[1\]\['account'\].*expected one value under account, got "ACC-1" and "ACC-2"/,
  )
})

test('a map of keys to values states the identifiers its keys name', () => {
  const order = new graph.OrderEvent(1n, {
    crosscode: 'O-1',
    identifiers: new Map([['orderid', 'O-1'], ['oms:clordid', 'C-1']]),
  })
  assert.deepEqual(order.identifiers.toArray().map((id) => id.key), ['clordid', 'oms:clordid', 'orderid'])
})

test('a fact that is no identifier map is refused where it is stated, by the core', () => {
  const stated = (facts) => () => new graph.OrderEvent(1n, { crosscode: 'O-1', ...facts })
  // A key that reads as none is refused, located on the key - never read as
  // an empty map.
  assert.throws(stated({ identifiers: new Map([['fix:', 'O-1']]) }), /\['fix:'\].*identifier key/s)
  // A value that is no text, in an array or under a key.
  assert.throws(stated({ securityids: new Map([['isin', ['base', 'isin']]]) }), /text value/)
  // Anything that is no map and no array is refused by the core's reading.
  assert.throws(stated({ securityids: 'x' }), /a map of identifier keys to values, or a sequence of such maps, got string/)
})
