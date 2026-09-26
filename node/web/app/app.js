// The replay application: the component library composed over one store and
// the replay service's wire (`api.js`). Every fact on the page is what the
// service answered - the walk's books, a view's rows, a lifecycle, a diff, a
// refusal - and the app only chooses what to ask for and where to show it:
// which book of the loaded window stands (an index, moved by instants the
// books state), which executions have printed by then, which scenario stream
// is shown. Components never reach into each other: each takes a slice of the
// state through `store.select`, emits what the person wants, and one handler
// per command id (`shortcuts.js`) changes the store or calls the service.
// `mountApp(root, { api, readonly })` is the whole composition, so a page
// holding a recorded replay mounts it over a recorded `api` of its own.

import { element } from '../component.js'
import { DepthChart } from '../depth-chart.js'
import { DiffView } from '../diff-view.js'
import { InsertForm } from '../insert-form.js'
import { formatInstant, parseInstant } from '../instant.js'
import { Ladder } from '../ladder.js'
import { Lifecycle } from '../lifecycle.js'
import { Metrics } from '../metrics.js'
import { Modal } from '../modal.js'
import { Palette } from '../palette.js'
import { PriceChart } from '../price-chart.js'
import { ScenarioDrawer } from '../scenario-drawer.js'
import { ShortcutsSheet } from '../shortcuts-sheet.js'
import { commandFor, displayChord, SHORTCUTS } from '../shortcuts.js'
import { Split } from '../split.js'
import { createStore } from '../store.js'
import { GLOBAL, SymbolSelect } from '../symbol-select.js'
import { Tabs } from '../tabs.js'
import { Tape } from '../tape.js'
import { ThemeToggle } from '../theme-toggle.js'
import { Toasts } from '../toast.js'
import { SPEEDS, Transport } from '../transport.js'

import { createApi } from './api.js'

/** What the insert modal says in its form's place on a page with no service behind it. */
export const READONLY_SENTENCE = 'Inserting events needs the replay service; this page renders a recorded replay.'

/** The grid width the toggle applies when the service names none. */
const GRID_MILLIS = 1000

/** Rows a view table lays out; the count of the rest is stated. */
const VIEW_ROWS = 1000

/** How a chain is chosen: said in the inspector, and by the lifecycle view while none is. */
const CHOOSE_CHAIN = 'Choose a limit in the ladder or a print in the tape to read its lifecycle.'

const DIFF_TAB = 'diff'
const VIEW_TAB = 'view:'

let serial = 0

// ---- reading the loaded window ---------------------------------------------

const INSTANTS = new WeakMap()
const TAPES = new WeakMap()

/** The books' own instants as bigints, read once per loaded window. */
function instantsOf(books) {
  let held = INSTANTS.get(books)
  if (held === undefined) {
    held = books.map((book) => parseInstant(book.currunix))
    INSTANTS.set(books, held)
  }
  return held
}

/** The last book at or before `at`, by the books' own instants; -1 when none stands yet. */
export function indexAt(books, at) {
  const instants = instantsOf(books)
  const target = parseInstant(at)
  let low = 0
  let high = instants.length
  while (low < high) {
    const middle = (low + high) >>> 1
    if (instants[middle] <= target) low = middle + 1
    else high = middle
  }
  return low - 1
}

/** The book standing at `at` in another stream, the first when none stands yet. */
function follow(books, at) {
  if (!books?.length || at === undefined || at === null) return 0
  return Math.max(0, indexAt(books, at))
}

/**
 * The time and sales at book `index`: every execution the window's books
 * carry, in walk order, up to and including that book's, newest first -
 * concatenated and reversed, never sorted. The reversed list is built once
 * per loaded window, and a book shows its tail; a step that prints nothing
 * answers the list the step before answered.
 */
export function executionsAt(books, index) {
  let held = TAPES.get(books)
  if (held === undefined) {
    const newest = []
    const upto = books.map((book) => {
      for (const execution of book.executions ?? []) newest.push(execution)
      return newest.length
    })
    newest.reverse()
    held = { newest, upto, end: 0, shown: Object.freeze([]) }
    TAPES.set(books, held)
  }
  const end = index >= 0 ? (held.upto[Math.min(index, held.upto.length - 1)] ?? 0) : 0
  if (end !== held.end) {
    held.end = end
    held.shown = Object.freeze(held.newest.slice(held.newest.length - end))
  }
  return held.shown
}

/** The instant the walk reads a book at: its grid `snapunix`, else its `currunix`. */
function walkInstant(book) {
  return parseInstant(book.snapunix ?? book.currunix)
}

/** The ns bucket the price chart folds candles by: a sixtieth of the window, one ns at least. */
function bucketOf(books) {
  if (books.length < 2) return 1_000_000_000n
  const span = parseInstant(books.at(-1).currunix) - parseInstant(books[0].currunix)
  const bucket = span / 60n
  return bucket > 0n ? bucket : 1n
}

/** A served cell as text: a map's `[key, value]` pairs as `key=value`, a list joined, a record as JSON. */
function cellText(value) {
  if (value === null || value === undefined) return ''
  if (Array.isArray(value)) {
    return value.map((item) => (Array.isArray(item) && item.length === 2 ? `${cellText(item[0])}=${cellText(item[1])}` : cellText(item))).join(', ')
  }
  return typeof value === 'object' ? JSON.stringify(value) : String(value)
}

// ---- the state ---------------------------------------------------------------

/** Whether the scenario stream is the one shown. */
const onScenario = (state) => state.stream === 'scenario' && state.scenarioBooks !== null
const shownBooks = (state) => (onScenario(state) ? state.scenarioBooks : state.books)
const shownIndex = (state) => (onScenario(state) ? state.scenarioIndex : state.index)
const currentBook = (state) => shownBooks(state)[shownIndex(state)] ?? null

/** The walk a request names: the symbol, the consolidated mode, the grid when it is on. */
function walkQuery(state) {
  return { symbol: state.symbol, global: state.global, snapshotMillis: state.grid ? state.snapshotMillis : 0 }
}

