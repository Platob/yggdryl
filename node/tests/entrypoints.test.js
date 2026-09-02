'use strict'

// The explicit JavaScript record surface: intent and representation are both
// visible in the selected method, and every adapter ends at one native reader.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, IOBase, fields } = require('yggdryl')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-entrypoints-'))
}

function table(ids = [1n, 2n], venues = ['XNAS', 'XNYS']) {
  return new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
  })
}

function rowsOf(handle) {
  return handle.readArrowReader().intoTable()
}

function sourceFor(suffix, value) {
  if (suffix === 'ArrowReader') return BatchReader.from(value)
  if (suffix === 'ArrowBatch') return value.batches[0]
  return value
}

function committedSourceFor(suffix, ids, venues) {
  if (suffix !== 'Records') return sourceFor(suffix, table(ids, venues))
  return (function* records() {
    for (let index = 0; index < ids.length; index += 1) {
      yield { id: ids[index], venue: venues[index] }
    }
  })()
}

function snapshotOf(handle) {
  const stored = rowsOf(handle)
  const ids = Array.from(stored.getChild('id'))
  const venues = stored.getChild('venue').toArray()
  return ids.map((id, index) => ({ id, venue: venues[index] }))
}

function optionsForIntent(handle, intent, cadence = null) {
  let options = handle
    .recordOptions()
    .withField(handle.readArrowField())
    .withBatchSize(2)
  if (cadence !== null) options = options.withCommitRowSize(cadence)
  if (intent === 'merge') options = options.withMergeByNames(['id'])
  return options
}

function controlledAsyncFailure(values) {
  let reachedFailure
  const staged = new Promise((resolve) => {
    reachedFailure = resolve
  })
  let rejectFailure
  const failure = new Promise((_, reject) => {
    rejectFailure = reject
  })
  return {
    source: (async function* records() {
      yield* values
      reachedFailure()
      await failure
    })(),
    staged,
    fail(message) {
      rejectFailure(new Error(message))
    },
  }
}

test('reader, table, and record-batch entry points preserve explicit intent', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  for (const suffix of ['ArrowReader', 'ArrowTable', 'ArrowBatch']) {
    const handle = new IOBase(path.join(root, `${suffix}.arrows`))
    handle[`overwrite${suffix}`](sourceFor(suffix, table()))
    handle[`append${suffix}`](sourceFor(suffix, table([3n], ['XLON'])))
    handle[`merge${suffix}`](
      sourceFor(suffix, table([2n, 4n], ['XPAR', 'XTKS'])),
      handle.recordOptions().withMergeByNames(['id']),
    )

    const stored = rowsOf(handle)
    assert.equal(stored.numRows, 4, suffix)
    assert.deepEqual(Array.from(stored.getChild('id')), [1n, 2n, 3n, 4n], suffix)
    assert.deepEqual(
      stored.getChild('venue').toArray(),
      ['XNAS', 'XPAR', 'XLON', 'XTKS'],
      suffix,
    )
  }
})

test('generic write entry points dispatch every representation by mode', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  for (const suffix of ['ArrowReader', 'ArrowTable', 'ArrowBatch', 'Records']) {
    const handle = new IOBase(path.join(root, `generic-${suffix}.arrows`))
    handle[`write${suffix}`](
      committedSourceFor(suffix, [1n, 2n], ['XNAS', 'XNYS']),
      'overwrite',
    )
    handle[`write${suffix}`](
      committedSourceFor(suffix, [3n], ['XLON']),
      'append',
    )
    handle[`write${suffix}`](
      committedSourceFor(suffix, [2n, 4n], ['XPAR', 'XTKS']),
      'merge',
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.deepEqual(
      snapshotOf(handle),
      [
        { id: 1n, venue: 'XNAS' },
        { id: 2n, venue: 'XPAR' },
        { id: 3n, venue: 'XLON' },
        { id: 4n, venue: 'XTKS' },
      ],
      suffix,
    )
  }
})

test('generic write mode is required and validated before input inspection', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'generic-preflight.arrows'))
  handle.overwriteArrowTable(table())

  for (const mode of [undefined, null, 'replace']) {
    const reader = BatchReader.from(table())
    assert.throws(
      () => handle.writeArrowReader(reader, mode),
      /mode must be|unknown write mode/,
    )
    assert.equal(reader.consumed, false, `reader mode=${String(mode)}`)

    for (const suffix of ['ArrowTable', 'ArrowBatch', 'Records']) {
      let accesses = 0
      const untouched = new Proxy({}, {
        get() {
          accesses += 1
          throw new Error(`${suffix} input was inspected`)
        },
      })
      assert.throws(
        () => handle[`write${suffix}`](untouched, mode),
        /mode must be|unknown write mode/,
      )
      assert.equal(accesses, 0, `${suffix} mode=${String(mode)}`)
    }
  }
})

