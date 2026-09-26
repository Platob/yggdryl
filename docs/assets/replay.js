/*
 * Render docs/assets/replay.json: the trading replay, recorded.
 *
 * The replay service is Node over a native addon, so this page cannot run it.
 * scripts/build_docs_replay.js ran the package over two sources and recorded
 * what each route answered; this module mounts the package's own application
 * (docs/assets/web/app/app.js, copied from node/web/app/) over an `api` that
 * answers from that record, and renders two sections of it beside. Nothing is
 * computed here - no book folded, no reading, hash or diff taken: a route the
 * record does not hold is refused, and says so.
 *
 * The application runs in a frame of its own, so its full-height layout and
 * its page-wide keyboard map meet no documentation around them; the frame's
 * document styles its own html and body, as the served page does. This one
 * module is both sides: on the documentation page it builds the frame and the
 * sections; in the frame, which names it again, it mounts the application.
 *
 * The script is loaded on every page and does nothing where no container asks
 * for it.
 */

const MANIFEST = new URL('replay.json', import.meta.url)
const WEB = new URL('web/', import.meta.url)
const COMMAND = 'node scripts/build_docs_replay.js'
const FRAME = 'data-replay-frame'
const GLOBAL = 'GLOBAL'

let pending = null

/** Fetch the manifest once per document, whatever asks for it first. */
function manifest() {
  pending ??= fetch(MANIFEST).then((answer) => {
    if (!answer.ok) throw new Error(`${answer.status} ${answer.statusText}`)
    return answer.json()
  })
  return pending
}

