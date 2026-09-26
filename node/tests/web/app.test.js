// `node/web/app/`: the replay application end to end, over the real service
// and the synthetic source, in one headless Chromium. Every flow reads the
// page (DOM text) and the store (`window.__store`, exposed by `?debug=1`),
// and compares what it shows with what the service and the native walk
// answer. Every test ends with the page's console empty.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { after, before, test } from 'node:test'

import { launch, WEB } from './browser.js'

const require = createRequire(import.meta.url)
const { createReplayServer } = require('../../replay/server.js')
const { loadSource } = require('../../replay/sources.js')
const { books: walked, T0 } = require('../../replay/synthetic.js')

const MS = 1_000_000n

let service
let base
let stateDir
let page

before(async () => {
  stateDir = mkdtempSync(path.join(os.tmpdir(), 'yggdryl-app-state-'))
  service = createReplayServer({ sources: [loadSource('synthetic')], webDir: WEB, stateDir })
  base = await service.listen(0)
  page = await launch()
})

after(async () => {
  try {
    await page?.close()
  } finally {
    await service?.close()
    rmSync(stateDir, { recursive: true, force: true })
  }
})

/** Open the app fresh at `route`, debug on, and wait for its first window. */
async function openApp(query = '', route = '/web/app/') {
  page.consoleLines.length = 0
  await page.open(new URL(`${route}?debug=1${query}`, base).href)
  await page.waitFor('window.__store && window.__app && __store.get().books.length > 0 && !__store.get().loading', 20_000)
  await page.frames()
}

/** A value of the store, read in the page: `s` is the state. */
function state(expression) {
  return page.evaluate(`(() => { const s = __store.get(); return (${expression}) })()`)
}

