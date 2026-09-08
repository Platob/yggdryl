'use strict'

/**
 * Generate the FIX documentation's manifests from the real package.
 *
 * The JavaScript extension is a native Node addon, so a browser cannot load
 * it. Everything the explorer states about the dictionary - every tag, name,
 * datatype, code set, lineage entry, projected column - therefore comes from
 * here: this runs the published surface over the committed dictionary and
 * writes what it answered. The page reads FIX text against those answers and
 * never restates one of its own.
 *
 * Two manifests, because the explorer needs them at two different moments:
 *
 *   docs/assets/fix.json         the index every page opens with - the counts,
 *                                the fields, the layouts, the fixed row, and
 *                                one decoded frame per shape a capture holds
 *   docs/assets/fix-codes.json   the code sets and the lineages, fetched
 *                                behind the first paint because they are two
 *                                thirds of the bytes and nothing renders
 *                                until a reader asks about one field
 *
 * Both are committed, so the same build runs on any machine: fixed corpus,
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

const { MimeType, fix } = require('../node/binding.js')

const ROOT = path.join(__dirname, '..')
const CONFIG = path.join(ROOT, 'config', 'fix')
const ASSETS = path.join(ROOT, 'docs', 'assets')
const INDEX = path.join(ASSETS, 'fix.json')
const CODES = path.join(ASSETS, 'fix-codes.json')
const VERSION = require('../node/package.json').version

/** The frame whose two self-describing tags are deliberately wrong. */
const UNSEALED = 'unsealed'

const SOH = '\u0001'

// The corpus: one line per shape a session log holds, so the decoder shows
// what the package answers rather than a curated summary. Each is read by the
// real reader below and the answer is what the page renders beside the
// reader's own live reading of the same line.
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
  const held = MimeType.inferBytesMsgtype(bytes)
  return held === null ? null : Buffer.from(held).toString('binary')
}

/** The dictionary this crate tracks, with its own fields registered. */
function dictionary() {
  const registry = fix.FixRegistry.fromHandle(CONFIG)
  registry.withCrateFields()
  return registry
}

/**
 * A repeating group is a List whose occurrence is a Struct with members.
 *
 * The occurrence carries the component's own name - `NoPartyIDs` heads a
 * `PartyID` - so the test is the shape rather than the name: the name is
 * descriptive, and nothing that decides anything may read it.
 */
function isGroup(field) {
  const occurrence = field.getFieldAt(0)
  return occurrence !== null && occurrence.fieldLen > 0
}

/** The tags one group's occurrence Struct declares, in wire order. */
function memberTags(field) {
  const item = field.fieldAt(0)
  const tags = []
  for (let at = 0; at < item.fieldLen; at += 1) {
    const tag = item.fieldAt(at).fix.tag
    if (tag !== null) tags.push(tag)
  }
  return tags
}

/**
 * One lineage entry's datatype, as the reader of this manifest shows it.
 *
 * The document stores the crate's serialized datatype, exactly as a field's
 * own `dtype` is stored, so this is a *reading* of the package's answer and
 * not a second vocabulary: the tag is the datatype's name and the remaining
 * keys are its parameters, in the order the document holds them.
 */
function lineageType(held) {
  if (held === null || held === undefined) return ''
  const { type, ...rest } = held
  const parameters = Object.values(rest)
  return parameters.length === 0 ? type : `${type}(${parameters.join(', ')})`
}

/**
 * What every counter tag introduces, read from the layouts.
 *
 * The registry holds a group as the flat members it can type, so a group
 * inside a group is not among them and a group of nothing but groups is not
 * nested there at all. A frame carries every one of them, so this is the list
 * a reader of wire text needs: a component flattened into the fields it
 * contributes, and a nested group standing as its own counter. One counter
 * heads several groups - Orchestra declares `NoRelatedSym` eleven times, once
 * per context - so the contexts union in wire order, first occurrence winning,
 * which is the rule the dictionary itself is built with.
 */
