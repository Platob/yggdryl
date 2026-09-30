'use strict'

// Pins node/src/media/options.rs: the record settings every encoding shares.

const assert = require('node:assert/strict')
const test = require('node:test')

const { RecordOptions } = require('yggdryl')

test('a row offset is a setting of its own, set, carried and cleared', () => {
  const options = RecordOptions.from('trades.parquet')
  assert.equal(options.rowOffset, null)

  options.rowOffset = 5
  assert.equal(options.rowOffset, 5)
  const copy = options.withRowOffset(7)
  assert.equal(copy.rowOffset, 7)
  // A `with` is a copy: the options it was taken from keep their offset.
  assert.equal(options.rowOffset, 5)
  assert.equal(options.equals(copy), false)

  for (const value of [-1, 1.5, Number.NaN]) {
    assert.throws(() => {
      options.rowOffset = value
    }, /rowOffset/)
    assert.equal(options.rowOffset, 5)
  }
  options.rowOffset = null
  assert.equal(options.rowOffset, null)
})

test("a plan's offset is the row offset, and the offset is the plan's", () => {
  const options = RecordOptions.from('trades.parquet').withPlan(
    'select id limit 3 offset 2',
  )
  assert.equal(options.rowOffset, 2)
  assert.equal(options.maxRowSize, 3)
  assert.equal(options.plan.offset, 2)
  assert.equal(options.plan.limit, 3)

  const fresh = RecordOptions.from('trades.parquet')
  fresh.plan = options.plan.toString()
  assert.ok(fresh.equals(options))
})

test('null is a value, and clears every section it names', () => {
  const options = RecordOptions.from('trades.parquet')
    .withSelect(['id'])
    .withFilter('id > 1')
    .withMergeBy(['id'])
  assert.equal(options.select.toString(), 'id')
  assert.equal(options.mergeBy.toString(), 'id')

  options.select = null
  options.filter = null
  options.mergeBy = null
  assert.ok(options.equals(RecordOptions.from('trades.parquet')))

  // The plan with no section: every section it spells is cleared.
  const planned = RecordOptions.from('trades.parquet').withPlan("select id where id > 1")
  planned.plan = null
  assert.equal(planned.select.toString(), '*')
  assert.ok(planned.equals(RecordOptions.from('trades.parquet')))
})

test('a CSV name answers the dialect, a TSV name a tab, and a tab is the TSV type', () => {
  const csv = RecordOptions.from('trades.csv')
  assert.equal(csv.mimeType.toString(), 'text/csv')
  assert.equal(csv.separator, ',')
  assert.equal(csv.quote, '"')
  assert.equal(csv.escape, null)
  assert.equal(csv.comment, null)
  assert.equal(csv.header, true)
  assert.deepEqual(csv.nullValues, [''])
  assert.equal(csv.trim, false)
  assert.equal(csv.inferRowSize, 1024)

  const tsv = RecordOptions.from('trades.tsv')
  assert.equal(tsv.mimeType.toString(), 'text/tab-separated-values')
  assert.equal(tsv.separator, '\t')
  assert.equal(tsv.quote, '"')
  // The MIME type follows the separator: the core's one rule, read here.
  assert.equal(csv.withSeparator('\t').mimeType.toString(), 'text/tab-separated-values')
  assert.equal(tsv.withSeparator(',').mimeType.toString(), 'text/csv')
  assert.ok(RecordOptions.forMimeType('text/tab-separated-values').equals(tsv))
})

test('every CSV setting is set, copied and read back', () => {
  const options = RecordOptions.from('trades.csv')

  options.separator = ';'
  options.quote = "'"
  options.escape = '\\'
  options.comment = '#'
  options.header = false
  options.nullValues = ['NA', 'null', '']
  options.trim = true
  options.inferRowSize = 16
  assert.equal(options.separator, ';')
  assert.equal(options.quote, "'")
  assert.equal(options.escape, '\\')
  assert.equal(options.comment, '#')
  assert.equal(options.header, false)
  assert.deepEqual(options.nullValues, ['NA', 'null', ''])
  assert.equal(options.trim, true)
  assert.equal(options.inferRowSize, 16)

  // `null` clears the three optional bytes.
  options.quote = null
  options.escape = null
  options.comment = null
  assert.equal(options.quote, null)
  assert.equal(options.escape, null)
  assert.equal(options.comment, null)

  // Each `with` is a copy, and the copy alone carries the setting.
  const plain = RecordOptions.from('trades.csv')
  const copies = [
    [plain.withSeparator('|'), 'separator', '|', ','],
    [plain.withQuote(null), 'quote', null, '"'],
    [plain.withEscape('\\'), 'escape', '\\', null],
    [plain.withComment('#'), 'comment', '#', null],
    [plain.withHeader(false), 'header', false, true],
    [plain.withNullValues(['NA']), 'nullValues', ['NA'], ['']],
    [plain.withTrim(true), 'trim', true, false],
    [plain.withInferRowSize(2), 'inferRowSize', 2, 1024],
  ]
  for (const [copy, name, set, kept] of copies) {
    assert.deepEqual(copy[name], set, name)
    assert.deepEqual(plain[name], kept, name)
    assert.equal(copy.equals(plain), false, name)
    assert.notEqual(copy.stableHash(), plain.stableHash(), name)
  }
  assert.ok(plain.clone().equals(plain))
  assert.equal(plain.clone().stableHash(), plain.stableHash())
  assert.equal(plain.withQuote('"').equals(plain), true)
})