/** The text of the first element `selector` matches. */
function text(selector) {
  return page.evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`)
}

/** The served JSON of a route, read by Node. */
async function served(route) {
  const answer = await fetch(new URL(route, base))
  return answer.json()
}

/** The walk's hashes of `symbol`, as the native package answers them. */
function hashes(symbol, snapshotMillis = 0, global = false) {
  return walked(snapshotMillis, global)
    .filter((book) => book.crosscode === symbol)
    .map((book) => String(book.stableHash()))
}

/** Press a key as a person does, then let the page draw. */
async function press(key, options) {
  await page.press(key, options)
  await page.frames()
}

/** The console lines `pattern` names, taken out: an expected refusal logs its status. */
function takeLines(pattern) {
  const taken = page.consoleLines.filter((line) => pattern.test(line))
  const kept = page.consoleLines.filter((line) => !pattern.test(line))
  page.consoleLines.splice(0, page.consoleLines.length, ...kept)
  return taken
}

test('load: the first source and symbol, book 0 of the served window in every panel', async () => {
  // The URL the command line prints is the service's root, which answers with the page's own folder.
  await openApp('', '/')
  assert.equal(await page.evaluate('location.pathname + location.search'), '/web/app/?debug=1')
  assert.deepEqual(await state('[s.source, s.symbol, s.global, s.index, s.stream, s.books.length]'), ['synthetic', 'ALPHA', false, 0, 'base', 6])
  assert.deepEqual(await state('s.books.map((book) => book.stableHash)'), hashes('ALPHA'))
  const sources = await served('/api/sources')
  assert.deepEqual(await state('s.sources'), sources.sources)
  assert.deepEqual(await state('s.symbols'), ['ALPHA', 'BETA', 'GLOBAL'])
  const field = await served('/api/field')
  assert.deepEqual(await state('[s.columns, s.kinds]'), [field.columns, field.kinds])

  assert.equal(await text('.ygg-ui__transport-position'), '1 of 6')
  assert.equal(await text('.ygg-ui__transport-at'), '2023-11-14T22:13:20.000000000Z')
  assert.equal(await text('.ygg-ui__ladder--bid .ygg-ui__ladder-summary'), 'Bid: best 82 x 100, 3 limits')
  assert.equal(await text('.ygg-ui__ladder--ask .ygg-ui__ladder-summary'), 'Ask: best 82.5 x 100, 3 limits')
  assert.match(await page.evaluate("document.querySelector('.ygg-ui__depth canvas').getAttribute('aria-label')"), /^Depth/)
  assert.doesNotMatch(await page.evaluate("document.querySelector('.ygg-ui__depth canvas').getAttribute('aria-label')"), /no book/)
  assert.equal(await text('.ygg-ui__metrics-bbo'), 'Bid 82 x 100, ask 82.5 x 100')
  assert.equal(await text('.ygg-ui__tape-summary'), 'No executions')
  assert.match(await page.evaluate("document.querySelector('.ygg-ui__price canvas').getAttribute('aria-label')"), /^Prices from 2023-11-14T22:13:20\.000000000Z to 2023-11-14T22:13:20\.005000000Z: 6 books, 3 executions/)
  assert.equal(await text('.ygg-app__status'), '6 books of ALPHA, base replay')
  // At phone width the panes stack and nothing scrolls the page sideways.
  await page.viewport({ width: 390, height: 844 })
  try {
    await page.settle()
    assert.ok(await page.evaluate('document.documentElement.scrollWidth <= document.documentElement.clientWidth'))
    // The transport fits the page: its edges, range and edge stack rather than scroll sideways.
    assert.deepEqual(
      await page.evaluate("(() => { const transport = document.querySelector('.ygg-ui__transport'); return [transport.scrollWidth, transport.clientWidth, getComputedStyle(transport).overflowX] })()"),
      await page.evaluate("(() => { const transport = document.querySelector('.ygg-ui__transport'); return [transport.clientWidth, transport.clientWidth, 'visible'] })()"),
    )
    await page.screenshot('app-narrow')
  } finally {
    await page.viewport({ width: 1280, height: 800 })
  }
  // At 1180 px the book pane is too narrow for two ladders side by side: they stack, and no header runs into the next.
  await page.viewport({ width: 1180, height: 800 })
  try {
    await page.settle()
    const heads = await page.evaluate(`[...document.querySelectorAll('.ygg-ui__ladder-head')].map((head) => [...head.children].map((cell) => {
      const range = document.createRange()
      range.selectNodeContents(cell)
      const text = range.getBoundingClientRect()
      const box = cell.getBoundingClientRect()
      return { name: cell.textContent, left: text.left, right: text.right, fits: cell.scrollWidth <= cell.clientWidth && text.left >= box.left - 0.5 && text.right <= box.right + 0.5 }
    }))`)
    assert.equal(heads.length, 2)
    for (const cells of heads) {
      assert.equal(cells.length, 3)
      for (const cell of cells) assert.ok(cell.fits, `${cell.name} overflows its cell`)
      for (let at = 1; at < cells.length; at += 1) assert.ok(cells[at - 1].right <= cells[at].left, `${cells[at - 1].name} runs into ${cells[at].name}`)
    }
    await page.screenshot('app-1180')
  } finally {
    await page.viewport({ width: 1280, height: 800 })
  }
  assert.deepEqual(page.consoleLines, [])
})

test('transport: steps, play by the books\' own gaps, pause, scrub, jump, speed, grid; one nanosecond is one step', async (t) => {
  await openApp('&snapshotMillis=2')
  const at = () => text('.ygg-ui__transport-at')
  // ALPHA-O-1 and ALPHA-O-2 stand one nanosecond apart: two books, two steps.
  await press('ArrowRight')
  assert.deepEqual(await state('[s.index, s.books[s.index].currunix]'), [1, String(T0 + MS)])
  assert.equal(await at(), '2023-11-14T22:13:20.001000000Z')
  const first = await state('s.books[s.index].stableHash')
  await press('ArrowRight')
  assert.deepEqual(await state('[s.index, s.books[s.index].currunix]'), [2, String(T0 + MS + 1n)])
  await page.frames()
  assert.equal(await at(), '2023-11-14T22:13:20.001000001Z')
  assert.notEqual(await state('s.books[s.index].stableHash'), first)
  await press('ArrowLeft')
  assert.equal(await state('s.index'), 1)
  await press('End')
  assert.equal(await state('s.index'), 5)
  await press('Home')
  assert.equal(await state('s.index'), 0)

  // Play: every book in turn, each in a frame of its own, until the last.
  await page.evaluate(`(() => {
    window.frame = 0
    const count = () => { frame += 1; requestAnimationFrame(count) }
    requestAnimationFrame(count)
    window.visited = []
    // What a screen reader is told while the books change: the regions they change in are busy.
    const busy = () => ['.ygg-app__main', '.ygg-app__transport'].map((selector) => document.querySelector(selector).getAttribute('aria-busy'))
    window.busyWhilePlaying = new Set()
    __store.subscribe((next, previous) => {
      if (next.index !== previous.index) {
        visited.push([next.index, frame])
        busyWhilePlaying.add(busy().join())
      }
    })
    window.busy = busy
  })()`)
  assert.deepEqual(await page.evaluate('busy()'), [null, null])
  await press(' ')
  assert.equal(await state('s.playing'), true)
  await page.waitFor('!__store.get().playing')
  const visited = await page.evaluate('visited')
  assert.deepEqual(visited.map(([index]) => index), [1, 2, 3, 4, 5])
  assert.deepEqual(await page.evaluate('[...busyWhilePlaying]'), ['true,true'])
  assert.deepEqual(await page.evaluate('busy()'), [null, null])
  assert.equal(await page.evaluate("document.querySelector('.ygg-app__transport > .ygg-ui__transport') !== null"), true)
  for (let at = 1; at < visited.length; at += 1) assert.ok(visited[at][1] > visited[at - 1][1], `book ${visited[at][0]} shares a frame with the one before`)
  assert.equal(await text('.ygg-ui__transport-position'), '6 of 6')
  // Pause holds the book it stands on.
  await press('Home')
  await press(' ')
  await press(' ')
  assert.equal(await state('s.playing'), false)
  const held = await state('s.index')
  await page.frames(6)
  assert.equal(await state('s.index'), held)

  // The scrubber seeks by index.
  await page.evaluate("(() => { const range = document.querySelector('.ygg-ui__transport-range'); range.value = '3'; range.dispatchEvent(new Event('input', { bubbles: true })) })()")
  assert.equal(await state('s.index'), 3)
  // Jump: `j` puts the caret in the field; the last book at or before the instant stands.
  await press('j')
  assert.equal(await page.evaluate("document.activeElement === document.querySelector('.ygg-ui__transport-input')"), true)
  await page.type(String(T0 + 2n * MS))
  await press('Enter')
  assert.deepEqual(await state('[s.index, s.books[s.index].currunix]'), [2, String(T0 + MS + 1n)])
  await page.evaluate('document.activeElement.blur()')
  // Speed through the chords and the select.
  await press(']')
  assert.equal(await state('s.speed'), 2)
  await press('[')
  await press('[')
  assert.equal(await state('s.speed'), 0.5)
  await page.evaluate("(() => { const speed = document.querySelector('.ygg-ui__transport-speed'); speed.value = '8'; speed.dispatchEvent(new Event('change', { bubbles: true })) })()")
  assert.equal(await state('s.speed'), 8)

  // A jump compares instants to the nanosecond: the pair one nanosecond apart is two books.
  await page.evaluate(`__app.command('transport.jump', '${T0 + MS}')`)
  assert.equal(await state('s.index'), 1)
  await page.evaluate(`__app.command('transport.jump', '${T0 + MS + 1n}')`)
  assert.equal(await state('s.index'), 2)

  // Play holds each book its gap divided by the speed: at 0.01 the 2 ms gap is 200 ms of wall clock,
  // while the pair one nanosecond apart still takes the next frame, never the same one. Each book is
  // timed by its frame's own time, the one every animation callback of that frame reads.
  await page.evaluate("__app.command('transport.speed', 0.01)")
  assert.equal(await state('s.speed'), 0.01)
  await press('Home')
  await page.evaluate(`(() => {
    window.timed = []
    __store.subscribe((next, previous) => { if (next.index !== previous.index && next.playing) timed.push([next.index, frame, document.timeline.currentTime]) })
  })()`)
  await page.evaluate("__app.command('transport.toggle', true)")
  await page.waitFor('!__store.get().playing', 5_000)
  const timed = await page.evaluate('timed')
  assert.deepEqual(timed.map(([index]) => index), [1, 2, 3, 4, 5])
  const [[, oneFrame, oneAt], [, pairFrame, pairAt], [, gapFrame, gapAt]] = timed
  assert.equal(pairFrame, oneFrame + 1, 'the book one nanosecond on is the next frame')
  // The pair's second book, kept a frame by the first, still holds its whole gap: 2 ms less 1 ns at 0.01x.
  assert.ok(gapAt - pairAt >= 199.9, `the book after the 2 ms gap came ${gapAt - pairAt} ms after the one before it`)
  assert.ok(gapFrame > pairFrame + 1)
  t.diagnostic(`at 0.01x, by frame time: the pair ${pairFrame - oneFrame} frame apart, ${(pairAt - oneAt).toFixed(1)} ms; the book after the 2 ms gap ${(gapAt - pairAt).toFixed(1)} ms after the one before it`)
  await page.evaluate("__app.command('transport.speed', 8)")
  await page.evaluate(`__app.command('transport.jump', '${T0 + 2n * MS}')`)

  // The grid re-opens the stream at the width the page names, keeping the instant; the status says so twice - loading, then loaded.
  await page.evaluate(`(() => {
    const status = document.querySelector('.ygg-app__status')
    window.told = []
    new MutationObserver(() => told.push(status.textContent)).observe(status, { childList: true, characterData: true, subtree: true })
  })()`)
  await press('g')
  await page.waitFor('__store.get().grid && !__store.get().loading && __store.get().books.length === 9')
  await page.frames()
  assert.deepEqual(await page.evaluate('told'), ['Loading the books of ALPHA', '9 books of ALPHA, base replay'])
  assert.deepEqual(await state('s.books.map((book) => book.stableHash)'), hashes('ALPHA', 2))
  assert.deepEqual(await state('[s.index, s.books[s.index].currunix]'), [2, String(T0 + MS + 1n)])
  assert.deepEqual(await state('s.books.filter((book) => book.isTick).map((book) => book.currunix)'), [2n, 4n, 6n].map((ms) => String(T0 + ms * MS)))
  // A step lands on a tick; a source step skips it.
  await press('ArrowRight')
  await page.frames()
  assert.equal(await state('s.books[s.index].isTick'), true)
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__transport-tick').hidden"), false)
  await press('ArrowLeft')
  await press('ArrowRight', { modifiers: 8 })
  assert.deepEqual(await state('[s.books[s.index].currunix, s.books[s.index].isTick]'), [String(T0 + 3n * MS), false])
  await press('ArrowRight', { modifiers: 8 })
  assert.equal(await state('s.books[s.index].currunix'), String(T0 + 4n * MS + 500_000n))
  await press('ArrowLeft', { modifiers: 8 })
  assert.equal(await state('s.books[s.index].currunix'), String(T0 + 3n * MS))
  await press('g')
  await page.waitFor('!__store.get().grid && !__store.get().loading && __store.get().books.length === 6')
  assert.equal(await state('s.books[s.index].currunix'), String(T0 + 3n * MS))
  assert.deepEqual(page.consoleLines, [])
})

test('selection: a print or a limit loads its lifecycle into the inspector', async () => {
  await openApp()
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__lifecycle').closest('[hidden]') !== null"), true)
  await press('End')
  // The tape: every execution of the window's books up to the one shown, in walk order, reversed.
  const tape = await state('s.books.flatMap((book) => book.executions).map((execution) => execution.curruuid).reverse()')
  assert.equal(tape.length, 3)
  const shown = await page.evaluate("[...document.querySelectorAll('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row .ygg-ui__tape-id')].map((cell) => cell.title)")
  assert.deepEqual(shown, tape)
  // Read once per window: a step that prints nothing hands the tape the list it already has.
  const lists = await page.evaluate(`(async () => {
    const { executionsAt } = await import('/web/app/app.js')
    const books = __store.get().books
    const lists = books.map((_, index) => executionsAt(books, index))
    return { ids: lists.map((list) => list.map((execution) => execution.curruuid)), same: lists.slice(1).map((list, at) => list === lists[at]) }
  })()`)
  assert.deepEqual(lists.ids.at(-1), tape)
  assert.deepEqual(lists.same, lists.ids.slice(1).map((ids, at) => ids.length === lists.ids[at].length))
  assert.ok(lists.same.includes(true))
  // One book back, the prints of the last book have not happened yet.
  await press('ArrowLeft')
  assert.deepEqual(
    await page.evaluate("[...document.querySelectorAll('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row .ygg-ui__tape-id')].map((cell) => cell.title)"),
    tape.slice(2),
  )
  await press('End')
  // The oldest print, a fill stated alone, has a chain of its own; a trade's fills live in the trade.
  await page.click('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row:nth-child(3)')
  await page.waitFor('__store.get().lifecycle')
  const crosscode = await state('s.selected.crosscode')
  assert.equal(crosscode, 'ALPHA-E-1')
  const chain = await served(`/api/sources/synthetic/lifecycle?crosscode=${encodeURIComponent(crosscode)}`)
  assert.deepEqual(await state('s.lifecycle'), { rows: chain.rows, crosscode })
  await page.frames()
  assert.equal(await text('.ygg-ui__lifecycle-title'), crosscode)
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__lifecycle').closest('[hidden]') === null"), true)
  assert.equal(chain.rows.length, 1)
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-ui__lifecycle-chain tbody tr').length"), chain.rows.length)

  // A limit: the chain of its first entry, found among the side's live entries by uuid.
  await page.click('.ygg-ui__ladder--ask .ygg-ui__virtual-layer .ygg-ui__ladder-row')
  const entry = await state('s.books[s.index].ask.live.find((held) => held.curruuid === s.books[s.index].ask.limits[0].uuids[0])')
  await page.waitFor(`__store.get().lifecycle?.crosscode === ${JSON.stringify(entry.crosscode)}`)
  const limitChain = await served(`/api/sources/synthetic/lifecycle?crosscode=${encodeURIComponent(entry.crosscode)}`)
  assert.deepEqual(await state('s.lifecycle.rows'), limitChain.rows)
  assert.deepEqual(await state('s.selected'), { crosscode: entry.crosscode, curruuid: entry.curruuid })
  await page.frames()
  assert.equal(await text('.ygg-ui__lifecycle-title'), entry.crosscode)
  assert.deepEqual(page.consoleLines, [])
})

test('views: a tab per market view, each the served columns and rows, lifts as typed, refusals verbatim', async () => {
  await openApp()
  const { enums } = require('../../binding.js')
  assert.deepEqual((await served('/api/sources')).views, [...enums.marketViews])
  const tabs = await page.evaluate("[...document.querySelectorAll('.ygg-ui__tab')].map((tab) => [tab.textContent, tab.getAttribute('aria-selected')])")
  assert.deepEqual(tabs, [...enums.marketViews.map((view, at) => [view, String(at === 0)]), ['Scenario diff', 'false']])
  await page.waitFor('__store.get().viewAnswer?.rows')
  await page.frames()
  const headers = () => page.evaluate("[...document.querySelectorAll('.ygg-app__view-table thead th')].map((cell) => cell.textContent)")
  const orders = await served('/api/sources/synthetic/view?view=orders&global=false&snapshotMillis=0')
  assert.deepEqual(await headers(), orders.columns.map((column) => column.name))
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-app__view-table tbody tr').length"), orders.rows.length)
  assert.equal(await text('.ygg-app__view-status'), `orders: ${orders.rows.length} rows`)

  await page.click('.ygg-ui__tab[data-id="view:book_sides"]')
  await page.waitFor("__store.get().view === 'book_sides' && __store.get().viewAnswer?.rows")
  await page.frames()
  const sides = await served('/api/sources/synthetic/view?view=book_sides&global=false&snapshotMillis=0')
  assert.deepEqual(await headers(), sides.columns.map((column) => column.name))
  assert.deepEqual(await state('s.viewAnswer'), sides)
  assert.equal(await page.evaluate("document.querySelector('[role=\"tabpanel\"]:not([hidden]) .ygg-app__view-table') !== null"), true)

  // The lifecycle view follows one chain: with none chosen nothing is asked, and the panel says how to choose one.
  const lifecycleAsked = () => page.evaluate("performance.getEntriesByType('resource').filter((entry) => entry.name.includes('view=lifecycle')).map((entry) => new URL(entry.name).searchParams.get('crosscode'))")
  await page.click('.ygg-ui__tab[data-id="view:lifecycle"]')
  await page.waitFor("__store.get().view === 'lifecycle'")
  await page.frames()
  assert.equal(await text('.ygg-app__view-status'), 'Choose a limit in the ladder or a print in the tape to read its lifecycle.')
  assert.equal(await page.evaluate("document.querySelector('.ygg-app__view-refusal').hidden"), true)
  assert.equal(await state('s.refusal'), null)
  assert.deepEqual(await lifecycleAsked(), [])
  // A print chosen in the tape: its chain, as the view route answers it for that cross code over the walk shown.
  await page.evaluate("__app.command('transport.last')")
  await page.frames()
  await page.click('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row:nth-child(3)')
  await page.waitFor("__store.get().selected?.crosscode === 'ALPHA-E-1' && __store.get().viewAnswer?.rows")
  await page.frames()
  const chain = await served('/api/sources/synthetic/view?view=lifecycle&crosscode=ALPHA-E-1&global=false&snapshotMillis=0')
  assert.equal(chain.rows.length, 1)
  assert.deepEqual(await state('s.viewAnswer'), chain)
  assert.deepEqual(await lifecycleAsked(), ['ALPHA-E-1'])
  assert.equal(await text('.ygg-app__view-status'), 'lifecycle: 1 row')

  // The lifts the person typed, one per line, as the plan names them.
  await page.click('.ygg-ui__tab[data-id="view:orders"]')
  await page.waitFor("__store.get().view === 'orders' && __store.get().viewAnswer?.rows")
  const lift = "securityids['ISIN'] as isin"
  await page.click('.ygg-app__lifts-input')
  await page.type(lift)
  await page.click('.ygg-app__lifts button[type="submit"]')
  await page.waitFor("__store.get().viewAnswer?.columns?.at(-1)?.name === 'isin'")
  await page.frames()
  const lifted = await served(`/api/sources/synthetic/view?view=orders&lift=${encodeURIComponent(lift)}`)
  assert.deepEqual(await state('s.viewAnswer'), lifted)
  assert.deepEqual(await headers(), lifted.columns.map((column) => column.name))

  // A lift the plan cannot read: the native refusal, verbatim, in the panel and a toast.
  await page.evaluate("document.querySelector('.ygg-app__lifts-input').value = ''")
  await page.click('.ygg-app__lifts-input')
  await page.type("nosuch['X'] as y")
  await page.click('.ygg-app__lifts button[type="submit"]')
  await page.waitFor('__store.get().viewAnswer?.refusal')
  await page.frames()
  const refused = await fetch(new URL(`/api/sources/synthetic/view?view=orders&lift=${encodeURIComponent("nosuch['X'] as y")}`, base))
  assert.equal(refused.status, 400)
  const { error } = await refused.json()
  assert.equal(await state('s.refusal'), error)
  assert.equal(await text('.ygg-app__view-refusal'), error)
  // Told once, by the panel's own alert: no toast repeats it.
  assert.equal(await page.evaluate("document.querySelector('.ygg-app__view-refusal').getAttribute('role')"), 'alert')
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-ui__toast--refusal').length"), 0)
  assert.equal(takeLines(/status of 400 .*\/api\/sources\/synthetic\/view\?/).length, 1)
  assert.deepEqual(page.consoleLines, [])
})

test('symbol and mode: GLOBAL (consolidated) walks the consolidated book; its ladder groups entries by ticker', async () => {
  await openApp()
  const options = await page.evaluate("[...document.querySelectorAll('.ygg-ui__symbol-select option')].map((option) => [option.value, option.textContent])")
  assert.deepEqual(options, [['ALPHA', 'ALPHA'], ['BETA', 'BETA'], ['GLOBAL', 'GLOBAL (consolidated)']])
  const choose = (symbol) =>
    page.evaluate(`(() => { const select = document.querySelector('.ygg-ui__symbol-select'); select.value = ${JSON.stringify(symbol)}; select.dispatchEvent(new Event('change', { bubbles: true })) })()`)
  await choose('GLOBAL')
  await page.waitFor('__store.get().global && !__store.get().loading && __store.get().books.length === 9')
  assert.deepEqual(await state('[s.symbol, s.global]'), ['GLOBAL', true])
  assert.deepEqual(await state('s.books.map((book) => book.stableHash)'), hashes('GLOBAL', 0, true))
  assert.ok(await state("s.books.every((book) => book.crosscode === 'GLOBAL')"))
  await page.frames()
  assert.equal(await text('.ygg-app__status'), '9 books of GLOBAL (consolidated), base replay')
  // The tooltip names each entry's own ticker.
  await page.hover('.ygg-ui__ladder--bid .ygg-ui__virtual-layer .ygg-ui__ladder-row')
  await page.waitFor("document.querySelector('.ygg-ui__tooltip:not([hidden]) .ygg-ui__tooltip-key')")
  assert.equal(await text('.ygg-ui__tooltip:not([hidden]) .ygg-ui__tooltip-key'), 'ALPHA')
  await page.mouse('mouseMoved', 5, 5)
  await choose('BETA')
  await page.waitFor("!__store.get().global && __store.get().symbol === 'BETA' && !__store.get().loading && __store.get().books[0]?.crosscode === 'BETA'")
  assert.deepEqual(await state('s.books.map((book) => book.stableHash)'), hashes('BETA'))
  assert.deepEqual(page.consoleLines, [])
})

/** The books a served event stream holds, read by Node. */
async function streamed(route) {
  const answer = await fetch(new URL(route, base))
  assert.equal(answer.status, 200)
  const events = (await answer.text()).split('\n\n').filter(Boolean).map((block) => {
    const lines = block.split('\n')
    return { event: lines[0].slice('event: '.length), data: JSON.parse(lines[1].slice('data: '.length)) }
  })
  return { books: events.filter((item) => item.event === 'book').map((item) => item.data), end: events.at(-1).data }
}

/** Set a control's value as typing would, and say so. */
function fill(selector, value) {
  return page.evaluate(`(() => {
    const control = document.querySelector(${JSON.stringify(selector)})
    control.value = ${JSON.stringify(value)}
    control.dispatchEvent(new Event(control.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }))
  })()`)
}

test('insert: the served columns and kinds, a refusal verbatim beside its field, then the scenario stream', async () => {
  const { leafFromJson } = require('../../replay/json.js')
  await openApp()
  for (let step = 0; step < 3; step += 1) await press('ArrowRight')
  assert.equal(await state('s.books[s.index].currunix'), String(T0 + 3n * MS))
  await press('i')
  await page.waitFor("document.querySelector('.ygg-ui__modal[open] .ygg-ui__insert')")
  const field = await served('/api/field')
  assert.deepEqual(await page.evaluate("[...document.querySelectorAll('.ygg-ui__insert-kind option')].map((option) => option.value)"), field.kinds)
  assert.deepEqual(
    await page.evaluate("[...document.querySelectorAll('.ygg-ui__insert-column option')].map((option) => option.value)"),
    field.columns.map((column) => column.name).filter((name) => name !== 'kind' && name !== 'currunix'),
  )
  // The instant starts at the book shown; the scenario at a name of the app's.
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__insert-at').value"), String(T0 + 3n * MS))
  assert.equal(await page.evaluate("document.querySelector('.ygg-app__scenario-name').value"), 'what-if')
  await fill('.ygg-app__scenario-name', 'ins-flow')
  const at = T0 + 3n * MS + 500_000n
  await fill('.ygg-ui__insert-at', String(at))
  const facts = { crosscode: 'ALPHA-X-1', ticker: 'ALPHA', side: 'BUY', price: 'abc', quantity: '5' }
  for (const [name, value] of Object.entries(facts)) {
    await fill('.ygg-ui__insert-column', name)
    await page.click('.ygg-ui__insert-add')
    await page.type(value)
  }
  await page.click('.ygg-ui__insert-submit')
  await page.waitFor('__store.get().refusal')
  await page.frames()
  let expected
  try {
    leafFromJson({ kind: 'order_event', currunix: String(at), facts })
  } catch (error) {
    expected = error.message
  }
  assert.ok(expected)
  assert.equal(await state('s.refusal'), expected)
  assert.match(expected, /\$\.price/)
  assert.equal(await text('[data-fact="price"] .ygg-ui__field-error'), expected)
  assert.equal(await page.evaluate("document.querySelector('[data-fact=\"price\"] input').getAttribute('aria-invalid')"), 'true')
  // Told once: the field it names takes focus, read with the refusal that describes it; no toast repeats it.
  assert.equal(await page.evaluate("document.activeElement === document.querySelector('[data-fact=\"price\"] input')"), true)
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-ui__toast--refusal').length"), 0)
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__modal').open"), true)
  assert.equal(takeLines(/status of 400 .*\/api\/scenarios\/ins-flow\/events/).length, 1)

  // A scenario name the service refuses: its own sentence, on the form's line.
  await fill('[data-fact="price"] input', '82.25')
  await fill('.ygg-app__scenario-name', 'Ins Flow')
  await page.evaluate('__store.set({ refusal: null })')
  await page.click('.ygg-ui__insert-submit')
  await page.waitFor('__store.get().refusal')
  await page.frames()
  const badName = await fetch(new URL('/api/scenarios/Ins%20Flow/events', base), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'order_event', currunix: String(at), facts: { ...facts, price: '82.25' } }),
  })
  assert.equal(badName.status, 400)
  const { error: nameRefusal } = await badName.json()
  assert.match(nameRefusal, /got "Ins Flow"$/)
  assert.equal(await state('s.refusal'), nameRefusal)
  assert.equal(await text('.ygg-ui__insert-refusal'), nameRefusal)
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__insert-refusal').getAttribute('role')"), 'alert')
  assert.equal(await text('[data-fact="price"] .ygg-ui__field-error'), '')
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-ui__toast--refusal').length"), 0)
  assert.equal(takeLines(/status of 400 \(Bad Request\) http:\/\/127\.0\.0\.1:\d+\/api\/scenarios\/Ins%20Flow\/events$/).length, 1)

  // Stated right, the event is stored and the page follows the scenario stream from the served `from`.
  await fill('.ygg-app__scenario-name', 'ins-flow')
  await page.click('.ygg-ui__insert-submit')
  await page.waitFor("__store.get().scenario === 'ins-flow' && __store.get().stream === 'scenario' && __store.get().scenarioBooks?.length === 7")
  await page.frames()
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__modal').open"), false)
  assert.equal(await state('s.scenarioFrom'), String(at))
  const service = await streamed(`/api/sources/synthetic/scenarios/ins-flow/books?symbol=ALPHA&from=0`)
  assert.deepEqual(await state('s.scenarioBooks.map((book) => book.stableHash)'), service.books.map((book) => book.stableHash))
  const baseHashes = hashes('ALPHA')
  assert.deepEqual(await state(`s.scenarioBooks.filter((book) => BigInt(book.currunix) < ${at}n).map((book) => book.stableHash)`), baseHashes.slice(0, 4))
  assert.ok(await state(`s.scenarioBooks.filter((book) => BigInt(book.currunix) >= ${at}n).every((book) => book.bid.limits.some((limit) => limit.price === '82.25'))`))
  assert.deepEqual(await state('[s.index, s.scenarioIndex]'), [3, 3])
  assert.equal(await text('.ygg-ui__transport-position'), '4 of 7')
  assert.equal(await text('.ygg-app__status'), '7 books of ALPHA, scenario ins-flow')
  assert.equal(await page.evaluate("document.querySelector('.ygg-app__stream').value"), 'scenario')
  assert.match(await text('.ygg-ui__toast--info .ygg-ui__toast-text'), /^Inserted order_event at 2023-11-14T22:13:20\.003500000Z into ins-flow$/)
  assert.deepEqual((await served('/api/scenarios')).scenarios.map((scenario) => [scenario.name, scenario.events.length]), [['ins-flow', 1]])
  // The base stream is one choice away, standing at the same instant.
  await fill('.ygg-app__stream', 'base')
  await page.frames()
  assert.deepEqual(await state('[s.stream, s.books[s.index].currunix]'), ['base', String(T0 + 3n * MS)])
  assert.equal(await text('.ygg-ui__transport-position'), '4 of 6')
  // A second insert starts again at the book shown, not at the instant typed for the first.
  await press('End')
  await press('i')
  await page.waitFor("document.querySelector('.ygg-ui__modal[open] .ygg-ui__insert')")
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__insert-at').value"), String(T0 + 5n * MS))
  await press('Escape')
  await page.waitFor("!document.querySelector('.ygg-ui__modal[open]')")
  assert.equal((await fetch(new URL('/api/scenarios/ins-flow', base), { method: 'DELETE' })).status, 204)
  assert.deepEqual(page.consoleLines, [])
})

test('scenarios: create (a bad name refused by the service, verbatim), select, diff and seek, rename, remove an event - the base restored exactly - and delete', async () => {
  await openApp()
  const baseHashes = hashes('ALPHA')
  await press('s')
  await page.waitFor("!document.querySelector('.ygg-ui__scenarios').hidden")
  // A name the service refuses: the page sends it, and shows the service's own sentence.
  await fill('.ygg-ui__scenario-new input', 'Bad Name')
  await page.click('.ygg-ui__scenario-new button[type="submit"]')
  await page.waitFor('__store.get().refusal')
  const badName = await fetch(new URL('/api/scenarios/Bad%20Name', base), {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'Bad Name', events: [] }),
  })
  assert.equal(badName.status, 400)
  const { error: nameRefusal } = await badName.json()
  assert.match(nameRefusal, /got "Bad Name"$/)
  assert.equal(await state('s.refusal'), nameRefusal)
  assert.equal(await text('.ygg-ui__toast--refusal .ygg-ui__toast-text'), nameRefusal)
  assert.equal(takeLines(/status of 400 \(Bad Request\) http:\/\/127\.0\.0\.1:\d+\/api\/scenarios\/Bad%20Name$/).length, 1)
  assert.deepEqual((await served('/api/scenarios')).scenarios, [])

  await fill('.ygg-ui__scenario-new input', 'drawer-flow')
  await page.click('.ygg-ui__scenario-new button[type="submit"]')
  await page.waitFor("__store.get().scenario === 'drawer-flow' && __store.get().scenarioBooks !== null")
  assert.deepEqual((await served('/api/scenarios')).scenarios, [{ name: 'drawer-flow', events: [] }])
  assert.deepEqual(await state('s.scenarioBooks.map((book) => book.stableHash)'), baseHashes)
  await page.frames()
  assert.equal(await text('.ygg-ui__scenario-name[aria-current="true"]'), 'drawer-flow (0 events)')

  // An event stored by another client: choosing the scenario reads it again.
  const at = T0 + 3n * MS + 500_000n
  const posted = await fetch(new URL('/api/scenarios/drawer-flow/events', base), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'order_event', currunix: String(at), facts: { crosscode: 'ALPHA-D-1', ticker: 'ALPHA', side: 'BUY', price: '82.25', quantity: '5' } }),
  })
  assert.equal(posted.status, 201)
  await page.click('.ygg-ui__scenario-name')
  await page.waitFor("__store.get().scenarios[0]?.events.length === 1 && __store.get().scenarioBooks?.length === 7")
  assert.equal(await state('s.scenarioFrom'), String(at))

  // The diff: the served instants; a row seeks both streams to its instant.
  await press('Escape')
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__scenarios').hidden"), true)
  await press('d')
  await page.waitFor("__store.get().tab === 'diff' && __store.get().diff")
  await page.frames()
  const diff = await served('/api/sources/synthetic/scenarios/drawer-flow/diff?symbol=ALPHA&global=false&snapshotMillis=0')
  assert.deepEqual(await state('s.diff'), diff)
  const differing = diff.instants.filter((row) => !row.changes.same)
  assert.equal(await text('.ygg-ui__diff-summary'), `${differing.length} of ${diff.instants.length} instants differ`)
  assert.equal(differing[0].at, String(at))
  assert.equal(differing[0].base, null)
  // The second differing instant's changes, keyed as diff.js keyed them, open for the screenshot.
  await page.evaluate("document.querySelectorAll('.ygg-ui__diff-expand')[1].click()")
  await page.evaluate("document.querySelectorAll('.ygg-ui__toast-dismiss').forEach((button) => button.click())")
  await page.viewport({ width: 1280, height: 1100 })
  await page.settle()
  await page.evaluate("document.querySelector('.ygg-app__diff').scrollIntoView({ block: 'start' })")
  await page.screenshot('app-diff')
  await page.viewport({ width: 1280, height: 800 })
  await page.click('.ygg-ui__diff-seek')
  await page.frames()
  assert.deepEqual(await state('[s.scenarioBooks[s.scenarioIndex].currunix, s.books[s.index].currunix]'), [String(at), String(T0 + 3n * MS)])
  assert.equal(await text('.ygg-ui__transport-at'), '2023-11-14T22:13:20.003500000Z')

  // Rename: saved under the new name, the old one removed, still the one followed.
  await press('s')
  await page.click('.ygg-ui__scenario-item [data-action="rename"]')
  await page.waitFor("document.querySelector('.ygg-ui__scenario-rename input')")
  await fill('.ygg-ui__scenario-rename input', 'drawer-renamed')
  await page.click('.ygg-ui__scenario-rename button[type="submit"]')
  await page.waitFor("__store.get().scenario === 'drawer-renamed' && __store.get().scenarios.map((held) => held.name).join() === 'drawer-renamed'")
  assert.deepEqual((await served('/api/scenarios')).scenarios.map((held) => [held.name, held.events.length]), [['drawer-renamed', 1]])
  await page.waitFor('__store.get().scenarioBooks?.length === 7')

  // Remove its one event: every book of the scenario stream is the base's again, hash for hash.
  await page.frames()
  await page.click('.ygg-ui__scenario-event [data-action="remove-event"]')
  await page.waitFor("__store.get().scenarios[0]?.events.length === 0 && __store.get().scenarioBooks?.length === 6")
  assert.deepEqual(await state('s.scenarioBooks.map((book) => book.stableHash)'), baseHashes)
  assert.deepEqual(await state('s.books.map((book) => book.stableHash)'), baseHashes)
  assert.equal(await state('s.scenarioFrom'), null)
  const restored = await streamed('/api/sources/synthetic/scenarios/drawer-renamed/books?symbol=ALPHA')
  assert.deepEqual(restored.books.map((book) => book.stableHash), baseHashes)
  // The diff read again: every instant the same.
  await page.waitFor("__store.get().diff?.instants.every((row) => row.changes.same)")

  // Delete, asked twice.
  await page.frames()
  await page.click('.ygg-ui__scenario-item [data-action="delete"]')
  await page.frames()
  await page.click('.ygg-ui__scenario-item [data-action="delete"]')
  await page.waitFor("__store.get().scenarios.length === 0 && __store.get().scenario === null")
  assert.deepEqual(await state('[s.stream, s.scenarioBooks, s.diff]'), ['base', null, null])
  assert.deepEqual((await served('/api/scenarios')).scenarios, [])
  assert.deepEqual(page.consoleLines, [])
})

test('a scenario window and its diff are shown under their own walk only: another symbol shows its own or none', async () => {
  const at = T0 + 3n * MS + 500_000n
  const posted = await fetch(new URL('/api/scenarios/walk-bound/events', base), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'order_event', currunix: String(at), facts: { crosscode: 'ALPHA-W-1', ticker: 'ALPHA', side: 'BUY', price: '82.25', quantity: '5' } }),
  })
  assert.equal(posted.status, 201)
  try {
    await openApp()
    await press('s')
    await page.waitFor("!document.querySelector('.ygg-ui__scenarios').hidden")
    await page.click('.ygg-ui__scenario-name')
    await page.waitFor("__store.get().stream === 'scenario' && __store.get().scenarioBooks?.length === 7")
    await press('Escape')
    await press('d')
    await page.waitFor("__store.get().diff?.symbol === 'ALPHA'")
    // Every state from here on: a scenario window or a diff names the walk the page names, or is not there.
    await page.evaluate(`(() => {
      window.wrong = []
      __store.subscribe((s) => {
        const walks = new Set((s.scenarioBooks ?? []).map((book) => book.crosscode))
        if (s.scenarioBooks && (walks.size !== 1 || !walks.has(s.symbol))) wrong.push(['window', s.symbol, [...walks]])
        if (s.diff && s.diff.symbol !== s.symbol) wrong.push(['diff', s.symbol, s.diff.symbol])
      })
    })()`)
    await fill('.ygg-ui__symbol-select', 'BETA')
    await page.waitFor("__store.get().symbol === 'BETA' && !__store.get().loading && __store.get().scenarioBooks?.[0]?.crosscode === 'BETA' && __store.get().diff?.symbol === 'BETA'")
    assert.deepEqual(await page.evaluate('wrong'), [])
    const beta = await streamed('/api/sources/synthetic/scenarios/walk-bound/books?symbol=BETA&from=0')
    assert.deepEqual(await state('s.scenarioBooks.map((book) => book.stableHash)'), beta.books.map((book) => book.stableHash))
    // Away and straight back before the other walk lands: that load is dropped, and the window and diff it cleared are read again.
    await page.evaluate("__store.set({ symbol: 'ALPHA' }); __store.set({ symbol: 'BETA' })")
    await page.waitFor("!__store.get().loading && __store.get().scenarioBooks?.[0]?.crosscode === 'BETA' && __store.get().diff?.symbol === 'BETA'")
    await page.frames(4)
    assert.deepEqual(await state('[s.symbol, s.books[0].crosscode, s.walk.symbol]'), ['BETA', 'BETA', 'BETA'])
    assert.deepEqual(await state('s.scenarioBooks.map((book) => book.stableHash)'), beta.books.map((book) => book.stableHash))
    assert.deepEqual(await page.evaluate('wrong'), [])
    assert.deepEqual(page.consoleLines, [])
  } finally {
    assert.equal((await fetch(new URL('/api/scenarios/walk-bound', base), { method: 'DELETE' })).status, 204)
  }
})

test('theme: prefers-color-scheme until chosen, the choice kept across loads; the loaded app in both themes', async () => {
  await page.emulate({ colorScheme: 'light' })
  try {
    await openApp()
    await page.evaluate("localStorage.removeItem('ygg-ui-theme')")
    await openApp()
    assert.equal(await page.evaluate('document.documentElement.dataset.theme ?? null'), null)
    assert.equal(await state('s.theme'), 'light')
    assert.equal(await page.evaluate('getComputedStyle(document.body).backgroundColor'), 'rgb(255, 255, 255)')
    assert.equal(await page.evaluate("document.querySelector('.ygg-ui__theme-toggle').getAttribute('aria-pressed')"), 'false')
    // A book with prints and a chain in the inspector: every panel has something to show.
    await press('End')
    await page.click('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row:nth-child(3)')
    await page.waitFor("__store.get().lifecycle?.crosscode === 'ALPHA-E-1'")
    await page.settle()
    await page.screenshot('app-light')

    await press('t')
    assert.equal(await page.evaluate('document.documentElement.dataset.theme'), 'dark')
    assert.equal(await page.evaluate("localStorage.getItem('ygg-ui-theme')"), 'dark')
    assert.equal(await state('s.theme'), 'dark')
    // Kept across a load, over the preference.
    await openApp()
    assert.equal(await page.evaluate('document.documentElement.dataset.theme'), 'dark')
    assert.equal(await state('s.theme'), 'dark')
    assert.equal(await page.evaluate('getComputedStyle(document.body).backgroundColor'), 'rgb(0, 0, 0)')
    await press('End')
    await page.click('.ygg-ui__tape .ygg-ui__virtual-layer .ygg-ui__tape-row:nth-child(3)')
    await page.waitFor("__store.get().lifecycle?.crosscode === 'ALPHA-E-1'")
    await page.settle()
    await page.screenshot('app-dark')
    await page.evaluate("localStorage.removeItem('ygg-ui-theme')")
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await page.emulate({})
  }
})

test('keyboard: a handler for every command; ? the sheet, Ctrl+K the palette, Escape the top panel; focus rings show', async () => {
  const { SHORTCUTS } = await import('../../web/shortcuts.js')
  await openApp()
  const ids = await page.evaluate('__app.commandIds')
  for (const [, id] of SHORTCUTS) assert.ok(ids.includes(id), `no handler for ${id}`)
  assert.ok(ids.includes('transport.seek') && ids.includes('transport.speed'))

  await press('?')
  await page.waitFor("document.querySelector('.ygg-ui__sheet[open]')")
  assert.equal(await page.evaluate("document.querySelectorAll('.ygg-ui__sheet tbody tr').length"), SHORTCUTS.length)
  // A shortcut pressed while a modal is open is the modal's, not the replay's.
  await press('ArrowRight')
  assert.equal(await state('s.index'), 0)
  await press('Escape')
  await page.waitFor("!document.querySelector('.ygg-ui__sheet[open]')")

  await press('k', { modifiers: 2 })
  await page.waitFor("document.querySelector('.ygg-ui__palette[open]')")
  assert.equal(await page.evaluate("document.activeElement === document.querySelector('.ygg-ui__palette-input')"), true)
  await page.type('Step one book forward')
  await press('Enter')
  await page.waitFor("!document.querySelector('.ygg-ui__palette[open]') && __store.get().index === 1")

  // Escape closes the drawer, the one panel open, from wherever focus is.
  await press('s')
  await page.waitFor("!document.querySelector('.ygg-ui__scenarios').hidden")
  await page.evaluate('document.activeElement.blur()')
  await press('Escape')
  assert.equal(await page.evaluate("document.querySelector('.ygg-ui__scenarios').hidden"), true)
  // The insert modal opens on `i` and closes on Escape, as every modal does.
  await press('i')
  await page.waitFor("document.querySelector('.ygg-ui__modal[open] .ygg-ui__insert')")
  await press('Escape')
  await page.waitFor("!document.querySelector('.ygg-ui__modal[open]')")

  // Tab reaches a control, and the control shows where focus is.
  await page.evaluate('document.activeElement.blur()')
  let tag = ''
  for (let step = 0; step < 12 && tag !== 'BUTTON'; step += 1) {
    await press('Tab')
    tag = await page.evaluate('document.activeElement.tagName')
  }
  assert.equal(tag, 'BUTTON')
  assert.deepEqual(await page.evaluate("(() => { const style = getComputedStyle(document.activeElement); return [style.outlineStyle, style.outlineWidth] })()"), ['solid', '2px'])
  assert.deepEqual(page.consoleLines, [])
})

test('read-only: mounted over a recorded api, the insert modal says why and scenario editing is off', async () => {
  const { cpSync, writeFileSync } = await import('node:fs')
  const { serve } = await import('./browser.js')
  // A recording of the service: what the page reads, answered by a plain object.
  const at = T0 + 3n * MS + 500_000n
  const posted = await fetch(new URL('/api/scenarios/recorded/events', base), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'order_event', currunix: String(at), facts: { crosscode: 'ALPHA-R-1', ticker: 'ALPHA', side: 'BUY', price: '82.25', quantity: '5' } }),
  })
  assert.equal(posted.status, 201)
  const walk = 'symbol=ALPHA&global=false&snapshotMillis=0'
  const scenario = await streamed(`/api/sources/synthetic/scenarios/recorded/books?${walk}`)
  const recorded = {
    sources: await served('/api/sources'),
    views: (await served('/api/sources')).views,
    field: await served('/api/field'),
    scenarios: await served('/api/scenarios'),
    books: (await streamed(`/api/sources/synthetic/books?${walk}`)).books,
    scenarioBooks: scenario.books,
    scenarioEnd: scenario.end,
    diff: await served(`/api/sources/synthetic/scenarios/recorded/diff?${walk}`),
  }
  assert.equal((await fetch(new URL('/api/scenarios/recorded', base), { method: 'DELETE' })).status, 204)
  const site = mkdtempSync(path.join(os.tmpdir(), 'yggdryl-app-recorded-'))
  cpSync(WEB, path.join(site, 'web'), { recursive: true })
  writeFileSync(path.join(site, 'recorded.json'), JSON.stringify(recorded))
  writeFileSync(
    path.join(site, 'index.html'),
    `<!doctype html>