function wireGroups(layouts) {
  const components = new Map(layouts.components.map((held) => [held.id, held]))
  const groups = new Map(layouts.groups.map((held) => [held.id, held]))
  const flatten = (counter, members, into, seen) => {
    for (const member of members) {
      if (member.kind === 'field') {
        if (member.id !== counter && !into.includes(member.id)) into.push(member.id)
      } else if (member.kind === 'component') {
        const component = components.get(member.id)
        if (component !== undefined && !seen.has(member.id)) {
          seen.add(member.id)
          flatten(counter, component.members, into, seen)
        }
      } else {
        const nested = groups.get(member.id)
        if (nested !== undefined && nested.tag !== counter && !into.includes(nested.tag)) {
          into.push(nested.tag)
        }
      }
    }
  }
  const wire = {}
  for (const group of layouts.groups) {
    const held = wire[group.tag] ?? { n: group.name, m: [] }
    flatten(group.tag, group.members, held.m, new Set())
    wire[group.tag] = held
  }
  return wire
}

/** The field tags one header or trailer component declares, groups included. */
function envelope(layouts, name) {
  const wire = { n: name, m: [] }
  const components = new Map(layouts.components.map((held) => [held.id, held]))
  const groups = new Map(layouts.groups.map((held) => [held.id, held]))
  const opening = layouts.components.find((held) => held.name === name)
  // By name, because a component identifier is the specification's and moves
  // with it; and loudly, because an empty header would silently reorder every
  // frame the composer writes.
  if (opening === undefined) throw new Error(`config/fix/layouts.json has no ${name} component`)
  const walk = (members) => {
    for (const member of members) {
      if (member.kind === 'field') {
        if (!wire.m.includes(member.id)) wire.m.push(member.id)
      } else if (member.kind === 'component') {
        const component = components.get(member.id)
        if (component !== undefined) walk(component.members)
      } else {
        const group = groups.get(member.id)
        if (group === undefined) continue
        // A counter is a header tag, and so is every member it introduces:
        // `HopCompID` belongs in the header wherever `HopGrp` puts it.
        if (!wire.m.includes(group.tag)) wire.m.push(group.tag)
        walk(group.members)
      }
    }
  }
  walk(opening.members)
  return wire.m
}

/** One stored JSON document, or null where the field carries none. */
function document(field, key) {
  const held = field.get(key)
  return held === null ? null : JSON.parse(held)
}

/**
 * Every registered field, as the index record the explorer searches.
 *
 * Short keys, because there are six thousand of them and the manifest is
 * fetched by a browser: `t` tag, `n` name, `d` display, `y` datatype, `b`
 * branch, `x` description, `a` aliases, `g` alternate tags, `k` kind, `m`
 * group member tags, `c` how many codes, `s`/`e` the versions the lineage
 * dates the field between.
 */
function fieldRecords(registry) {
  const records = []
  const details = {}
  for (const field of registry) {
    const view = field.fix
    const tag = view.tag
    if (tag === null) continue
    const group = isGroup(field)
    const codes = document(field, 'fix:codes')
    const lineage = document(field, 'fix:lineage')
    const entries = lineage === null ? [] : lineage.entries
    // A group's own datatype spells its whole occurrence Struct - two
    // kilobytes for a large one - and the explorer shows the members from `m`
    // instead, so the record carries the shape and not the transcription.
    const record = { t: tag, n: field.name, y: group ? 'list' : field.dtype.toString() }
    if (field.display !== null && field.display !== field.name) record.d = field.display
    if (view.branch !== '') record.b = view.branch
    if (field.description !== null) record.x = field.description
    const aliases = view.aliases
    if (aliases.length > 0) record.a = aliases
    const alternates = view.tags
    if (alternates.length > 0) record.g = alternates
    if (group) {
      record.k = 'group'
      record.m = memberTags(field)
    }

    if (codes !== null) record.c = codes.codes.length
    if (entries.length > 0) {
      record.s = entries[0].since ?? ''
      const closed = entries[entries.length - 1]
      if (closed.until !== undefined) record.e = closed.until
    }
    records.push(record)

    if (codes !== null || entries.length > 0) {
      const detail = {}
      if (codes !== null) {
        // Value, name, wording, the version and pack that added it, and the
        // version that deprecated it: what an explanation of a wire value
        // needs, and nothing the page never shows.
        detail.c = codes.codes.map((code) => [
          code.value,
          code.name,
          code.doc ?? '',
          code.since ?? '',
          code.ep ?? 0,
          code.deprecated ?? '',
        ])
      }
      if (entries.length > 0) {
        detail.l = entries.map((entry) => [
          entry.since ?? '',
          entry.name ?? '',
          lineageType(entry.type),
          entry.until ?? '',
        ])
      }
      details[view.id] = detail
    }
  }
  records.sort((left, right) => left.t - right.t || (left.b ?? '').localeCompare(right.b ?? ''))
  return { records, details }
}

