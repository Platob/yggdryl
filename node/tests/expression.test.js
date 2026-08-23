'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Expression, Field, IOBase, Statement, Scalar, fields, iceberg } = require('yggdryl')

const TRADES = new Field(
  'trades',
  'struct(field("ccy",utf8,nullable=true,metadata={}),field("price",decimal128(9,2),nullable=true,metadata={}),field("size",int64,nullable=true,metadata={}))',
  false,
)

// One commit is one manifest, so several commits give the manifest list rows a
// partition filter can rule out. The second XLON row is what lets a predicate
// over the rows discriminate *within* a surviving file, rather than being
// answered by the partition alone.
function venues(root) {
  const declared = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8'), Field.from('venue: utf8')],
    { nullable: false },
  )
  const table = iceberg.Table.create(path.join(root, 'trades'), declared, ['venue'])
  for (const [id, symbol, venue] of [
    [1n, 'AAPL', 'XNAS'],
    [2n, 'MSFT', 'XNYS'],
    [3n, 'VOD', 'XLON'],
    [4n, 'BP', 'XLON'],
  ]) {
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([id], new arrow.Int64()),
        symbol: arrow.vectorFromArray([symbol], new arrow.Utf8()),
        venue: arrow.vectorFromArray([venue], new arrow.Utf8()),
      }),
    )
  }
  return table
}

test('text parses and round-trips through the canonical form', () => {
  const filter = new Expression("ccy = 'EUR' and price > 100")
  assert.equal(filter.toString(), "ccy = 'EUR' and price > 100")
  assert.ok(new Expression(filter.toString()).equals(filter))
  assert.deepEqual(filter.columns, ['ccy', 'price'])
  const clone = filter.clone()
  assert.notEqual(clone, filter)
  assert.ok(clone.equals(filter))
  assert.equal(clone.compare(filter), 0)
  assert.equal(clone.stableHash(), filter.stableHash())
  assert.ok(new Expression("ccy = 'EUR'").compare("ccy = 'USD'") < 0)
  assert.deepEqual(JSON.parse(JSON.stringify(filter)), JSON.parse(filter.intoJson()))
})

test('text is never taken as a string literal', () => {
  // The one failure this layer must not have: a filter that silently matches
  // everything because its text became a constant.
  assert.throws(() => new Expression('ccy = '), /expression/)
})

test('a document round-trips', () => {
  const filter = new Expression('size between 1 and 10')
  assert.ok(Expression.fromJson(filter.intoJson()).equals(filter))
  assert.equal(filter.toJson, undefined)
})

test('the tree is built from either spelling', () => {
  const left = new Expression("ccy = 'EUR'")
  assert.equal(left.and('size > 1').toString(), "ccy = 'EUR' and size > 1")
  assert.equal(left.or('size > 1').toString(), "ccy = 'EUR' or size > 1")
  assert.equal(left.not().toString(), "not ccy = 'EUR'")
})

test('arithmetic builders stay lazy expression nodes', () => {
  const size = Expression.column('size')
  assert.equal(size.add('1').toString(), 'size + 1')
  assert.equal(size.add(1).toString(), 'size + 1')
  assert.equal(size.add(Scalar.fromJs(1)).toString(), 'size + 1')
  assert.equal(size.subtract('1').toString(), 'size - 1')
  assert.equal(size.multiply('2').toString(), 'size * 2')
  assert.equal(size.divide('2').toString(), 'size / 2')
  assert.equal(size.remainder('2').toString(), 'size % 2')
  assert.equal(size.negate().toString(), '-size')

  const computed = size.add('1').bind(TRADES).eval(Scalar.fromJs(['EUR', null, 4]))
  assert.ok(computed.equals(Scalar.fromJs(5)))
  for (const hidden of [
    '_addNative',
    '_subtractNative',
    '_multiplyNative',
    '_divideNative',
    '_remainderNative',
  ]) {
    assert.equal(size[hidden], undefined, hidden)
  }
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
  assert.equal(bound.matches(Scalar.fromJs(['EUR', null, 5])), true)
  assert.equal(bound.matches(Scalar.fromJs(['USD', null, 5])), false)
  assert.equal(bound.matches(Scalar.fromJs(['EUR', null, null])), false)
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
  const clone = statement.clone()
  assert.notEqual(clone, statement)
  assert.ok(clone.equals(statement))
  assert.ok(new Statement(statement).equals(statement))
  assert.equal(clone.compare(statement), 0)
  assert.ok(new Statement('select ccy').compare('select price') < 0)
  assert.equal(clone.stableHash(), statement.stableHash())
  assert.deepEqual(JSON.parse(JSON.stringify(statement)), JSON.parse(statement.intoJson()))
})

