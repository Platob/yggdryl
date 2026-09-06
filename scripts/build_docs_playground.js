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
 * The manifest is committed, so the same build runs on any
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

const { AsciiEnum, DataType, fields } = require('../node/binding.js')

const ROOT = path.join(__dirname, '..')
const MANIFEST = path.join(ROOT, 'docs', 'assets', 'playground.json')
const VERSION = require('../node/package.json').version
// Resolve Apache Arrow from the package that depends on it, which is how the
// documentation examples reach it too; this folder has no `node_modules`.
const arrow = require(require.resolve('apache-arrow', {
  paths: [path.join(ROOT, 'node')],
}))

// Every ASCII datatype the grammar spells: the variable form, the widths a
// commodity system actually declares, and the four registered codes.
const TYPES = [
  ['ascii', null],
  ['ascii(2)', 2],
  ['ascii(3)', 3],
  ['ascii(4)', 4],
  ['ascii(8)', 8],
  ['ascii(12)', 12],
  ['ascii(16)', 16],
  ['country', 'country'],
  ['currency', 'currency'],
  ['mic', 'mic'],
  ['cfi', 'cfi'],
  // Not a case of its own: the text column every stored row is read back
  // under, so it needs a factory here and no corpus.
  ['utf8', 'utf8'],
]
const FACTORY = new Map(TYPES)
const WIDTHS = TYPES.map(([key]) => key).filter((key) => key !== 'utf8')

/** The field factory call one ASCII datatype is declared with. */
function fieldCall(key) {
  const spec = FACTORY.get(key)
  if (spec === null) return "fields.ascii('ccy')"
  if (typeof spec === 'number') return `fields.fixedAscii('ccy', ${spec})`
  return `fields.${spec}('ccy')`
}

/** That same call, made. */
function fieldOf(key) {
  const spec = FACTORY.get(key)
  if (spec === null) return fields.ascii('ccy')
  if (typeof spec === 'number') return fields.fixedAscii('ccy', spec)
  return fields[spec]('ccy')
}