test('a CSV role byte is cleared by null alone: undefined is an argument not given', () => {
  const options = RecordOptions.from('trades.csv').withEscape('\\').withComment('#')
  const unset = {}
  for (const name of ['Quote', 'Escape', 'Comment']) {
    const property = name.toLowerCase()
    for (const call of [() => options[`with${name}`](), () => options[`with${name}`](unset[property])]) {
      assert.throws(call, /none of these types `String`, `null`/, name)
    }
    assert.throws(() => {
      options[property] = undefined
    }, /none of these types `String`, `null`/)
  }
  // Nothing was cleared, and `null` still is the one spelling that clears.
  assert.equal(options.quote, '"')
  assert.equal(options.escape, '\\')
  assert.equal(options.comment, '#')
  assert.equal(options.withQuote(null).quote, null)
  assert.equal(options.withEscape(null).escape, null)
  assert.equal(options.withComment(null).comment, null)
})

test('a CSV role byte is one ASCII character no other role holds, refused by name', () => {
  const options = RecordOptions.from('trades.csv')

  // Not one character: no byte to hand over, refused here naming the property.
  for (const [name, value] of [
    ['separator', 'ab'],
    ['separator', ''],
    ['quote', '""'],
    ['escape', 'ab'],
    ['comment', '\u{1F600}'],
  ]) {
    assert.throws(() => {
      options[name] = value
    }, new RegExp(`expected one ASCII character for ${name}, got `))
  }
  // One character that is not ASCII, a line break, or another role's byte:
  // the core's refusal, at the property's own path.
  assert.throws(() => {
    options.separator = 'é'
  }, /\$\.separator: expected an ASCII byte for a separator.*got 0xe9/)
  assert.throws(() => {
    options.separator = '\n'
  }, /\$\.separator: .*got '\\n'/)
  assert.throws(() => {
    options.separator = '"'
  }, /\$\.separator: .*got '"', which is the quote/)
  assert.throws(() => {
    options.quote = ','
  }, /\$\.quote: .*which is the separator/)
  assert.throws(() => {
    options.escape = ','
  }, /\$\.escape: .*which is the separator/)
  assert.throws(() => {
    options.comment = '"'
  }, /\$\.comment: .*which is the quote/)
  // A refused setting leaves the options as they were.
  assert.equal(options.separator, ',')
  assert.equal(options.quote, '"')
  assert.equal(options.escape, null)
  assert.equal(options.comment, null)

  // The other settings' refusals.
  assert.throws(() => {
    options.nullValues = ['NA', 'NA']
  }, /\$\.null_values: expected each null spelling once, got "NA" twice/)
  assert.throws(() => {
    options.inferRowSize = 0
  }, /\$\.infer_row_size: expected at least one record/)
  for (const value of [-1, 1.5, Number.NaN]) {
    assert.throws(() => {
      options.inferRowSize = value
    }, /inferRowSize must be a non-negative whole number/)
  }
  assert.deepEqual(options.nullValues, [''])
  assert.equal(options.inferRowSize, 1024)
})

test('a CSV setting is absent on another encoding rather than invented', () => {
  for (const name of ['trades.parquet', 'trades.arrows', 'trades.avro']) {
    const options = RecordOptions.from(name)
    assert.equal(options.separator, null, name)
    assert.equal(options.quote, null, name)
    assert.equal(options.escape, null, name)
    assert.equal(options.comment, null, name)
    assert.equal(options.header, null, name)
    assert.equal(options.nullValues, null, name)
    assert.equal(options.trim, null, name)
    assert.equal(options.inferRowSize, null, name)
  }
  const parquet = RecordOptions.from('trades.parquet')
  assert.throws(() => {
    parquet.separator = ';'
  }, /\$\.separator: expected CSV options to set a separator, got application\/vnd\.apache\.parquet options/)
  assert.throws(() => {
    parquet.quote = null
  }, /\$\.quote: expected CSV options/)
  assert.throws(() => {
    parquet.header = false
  }, /\$\.header: expected CSV or Excel options/)
  assert.throws(() => {
    parquet.nullValues = []
  }, /\$\.null_values: expected CSV options/)
  assert.throws(() => {
    parquet.trim = true
  }, /\$\.trim: expected CSV options/)
  assert.throws(() => {
    parquet.inferRowSize = 4
  }, /\$\.infer_row_size: expected CSV options/)
  assert.throws(() => parquet.withSeparator(';'), /expected CSV options/)
  assert.throws(() => parquet.withComment('#'), /expected CSV options/)
})
