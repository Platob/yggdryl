'use strict'

// The replay's walk: operations folded into books by the native
// `BookIterator`, the books indexed by symbol and instant, and a scenario's
// inserted events merged into the base stream. Nothing here folds, sorts
// within an instant, or decides what a book holds - the iterator does, and
// refuses what it cannot walk. What this file owns is bookkeeping over what
// it answered: where a book sits, and which stream a re-run reads.

const { graph } = require('../binding.js')

const web = require('./web.js')

/** The leaf a stream item holds: a `MarketData` answers its own. */
function leafOf(item) {
  return item instanceof graph.MarketData ? item.intoLeaf() : item
}

/**
 * The instant a stream item is walked at - its grid `snapunix`, else its
 * `currunix` - as a bigint, or `null` for an undated leaf, which states
 * neither and which the walk itself refuses.
 */
function instantOf(item) {
  const leaf = leafOf(item)
  if (leaf.currunix === undefined) return null
  return leaf.snapunix ?? leaf.currunix
}

/** Every book the native walk answers over `operations`, in walk order. */
function walk(operations, { snapshotMillis = 0, global = false } = {}) {
  return [...new graph.BookIterator(operations, snapshotMillis, global)]
}

/**
 * Books indexed for lookup: `all` in walk order, and per symbol - the book's
 * cross code, `GLOBAL` for a consolidated walk - the books and their
 * instants, both in walk order, which is instant order.
 */
function indexBooks(books) {
  const bySymbol = new Map()
  for (const book of books) {
    const symbol = book.crosscode
    let held = bySymbol.get(symbol)
    if (held === undefined) {
      held = { books: [], instants: [] }
      bySymbol.set(symbol, held)
    }
    held.books.push(book)
    held.instants.push(instantOf(book))
  }
  const instants = books.map(instantOf)
  return { all: { books, instants }, bySymbol }
}

/** The first position whose instant is at or after `at` (`strict`: after). */
function lowerBound(instants, at, strict = false) {
  let low = 0
  let high = instants.length
  while (low < high) {
    const middle = (low + high) >>> 1
    if (instants[middle] < at || (strict && instants[middle] === at)) low = middle + 1
    else high = middle
  }
  return low
}

/**
 * The books of `symbol` - every symbol when `undefined` - whose instant lies
 * in `[from, to]`, either bound open when `undefined`, in walk order; an
 * unknown symbol answers none.
 */
function booksBetween(index, symbol, from, to) {
  const held = symbol === undefined ? index.all : index.bySymbol.get(symbol)
  if (held === undefined) return []
  const begin = from === undefined ? 0 : lowerBound(held.instants, from)
  const end = to === undefined ? held.instants.length : lowerBound(held.instants, to, true)
  return held.books.slice(begin, Math.max(begin, end))
}

/**
 * The latest book of `symbol` at or before `at` - the book standing at that
 * instant - or `null` when none stands yet; the last one when `at` is
 * `undefined`.
 */
function bookAt(index, symbol, at) {
  const held = index.bySymbol.get(symbol)
  if (held === undefined) return null
  const end = at === undefined ? held.instants.length : lowerBound(held.instants, at, true)
  return end === 0 ? null : held.books[end - 1]
}

/**
 * One stream of `base` and `inserted`, both already in walk order, by instant:
 * a tie keeps the base item first, so an inserted event lands after every
 * base statement of its instant, and an undated base item keeps its place.
 */
function merged(base, inserted) {
  const out = []
  let at = 0
  for (const item of inserted) {
    const instant = instantOf(item)
    while (at < base.length) {
      const held = instantOf(base[at])
      if (held !== null && instant !== null && held > instant) break
      out.push(base[at])
      at += 1
    }
    out.push(item)
  }
  for (; at < base.length; at += 1) out.push(base[at])
  return out
}

/** The native leaf an event holds, rebuilt from its `toJSON` text by `MarketData.fromJSON`, which refuses a text that is none. */
function eventLeaf(event) {
  if (event === null || typeof event !== 'object' || typeof event.native !== 'string') {
    throw new TypeError('expected an event holding its native leaf text')
  }
  return graph.MarketData.fromJSON(event.native)
}

/**
 * The native values a scenario's events hold: `operations`, one per event in
 * the same order, when the caller holds them - as `load` answers them - and
 * otherwise each rebuilt from its `toJSON` text.
 */
function eventsOf(scenario, operations) {
  if (operations === undefined) return scenario.events.map(eventLeaf)
  if (operations.length !== scenario.events.length) {
    throw new TypeError(
      `expected one operation per scenario event: ${scenario.events.length} events, ${operations.length} operations`,
    )
  }
  return operations
}

/**
 * `leaf`, when the native walk folds it alone; its refusal, verbatim,
 * otherwise. What a scenario holds is what a re-run can walk.
 */
function admit(leaf) {
  walk([leaf])
  return leaf
}

/**
 * Re-run the replay with a scenario's events inserted: the whole merged
 * stream walked through a fresh `BookIterator`, and `from`, the earliest
 * instant the scenario affects - before it every book is the base's - or
 * `null` for a scenario with no event. `operations` are the events' native
 * leaves when the caller holds them, as `load` answers them; each event is
 * decoded from its text otherwise.
 */
async function rerun(base, scenario, options = {}, operations = undefined) {
  const { createScenario, earliestAffected } = await web()
  const books = walk(merged(base, eventsOf(scenario, operations)), options)
  const from = earliestAffected(createScenario(scenario.name), scenario)
  return { books, from: from ?? null }
}

module.exports = { walk, indexBooks, booksBetween, bookAt, merged, rerun, admit, eventsOf, instantOf, leafOf }
