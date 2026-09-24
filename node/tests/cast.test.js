'use strict'

// Pins node/src/cast.rs: the ArrowCastPlan compiled once and applied to every
// column of its source layout.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')
const { ArrowCastPlan, ChunkedSerie, Field, Serie, StructSerie, fields } = require('yggdryl')

test('the private natives stay outside the public surface', () => {
  assert.equal(Object.hasOwn(ArrowCastPlan, '_compileNative'), false)
  assert.equal('_applyNative' in ArrowCastPlan.prototype, false)
  assert.equal('_applyChunkedNative' in ArrowCastPlan.prototype, false)
  assert.throws(() => new ArrowCastPlan(), /no `constructor`/)
})

test('one plan casts every column of its source layout', () => {
  const source = fields.int32('id', { nullable: false })
  const target = fields.int64('id', { nullable: false })
  const plan = ArrowCastPlan.compile(source, target)

  assert.ok(plan.source.equals(source))
  assert.ok(plan.target.equals(target))
  assert.deepEqual(plan.options, { safe: true, nullability: 'default', representation: 'value' })
  assert.equal(plan.isIdentity, false)
  plan.preflight()

  for (const values of [[1, 2], [3], []]) {
    const cast = plan.apply(Serie.fromScalars(source, values))
    assert.ok(cast.field.equals(target))
    assert.deepEqual(cast.asJs(), values)
  }

  // A column of another layout is refused, naming both.
  assert.throws(
    () => plan.apply(Serie.fromScalars(fields.utf8('id'), ['1'])),
    /lays out as Utf8, and this cast plan was compiled for Int32/,
  )
  // A run has no layout for a plan to read.
  assert.throws(() => plan.apply(new Serie([1])), /run/)
  assert.throws(() => plan.apply([1, 2]), /ArrowCastPlan.apply takes a Serie or a ChunkedSerie/)

  // An equal layout is the identity, and hands the column back.
  const same = ArrowCastPlan.compile(source, 'id: int32 not null')
  assert.equal(same.isIdentity, true)
  const ids = Serie.fromScalars(source, [1, 2])
  assert.ok(same.apply(ids).equals(ids))
})

test('the three cast answers are the plan own', () => {
  const source = fields.int64('quantity')
  const target = fields.int8('quantity', { nullable: false })
  const overflowing = Serie.fromScalars(source, [7n, 130n, null])

  assert.deepEqual(ArrowCastPlan.compile(source, target).apply(overflowing).asJs(), [7, 0, 0])

  const strict = ArrowCastPlan.compile(source, target, { nullability: 'strict' })
  assert.deepEqual(strict.options, {
    safe: true,
    nullability: 'strict',
    representation: 'value',
  })
  assert.throws(() => strict.apply(overflowing), /required Arrow field \$\.quantity holds 2 null values/)

  const unsafe = ArrowCastPlan.compile(source, target, { safe: false })
  assert.equal(unsafe.options.safe, false)
  assert.throws(() => unsafe.apply(overflowing), /Can't cast value 130 to type Int8/)

  const bits = ArrowCastPlan.compile(fields.uint64('digest'), fields.int64('digest'), {
    representation: 'bits',
  })
  assert.equal(bits.options.representation, 'bits')
  assert.deepEqual(
    bits.apply(Serie.fromScalars(fields.uint64('digest'), [2n ** 64n - 1n])).asJs(),
    [-1],
  )

  assert.throws(
    () => ArrowCastPlan.compile(source, target, { nullability: 'lenient' }),
    /expected one of default, strict/,
  )
  assert.throws(
    () => ArrowCastPlan.compile(source, target, { strict: true }),
    /cast options take safe, nullability and representation/,
  )
})

test('an Arrow JS schema, table or batch is a record source named row', () => {
  const table = new arrow.Table({
    id: arrow.vectorFromArray([1, 2], new arrow.Int32()),
    symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
  })
  const root = fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
    nullable: false,
  })

  for (const source of [table.schema, table, table.batches[0]]) {
    const plan = ArrowCastPlan.compile(source, root)
    assert.equal(plan.source.name, 'row')
    assert.equal(plan.source.nullable, false)
    assert.ok(plan.target.equals(root))
    const cast = plan.apply(Serie.fromArrowBatch(table))
    assert.ok(cast instanceof StructSerie)
    assert.deepEqual(cast.child('id').asJs(), [1, 2])
  }

  // A required column the schemas cannot fill is refused at compile, before
  // a row exists.
  const required = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('venue: utf8 not null')],
    { nullable: false },
  )
  assert.throws(
    () => ArrowCastPlan.compile(table.schema, required, { nullability: 'strict' }),
    /required Arrow field \$\.venue is missing from the source/,
  )
})

test('one plan casts every chunk of a chunked column, kept apart', () => {
  const source = fields.int32('id', { nullable: false })
  const target = fields.int64('id', { nullable: false })
  const plan = ArrowCastPlan.compile(source, target)
  const chunked = ChunkedSerie.fromSeries(
    [Serie.fromScalars(source, [1, 2]), Serie.fromScalars(source, [3])],
    source,
  )

  const cast = plan.apply(chunked)
  assert.ok(cast instanceof ChunkedSerie)
  assert.equal(cast.numChunks, 2)
  assert.ok(cast.field.equals(target))
  assert.deepEqual(cast.asJs(), [1, 2, 3])

  // An identity plan hands the chunked column back as it is, and a foreign
  // layout is refused before any chunk.
  const same = ArrowCastPlan.compile(source, source)
  assert.ok(same.apply(chunked).equals(chunked))
  assert.throws(
    () => plan.apply(ChunkedSerie.fromSerie(Serie.fromScalars(fields.utf8('id'), ['1']))),
    /compiled for/,
  )
})