test('commit cadence has parity across every synchronous representation and intent', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const baseline = [
    { id: 0n, venue: 'BASE' },
    { id: 2n, venue: 'OLD' },
  ]
  const incoming = [
    { id: 1n, venue: 'A' },
    { id: 2n, venue: 'B' },
    { id: 3n, venue: 'C' },
    { id: 4n, venue: 'D' },
    { id: 5n, venue: 'E' },
  ]
  const expected = {
    overwrite: incoming,
    append: [...baseline, ...incoming],
    merge: [baseline[0], incoming[1], incoming[0], ...incoming.slice(2)],
  }

  // 1 publishes every row, 8 is larger than the stream, and 3 neither
  // divides the five rows nor the two-row conversion batch.
  for (const cadence of [1, 8, 3]) {
    for (const suffix of ['ArrowReader', 'ArrowTable', 'ArrowBatch', 'Records']) {
      for (const intent of ['overwrite', 'append', 'merge']) {
        const label = `${suffix} ${intent} commit=${cadence}`
        const handle = new IOBase(path.join(root, `${suffix}-${intent}-${cadence}.arrows`))
        handle.overwriteArrowTable(
          table(
            baseline.map(({ id }) => id),
            baseline.map(({ venue }) => venue),
          ),
        )
        const options = optionsForIntent(handle, intent, cadence)
        handle[`${intent}${suffix}`](
          committedSourceFor(
            suffix,
            incoming.map(({ id }) => id),
            incoming.map(({ venue }) => venue),
          ),
          options,
        )
        assert.deepEqual(snapshotOf(handle), expected[intent], label)
      }
    }
  }
})

test('write intent is authoritative and mergeByNames only supplies keys', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'intent.arrows'))
  handle.overwriteArrowTable(table())
  const keyed = handle.recordOptions().withMergeByNames(['id'])

  assert.throws(
    () => handle.overwriteArrowTable(table(), keyed),
    /overwrite.*merge|merge.*overwrite/i,
  )
  assert.throws(
    () => handle.appendArrowTable(table(), keyed),
    /append.*merge|merge.*append/i,
  )
  assert.throws(
    () => handle.mergeArrowTable(table()),
    /merge.*(key|mergeByNames|merge_by_names)/i,
  )
  assert.equal(rowsOf(handle).numRows, 2)
})

test('invalid intent does not convert or consume any adapter input', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'preflight.arrows'))
  handle.overwriteArrowTable(table())
  const plain = handle.recordOptions()
  const keyed = plain.withMergeByNames(['id'])

  for (const [intent, options, message] of [
    ['overwrite', keyed, /overwrite.*merge|merge.*overwrite/i],
    ['append', keyed, /append.*merge|merge.*append/i],
    ['merge', plain, /merge.*(key|mergeByNames|merge_by_names)/i],
  ]) {
    const reader = BatchReader.from(table())
    assert.throws(() => handle[`${intent}ArrowReader`](reader, options), message)
    assert.equal(reader.consumed, false, `${intent} reader`)

    for (const suffix of ['ArrowTable', 'ArrowBatch']) {
      let accesses = 0
      const throwing = new Proxy({}, {
        get() {
          accesses += 1
          throw new Error(`${suffix} was converted`)
        },
      })
      assert.throws(() => handle[`${intent}${suffix}`](throwing, options), message)
      assert.equal(accesses, 0, `${intent} ${suffix}`)
    }

    let pulls = 0
    const records = {
      [Symbol.iterator]() {
        pulls += 1
        throw new Error('records were pulled')
      },
    }
    assert.throws(() => handle[`${intent}Records`](records, options), message)
    assert.equal(pulls, 0, `${intent} records`)

    let asyncPulls = 0
    const asyncRecords = {
      [Symbol.asyncIterator]() {
        asyncPulls += 1
        throw new Error('async records were pulled')
      },
    }
    assert.throws(() => handle[`${intent}Records`](asyncRecords, options), message)
    assert.equal(asyncPulls, 0, `${intent} async records`)
  }
})

