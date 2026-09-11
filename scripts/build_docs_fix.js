'use strict'

/**
 * Generate the FIX documentation's manifests from the real package.
 *
 * The JavaScript extension is a native Node addon, so a browser cannot load
 * it. Everything the explorer states about the dictionary - every tag, name,
 * datatype, code set, lineage entry, projected column - therefore comes from
 * here: this runs the published surface over the committed dictionary and
 * writes what it answered. The page displays the native catalog and decoded
 * sample frames without rebuilding the protocol's schemas.
 *
 * docs/assets/fix.json carries the native catalog, registry counts, fixed
 * capture columns, and recorded decoded/emitted sample results. Codes and
 * lineage stay inline in their owning native Field metadata.
 *
 * The manifest is committed, so the same build runs on any machine: fixed corpus,
 * fixed key order, two-space JSON, LF, no timestamps and no paths. `--check`
 * proves the tree still matches what a regeneration writes, comparing the
 * text rather than the endings a checkout imposed on it.
 *
 * Usage:
 *     node scripts/build_docs_fix.js            # regenerate
 *     node scripts/build_docs_fix.js --check    # report drift, write nothing
 */

const fs = require('node:fs')
const path = require('node:path')

const { MimeType, Version, fix } = require('../node/binding.js')

const ROOT = path.join(__dirname, '..')
const CONFIG = path.join(ROOT, 'config', 'fix')
const ASSETS = path.join(ROOT, 'docs', 'assets')
const INDEX = path.join(ASSETS, 'fix.json')
const VERSION = require('../node/package.json').version

/** The frame whose two self-describing tags are deliberately wrong. */
const UNSEALED = 'unsealed'

const SOH = '\u0001'

// The corpus: one line per shape a session log holds, so the decoder shows
// what the package answers. Each is read by the native codec below and the
// page renders its field, value, and protocol-derived answers together.
const FRAMES = [
  [
    'order',
    'A new order, with its verb',
    'sending >> 8=FIX.4.4|9=145|35=D|49=BUYSIDE|56=VENUE|34=12|52=20240201-12:34:56.000|11=ORDER-1|55=AAPL|207=XNAS|54=1|38=100|44=10.5|40=2|59=0|60=20240201-12:34:56.123|10=072|',
  ],
  [
    'execution',
    'An execution report, filled',
    '8=FIX.4.4|35=8|49=VENUE|56=BUYSIDE|34=98|52=20240201-12:34:57.880|37=O-9|17=E-1|150=F|39=2|55=AAPL|54=1|31=10.5|32=100|14=100|151=0|6=10.5|60=20240201-12:34:57.900|10=118|',
  ],
  [
    'parties',
    'An order naming its parties, a repeating group',
    '8=FIX.4.4|35=D|49=BUYSIDE|56=VENUE|11=ORDER-2|55=BRN|54=1|38=50|44=41.25|453=2|448=BUYSIDE|447=D|452=1|448=VENUE|447=D|452=17|60=20240201-12:35:02.000|10=000|',
  ],
  [
    'marketdata',
    'A market data snapshot, two entries',
    '8=FIX.4.4|35=W|49=VENUE|56=BUYSIDE|55=BRN|268=2|269=0|270=41.20|271=500|269=1|270=41.30|271=400|10=000|',
  ],
  ['quote', 'A quote, two-sided', '8=FIX.4.4|35=S|55=BRN|132=41.20|133=41.30|134=500|135=500|60=20240201-12:35:00|10=004|'],
  [
    'logon',
    'A logon, opening the session',
    '8=FIX.4.4|35=A|49=BUYSIDE|56=VENUE|34=1|52=20240201-12:00:00.000|98=0|108=30|141=Y|1137=9|10=000|',
  ],
  ['heartbeat', 'A heartbeat', '8=FIX.4.2|35=0|49=VENUE|56=BUYSIDE|34=7|52=20240201-12:35:01|10=000|'],
  [
    'reject',
    'A session-level reject, naming what it refused',
    '8=FIX.4.4|35=3|49=VENUE|56=BUYSIDE|34=99|45=12|371=44|372=D|373=6|58=Incorrect data format for value|10=000|',
  ],
  [
    'cancelreplace',
    'A cancel/replace, carrying the order it amends',
    '8=FIX.4.4|35=G|49=BUYSIDE|56=VENUE|11=ORDER-3|41=ORDER-1|37=O-9|55=AAPL|54=1|38=150|44=10.75|40=2|60=20240201-12:36:00.000|10=000|',
  ],
  [
    'isin',
    'An instrument named by ISIN rather than by symbol',
    '8=FIX.4.4|35=D|48=US0378331005|22=4|207=XNAS|54=1|38=100|44=10.5|15=USD|10=000|',
  ],
  [
    'securitydefinition',
    'A security definition, with the instrument block',
    '8=FIX.4.4|35=d|49=VENUE|56=BUYSIDE|320=REQ-1|322=RESP-1|323=1|55=BRN|48=GB00BN7SWP63|22=4|167=FUT|207=XLON|15=GBP|200=202403|10=000|',
  ],
  [
    'bridge',
    'A bridge frame, name keys and a packed occurrence',
    '|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
  ],
  [
    'fixul',
    'A numeric frame also carrying symbolic keys',
    '8=FIX.4.4|35=D|11=ORDER-4|SYMBOL=AAPL|SIDE=1|ORDERQTY=100|10=000|',
  ],
  [
    'unknown-tag',
    'A tag no dictionary explains',
    '8=FIX.4.4|35=D|55=AAPL|54=1|38=100|9999=house-flag|10=000|',
  ],
  [
    'untyped',
    'A value that will not type, kept as it arrived',
    '8=FIX.4.4|35=D|55=AAPL|54=1|38=one hundred|44=n/a|10=000|',
  ],
  [
    'keyvalue',
    'An enriched line, no frame at all',
    'After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR',
  ],
  ['unframed', 'A line nobody framed', 'no level printed by this plugin'],
  [
    UNSEALED,
    'A frame that does not add up',
    '8=FIX.4.4|9=42|35=D|55=AAPL|54=1|38=100|44=10.5|10=999|',
  ],
]

