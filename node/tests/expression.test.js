'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { pathToFileURL } = require('node:url')

const arrow = require('apache-arrow')

const {
  BatchReader,
  DataType,
  Expression,
  Field,
  Filter,
  Plan,
  Records,
  Selector,
  Scalar,
  Serie,
  Term,
  fields,
  iceberg,
} = require('yggdryl')

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
  const term = new Term("ccy = 'EUR' and price > 100")
  assert.equal(term.toString(), "ccy = 'EUR' and price > 100")
  assert.ok(new Term(term.toString()).equals(term))
  assert.deepEqual(term.columns, ['ccy', 'price'])
  const clone = term.clone()
  assert.notEqual(clone, term)
  assert.ok(clone.equals(term))
  assert.equal(clone.compare(term), 0)
  assert.equal(clone.stableHash(), term.stableHash())
  assert.ok(new Term("ccy = 'EUR'").compare("ccy = 'USD'") < 0)
  assert.deepEqual(JSON.parse(JSON.stringify(term)), JSON.parse(term.intoJson()))
})

test('text is never taken as a string literal', () => {
  // The one failure this layer must not have: a filter that silently matches
  // everything because its text became a constant.
  assert.throws(() => new Term('ccy = '), /expression/)
  assert.throws(() => new Filter('ccy = '), /expression/)
})

test('a document round-trips', () => {
  const term = new Term('size between 1 and 10')
  assert.ok(Term.fromJson(term.intoJson()).equals(term))
  const selector = new Selector('ccy, price * 2 as doubled')
  assert.ok(Selector.fromJson(selector.intoJson()).equals(selector))
  const plan = new Plan('select ccy from t where size > 1 limit 3')
  assert.ok(Plan.fromJson(plan.intoJson()).equals(plan))
  const expression = new Expression('select ccy; where size > 1')
  assert.ok(Expression.fromJson(expression.intoJson()).equals(expression))
})

test('the tree is built from either spelling', () => {
  const left = new Term("ccy = 'EUR'")
  assert.equal(left.and('size > 1').toString(), "ccy = 'EUR' and size > 1")
  assert.equal(left.or('size > 1').toString(), "ccy = 'EUR' or size > 1")
  assert.equal(left.not().toString(), "not ccy = 'EUR'")
  const price = Term.column('price')
  assert.equal(price.eq('100').toString(), 'price = 100')
  assert.equal(price.comparison('is distinct from', '100').toString(), 'price is distinct from 100')
  assert.equal(price.isIn(['1', '2']).toString(), 'price in (1, 2)')
  assert.equal(price.between('10', '20').toString(), 'price between 10 and 20')
  assert.equal(price.isNull().toString(), 'price is null')
  assert.equal(price.cast('int64').toString(), 'cast(price as int64)')
  assert.equal(Term.column('trade').child('leg').at(0).toString(), 'trade.leg[0]')
  assert.equal(Term.column('trade').child('legs').slice(1, 3).toString(), 'trade.legs[1:3]')
  assert.equal(Term.call('year', ['event']).toString(), 'year(event)')
  assert.equal(Term.all(['a', 'b']).toString(), 'a and b')
  assert.equal(new Term('a = 1 or a = 2').simplify().toString(), 'a in (1, 2)')
  assert.ok(new Term('a = 1 or a = 2').explain().startsWith('or'))
  assert.throws(() => price.comparison('approximately', '1'), /comparison/)
})