test('zero write limits never inspect any representation source', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const declared = BatchReader.from(table()).field

  for (const bound of ['maxRowSize', 'maxByteSize']) {
    for (const suffix of ['ArrowReader', 'ArrowTable', 'ArrowBatch', 'Records']) {
      for (const intent of ['overwrite', 'append', 'merge']) {
        const handle = new IOBase(path.join(root, `${bound}-${suffix}-${intent}.arrows`))
        let options = handle.recordOptions().withField(declared)
        options[bound] = 0
        if (intent === 'merge') options = options.withMergeByNames(['id'])

        let inspected = 0
        let source
        let reader
        if (suffix === 'ArrowReader') {
          reader = BatchReader.from(table())
          source = reader
        } else {
          source = new Proxy({}, {
            get() {
              inspected += 1
              throw new Error('zero-limit source was inspected')
            },
          })
        }

        const write = () => handle[`${intent}${suffix}`](source, options)
        if (intent === 'merge') {
          assert.throws(write, /max_(row|byte)_size.*merge_by_names|merge_by_names.*max_/i)
        } else {
          assert.equal(write(), undefined)
        }
        assert.equal(inspected, 0, `${bound} ${suffix} ${intent}`)
        if (reader !== undefined) assert.equal(reader.consumed, false)
        if (intent === 'overwrite') {
          assert.ok(handle.readArrowField().equals(declared))
          assert.equal(rowsOf(handle).numRows, 0)
        } else {
          assert.equal(handle.size, 0)
        }
      }
    }
  }
})

test('zero commit cadence is rejected without inspecting any source', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'zero-commit.arrows'))
  const options = handle
    .recordOptions()
    .withField(BatchReader.from(table()).field)
    .withCommitRowSize(0)
  let inspected = 0
  const source = new Proxy({}, {
    get() {
      inspected += 1
      throw new Error('zero-cadence source was inspected')
    },
  })

  for (const suffix of ['ArrowTable', 'ArrowBatch', 'Records']) {
    assert.throws(
      () => handle[`overwrite${suffix}`](source, options),
      /commit_row_size|commitRowSize/,
    )
  }
  assert.equal(inspected, 0)
})

test('representation-specific methods reject a different representation', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'shape.arrows'))
  const rows = table()

  assert.throws(() => handle.overwriteArrowReader(rows), /native BatchReader/)
  assert.throws(() => handle.overwriteArrowTable(rows.batches[0]), /Arrow JS Table/)
  assert.throws(() => handle.overwriteArrowBatch(rows), /Arrow JS RecordBatch/)
})

test('plain records infer a field and support all three intents', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'records.arrows'))

  handle.overwriteRecords([
    { id: 1n, venue: 'XNAS' },
    { id: 2n, venue: 'XNYS' },
  ])
  handle.appendRecords({ id: 3n, venue: 'XLON' })
  handle.mergeRecords(
    [
      { id: 2n, venue: 'XPAR' },
      { id: 4n, venue: 'XTKS' },
    ],
    handle.recordOptions().withMergeByNames(['id']),
  )

  const stored = rowsOf(handle)
  assert.equal(stored.numRows, 4)
  assert.deepEqual(stored.getChild('venue').toArray(), ['XNAS', 'XPAR', 'XLON', 'XTKS'])
})

test('field-class records infer and cache intoStructField', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  let accesses = 0
  class Trade {
    constructor(row) {
      Object.assign(this, row)
    }
    static get intoStructField() {
      accesses += 1
      return fields.struct(
        'trade',
        [Field.from('id: int64'), Field.from('venue: utf8')],
        { nullable: false },
      )
    }
  }

  const handle = new IOBase(path.join(root, 'decorated.arrows'))
  handle.overwriteRecords(
    [
      new Trade({ id: 1n, venue: 'XNAS' }),
      new Trade({ id: 2n, venue: 'XNYS' }),
    ],
    handle.recordOptions().withBatchSize(1),
  )
  handle.appendRecords([new Trade({ id: 3n, venue: 'XLON' })])
  assert.equal(accesses, 1)
  assert.deepEqual(
    Array.from(handle.readArrowField().dtype, (child) => child.name),
    ['id', 'venue'],
  )

  const records = [...handle.readRecords(Trade)]
  assert.ok(records.every((record) => record instanceof Trade))
  assert.equal(records.length, 3)
  assert.equal(accesses, 1)
})

test('empty records require options.field and then form a typed no-op', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'empty.arrows'))

  assert.throws(() => handle.overwriteRecords([]), /empty.*requires options\.field/i)
  const declared = handle.recordOptions().withField(
    fields.struct('row', [Field.from('id: int64')], { nullable: false }),
  )
  assert.doesNotThrow(() => handle.overwriteRecords([], declared))
})