<html lang="en" data-readonly>
<head><meta charset="utf-8"><title>recorded replay</title><link rel="icon" href="data:,">
<link rel="stylesheet" href="/web/theme.css"><link rel="stylesheet" href="/web/app/app.css"></head>
<body><div id="replay"></div>
<script type="module">
import { mountApp } from '/web/app/app.js'
const recorded = await (await fetch('/recorded.json')).json()
const refused = async () => ({ refusal: 'a recorded replay changes nothing', status: 0 })
const replay = (books, end, { onBook }) => {
  for (const book of books) onBook(book)
  return end
}
// Every call the app makes, by method: what it reads, and that it writes nothing.
window.calls = {}
window.viewQueries = []
const count = (name, answer) => (...args) => {
  calls[name] = (calls[name] ?? 0) + 1
  return answer(...args)
}
const methods = {
  sources: async () => recorded.sources,
  views: async () => ({ views: recorded.views }),
  field: async () => recorded.field,
  scenarios: async () => recorded.scenarios,
  // A recording holds the walk without a grid, and refuses one - as the docs page's does.
  books: async (id, query, handlers) =>
    Number(query.snapshotMillis ?? 0) > 0 ? { refusal: 'this recording holds no grid', status: 0 } : replay(recorded.books, { count: recorded.books.length }, handlers),
  scenarioBooks: async (id, name, query, handlers) => replay(recorded.scenarioBooks, recorded.scenarioEnd, handlers),
  diff: async () => recorded.diff,
  lifecycle: async () => ({ columns: [], rows: [] }),
  view: async (id, query) => {
    viewQueries.push(query)
    return { columns: [], rows: [] }
  },
  book: refused,
  insert: refused,
  removeEvent: refused,
  saveScenario: refused,
  deleteScenario: refused,
}
const api = Object.fromEntries(Object.entries(methods).map(([name, answer]) => [name, count(name, answer)]))
window.app = mountApp(document.getElementById('replay'), { api, debug: true })
await app.ready
window.ready = true
</script>
</body>
</html>
`,
  )
  const server = await serve(site)
  try {
    page.consoleLines.length = 0
    await page.open(server.url)
    await page.waitFor('window.ready === true', 20_000)
    await page.frames()
    assert.deepEqual(await state('[s.readonly, s.books.length, s.index]'), [true, 6, 0])
    // The view names come with the listing: one read of it, no second.
    assert.deepEqual(await page.evaluate('[calls.sources, calls.views ?? 0]'), [1, 0])
    assert.deepEqual(await state('s.views'), recorded.views)
    assert.equal(await page.evaluate("document.getElementById('replay').hasAttribute('data-readonly')"), true)
    // The host document keeps its own frame: the app styles its root, never the host's body.
    assert.equal(await page.evaluate('getComputedStyle(document.body).marginTop'), '8px')
    const pageColor = await page.evaluate("(() => { const probe = document.createElement('div'); probe.style.background = 'var(--ygg-ui-page)'; document.body.append(probe); const color = getComputedStyle(probe).backgroundColor; probe.remove(); return color })()")
    assert.equal(await page.evaluate("getComputedStyle(document.getElementById('replay')).backgroundColor"), pageColor)
    // A refused walk: the refusal told, the window and every control naming it still the loaded walk's, no view asked of it.
    await page.waitFor('viewQueries.length === 1')
    const walk = await state('s.walk')
    await press('g')
    await page.waitFor('__store.get().refusal')
    await page.frames()
    assert.equal(await state('s.refusal'), 'this recording holds no grid')
    assert.equal(await text('.ygg-ui__toast--refusal .ygg-ui__toast-text'), 'this recording holds no grid')
    assert.deepEqual(await state('[s.grid, s.loading, s.books.length, s.walk]'), [false, false, 6, walk])
    assert.equal(await page.evaluate("document.querySelector('.ygg-ui__transport-grid').getAttribute('aria-pressed')"), 'false')
    await page.frames(6)
    assert.deepEqual(await page.evaluate('[calls.books, calls.view]'), [2, 1])
    assert.deepEqual(await page.evaluate('viewQueries.map((query) => query.snapshotMillis)'), [0])
    await page.evaluate("document.querySelectorAll('.ygg-ui__toast-dismiss').forEach((button) => button.click())")
    await page.evaluate('__store.set({ refusal: null })')
    await press('i')
    await page.waitFor("document.querySelector('.ygg-ui__modal[open]')")
    assert.equal(await text('.ygg-ui__modal[open] .ygg-app__readonly'), 'Inserting events needs the replay service; this page renders a recorded replay.')
    assert.equal(await page.evaluate("document.querySelector('.ygg-ui__modal .ygg-ui__insert')"), null)
    await press('Escape')
    await page.waitFor("!document.querySelector('.ygg-ui__modal[open]')")
    // The scenarios read, and choose, but change nothing.
    await press('s')
    await page.waitFor("!document.querySelector('.ygg-ui__scenarios').hidden")
    const hidden = (selector) => page.evaluate(`getComputedStyle(document.querySelector(${JSON.stringify(selector)})).display === 'none'`)
    for (const selector of ['.ygg-ui__scenario-new', '.ygg-ui__scenario-item [data-action="rename"]', '.ygg-ui__scenario-item [data-action="delete"]']) {
      assert.equal(await hidden(selector), true, selector)
    }
    await page.click('.ygg-ui__scenario-name')
    await page.waitFor("__store.get().stream === 'scenario' && __store.get().scenarioBooks?.length === 7")
    await page.frames()
    assert.equal(await hidden('.ygg-ui__scenario-event [data-action="remove-event"]'), true)
    assert.deepEqual(await state('s.scenarioBooks.slice(4).map((book) => book.stableHash)'), recorded.scenarioBooks.map((book) => book.stableHash))
    await press('d')
    await page.waitFor("__store.get().diff")
    assert.deepEqual(await state('s.diff'), recorded.diff)
    // Every edit a person could ask for - the drawer's four and an insert - writes nothing and refuses nothing: the page says why.
    await page.evaluate(`(() => {
      const drawer = document.querySelector('.ygg-ui__scenarios')
      for (const detail of [
        { command: 'create', name: 'another' },
        { command: 'rename', name: 'recorded', to: 'renamed' },
        { command: 'delete', name: 'recorded' },
        { command: 'remove-event', name: 'recorded', curruuid: __store.get().scenarios[0].events[0].curruuid },
      ]) {
        drawer.dispatchEvent(new CustomEvent('ygg:scenario', { detail, bubbles: true }))
      }
      document.getElementById('replay').dispatchEvent(new CustomEvent('ygg:insert', { detail: { kind: 'order_event', currunix: '${at}', facts: {} }, bubbles: true }))
    })()`)
    await page.frames()
    assert.deepEqual(await page.evaluate("['insert', 'removeEvent', 'saveScenario', 'deleteScenario'].map((name) => calls[name] ?? 0)"), [0, 0, 0, 0])
    assert.equal(await state('s.refusal'), null)
    assert.deepEqual(
      await page.evaluate("[...document.querySelectorAll('.ygg-ui__toast--info .ygg-ui__toast-text')].map((toast) => toast.textContent)"),
      Array(4).fill('Scenarios are read-only here: this page renders a recorded replay'),
    )
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await server.close()
    rmSync(site, { recursive: true, force: true })
  }
})

test('ten thousand books: a scrub stays inside a frame', async (t) => {
  await openApp()
  const measured = await page.evaluate(`(async () => {
    const served = __store.get().books
    const origin = BigInt(served[0].currunix)
    const books = []
    for (let at = 0; at < 10000; at += 1) {
      const book = served[at % served.length]
      books.push({ ...book, currunix: String(origin + BigInt(at) * 1000n), snapunix: null })
    }
    __store.set({ books: Object.freeze(books), index: 0, stream: 'base' })
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
    await new Promise((resolve) => setTimeout(resolve, 250))
    const costs = []
    const syncs = []
    let seed = 7
    for (let step = 0; step < 200; step += 1) {
      seed = (seed * 48271) % 2147483647
      const index = seed % books.length
      const started = performance.now()
      __app.command('transport.seek', index)
      const sync = performance.now() - started
      // The frame after the seek: its callbacks before this one are the components' draws.
      const drawn = await new Promise((resolve) => requestAnimationFrame((frame) => resolve(performance.now() - frame)))
      syncs.push(sync)
      costs.push(sync + drawn)
    }
    const sorted = (list) => [...list].sort((a, b) => a - b)
    const cost = sorted(costs)
    const sync = sorted(syncs)
    return {
      median: cost[cost.length >> 1],
      p95: cost[Math.floor(cost.length * 0.95)],
      max: cost.at(-1),
      syncMedian: sync[sync.length >> 1],
      index: __store.get().index,
      position: document.querySelector('.ygg-ui__transport-position').textContent,
    }
  })()`)
  t.diagnostic(`scrub over 10000 books: median ${measured.median.toFixed(2)} ms, p95 ${measured.p95.toFixed(2)} ms, max ${measured.max.toFixed(2)} ms (store update median ${measured.syncMedian.toFixed(2)} ms)`)
  assert.equal(measured.position, `${measured.index + 1} of 10000`)
  assert.ok(measured.p95 < 1000 / 60, `a scrub's 95th percentile took ${measured.p95} ms`)
  assert.deepEqual(page.consoleLines, [])
})

