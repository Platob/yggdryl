'use strict'

// The one renderer between the native package and JSON. A book crosses as
// the native package wrote it - its `marketdata` row - read column by column
// out of the Arrow JS table, each column converted by one reader chosen once
// from its Arrow type: a decimal is its exact text, an instant the decimal
// text of its nanoseconds, a UUID its hyphenated text, a 64-bit integer its
// decimal text, a map its `[key, value]` pairs in the native key order - an
// object would move an integer-like key first and lose a `__proto__` one.
// What is not a column - a book's depth, imbalance, midpoint and hash - is
// asked of the native object; nothing here computes a market fact.

const { Type, util } = require('apache-arrow')

const { graph } = require('../binding.js')

/** The level counts a book's depth and imbalance are served at. */
const DEPTHS = Object.freeze([1, 5, 10])

/** `[{ name, dtype, nullable }]`: a native field's children, as the wire names them. */
function columnsOf(field) {
  const columns = []
  for (let at = 0; at < field.fieldLen; at += 1) {
    const child = field.fieldAt(at)
    columns.push({ name: child.name, dtype: child.dtype.toString(), nullable: child.nullable })
  }
  return columns
}

const MARKETDATA = graph.MarketData.field()

/** The names of the `marketdata` columns whose datatype `id` is one of `ids`. */
function columnsWith(...ids) {
  return Object.freeze(
    new Set(
      Array.from({ length: MARKETDATA.fieldLen }, (_, at) => MARKETDATA.fieldAt(at))
        .filter((child) => ids.includes(child.dtype.id))
        .map((child) => child.name),
    ),
  )
}

/** The `marketdata` columns holding a map: their facts cross as `[key, value]` pairs. */
const MAP_COLUMNS = columnsWith('map', 'sorted_map')

const HEX = Array.from({ length: 256 }, (_, byte) => byte.toString(16).padStart(2, '0'))
const UTF8 = new TextDecoder('utf-8', { fatal: true })
const NANOS = [1_000_000_000n, 1_000_000n, 1_000n, 1n] // Arrow's TimeUnit: SECOND, MILLISECOND, MICROSECOND, NANOSECOND

/** The exact text of `units` scaled by `10^-scale`, trailing fractional zeros trimmed. */
function decimalText(units, scale) {
  const negative = units < 0n
  const digits = (negative ? -units : units).toString()
  if (scale <= 0) return `${negative ? '-' : ''}${digits}${'0'.repeat(-scale)}`
  const padded = digits.padStart(scale + 1, '0')
  const integer = padded.slice(0, padded.length - scale)
  const fraction = padded.slice(padded.length - scale).replace(/0+$/, '')
  return `${negative ? '-' : ''}${integer}${fraction ? `.${fraction}` : ''}`
}