test('a record generator crosses in bounded chunks under one atomic write', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'generated.arrows'))
  handle.overwriteArrowTable(table())
  const options = handle.recordOptions().withBatchSize(17)
  let pulled = 0

  function* records() {
    for (let id = 0; id < 257; id += 1) {
      pulled += 1
      yield { id: BigInt(id), venue: id % 2 === 0 ? 'XNAS' : 'XNYS' }
    }
  }

  handle.overwriteRecords(records(), options)
  assert.equal(pulled, 257)
  assert.equal(rowsOf(handle).numRows, 257)

  // The second chunk fails after the native overwrite has begun. One reader
  // still means one publication, so the old 257-row value remains intact.
  let malformedPulled = 0
  function* malformed() {
    for (let id = 0; id < 17; id += 1) {
      malformedPulled += 1
      yield { id: BigInt(id), venue: 'XNAS' }
    }
    malformedPulled += 1
    yield { id: 'not-an-int64', venue: 'XNYS' }
  }
  assert.throws(() => handle.overwriteRecords(malformed(), options))
  assert.equal(malformedPulled, 18)
  assert.equal(rowsOf(handle).numRows, 257)

  let defaultPulled = 0
  function* defaultBound() {
    for (let id = 0; id < 1_024; id += 1) {
      defaultPulled += 1
      yield { id: BigInt(id), venue: 'XNAS' }
    }
    defaultPulled += 1
    yield { id: 'not-an-int64', venue: 'XNYS' }
  }
  assert.throws(() => handle.overwriteRecords(defaultBound()))
  assert.equal(defaultPulled, 1_025)
  assert.equal(rowsOf(handle).numRows, 257)
})

test('an async record sequence returns a promise and uses the same adapter', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'async.arrows'))

  async function* records() {
    yield { id: 1n, venue: 'XNAS' }
    yield { id: 2n, venue: 'XNYS' }
    yield { id: 3n, venue: 'XLON' }
  }

  const pending = handle.overwriteRecords(
    records(),
    handle.recordOptions().withBatchSize(1),
  )
  assert.ok(pending instanceof Promise)
  await pending
  assert.equal(rowsOf(handle).numRows, 3)
})

test('unset async cadence keeps every intent unpublished through a source failure', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const baseline = [
    { id: 10n, venue: 'BASE' },
    { id: 20n, venue: 'KEEP' },
  ]
  const incoming = [
    { id: 20n, venue: 'UPDATED' },
    { id: 30n, venue: 'NEW' },
  ]

  for (const intent of ['overwrite', 'append', 'merge']) {
    const handle = new IOBase(path.join(root, `unbounded-${intent}.arrows`))
    handle.overwriteArrowTable(
      table(
        baseline.map(({ id }) => id),
        baseline.map(({ venue }) => venue),
      ),
    )
    const controlled = controlledAsyncFailure(incoming)
    const pending = handle[`${intent}Records`](
      controlled.source,
      optionsForIntent(handle, intent),
    )
    await controlled.staged
    assert.deepEqual(snapshotOf(handle), baseline, `${intent} staged rows`)
    controlled.fail(`unbounded ${intent} source failed`)
    await assert.rejects(pending, new RegExp(`unbounded ${intent} source failed`))
    assert.deepEqual(snapshotOf(handle), baseline, `${intent} failed rows`)
  }
})

test('bounded async records publish a non-dividing prefix for every intent', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const baseline = [
    { id: 1n, venue: 'OLD' },
    { id: 9n, venue: 'KEEP' },
  ]
  const incoming = [
    { id: 1n, venue: 'NEW' },
    { id: 2n, venue: 'TWO' },
    { id: 3n, venue: 'THREE' },
  ]
  const expected = {
    overwrite: incoming,
    append: [...baseline, ...incoming],
    merge: [incoming[0], baseline[1], ...incoming.slice(1)],
  }

  for (const intent of ['overwrite', 'append', 'merge']) {
    const handle = new IOBase(path.join(root, `async-prefix-${intent}.arrows`))
    handle.overwriteArrowTable(
      table(
        baseline.map(({ id }) => id),
        baseline.map(({ venue }) => venue),
      ),
    )
    const controlled = controlledAsyncFailure(incoming)
    const pending = handle[`${intent}Records`](
      controlled.source,
      optionsForIntent(handle, intent, 3),
    )
    await controlled.staged
    assert.deepEqual(snapshotOf(handle), expected[intent], `${intent} visible prefix`)
    controlled.fail(`bounded ${intent} source failed`)
    await assert.rejects(pending, new RegExp(`bounded ${intent} source failed`))
    assert.deepEqual(snapshotOf(handle), expected[intent], `${intent} retained prefix`)
  }
})