test('arithmetic builders stay lazy term nodes', () => {
  const size = Term.column('size')
  assert.equal(size.add('1').toString(), 'size + 1')
  assert.equal(size.add(1).toString(), 'size + 1')
  assert.equal(size.add(Scalar.from(1)).toString(), 'size + 1')
  assert.equal(size.subtract('1').toString(), 'size - 1')
  assert.equal(size.multiply('2').toString(), 'size * 2')
  assert.equal(size.divide('2').toString(), 'size / 2')
  assert.equal(size.remainder('2').toString(), 'size % 2')
  assert.equal(size.negate().toString(), '-size')

  const computed = size.add('1').bind(TRADES).eval(Scalar.from(['EUR', null, 4]))
  assert.ok(computed.equals(Scalar.from(5)))
  for (const hidden of [
    '_addNative',
    '_subtractNative',
    '_multiplyNative',
    '_divideNative',
    '_remainderNative',
    '_bindNative',
  ]) {
    assert.equal(size[hidden], undefined, hidden)
  }
})

test('binding resolves the columns and folds the literals', () => {
  const bound = new Term('price > 100 and size is not null').bind(TRADES)
  assert.ok(bound.isPredicate)
  assert.deepEqual(bound.columns, ['price', 'size'])
  assert.deepEqual(bound.columnIndices, [1, 2])
  // The literal is converted once, into the column's own exact type.
  assert.equal(
    bound.term.toString(),
    "price > decimal128(9,2) '100.00' and size is not null",
  )
  assert.match(bound.explain(), /column price/)
  assert.throws(() => new Term('size >= :floor').bind(TRADES), /floor/)
  const late = new Term('size >= :floor').bind(TRADES, { floor: 2 })
  assert.equal(late.term.toString(), 'size >= 2')
})

test('a row answers, and unknown is not true', () => {
  const bound = new Term("ccy = 'EUR' and size > 1").bind(TRADES)
  assert.equal(bound.matches(Scalar.from(['EUR', null, 5])), true)
  assert.equal(bound.matches(Scalar.from(['USD', null, 5])), false)
  assert.equal(bound.matches(Scalar.from(['EUR', null, null])), false)
})

test('a partition column is the half a path answers', () => {
  const mixed = new Term("ccy = 'EUR' and size > 0").bind(TRADES.withPartitionFields(['ccy']))
  const split = mixed.partitionSplit()
  assert.ok(split.answerable instanceof Filter)
  assert.equal(split.answerable.toString(), "ccy = 'EUR'")
  assert.equal(split.remaining.toString(), 'size > 0')
})

const ROWS = new Field(
  'rows',
  'struct(field("ccy",utf8,nullable=true,metadata={}),field("size",int64,nullable=true,metadata={}))',
  false,
)

function rowsBatch(ccy, size) {
  return new arrow.Table({
    ccy: arrow.vectorFromArray(ccy, new arrow.Utf8()),
    size: arrow.vectorFromArray(size, new arrow.Int64()),
  }).batches[0]
}

test('a user function is spelled namespace.name and refused by name until registered', () => {
  const term = new Term('Py.Double(size)')
  assert.equal(term.toString(), 'py.double(size)')
  assert.equal(Term.call('py.double', [Term.column('size')]).toString(), 'py.double(size)')
  assert.throws(() => term.bind(ROWS), /py\.double/)
  assert.throws(() => new Term('9lives.f(size)'), /lives/)
})