const STATEMENT_SCHEMA = new Field(
  'rows',
  'struct(field("ccy",utf8,nullable=true,metadata={}),field("size",int64,nullable=true,metadata={}))',
  false,
)

function statementBatch(ccy, size) {
  return new arrow.Table({
    ccy: arrow.vectorFromArray(ccy, new arrow.Utf8()),
    size: arrow.vectorFromArray(size, new arrow.Int64()),
  }).batches[0]
}

test('statement ordering, binding, and all are native views', () => {
  const statement = new Statement(
    'select ccy, size as quantity where size >= :floor ' +
      'order by size desc nulls last limit 2',
  )
  const [order] = statement.ordering
  assert.equal(order.expression.toString(), 'size')
  assert.equal(order.direction, 'descending')
  assert.equal(order.nulls, 'last')
  assert.equal(statement.isAll, false)

  assert.throws(() => statement.bind(STATEMENT_SCHEMA), /floor/)
  const bound = statement.bind(STATEMENT_SCHEMA, { floor: 2 })
  assert.ok(bound.schema.equals(STATEMENT_SCHEMA))
  assert.equal(bound.output.name, 'rows')
  assert.deepEqual(bound.projections.map((projection) => projection.field.name), ['ccy', 'size'])
  assert.equal(bound.predicate.isPredicate, true)
  assert.deepEqual(bound.ordering[0].expression.columns, ['size'])
  assert.equal(bound.ordering[0].direction, 'descending')
  assert.equal(bound.ordering[0].nulls, 'last')
  assert.equal(bound.limit, 2)
  assert.equal(bound.isAll, false)

  const all = new Statement('select *').bind(STATEMENT_SCHEMA)
  assert.equal(all.isAll, true)
  assert.deepEqual(all.projections, [])
})

test('bound statement projection preserves each Arrow holder', () => {
  const bound = new Statement(
    'select ccy, size as quantity where size >= 2 limit 2',
  ).bind(STATEMENT_SCHEMA)
  const first = statementBatch(['A', 'B', 'C', 'D'], [1n, 4n, 3n, null])
  const second = statementBatch(['E', 'F'], [5n, 6n])

  const projectedBatch = bound.projectArrowRecordBatch(first)
  assert.ok(arrow.isArrowRecordBatch(projectedBatch))
  assert.deepEqual([...projectedBatch.getChild('quantity')], [4n, 3n])
  assert.ok(arrow.isArrowRecordBatch(bound.projectArrow(first)))

  const table = new arrow.Table([first, second])
  const projectedTable = bound.projectArrowTable(table)
  assert.ok(arrow.isArrowTable(projectedTable))
  assert.deepEqual([...projectedTable.getChild('quantity')], [4n, 3n])
  assert.ok(arrow.isArrowTable(bound.projectArrow(table)))

  const projectedReader = bound.projectArrowReader(
    BatchReader.from(new arrow.Table([first, second])),
  )
  assert.ok(projectedReader instanceof BatchReader)
  assert.deepEqual([...projectedReader.intoTable().getChild('quantity')], [4n, 3n])
  const inferredReader = bound.projectArrow(
    BatchReader.from(new arrow.Table([first, second])),
  )
  assert.ok(inferredReader instanceof BatchReader)
  assert.equal(inferredReader.intoTable().numRows, 2)

  assert.throws(() => bound.projectArrowTable(first), /Arrow Table/)
  assert.equal(bound._projectArrowReaderNative, undefined)
})

