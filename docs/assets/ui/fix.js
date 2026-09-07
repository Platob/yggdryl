import {
  button,
  callBlock,
  cardRow,
  choiceList,
  code,
  control,
  createSearchIndex,
  facts,
  grid,
  input,
  make,
  note,
  panel,
  pill,
  search,
  searchableList,
  select,
  textarea,
  wireBlock,
} from './index.js'
import { fieldDocument } from './yggdryl.js'

const SOH = '\x01'
let fixControlId = 0

const appendValue = (parent, value) => {
  if (value === null || value === undefined) return
  if (Array.isArray(value)) {
    for (const entry of value) appendValue(parent, entry)
  } else {
    parent.append(value)
  }
}

const utf8 = (text) => new TextEncoder().encode(text)

const showable = (text) =>
  [...String(text)]
    .map((letter) => {
      const value = letter.codePointAt(0)
      if (value === 1) return '␁'
      if (value < 0x20 && letter !== '\n' && letter !== '\t') {
        return `\\x${value.toString(16).padStart(2, '0')}`
      }
      if (value === 0x7f) return '\\x7f'
      return letter
    })
    .join('')

const unescaped = (text) =>
  String(text).replace(/\\\\|\\x([0-9a-f]{2})/gi, (_, hex) =>
    hex === undefined ? '\\' : String.fromCharCode(Number.parseInt(hex, 16)),
  )

const directionWords = [
  ['SENT', ['sending', 'sent', 'send', 'outbound', 'outgoing', 'out']],
  ['RECV', ['receiving', 'received', 'receive', 'recv', 'inbound', 'incoming', 'in']],
]

const splitDirection = (line) => {
  const held = /^([A-Za-z]+)\b[\s>:<-]*/.exec(line)
  if (held === null) return [null, line]
  const word = held[1].toLowerCase()
  for (const [direction, words] of directionWords) {
    if (words.includes(word)) return [direction, line.slice(held[0].length)]
  }
  return [null, line]
}

const frameText = (line) => {
  let at = line.indexOf('8=')
  while (at > 0 && /\d/.test(line[at - 1])) at = line.indexOf('8=', at + 1)
  return at > 0 ? line.slice(at) : line
}