/**
 * One corpus line with `BodyLength(9)` and `CheckSum(10)` made true.
 *
 * A frame written by hand states a length and a sum nobody computed, and a
 * page that checks them would then open on a frame that does not add up. So
 * every framed line in the corpus is sealed here, over the separator written
 * as one SOH, which is what a session actually sends.
 */
function sealed(line) {
  const start = line.indexOf('8=')
  if (start === -1 || !line.includes('|10=')) return line
  const prefix = line.slice(0, start)
  const parts = line
    .slice(start)
    .split('|')
    .filter((segment) => segment !== '')
  const body = parts.filter((segment) => !/^(?:8|9|10)=/.test(segment))
  const begin = parts[0]
  const payload = `${body.join(SOH)}${SOH}`
  const framed = `${begin}${SOH}9=${payload.length}${SOH}${payload}`
  let total = 0
  for (let at = 0; at < framed.length; at += 1) total += framed.charCodeAt(at) & 0xff
  const sum = String(total % 256).padStart(3, '0')
  return `${prefix}${framed}10=${sum}${SOH}`.replaceAll(SOH, '|')
}

/** The columns the frame view leads with, whether or not a frame filled them. */
const SHOWN = [35, 49, 56, 34, 55, 48, 22, 207, 54, 38, 44, 31, 32, 132, 133, 39, 150, 60]

/** Two spaces, LF, and a trailing newline. */
const rendered = (value) => `${JSON.stringify(value, null, 2)}\n`

/** Bytes as text a JSON string can carry and a reader can see. */
function escapedText(bytes) {
  let text = ''
  for (const byte of bytes) {
    if (byte === 0x5c) text += '\\\\'
    else if (byte >= 0x20 && byte < 0x7f) text += String.fromCharCode(byte)
    else text += `\\x${byte.toString(16).padStart(2, '0')}`
  }
  return text
}

/** The message type a line declares, as text; the classifier answers bytes. */
function msgtypeOf(bytes) {
  const held = fix.FixCodec.inferMsgtypeBytes(bytes)
  return held === null ? null : Buffer.from(held).toString('binary')
}

/** The dictionary this crate tracks, with its own fields registered. */
function dictionary() {
  return fix.FixRegistry.fromHandle(CONFIG)
}

/** One stored JSON document, or null where the field carries none. */
function document(field, key) {
  const held = field.get(key)
  return held === null ? null : JSON.parse(held)
}

