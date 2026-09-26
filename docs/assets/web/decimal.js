// Decimals cross as their exact text, as the native package spells them. The
// browser never sums, sorts or re-scales one: it displays the text, compares
// two texts exactly when a chart needs a high and a low, and reads a float
// only to place a pixel.

const DECIMAL = /^(-)?(\d+)(?:\.(\d+))?$/

/** The sign, the integer digits and the fractional digits of a decimal text. */
export function decimalParts(text) {
  if (typeof text !== 'string') throw new TypeError(`expected decimal text, got ${typeof text}`)
  const match = DECIMAL.exec(text.trim())
  if (!match) throw new TypeError(`expected decimal text, got ${JSON.stringify(text)}`)
  const fraction = (match[3] ?? '').replace(/0+$/, '')
  const integer = match[2].replace(/^0+(?=\d)/, '')
  const zero = integer === '0' && fraction === ''
  return { negative: Boolean(match[1]) && !zero, integer, fraction }
}

/** Whether a text is decimal text. */
export function isDecimalText(text) {
  return typeof text === 'string' && DECIMAL.test(text.trim())
}

/**
 * The display of a decimal: its own digits, padded to `places` fractional
 * digits where it has fewer and never cut where it has more, so nothing the
 * value states is lost.
 */
export function formatDecimal(text, { places = 0, group = false } = {}) {
  if (text === null || text === undefined) return ''
  const { negative, integer, fraction } = decimalParts(text)
  const whole = group ? integer.replace(/\B(?=(\d{3})+(?!\d))/g, ' ') : integer
  const digits = fraction.length >= places ? fraction : fraction.padEnd(places, '0')
  return `${negative ? '-' : ''}${whole}${digits ? `.${digits}` : ''}`
}

/** Exact order of two decimal texts: -1, 0 or 1, with no float in between. */
export function compareDecimal(left, right) {
  const a = decimalParts(left)
  const b = decimalParts(right)
  if (a.negative !== b.negative) return a.negative ? -1 : 1
  const magnitude = compareMagnitude(a, b)
  return a.negative ? -magnitude : magnitude
}

function compareMagnitude(a, b) {
  if (a.integer.length !== b.integer.length) return a.integer.length < b.integer.length ? -1 : 1
  if (a.integer !== b.integer) return a.integer < b.integer ? -1 : 1
  const width = Math.max(a.fraction.length, b.fraction.length)
  const fa = a.fraction.padEnd(width, '0')
  const fb = b.fraction.padEnd(width, '0')
  return fa === fb ? 0 : fa < fb ? -1 : 1
}

/** The greater and the lesser of decimal texts, by exact comparison. */
export function maxDecimal(...texts) {
  return texts.reduce((held, text) => (held === undefined || compareDecimal(text, held) > 0 ? text : held), undefined)
}

export function minDecimal(...texts) {
  return texts.reduce((held, text) => (held === undefined || compareDecimal(text, held) < 0 ? text : held), undefined)
}

/** The float nearest a decimal text: for a pixel position and nothing else. */
export function toFloat(text) {
  if (text === null || text === undefined) return NaN
  decimalParts(text)
  return Number(text)
}

/**
 * The exact sum of decimal texts, as text: every text is scaled to the widest
 * fraction and summed as a `BigInt`, so no float touches a quantity. The sum
 * is trimmed of trailing fractional zeros.
 */
export function sumDecimal(texts) {
  const parts = texts.map(decimalParts)
  const width = parts.reduce((held, part) => Math.max(held, part.fraction.length), 0)
  let total = 0n
  for (const part of parts) {
    const units = BigInt(part.integer + part.fraction.padEnd(width, '0'))
    total += part.negative ? -units : units
  }
  const negative = total < 0n
  const digits = (negative ? -total : total).toString().padStart(width + 1, '0')
  const integer = digits.slice(0, digits.length - width) || '0'
  const fraction = digits.slice(digits.length - width).replace(/0+$/, '')
  return `${negative ? '-' : ''}${integer}${fraction ? `.${fraction}` : ''}`
}
