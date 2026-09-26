'use strict'

// Where a replay's operations come from: a file of `marketdata` rows (Arrow
// IPC or Parquet), a FIX capture, or the synthetic scenario. Each loader is
// the native package's own door - `MarketData.fromArrowReader` over the
// file's batches; for a capture, the text lines under the row header, each
// parsed by the codec, walked through one lifecycle and expanded into the
// sorted market operations - and answers the operations in the order the
// core answered them, refusing a stream whose instants go back.

const path = require('node:path')

const { DataType, IOBase, TextOptions, fix, graph } = require('../binding.js')

const { refusalText } = require('./json.js')
const { synthetic } = require('./synthetic.js')
const { instantOf, walk } = require('./walk.js')

/** The clock an undated FIX message takes by default - 2024-01-02T10:15:30Z - the one the Rust suites read under. */
const SEED_SENDING_TIME = 1_704_190_530_000_000_000n

/** A file suffix naming a `marketdata` file, and what it holds. */
const ARROW_SUFFIXES = Object.freeze({ '.arrow': 'arrow', '.ipc': 'arrow', '.feather': 'arrow', '.parquet': 'parquet' })

const UTC_NANOS = new DataType('datetime64(ns,"UTC")')

/**
 * The codec's sending clock: nanoseconds as a bigint or their decimal text,
 * any other text through the datetime value door (ISO 8601), a `Date` or a
 * native `Scalar` as the codec reads it, `null` for UTC now per message.
 */
function sendingTimeOf(value) {
  if (value === null) return null
  if (typeof value === 'bigint') return UTC_NANOS.scalar(value)
  if (typeof value === 'string') return UTC_NANOS.scalar(/^-?\d+$/.test(value) ? BigInt(value) : value)
  return value
}

/** A dictionary: a `FixRegistry`, the folder one is stored in, or `null` for the process default. */
function registryOf(registry) {
  if (registry === null || registry === undefined) return null
  return typeof registry === 'string' ? fix.FixRegistry.fromHandle(registry) : registry
}

/**
 * What a row header's captures are called, in the order a line answers them:
 * the columns the text options' source field states after `body`.
 */
function captureNamesOf(options) {
  const field = options.sourceField()
  const names = Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at).name)
  return names.slice(names.indexOf('body') + 1)
}

/** Drain a stream whose items may each refuse: the items, and every refusal verbatim in order. */
function drain(stream) {
  const items = []
  const refusals = []
  for (;;) {
    let step
    try {
      step = stream.next()
    } catch (error) {
      refusals.push(refusalText(error))
      continue
    }
    if (step.done) return { items, refusals }
    items.push(step.value)
  }
}

/**
 * The market operations of a FIX capture - any location `IOBase.from` opens,
 * a missing one reading as no bytes. The capture is read from its bytes, so an
 * undated line is dated by the codec's sending clock rather than by the
 * file's modification time: the same bytes answer the same operations on
 * every machine and in every language.
 */
function readCapture(file, { rowheader, sendingTime, registry, timezone }) {
  const options = new TextOptions()
  options.rowheader = rowheader
  options.startRownum = 1n
  options.timezone = timezone
  const codec = new fix.FixCodec(registryOf(registry), {
    excludeMsgtypes: [],
    captureNames: captureNamesOf(options),
    defaultSendingTime: sendingTimeOf(sendingTime),
  })
  const messages = []
  const refusals = []
  for (const line of IOBase.fromBytes(IOBase.from(file).readBytes()).readTextLines(options)) {
    try {
      for (const message of codec.parseTextLine(line)) messages.push(message)
    } catch (error) {
      refusals.push(refusalText(error))
    }
  }
  const walked = [...codec.lifecycle(messages)]
  const operations = drain(codec.marketOperations(walked))
  return { operations: operations.items, refusals: [...refusals, ...operations.refusals] }
}

/**
 * Refuse a stream whose instants go back - `snapunix` else `currunix`, the
 * key the walk orders by - naming the two; an undated leaf is left to the
 * walk, which refuses it by name.
 */
function assertSorted(operations, id) {
  let previous = null
  let previousAt = -1
  operations.forEach((item, at) => {
    const instant = instantOf(item)
    if (instant === null) return
    if (previous !== null && instant < previous) {
      throw new RangeError(
        `source ${id}: operations[${at}] at ${instant} comes before operations[${previousAt}] at ${previous}: ` +
          'expected nondecreasing instants, snapunix else currunix',
      )
    }
    previous = instant
    previousAt = at
  })
}

/** A source identifier a URL path segment carries unescaped. */
function idOf(name) {
  return name.replace(/[^A-Za-z0-9._-]/g, '-')
}

/**
 * Load one source: `'synthetic'`, a `.arrow`/`.ipc`/`.feather`/`.parquet`
 * file of `marketdata` rows, or any other file as a FIX capture read under
 * `rowheader` (the ULBridge header by default) with the codec's `sendingTime`
 * (the seed clock by default) and `registry` (a `FixRegistry` or its folder;
 * the process default when `null`). Answers `{ id, kind, name, operations,
 * symbols, refusals }`: the operations as the core answered them, the symbols
 * the per-symbol walk emits books for, and every message the capture refused.
 */
function loadSource(spec, options = {}) {
  const {
    rowheader = fix.ULBRIDGE_ROWHEADER,
    sendingTime = SEED_SENDING_TIME,
    registry = null,
    timezone = 'UTC',
  } = options
  let kind
  let name
  let operations
  let refusals = []
  if (spec === 'synthetic') {
    kind = 'synthetic'
    name = 'synthetic'
    operations = synthetic()
  } else if (typeof spec === 'string') {
    name = path.basename(spec)
    kind = ARROW_SUFFIXES[path.extname(spec).toLowerCase()] ?? 'fix'
    if (kind === 'fix') {
      ;({ operations, refusals } = readCapture(spec, { rowheader, sendingTime, registry, timezone }))
    } else {
      operations = [...graph.MarketData.fromArrowReader(IOBase.from(spec).readArrowReader())]
    }
  } else {
    throw new TypeError(`expected 'synthetic' or a file path, got ${typeof spec}`)
  }
  const id = options.id ?? idOf(name)
  assertSorted(operations, id)
  const symbols = [...new Set(walk(operations).map((book) => book.crosscode))].sort()
  return { id, kind, name, operations, symbols, refusals }
}

module.exports = { loadSource, captureNamesOf, sendingTimeOf, ARROW_SUFFIXES, SEED_SENDING_TIME }