// One case per rule the datatype enforces, in its own vocabulary: currency
// codes, tickers, ISINs. `USD` is both the typical `ascii(4)` value and the
// first of the ISO 4217 codes, so it is listed once.
const ENCODE = {
  ascii: [
    ['typical', 'USD'],
    ['no width to fit', 'a note of any length at all'],
    ['empty', ''],
    ['non-ASCII', 'USÉ'],
    ['an interior NUL', 'US\u0000D'],
    ['trailing NULs', 'USD\u0000'],
    ['lower case', 'usd'],
  ],
  'ascii(2)': [
    ['typical', 'US'],
    ['ISO 3166-1', 'FR'],
    ['ISO 3166-1', 'DE'],
    ['empty', ''],
    ['exactly the width', 'GB'],
    ['one byte too long', 'USA'],
    ['non-ASCII', 'FÉ'],
    ['an interior NUL', 'U\u0000S'],
    ['trailing NULs', 'U\u0000'],
    ['lower case', 'us'],
  ],
  'ascii(3)': [
    ['typical', 'USD'],
    ['ISO 4217', 'EUR'],
    ['ISO 4217', 'JPY'],
    ['empty', ''],
    ['exactly the width', 'GBP'],
    ['one byte too long', 'USDT'],
    ['non-ASCII', 'USÉ'],
    ['an interior NUL', 'U\u0000D'],
    ['trailing NULs', 'US\u0000'],
    ['lower case', 'usd'],
  ],
  'ascii(4)': [
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
  'ascii(8)': [
    ['typical', 'AAPL'],
    ['empty', ''],
    ['exactly the width', 'GOOGL.US'],
    ['one byte too long', 'GOOGL.USA'],
    ['non-ASCII', 'NESTLÉ'],
    ['an interior NUL', 'AA\u0000PL'],
    ['trailing NULs', 'AAPL\u0000\u0000'],
    ['lower case', 'aapl'],
  ],
  'ascii(12)': [
    ['typical', 'US0378331005'],
    ['ISIN', 'GB0002634946'],
    ['empty', ''],
    ['exactly the width', 'US0378331005'],
    ['one byte too long', 'US0378331005X'],
    ['non-ASCII', 'US037833100Ä'],
    ['an interior NUL', 'US03783\u000031005'],
    ['trailing NULs', 'US037833\u0000\u0000\u0000\u0000'],
    ['lower case', 'us0378331005'],
  ],
  'ascii(16)': [
    ['typical', 'US0378331005'],
    ['empty', ''],
    ['exactly the width', 'US0378331005XNAS'],
    ['one byte too long', 'US0378331005XNASD'],
    ['non-ASCII', 'US0378331005Ä'],
    ['an interior NUL', 'US03783\u000031005'],
    ['trailing NULs', 'US0378331005\u0000\u0000\u0000\u0000'],
    ['lower case', 'us0378331005'],
  ],
  country: [
    ['typical', 'US'],
    ['ISO 3166-1', 'FR'],
    ['empty', ''],
    ['exactly the width', 'GB'],
    ['one byte too long', 'USA'],
    ['non-ASCII', 'FÉ'],
    ['lower case', 'us'],
  ],
  currency: [
    ['typical', 'USD'],
    ['ISO 4217', 'EUR'],
    ['empty', ''],
    ['exactly the width', 'GBP'],
    ['one byte too long', 'USDT'],
    ['non-ASCII', 'USÉ'],
    ['lower case', 'usd'],
  ],
  mic: [
    ['typical', 'XNAS'],
    ['ISO 10383', 'XPAR'],
    ['empty', ''],
    ['exactly the width', 'XLON'],
    ['one byte too long', 'XNASD'],
    ['non-ASCII', 'XPÄR'],
    ['lower case', 'xnas'],
  ],
  cfi: [
    ['typical', 'ESVUFR'],
    ['ISO 10962', 'DBFTFR'],
    ['empty', ''],
    ['exactly the width', 'OCASPS'],
    ['one byte too long', 'ESVUFRX'],
    ['non-ASCII', 'ESVUFÉ'],
    ['lower case', 'esvufr'],
  ],
}

// The decode direction starts at storage, so its corpus is bytes.
const DECODE = {
  ascii: [
    ['the bytes it is', [0x55, 0x53, 0x44]],
    ['longer than any width', [0x61, 0x20, 0x6c, 0x6f, 0x6e, 0x67, 0x20, 0x6e, 0x6f, 0x74, 0x65]],
    ['no bytes at all', []],
  ],
  'ascii(2)': [
    ['exactly the width', [0x55, 0x53]],
    ['padded', [0x55, 0x00]],
    ['all NUL', [0x00, 0x00]],
  ],
  'ascii(3)': [
    ['exactly the width', [0x55, 0x53, 0x44]],
    ['padded', [0x55, 0x53, 0x00]],
    ['all NUL', [0x00, 0x00, 0x00]],
  ],
  'ascii(4)': [
    ['padded', [0x55, 0x53, 0x44, 0x00]],
    ['exactly the width', [0x55, 0x53, 0x44, 0x54]],
    ['all NUL', [0x00, 0x00, 0x00, 0x00]],
  ],
  'ascii(8)': [
    ['padded', [0x41, 0x41, 0x50, 0x4c, 0x00, 0x00, 0x00, 0x00]],
    ['exactly the width', [0x47, 0x4f, 0x4f, 0x47, 0x4c, 0x2e, 0x55, 0x53]],
    ['all NUL', [0, 0, 0, 0, 0, 0, 0, 0]],
  ],
  'ascii(12)': [
    [
      'exactly the width',
      // "US0378331005" fills every one of the twelve bytes.
      [0x55, 0x53, 0x30, 0x33, 0x37, 0x38, 0x33, 0x33, 0x31, 0x30, 0x30, 0x35],
    ],
    ['padded', [0x55, 0x53, 0x30, 0x33, 0x37, 0x38, 0, 0, 0, 0, 0, 0]],
    ['all NUL', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ],
  'ascii(16)': [
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
  country: [
    ['exactly the width', [0x55, 0x53]],
    ['padded', [0x55, 0x00]],
    ['all NUL', [0x00, 0x00]],
  ],
  currency: [
    ['exactly the width', [0x55, 0x53, 0x44]],
    ['padded', [0x55, 0x53, 0x00]],
    ['all NUL', [0x00, 0x00, 0x00]],
  ],
  mic: [
    ['exactly the width', [0x58, 0x4e, 0x41, 0x53]],
    ['padded', [0x58, 0x4e, 0x00, 0x00]],
    ['all NUL', [0x00, 0x00, 0x00, 0x00]],
  ],
  cfi: [
    ['exactly the width', [0x45, 0x53, 0x56, 0x55, 0x46, 0x52]],
    ['padded', [0x45, 0x53, 0x56, 0x55, 0x00, 0x00]],
    ['all NUL', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]],
  ],
}

// The declared vocabulary the section walks: the ISO 4217 listing the package
// ships, over the `currency` datatype it is registered against.
const VOCABULARY = 'currency'
// The members the stepper walks, one per rule the naming applies.
const DECLARED = [
  ['USD', 'USD'],
  ['EUR', 'EUR'],
  ['JPY', 'JPY'],
  ['N_A', 'n/a'],
  ['_3M', '3M'],
]
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
const row = (dtype) => fields.struct('row', [fieldOf(dtype)], { nullable: false })
const rowCall = (dtype) => `fields.struct('row', [${fieldCall(dtype)}], { nullable: false })`

/** A one-element Arrow JS table of text, the input side of an encode. */
const textTable = (value) =>
  new arrow.Table({ ccy: arrow.vectorFromArray([value], new arrow.Utf8()) })
const textTableCall = (value) =>
  `new arrow.Table({ ccy: arrow.vectorFromArray([${literal(value)}], new arrow.Utf8()) })`

/** A one-element Arrow JS table of storage bytes, the input side of a decode.
 *
 * The variable form stores its own bytes under Arrow's `Binary`, so its
 * storage is that layout rather than a width's `FixedSizeBinary`.
 */
const storageTable = (dtype, bytes) =>
  new arrow.Table({
    ccy: arrow.vectorFromArray([Uint8Array.from(bytes)], storageType(dtype, bytes)),
  })
const storageType = (dtype, bytes) =>
  dtype === 'ascii' ? new arrow.Binary() : new arrow.FixedSizeBinary(bytes.length)
const storageTableCall = (dtype, bytes) =>
  `new arrow.Table({ ccy: arrow.vectorFromArray([Uint8Array.of(${bytes.join(', ')})], ` +
  `${dtype === 'ascii' ? 'new arrow.Binary()' : `new arrow.FixedSizeBinary(${bytes.length})`}) })`

/** The storage of the single row of a cast table. */
const stored = (table) => Array.from(table.getChild('ccy').get(0))

/** What one ASCII datatype is, read off a field projected to Arrow. */
const projectedField = (dtype) =>
  row(dtype).castArrow(textTable('A')).schema.fields[0]

/** The single row read back under a declared `utf8` field: the trimmed text. */
const readBack = (table) => [...row('utf8').castArrow(table).getChild('ccy')][0]

/** What every ASCII datatype is, read off a field projected to Arrow. */
function widths() {
  return WIDTHS.map((dtype) => {
    const declared = row(dtype)
    const projected = projectedField(dtype)
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
  const table = row(dtype).castArrow(storageTable(dtype, bytes))
  return {
    dtype,
    label,
    storage: bytes,
    storageHex: hex(bytes),
    storageEscaped: escapedText(bytes),
    text: readBack(table),
    call:
      `const stored = ${rowCall(dtype)}\n` +
      `  .castArrow(${storageTableCall(dtype, bytes)})\n` +
      `${rowCall('utf8')}.castArrow(stored)`,
  }
}

/** A declared vocabulary: the code each value packs into, then the enum. */
function vocabulary() {
  const dtype = new DataType(VOCABULARY)
  const prebuilt = AsciiEnum.fromLogicalName(VOCABULARY)
  const declared = new AsciiEnum(ENUM, Object.fromEntries(DECLARED))
  const codes = declared.intoMembers(dtype)

  // One step per declared member: the value, the integer its own bytes pack
  // into big-endian, and the storage those bytes are.
  const steps = DECLARED.map(([member, value]) => {
    const code = dtype.asciiPacked(value)
    const bytes = [...Buffer.from(dtype.asciiValue(code).padEnd(dtype.asciiWidth, '\0'), 'latin1')]
    return {
      member,
      value,
      code: String(code),
      generated: AsciiEnum.memberName(value),
      isPrebuilt: prebuilt.get(value) !== null,
      storageHex: hex(bytes),
      call: `${literal(VOCABULARY)} packs ${literal(value)} as ${code}n`,
    }
  })

  // The declaration rides on the field as ordinary metadata under one
  // reserved key, so it crosses Arrow beside the extension identity and reads
  // back as the enum that wrote it.
  const field = fields.currency('ccy', { nullable: false })
  field.setAsciiEnum(declared)
  const projected = fields
    .struct('row', [field], { nullable: false })
    .castArrow(textTable('USD')).schema.fields[0]

  return {
    name: ENUM,
    dtype: dtype.toString(),
    prebuilt: {
      name: VOCABULARY,
      size: prebuilt.length,
      // The native mapping has no order of its own, so the sample is sorted
      // and a regeneration is byte-identical everywhere.
      sample: Object.keys(prebuilt.members).sort().slice(0, 12),
      call: `AsciiEnum.fromLogicalName(${literal(VOCABULARY)})`,
    },
    steps,
    declaration: {
      json: declared.intoJson(),
      extensionName: projected.metadata.get('ARROW:extension:name'),
      // The reserved key the declaration travels under, read off the column
      // the package projected.
      carried: projected.metadata.get('field:enum'),
      call:
        `const ccy = ${fieldCall('currency')}\n` +
        `ccy.setAsciiEnum(new AsciiEnum(${literal(ENUM)}, ` +
        `${JSON.stringify(Object.fromEntries(DECLARED))}))\n` +
        `fields.struct('row', [ccy], { nullable: false })\n` +
        `  .castArrow(${textTableCall('USD')}).schema.fields[0].metadata`,
    },
    enum: {
      name: ENUM,
      // A member's code is its value packed big-endian, which reaches 128 bits,
      // so the manifest carries its decimal text: JSON has no bigint and a
      // number would round.
      members: Object.entries(codes)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([member, code]) => [member, String(code)]),
      call: `declared.intoEnum(${literal(VOCABULARY)})`,
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
    encode,
    decode,
    vocabulary: vocabulary(),
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