/** The hyphenated text of sixteen bytes. */
function uuidText(bytes) {
  let hex = ''
  for (const byte of bytes) hex += HEX[byte]
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

/** Guard a reader with the slot's validity: a null slot is `null`. */
function nullable(read) {
  return (data, index) => (data.getValid(index) ? read(data, index) : null)
}

/**
 * The reader of one Arrow type: `(data, index) -> JSON value`, where `index`
 * is a slot of the `Data` chunk. Chosen once per column; the per-row path
 * reads buffers.
 */
function readerOf(type, name) {
  switch (type.typeId) {
    case Type.Null:
      return () => null
    case Type.Bool:
      return nullable(({ offset, values }, index) => {
        const bit = offset + index
        return ((values[bit >> 3] >> (bit & 7)) & 1) === 1
      })
    case Type.Int:
      // A 64-bit column is text: its values are not safe as numbers.
      return type.bitWidth === 64
        ? nullable(({ values }, index) => values[index].toString())
        : nullable(({ values }, index) => values[index])
    case Type.Float:
      return type.precision === 0
        ? nullable(({ values }, index) => util.uint16ToFloat64(values[index]))
        : nullable(({ values }, index) => values[index])
    case Type.Utf8:
      return nullable(({ values, valueOffsets }, index) =>
        UTF8.decode(values.subarray(valueOffsets[index], valueOffsets[index + 1])),
      )
    case Type.LargeUtf8:
      return nullable(({ values, valueOffsets }, index) =>
        UTF8.decode(values.subarray(Number(valueOffsets[index]), Number(valueOffsets[index + 1]))),
      )
    case Type.Decimal: {
      const { scale } = type
      return nullable(({ values, stride }, index) => {
        // Little-endian 32-bit words, two's complement over the whole width.
        let units = 0n
        for (let word = stride - 1; word >= 0; word -= 1) units = (units << 32n) | BigInt(values[stride * index + word])
        if (values[stride * index + stride - 1] & 0x8000_0000) units -= 1n << BigInt(32 * stride)
        return decimalText(units, scale)
      })
    }
    case Type.Timestamp: {
      const factor = NANOS[type.unit]
      return nullable(({ values }, index) => (BigInt(values[index]) * factor).toString())
    }
    case Type.FixedSizeBinary: {
      const width = type.byteWidth
      if (width === 16) return nullable(({ values }, index) => uuidText(values.subarray(16 * index, 16 * index + 16)))
      return nullable(({ values }, index) => {
        let hex = ''
        for (const byte of values.subarray(width * index, width * (index + 1))) hex += HEX[byte]
        return hex
      })
    }
    case Type.List:
    case Type.LargeList: {
      const item = readerOf(type.children[0].type, `${name}[]`)
      return nullable(({ valueOffsets, children }, index) => {
        const out = []
        for (let at = Number(valueOffsets[index]); at < Number(valueOffsets[index + 1]); at += 1) {
          out.push(item(children[0], at))
        }
        return out
      })
    }
    case Type.FixedSizeList: {
      const item = readerOf(type.children[0].type, `${name}[]`)
      const size = type.listSize
      return nullable(({ children }, index) => {
        const out = []
        for (let at = size * index; at < size * (index + 1); at += 1) out.push(item(children[0], at))
        return out
      })
    }
    case Type.Map: {
      const [keyField, valueField] = type.children[0].type.children
      const key = readerOf(keyField.type, `${name}.key`)
      const value = readerOf(valueField.type, `${name}.value`)
      return nullable(({ valueOffsets, children }, index) => {
        const [keys, values] = children[0].children
        const out = []
        for (let at = valueOffsets[index]; at < valueOffsets[index + 1]; at += 1) out.push([key(keys, at), value(values, at)])
        return out
      })
    }
    case Type.Struct: {
      const fields = type.children.map((child) => [child.name, readerOf(child.type, `${name}.${child.name}`)])
      return nullable(({ children }, index) => {
        const out = {}
        for (let at = 0; at < fields.length; at += 1) out[fields[at][0]] = fields[at][1](children[at], index)
        return out
      })
    }
    default:
      throw new TypeError(`column ${name}: no JSON reading for Arrow type ${type}`)
  }
}

/**
 * The rows of a native batch stream: `{ columns, rows }`, the columns as the
 * stream's own field describes them and each row an object keyed by column
 * name, in column order. Drains the reader.
 */
function rowsOf(reader) {
  const columns = columnsOf(reader.field)
  const table = reader.intoTable()
  const rows = Array.from({ length: table.numRows }, () => ({}))
  table.schema.fields.forEach((field, column) => {
    const read = readerOf(field.type, field.name)
    let row = 0
    for (const data of table.getChildAt(column).data) {
      for (let index = 0; index < data.length; index += 1) rows[row++][field.name] = read(data, index)
    }
  })
  return { columns, rows }
}

/** `{ '1': ..., '5': ..., '10': ... }`: one native reading per served level count. */
function perDepth(read) {
  return Object.fromEntries(DEPTHS.map((levels) => [String(levels), read(levels)]))
}

/** A side's row with the readings its native side answers beside it. */
function sideJson(row, side) {
  return { ...row, depth: perDepth((levels) => side.depth(levels)), length: side.length }
}

/**
 * A book's `marketdata` row, its sides in place of the two lanes - a side's
 * `price` and `quantity` are the lane's best price and quantity, so nothing
 * is lost - and the native readings that are not columns: the imbalance and
 * the depths at each served level count, the midpoint, the median quantity,
 * the `stableHash` as text, and `isTick`, a grid step that changed nothing.
 */
function shapeBook(row, book) {
  const bid = book.bid
  const ask = book.ask
  const out = {}
  for (const [name, value] of Object.entries(row)) {
    if (name === 'bidside' || name === 'askside') continue
    if (name === 'bid') out.bid = sideJson(row.bidside, bid)
    else if (name === 'ask') out.ask = sideJson(row.askside, ask)
    else out[name] = value
  }
  out.imbalance = perDepth((levels) => book.imbalance(levels))
  out.bboMidpoint = book.bboMidpoint
  out.medianQuantity = book.medianQuantity
  out.stableHash = String(book.stableHash())
  out.isTick = row.snapunix !== null && !bid.deltas.length && !ask.deltas.length && !book.executions.length
  return out
}

/** Every book of `books` as JSON, through one Arrow stream. */
function booksJson(books) {
  if (!books.length) return []
  const { rows } = rowsOf(graph.MarketData.arrowReader(books))
  return rows.map((row, at) => shapeBook(row, books[at]))
}

/** One book as JSON: `booksJson([book])[0]`. */
function bookJson(book) {
  return booksJson([book])[0]
}

/** The message a refusal carries, verbatim. */
function refusalText(error) {
  return error instanceof Error ? error.message : String(error)
}

/** `JSON.stringify` with every bigint as its decimal text. */
function toJson(value, space) {
  return JSON.stringify(value, (_, held) => (typeof held === 'bigint' ? held.toString() : held), space)
}

module.exports = {
  DEPTHS,
  MAP_COLUMNS,
  bookJson,
  booksJson,
  decimalText,
  refusalText,
  rowsOf,
  toJson,
}