test('api: every route answers the served JSON untouched; a refused call or stream answers the service\'s text', async () => {
  await openApp()
  const answers = await page.evaluate(`(async () => {
    const { createApi } = await import('/web/app/api.js')
    const api = createApi(location.origin)
    const books = []
    const end = await api.books('synthetic', { symbol: 'ALPHA', from: '${T0 + MS}', to: '${T0 + MS + 1n}' }, { onBook: (book) => books.push(book) })
    const refusedStream = await api.books('synthetic', { symbol: 'GAMMA' }, { onBook: () => { throw new Error('no book was expected') } })
    const control = new AbortController()
    control.abort(new Error('left'))
    let aborted = null
    try {
      await api.books('synthetic', { symbol: 'ALPHA' }, { signal: control.signal })
    } catch (error) {
      aborted = error.message
    }
    return {
      sources: await api.sources(),
      field: await api.field(),
      book: await api.book('synthetic', { symbol: 'ALPHA', at: '${T0 + 2n * MS}' }),
      lifecycle: await api.lifecycle('synthetic', 'BETA-O-1'),
      scenarios: await api.scenarios(),
      refused: await api.lifecycle('synthetic', undefined),
      books,
      end,
      refusedStream,
      aborted,
    }
  })()`)
  assert.deepEqual(answers.sources, await served('/api/sources'))
  assert.deepEqual(answers.field, await served('/api/field'))
  assert.deepEqual(answers.book, await served(`/api/sources/synthetic/book?symbol=ALPHA&at=${T0 + 2n * MS}`))
  assert.deepEqual(answers.lifecycle, await served('/api/sources/synthetic/lifecycle?crosscode=BETA-O-1'))
  assert.deepEqual(answers.scenarios, await served('/api/scenarios'))
  // Two books one nanosecond apart, in walk order, then the end event's count.
  const window = await streamed(`/api/sources/synthetic/books?symbol=ALPHA&from=${T0 + MS}&to=${T0 + MS + 1n}`)
  assert.deepEqual(answers.books, window.books)
  assert.deepEqual(answers.books.map((book) => book.currunix), [String(T0 + MS), String(T0 + MS + 1n)])
  assert.deepEqual(answers.end, window.end)
  assert.deepEqual(answers.refused, { refusal: (await served('/api/sources/synthetic/lifecycle')).error, status: 400 })
  assert.deepEqual(answers.refusedStream, { refusal: 'no symbol "GAMMA" in this walk; its symbols are ALPHA, BETA', status: 404 })
  assert.equal(answers.aborted, 'left')
  // The refused calls log their status, as the browser logs every refused request: the lifecycle read once, and the
  // GAMMA stream twice - the event source, then the plain read that recovers the service's text.
  assert.equal(
    takeLines(/^log: Failed to load resource: the server responded with a status of 400 \(Bad Request\) http:\/\/127\.0\.0\.1:\d+\/api\/sources\/synthetic\/lifecycle$/).length,
    1,
  )
  assert.equal(
    takeLines(/^log: Failed to load resource: the server responded with a status of 404 \(Not Found\) http:\/\/127\.0\.0\.1:\d+\/api\/sources\/synthetic\/books\?symbol=GAMMA$/).length,
    2,
  )
  assert.deepEqual(page.consoleLines, [])
})