/** What the dictionary is, counted once so the page states no arithmetic. */
function counts(records, layouts, projection) {
  const versions = new Set()
  const branches = new Map()
  const dtypes = new Map()
  let codeSets = 0
  let codes = 0
  let lineage = 0
  let aliases = 0
  let alternates = 0
  let groups = 0
  for (const record of records) {
    const branch = record.b ?? ''
    branches.set(branch, (branches.get(branch) ?? 0) + 1)
    dtypes.set(record.y, (dtypes.get(record.y) ?? 0) + 1)
    if (record.c) {
      codeSets += 1
      codes += record.c
    }
    if (record.s !== undefined) {
      lineage += 1
      versions.add(record.s)
    }
    if (record.e !== undefined) versions.add(record.e)
    if (record.a) aliases += record.a.length
    if (record.g) alternates += record.g.length
    if (record.k === 'group') groups += 1
  }
  return {
    fields: records.length,
    groups,
    primitives: records.length - groups,
    codeSets,
    codes,
    lineage,
    aliases,
    alternates,
    messages: layouts.messages.length,
    components: layouts.components.length,
    layoutGroups: layouts.groups.length,
    columns: projection.columns.length,
    branches: branches.size,
    datatypes: dtypes.size,
    versions: versions.size,
    dtypes: [...dtypes.entries()].sort((left, right) => right[1] - left[1] || (left[0] < right[0] ? -1 : 1)),
    branchSizes: [...branches.entries()].sort((left, right) => right[1] - left[1]),
    versionList: [...versions].filter((held) => held !== '').sort(compareVersions),
  }
}

/** FIX versions order by their numbers, then by service pack. */
function compareVersions(left, right) {
  const parts = (text) => {
    const [head, pack] = text.split('SP')
    return [...head.split('.').map(Number), pack === undefined ? 0 : Number(pack)]
  }
  const one = parts(left)
  const two = parts(right)
  for (let at = 0; at < Math.max(one.length, two.length); at += 1) {
    const difference = (one[at] ?? 0) - (two[at] ?? 0)
    if (difference !== 0) return difference
  }
  return 0
}

/** The fixed row a capture lands in, column by column. */
function projected(registry) {
  const projection = new fix.FixProjection(registry, 'FixMessage')
  const schema = projection.field
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
    carried: projection.carried,
    call:
      "const registry = fix.FixRegistry.fromHandle('config/fix')\n" +
      'registry.withCrateFields()\n' +
      "new fix.FixProjection(registry, 'FixMessage')",
  }
}