test('bound statement sorts one materialized batch', () => {
  const bound = new Statement('select * order by size desc nulls last limit 2').bind(
    STATEMENT_SCHEMA,
  )
  const source = statementBatch(['A', 'B', 'C', 'D'], [null, 2n, 5n, 3n])
  const sorted = bound.sortArrowRecordBatch(source)
  assert.ok(arrow.isArrowRecordBatch(sorted))
  assert.deepEqual([...sorted.getChild('ccy')], ['C', 'D'])
  assert.deepEqual([...sorted.getChild('size')], [5n, 3n])
  assert.equal(bound._sortArrowRecordBatchNative, undefined)
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
    const matched = [...handle.childrenMatching("&holder.partition['year'] = '2024'")]
    assert.ok(matched.length > 0)
    for (const entry of matched) {
      assert.match(String(entry.url), /year=2024/)
    }

    // The pair spelling selects the leaves, and it selects the same ones.
    const pairs = [...handle.childrenWhere({ year: '2024' })]
    assert.equal(pairs.length, 1)
    assert.match(String(pairs[0].url), /year=2024\/part-0\.parquet$/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('an expression prunes manifests before a byte is read', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-expression-'))
  try {
    const table = venues(root)

    // The baseline: a predicate no summary can settle opens every manifest, so
    // the numbers below are pruning rather than a constant.
    const whole = table.planMatching('id >= 1')
    assert.equal(whole.manifestsRead, 4)
    assert.equal(whole.manifestsSkipped, 0)
    assert.equal(whole.tasks, 4)

    // A manifest-list summary bounds each manifest's partition values, so a
    // question about the file is settled without opening the Avro.
    const held = table.planMatching("&holder.partition['venue'] = 'XNYS'")
    assert.equal(held.manifestsSkipped, 3)
    assert.equal(held.manifestsRead, 1)
    assert.equal(held.tasks, 1)
    assert.equal(held.recordCount, 1)

    // The shape the filter exists for, with both halves load-bearing: the
    // holder conjunct leaves the two XLON rows and the row conjuncts keep one
    // of them, so neither can be dropped without changing the answer.
    const mixed = table
      .scanMatching("id >= 4 and symbol is not null and &holder.partition['venue'] = 'XLON'")
      .intoTable()
    assert.deepEqual([...mixed.getChild('id')], [4n])
    assert.deepEqual([...mixed.getChild('symbol')], ['BP'])
    assert.deepEqual([...mixed.getChild('venue')], ['XLON'])
    assert.deepEqual(
      [...table.scanMatching("&holder.partition['venue'] = 'XLON'").intoTable().getChild('id')],
      [3n, 4n],
    )
    assert.deepEqual([...table.scanMatching('id >= 4').intoTable().getChild('id')], [4n])

    // The pair spelling and the expression spelling are one plan and one read.
    // Each side is pinned to the measured number, so two broken sides cannot
    // agree their way past this.
    const byPair = table.plan({ venue: 'XLON' })
    const byText = table.planMatching("venue = 'XLON'")
    assert.equal(byText.tasks, 2)
    assert.equal(byPair.filesPlanned, 2)
    assert.equal(byText.manifestsSkipped, 2)
    assert.equal(byPair.manifestsSkipped, 2)
    assert.equal(byText.recordCount, 2)
    assert.equal(byPair.recordCount, 2)
    assert.deepEqual([...table.scanMatching("venue = 'XLON'").intoTable().getChild('id')], [3n, 4n])
    assert.deepEqual([...table.scanWhere({ venue: 'XLON' }).intoTable().getChild('id')], [3n, 4n])
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})
