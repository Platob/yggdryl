// Instants are `bigint` nanoseconds since the Unix epoch, UTC, as the native
// package hands them over. A `Number` holds 53 bits and a nanosecond instant
// needs 63, so nothing here converts an instant to a number: a difference is
// taken first, as a bigint, and only that difference is scaled to pixels.

const NANOS_PER_SECOND = 1_000_000_000n
const NANOS_PER_MILLI = 1_000_000n

/** Read the decimal text of an instant, as the replay service spells one. */
export function parseInstant(text) {
  if (typeof text === 'bigint') return text
  if (typeof text !== 'string' || !/^-?\d+$/.test(text)) {
    throw new TypeError(`expected an instant as decimal nanoseconds, got ${JSON.stringify(text)}`)
  }
  return BigInt(text)
}

/** The decimal text of an instant, the spelling the service reads back. */
export function instantText(ns) {
  if (typeof ns !== 'bigint') throw new TypeError('expected a bigint instant')
  return ns.toString()
}

/** Floor division and the non-negative remainder, for negative instants too. */
function split(ns, unit) {
  let quotient = ns / unit
  let remainder = ns % unit
  if (remainder < 0n) {
    remainder += unit
    quotient -= 1n
  }
  return [quotient, remainder]
}

/** The calendar parts of an instant: whole seconds as a Date, nanos as a bigint. */
export function instantParts(ns) {
  const [seconds, nanos] = split(parseInstant(ns), NANOS_PER_SECOND)
  return { date: new Date(Number(seconds) * 1000), nanos }
}

/** `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`: every digit the instant has. */
export function formatInstant(ns) {
  const { date, nanos } = instantParts(ns)
  const iso = date.toISOString()
  return `${iso.slice(0, 19)}.${nanos.toString().padStart(9, '0')}Z`
}

/** `HH:MM:SS.mmm`, the clock a tape shows; the nanoseconds below stay in the value. */
export function formatClock(ns) {
  const { date, nanos } = instantParts(ns)
  const iso = date.toISOString()
  const millis = nanos / NANOS_PER_MILLI
  return `${iso.slice(11, 19)}.${millis.toString().padStart(3, '0')}`
}

/** `to - from`, a bigint. */
export function elapsed(from, to) {
  return parseInstant(to) - parseInstant(from)
}

/**
 * The pixel a difference from `origin` lands on at `nsPerPixel` nanoseconds
 * per pixel: the difference is exact, and only it is scaled, to a thousandth
 * of a pixel.
 */
export function toPixels(ns, origin, nsPerPixel) {
  if (typeof nsPerPixel !== 'bigint' || nsPerPixel <= 0n) {
    throw new TypeError('expected a positive bigint nanoseconds-per-pixel')
  }
  const delta = parseInstant(ns) - parseInstant(origin)
  return Number((delta * 1000n) / nsPerPixel) / 1000
}

/** The instant `pixels` from `origin`: the inverse of `toPixels`, truncated. */
export function fromPixels(pixels, origin, nsPerPixel) {
  if (!Number.isFinite(pixels)) throw new TypeError('expected a finite pixel offset')
  const thousandths = BigInt(Math.trunc(pixels * 1000))
  return parseInstant(origin) + (thousandths * nsPerPixel) / 1000n
}

/** The nanoseconds one pixel covers when `width` pixels span `[from, to]`. */
export function nanosPerPixel(from, to, width) {
  const span = elapsed(from, to)
  if (span <= 0n || !(width > 0)) return 1n
  const perPixel = span / BigInt(Math.trunc(width))
  return perPixel > 0n ? perPixel : 1n
}

/** Compare two instants: -1, 0 or 1. */
export function compareInstant(left, right) {
  const a = parseInstant(left)
  const b = parseInstant(right)
  return a < b ? -1 : a > b ? 1 : 0
}
