'use strict'
const assert = require('node:assert/strict')
const test = require('node:test')
const arrow = require('apache-arrow')
const y = require('yggdryl')
const sample = () => y.Serie.from(new arrow.Table({
  venue: arrow.vectorFromArray(['a', 'a', 'b'], new arrow.Utf8()),
  qty: arrow.vectorFromArray([1, 2, 3], new arrow.Int32()),
}))

test('key contexts and native payloads use the same stream doors', () => {
  const groups = sample().windowBy('venue as desk')
  assert.ok(groups instanceof y.KeySeries)
  assert.equal(groups.length, 2)
  const first = groups.get(0)
  assert.ok(first instanceof y.KeySerie)
  assert.equal(first.rownum, 0)
  assert.deepEqual(first.key.asJs(), ['a'])
  assert.deepEqual(first.rows.child('qty').asJs(), [1, 2])
  assert.deepEqual([...groups].map(item => item.rownum), [0, 2])
  assert.equal([...groups.intoStream()].length, 3)
  assert.equal([...sample().partitionBy('venue')].length, 2)
  const external = y.Serie.from(arrow.vectorFromArray(['x', 'x', 'y'], new arrow.Utf8()))
  assert.equal(sample().partitionBy(external).length, 2)
})

test('native row and chunk streams expose their remaining values', () => {
  const stream = sample().intoStream().intoChunkedStream(2, null)
  assert.ok(stream instanceof y.StreamChunkedSerie)
  assert.deepEqual(stream.schema.fields.map(field => field.name), ['venue', 'qty'])
  assert.equal(stream.readNextBatch().numRows, 2)
  assert.equal(stream.readAll().numRows, 1)
  assert.throws(() => stream.readNextBatch(), /already been consumed/)
  const drained = sample().intoStream().intoChunkedStream(2, null)
  assert.equal(drained.readNextBatch().numRows, 2)
  assert.equal(drained.readNextBatch().numRows, 1)
  assert.equal(drained.readNextBatch(), null)
  assert.equal(drained.readNextBatch(), null)
  const rows = sample().intoStream()
  assert.ok(rows.next().value instanceof y.Scalar)
  assert.equal(rows.collect().length, 2)
  assert.throws(() => rows.collect(), /consumed/)
})

test('generic writes retain native key and row kinds', () => {
  const fs = require('node:fs')
  const os = require('node:os')
  const path = require('node:path')
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-serie-'))
  try {
    const handle = new y.IOBase(path.join(dir, 'rows.csv'))
    for (const source of [sample().windowBy('venue'), sample().intoStream(), sample().intoChunkedStream()]) {
      assert.equal(handle.overwriteSerie(source).writtenRows, 3)
      const read = handle.readSerie()
      assert.ok(read instanceof y.Serie)
      assert.equal([...read.intoStream()].length, 3)
    }
  } finally { fs.rmSync(dir, { recursive: true }) }
})

test('streamed key payloads remain ordered and old classes are absent', () => {
  const stream = sample().intoStream().windowBy('venue')
  assert.ok(stream instanceof y.StreamKeySerie)
  const first = stream.next().value
  const second = stream.next().value
  assert.throws(() => [...first.rows.intoStream()], /window|passed|advance/)
  assert.equal([...second.rows.intoStream()].length, 1)
  assert.deepEqual([...stream], [])
  for (const name of ['SerieReader', 'SerieReaderWindows', 'SerieReaderPartitions', 'Records']) {
    assert.equal(y[name], undefined)
  }
})
