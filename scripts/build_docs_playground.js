'use strict'

/**
 * Generate the interactive documentation's manifest from the real package.
 *
 * The JavaScript extension is a native Node addon, so a browser cannot load it,
 * and a page may not reimplement encode/decode of its own. So the page is
 * generated: this runs the published surface over a fixed corpus and writes
 * `docs/assets/playground.json`, which `docs/assets/playground.js` renders and
 * nothing else computes. Every value below - every byte, every refusal message,
 * every code - is what a real call answered.
 *
 * The manifest is committed, like the notebooks, so the same build runs on any
 * machine: fixed corpus, fixed key order, two-space JSON, LF, no timestamps and
 * no paths. `--check` proves the tree still matches what a regeneration writes,
 * comparing the text rather than the endings a checkout imposed on it.
 *
 * Usage:
 *     node scripts/build_docs_playground.js            # regenerate
 *     node scripts/build_docs_playground.js --check    # report drift, write nothing
 */

const fs = require('node:fs')
const path = require('node:path')

const { AsciiDictionary, DataType, fields } = require('../node/binding.js')

const ROOT = path.join(__dirname, '..')
const MANIFEST = path.join(ROOT, 'docs', 'assets', 'playground.json')
const VERSION = require('../node/package.json').version
// Resolve Apache Arrow from the package that depends on it, which is how the
// documentation examples reach it too; this folder has no `node_modules`.
const arrow = require(require.resolve('apache-arrow', {
  paths: [path.join(ROOT, 'node')],
}))

const WIDTHS = ['ascii32', 'ascii64', 'ascii128']

// One case per rule the width enforces, in the width's own vocabulary:
// currency codes, tickers, ISINs. `USD` is both the typical `ascii32` value and
// the first of the ISO 4217 codes, so it is listed once.
const ENCODE = {
  ascii32: [
    ['typical', 'USD'],
    ['ISO 4217', 'EUR'],
    ['ISO 4217', 'JPY'],
    ['ISO 4217', 'GBP'],
    ['empty', ''],
    ['exactly the width', 'USDT'],
    ['one byte too long', 'EUROS'],
    ['non-ASCII', 'USÉ'],
    ['an interior NUL', 'US\u0000D'],
    ['trailing NULs', 'USD\u0000'],
    ['lower case', 'usd'],
  ],
  ascii64: [
    ['typical', 'AAPL'],
    ['empty', ''],
    ['exactly the width', 'GOOGL.US'],
    ['one byte too long', 'GOOGL.USA'],
    ['non-ASCII', 'NESTLÉ'],
    ['an interior NUL', 'AA\u0000PL'],
    ['trailing NULs', 'AAPL\u0000\u0000'],
    ['lower case', 'aapl'],
  ],
  ascii128: [
    ['typical', 'US0378331005'],
    ['empty', ''],
    ['exactly the width', 'US0378331005XNAS'],
    ['one byte too long', 'US0378331005XNASD'],
    ['non-ASCII', 'US0378331005Ä'],
    ['an interior NUL', 'US03783\u000031005'],
    ['trailing NULs', 'US0378331005\u0000\u0000\u0000\u0000'],
    ['lower case', 'us0378331005'],
  ],
}