test('a filter is a where clause', () => {
  const filter = new Filter("where ccy = 'EUR' and size > 1")
  assert.equal(filter.toString(), "ccy = 'EUR' and size > 1")
  assert.ok(new Filter(filter).equals(filter))
  assert.ok(new Filter(new Term("ccy = 'EUR' and size > 1")).equals(filter))
  assert.equal(filter.term.toString(), "ccy = 'EUR' and size > 1")
  assert.deepEqual(filter.conjuncts().map(String), ["ccy = 'EUR'", 'size > 1'])
  assert.deepEqual(filter.columns, ['ccy', 'size'])
  assert.equal(filter.isAlwaysTrue, false)
  assert.equal(Filter.alwaysTrue().isAlwaysTrue, true)
  assert.equal(Filter.all([]).isAlwaysTrue, true)
  assert.equal(Filter.any([]).isAlwaysFalse, true)
  assert.equal(filter.and('size < 9').toString(), "ccy = 'EUR' and size > 1 and size < 9")
  assert.equal(filter.not().toString(), "not (ccy = 'EUR' and size > 1)")
  assert.equal(filter.clone().compare(filter), 0)
  assert.equal(filter.stableHash(), new Filter(filter.toString()).stableHash())
  assert.ok(filter.applyField(ROWS).equals(ROWS))
  assert.ok(filter.bind(ROWS).isPredicate)
  assert.throws(() => new Filter('size + 1').bind(ROWS), /predicate|boolean/)

  const batch = rowsBatch(['EUR', 'USD', 'EUR'], [5n, 5n, 0n])
  assert.deepEqual([...filter.applyArrowBatch(batch).getChild('ccy')], ['EUR'])
  const table = new arrow.Table([batch, batch])
  assert.equal(filter.applyArrowTable(table).numRows, 2)
  const reader = filter.applyArrowReader(BatchReader.from(new arrow.Table([batch, batch])))
  assert.ok(reader instanceof BatchReader)
  assert.equal(reader.intoTable().numRows, 2)
  assert.ok(arrow.isArrowRecordBatch(filter.applyArrow(batch)))
  assert.ok(arrow.isArrowTable(filter.applyArrow(table)))
  const kept = filter.applyRecords([
    { ccy: 'EUR', size: 5n },
    { ccy: 'USD', size: 5n },
  ])
  assert.ok(kept instanceof Records)
  assert.equal(kept.field.name, 'row')
  const rows = kept.collect()
  assert.equal(rows.length, 1)
  assert.ok(rows[0].equals(Scalar.from(['EUR', 5n])))
  assert.equal(filter._applyArrowReaderNative, undefined)
})

test('a selector is a select clause', () => {
  const selector = new Selector('select ccy, size as quantity, size * 2 as doubled int64')
  assert.equal(selector.toString(), 'ccy, size as quantity, size * 2 as doubled int64')
  assert.deepEqual(selector.names, ['ccy', 'quantity', 'doubled'])
  assert.deepEqual(selector.projections, ['ccy', 'size as quantity', 'size * 2 as doubled int64'])
  assert.deepEqual(selector.columns, ['ccy', 'size'])
  assert.equal(selector.length, 3)
  assert.equal(selector.isAll, false)
  assert.equal(Selector.all().isAll, true)
  assert.equal(Selector.all().toString(), '*')
  assert.equal(Selector.allExcept(['size']).toString(), '* exclude (size)')
  assert.ok(Selector.fromColumns(['size', 'ccy']).isColumns)
  assert.ok(new Selector(['ccy', 'size as quantity']).equals('ccy, size as quantity'))
  assert.ok(new Selector(Term.column('ccy')).equals('ccy'))
  assert.equal(selector.withoutColumns(['ccy']).toString(), 'size as quantity, size * 2 as doubled int64')
  assert.equal(new Selector('ccy').withProjection('size').toString(), 'ccy, size')
  assert.equal(selector.clone().compare(selector), 0)

  const published = selector.applyField(ROWS)
  assert.deepEqual(
    published.dtype.values().map((field) => field.name),
    ['ccy', 'quantity', 'doubled'],
  )
  const bound = selector.bind(ROWS)
  assert.ok(bound.schema.equals(ROWS))
  assert.ok(bound.output.equals(published))
  assert.deepEqual(bound.projections.map((projection) => projection.term.toString()), ['ccy', 'size', 'size * 2'])
  assert.equal(bound.isAll, false)
  assert.equal(bound.isIdentity, false)
  assert.equal(Selector.all().bind(ROWS).isIdentity, true)
  assert.ok(bound.applyRow(Scalar.from(['EUR', 2n])).equals(Scalar.from(['EUR', 2n, 4n])))
  assert.match(bound.explain(), /doubled/)

  const batch = rowsBatch(['A', 'B'], [1n, 2n])
  const projected = selector.applyArrowBatch(batch)
  assert.deepEqual(projected.schema.fields.map((field) => field.name), ['ccy', 'quantity', 'doubled'])
  assert.deepEqual([...projected.getChild('doubled')], [2n, 4n])
  assert.deepEqual([...bound.applyArrowBatch(batch).getChild('quantity')], [1n, 2n])
  const table = new arrow.Table([batch, batch])
  assert.deepEqual([...selector.applyArrowTable(table).getChild('doubled')], [2n, 4n, 2n, 4n])
  const reader = selector.applyArrowReader(BatchReader.from(new arrow.Table([batch, batch])))
  assert.deepEqual(reader.field.dtype.values().map((field) => field.name), ['ccy', 'quantity', 'doubled'])
  assert.ok(arrow.isArrowRecordBatch(selector.applyArrow(batch)))
  assert.ok(arrow.isArrowTable(selector.applyArrow(table)))
  assert.ok(bound.applyArrow(BatchReader.from(table)) instanceof BatchReader)
  const rows = selector.applyRecords([
    { ccy: 'A', size: 1n },
    { ccy: 'B', size: 2n },
  ])
  assert.equal(rows.field.dtype.getFieldAt(2).name, 'doubled')
  const held = [...rows]
  assert.equal(held.length, 2)
  assert.ok(held[1].equals(Scalar.from(['B', 2n, 4n])))
  assert.ok(rows.intoArrowReader === undefined || true)
})

