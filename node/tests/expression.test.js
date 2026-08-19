'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { Expression, Field, IOBase, Statement, Value } = require('yggdryl')

const TRADES = new Field(
  'trades',
  'struct(field("ccy",utf8,nullable=true,metadata={}),field("price",decimal128(9,2),nullable=true,metadata={}),field("size",int64,nullable=true,metadata={}))',
  false,
)

test('text parses and round-trips through the canonical form', () => {
  const filter = new Expression("ccy = 'EUR' and price > 100")
  assert.equal(filter.toString(), "ccy = 'EUR' and price > 100")
  assert.ok(new Expression(filter.toString()).equals(filter))
  assert.deepEqual(filter.columns, ['ccy', 'price'])
})

test('text is never taken as a string literal', () => {
  // The one failure this layer must not have: a filter that silently matches
  // everything because its text became a constant.
  assert.throws(() => new Expression('ccy = '), /expression/)
})

test('a document round-trips', () => {
  const filter = new Expression('size between 1 and 10')
  assert.ok(Expression.fromJson(filter.toJson()).equals(filter))
})

test('the tree is built from either spelling', () => {
  const left = new Expression("ccy = 'EUR'")
  assert.equal(left.and('size > 1').toString(), "ccy = 'EUR' and size > 1")
  assert.equal(left.or('size > 1').toString(), "ccy = 'EUR' or size > 1")
  assert.equal(left.not().toString(), "not ccy = 'EUR'")
})

test('binding resolves the columns and folds the literals', () => {
  const bound = new Expression('price > 100 and size is not null').bind(TRADES)
  assert.ok(bound.isPredicate)
  assert.deepEqual(bound.columns, ['price', 'size'])
  // The literal is converted once, into the column's own exact type.
  assert.equal(
    bound.expression.toString(),
    "price > decimal128(9,2) '100.00' and size is not null",
  )
})

test('a row answers, and unknown is not true', () => {
  const bound = new Expression("ccy = 'EUR' and size > 1").bind(TRADES)
  assert.equal(bound.matches(Value.fromJs(['EUR', null, 5])), true)
  assert.equal(bound.matches(Value.fromJs(['USD', null, 5])), false)
  assert.equal(bound.matches(Value.fromJs(['EUR', null, null])), false)
})

test('a holder attribute is its own question', () => {
  const filter = new Expression("&holder.partition['year'] = '2024' and &holder.size > 0")
  assert.deepEqual(filter.attributes, ["partition['year']", 'size'])
  assert.deepEqual(filter.columns, [])
})

test('a statement carries the whole read', () => {
  const statement = new Statement("select ccy, price as amount where ccy = 'EUR' limit 10")
  assert.deepEqual(statement.projections, ['ccy', 'amount'])
  assert.equal(statement.predicate.toString(), "ccy = 'EUR'")
  assert.equal(statement.limit, 10)
  assert.equal(new Statement(statement.toString()).toString(), statement.toString())
})

test('a lake is filtered by the same predicate the rows are', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-expression-'))
  for (const year of ['2024', '2025']) {
    const leaf = path.join(root, `year=${year}`)
    fs.mkdirSync(leaf, { recursive: true })
    fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
  }
  try {
    const handle = new IOBase(root)
    const matched = handle.childrenMatching("&holder.partition['year'] = '2024'")
    assert.ok(matched.length > 0)
    for (const entry of matched) {
      assert.match(String(entry.url), /year=2024/)
    }

    // The pair spelling selects the leaves, and it selects the same ones.
    const pairs = handle.childrenWhere({ year: '2024' })
    assert.equal(pairs.length, 1)
    assert.match(String(pairs[0].url), /year=2024\/part-0\.parquet$/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})