function make(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

/** A table of `rows` under `head`, each cell its text. */
function table(className, head, rows) {
  const node = make('table', className)
  const top = make('thead')
  const line = make('tr')
  for (const name of head) {
    const cell = make('th', null, name)
    cell.scope = 'col'
    line.append(cell)
  }
  top.append(line)
  const body = make('tbody')
  for (const row of rows) {
    const tr = make('tr')
    for (const value of row) tr.append(make('td', null, value))
    body.append(tr)
  }
  node.append(top, body)
  return node
}

/** A served value as a cell shows it: text as it is, a map's pairs and a list as their JSON, absence as a dash. */
function shown(value) {
  if (value === null || value === undefined) return '—'
  return typeof value === 'object' ? JSON.stringify(value) : String(value)
}

/** The JavaScript and the route behind a recorded answer. */
function call(record, key) {
  const held = record.calls[key]
  const block = make('pre', 'ygg-rp__call')
  block.append(make('code', null, held.route ? `${held.call}\n// ${held.route}` : held.call))
  return block
}

function fail(roots, reason) {
  for (const root of roots) {
    root.textContent = ''
    const note = make('p', 'ygg-rp__error')
    note.textContent =
      `The generated manifest assets/replay.json could not be loaded (${reason}). ` +
      'It is committed, and a local build writes it with:'
    const command = make('pre', 'ygg-rp__call')
    command.append(make('code', null, COMMAND))
    root.append(note, command)
  }
}

// ---- the documentation page ------------------------------------------------

/** The frame the application runs in: this module again, told by `FRAME` which side it is. */
function renderApp(root) {
  root.textContent = ''
  const frame = make('iframe', 'ygg-rp__frame')
  frame.title = 'The replay application over the recorded answers'
  frame.srcdoc = [
    '<!doctype html>',
    `<html lang="en" ${FRAME} data-readonly>`,
    '<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">',
    '<title>Recorded replay</title><link rel="icon" href="data:,">',
    `<link rel="stylesheet" href="${new URL('theme.css', WEB)}">`,
    `<link rel="stylesheet" href="${new URL('app/app.css', WEB)}">`,
    '<style>html, body { height: 100%; margin: 0; } body { background: var(--ygg-ui-page); }</style>',
    `<script type="module" src="${import.meta.url}"></script></head>`,
    '<body><div data-replay-root><p>Loading the recorded replay.</p></div></body></html>',
  ].join('')
  root.append(frame)
}

/** The recorded `orders` view with its lifts, per source: the identifying columns, then every column. */
function renderViews(root, data) {
  root.textContent = ''
  for (const [id, record] of Object.entries(data.sources)) {
    const lifted = record.views.orders.find((answer) => answer.lifts.length && answer.global.includes(false))
    const section = make('section', 'ygg-rp__view')
    section.append(make('h3', null, `${id}: ${lifted.rows.length} rows, lifts ${lifted.lifts.join(', ')}`))
    const names = ['kind', 'currunix', 'crosscode', 'state', 'side', 'price', 'quantity', ...lifted.lifts.map((lift) => lift.split(' as ').pop())]
    section.append(
      table(
        'ygg-rp__table',
        names,
        lifted.rows.map((row) => names.map((name) => shown(row[name]))),
      ),
    )
    const all = make('details', 'ygg-rp__all')
    all.append(make('summary', null, `Every column (${lifted.columns.length})`))
    const scroll = make('div', 'ygg-rp__scroll')
    const columns = lifted.columns.map((column) => column.name)
    scroll.append(table('ygg-rp__table', columns, lifted.rows.map((row) => columns.map((name) => shown(row[name])))))
    all.append(scroll, call(record, 'orders'))
    section.append(all)
    root.append(section)
  }
}

/** The recorded scenario: the bodies posted, the events stored, and the diff per instant. */
async function renderScenario(root, data) {
  const [{ changeLines, countChanges }, { formatInstant }] = await Promise.all([
    import(new URL('diff-view.js', WEB)),
    import(new URL('instant.js', WEB)),
  ])
  root.textContent = ''
  const [id, record] = Object.entries(data.sources).find(([, held]) => held.scenario)
  const { scenario } = record
  root.append(make('h3', null, `${scenario.name} over ${id}: ${scenario.events.length} inserted events`))
  root.append(
    table(
      'ygg-rp__table',
      ['currunix', 'kind', 'crosscode', 'side', 'price', 'quantity', 'metadata', 'stableHash'],
      scenario.events.map((event) =>
        [formatInstant(event.currunix), event.kind, event.crosscode, event.side, event.price, event.quantity, event.metadata, event.stableHash].map(shown),
      ),
    ),
  )
  const bodies = make('details', 'ygg-rp__all')
  bodies.append(make('summary', null, 'The bodies posted'))
  const posted = make('pre', 'ygg-rp__call')
  posted.append(make('code', null, JSON.stringify(scenario.inserted, null, 2)))
  bodies.append(posted, call(record, 'scenario'))
  root.append(bodies)

  const controls = make('div', 'ygg-rp__controls')
  const label = make('label', null, 'Symbol')
  const select = make('select', 'ygg-rp__select')
  select.id = 'ygg-rp-diff-symbol'
  label.htmlFor = select.id
  for (const symbol of Object.keys(record.diff)) select.append(new Option(symbol, symbol))
  controls.append(label, select)
  const holder = make('div')
  // A book only one stream holds reads as added or removed, as the diff view reads it.
  const changed = (instant) => {
    if (instant.base === null) return 'added: the scenario stream alone holds a book here'
    if (instant.scenario === null) return 'removed: the base stream alone holds a book here'
    if (instant.changes.same) return 'same'
    return `${countChanges(instant.changes)}: ${changeLines(instant.changes).join('; ')}`
  }
  const draw = () => {
    const diff = record.diff[select.value]
    holder.textContent = ''
    holder.append(
      make('p', 'ygg-rp__note', `Re-run from ${diff.from === null ? 'no instant' : formatInstant(diff.from)}; ${diff.instants.length} instants.`),
      table(
        'ygg-rp__table ygg-rp__diff',
        ['at', 'base', 'scenario', 'changes'],
        diff.instants.map((instant) => [
          formatInstant(instant.at),
          shown(instant.base),
          shown(instant.scenario),
          changed(instant),
        ]),
      ),
    )
  }
  select.addEventListener('change', draw)
  draw()
  root.append(controls, holder, call(record, 'diff'))
}

function start() {
  const roots = [...document.querySelectorAll('[data-replay]')]
  if (roots.length === 0) return
  for (const root of roots.filter((node) => node.dataset.replay === 'app')) renderApp(root)
  const sections = roots.filter((node) => node.dataset.replay !== 'app')
  if (sections.length === 0) return
  manifest().then(
    async (data) => {
      for (const root of sections) {
        if (root.dataset.replay === 'views') renderViews(root, data)
        else if (root.dataset.replay === 'scenario') await renderScenario(root, data)
      }
    },
    (error) => fail(sections, error.message),
  ).catch((error) => fail(sections, error.message))
}

// ---- the frame ---------------------------------------------------------------

const GRID = 'This page recorded the walks without a snapshot grid; a grid needs the replay service.'
const READONLY = 'Inserting events needs the replay service; this page renders a recorded replay.'

/** Two lists of lifts are one when they state the same lifts in the same order. */
function sameLifts(left, right) {
  return left.length === right.length && left.every((lift, at) => lift === right[at])
}

/**
 * The application's `api` (`node/web/app/api.js`'s method surface) answered
 * from the record: each answer a fresh copy, as the wire would give, and a
 * route the record does not hold refused as the service refuses, with a
 * sentence naming what the page holds.
 */
function recordedApi(data, readonly) {
  const answer = (value) => Promise.resolve(structuredClone(value))
  const refuse = (refusal, status = 0) => Promise.resolve({ refusal, status })
  const ids = Object.keys(data.sources)
  const record = (id) => (Object.hasOwn(data.sources, id) ? data.sources[id] : null)
  const unknown = (id) => refuse(`no source ${JSON.stringify(id)}; the sources are ${ids.join(', ')}`, 404)
  const at = (book) => BigInt(book.snapunix ?? book.currunix)
  const between = (list, from, to) =>
    list.filter(
      (item) =>
        (from === undefined || from === null || at(item) >= BigInt(from)) && (to === undefined || to === null || at(item) <= BigInt(to)),
    )
  const walkOf = (walks, query = {}) => {
    const symbol = query.global ? GLOBAL : query.symbol
    return symbol !== undefined && symbol !== null && Object.hasOwn(walks, symbol) ? [symbol, walks[symbol]] : [symbol, null]
  }
  const gridded = (query = {}) => Number(query.snapshotMillis ?? 0) > 0
  const stream = async (list, end, { onBook, signal } = {}) => {
    for (const book of list) {
      if (signal?.aborted) throw signal.reason
      onBook?.(structuredClone(book))
    }
    return { count: list.length, ...end }
  }
  const scenarioOf = (name) => Object.values(data.sources).find((held) => held.scenario?.name === name)?.scenario ?? null
  const sourceOfScenario = (name) => ids.find((id) => data.sources[id].scenario?.name === name)

  return Object.freeze({
    sources: () => answer(data.listing),
    views: () => answer({ views: data.listing.views }),
    field: () => answer(data.field),
    books: (id, query, handlers) => {
      const held = record(id)
      if (!held) return unknown(id)
      if (gridded(query)) return refuse(GRID)
      const [symbol, list] = walkOf(held.books, query)
      if (!list) return refuse(`no symbol ${JSON.stringify(symbol)} in this walk; its symbols are ${Object.keys(held.books).join(', ')}`, 404)
      return stream(between(list, query.from, query.to), {}, handlers)
    },
    book: (id, query = {}) => {
      const held = record(id)
      if (!held) return unknown(id)
      if (gridded(query)) return refuse(GRID)
      const [symbol, list] = walkOf(held.books, query)
      if (!list) return refuse(`no symbol ${JSON.stringify(symbol)} in this walk; its symbols are ${Object.keys(held.books).join(', ')}`, 404)
      const standing = between(list, undefined, query.at).at(-1)
      return standing ? answer(standing) : refuse(`no book of ${symbol} stands at ${query.at}`, 404)
    },
    lifecycle: (id, crosscode) => {
      const held = record(id)
      if (!held) return unknown(id)
      const { columns, rows } = held.lifecycle
      return answer({ columns, rows: Object.hasOwn(rows, crosscode) ? rows[crosscode] : [] })
    },
    view: (id, { view, lifts = [], ...query } = {}) => {
      const held = record(id)
      if (!held) return unknown(id)
      if (gridded(query)) return refuse(GRID)
      // The lifecycle view of one chain answers as the lifecycle route does for its cross code, which is recorded.
      if (view === 'lifecycle' && query.crosscode !== undefined && query.crosscode !== null && lifts.length === 0) {
        const { columns, rows } = held.lifecycle
        return answer({ columns, rows: Object.hasOwn(rows, query.crosscode) ? rows[query.crosscode] : [] })
      }
      const recorded = Object.hasOwn(held.views, view) ? held.views[view] : null
      if (!recorded) {
        return refuse(`This page recorded the ${Object.keys(held.views).join(', ')} views; the ${view} view needs the replay service.`)
      }
      const found = recorded.find((entry) => sameLifts(entry.lifts, lifts) && entry.global.includes(Boolean(query.global)))
      if (!found) {
        const spelled = [...new Set(recorded.map((entry) => (entry.lifts.length ? entry.lifts.join(', ') : 'no lift')))].join('; ')
        return refuse(`This page recorded the ${view} view with ${spelled}; other lifts need the replay service.`)
      }
      if (found.refusal) return refuse(found.refusal, found.status)
      return answer({ columns: found.columns, rows: found.rows })
    },
    scenarios: () =>
      answer({
        scenarios: Object.values(data.sources)
          .filter((held) => held.scenario)
          .map(({ scenario }) => ({ name: scenario.name, events: scenario.events })),
      }),
    saveScenario: () => refuse(readonly),
    deleteScenario: () => refuse(readonly),
    insert: () => refuse(readonly),
    removeEvent: () => refuse(readonly),
    scenarioBooks: (id, name, query = {}, handlers) => {
      const held = record(id)
      if (!held) return unknown(id)
      if (!scenarioOf(name)) return refuse(`no scenario ${JSON.stringify(name)}`, 404)
      if (held.scenario?.name !== name) {
        return refuse(`This page recorded the scenario ${name} over the ${sourceOfScenario(name)} source; over ${id} it needs the replay service.`)
      }
      if (gridded(query)) return refuse(GRID)
      const [symbol, list] = walkOf(held.scenario.books, query)
      if (!list) return refuse(`no symbol ${JSON.stringify(symbol)} in this walk; its symbols are ${Object.keys(held.scenario.books).join(', ')}`, 404)
      const from = query.from ?? held.scenario.from ?? undefined
      return stream(between(list, from, query.to), { from: held.scenario.from }, handlers)
    },
    diff: (id, name, query = {}) => {
      const held = record(id)
      if (!held) return unknown(id)
      if (!scenarioOf(name)) return refuse(`no scenario ${JSON.stringify(name)}`, 404)
      if (held.scenario?.name !== name) {
        return refuse(`This page recorded the scenario ${name} over the ${sourceOfScenario(name)} source; over ${id} it needs the replay service.`)
      }
      if (gridded(query)) return refuse(GRID)
      const [symbol, diff] = walkOf(held.diff, query)
      if (!diff) return refuse(`no symbol ${JSON.stringify(symbol)} in the base or the scenario walk`, 404)
      const instants = diff.instants.filter(
        (instant) =>
          (query.from === undefined || query.from === null || BigInt(instant.at) >= BigInt(query.from)) &&
          (query.to === undefined || query.to === null || BigInt(instant.at) <= BigInt(query.to)),
      )
      return answer({ symbol: diff.symbol, from: diff.from, instants })
    },
  })
}

/** Follow the documentation's colour scheme when the application opens; its own switch decides after that. */
function followScheme() {
  try {
    const scheme = window.parent.document.body.getAttribute('data-md-color-scheme')
    if (scheme) document.documentElement.dataset.theme = scheme === 'slate' ? 'dark' : 'light'
  } catch {
    // A frame the page cannot read keeps the preference's theme.
  }
}

async function mountRecorded() {
  const root = document.querySelector('[data-replay-root]')
  try {
    const [{ mountApp, READONLY_SENTENCE }, data] = await Promise.all([import(new URL('app/app.js', WEB)), manifest()])
    followScheme()
    mountApp(root, { api: recordedApi(data, READONLY_SENTENCE ?? READONLY), readonly: true, search: '', debug: false })
  } catch (error) {
    root.textContent = ''
    const note = make('p', 'ygg-ui ygg-ui__muted')
    note.textContent = `The recorded replay could not be mounted (${error.message}). A local build writes it with: ${COMMAND}`
    root.append(note)
  }
}

if (document.documentElement.hasAttribute(FRAME)) {
  mountRecorded()
} else if (typeof document$ !== 'undefined' && document$ && typeof document$.subscribe === 'function') {
  // Material's instant navigation swaps the document without reloading this
  // module, so the render is driven by its document observable where it exists.
  document$.subscribe(start)
} else if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', start)
} else {
  start()
}