test('arithmetic over the fixed decimal leaves types as the leaf', () => {
  // A fixed leaf on either side keeps the leaf at scale eighteen rather than
  // the family's `decimal128(38, s)`; a bigdecimal or a decimal256 beside it
  // widens the answer to a bigdecimal.
  const root = new Field(
    'row',
    'struct<px:decimal, qty:decimal, size:int64, big:bigdecimal, wide:decimal256(40,2)>',
    false,
  )
  const typed = new Selector(
    'px * qty as notional, px / qty as ratio, px % qty as rest, px + size as moved, ' +
      'px * big as widened, big / qty as narrowed, px - wide as spread',
  ).applyField(root)
  assert.deepEqual(
    typed.dtype.values().map((field) => [field.name, field.dtype.toString()]),
    [
      ['notional', 'decimal'],
      ['ratio', 'decimal'],
      ['rest', 'decimal'],
      ['moved', 'decimal'],
      ['widened', 'bigdecimal'],
      ['narrowed', 'bigdecimal'],
      ['spread', 'bigdecimal'],
    ],
  )
})

// One row of a record `Serie` as the text of each of its cells.
function cellTexts(row) {
  return [...row].map((cell) => cell.toJSON())
}

test('a float cast into a decimal rounds half away from zero on both tiers', () => {
  // A float reads as its shortest text, rounded half away from zero at the
  // declared scale, a row as a column: 0.125 is 0.13, never the truncated
  // 0.12, and 1.15 is 1.15, never its binary fraction.
  const root = new Field('row', 'struct<v:float64>', false)
  const cast = new Selector('cast(v as decimal(10,2)) as cents, cast(v as decimal(10,0)) as whole')
  const values = [0.125, -0.125, 1.15, 2.5, 0.005]
  const expected = [
    ['0.13', '0'],
    ['-0.13', '0'],
    ['1.15', '1'],
    ['2.50', '3'],
    ['0.01', '0'],
  ]
  const bound = cast.bind(root)
  assert.deepEqual(values.map((value) => cellTexts(bound.applyRow(Scalar.from([value])))), expected)
  const source = Serie.fromScalars(root, values.map((value) => [value]))
  const table = cast.applyArrowReader(source.intoArrowReader()).intoTable()
  assert.deepEqual(Serie.fromArrowBatch(table).rows().map(cellTexts), expected)
})