/** One text per walk: a change to any part of it re-opens the stream. */
function walkKey(state) {
  if (!state.source || !state.symbol) return ''
  const { symbol, global, snapshotMillis } = walkQuery(state)
  return `${state.source}\n${symbol}\n${global}\n${snapshotMillis}`
}

/** The fields a walk is named by, as a loaded window keeps them; its width only when the grid is on. */
function walkFields({ source, symbols, symbol, global, grid, snapshotMillis }) {
  return { source, symbols, symbol, global, grid, ...(grid ? { snapshotMillis } : {}) }
}

/** One text per scenario window: the walk and the scenario it re-runs. */
function scenarioKey(state) {
  return `${walkKey(state)}\n${state.scenario ?? ''}`
}

/** `build(...inputs)`, answered again only when one input changes (`Object.is`). */
function memo(pick, build) {
  let inputs
  let value
  return (state) => {
    const next = pick(state)
    if (inputs && next.every((item, at) => Object.is(item, inputs[at]))) return value
    inputs = next
    value = build(...next)
    return value
  }
}

// ---- the page ------------------------------------------------------------------

/**
 * Mount the replay application in `root`. `api` answers the service's routes
 * (`createApi` over the page's own origin by default); `readonly` - by default
 * `data-readonly` on `<html>` - turns the insert modal into a sentence and
 * scenario editing off; `debug` - by default `?debug=1` - exposes the store as
 * `window.__store`; `keys` is `'page'` (every key the page receives) or
 * `'app'` (only keys pressed inside `root`). Answers `{ store, command,
 * ready, destroy }`; `ready` settles once the first window has loaded.
 */
