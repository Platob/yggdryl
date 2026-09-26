// Candles over executions, for the price chart: a presentation aggregate of
// what the tape already shows. A candle reads each execution's `lastpx` - the
// price it last executed at - and nothing else: an execution stating none
// adds no candle, and an order's own `price` never stands in for it. Prices
// stay decimal texts compared exactly; the bucket is a bigint instant.

import { compareDecimal } from './decimal.js'
import { parseInstant } from './instant.js'

/** The bucket index of `ns` for `bucketNs`-wide buckets starting at `origin` (floor). */
export function bucketOf(ns, origin, bucketNs) {
  if (typeof bucketNs !== 'bigint' || bucketNs <= 0n) throw new TypeError('expected a positive bigint bucket width')
  const delta = parseInstant(ns) - parseInstant(origin)
  let index = delta / bucketNs
  if (delta < 0n && delta % bucketNs !== 0n) index -= 1n
  return index
}

/**
 * Fold executions - `{ currunix, lastpx }` in the served order - into candles
 * `{ open: bigint instant, close: bigint instant, first, last, high, low,
 * count }` per bucket, in bucket order.
 */
export function aggregateCandles(executions, { origin, bucketNs }) {
  const buckets = new Map()
  for (const execution of executions) {
    const price = execution.lastpx
    if (price === null || price === undefined) continue
    const at = parseInstant(execution.currunix)
    const index = bucketOf(at, origin, bucketNs)
    let candle = buckets.get(index)
    if (candle === undefined) {
      const open = parseInstant(origin) + index * bucketNs
      candle = { open, close: open + bucketNs, first: price, last: price, high: price, low: price, count: 0 }
      buckets.set(index, candle)
    }
    candle.last = price
    if (compareDecimal(price, candle.high) > 0) candle.high = price
    if (compareDecimal(price, candle.low) < 0) candle.low = price
    candle.count += 1
  }
  return [...buckets.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([, candle]) => candle)
}