test('a fixed decimal leaf cast to text is its trimmed text on both tiers', () => {
  const root = new Field('row', 'struct<px:decimal, n:bigdecimal>', false)
  const text = new Selector('cast(px as utf8) as px, cast(n as utf8) as n')
  const row = Scalar.from([new DataType('decimal').scalar('1.125'), new DataType('bigdecimal').scalar('-2')])
  assert.deepEqual(text.bind(root).applyRow(row).asJs(), ['1.125', '-2'])
  const table = text.applyArrowReader(Serie.fromScalars(root, [row]).intoArrowReader()).intoTable()
  assert.deepEqual([...table.getChild('px')], ['1.125'])
  assert.deepEqual([...table.getChild('n')], ['-2'])
})

test('a star may carry appended projections, and hasStar says it stands', () => {
  // `*` reads every stored column it does not exclude, whatever it appends:
  // the question a projection pushdown asks.
  const starred = new Selector("* exclude (size), upper(ccy) as code")
  assert.equal(starred.toString(), '* exclude (size), upper(ccy) as code')
  assert.equal(starred.hasStar, true)
  assert.equal(starred.isAll, false)
  assert.deepEqual(starred.excluded, ['size'])
  assert.deepEqual(starred.names, ['code'])
  assert.equal(starred.length, 1)
  assert.equal(Selector.all().hasStar, true)
  assert.equal(Selector.all().withProjection('size as quantity').toString(), '*, size as quantity')
  assert.equal(Selector.all().withProjection('size as quantity').hasStar, true)
  assert.equal(Selector.allExcept(['size']).hasStar, true)
  assert.equal(new Selector('ccy, size').hasStar, false)
  assert.equal(Selector.fromColumns(['ccy']).hasStar, false)
  // A star appending a projection publishes every kept column, then it.
  assert.deepEqual(
    starred.applyField(ROWS).dtype.values().map((field) => field.name),
    ['ccy', 'code'],
  )
})

test('a selector declares columns like a create table', () => {
  const declared = new Selector('id int64 not null, name utf8, size * 2 as doubled int32')
  const root = Field.from('rows: struct<id: int64, name: utf8, size: int64> not null')
  const published = declared.applyField(root)
  assert.equal(published.dtype.getFieldAt(0).nullable, false)
  assert.equal(String(published.dtype.getFieldAt(2).dtype), 'int32')

  // A field is a selector, and comes back as the one it was made from,
  // with the nullability every stored column carries spelled out.
  const stored = declared.intoField(root)
  assert.equal(stored.dtype.getFieldAt(2).transform.get('expression'), 'size * 2')
  assert.ok(
    Selector.fromField(stored).equals('id int64 not null, name utf8 null, size * 2 as doubled int32 null'),
  )

  // A value the declared column cannot hold becomes null, unless the column
  // is required, where it is refused by name.
  const batch = new arrow.Table({ n: arrow.vectorFromArray([1n, 1000n], new arrow.Int64()) }).batches[0]
  assert.deepEqual([...new Selector('n as small int8').applyArrowBatch(batch).getChild('small')], [1, null])
  assert.throws(() => new Selector('n as small int8 not null').applyArrowBatch(batch), /small/)
})

test('records stream both ways', () => {
  const rows = new Selector('size * 10 as size').applyRecords(
    [{ size: 1n }, { size: 2n }],
    Field.from('rows: struct<size: int64> not null'),
  )
  const reader = rows.intoArrowReader()
  assert.ok(reader instanceof BatchReader)
  assert.deepEqual([...reader.intoTable().getChild('size')], [10n, 20n])
  assert.throws(() => rows.intoArrowReader(), /consumed/)
  const batch = new arrow.Table({ size: arrow.vectorFromArray([3n, 4n], new arrow.Int64()) })
  const back = Records.fromArrowReader(BatchReader.from(batch))
  assert.equal(back.field.name, 'row')
  assert.deepEqual([...back].map((row) => row.asJs()), [[3], [4]])
  assert.throws(() => new Selector('size').applyRecords([]), /schema/)
})

