'use strict'

// The replay's walk: operations folded into books by the native
// `BookIterator`, and the books indexed by symbol and instant. Nothing here
// folds, sorts within an instant, or decides what a book holds - the
// iterator does, and refuses what it cannot walk. What this file owns is
// bookkeeping over what it answered: where a book sits.

const { graph } = require('../binding.js')

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

module.exports = { walk, indexBooks, booksBetween, bookAt, instantOf, leafOf }