test('generic writeRecords retains each completed async cadence by mode', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const baseline = [
    { id: 1n, venue: 'OLD' },
    { id: 9n, venue: 'KEEP' },
  ]
  const incoming = [
    { id: 1n, venue: 'NEW' },
    { id: 2n, venue: 'TWO' },
    { id: 3n, venue: 'INCOMPLETE' },
  ]
  const visible = {
    overwrite: incoming.slice(0, 2),
    append: [...baseline, ...incoming.slice(0, 2)],
    merge: [incoming[0], baseline[1], incoming[1]],
  }

  for (const mode of ['overwrite', 'append', 'merge']) {
    const handle = new IOBase(path.join(root, `generic-async-${mode}.arrows`))
    handle.overwriteArrowTable(
      table(
        baseline.map(({ id }) => id),
        baseline.map(({ venue }) => venue),
      ),
    )
    const controlled = controlledAsyncFailure(incoming)
    const pending = handle.writeRecords(
      controlled.source,
      mode,
      optionsForIntent(handle, mode, 2),
    )
    assert.ok(pending instanceof Promise)
    await controlled.staged
    assert.deepEqual(snapshotOf(handle), visible[mode], `${mode} visible prefix`)
    controlled.fail(`generic ${mode} source failed`)
    await assert.rejects(pending, new RegExp(`generic ${mode} source failed`))
    assert.deepEqual(snapshotOf(handle), visible[mode], `${mode} retained prefix`)
  }
})

test('bounded async row and byte limits stop without another source pull', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  for (const [name, limited] of [
    ['rows', (options) => options.withMaxRowSize(3)],
    ['bytes', (options) => options.withMaxByteSize(1)],
  ]) {
    const handle = new IOBase(path.join(root, `${name}.arrows`))
    let pulls = 0
    let closed = false
    const source = {
      [Symbol.asyncIterator]() {
        return {
          async next() {
            pulls += 1
            if (pulls > 3) throw new Error('the limited source was pulled again')
            return { done: false, value: { id: BigInt(pulls), venue: 'XNAS' } }
          },
          async return() {
            closed = true
            return { done: true }
          },
        }
      },
    }
    const options = limited(
      handle.recordOptions().withBatchSize(2).withCommitRowSize(2),
    )
    await handle.overwriteRecords(source, options)
    assert.ok(closed)
    assert.equal(rowsOf(handle).numRows, name === 'rows' ? 3 : 1)
    assert.equal(pulls, name === 'rows' ? 3 : 2)
  }
})

test('async record spools are removed on success, source failure, and invalid intent', async (t) => {
  const root = scratch()
  const spoolRoot = path.join(root, 'spool')
  fs.mkdirSync(spoolRoot)
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const previous = {
    TEMP: process.env.TEMP,
    TMP: process.env.TMP,
    TMPDIR: process.env.TMPDIR,
  }
  process.env.TEMP = spoolRoot
  process.env.TMP = spoolRoot
  process.env.TMPDIR = spoolRoot
  t.after(() => {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
  })

  const handle = new IOBase(path.join(root, 'spooled.arrows'))
  const options = handle.recordOptions().withBatchSize(1)
  async function* valid() {
    yield { id: 1n, venue: 'XNAS' }
    yield { id: 2n, venue: 'XNYS' }
  }
  await handle.overwriteRecords(valid(), options)
  assert.deepEqual(fs.readdirSync(spoolRoot), [])

  async function* failing() {
    yield { id: 3n, venue: 'XLON' }
    throw new Error('async source failed')
  }
  await assert.rejects(handle.overwriteRecords(failing(), options), /async source failed/)
  assert.deepEqual(fs.readdirSync(spoolRoot), [])
  assert.equal(rowsOf(handle).numRows, 2)

  await assert.rejects(
    handle.mergeRecords(
      valid(),
      options.withMergeByNames(['missing']),
    ),
    /missing/,
  )
  assert.deepEqual(fs.readdirSync(spoolRoot), [])
  assert.equal(rowsOf(handle).numRows, 2)

  let pulls = 0
  const invalid = {
    [Symbol.asyncIterator]() {
      pulls += 1
      return valid()
    },
  }
  assert.throws(
    () => handle.overwriteRecords(invalid, options.withMergeByNames(['id'])),
    /overwrite.*merge|merge.*overwrite/i,
  )
  assert.equal(pulls, 0)
  assert.deepEqual(fs.readdirSync(spoolRoot), [])
})

test('readRecords accepts one options value, not two', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = 'application/vnd.apache.arrow.stream'
  const options = handle.recordOptions()
  assert.throws(() => handle.readRecords(options, options), /one options value/)
})