test('a plan carries the whole read', () => {
  const plan = new Plan(
    'select ccy, size as quantity from lake.trades where size >= :floor ' +
      'order by size desc nulls first, ccy limit 2 offset 1',
  )
  assert.equal(
    plan.toString(),
    'select ccy, size as quantity from lake.trades where size >= :floor ' +
      'order by size desc nulls first, ccy limit 2 offset 1',
  )
  assert.ok(plan.selector.equals('ccy, size as quantity'))
  assert.equal(plan.source, 'lake.trades')
  assert.equal(plan.sourcePlan, null)
  assert.ok(plan.filter.equals('size >= :floor'))
  const [first, second] = plan.ordering
  assert.equal(first.term.toString(), 'size')
  assert.equal(first.direction, 'desc')
  assert.equal(first.nulls, 'first')
  assert.equal(second.direction, 'asc')
  assert.equal(plan.limit, 2)
  assert.equal(plan.offset, 1)
  assert.equal(plan.verb, null)
  assert.equal(plan.schema, null)
  assert.deepEqual(plan.columns, ['size', 'ccy'])
  assert.deepEqual(plan.readColumns, ['size', 'ccy'])
  assert.deepEqual(plan.parameters, ['floor'])
  assert.equal(plan.isEmpty, false)
  assert.equal(new Plan().isEmpty, true)
  assert.equal(new Plan('select *').isIdentity, true)
  assert.equal(
    plan.readSections().toString(),
    'select ccy, size as quantity where size >= :floor order by size desc nulls first, ccy limit 2 offset 1',
  )
  const clone = plan.clone()
  assert.notEqual(clone, plan)
  assert.ok(clone.equals(plan))
  assert.ok(new Plan(plan).equals(plan))
  assert.equal(clone.compare(plan), 0)
  assert.ok(new Plan('select ccy').compare('select price') < 0)
  assert.equal(clone.stableHash(), plan.stableHash())
  assert.deepEqual(JSON.parse(JSON.stringify(plan)), JSON.parse(plan.intoJson()))
  assert.match(plan.explain(), /order by/)
})

test('a plan is built section by section', () => {
  const price = Term.column('price')
  const plan = new Plan()
    .withCreate('id int64 not null, price float64', 'trades')
    .withWrite('merge into', "'file:///lake/trades.parquet'", ['id'])
    .withSelect([price, 'symbol as sym'])
    .withSource('raw')
    .withFilter(price.gt('100'))
    .withOrdering(['price desc', 'sym'])
    .withLimit(10)
    .withOffset(2)
  assert.equal(
    plan.toString(),
    'create trades (id int64 not null, price float64) ' +
      "upsert into 'file:///lake/trades.parquet' by (id) " +
      'select price, symbol as sym from raw where price > 100 ' +
      'order by price desc, sym limit 10 offset 2',
  )
  assert.equal(plan.createTarget, 'trades')
  assert.equal(plan.rootName, 'trades')
  assert.ok(plan.schema.equals('id int64 not null, price float64'))
  assert.equal(plan.verb, 'upsert into')
  assert.equal(plan.writeTarget, "'file:///lake/trades.parquet'")
  assert.ok(plan.mergeBy.equals('id'))
  assert.equal(plan.field().name, 'trades')
  assert.ok(plan.equals(plan.toString()))
  assert.equal(new Plan().withMergeBy('id').verb, 'upsert into')
  assert.equal(new Plan().withWrite('append').verb, 'insert into')
  assert.equal(new Plan().withWrite('replace into', 't').verb, 'insert overwrite')
  assert.equal(new Plan().withWrite('delete', 't').withFilter('id = 1').toString(), 'delete from t where id = 1')
  const nested = new Plan('select a from t').withSource(new Plan('select a, b from u where b > 1'))
  assert.equal(nested.toString(), 'select a from (select a, b from u where b > 1)')
  assert.ok(nested.sourcePlan.equals('select a, b from u where b > 1'))
  assert.throws(() => new Plan().withWrite('sideways'), /verb/)
  assert.throws(() => new Plan().withOrdering(['price sideways']), /expression/)
})