export function mountApp(root, options = {}) {
  const params = new URLSearchParams(options.search ?? globalThis.location?.search ?? '')
  const api = options.api ?? createApi(globalThis.location.origin)
  const readonly = options.readonly ?? document.documentElement.hasAttribute('data-readonly')
  const debug = options.debug ?? params.has('debug')
  const keys = options.keys ?? 'page'
  const uid = `ygg-app-${++serial}`
  const requestedMillis = Number(params.get('snapshotMillis'))

  const store = createStore({
    sources: [],
    source: null,
    symbols: [],
    symbol: null,
    global: false,
    snapshotMillis: Number.isSafeInteger(requestedMillis) && requestedMillis > 0 ? requestedMillis : GRID_MILLIS,
    grid: false,
    books: Object.freeze([]),
    // The walk the loaded window is: its key and the fields that name it. A refused walk puts them back.
    walk: null,
    index: 0,
    loading: false,
    playing: false,
    speed: 1,
    selected: null,
    lifecycle: null,
    columns: [],
    kinds: [],
    views: [],
    tab: null,
    view: null,
    lifts: [],
    viewAnswer: null,
    scenarios: [],
    scenario: null,
    scenarioBooks: null,
    scenarioIndex: 0,
    scenarioFrom: null,
    stream: 'base',
    diff: null,
    theme: null,
    refusal: null,
    readonly,
    sizes: {},
  })

  const forget = []
  const listen = (target, type, handler, listenerOptions) => {
    target.addEventListener(type, handler, listenerOptions)
    forget.push(() => target.removeEventListener(type, handler, listenerOptions))
  }

  // ---- the shell -------------------------------------------------------------

  root.classList.add('ygg-app')
  if (readonly) root.dataset.readonly = ''
  root.replaceChildren()

  const header = element('header', 'ygg-ui ygg-app__header')
  const title = element('h1', 'ygg-app__title', 'Replay')
  const sourceName = element('span', 'ygg-app__source-name ygg-ui__muted')
  const sourceLabel = element('label', 'ygg-app__field')
  sourceLabel.append(element('span', 'ygg-ui__muted', 'Source '))
  const sourceSelect = element('select', 'ygg-app__source')
  sourceLabel.append(sourceSelect)
  const streamLabel = element('label', 'ygg-app__field')
  streamLabel.append(element('span', 'ygg-ui__muted', 'Stream '))
  const streamSelect = element('select', 'ygg-app__stream')
  streamSelect.append(new Option('Base replay', 'base'), new Option('Scenario', 'scenario'))
  streamLabel.append(streamSelect)
  const gridLabel = element('label', 'ygg-app__field')
  gridLabel.append(element('span', 'ygg-ui__muted', 'Grid ms '))
  const gridInput = element('input', 'ygg-app__grid ygg-ui__num')
  gridInput.type = 'number'
  gridInput.min = '1'
  gridInput.step = '1'
  gridInput.setAttribute('aria-describedby', `${uid}-grid-help`)
  gridLabel.append(gridInput)
  const gridHelp = element('span', 'ygg-ui__sr-only', 'The snapshot grid width the Grid control applies, in milliseconds')
  gridHelp.id = `${uid}-grid-help`
  const status = element('p', 'ygg-app__status ygg-ui__muted')
  status.setAttribute('role', 'status')
  const actions = element('div', 'ygg-app__actions')
  const actionButton = (text, id) => {
    const button = element('button', 'ygg-ui__button', text)
    button.type = 'button'
    button.dataset.command = id
    const chord = SHORTCUTS.find(([, command]) => command === id)?.[0]
    if (chord) button.setAttribute('aria-keyshortcuts', chord.replace('Mod', 'Control'))
    if (chord) button.title = `${text} (${displayChord(chord)})`
    return button
  }
  const insertButton = actionButton('Insert event', 'scenario.insert')
  const scenariosButton = element('button', 'ygg-ui__button', 'Scenarios')
  scenariosButton.type = 'button'
  scenariosButton.title = `Scenarios (${displayChord('s')})`
  scenariosButton.setAttribute('aria-keyshortcuts', 's')
  const diffButton = actionButton('Diff', 'scenario.diff')
  const paletteButton = actionButton('Commands', 'palette.open')
  const keysButton = actionButton('Keys', 'help.shortcuts')
  actions.append(insertButton, scenariosButton, diffButton, paletteButton, keysButton)
  header.append(title, sourceName, sourceLabel)
  const symbolSelect = new SymbolSelect().mount(header)
  header.append(streamLabel, gridLabel, gridHelp, status, actions)
  const themeToggle = new ThemeToggle().mount(header)
  root.append(header)

  const transportHolder = element('div', 'ygg-app__transport')
  root.append(transportHolder)
  const transport = new Transport().mount(transportHolder)

  const main = element('main', 'ygg-app__main')
  root.append(main)
  const rows = new Split({ direction: 'vertical', sizes: [66, 34], label: 'the replay and its tables' }).mount(main)
  const columns = new Split({ direction: 'horizontal', sizes: [32, 30, 38], label: 'the book, the prices and the tape' }).mount(rows.panes[0])
  const [bookPane, pricePane, tapePane] = columns.panes
  for (const pane of [bookPane, pricePane, tapePane, rows.panes[1]]) pane.classList.add('ygg-app__pane')

  const ladders = element('div', 'ygg-app__ladders')
  bookPane.append(ladders)
  const bidLadder = new Ladder({ side: 'bid', rows: 10 }).mount(ladders)
  const askLadder = new Ladder({ side: 'ask', rows: 10 }).mount(ladders)
  const depth = new DepthChart({ height: 180 }).mount(bookPane)

  const priceHolder = element('div', 'ygg-app__price')
  pricePane.append(priceHolder)
  let priceChart = null
  const metrics = new Metrics().mount(pricePane)

  const tape = new Tape({ rows: 8 }).mount(tapePane)
  const inspector = element('section', 'ygg-ui ygg-app__inspector')
  inspector.setAttribute('aria-label', 'Inspector')
  const inspectorHint = element('p', 'ygg-ui__muted', CHOOSE_CHAIN)
  inspector.append(inspectorHint)
  tapePane.append(inspector)
  const lifecycle = new Lifecycle().mount(inspector)

  // The views' body moves into whichever view tab is chosen: one lifts form, one table.
  const viewBody = element('div', 'ygg-app__view')
  const liftsForm = element('form', 'ygg-app__lifts')
  liftsForm.noValidate = true
  const liftsLabel = element('label', 'ygg-app__field')
  liftsLabel.append(element('span', 'ygg-ui__muted', 'Lifts, one per line '))
  const liftsInput = element('textarea', 'ygg-app__lifts-input ygg-ui__mono')
  liftsInput.rows = 2
  liftsInput.spellcheck = false
  liftsInput.placeholder = "securityids['ISIN'] as isin"
  liftsLabel.append(liftsInput)
  const liftsApply = element('button', 'ygg-ui__button', 'Apply')
  liftsApply.type = 'submit'
  liftsForm.append(liftsLabel, liftsApply)
  const viewStatus = element('p', 'ygg-app__view-status ygg-ui__muted')
  viewStatus.setAttribute('role', 'status')
  const viewRefusal = element('p', 'ygg-ui__error ygg-app__view-refusal')
  viewRefusal.setAttribute('role', 'alert')
  viewRefusal.hidden = true
  const viewScroll = element('div', 'ygg-app__view-scroll')
  const viewTable = element('table', 'ygg-ui__table ygg-app__view-table')
  viewScroll.append(viewTable)
  viewBody.append(liftsForm, viewStatus, viewRefusal, viewScroll)

  const diffBody = element('div', 'ygg-app__diff')
  const diffHint = element('p', 'ygg-ui__muted', 'Choose or create a scenario to compare it with the base replay.')
  diffBody.append(diffHint)
  const diffView = new DiffView().mount(diffBody)
  let tabs = null

  const drawer = new ScenarioDrawer().mount(root)
  drawer.bind(scenariosButton)

  const insertModal = new Modal({ title: 'Insert an event' }).mount(root)
  let insertForm = null
  const insertName = element('input', 'ygg-app__scenario-name ygg-ui__mono')
  if (readonly) {
    insertModal.body.append(element('p', 'ygg-app__readonly', READONLY_SENTENCE))
  } else {
    const nameRow = element('div', 'ygg-ui__insert-field ygg-app__insert-scenario')
    const nameLabel = element('label', 'ygg-ui__insert-label', 'Scenario')
    insertName.type = 'text'
    insertName.autocomplete = 'off'
    insertName.spellcheck = false
    insertName.id = `${uid}-insert-scenario`
    nameLabel.htmlFor = insertName.id
    nameRow.append(nameLabel, insertName)
    insertModal.body.append(nameRow)
    insertForm = new InsertForm().mount(insertModal.body)
  }

  const palette = new Palette().mount(root)
  const sheet = new ShortcutsSheet().mount(root)
  const toasts = new Toasts().mount(root)

  // ---- components over the store ------------------------------------------------

  const unbind = []
  const bind = (component, selector) => {
    component.update(selector(store.get()))
    unbind.push(store.select(selector, (value) => component.update(value)))
  }
  const watch = (selector, listener, now = true) => {
    if (now) listener(selector(store.get()), store.get())
    unbind.push(store.select(selector, listener))
  }

  bind(transport, memo(
    (s) => [shownBooks(s), shownIndex(s), s.playing, s.speed, s.grid],
    (books, index, playing, speed, grid) => ({
      index,
      count: books.length,
      at: books[index]?.currunix ?? null,
      first: books[0]?.currunix ?? null,
      last: books.at(-1)?.currunix ?? null,
      playing,
      speed,
      grid,
      isTick: Boolean(books[index]?.isTick),
    }),
  ))
  bind(bidLadder, memo((s) => [currentBook(s), s.symbol, s.global], (book, symbol, global) => ({ side: book?.bid ?? null, symbol, global })))
  bind(askLadder, memo((s) => [currentBook(s), s.symbol, s.global], (book, symbol, global) => ({ side: book?.ask ?? null, symbol, global })))
  bind(depth, memo((s) => [currentBook(s)], (book) => ({ bid: book?.bid ?? null, ask: book?.ask ?? null })))
  bind(metrics, memo((s) => [currentBook(s)], (book) => ({ book })))
  bind(tape, memo((s) => [executionsAt(shownBooks(s), shownIndex(s))], (executions) => ({ executions })))
  bind(symbolSelect, memo((s) => [s.symbols, s.symbol, s.global], (symbols, symbol, global) => ({ symbols, symbol, global })))
  bind(drawer, memo((s) => [s.scenarios, s.scenario], (scenarios, active) => ({ scenarios, active })))
  bind(diffView, memo((s) => [s.diff], (diff) => ({ rows: diff?.instants ?? [] })))
  if (insertForm) {
    bind(insertForm, memo((s) => [s.columns, s.kinds, currentBook(s)?.currunix ?? null], (columns, kinds, at) => ({ columns, kinds, at })))
  }
  bind(lifecycle, memo((s) => [s.lifecycle], (held) => held ?? { rows: [], crosscode: null }))
  watch((s) => s.lifecycle, (held) => {
    lifecycle.el.hidden = !held
    inspectorHint.hidden = Boolean(held)
  })

  // The price chart folds candles by a bucket fixed at construction: a new window of another span gets a new chart.
  watch((s) => shownBooks(s), (books) => {
    const bucketNs = bucketOf(books)
    if (priceChart?.props.bucketNs !== bucketNs) {
      priceChart?.destroy()
      priceChart = new PriceChart({ bucketNs, height: 220 }).mount(priceHolder)
    }
    priceChart.update({ books, window: { from: books[0]?.currunix ?? null, to: books.at(-1)?.currunix ?? null } })
  })

  watch((s) => s.sources, (sources) => {
    sourceSelect.replaceChildren(...sources.map((source) => new Option(`${source.name} (${source.operations} operations)`, source.id)))
    // One source is nothing to choose: its name is shown beside the title instead.
    sourceLabel.hidden = sources.length < 2
    sourceName.hidden = sources.length !== 1
    sourceName.textContent = sources.length === 1 ? sources[0].name : ''
  })
  watch((s) => s.source, (source) => {
    if (source !== null && sourceSelect.value !== source) sourceSelect.value = source
  })
  watch((s) => `${s.stream}\n${s.scenario ?? ''}`, () => {
    const s = store.get()
    const option = streamSelect.options[1]
    option.textContent = s.scenario ? `Scenario ${s.scenario}` : 'Scenario (none chosen)'
    option.disabled = !s.scenario
    // With no scenario there is one stream: nothing to choose.
    streamLabel.hidden = !s.scenario
    streamSelect.value = s.stream === 'scenario' && s.scenario ? 'scenario' : 'base'
  })
  watch((s) => s.snapshotMillis, (millis) => {
    if (gridInput.value !== String(millis)) gridInput.value = String(millis)
  })
  // Said once as a load starts and once as it ends: the text holds no count while the books arrive.
  watch(
    (s) => {
      if (s.loading) return `Loading the books of ${s.global ? `${GLOBAL} (consolidated)` : s.symbol}`
      const books = shownBooks(s)
      const stream = onScenario(s) ? `scenario ${s.scenario}` : 'base replay'
      return `${books.length} ${books.length === 1 ? 'book' : 'books'} of ${s.global ? `${GLOBAL} (consolidated)` : (s.symbol ?? 'no symbol')}, ${stream}`
    },
    (text) => {
      status.textContent = text
    },
  )
  watch((s) => s.viewAnswer, (answer) => renderView(answer))
  watch((s) => s.scenario, (scenario) => {
    diffHint.hidden = Boolean(scenario)
    diffView.el.hidden = !scenario
  })

  // ---- the views' table -----------------------------------------------------

  function renderView(answer) {
    viewRefusal.hidden = !answer?.refusal
    viewRefusal.textContent = answer?.refusal ?? ''
    if (!answer || answer.refusal || answer.loading || answer.hint) {
      viewStatus.textContent = answer?.hint ?? (answer?.loading ? 'Loading the view' : answer?.refusal ? '' : 'Choose a view')
      viewTable.replaceChildren()
      viewTable.hidden = true
      return
    }
    const { columns: served, rows: all } = answer
    const head = element('thead')
    const headRow = element('tr')
    for (const column of served) {
      const cell = element('th', undefined, column.name)
      cell.scope = 'col'
      cell.title = `${column.dtype}${column.nullable ? '' : ', not null'}`
      headRow.append(cell)
    }
    head.append(headRow)
    const body = element('tbody')
    for (const row of all.slice(0, VIEW_ROWS)) {
      const tr = element('tr')
      for (const column of served) tr.append(element('td', 'ygg-ui__mono', cellText(row[column.name])))
      body.append(tr)
    }
    viewTable.replaceChildren(head, body)
    viewTable.hidden = false
    const shown = all.length > VIEW_ROWS ? `, the first ${VIEW_ROWS} shown` : ''
    viewStatus.textContent = `${store.get().view}: ${all.length} ${all.length === 1 ? 'row' : 'rows'}${shown}`
  }

  // ---- telling the person --------------------------------------------------------

  /**
   * A refusal the service answered, verbatim: kept in the store, and told by
   * a toast - unless a panel of its own already tells it (`shown`), so it is
   * announced once.
   */
  function refuse(text, { shown = false } = {}) {
    store.set({ refusal: text })
    if (!shown) toasts.push({ text, kind: 'refusal' })
  }

  function inform(text) {
    toasts.push({ text, kind: 'info' })
  }

  // ---- position --------------------------------------------------------------------

  /** The patch that shows book `at` of the shown stream, the other stream standing at its instant. */
  function positionAt(state, at) {
    const books = shownBooks(state)
    const instant = books[at]?.currunix
    if (onScenario(state)) return { scenarioIndex: at, index: follow(state.books, instant) }
    return { index: at, scenarioIndex: follow(state.scenarioBooks, instant) }
  }

  function moveTo(index) {
    const state = store.get()
    const count = shownBooks(state).length
    if (!count || !Number.isFinite(index)) return
    store.set(positionAt(state, Math.min(count - 1, Math.max(0, Math.trunc(index)))))
  }

  function step(by) {
    moveTo(shownIndex(store.get()) + by)
  }

  /** The next or previous book the source stated, skipping grid ticks. */
  function stepSource(by) {
    const state = store.get()
    const books = shownBooks(state)
    for (let at = shownIndex(state) + by; at >= 0 && at < books.length; at += by) {
      if (!books[at].isTick) {
        moveTo(at)
        return
      }
    }
  }

  /** Both streams to the book standing at `at`. */
  function seek(at) {
    const state = store.get()
    try {
      parseInstant(at)
    } catch (error) {
      inform(error.message)
      return
    }
    store.set({ index: follow(state.books, at), scenarioIndex: follow(state.scenarioBooks, at) })
  }

  /** The last book at or before the instant typed. */
  function jump(text) {
    const state = store.get()
    const books = shownBooks(state)
    const at = indexAt(books, text)
    if (at < 0) {
      inform(books.length ? `No book stands at or before ${formatInstant(text)}; the first is at ${formatInstant(books[0].currunix)}` : 'No book is loaded')
      return
    }
    store.set(positionAt(state, at))
  }

  // ---- play --------------------------------------------------------------------------

  // Play shows each book for the wall-clock time its own instant gap takes at
  // the chosen speed, counted from the frame it is shown in, and never two
  // books in one frame: two books one nanosecond apart are two steps, and a
  // book a frame kept waiting still holds its whole gap. Each book may run
  // up to one frame long; none runs short.
  const clock = { frame: 0, books: null, index: -1, due: 0 }

  function gapMillis(books, index, speed) {
    const instants = instantsOf(books)
    if (index + 1 >= instants.length) return 0
    return Number(instants[index + 1] - instants[index]) / 1e6 / speed
  }

  function tick(now) {
    clock.frame = 0
    const state = store.get()
    if (!state.playing) return
    const books = shownBooks(state)
    const index = shownIndex(state)
    if (index >= books.length - 1) {
      store.set({ playing: false })
      return
    }
    if (clock.books !== books || clock.index !== index) {
      // Started, stepped by hand, or a new window: the gap is waited out from now.
      clock.books = books
      clock.index = index
      clock.due = now + gapMillis(books, index, state.speed)
    }
    if (now >= clock.due) {
      clock.index = index + 1
      clock.due = now + gapMillis(books, index + 1, state.speed)
      store.set(positionAt(state, index + 1))
    }
    clock.frame = requestAnimationFrame(tick)
  }

  watch((s) => s.playing, (playing) => {
    if (clock.frame) cancelAnimationFrame(clock.frame)
    clock.frame = 0
    clock.books = null
    if (playing) clock.frame = requestAnimationFrame(tick)
    // Every book changes the transport's instant, the ladders' and the tape's summaries and the best prices:
    // while playing, those regions are busy and announce nothing book by book.
    for (const region of [main, transportHolder]) {
      if (playing) region.setAttribute('aria-busy', 'true')
      else region.removeAttribute('aria-busy')
    }
  })

  function setPlaying(on) {
    const state = store.get()
    const books = shownBooks(state)
    if (on && !books.length) return
    // Play from the last book starts over.
    if (on && shownIndex(state) >= books.length - 1) store.set(positionAt(state, 0))
    store.set({ playing: Boolean(on) })
  }

  function setSpeed(speed) {
    if (typeof speed === 'number' && speed > 0) store.set({ speed })
  }

  function changeSpeed(by) {
    const held = store.get().speed
    let at = SPEEDS.indexOf(held)
    if (at < 0) at = SPEEDS.findIndex((speed) => speed > held) - (by > 0 ? 1 : 0)
    setSpeed(SPEEDS[Math.min(SPEEDS.length - 1, Math.max(0, at + by))])
  }

  // ---- loading the streams ---------------------------------------------------------------

  let baseLoad = null
  let scenarioLoad = null
  let viewSerial = 0
  let diffSerial = 0

  /** The base window under the walk the store names; the position kept by instant. */
  async function loadBooks() {
    baseLoad?.abort()
    // Another walk's scenario window and diff are never shown under this one's name: both are read again over it.
    scenarioLoad?.abort()
    scenarioLoad = null
    diffSerial += 1
    const state = store.get()
    if (!walkKey(state)) return
    const control = new AbortController()
    baseLoad = control
    const cursor = currentBook(state)?.currunix
    const books = []
    store.set({ loading: true, playing: false, scenarioBooks: null, scenarioIndex: 0, scenarioFrom: null, diff: null })
    let answer
    try {
      answer = await api.books(state.source, walkQuery(state), {
        signal: control.signal,
        onBook(book) {
          books.push(book)
        },
      })
    } catch (error) {
      if (control.signal.aborted) return
      store.set({ loading: false })
      throw error
    }
    if (baseLoad !== control) return
    baseLoad = null
    if (answer.refusal) {
      // The window stays the loaded walk's, and so do the controls that name a walk.
      const held = store.get().walk
      store.set({ loading: false, ...(held ? walkFields(held) : {}) })
      refuse(answer.refusal)
    } else {
      const loaded = Object.freeze(books)
      const walk = Object.freeze({ key: walkKey(state), ...walkFields(state) })
      store.set({ books: loaded, walk, index: follow(loaded, cursor), loading: false })
    }
    await readScenario()
  }

  /** The scenario's window starts with the base's books: it is read again over them, and so is its diff when that tab is open. */
  async function readScenario() {
    if (!store.get().scenario) return
    await loadScenarioBooks()
    if (store.get().tab === DIFF_TAB) await loadDiff()
  }

  /**
   * The active scenario's window: the service streams its books from the
   * earliest instant the scenario affects (`end.from`); before it every book
   * is the base's, so the window is the base's books walked before that
   * instant, then the ones streamed. It is read over the loaded base window
   * alone - a base load in flight reads it again when it lands - and an
   * answer for a walk or a scenario the page no longer names is dropped.
   */
  async function loadScenarioBooks() {
    scenarioLoad?.abort()
    scenarioLoad = null
    const state = store.get()
    if (!state.scenario || !walkKey(state)) {
      store.set({ scenarioBooks: null, scenarioIndex: 0 })
      return
    }
    if (baseLoad !== null) return
    const control = new AbortController()
    scenarioLoad = control
    const asked = scenarioKey(state)
    const streamed = []
    let answer
    try {
      answer = await api.scenarioBooks(state.source, state.scenario, walkQuery(state), {
        signal: control.signal,
        onBook(book) {
          streamed.push(book)
        },
      })
    } catch (error) {
      if (control.signal.aborted) return
      throw error
    }
    if (scenarioLoad !== control) return
    scenarioLoad = null
    if (baseLoad !== null || scenarioKey(store.get()) !== asked) return
    if (answer.refusal) {
      store.set({ scenarioBooks: null, scenarioIndex: 0, stream: 'base' })
      refuse(answer.refusal)
      return
    }
    const now = store.get()
    const from = answer.from === null || answer.from === undefined ? null : parseInstant(answer.from)
    let cut = 0
    if (from !== null) while (cut < now.books.length && walkInstant(now.books[cut]) < from) cut += 1
    const books = Object.freeze([...now.books.slice(0, cut), ...streamed])
    const cursor = currentBook(now)?.currunix
    store.set({ scenarioBooks: books, scenarioIndex: follow(books, cursor), scenarioFrom: answer.from ?? null })
  }

  let pendingBooks = null
  watch((s) => walkKey(s), (key) => {
    if (!key) return
    if (key !== store.get().walk?.key) {
      pendingBooks = run(loadBooks())
      return
    }
    // Back to the walk the window already is: a load in flight for another is dropped, and what it cleared is read again.
    if (baseLoad === null) return
    baseLoad.abort()
    baseLoad = null
    store.set({ loading: false })
    pendingBooks = run(readScenario())
  })

  // ---- views, lifecycle, diff ----------------------------------------------------------------

  /**
   * The chosen view over the loaded window's walk: a view is asked of a walk
   * whose books loaded, never of a refused one. The lifecycle view follows
   * the chain chosen in the ladder or the tape; with none chosen nothing is
   * asked.
   */
  async function loadView() {
    const state = store.get()
    if (!state.view || !state.walk) return
    const serial = ++viewSerial
    const crosscode = state.view === 'lifecycle' ? (state.selected?.crosscode ?? null) : undefined
    if (crosscode === null) {
      store.set({ viewAnswer: { hint: CHOOSE_CHAIN } })
      return
    }
    store.set({ viewAnswer: { loading: true } })
    const { source, global, grid, snapshotMillis } = state.walk
    const answer = await api.view(source, { view: state.view, lifts: state.lifts, global, snapshotMillis: grid ? snapshotMillis : 0, crosscode })
    if (serial !== viewSerial) return
    // The panel's alert tells it.
    if (answer.refusal) refuse(answer.refusal, { shown: true })
    store.set({ viewAnswer: answer })
  }

  watch(
    (s) =>
      s.tab?.startsWith(VIEW_TAB) && s.walk
        ? [s.walk.key, s.view, s.lifts.join('\n'), s.view === 'lifecycle' ? (s.selected?.crosscode ?? '') : ''].join('\u0000')
        : '',
    (key) => {
      if (key) run(loadView())
    },
  )

  async function selectElement({ crosscode, curruuid }) {
    const state = store.get()
    if (!crosscode || !state.source) return
    store.set({ selected: { crosscode, curruuid: curruuid ?? null } })
    if (state.lifecycle?.crosscode === crosscode) return
    const answer = await api.lifecycle(state.source, crosscode)
    if (store.get().selected?.crosscode !== crosscode) return
    if (answer.refusal) {
      refuse(answer.refusal)
      return
    }
    store.set({ lifecycle: { rows: answer.rows, crosscode } })
  }

  /** The active scenario's diff over the walk the page names; an answer for another walk or scenario is dropped. */
  async function loadDiff() {
    const state = store.get()
    if (!state.scenario || !walkKey(state)) {
      store.set({ diff: null })
      return
    }
    const serial = ++diffSerial
    const asked = scenarioKey(state)
    const answer = await api.diff(state.source, state.scenario, walkQuery(state))
    if (serial !== diffSerial || scenarioKey(store.get()) !== asked) return
    if (answer.refusal) {
      store.set({ diff: null })
      refuse(answer.refusal)
      return
    }
    store.set({ diff: answer })
  }

  function showDiff() {
    const state = store.get()
    if (!state.scenario) {
      inform('Choose or create a scenario first: the diff compares it with the base replay')
      drawer.open(scenariosButton)
      return
    }
    store.set({ tab: DIFF_TAB })
    run(loadDiff())
  }

  // ---- scenarios: a name is the service's to judge, and its refusal is shown verbatim --------

  async function refreshScenarios() {
    const answer = await api.scenarios()
    if (answer.refusal) {
      refuse(answer.refusal)
      return false
    }
    store.set({ scenarios: answer.scenarios })
    return true
  }

  /** After any change to a scenario: the list, the scenario stream and the diff read again. */
  async function scenarioChanged() {
    await refreshScenarios()
    const state = store.get()
    if (state.scenario && !state.scenarios.some((scenario) => scenario.name === state.scenario)) {
      store.set({ scenario: null, scenarioBooks: null, scenarioIndex: 0, stream: 'base', diff: null })
    }
    await loadScenarioBooks()
    if (store.get().tab === DIFF_TAB) await loadDiff()
    else store.set({ diff: null })
  }

  /** Show scenario `name`'s stream; another scenario's window and diff - held or in flight - are dropped, never shown under its name. */
  function followScenario(name) {
    const held = store.get().scenario === name
    if (!held) {
      scenarioLoad?.abort()
      scenarioLoad = null
      diffSerial += 1
    }
    store.set({ scenario: name, stream: 'scenario', ...(held ? {} : { scenarioBooks: null, scenarioIndex: 0, scenarioFrom: null, diff: null }) })
  }

  /** Follow scenario `name`: its events as the service holds them now, its stream, its diff when shown. */
  async function chooseScenario(name) {
    followScenario(name)
    await scenarioChanged()
  }

  const scenarioCommands = {
    async select({ name }) {
      await chooseScenario(name)
    },
    async create({ name }) {
      const answer = await api.saveScenario(name, { name, events: [] })
      if (answer.refusal) return refuse(answer.refusal)
      await chooseScenario(name)
    },
    async rename({ name, to }) {
      const held = store.get().scenarios.find((scenario) => scenario.name === name)
      if (!held) return refuse(`no scenario ${JSON.stringify(name)}`)
      // Saved under the new name first, so a refused save loses nothing.
      const saved = await api.saveScenario(to, { name: to, events: held.events })
      if (saved.refusal) return refuse(saved.refusal)
      const removed = await api.deleteScenario(name)
      if (removed.refusal) refuse(removed.refusal)
      if (store.get().scenario === name) store.set({ scenario: to })
      await scenarioChanged()
    },
    async delete({ name }) {
      const answer = await api.deleteScenario(name)
      if (answer.refusal) return refuse(answer.refusal)
      await scenarioChanged()
    },
    async 'remove-event'({ name, curruuid }) {
      const answer = await api.removeEvent(name, curruuid)
      if (answer.refusal) return refuse(answer.refusal)
      followScenario(name)
      await scenarioChanged()
    },
  }

  function scenarioCommand(detail) {
    if (readonly && detail.command !== 'select') {
      inform('Scenarios are read-only here: this page renders a recorded replay')
      return
    }
    const handler = scenarioCommands[detail.command]
    if (handler) run(handler(detail))
  }

  function openInsert(opener) {
    if (!readonly) {
      insertName.value = store.get().scenario ?? 'what-if'
      insertForm.reset()
    }
    insertModal.open(opener)
  }

  async function insert({ kind, currunix, facts }) {
    if (readonly) return
    const name = insertName.value.trim()
    const answer = await api.insert(name, { kind, currunix, facts })
    if (answer.refusal) {
      // The form tells it: beside the field it names, which takes focus, else on its own alert line.
      insertForm.showRefusal(answer.refusal)
      refuse(answer.refusal, { shown: true })
      return
    }
    insertForm.showRefusal(null)
    insertModal.close()
    inform(`Inserted ${answer.kind} at ${formatInstant(answer.currunix)} into ${name}`)
    followScenario(name)
    await scenarioChanged()
  }

  // ---- commands ----------------------------------------------------------------------

  function closeTop() {
    if (root.querySelector('dialog[open]')) return
    if (drawer.isOpen) drawer.close()
  }

  const commands = {
    'transport.toggle': (value) => setPlaying(value ?? !store.get().playing),
    'transport.forward': () => step(1),
    'transport.back': () => step(-1),
    'transport.forwardSource': () => stepSource(1),
    'transport.backSource': () => stepSource(-1),
    'transport.first': () => moveTo(0),
    'transport.last': () => moveTo(shownBooks(store.get()).length - 1),
    'transport.faster': () => changeSpeed(1),
    'transport.slower': () => changeSpeed(-1),
    'transport.grid': (value) => store.set({ grid: value ?? !store.get().grid }),
    'transport.jump': (value) => (value === undefined ? transport.focusJump() : jump(value)),
    'transport.seek': (value) => moveTo(value),
    'transport.speed': (value) => setSpeed(value),
    'scenario.insert': (_, opener) => openInsert(opener),
    'scenario.drawer': (_, opener) => drawer.toggle(opener),
    'scenario.diff': () => showDiff(),
    'theme.toggle': () => themeToggle.toggle(),
    'palette.open': (_, opener) => palette.open(opener),
    'help.shortcuts': (_, opener) => sheet.open(opener),
    'ui.close': () => closeTop(),
  }

  /** Run command `id` with its value, as a shortcut, a button or the palette asked. */
  function command(id, value, opener) {
    const handler = commands[id]
    if (!handler) return false
    handler(value, opener instanceof HTMLElement ? opener : document.activeElement)
    return true
  }

  /** Keys a focused control answers itself: Space presses a button, arrows move a slider. */
  const OWN_KEYS = 'button, a[href], summary, input, select, textarea, [role="button"], [role="tab"], [role="option"], [role="slider"], [role="separator"], [role="grid"]'

  listen(document, 'keydown', (event) => {
    if (event.defaultPrevented) return
    if (keys === 'app' && !root.contains(event.target)) return
    const id = commandFor(event)
    if (!id) return
    // A modal panel owns the keyboard; Escape is its own to close it with.
    if (root.querySelector('dialog[open]')) return
    if (id === 'transport.toggle' && event.target?.closest?.(OWN_KEYS)) return
    event.preventDefault()
    command(id, undefined, event.target)
  })

  // ---- what the components ask for -------------------------------------------------------

  listen(root, 'ygg:transport', (event) => command(event.detail.command, event.detail.value, event.target))
  listen(root, 'ygg:command', (event) => command(event.detail.id, undefined, document.activeElement))
  listen(root, 'ygg:seek', (event) => seek(event.detail.at))
  listen(root, 'ygg:select-element', (event) => run(selectElement(event.detail)))
  listen(root, 'ygg:limit-select', (event) => {
    // A limit names its entries by uuid; the first one's chain is read.
    const side = event.target.closest?.('.ygg-ui__ladder--ask') ? 'ask' : 'bid'
    const [curruuid] = event.detail.uuids ?? []
    const entry = (currentBook(store.get())?.[side]?.live ?? []).find((held) => held.curruuid === curruuid)
    if (entry) run(selectElement({ crosscode: entry.crosscode, curruuid }))
  })
  listen(root, 'ygg:symbol', (event) => {
    store.set({ symbol: event.detail.symbol, global: Boolean(event.detail.global) })
  })
  listen(root, 'ygg:tab', (event) => {
    const id = event.detail.id
    store.set({ tab: id, ...(id.startsWith(VIEW_TAB) ? { view: id.slice(VIEW_TAB.length) } : {}) })
    if (id === DIFF_TAB) run(loadDiff())
  })
  listen(root, 'ygg:theme', (event) => store.set({ theme: event.detail.theme }))
  listen(root, 'ygg:insert', (event) => run(insert(event.detail)))
  listen(root, 'ygg:scenario', (event) => scenarioCommand(event.detail))
  listen(root, 'ygg:resize', (event) => {
    const which = event.target === rows.el ? 'rows' : event.target === columns.el ? 'columns' : null
    if (which) store.set({ sizes: { ...store.get().sizes, [which]: event.detail.sizes } })
  })
  listen(actions, 'click', (event) => {
    const button = event.target.closest?.('button[data-command]')
    if (button && button.getAttribute('aria-disabled') !== 'true') command(button.dataset.command, undefined, button)
  })
  listen(sourceSelect, 'change', () => chooseSource(sourceSelect.value))
  listen(streamSelect, 'change', () => store.set({ stream: streamSelect.value }))
  listen(gridInput, 'change', () => {
    const millis = Number(gridInput.value)
    if (Number.isSafeInteger(millis) && millis > 0) store.set({ snapshotMillis: millis })
    else gridInput.value = String(store.get().snapshotMillis)
  })
  listen(liftsForm, 'submit', (event) => {
    event.preventDefault()
    const lifts = liftsInput.value.split('\n').map((line) => line.trim()).filter(Boolean)
    const held = store.get().lifts
    if (lifts.length === held.length && lifts.every((lift, at) => lift === held[at])) run(loadView())
    else store.set({ lifts })
  })

  // ---- start -------------------------------------------------------------------------------

  /** A pending call's failure: a network error or a bug, told rather than lost. */
  function run(promise) {
    promise.catch((error) => {
      store.set({ loading: false })
      toasts.push({ text: error?.message ?? String(error), kind: 'refusal' })
    })
    return promise
  }

  function chooseSource(id) {
    const source = store.get().sources.find((held) => held.id === id)
    if (!source) return
    const state = store.get()
    const wanted = state.symbol && source.symbols.includes(state.symbol) ? state.symbol : null
    const symbol = wanted ?? source.symbols.find((held) => held !== GLOBAL) ?? GLOBAL
    store.set({
      source: source.id,
      symbols: source.symbols,
      symbol,
      global: symbol === GLOBAL,
      lifecycle: null,
      selected: null,
      viewAnswer: null,
    })
  }

  async function start() {
    const [served, field, saved] = await Promise.all([api.sources(), api.field(), api.scenarios()])
    for (const answer of [served, field, saved]) if (answer.refusal) refuse(answer.refusal)
    const views = served.views ?? []
    tabs = new Tabs({
      tabs: [...views.map((view) => ({ id: `${VIEW_TAB}${view}`, label: view })), { id: DIFF_TAB, label: 'Scenario diff' }],
      label: 'Views and the scenario diff',
    }).mount(rows.panes[1])
    tabs.panel(DIFF_TAB).append(diffBody)
    watch((s) => s.tab, (tab) => {
      if (tab?.startsWith(VIEW_TAB)) tabs.panel(tab)?.append(viewBody)
    })
    bind(tabs, memo((s) => [s.tab], (tab) => ({ selected: tab })))

    const sources = served.sources ?? []
    const serviceMillis = served.snapshotMillis ?? 0
    const firstTab = views.length ? `${VIEW_TAB}${views[0]}` : DIFF_TAB
    store.set({
      sources,
      views,
      columns: field.columns ?? [],
      kinds: field.kinds ?? [],
      scenarios: saved.scenarios ?? [],
      theme: themeToggle.theme,
      tab: firstTab,
      view: views[0] ?? null,
      ...(serviceMillis > 0 ? { snapshotMillis: serviceMillis, grid: true } : {}),
      global: Boolean(served.global),
      symbol: served.global ? GLOBAL : params.get('symbol'),
    })
    const wanted = sources.find((source) => source.id === params.get('source')) ?? sources[0]
    if (!wanted) {
      inform('The service serves no source')
      return
    }
    // Choosing the source names a walk, and the walk's watch opens its stream.
    chooseSource(wanted.id)
    await pendingBooks
  }

  const app = {
    store,
    command,
    /** Every command id a handler answers: each one `shortcuts.js` names, and the two with a value. */
    commandIds: Object.freeze(Object.keys(commands)),
    ready: null,
    destroy() {
      baseLoad?.abort()
      scenarioLoad?.abort()
      if (clock.frame) cancelAnimationFrame(clock.frame)
      for (const stop of unbind.splice(0)) stop()
      for (const drop of forget.splice(0)) drop()
      for (const component of [transport, bidLadder, askLadder, depth, priceChart, metrics, tape, lifecycle, diffView, tabs, drawer, insertModal, palette, sheet, toasts, symbolSelect, themeToggle, rows, columns]) {
        component?.destroy()
      }
      root.replaceChildren()
      if (debug && globalThis.__store === store) delete globalThis.__store
    },
  }
  app.ready = run(start())
  if (debug) {
    document.documentElement.dataset.debug = ''
    globalThis.__store = store
    globalThis.__app = app
  }
  return app
}

// The served page mounts itself; a page importing this module mounts it where it wants.
const host = globalThis.document?.querySelector('[data-ygg-app]')
if (host) mountApp(host)