/** Scalar summaries used to count the registry's metadata. */
function fieldRecords(registry) {
  const records = []
  for (const field of registry) {
    const view = field.fix
    const tag = view.tag
    if (tag === null) continue
    const codes = document(field, 'fix:codes')
    const lineage = document(field, 'fix:lineage')
    const entries = lineage === null ? [] : lineage.entries
    const record = { t: tag, n: field.name, y: field.dtype.toString() }
    if (field.display !== null && field.display !== field.name) record.d = field.display
    if (view.branch !== '') record.b = view.branch
    if (field.description !== null) record.x = field.description
    const aliases = view.aliases
    if (aliases.length > 0) record.a = aliases
    const alternates = view.tags
    if (alternates.length > 0) record.g = alternates

    if (codes !== null) record.c = codes.codes.length
    if (entries.length > 0) {
      record.s = entries[0].since ?? ''
      const closed = entries[entries.length - 1]
      if (closed.until !== undefined) record.e = closed.until
    }
    records.push(record)

  }
  records.sort((left, right) => left.t - right.t || (left.b ?? '').localeCompare(right.b ?? ''))
  return records
}

/** What the dictionary is, counted once so the page states no arithmetic. */
function counts(records, catalog, row) {
  const versions = new Set()
  const branches = new Map()
  const dtypes = new Map()
  let enumFields = 0
  let codes = 0
  let lineage = 0
  let aliases = 0
  let alternates = 0
  for (const record of records) {
    const branch = record.b ?? ''
    branches.set(branch, (branches.get(branch) ?? 0) + 1)
    dtypes.set(record.y, (dtypes.get(record.y) ?? 0) + 1)
    if (record.c) {
      enumFields += 1
      codes += record.c
    }
    if (record.s !== undefined) {
      lineage += 1
      versions.add(record.s)
    }
    if (record.e !== undefined) versions.add(record.e)
    if (record.a) aliases += record.a.length
    if (record.g) alternates += record.g.length
  }
  return {
    fields: records.length,
    groups: catalog.groups.length,
    enumFields,
    codes,
    lineage,
    aliases,
    alternates,
    messages: catalog.messages.length,
    components: catalog.components.length,
    columns: row.columns.length,
    branches: branches.size,
    datatypes: dtypes.size,
    versions: versions.size,
    dtypes: [...dtypes.entries()].sort((left, right) => right[1] - left[1] || (left[0] < right[0] ? -1 : 1)),
    branchSizes: [...branches.entries()].sort((left, right) => right[1] - left[1]),
    versionList: [...versions]
      .filter((held) => held !== '')
      .map((held) => Version.fromStr(held))
      .sort((left, right) => left.compare(right))
      .map(String),
  }
}

/** The fixed row a capture lands in, column by column. */
function fixedRow(registry) {
  const schema = fix.schema(registry, 'FixMessage')
  const columns = []
  for (let at = 0; at < schema.fieldLen; at += 1) {
    const field = schema.fieldAt(at)
    columns.push({
      c: field.name,
      t: field.fix.tag ?? 0,
      n: field.display ?? field.name,
      y: field.dtype.toString(),
      x: field.description ?? '',
    })
  }
  return {
    columns,
    call:
      "const registry = fix.FixRegistry.fromHandle('config/fix')\n" +
      "fix.schema(registry, 'FixMessage')",
  }
}

/** One captured line, and everything the package answered about it. */
function frameCase(registry, reader, schema, key, label, line) {
  const bytes = Buffer.from(line, 'binary')
  const messages = reader.parseLine(bytes)
  const first = messages.next()
  if (first.done || !messages.next().done) throw new Error(`corpus ${key} must yield one message`)
  const held = first.value
  const row = held.intoRow(schema).toJSON()

  const columns = SHOWN.map((tag) => {
    const at = schema.indexOf(String(tag))
    const value = at === null ? null : row[at]
    if (value === null || value === undefined) return null
    const field = schema.fieldAt(at)
    return {
      t: tag,
      n: field.display ?? field.name,
      y: field.dtype.toString(),
      v: String(value),
    }
  }).filter((column) => column !== null)

  const ticker = held.symbolTicker()
  const clock = held.marketTimestamp()
  const partition = held.unixPartition(3600)
  const text = escapedText([...bytes])
  return {
    key,
    label,
    // Escaped, because two of the bytes in a bridge frame are controls and a
    // JSON string carrying them raw is a string a reader cannot see.
    line: text,
    root: held.field.name,
    field: held.field.toJSON(),
    value: held.value.toJSON(),
    mime: String(MimeType.inferBytes(bytes)),
    msgtype: msgtypeOf(bytes),
    direction: MimeType.inferBytesDirection(bytes),
    branch: held.branch,
    size: held.size,
    columns,
    arrivals: held.arrivals().map(([tag, key_, value]) => [String(tag), key_, value]),
    unmapped: held
      .arrivals()
      .filter(([tag]) => registry.getFieldByTag(tag) === null)
      .map(([, key_]) => key_),
    lift: held.lift().map(([facet, value]) => [facet, String(value.toJSON()), held.liftSource(facet)]),
    anomalies: held.anomalies(),
    digest: Buffer.from(held.digest()).toString('hex'),
    ticker: ticker === null ? null : String(ticker.toJSON()),
    clock: clock === null ? null : String(clock.toJSON()),
    partition: partition === null ? null : String(partition.toJSON()),
    // What the package re-emits from the entries, which is the encoder's
    // proof: a composed frame that does not match this is a composed frame
    // that is wrong.
    emitted: held.size === 0 ? '' : escapedText([...Buffer.from(held.intoBytes(0x7c))]),
    // The raw line rather than its escape, and the encoding `frameCase` itself
    // read it under: a snippet that does not reproduce the answer beside it is
    // not the call that answered.
    call: `[...new fix.FixCodec(registry).parseLine(Buffer.from(${JSON.stringify(line)}, 'binary'))]`,
  }
}