const splitSegments = (body, separator) =>
  separator === ' ' ? body.split(/\s+(?=[#\w.[\]]+=)/) : body.split(separator)

const separatorOf = (body) => {
  if (body.includes(SOH)) return SOH
  if (body.includes('^A')) return '^A'
  let best = '|'
  let most = 0
  for (const candidate of ['|', '\n', ';', ' ']) {
    const pairs = splitSegments(body, candidate).filter((segment) =>
      /^\s*[#\w.[\]]+=/.test(segment),
    ).length
    if (pairs > most) {
      most = pairs
      best = candidate
    }
  }
  return best
}

const checksum = (frame, stop) => {
  let total = 0
  for (let at = 0; at <= stop; at += 1) total += frame[at]
  return String(total % 256).padStart(3, '0')
}

const checksFor = (entries) => {
  const held = []
  const stated = (tag) => entries.find((entry) => entry.tag === tag)?.value ?? null
  const chunks = []
  let cursor = 0
  let start = -1
  let end = -1
  for (const entry of entries) {
    if (entry.tag === 10 && end === -1) end = cursor
    const chunk = utf8(`${entry.key}=${entry.value}${SOH}`)
    chunks.push(chunk)
    cursor += chunk.length
    if (entry.tag === 9 && start === -1) start = cursor
  }
  const frame = new Uint8Array(cursor)
  let offset = 0
  for (const chunk of chunks) {
    frame.set(chunk, offset)
    offset += chunk.length
  }
  const length = stated(9)
  if (length !== null && start !== -1 && end >= start) {
    const computed = String(end - start)
    held.push({
      name: 'BodyLength (9)',
      stated: length,
      computed,
      ok: Number(length) === Number(computed),
    })
  }
  const sum = stated(10)
  if (sum !== null && end !== -1) {
    const computed = checksum(frame, end - 1)
    held.push({ name: 'CheckSum (10)', stated: sum, computed, ok: sum === computed })
  }
  return held
}

const expectations = (model, messageType) => {
  const message = messageType === null ? undefined : model.messages.get(messageType)
  if (message === undefined) return { message: null, required: new Set(), known: new Set() }
  const tags = model.layoutTags(message)
  return {
    message,
    required: new Set(tags.filter((entry) => entry.required).map((entry) => entry.tag)),
    known: new Set(tags.map((entry) => entry.tag)),
  }
}

/**
 * Read one textual FIX frame against a generated manifest. Boolean and date
 * spellings get a display-only gloss; no native validation or canonicalization runs.
 */
export function inspectFixText({ model, text }) {
  const [direction, verbless] = splitDirection(String(text).trimStart())
  const body = frameText(verbless)
  const separator = separatorOf(body)
  const pairs = []
  let unknown = 0

  for (const segment of splitSegments(body, separator)) {
    const piece = segment.replace(/\r$/, '')
    const at = piece.indexOf('=')
    if (at <= 0) continue
    const key = piece.slice(0, at).trim()
    if (key === '') continue
    const record = model.field(key)
    if (record === null) unknown += 1
    pairs.push({
      key,
      value: piece.slice(at + 1),
      record,
      tag: record === null ? null : record.t,
    })
  }

  const messageType = pairs.find((entry) => entry.tag === 35)?.value ?? null
  const scoped = model.groupsOf(messageType)
  const groupFor = (tag) => scoped.get(tag) ?? model.groupsByTag.get(tag)
  const tree = []
  let at = 0

  const gather = (group, into) => {
    const members = new Set(group.m)
    const delimiter = group.m[0]
    let seen = new Set()
    let occurrence = null
    while (at < pairs.length) {
      const entry = pairs[at]
      if (entry.tag === null || !members.has(entry.tag)) break
      if (occurrence === null || entry.tag === delimiter || seen.has(entry.tag)) {
        occurrence = { members: [] }
        into.push(occurrence)
        seen = new Set()
      }
      seen.add(entry.tag)
      at += 1
      const nested = groupFor(entry.tag)
      if (nested !== undefined) {
        const occurrences = []
        gather(nested, occurrences)
        occurrence.members.push({ ...entry, occurrences })
      } else {
        occurrence.members.push(entry)
      }
    }
  }

  while (at < pairs.length) {
    const entry = pairs[at]
    at += 1
    const group = entry.tag === null ? undefined : groupFor(entry.tag)
    if (group !== undefined) {
      const occurrences = []
      gather(group, occurrences)
      tree.push({ ...entry, occurrences })
    } else {
      tree.push(entry)
    }
  }

  return {
    direction,
    separator,
    body,
    pairs,
    tree,
    unknown,
    messageType,
    version: pairs.find((entry) => entry.tag === 8)?.value ?? null,
    checks: checksFor(pairs),
  }
}

const normalizedPair = (model, entry) => {
  const pair = Array.isArray(entry) ? { key: entry[0], value: entry[1] } : entry
  if (pair === null || typeof pair !== 'object') throw new TypeError('a FIX entry must be a pair')
  const key = String(pair.key).trim()
  if (key === '') throw new TypeError('a FIX entry key must not be empty')
  const record = model.field(pair.key)
  return {
    key: record === null ? key : String(record.t),
    value: String(pair.value),
    tag: record?.t ?? (/^\d+$/.test(key) ? Number(key) : null),
  }
}

const displaySeparator = (separator) => {
  if (separator === 'soh' || separator === SOH) return { raw: SOH, shown: '␁' }
  return { raw: SOH, shown: String(separator) }
}

/** Compose a browser draft. The returned `wire` always contains real SOH bytes. */
export function composeFixText({
  model,
  messageType,
  beginString = 'FIX.4.4',
  entries,
  separator = '|',
}) {
  const type = String(messageType)
  const begin = String(beginString)
  const prepared = []
  for (const input of entries) {
    const entry = normalizedPair(model, input)
    if ([8, 9, 10, 35].includes(entry.tag)) continue
    prepared.push(entry)
  }
  const bodyPairs = [{ key: '35', value: type, tag: 35 }, ...prepared]
  const body = bodyPairs.map((entry) => `${entry.key}=${entry.value}${SOH}`).join('')
  const bodyLength = utf8(body).length
  const prefix = `8=${begin}${SOH}9=${bodyLength}${SOH}${body}`
  const sum = checksum(utf8(prefix), utf8(prefix).length - 1)
  const wire = `${prefix}10=${sum}${SOH}`
  const display = displaySeparator(separator)
  const shown = display.shown === '␁' ? showable(wire) : wire.replaceAll(SOH, display.shown)
  const conflicts =
    display.shown === '␁'
      ? []
      : prepared.filter((entry) => entry.value.includes(display.shown)).map((entry) => entry.key)
  return {
    wire,
    display: shown,
    separator: display.shown,
    bodyLength,
    checksum: sum,
    byteLength: utf8(wire).length,
    pairs: [
      { key: '8', value: begin, tag: 8 },
      { key: '9', value: String(bodyLength), tag: 9 },
      ...bodyPairs,
      { key: '10', value: sum, tag: 10 },
    ],
    conflicts,
    message: model.messages.get(type) ?? null,
  }
}

const detailLoader = (model, loadDetails) => {
  let pending
  return () => {
    if (pending !== undefined) return pending
    pending = Promise.resolve().then(() => {
      const source =
        loadDetails === undefined
          ? model.details
          : typeof loadDetails === 'function'
            ? loadDetails()
            : loadDetails
      return source ?? null
    })
    return pending
  }
}

const hasDetailSource = (model, loadDetails) =>
  loadDetails !== undefined || (model.details !== null && model.details !== undefined)

const renderDetailFailure = (error, renderDetailsError) => {
  const root = make('div')
  root.setAttribute('role', 'alert')
  let content = null
  if (renderDetailsError !== undefined) {
    try {
      content = renderDetailsError(error)
    } catch (renderError) {
      error = renderError
    }
  }
  appendValue(
    root,
    content ??
      note({
        content:
          `The FIX detail asset could not be loaded (${error instanceof Error ? error.message : String(error)}). ` +
          'Regenerate or restore that asset, then reload this page.',
        kind: 'warning',
      }),
  )
  return root
}

/** The generated registry counts and distribution. */
export function fixRegistrySummary({ model }) {
  const root = make('div', 'ygg-ui__fix-summary')
  const kpi = model.data.kpi
  const spec = model.data.spec
  root.append(
    cardRow({
      cards: [
        { value: kpi.fields.toLocaleString(), label: 'fields', note: `${kpi.primitives.toLocaleString()} flat` },
        { value: kpi.groups.toLocaleString(), label: 'repeating groups', note: 'nested tree' },
        { value: kpi.codes.toLocaleString(), label: 'codes', note: `in ${kpi.codeSets.toLocaleString()} sets` },
        { value: kpi.messages.toLocaleString(), label: 'message types', note: `${kpi.components} components` },
        { value: kpi.lineage.toLocaleString(), label: 'dated fields', note: `${kpi.versions} versions` },
        { value: kpi.columns.toLocaleString(), label: 'fixed columns', note: 'one capture row' },
        { value: String(spec.version), label: 'specification', note: `EP${spec.ep}` },
        { value: String(spec.shards), label: 'shards', note: `${spec.sources.length} sources` },
      ],
    }),
    panel({
      title: 'Datatypes the dictionary uses',
      aside: `${kpi.datatypes} distinct`,
      render: () =>
        grid({
          columns: ['datatype', 'fields', 'share'],
          rows: kpi.dtypes.map(([name, count]) => [
            name,
            count,
            `${((count / kpi.fields) * 100).toFixed(1)}%`,
          ]),
        }),
    }).element,
    panel({
      title: 'Branches',
      aside: `${kpi.branches}`,
      render: () =>
        grid({
          columns: ['branch', 'fields'],
          rows: kpi.branchSizes.map(([name, count]) => [name === '' ? 'standard' : name, count]),
        }),
    }).element,
    panel({
      title: 'Versions a lineage names',
      aside: `${kpi.versions}`,
      render: () => {
        const choices = make('div', 'ygg-ui__choices')
        for (const version of kpi.versionList) choices.append(pill({ text: version }))
        return choices
      },
    }).element,
  )
  return root
}

/** The pinned generated inputs behind a FIX manifest. */
export function fixSourceTable({ model }) {
  const root = make('div', 'ygg-ui__fix-source')
  const spec = model.data.spec
  root.append(
    grid({
      columns: ['source', 'format', 'version', 'sha256', 'licence'],
      rows: spec.sources.map((source) => {
        const link = make('a', null, source.id)
        link.href = source.url
        link.rel = 'noreferrer'
        const licence = make('a', null, 'licence')
        licence.href = source.license
        licence.rel = 'noreferrer'
        return [link, source.format, source.version, source.sha256.slice(0, 16), licence]
      }),
    }),
    note({
      content: `Every source is pinned to a commit and checked by its digest, so FIX ${spec.version} EP${spec.ep} regenerates byte for byte.`,
    }),
  )
  return root
}

const codeTable = (entries) => {
  const index = createSearchIndex({
    items: entries,
    haystack: ([value, name, documentation]) => `${value} ${name} ${documentation}`,
  })
  const listing = searchableList({
    index,
    pageSize: 60,
    noun: 'codes',
    label: 'Filter codes',
    placeholder: 'Filter the codes…',
    renderPage: (kept) =>
      grid({
        columns: ['value', 'name', 'since', 'deprecated', 'meaning'],
        rows: kept.map(([value, name, documentation, since, ep, deprecated]) => [
          value,
          name,
          since === '' ? null : ep ? `${since} EP${ep}` : since,
          deprecated === '' ? null : deprecated,
          documentation === '' ? null : make('span', 'ygg-ui__prose', documentation),
        ]),
      }),
  })
  listing.input.parentElement && (listing.input.parentElement.hidden = entries.length <= 12)
  return listing.element
}

const lineageTable = (entries) =>
  grid({
    columns: ['since', 'named', 'typed', 'until'],
    rows: entries.map(([since, name, type, until]) => [
      since,
      name,
      type,
      until === '' ? null : until,
    ]),
  })

const fieldDetail = (model, record, loadDetails, renderDetailsError) => {
  const body = make('div')
  const rows = [
    { label: 'identifier', value: `${record.t}:${record.b ?? ''}` },
    { label: 'tag', value: String(record.t) },
    { label: 'name', value: record.n },
    { label: 'display', value: record.d },
    {
      label: 'datatype',
      value: record.k === 'group' ? `list of ${record.m.length} members` : record.y,
    },
    { label: 'introduces', value: model.groupsByTag.get(record.t)?.n },
    { label: 'branch', value: (record.b ?? '') === '' ? 'standard' : record.b },
    { label: 'aliases', value: (record.a ?? []).join(', ') },
    { label: 'alternate tags', value: (record.g ?? []).join(', ') },
    { label: 'since', value: record.s },
    { label: 'until', value: record.e },
  ]
  if (record.x) rows.push({ label: 'wording', value: make('span', 'ygg-ui__prose', record.x) })
  body.append(facts({ rows }))

  const wire = model.groupsByTag.get(record.t)
  if (wire !== undefined) {
    const members = wire.m.map((tag) => model.byTag.get(tag)).filter(Boolean)
    body.append(
      panel({
        title: 'Group members',
        aside: `${wire.n} · ${wire.m.length}`,
        render: () =>
          grid({
            columns: ['tag', 'field', 'type', 'meaning'],
            rows: members.map((member) => [
              member.t,
              model.titleOf(member),
              model.groupsByTag.has(member.t) ? 'group' : member.y,
              member.x ? make('span', 'ygg-ui__prose', member.x) : null,
            ]),
          }),
      }).element,
    )
  }

  const carriers = model.carriers(record.t)
  if (carriers.length > 0) {
    body.append(
      panel({
        title: 'Carried by',
        aside: `${carriers.length} message types`,
        render: () => {
          const list = make('div', 'ygg-ui__choices')
          for (const message of carriers.slice(0, 80)) {
            list.append(pill({ text: `${message.y} ${message.n}` }))
          }
          if (carriers.length > 80) {
            list.append(pill({ text: `+${carriers.length - 80} more`, kind: 'quiet' }))
          }
          return list
        },
      }).element,
    )
  }

  if (record.c || record.s !== undefined) {
    const extra = make('div')
    const loading = note({ content: 'Loading the code set…', kind: 'info' })
    loading.setAttribute('role', 'status')
    extra.append(loading)
    body.append(extra)
    loadDetails().then(
      (all) => {
        extra.textContent = ''
        const found = all === null ? null : model.detail(record, all)
        if (found?.c) {
          extra.append(
            panel({
              title: 'Code set',
              aside: `${found.c.length} values`,
              open: found.c.length <= 12,
              render: () => codeTable(found.c),
            }).element,
          )
        }
        if (found?.l) {
          extra.append(
            panel({
              title: 'Lineage',
              aside: `${found.l.length} versions`,
              render: () => lineageTable(found.l),
            }).element,
          )
        }
      },
      (error) => {
        extra.textContent = ''
        extra.append(renderDetailFailure(error, renderDetailsError))
      },
    )
  }
  body.append(
    callBlock({
      code:
        `registry.fieldByTag(${record.t})            // JavaScript\n` +
        `registry.field_by_tag(${record.t})?         // Rust\n` +
        `registry.field_by_tag(${record.t})          # Python\n` +
        `cargo run -p yggdryl-cli -- --root config/fix show ${record.t}`,
    }),
  )
  return body
}

/** Search the compact generated fields without rebuilding their haystacks per keypress. */
export function fixFieldExplorer({
  model,
  loadDetails,
  renderDetailsError,
  pageSize = 60,
}) {
  const details = detailLoader(model, loadDetails)
  const declaredTypes = model.data.kpi?.dtypes
  const dtypeRows = Array.isArray(declaredTypes)
    ? declaredTypes
    : [...model.data.fields.reduce((counts, record) => {
        counts.set(record.y, (counts.get(record.y) ?? 0) + 1)
        return counts
      }, new Map())].sort((left, right) => right[1] - left[1])
  const kind = select({
    value: 'all',
    options: [
      { value: 'all', label: 'Every field' },
      { value: 'group', label: 'Repeating groups' },
      { value: 'coded', label: 'With a code set' },
      { value: 'dated', label: 'With a lineage' },
      { value: 'crate', label: 'The capture’s own' },
    ],
  })
  const dtype = select({
    value: 'any',
    options: [
      { value: 'any', label: 'Any datatype' },
      ...dtypeRows.map(([name, count]) => ({
        value: name,
        label: `${name} (${count})`,
      })),
    ],
  })
  const matches = (record) => {
    if (kind.value === 'group' && record.k !== 'group') return false
    if (kind.value === 'coded' && !record.c) return false
    if (kind.value === 'dated' && record.s === undefined) return false
    if (kind.value === 'crate' && (record.b ?? '') === '') return false
    return dtype.value === 'any' || record.y === dtype.value
  }
  const row = (record) => {
    const entry = panel({
      title: `${record.t}`,
      aside: model.titleOf(record),
      render: () => fieldDetail(model, record, details, renderDetailsError),
    })
    const marks = make('span', 'ygg-ui__choices')
    const isGroup = record.k === 'group' || model.groupsByTag.has(record.t)
    marks.append(
      pill({ text: isGroup ? 'group' : record.y, kind: isGroup ? 'group' : undefined }),
    )
    if (record.c) marks.append(pill({ text: `${record.c} codes`, kind: 'codes' }))
    if ((record.b ?? '') !== '') marks.append(pill({ text: record.b, kind: 'branch' }))
    if (record.s !== undefined) marks.append(pill({ text: `since ${record.s}`, kind: 'quiet' }))
    entry.summary.append(marks)
    return entry.element
  }
  const listing = searchableList({
    index: model.searchIndex,
    pageSize,
    noun: 'fields',
    label: 'Search',
    placeholder: 'symbol, 55, ClOrdID, "settlement date"…',
    controls: [
      control({ id: `ygg-ui-fix-kind-${++fixControlId}`, label: 'Kind', node: kind }),
      control({ id: `ygg-ui-fix-dtype-${++fixControlId}`, label: 'Datatype', node: dtype }),
    ],
    filter: matches,
    renderPage: (records) => records.map(row),
    className: 'ygg-ui__fix-fields',
  })
  kind.addEventListener('change', listing.reset)
  dtype.addEventListener('change', listing.reset)
  return { ...listing, kind, dtype }
}

/** Filter message types and inspect one lazy layout tree. */
export function fixMessageExplorer({ model }) {
  const element = make('div', 'ygg-ui__fix-messages')
  const controls = make('div', 'ygg-ui__controls')
  const inputNode = search({ placeholder: 'NewOrderSingle, D, execution…', autocomplete: 'off' })
  const chooser = select({ wide: true })
  controls.append(
    control({ id: `ygg-ui-fix-message-filter-${++fixControlId}`, label: 'Filter', node: inputNode }),
    control({ id: `ygg-ui-fix-message-${++fixControlId}`, label: 'Message', node: chooser }),
  )
  const summary = make('div')
  const view = make('div')
  const index = createSearchIndex({
    items: model.data.messages,
    haystack: (message) => `${message.y} ${message.n}`,
  })

  const show = () => {
    const message = model.messages.get(chooser.value)
    summary.textContent = ''
    view.textContent = ''
    if (message === undefined) {
      summary.append(note({ content: 'No message type matches that filter.', kind: 'info' }))
      return null
    }
    const tags = model.layoutTags(message)
    const groups = tags.filter((entry) => entry.group !== undefined)
    summary.append(
      cardRow({
        cards: [
          { value: message.y, label: 'message type', note: message.n },
          { value: String(tags.length), label: 'fields reached', note: 'through every block' },
          {
            value: String(tags.filter((entry) => entry.required).length),
            label: 'required',
            note: 'by the layout',
          },
          { value: String(groups.length), label: 'repeating groups', note: 'nested' },
          {
            value: String(message.m.filter(([kind]) => kind === 'c').length),
            label: 'components',
            note: 'at the top level',
          },
        ],
      }),
    )
    view.append(model.layoutTree(message.m))
    return message
  }

  const refresh = () => {
    const kept = index.search(inputNode.value)
    const current = chooser.value
    chooser.textContent = ''
    for (const message of kept) {
      const option = make('option', null, `${message.n} (${message.y})`)
      option.value = message.y
      chooser.append(option)
    }
    chooser.value = kept.some((message) => message.y === current) ? current : (kept[0]?.y ?? '')
    return show()
  }

  inputNode.addEventListener('input', refresh)
  chooser.addEventListener('change', show)
  element.append(controls, summary, view)
  refresh()
  return { element, input: inputNode, select: chooser, summary, view, refresh, show }
}

/** Search the package-produced fixed capture projection. */
export function fixProjectionExplorer({ model, pageSize = 60 }) {
  const projection = model.data.projection
  const listing = searchableList({
    index: createSearchIndex({
      items: projection.columns,
      haystack: (column) => `${column.c} ${column.n} ${column.x}`,
    }),
    pageSize,
    noun: 'columns',
    label: 'Filter',
    placeholder: 'symbol, 55, price…',
    renderPage: (kept) =>
      grid({
        columns: ['column', 'field', 'type', 'wording'],
        rows: kept.map((column) => [
          column.c,
          column.n,
          column.y,
          column.x === '' ? null : make('span', 'ygg-ui__prose', column.x),
        ]),
      }),
    className: 'ygg-ui__fix-projection',
  })
  const own = projection.columns.filter(
    (column) => (model.byTag.get(column.t)?.b ?? '') !== '',
  ).length
  listing.element.append(
    note({ content: `${own} columns are the capture's own.` }),
    callBlock({ code: projection.call }),
  )
  return listing
}

const valueReading = (record, text) => {
  if (record === null) return null
  if (record.y === 'boolean') {
    if (text === 'Y') return 'true'
    if (text === 'N') return 'false'
    return null
  }
  if (record.y.startsWith('datetime64')) {
    const held = /^(\d{4})(\d{2})(\d{2})-(\d{2}):(\d{2}):(\d{2})(\.\d+)?$/.exec(text)
    if (held === null) return null
    return `${held[1]}-${held[2]}-${held[3]}T${held[4]}:${held[5]}:${held[6]}${held[7] ?? ''}Z`
  }
  if (record.y === 'date32') {
    const held = /^(\d{4})(\d{2})(\d{2})$/.exec(text)
    return held === null ? null : `${held[1]}-${held[2]}-${held[3]}`
  }
  return null
}

const flattenDecodeEntries = (entries, depth = 0, flattened = []) => {
  for (const entry of entries) {
    flattened.push({ entry, depth })
    for (const occurrence of entry.occurrences ?? []) {
      flattenDecodeEntries(occurrence.members, depth + 1, flattened)
    }
  }
  return flattened
}

const rowHaystack = (row) =>
  row
    .map((value) =>
      value !== null && typeof value === 'object' && 'textContent' in value
        ? value.textContent
        : (value ?? ''),
    )
    .join(' ')

const decodeHaystack = (model, { entry }) => {
  const record = entry.record
  return [
    entry.key,
    entry.tag,
    entry.value,
    record === null ? 'unknown' : model.titleOf(record),
    record?.y,
    record?.x,
  ].join(' ')
}

const decodeRow = (model, { entry, depth }, details, expected) => {
  const record = entry.record
  const name = make('span', 'ygg-ui__tree-label')
  if (depth > 0) name.style.marginLeft = `${depth * 0.8}em`
  name.textContent = record === null ? entry.key : model.titleOf(record)
  const marks = make('span', 'ygg-ui__choices')
  if (record === null) {
    marks.append(pill({ text: 'unknown', kind: 'unknown' }))
  } else {
    if (model.header.has(record.t)) marks.append(pill({ text: 'header', kind: 'quiet' }))
    if (model.trailer.has(record.t)) marks.append(pill({ text: 'trailer', kind: 'quiet' }))
    if (expected.required.has(record.t)) {
      marks.append(pill({ text: 'required', kind: 'required' }))
    }
  }
  if (entry.occurrences !== undefined) {
    marks.append(pill({ text: `${entry.occurrences.length} occurrences`, kind: 'group' }))
  }
  const meaning = make('span')
  const found = record === null || details === null ? undefined : model.detail(record, details)
  const named = found?.c?.find(([value]) => value === entry.value)
  if (named !== undefined) {
    meaning.append(make('strong', null, named[1]))
    if (named[2]) meaning.append(` — ${named[2]}`)
  } else {
    const reading = valueReading(record, entry.value)
    if (reading !== null) meaning.append(code({ text: reading }))
    else if (record?.x) meaning.append(make('span', 'ygg-ui__prose', record.x))
  }
  return [
    entry.tag === null ? entry.key : String(entry.tag),
    name,
    showable(entry.value),
    record === null ? 'unknown' : record.k === 'group' ? 'group' : record.y,
    marks,
    meaning,
  ]
}

const boundedGrid = ({ columns, items, pageSize, noun, label, haystack, renderRow }) =>
  searchableList({
    index: createSearchIndex({ items, haystack: haystack ?? rowHaystack }),
    pageSize,
    noun,
    label,
    placeholder: `Filter ${noun}…`,
    renderPage: (kept) => grid({ columns, rows: renderRow ? kept.map(renderRow) : kept }),
  }).element

const projectionSize = (model) =>
  Array.isArray(model.data.projection?.columns) ? model.data.projection.columns.length : null

const packageAnswer = (frame, totalColumns = null) =>
  panel({
    title: 'What the package answered',
    aside: frame.label,
    open: true,
    render: () => {
      const content = [
        facts({
          rows: [
            { label: 'message', value: frame.root },
            { label: 'media type', value: frame.mime },
            { label: 'message type', value: frame.msgtype },
            { label: 'direction', value: frame.direction },
            {
              label: 'branch',
              value: frame.branch === undefined ? null : frame.branch === '' ? 'standard' : frame.branch,
            },
            { label: 'fields', value: frame.size === undefined ? null : String(frame.size) },
            { label: 'digest', value: frame.digest },
            { label: 'symbol ticker', value: frame.ticker },
            { label: 'market timestamp', value: frame.clock },
            { label: 'unix partition', value: frame.partition },
          ],
        }),
      ]
      if ((frame.columns ?? []).length > 0) {
        content.push(
          panel({
            title: 'The columns it filled',
            aside:
              totalColumns === null
                ? `${frame.columns.length}`
                : `${frame.columns.length} of ${totalColumns}`,
            render: () =>
              grid({
                columns: ['tag', 'column', 'type', 'value'],
                rows: frame.columns.map((column) => [column.t, column.n, column.y, column.v]),
              }),
          }).element,
        )
      }
      if ((frame.lift ?? []).length > 0) {
        content.push(
          panel({
            title: 'What it derived',
            aside: `${frame.lift.length} facets`,
            render: () =>
              grid({
                columns: ['facet', 'value', 'read from'],
                rows: frame.lift.map(([facet, value, source]) => [facet, value, source]),
              }),
          }).element,
        )
      }
      if ((frame.anomalies ?? []).length > 0) {
        content.push(
          panel({
            title: 'What does not add up',
            aside: `${frame.anomalies.length}`,
            render: () =>
              grid({
                columns: ['anomaly'],
                rows: frame.anomalies.map((text) => [make('span', 'ygg-ui__prose', text)]),
              }),
          }).element,
        )
      }
      if (frame.emitted) {
        const raw = unescaped(frame.emitted)
        content.push(
          panel({
            title: 'Re-emitted from its entries',
            aside: 'the wire record',
            render: () =>
              wireBlock({ display: raw.replaceAll(SOH, '|'), copy: raw }).element,
          }).element,
        )
      }
      if (frame.call) content.push(callBlock({ code: frame.call }))
      return content
    },
  }).element

/** A live manifest reading with an optional explicit host/package decode. */
export function fixDecodeWorkbench({
  model,
  loadDetails,
  renderDetailsError,
  frames: configuredFrames,
  value,
  decode,
  pageSize = 60,
}) {
  const frames = Array.isArray(configuredFrames)
    ? configuredFrames
    : Array.isArray(model.data.frames)
      ? model.data.frames
      : []
  const element = make('div', 'ygg-ui__fix-decode')
  element.append(
    note({
      content:
        'Manifest reading: this browser names fields, gathers groups, checks BodyLength and CheckSum, and adds display-only boolean/date glosses. It does not run native validation, canonicalization, derived facets, or the package codec.',
      kind: 'info',
    }),
  )
  const editor = make('div')
  const inputNode = textarea({
    rows: 4,
    spellcheck: false,
    value,
    placeholder: '8=FIX.4.4|35=D|55=AAPL|54=1|38=100|44=10.5|10=000|',
  })
  editor.append(
    control({
      id: `ygg-ui-fix-decode-${++fixControlId}`,
      label: 'A FIX frame — pipes, SOH, ^A or newlines',
      node: inputNode,
    }),
  )

  const presets = choiceList({
    items: frames,
    render: (frame) => frame.label,
    onSelect: (frame) => setValue(unescaped(frame.line)),
  })
  const summary = make('div')
  const view = make('div')
  const detailFailure = make('div')
  const host = make('div')
  host.setAttribute('aria-live', 'polite')
  const failure = make('p', 'ygg-ui__note ygg-ui__note--danger')
  failure.setAttribute('role', 'alert')
  failure.setAttribute('tabindex', '-1')
  failure.hidden = true
  const runHost = decode
    ? button({ label: 'Decode with the host', kind: 'primary' })
    : null
  if (runHost !== null) editor.append(runHost)

  let details = null
  let detailStarted = false
  let hostVersion = 0
  let inputRevision = 0
  let last = null
  const load = detailLoader(model, loadDetails)
  const corpus = new Map()
  for (const frame of frames) {
    const reading = inspectFixText({ model, text: unescaped(frame.line) })
    const key = `${reading.direction ?? ''}\x00${reading.body}`
    if (!corpus.has(key)) corpus.set(key, frame)
  }

  const startDetails = () => {
    if (detailStarted || !hasDetailSource(model, loadDetails)) return
    detailStarted = true
    load().then(
      (found) => {
        details = found
        inspect({ preserveHostFailure: true })
      },
      (error) => {
        detailFailure.textContent = ''
        detailFailure.append(renderDetailFailure(error, renderDetailsError))
      },
    )
  }

  const inspect = ({ preserveHostFailure = false } = {}) => {
    summary.textContent = ''
    view.textContent = ''
    if (!preserveHostFailure) failure.hidden = true
    const text = inputNode.value
    if (text.trim() === '') {
      last = null
      summary.append(note({ content: 'Paste a frame, or pick one of the captures above.', kind: 'info' }))
      return null
    }
    startDetails()
    const reading = inspectFixText({ model, text })
    last = reading
    const expected = expectations(model, reading.messageType)
    const missing = [...expected.required].filter(
      (tag) => !reading.pairs.some((entry) => entry.tag === tag),
    )
    const groups = reading.tree.filter((entry) => entry.occurrences !== undefined)
    summary.append(
      cardRow({
        cards: [
          {
            value: reading.messageType ?? '—',
            label: 'message type',
            note: expected.message ? expected.message.n : 'not in the dictionary',
          },
          {
            value: reading.version ?? '—',
            label: 'begin string',
            note: reading.direction ?? 'no direction stated',
          },
          {
            value: String(reading.pairs.length),
            label: 'pairs',
            note: `${reading.pairs.length - reading.unknown} named`,
          },
          {
            value: String(reading.unknown),
            label: 'unexplained',
            note: reading.unknown === 0 ? 'every tag resolved' : 'no dictionary names them',
          },
          {
            value: String(groups.length),
            label: 'repeating groups',
            note: `${groups.reduce((total, entry) => total + entry.occurrences.length, 0)} occurrences`,
          },
          {
            value: String(missing.length),
            label: 'required missing',
            note: expected.message ? 'against its layout' : 'no layout to check',
          },
        ],
      }),
    )
    if (reading.checks.length > 0) {
      summary.append(
        grid({
          columns: ['check', 'stated', 'computed', 'result'],
          rows: reading.checks.map((check) => [
            check.name,
            check.stated,
            check.computed,
            pill({ text: check.ok ? 'agrees' : 'differs', kind: check.ok ? 'success' : 'danger' }),
          ]),
        }),
      )
    }
    view.append(
      panel({
        title: 'Field by field',
        aside: `${reading.pairs.length} pairs`,
        open: true,
        render: () => {
          const items = flattenDecodeEntries(reading.tree)
          return boundedGrid({
            columns: ['tag', 'field', 'value', 'type', '', 'meaning'],
            items,
            pageSize,
            noun: 'pairs',
            label: 'Filter decoded pairs',
            haystack: (item) => decodeHaystack(model, item),
            renderRow: (item) => decodeRow(model, item, details, expected),
          })
        },
      }).element,
    )
    if (missing.length > 0) {
      view.append(
        panel({
          title: 'Required by the layout and not sent',
          aside: `${missing.length}`,
          render: () => {
            const rows = missing.map((tag) => {
              const record = model.byTag.get(tag)
              return [tag, record ? model.titleOf(record) : 'unknown', record?.y ?? null]
            })
            return boundedGrid({
              columns: ['tag', 'field', 'type'],
              items: rows,
              pageSize,
              noun: 'missing fields',
              label: 'Filter missing fields',
            })
          },
        }).element,
      )
    }
    if (reading.unknown > 0) {
      view.append(
        panel({
          title: 'What no dictionary explains',
          aside: `${reading.unknown}`,
          render: () => {
            const rows = reading.pairs
              .filter((entry) => entry.record === null)
              .map((entry) => [entry.key, showable(entry.value)])
            return boundedGrid({
              columns: ['key', 'value'],
              items: rows,
              pageSize,
              noun: 'unknown fields',
              label: 'Filter unknown fields',
            })
          },
        }).element,
      )
    }
    const generated = corpus.get(`${reading.direction ?? ''}\x00${reading.body}`)
    if (generated !== undefined) {
      view.append(packageAnswer(generated, projectionSize(model)))
    } else {
      view.append(
        note({
          content:
            'This frame is not in the generated corpus, so the typed row, digest, derived facets, and anomalies are unavailable. Those are package answers; read the frame with the package to produce them.',
          kind: 'info',
        }),
        callBlock({
          code: `new fix.FixReader(registry).text(${JSON.stringify(text)})`,
        }),
      )
    }
    return reading
  }

  const invalidateHost = () => {
    inputRevision += 1
    host.textContent = ''
    failure.hidden = true
    if (runHost?.disabled) {
      hostVersion += 1
      runHost.disabled = false
      element.setAttribute('aria-busy', 'false')
    }
  }

  const setValue = (text) => {
    invalidateHost()
    inputNode.value = String(text)
    return inspect()
  }

  const decodeWithHost = async () => {
    if (decode === undefined || last === null) return null
    const version = ++hostVersion
    const revision = inputRevision
    const text = inputNode.value
    runHost.disabled = true
    element.setAttribute('aria-busy', 'true')
    failure.hidden = true
    host.textContent = 'Decoding…'
    try {
      const result = await decode(text, last)
      if (version !== hostVersion || inputRevision !== revision || text !== inputNode.value) {
        host.textContent = ''
        return null
      }
      host.textContent = ''
      appendValue(host, result)
      return result
    } catch (error) {
      if (version !== hostVersion || inputRevision !== revision || text !== inputNode.value) {
        return null
      }
      host.textContent = ''
      failure.textContent = error instanceof Error ? error.message : String(error)
      failure.hidden = false
      failure.focus()
      return null
    } finally {
      if (version === hostVersion) {
        runHost.disabled = false
        element.setAttribute('aria-busy', 'false')
      }
    }
  }

  inputNode.addEventListener('input', () => {
    invalidateHost()
    inspect()
  })
  runHost?.addEventListener('click', () => void decodeWithHost())
  element.append(editor, presets.element, detailFailure, summary, view, host, failure)
  if (value === undefined && frames.length > 0) inputNode.value = unescaped(frames[0].line)
  inspect()
  return {
    element,
    input: inputNode,
    summary,
    view,
    host,
    error: failure,
    setValue,
    inspect,
    decode: decodeWithHost,
    get reading() {
      return last
    },
  }
}

/** Browse generated frames and the package answers embedded beside them. */
export function fixFrameGallery({ model, frames = model.data.frames ?? [] }) {
  frames = Array.isArray(frames) ? frames : []
  const element = make('div', 'ygg-ui__fix-frames')
  const view = make('div')
  if (frames.length === 0) {
    element.append(note({ content: 'No generated frames are available.', kind: 'info' }))
    return { element, choices: null, view, select: () => null }
  }
  const show = (index) => {
    const frame = frames[index]
    if (frame === undefined) throw new RangeError(`frame index ${index} is outside the gallery`)
    const raw = unescaped(frame.line)
    view.textContent = ''
    view.append(wireBlock({ display: showable(raw).replaceAll('␁', '|'), copy: raw }).element)
    view.append(packageAnswer(frame, projectionSize(model)))
    return frame
  }
  const choices = choiceList({
    items: frames,
    render: (frame) => frame.label,
    selected: 0,
    onSelect: (_, index) => show(index),
  })
  element.append(choices.element, view)
  show(0)
  return {
    element,
    choices,
    view,
    select(index) {
      choices.select(index)
      return frames[index]
    },
  }
}

const seedValues = new Map([
  [8, 'FIX.4.4'],
  [49, 'BUYSIDE'],
  [56, 'VENUE'],
  [34, '1'],
  [52, '20240201-12:34:56.000'],
  [11, 'ORDER-1'],
  [55, 'AAPL'],
  [54, '1'],
  [38, '100'],
  [44, '10.5'],
  [40, '2'],
  [60, '20240201-12:34:56.123'],
])

const initialValues = (model, values) => {
  const held = new Map()
  if (values === undefined || values === null) return held
  const entries = typeof values[Symbol.iterator] === 'function' ? values : Object.entries(values)
  for (const [key, value] of entries) {
    const record = model.field(key)
    if (record === null) throw new RangeError(`unknown FIX field ${key}`)
    if (record.t === 35) {
      throw new RangeError('FIX field 35 is selected through messageType')
    }
    if (record.t === 9 || model.trailer.has(record.t)) {
      throw new RangeError(`FIX field ${record.t} is computed by the draft`)
    }
    if (model.groupsByTag.has(record.t)) {
      throw new RangeError(`FIX field ${record.t} introduces a repeating group`)
    }
    held.set(record.t, String(value))
  }
  return held
}

/** Build a manifest-guided browser draft and optionally ask a host to validate it. */
export function fixEncodeWorkbench({
  model,
  loadDetails,
  renderDetailsError,
  messageType = 'D',
  beginString = 'FIX.4.4',
  values: configuredValues,
  onDecode,
  validate,
}) {
  const element = make('div', 'ygg-ui__fix-encode')
  const disclosure = note({
    content:
      'Browser draft: field names, layouts, and code choices come from the generated manifest; this browser computes BodyLength and CheckSum. It does not run the package encoder.',
    kind: 'info',
  })
  const controls = make('div', 'ygg-ui__controls')
  const messages = model.data.messages
  const wantedType = model.messages.has(String(messageType))
    ? String(messageType)
    : (messages[0]?.y ?? '')
  const messageSelect = select({
    wide: true,
    value: wantedType,
    options: messages.map((message) => ({
      value: message.y,
      label: `${message.n} (${message.y})`,
    })),
  })
  const scopeSelect = select({
    value: 'common',
    options: [
      { value: 'common', label: 'Required and common' },
      { value: 'required', label: 'Required only' },
      { value: 'declared', label: 'Declared by the message itself' },
    ],
  })
  const separatorSelect = select({
    value: '|',
    options: [
      { value: '|', label: 'Pipe (readable)' },
      { value: 'soh', label: 'SOH (the wire)' },
      { value: '^A', label: 'Caret-A (logs)' },
    ],
  })
  const extra = search({ placeholder: 'a tag or a name, then Enter', autocomplete: 'off' })
  controls.append(
    control({ id: `ygg-ui-fix-encode-message-${++fixControlId}`, label: 'Message', node: messageSelect }),
    control({ id: `ygg-ui-fix-encode-scope-${++fixControlId}`, label: 'Show', node: scopeSelect }),
    control({ id: `ygg-ui-fix-encode-extra-${++fixControlId}`, label: 'Add a field', node: extra }),
    control({ id: `ygg-ui-fix-encode-separator-${++fixControlId}`, label: 'Separator', node: separatorSelect }),
  )

  const said = make('p', 'ygg-ui__counter')
  said.setAttribute('aria-live', 'polite')
  const form = make('form', 'ygg-ui__fix-form')
  form.addEventListener('submit', (event) => event.preventDefault())
  const output = make('div')
  const actions = make('div', 'ygg-ui__controls')
  const validation = make('div', 'ygg-ui__status')
  validation.setAttribute('aria-live', 'polite')
  const validationError = make('p', 'ygg-ui__note ygg-ui__note--danger')
  validationError.setAttribute('role', 'alert')
  validationError.setAttribute('tabindex', '-1')
  validationError.hidden = true
  const detailFailure = make('div')
  const decodeButton = onDecode ? button({ label: 'Read it in the decoder', kind: 'primary' }) : null
  const validateButton = validate ? button({ label: 'Validate with the host' }) : null
  if (decodeButton !== null) actions.append(decodeButton)
  if (validateButton !== null) actions.append(validateButton)

  const values = initialValues(model, configuredValues)
  const added = new Set([...values.keys()].filter((tag) => tag !== 8))
  let shown = []
  let fieldNodes = new Map()
  let detail = null
  let detailStarted = false
  let draftVersion = 0
  let validationVersion = 0
  let current = null
  const load = detailLoader(model, loadDetails)

  if (!values.has(8)) values.set(8, String(beginString))

  const offered = () => {
    const message = model.messages.get(messageSelect.value)
    if (message === undefined) return []
    const scope = scopeSelect.value
    const tags =
      scope === 'declared'
        ? message.m
            .filter(([kind]) => kind === 'f')
            .map(([, tag, required]) => ({ tag, required: required === 1 }))
        : model.layoutTags(message)
    const seen = new Set()
    const kept = []
    for (const entry of tags) {
      if (seen.has(entry.tag)) continue
      const record = model.byTag.get(entry.tag)
      if (record === undefined || model.groupsByTag.has(entry.tag)) continue
      if (entry.tag === 9 || model.trailer.has(entry.tag)) continue
      const wanted =
        added.has(entry.tag) ||
        scope === 'declared' ||
        entry.required ||
        (scope === 'common' && seedValues.has(entry.tag))
      if (!wanted) continue
      seen.add(entry.tag)
      kept.push({ tag: entry.tag, required: entry.required })
    }
    for (const tag of added) {
      if (seen.has(tag)) continue
      const record = model.byTag.get(tag)
      if (record === undefined || model.groupsByTag.has(tag)) continue
      seen.add(tag)
      kept.push({ tag, required: false })
    }
    return kept
  }

  const compose = ({ preserveValidationIfUnchanged = false } = {}) => {
    const ordered = shown
      .filter((tag) => ![8, 9, 10, 35].includes(tag) && (values.get(tag) ?? '') !== '')
      .sort((left, right) => {
        const rank = (tag) => (model.header.has(tag) ? model.data.header.indexOf(tag) : 1000 + tag)
        return rank(left) - rank(right)
      })
    const next = composeFixText({
      model,
      messageType: messageSelect.value,
      beginString: values.get(8) ?? String(beginString),
      entries: ordered.map((tag) => [tag, values.get(tag)]),
      separator: separatorSelect.value,
    })
    const preserveValidation =
      preserveValidationIfUnchanged &&
      current !== null &&
      current.wire === next.wire &&
      current.display === next.display
    if (!preserveValidation) {
      if (validateButton?.disabled) {
        validationVersion += 1
        validateButton.disabled = false
        element.setAttribute('aria-busy', 'false')
      }
      current = next
      draftVersion += 1
    }
    output.textContent = ''
    output.append(
      cardRow({
        cards: [
          {
            value: messageSelect.value,
            label: 'message type',
            note: current.message?.n ?? '',
          },
          {
            value: String(current.pairs.length),
            label: 'tags',
            note: 'header, body, trailer',
          },
          { value: String(current.byteLength), label: 'bytes', note: `body ${current.bodyLength}` },
          { value: current.checksum, label: 'checksum', note: 'computed in this browser' },
        ],
      }),
    )
    if (current.conflicts.length > 0) {
      output.append(
        note({
          content:
            `A value of ${current.conflicts.join(', ')} carries the display separator ` +
            `“${current.separator}”. The raw wire below still uses SOH.`,
          kind: 'warning',
        }),
      )
    }
    output.append(
      wireBlock({
        display: current.display,
        copy: current.wire,
        labels: { copy: 'Copy raw wire', copied: 'Raw wire copied' },
      }).element,
    )
    if (!preserveValidation) {
      validation.textContent = ''
      validationError.hidden = true
    }
    return current
  }

  const paint = (entries, all, { preserveValidation = false } = {}) => {
    const active = element.ownerDocument?.activeElement
    let activeTag = null
    let selection = null
    for (const [tag, node] of fieldNodes) {
      if (node !== active) continue
      activeTag = tag
      selection = {
        start: node.selectionStart,
        end: node.selectionEnd,
        direction: node.selectionDirection,
      }
      break
    }
    form.textContent = ''
    shown = entries.map((entry) => entry.tag)
    const nextNodes = new Map()
    for (const entry of entries) {
      const record = model.byTag.get(entry.tag)
      if (record === undefined) continue
      const held = values.has(entry.tag) ? values.get(entry.tag) : (seedValues.get(entry.tag) ?? '')
      values.set(entry.tag, held)
      const set = all === null ? undefined : model.detail(record, all)?.c
      let node
      if (set !== undefined && set.length <= 60) {
        const options = [{ value: '', label: '—' }]
        if (held !== '' && !set.some(([value]) => value === held)) {
          options.push({ value: held, label: `${held} · not in the generated code set` })
        }
        options.push(
          ...set.map(([value, name, documentation]) => ({
            value,
            label: `${value} · ${name}${documentation ? ` — ${documentation}` : ''}`,
          })),
        )
        node = select({
          wide: true,
          value: held,
          options,
        })
      } else {
        node = input({ wide: true, value: held, autocomplete: 'off', placeholder: record.y })
      }
      if (entry.tag === 35) {
        node.value = messageSelect.value
        node.disabled = true
      }
      const update = () => {
        values.set(entry.tag, node.value)
        compose()
      }
      node.addEventListener('input', update)
      node.addEventListener('change', update)
      nextNodes.set(entry.tag, node)
      const label = [code({ text: String(entry.tag) }), ` ${model.titleOf(record)}`]
      if (entry.required) label.push(pill({ text: 'required', kind: 'required' }))
      const field = control({
        id: `ygg-ui-fix-tag-${++fixControlId}`,
        label,
        node,
        className: 'ygg-ui__fix-field',
      })
      if (record.x) field.append(make('p', 'ygg-ui__prose', record.x))
      form.append(field)
    }
    fieldNodes = nextNodes
    compose({ preserveValidationIfUnchanged: preserveValidation })
    const refocus = activeTag === null ? null : fieldNodes.get(activeTag)
    if (refocus !== undefined && refocus !== null) {
      refocus.focus()
      if (
        selection?.start !== undefined &&
        selection?.end !== undefined &&
        typeof refocus.setSelectionRange === 'function'
      ) {
        refocus.setSelectionRange(selection.start, selection.end, selection.direction)
      }
    }
  }

  const build = () => {
    const entries = offered()
    values.set(35, messageSelect.value)
    paint(entries, detail)
    if (!detailStarted && hasDetailSource(model, loadDetails)) {
      detailStarted = true
      load().then(
        (found) => {
          detail = found
          paint(offered(), detail, { preserveValidation: true })
        },
        (error) => {
          detailFailure.textContent = ''
          detailFailure.append(renderDetailFailure(error, renderDetailsError))
        },
      )
    }
    return entries
  }

  const setMessageType = (type) => {
    const value = String(type)
    if (!model.messages.has(value)) throw new RangeError(`unknown FIX message type ${value}`)
    messageSelect.value = value
    build()
    return current
  }

  const setValue = (key, value) => {
    const record = model.field(key)
    if (record === null) throw new RangeError(`unknown FIX field ${key}`)
    if (record.t === 35) return setMessageType(String(value))
    if (record.t === 9 || model.trailer.has(record.t)) {
      throw new RangeError(`FIX field ${record.t} is computed by the draft`)
    }
    if (model.groupsByTag.has(record.t)) {
      throw new RangeError(`FIX field ${record.t} introduces a repeating group`)
    }
    values.set(record.t, String(value))
    if (!shown.includes(record.t)) added.add(record.t)
    build()
    return current
  }

  const validateDraft = async () => {
    if (validate === undefined || current === null) return null
    const version = ++validationVersion
    const heldDraft = current
    const heldDraftVersion = draftVersion
    validateButton.disabled = true
    element.setAttribute('aria-busy', 'true')
    validation.textContent = 'Validating…'
    validationError.hidden = true
    try {
      const result = await validate(heldDraft.wire, heldDraft)
      if (version !== validationVersion || heldDraftVersion !== draftVersion) return null
      validation.textContent = ''
      appendValue(validation, result)
      return result
    } catch (error) {
      if (version !== validationVersion || heldDraftVersion !== draftVersion) return null
      validation.textContent = ''
      validationError.textContent = error instanceof Error ? error.message : String(error)
      validationError.hidden = false
      validationError.focus()
      return null
    } finally {
      if (version === validationVersion) {
        validateButton.disabled = false
        element.setAttribute('aria-busy', 'false')
      }
    }
  }

  extra.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    const wanted = extra.value.trim()
    if (wanted === '') return
    const record = model.field(wanted)
    extra.value = ''
    if (record === null) {
      said.textContent = `No field is named ${wanted}.`
      return
    }
    if (model.groupsByTag.has(record.t)) {
      said.textContent = `${model.titleOf(record)} (${record.t}) is a repeating group; the draft writes flat tags.`
      return
    }
    if (record.t === 9 || model.trailer.has(record.t)) {
      said.textContent = `${model.titleOf(record)} (${record.t}) is computed by the draft.`
      return
    }
    added.add(record.t)
    values.set(record.t, values.get(record.t) ?? '')
    said.textContent = `${model.titleOf(record)} (${record.t}) added.`
    build()
  })
  messageSelect.addEventListener('change', build)
  scopeSelect.addEventListener('change', build)
  separatorSelect.addEventListener('change', compose)
  decodeButton?.addEventListener('click', () => {
    if (current !== null) onDecode(current.wire, current)
  })
  validateButton?.addEventListener('click', () => void validateDraft())

  element.append(
    disclosure,
    controls,
    said,
    detailFailure,
    form,
    output,
    actions,
    validation,
    validationError,
  )
  build()
  return {
    element,
    messageSelect,
    scopeSelect,
    separatorSelect,
    form,
    output,
    status: validation,
    error: validationError,
    setMessageType,
    setValue,
    compose,
    validate: validateDraft,
    get draft() {
      return current
    },
  }
}

const fieldId = (field) => {
  const tag = field?.metadata?.['fix:tag']
  if (tag === undefined || tag === null) return null
  return `${tag}:${field.metadata?.['fix:branch'] ?? ''}`
}

const fieldTitle = (field) => field?.metadata?.display ?? field?.name ?? '(unnamed)'

const cloneFieldDocument = (field) => {
  if (field === null || typeof field !== 'object' || Array.isArray(field)) {
    throw new TypeError('each registry field must be one Field.toJSON object')
  }
  const encoded = JSON.stringify(field)
  if (encoded === undefined) throw new TypeError('a registry field must be JSON serializable')
  return JSON.parse(encoded)
}

const checkedFields = (fields) => {
  if (fields === null || fields === undefined || typeof fields[Symbol.iterator] !== 'function') {
    throw new TypeError('registry fields must be iterable Field.toJSON documents')
  }
  return Array.from(fields, cloneFieldDocument)
}

const checkedActions = (actions) => {
  for (const name of ['list', 'insert', 'update', 'removeById']) {
    if (typeof actions[name] !== 'function') {
      throw new TypeError(`registry actions.${name} must be a function`)
    }
  }
  return actions
}

/**
 * Edit canonical Field.toJSON documents through an authoritative host.
 * With no actions, a compact manifest is explored read-only and never round-tripped.
 */
export function fixRegistryEditor({
  model,
  fields,
  actions,
  loadDetails,
  renderDetailsError,
  pageSize = 60,
}) {
  if (model !== undefined && (fields !== undefined || actions !== undefined)) {
    throw new TypeError('compact model and canonical registry fields are separate editor modes')
  }
  if (actions === undefined) {
    if (model !== undefined) {
      const listing = fixFieldExplorer({
        model,
        loadDetails,
        renderDetailsError,
        pageSize,
      })
      const element = make('div', 'ygg-ui__registry-editor')
      element.append(
        note({
          content:
            'Read-only generated manifest. Editing requires full canonical Field documents and a host that runs FixRegistry mutations.',
          kind: 'info',
        }),
        listing.element,
      )
      return {
        element,
        listing,
        editor: null,
        status: null,
        error: null,
        refresh: async () => null,
        insert: async () => false,
        update: async () => false,
        remove: async () => false,
        select: () => false,
        get fields() {
          return null
        },
        get selectedId() {
          return null
        },
        get stale() {
          return false
        },
      }
    }
    if (fields === undefined) throw new TypeError('a read-only registry needs model or fields')
  }
  if (actions !== undefined && fields === undefined) {
    throw new TypeError('registry mutation requires full canonical Field.toJSON documents')
  }

  const hostActions = actions === undefined ? null : checkedActions(actions)
  const element = make('div', 'ygg-ui__registry-editor')
  const listingRoot = make('div')
  const editorRoot = make('form', 'ygg-ui__field-editor')
  editorRoot.addEventListener('submit', (event) => event.preventDefault())
  const editor = textarea({ rows: 18, spellcheck: false, wrap: 'off' })
  const editorId = `ygg-ui-fix-field-document-${++fixControlId}`
  const status = make('p', 'ygg-ui__status')
  status.setAttribute('role', 'status')
  status.setAttribute('aria-live', 'polite')
  const error = make('p', 'ygg-ui__note ygg-ui__note--danger')
  const errorId = `ygg-ui-fix-field-error-${++fixControlId}`
  error.id = errorId
  error.setAttribute('role', 'alert')
  error.setAttribute('tabindex', '-1')
  error.hidden = true
  editor.setAttribute('aria-describedby', errorId)
  editor.setAttribute('aria-invalid', 'false')
  const buttons = make('div', 'ygg-ui__controls')
  const freshButton = button({ label: 'New field' })
  const insertButton = button({ label: 'Insert / replace', kind: 'primary' })
  const updateButton = button({ label: 'Merge update' })
  const removeButton = button({ label: 'Remove', kind: 'danger' })
  const refreshButton = button({ label: 'Retry authoritative refresh' })
  refreshButton.hidden = true
  const mutationButtons = [freshButton, insertButton, updateButton, removeButton]
  let snapshot = checkedFields(fields)
  let selected = null
  let listing = null
  let pending = false
  let stale = false
  let confirming = false
  const editButtons = new Set()

  const resetConfirmation = () => {
    confirming = false
    removeButton.textContent = 'Remove'
  }

  const showError = (reason, { input = true } = {}) => {
    error.textContent = reason instanceof Error ? reason.message : String(reason)
    error.hidden = false
    editor.setAttribute('aria-invalid', input ? 'true' : 'false')
    if (input) {
      editor.setAttribute('aria-describedby', errorId)
      status.removeAttribute('aria-describedby')
    } else {
      editor.removeAttribute('aria-describedby')
      status.setAttribute('aria-describedby', errorId)
    }
    error.focus()
  }

  const clearError = () => {
    error.textContent = ''
    error.hidden = true
    editor.setAttribute('aria-invalid', 'false')
    editor.setAttribute('aria-describedby', errorId)
    status.removeAttribute('aria-describedby')
  }

  const selectField = (id, force = false) => {
    if (!force && pending) {
      status.textContent = 'A registry action is in progress; field selection is temporarily disabled.'
      return false
    }
    if (!force && stale) {
      status.textContent =
        'Field selection is locked until the authoritative registry refresh succeeds.'
      return false
    }
    const wanted = String(id)
    const field = snapshot.find((entry) => fieldId(entry) === wanted)
    if (field === undefined) return false
    selected = wanted
    editor.value = JSON.stringify(field, null, 2)
    status.textContent = `Editing ${wanted}.`
    clearError()
    resetConfirmation()
    return true
  }

  const renderListing = (query = listing?.input.value ?? '') => {
    listingRoot.textContent = ''
    editButtons.clear()
    const index = createSearchIndex({
      items: snapshot,
      haystack: (field) =>
        `${fieldId(field) ?? ''} ${field.name ?? ''} ${fieldTitle(field)} ${JSON.stringify(field.metadata ?? {})}`,
    })
    listing = searchableList({
      index,
      pageSize,
      noun: 'fields',
      label: 'Search registry',
      placeholder: 'tag, name, branch, alias…',
      query,
      renderPage: (kept) =>
        kept.map((field) => {
          const id = fieldId(field) ?? '(untagged)'
          const block = panel({
            title: id,
            aside: fieldTitle(field),
            render: () => {
              const content = [fieldDocument({ document: field, open: true })]
              if (hostActions !== null && fieldId(field) !== null) {
                const editButton = button({
                  label: `Edit ${id}`,
                  onClick: () => selectField(id),
                })
                editButton.disabled = pending || stale
                editButtons.add(editButton)
                content.unshift(editButton)
              }
              return content
            },
          })
          return block.element
        }),
      className: 'ygg-ui__registry-list',
    })
    listingRoot.append(listing.element)
    return listing
  }

  const setPending = (value) => {
    pending = value
    element.setAttribute('aria-busy', value ? 'true' : 'false')
    listingRoot.inert = value
    listingRoot.setAttribute('aria-disabled', value ? 'true' : 'false')
    editor.disabled = value || stale
    for (const node of mutationButtons) node.disabled = value || stale
    for (const node of editButtons) node.disabled = value || stale
    refreshButton.disabled = value
  }

  const documentFromEditor = () => {
    const value = JSON.parse(editor.value)
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      throw new TypeError('the field document must be one JSON object')
    }
    return value
  }

  const replaceFromHost = (next) => {
    const held = checkedFields(next)
    snapshot = held
    const query = listing?.input.value ?? ''
    renderListing(query)
  }

  const reconcileSelection = (wanted = selected) => {
    if (wanted !== null && snapshot.some((field) => fieldId(field) === wanted)) {
      return selectField(wanted, true)
    }
    selected = null
    editor.value = ''
    resetConfirmation()
    return false
  }

  const perform = async (operation) => {
    if (hostActions === null || pending) return false
    if (stale) {
      status.textContent =
        'Mutation controls remain locked until the authoritative registry reload succeeds.'
      return false
    }
    clearError()
    let argument
    try {
      if (operation === 'removeById') {
        if (selected === null) throw new Error('Select a canonical field before removing it.')
        argument = selected
      } else {
        argument = documentFromEditor()
      }
    } catch (reason) {
      showError(reason)
      return false
    }

    setPending(true)
    status.textContent =
      operation === 'insert'
        ? 'Inserting…'
        : operation === 'update'
          ? 'Updating…'
          : 'Removing…'
    const previous = selected
    try {
      try {
        await hostActions[operation](argument)
      } catch (reason) {
        showError(reason)
        status.textContent = 'Mutation failed; the displayed registry was not changed.'
        return false
      }

      let authoritative
      try {
        authoritative = checkedFields(await hostActions.list())
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason)
        stale = true
        refreshButton.hidden = false
        showError(
          new Error(
            `The mutation committed, but the authoritative registry could not be reloaded (${message}).`,
          ),
          { input: false },
        )
        status.textContent =
          'The displayed snapshot is unchanged and stale; mutations are locked until refresh succeeds.'
        return true
      }
      replaceFromHost(authoritative)
      stale = false
      refreshButton.hidden = true
      const nextId = operation === 'removeById' ? null : fieldId(argument)
      selected = nextId !== null && snapshot.some((field) => fieldId(field) === nextId) ? nextId : null
      reconcileSelection(selected)
      status.textContent =
        operation === 'insert'
          ? 'Authoritative registry reloaded after insert / replace.'
          : operation === 'update'
            ? 'Authoritative registry reloaded after merge update.'
            : `Authoritative registry reloaded after removing ${previous}.`
      resetConfirmation()
      return true
    } finally {
      setPending(false)
    }
  }

  const refresh = async () => {
    if (hostActions === null || pending) return null
    clearError()
    setPending(true)
    status.textContent = 'Loading the authoritative registry…'
    try {
      const authoritative = checkedFields(await hostActions.list())
      replaceFromHost(authoritative)
      stale = false
      refreshButton.hidden = true
      reconcileSelection()
      status.textContent = 'Authoritative registry loaded.'
      return checkedFields(snapshot)
    } catch (reason) {
      showError(reason, { input: false })
      status.textContent = stale
        ? 'Authoritative reload failed; the snapshot remains stale and mutations remain locked.'
        : 'The registry view was not changed.'
      return null
    } finally {
      setPending(false)
    }
  }

  renderListing()
  if (hostActions === null) {
    element.append(
      note({
        content: 'Read-only canonical Field documents. Supply registry actions to mutate them.',
        kind: 'info',
      }),
      listingRoot,
    )
  } else {
    editorRoot.append(control({ id: editorId, label: 'Canonical Field JSON', node: editor }))
    buttons.append(freshButton, insertButton, updateButton, removeButton, refreshButton)
    editorRoot.append(buttons, status, error)
    freshButton.addEventListener('click', () => {
      if (pending || stale) return
      selected = null
      editor.value = JSON.stringify(
        {
          dtype: { type: 'utf8' },
          metadata: { 'fix:tag': '' },
          name: '',
          nullable: true,
        },
        null,
        2,
      )
      status.textContent = 'New field draft. The host validates every registry rule.'
      clearError()
      resetConfirmation()
      editor.focus()
    })
    insertButton.addEventListener('click', () => void perform('insert'))
    updateButton.addEventListener('click', () => void perform('update'))
    removeButton.addEventListener('click', () => {
      if (pending || stale) return
      if (selected === null) {
        showError(new Error('Select a canonical field before removing it.'))
        return
      }
      if (!confirming) {
        confirming = true
        removeButton.textContent = `Confirm remove ${selected}`
        status.textContent = `Confirm removal of ${selected}.`
        return
      }
      void perform('removeById')
    })
    refreshButton.addEventListener('click', () => void refresh())
    element.append(listingRoot, editorRoot)
  }

  return {
    element,
    get listing() {
      return listing
    },
    editor,
    status,
    error,
    select: selectField,
    refresh,
    insert: () => perform('insert'),
    update: () => perform('update'),
    remove: () => perform('removeById'),
    get fields() {
      return checkedFields(snapshot)
    },
    get selectedId() {
      return selected
    },
    get stale() {
      return stale
    },
  }
}