// The decode direction starts at storage, so its corpus is bytes.
const DECODE = {
  ascii32: [
    ['padded', [0x55, 0x53, 0x44, 0x00]],
    ['exactly the width', [0x55, 0x53, 0x44, 0x54]],
    ['all NUL', [0x00, 0x00, 0x00, 0x00]],
  ],
  ascii64: [
    ['padded', [0x41, 0x41, 0x50, 0x4c, 0x00, 0x00, 0x00, 0x00]],
    ['exactly the width', [0x47, 0x4f, 0x4f, 0x47, 0x4c, 0x2e, 0x55, 0x53]],
    ['all NUL', [0, 0, 0, 0, 0, 0, 0, 0]],
  ],
  ascii128: [
    [
      'padded',
      // "US0378331005" and four bytes of padding.
      [0x55, 0x53, 0x30, 0x33, 0x37, 0x38, 0x33, 0x33, 0x31, 0x30, 0x30, 0x35, 0, 0, 0, 0],
    ],
    [
      'exactly the width',
      [
        0x55, 0x53, 0x30, 0x33, 0x37, 0x38, 0x33, 0x33, 0x31, 0x30, 0x30, 0x35, 0x58, 0x4e,
        0x41, 0x53,
      ],
    ],
    ['all NUL', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ],
}

const VOCABULARY = 'ascii32'
// The stepper: a repeat answers the code of its first appearance, so `USD` and
// `EUR` come back a second time.
const PUSHED = ['USD', 'EUR', 'USD', 'JPY', 'GBP', 'EUR']
const COLUMN = ['USD', null, 'JPY', 'EUR']
const ENUM = 'Currency'

/** Render a JavaScript string literal, with every control byte escaped. */
function literal(value) {
  const escaped = value
    .replace(/\\/g, '\\\\')
    .replace(/'/g, "\\'")
    .replace(/[\u0000-\u001f\u007f]/g, (character) => {
      const code = character.charCodeAt(0).toString(16).padStart(2, '0')
      return `\\x${code}`
    })
  return `'${escaped}'`
}

/** Render storage bytes as spaced hex, the form a byte dump is read in. */
function hex(bytes) {
  return bytes.map((byte) => byte.toString(16).padStart(2, '0')).join(' ')
}

/** Render storage bytes as text, with the padding and anything unprintable shown. */
function escapedText(bytes) {
  return bytes
    .map((byte) => {
      if (byte === 0) return '\\0'
      if (byte >= 0x20 && byte <= 0x7e) return String.fromCharCode(byte)
      return `\\x${byte.toString(16).padStart(2, '0')}`
    })
    .join('')
}

/** The one non-null row of one column that a record cast accepts. */
const row = (dtype) => fields.struct('row', [fields[dtype]('ccy')], { nullable: false })
const rowCall = (dtype) => `fields.struct('row', [fields.${dtype}('ccy')], { nullable: false })`

/** A one-element Arrow JS table of text, the input side of an encode. */
const textTable = (value) =>
  new arrow.Table({ ccy: arrow.vectorFromArray([value], new arrow.Utf8()) })
const textTableCall = (value) =>
  `new arrow.Table({ ccy: arrow.vectorFromArray([${literal(value)}], new arrow.Utf8()) })`

/** A one-element Arrow JS table of storage bytes, the input side of a decode. */
const storageTable = (bytes) =>
  new arrow.Table({
    ccy: arrow.vectorFromArray(
      [Uint8Array.from(bytes)],
      new arrow.FixedSizeBinary(bytes.length),
    ),
  })
const storageTableCall = (bytes) =>
  `new arrow.Table({ ccy: arrow.vectorFromArray([Uint8Array.of(${bytes.join(', ')})], ` +
  `new arrow.FixedSizeBinary(${bytes.length})) })`

/** The storage of the single row of a cast table. */
const stored = (table) => Array.from(table.getChild('ccy').get(0))

/** The single row read back under a declared `utf8` field: the trimmed text. */
const readBack = (table) => [...row('utf8').castArrow(table).getChild('ccy')][0]

/** What the three widths are, read off a field actually projected to Arrow. */
function widths() {
  return WIDTHS.map((dtype) => {
    const declared = row(dtype)
    const projected = declared.castArrow(textTable('A')).schema.fields[0]
    const type = declared.getField('ccy').dtype
    return {
      dtype: type.toString(),
      asciiWidth: type.asciiWidth,
      kind: type.kind,
      arrow: String(projected.type),
      extensionName: projected.metadata.get('ARROW:extension:name'),
      extensionDocument: projected.metadata.get('ARROW:extension:metadata'),
      call: `${rowCall(dtype)}.castArrow(${textTableCall('A')}).schema.fields[0]`,
    }
  })
}

/** The registered names, each resolved to the width it spells. */
function logicalNames() {
  const registry = DataType.logicalNames()
  const named = {}
  for (const name of Object.keys(registry).sort()) {
    named[name] = registry[name].toString()
  }
  return named
}

/** One encode case: the padded storage and the read-back text, or the refusal. */
function encodeCase(dtype, label, input) {
  const head = {
    dtype,
    label,
    input,
    // The literal is what the page shows, so a NUL or an accented byte is
    // visible there instead of rendering as nothing.
    inputLiteral: literal(input),
    call: `${rowCall(dtype)}.castArrow(${textTableCall(input)})`,
  }
  let table
  try {
    table = row(dtype).castArrow(textTable(input))
  } catch (error) {
    return { ...head, ok: false, error: error.message }
  }
  const bytes = stored(table)
  return {
    ...head,
    ok: true,
    storage: bytes,
    storageHex: hex(bytes),
    storageEscaped: escapedText(bytes),
    readBack: readBack(table),
    // A refusal stops at the storage cast, but a stored row also reports the
    // read-back text, so its call carries the second statement that produced it.
    call:
      `const stored = ${rowCall(dtype)}\n` +
      `  .castArrow(${textTableCall(input)})\n` +
      `${rowCall('utf8')}.castArrow(stored)`,
  }
}

/** One decode case: what the package answers for a run of storage bytes. */
function decodeCase(dtype, label, bytes) {
  const table = row(dtype).castArrow(storageTable(bytes))
  return {
    dtype,
    label,
    storage: bytes,
    storageHex: hex(bytes),
    storageEscaped: escapedText(bytes),
    text: readBack(table),
    call:
      `const stored = ${rowCall(dtype)}\n` +
      `  .castArrow(${storageTableCall(bytes)})\n` +
      `${rowCall('utf8')}.castArrow(stored)`,
  }
}

/** The auto-registering vocabulary, step by step, then the column and the enum. */
function dictionary() {
  const currencies = new AsciiDictionary(VOCABULARY)
  const steps = PUSHED.map((value) => {
    const isNew = currencies.getCode(value) === null
    const code = currencies.push(value)
    return {
      value,
      code,
      isNew,
      vocabulary: currencies.values(),
      dtype: currencies.dtype.toString(),
      call: `currencies.push(${literal(value)})`,
    }
  })

  const column = currencies.intoArrowArray(COLUMN)
  const keys = Array.from(column.data[0].values)
  const members = currencies.intoEnum(ENUM)
  const inputCall = COLUMN.map((value) => (value === null ? 'null' : literal(value))).join(', ')

  return {
    dtype: currencies.dtype.toString(),
    key: currencies.key.toString(),
    values: currencies.valuesDtype.toString(),
    call: `new AsciiDictionary(${literal(VOCABULARY)})`,
    steps,
    column: {
      input: COLUMN,
      // A null key has no code, so the slot reads as null rather than as the
      // zero the keys buffer happens to hold under it.
      codes: COLUMN.map((_, index) => (column.get(index) === null ? null : keys[index])),
      call: `currencies.intoArrowArray([${inputCall}])`,
    },
    enum: {
      name: ENUM,
      members: Object.entries(members),
      call: `currencies.intoEnum(${literal(ENUM)})`,
    },
  }
}

/** Build the whole manifest, in the order it is written. */
function manifest() {
  const encode = []
  const decode = []
  for (const dtype of WIDTHS) {
    for (const [label, input] of ENCODE[dtype]) encode.push(encodeCase(dtype, label, input))
    for (const [label, bytes] of DECODE[dtype]) decode.push(decodeCase(dtype, label, bytes))
  }
  return {
    version: VERSION,
    widths: widths(),
    logicalNames: logicalNames(),
    encode,
    decode,
    dictionary: dictionary(),
  }
}

/** Count the cases: one per expression the manifest records. */
function cases(value) {
  if (Array.isArray(value)) return value.reduce((total, item) => total + cases(item), 0)
  if (value === null || typeof value !== 'object') return 0
  let total = typeof value.call === 'string' ? 1 : 0
  for (const item of Object.values(value)) total += cases(item)
  return total
}

/** Report the first line two renderings differ on, so drift names itself. */
function difference(current, wanted) {
  if (current === null) return `${path.relative(ROOT, MANIFEST)} is missing`
  const was = current.split('\n')
  const now = wanted.split('\n')
  for (let line = 0; line < Math.max(was.length, now.length); line += 1) {
    if (was[line] === now[line]) continue
    return (
      `${path.relative(ROOT, MANIFEST)} is out of date at line ${line + 1}:\n` +
      `  committed: ${was[line] ?? '<end of file>'}\n` +
      `  generated: ${now[line] ?? '<end of file>'}`
    )
  }
  return `${path.relative(ROOT, MANIFEST)} is out of date`
}

function main(argv) {
  const check = argv.includes('--check')
  const built = manifest()
  // Two spaces, LF, and a trailing newline; the key order is the order the
  // manifest is built in, so a rebuild is byte-identical everywhere.
  const wanted = `${JSON.stringify(built, null, 2)}\n`
  const groups = Object.keys(built).length - 1

  console.log(
    `playground: ${cases(built)} cases from ${groups} groups` +
      `${check ? ' checked' : ' generated'}`,
  )

  // A checkout under `core.autocrlf` materialises the committed LF manifest as
  // CRLF, which is not drift, so compare the text and keep the file's endings.
  const raw = fs.existsSync(MANIFEST) ? fs.readFileSync(MANIFEST, 'utf8') : null
  const current = raw === null ? null : raw.replace(/\r\n/g, '\n')
  if (current === wanted) return 0
  if (check) {
    console.log(`  ${difference(current, wanted)}`)
    return 1
  }
  fs.mkdirSync(path.dirname(MANIFEST), { recursive: true })
  const eol = raw !== null && raw.includes('\r\n') ? '\r\n' : '\n'
  fs.writeFileSync(MANIFEST, wanted.replace(/\n/g, eol))
  return 0
}

process.exitCode = main(process.argv.slice(2))