/** One captured line, and everything the package answered about it. */
function frameCase(registry, reader, projection, key, label, line) {
  const bytes = Buffer.from(line, 'binary')
  const held = reader.transformLine(bytes)
  const row = held.toRow(projection).toJSON()

  const columns = SHOWN.map((tag) => {
    const at = projection.positionOf(tag)
    const value = at === null ? null : row[at]
    if (value === null || value === undefined) return null
    const field = projection.column(at)
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
    mime: String(MimeType.inferBytes(bytes)),
    msgtype: msgtypeOf(bytes),
    direction: MimeType.inferBytesDirection(bytes),
    branch: held.branch,
    size: held.size,
    columns,
    arrivals: held.arrivals().map(([tag, , key_, value]) => [String(tag), key_, value]),
    unmapped: held
      .arrivals()
      .filter(([tag]) => registry.getFieldByTag(tag) === null)
      .map(([, , key_]) => key_),
    lift: held.lift().map(([facet, value]) => [facet, String(value.toJSON()), held.liftSource(facet)]),
    anomalies: held.anomalies(),
    digest: Buffer.from(held.digest()).toString('hex'),
    ticker: ticker === null ? null : String(ticker.toJSON()),
    clock: clock === null ? null : String(clock.toJSON()),
    partition: partition === null ? null : String(partition.toJSON()),
    // What the package re-emits from the entries, which is the encoder's
    // proof: a composed frame that does not match this is a composed frame
    // that is wrong.
    emitted: held.size === 0 ? '' : escapedText([...Buffer.from(held.toBytes(0x7c))]),
    // The raw line rather than its escape, and the encoding `frameCase` itself
    // read it under: a snippet that does not reproduce the answer beside it is
    // not the call that answered.
    call: `new fix.FixCodec(registry).transformLine(Buffer.from(${JSON.stringify(line)}, 'binary'))`,
  }
}

/** Build the index manifest and the detail manifest, in the order written. */
function manifests() {
  const registry = dictionary()
  const reader = new fix.FixCodec(registry)
  const projection = new fix.FixProjection(registry, 'FixMessage')
  const layouts = JSON.parse(fs.readFileSync(path.join(CONFIG, 'layouts.json'), 'utf8'))
  const provenance = JSON.parse(fs.readFileSync(path.join(CONFIG, 'provenance.json'), 'utf8'))
  const { records, details } = fieldRecords(registry)
  const row = projected(registry)
  const kpi = counts(records, layouts, row)


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
    header: envelope(layouts, 'StandardHeader'),
    trailer: envelope(layouts, 'StandardTrailer'),
    wire: wireGroups(layouts),
    fields: records,
    messages: layouts.messages.map((message) => ({
      y: message.msgtype,
      n: message.name,
      i: message.id,
      m: message.members.map((held) => [held.kind[0], held.id, held.required ? 1 : 0]),
    })),
    components: layouts.components.map((component) => ({
      n: component.name,
      i: component.id,
      m: component.members.map((held) => [held.kind[0], held.id, held.required ? 1 : 0]),
    })),
    groups: layouts.groups.map((group) => ({
      n: group.name,
      i: group.id,
      t: group.tag,
      m: group.members.map((held) => [held.kind[0], held.id, held.required ? 1 : 0]),
    })),
    projection: row,
    frames: FRAMES.map(([key, label, line]) =>
      frameCase(registry, reader, projection, key, label, key === UNSEALED ? line : sealed(line)),
    ),
    calls: {
      registry:
        "const registry = fix.FixRegistry.fromHandle('config/fix')\nregistry.withCrateFields()",
      reader: 'const reader = new fix.FixCodec(registry)',
    },
  }
  return { index, details }
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
  const { index, details } = manifests()
  const kpi = index.kpi

  console.log(
    `fix: ${kpi.fields} fields, ${kpi.codes} codes in ${kpi.codeSets} sets, ` +
      `${kpi.messages} messages, ${kpi.columns} columns, ${index.frames.length} frames` +
      `${check ? ' checked' : ' generated'}`,
  )

  const written = [
    settle(INDEX, rendered(index), check),
    settle(CODES, rendered(details), check),
  ]
  return written.every(Boolean) ? 0 : 1
}

process.exitCode = main(process.argv.slice(2))