/** Build the native catalog and recorded result manifest. */
function manifest() {
  const registry = dictionary()
  const reader = new fix.FixCodec(registry)
  const schema = fix.schema(registry, 'FixMessage')
  const catalog = registry.toJSON()
  const provenance = JSON.parse(fs.readFileSync(path.join(CONFIG, 'provenance.json'), 'utf8'))
  const records = fieldRecords(registry)
  const row = fixedRow(registry)
  const kpi = counts(records, catalog, row)


  const index = {
    version: VERSION,
    spec: {
      version: provenance.version,
      ep: provenance.ep,
      shards: Object.keys(provenance.definitions ?? {}).length,
      sources: provenance.sources.map((source) => ({
        id: source.source_id,
        format: source.format,
        version: source.version,
        url: source.url,
        sha256: source.sha256,
        license: source.license_url,
      })),
    },
    kpi,
    // Native compact Field documents preserve category and contextual references.
    // The browser displays this graph; it does not resolve or union schemas.
    catalog,
    row,
    frames: FRAMES.map(([key, label, line]) =>
      frameCase(registry, reader, schema, key, label, key === UNSEALED ? line : sealed(line)),
    ),
    calls: {
      registry: "const registry = fix.FixRegistry.fromHandle('config/fix')",
      reader: 'const reader = new fix.FixCodec(registry)',
    },
  }
  return index
}

/** Report the first line two renderings differ on, so drift names itself. */
function difference(target, current, wanted) {
  if (current === null) return `${path.relative(ROOT, target)} is missing`
  const was = current.split('\n')
  const now = wanted.split('\n')
  for (let line = 0; line < Math.max(was.length, now.length); line += 1) {
    if (was[line] === now[line]) continue
    return (
      `${path.relative(ROOT, target)} is out of date at line ${line + 1}:\n` +
      `  committed: ${(was[line] ?? '<end of file>').slice(0, 160)}\n` +
      `  generated: ${(now[line] ?? '<end of file>').slice(0, 160)}`
    )
  }
  return `${path.relative(ROOT, target)} is out of date`
}

/** Write one manifest, or report its drift; answers whether it was current. */
function settle(target, wanted, check) {
  // A checkout under `core.autocrlf` materialises the committed LF manifest as
  // CRLF, which is not drift, so compare the text and keep the file's endings.
  const raw = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : null
  const current = raw === null ? null : raw.replace(/\r\n/g, '\n')
  if (current === wanted) return true
  if (check) {
    console.log(`  ${difference(target, current, wanted)}`)
    return false
  }
  fs.mkdirSync(path.dirname(target), { recursive: true })
  const eol = raw !== null && raw.includes('\r\n') ? '\r\n' : '\n'
  fs.writeFileSync(target, wanted.replace(/\n/g, eol))
  return true
}

function main(argv) {
  const check = argv.includes('--check')
  const index = manifest()
  const kpi = index.kpi

  console.log(
    `fix: ${kpi.fields} fields, ${kpi.codes} inline codes across ${kpi.enumFields} fields, ` +
      `${kpi.messages} messages, ${kpi.columns} columns, ${index.frames.length} frames` +
      `${check ? ' checked' : ' generated'}`,
  )

  return settle(INDEX, rendered(index), check) ? 0 : 1
}

process.exitCode = main(process.argv.slice(2))