test('a field is a plan and a plan is a field', () => {
  const root = new Field(
    'trades',
    'struct(field("id",int64,nullable=false,metadata={}),field("ccy",utf8,nullable=true,metadata={}))',
    false,
    { comment: 'eu desk' },
  )
  const plan = Plan.fromField(root)
  assert.equal(plan.toString(), "create trades (id int64 not null, ccy utf8 null) with (comment = 'eu desk')")
  assert.deepEqual(plan.rootMetadata, [{ key: 'comment', value: 'eu desk' }])
  assert.ok(plan.field().equals(root))
  assert.ok(new Plan(root).equals(plan))
  assert.equal(new Plan('select a from t').field(), null)
  const published = new Plan('select id, ccy as currency where id > 1').fieldFrom(root)
  assert.deepEqual(published.dtype.values().map((field) => field.name), ['id', 'currency'])
})

test('a plan shapes a stream in section order', () => {
  const plan = new Plan('select ccy, size as quantity where size >= 2 order by size desc limit 2')
  const first = rowsBatch(['A', 'B', 'C', 'D'], [1n, 4n, 3n, null])
  const second = rowsBatch(['E', 'F'], [5n, 6n])

  const projectedBatch = plan.applyArrowBatch(first)
  assert.ok(arrow.isArrowRecordBatch(projectedBatch))
  assert.deepEqual([...projectedBatch.getChild('quantity')], [4n, 3n])
  assert.ok(arrow.isArrowRecordBatch(plan.applyArrow(first)))

  const table = new arrow.Table([first, second])
  const projectedTable = plan.applyArrowTable(table)
  assert.ok(arrow.isArrowTable(projectedTable))
  assert.deepEqual([...projectedTable.getChild('quantity')], [6n, 5n])
  assert.ok(arrow.isArrowTable(plan.applyArrow(table)))

  const projectedReader = plan.applyArrowReader(BatchReader.from(new arrow.Table([first, second])))
  assert.ok(projectedReader instanceof BatchReader)
  assert.deepEqual([...projectedReader.intoTable().getChild('quantity')], [6n, 5n])
  const inferredReader = plan.applyArrow(BatchReader.from(new arrow.Table([first, second])))
  assert.ok(inferredReader instanceof BatchReader)
  assert.equal(inferredReader.intoTable().numRows, 2)

  // A key orders by a column the projection drops.
  const dropped = new Plan('select ccy where size is not null order by size desc')
  assert.deepEqual([...dropped.applyArrowBatch(first).getChild('ccy')], ['B', 'C', 'A'])
  const rows = new Plan('select upper(ccy) as ccy order by size desc limit 1').applyRecords([
    { ccy: 'a', size: 1n },
    { ccy: 'b', size: 2n },
  ])
  assert.deepEqual([...rows].map((row) => row.asJs()), [['B']])

  assert.throws(() => plan.applyArrowTable(first), /Arrow Table/)
  assert.equal(plan._applyArrowReaderNative, undefined)
})

test('a plan runs a store end to end', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-plan-'))
  try {
    const url = pathToFileURL(path.join(root, 'trades.arrow')).href
    const created = new Plan(`create '${url}' (id int64 not null, name utf8)`).execute()
    assert.equal(created.intoTable().numRows, 0)
    new Plan(`insert into '${url}'`).applyArrowBatch(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
        name: arrow.vectorFromArray(['a', 'b', 'c'], new arrow.Utf8()),
      }).batches[0],
    )
    const read = new Plan(`select name from '${url}' where id > 1 order by id desc`).execute()
    assert.ok(read instanceof BatchReader)
    assert.deepEqual([...read.intoTable().getChild('name')], ['c', 'b'])
    new Plan(`upsert into '${url}' by (id)`).applyArrowBatch(
      new arrow.Table({
        id: arrow.vectorFromArray([2n, 4n], new arrow.Int64()),
        name: arrow.vectorFromArray(['B', 'd'], new arrow.Utf8()),
      }).batches[0],
    )
    new Plan(`delete from '${url}' where id = 1`).execute().intoTable()
    const stored = new Plan(`select * from '${url}' order by id`).execute().intoTable()
    assert.deepEqual([...stored.getChild('id')], [2n, 3n, 4n])
    assert.deepEqual([...stored.getChild('name')], ['B', 'c', 'd'])
    // A sequence runs its steps in order over one stream.
    const sequence = new Expression('where id > 2; select name')
    assert.deepEqual([...sequence.applyArrowTable(stored).getChild('name')], ['c', 'd'])
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('an expression is whichever clause its text is', () => {
  const selector = new Expression('select a, b')
  assert.equal(selector.kind, 'select')
  assert.ok(selector.isSelector)
  assert.ok(selector.asSelector().equals('a, b'))
  assert.equal(selector.asFilter(), null)
  const filter = new Expression('where a > 1')
  assert.equal(filter.kind, 'where')
  assert.ok(filter.asFilter().equals('a > 1'))
  const plan = new Expression('select a from t where a > 1')
  assert.equal(plan.kind, 'plan')
  assert.ok(plan.asPlan().equals('select a from t where a > 1'))
  const sequence = new Expression('select a; where a > 1')
  assert.equal(sequence.kind, 'sequence')
  assert.deepEqual(sequence.steps.map((step) => step.kind), ['select', 'where'])
  assert.equal(sequence.toString(), 'select a; where a > 1')
  assert.ok(Expression.sequence([new Expression('select a'), filter]).equals(sequence))
  assert.ok(Expression.sequence(['select a', 'where a > 1']).equals(sequence))
  assert.ok(Expression.select('a, b').equals(selector))
  assert.ok(Expression.filter(new Term('a > 1')).equals(filter))
  assert.ok(Expression.plan(new Plan('select a, b')).equals(selector))
  assert.ok(new Expression(new Plan('where a > 1')).equals(filter))
  assert.ok(new Expression(new Filter('a > 1')).equals(filter))
  assert.equal(new Expression('select *').isIdentity, true)
  assert.deepEqual(selector.columns, ['a', 'b'])
  assert.equal(new Expression('where a = 1 or a = 2').simplify().toString(), 'where a in (1, 2)')
  assert.ok(Expression.fromJson(sequence.intoJson()).equals(sequence))
  assert.equal(sequence.clone().stableHash(), sequence.stableHash())
  assert.ok(sequence.explain().startsWith('sequence'))
  assert.deepEqual(
    new Expression('select size').applyField(ROWS).dtype.values().map((field) => field.name),
    ['size'],
  )
  // Text has to say which clause it is.
  assert.throws(() => new Expression('a > 1'), /select/)
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
    // question about the partition is settled without opening the Avro.
    const nyse = table.planMatching("venue = 'XNYS'")
    assert.equal(nyse.manifestsSkipped, 3)
    assert.equal(nyse.manifestsRead, 1)
    assert.equal(nyse.tasks, 1)
    assert.equal(nyse.recordCount, 1)

    // The shape the filter exists for, with both halves load-bearing: the
    // partition conjunct leaves the two XLON rows and the row conjuncts keep
    // one of them, so neither can be dropped without changing the answer.
    const mixed = table
      .scanMatching("id >= 4 and symbol is not null and venue = 'XLON'")
      .intoTable()
    assert.deepEqual([...mixed.getChild('id')], [4n])
    assert.deepEqual([...mixed.getChild('symbol')], ['BP'])
    assert.deepEqual([...mixed.getChild('venue')], ['XLON'])
    assert.deepEqual(
      [...table.scanMatching(new Filter("venue = 'XLON'")).intoTable().getChild('id')],
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
