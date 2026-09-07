'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')

const { byClass, byTag, installDOM } = require('./dom.js')
const uiPromise = import('../../ui/index.mjs')

test('elements, cards, pills, calls and notes keep caller text literal', async () => {
  const held = installDOM()
  try {
    const { callBlock, cardRow, make, note, pill } = await uiPromise
    const raw = '<b>cash & carry</b>'
    const node = make('span', 'quote', raw)
    assert.equal(node.textContent, raw)
    assert.equal(node.children.length, 0)

    const cards = cardRow({
      cards: [
        { value: 6210, label: 'Fields', note: 'indexed once' },
        { value: 181, label: pill({ text: 'Messages', kind: 'info' }) },
      ],
    })
    assert.equal(byClass(cards, 'ygg-ui__card').length, 2)
    assert.match(cards.textContent, /6?210Fieldsindexed once/)
    assert.equal(byClass(cards, 'ygg-ui__pill--info').length, 1)
    assert.equal(callBlock({ code: raw, answer: 'result' }).textContent, `result${raw}`)
    assert.ok(note({ content: raw, kind: 'warning' }).className.includes('ygg-ui__note--warning'))
  } finally {
    held.restore()
  }
})

test('panel fills once on first open and native details state remains usable', async () => {
  const held = installDOM()
  try {
    const { panel } = await uiPromise
    let fills = 0
    const view = panel({
      title: 'Risk',
      aside: 'lazy',
      render: () => {
        fills += 1
        return `filled ${fills}`
      },
    })
    assert.equal(view.element.tagName, 'DETAILS')
    assert.equal(view.body.textContent, '')
    view.element.open = true
    view.element.dispatchEvent('toggle')
    view.element.open = false
    view.element.dispatchEvent('toggle')
    view.element.open = true
    view.element.dispatchEvent('toggle')
    assert.equal(fills, 1)
    assert.equal(view.body.textContent, 'filled 1')
  } finally {
    held.restore()
  }
})

test('facts omit absent rows while grids retain zero and mark only absence', async () => {
  const held = installDOM()
  try {
    const { facts, grid, make } = await uiPromise
    const table = facts({
      rows: [
        { label: 'empty', value: '' },
        { label: 'null', value: null },
        { label: 'zero', value: 0 },
        { label: 'false', value: false },
        { label: 'node', value: make('strong', null, 'kept') },
      ],
    })
    assert.equal(byTag(table, 'tr').length, 3)
    assert.match(table.textContent, /zero0falsefalse nodekept|zero0falsefalse.*nodekept/)

    const view = grid({
      columns: [
        { label: 'Name', key: 'name', rowHeader: true },
        { label: 'PnL', key: 'pnl', format: (value) => `$${value}` },
      ],
      rows: [
        { name: 'Alpha', pnl: 0 },
        { name: 'Beta', pnl: null },
      ],
    })
    assert.equal(byTag(view, 'thead').length, 1)
    assert.match(view.textContent, /Alpha\$0Beta—/)
    assert.equal(byTag(view, 'tbody')[0].children[0].children[0].getAttribute('scope'), 'row')
  } finally {
    held.restore()
  }
})

test('controls are labelled and surface input, change and click events', async () => {
  const held = installDOM()
  try {
    const { button, control, input, search, select, textarea } = await uiPromise
    let inputs = 0
    let changes = 0
    let clicks = 0
    const box = input({ value: '10', onInput: () => { inputs += 1 } })
    const labelled = control({ id: 'limit', label: 'Limit', node: box })
    box.dispatchEvent('input')
    assert.equal(inputs, 1)
    assert.equal(box.id, 'limit')
    assert.equal(byTag(labelled, 'label')[0].getAttribute('for'), 'limit')

    const query = search({ placeholder: 'Symbol', wide: true })
    assert.equal(query.type, 'search')
    assert.match(query.className, /ygg-ui__input--wide/)
    const chooser = select({
      value: 'b',
      options: [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }],
      onChange: () => { changes += 1 },
    })
    chooser.dispatchEvent('change')
    assert.equal(changes, 1)
    assert.equal(chooser.value, 'b')
    assert.equal(byTag(chooser, 'option').length, 2)
    assert.equal(textarea({ value: 'x', rows: 4, spellcheck: false }).rows, 4)
    const action = button({ label: 'Run', kind: 'primary', onClick: () => { clicks += 1 } })
    const danger = button({ label: 'Remove', kind: 'danger' })
    action.dispatchEvent('click')
    assert.equal(clicks, 1)
    assert.equal(action.type, 'button')
    assert.match(danger.className, /ygg-ui__button--danger/)
  } finally {
    held.restore()
  }
})

test('modal uses native dialog, closes predictably and restores focus', async () => {
  const held = installDOM({ dialog: true })
  try {
    const { button, input, modal } = await uiPromise
    const trigger = button({ label: 'Review' })
    const initial = input({ value: 'AAPL' })
    const reasons = []
    const view = modal({
      title: 'Review order',
      content: initial,
      initialFocus: initial,
      onClose: (reason) => reasons.push(reason),
    })
    held.document.body.append(trigger, view.element)
    trigger.focus()

    view.open()
    view.open()
    assert.equal(view.opened, true)
    assert.equal(view.element.showModalCalls, 1)
    assert.equal(view.element.getAttribute('role'), 'dialog')
    assert.equal(view.element.getAttribute('aria-modal'), 'true')
    assert.equal(view.element.getAttribute('aria-hidden'), null)
    assert.equal(held.document.activeElement, initial)

    view.element.dispatchEvent({ type: 'cancel' })
    assert.equal(view.opened, false)
    assert.equal(view.element.open, false)
    assert.deepEqual(reasons, ['escape'])
    assert.equal(held.document.activeElement, trigger)

    view.open()
    view.closeButton.dispatchEvent('click')
    assert.deepEqual(reasons, ['escape', 'button'])
    assert.equal(held.document.activeElement, trigger)
  } finally {
    held.restore()
  }
})

test('modal fallback traps dismissal inside its scoped overlay', async () => {
  const held = installDOM()
  try {
    const { button, modal } = await uiPromise
    const trigger = button({ label: 'Open' })
    const reasons = []
    const view = modal({
      title: 'Fallback',
      content: 'No dialog methods are required.',
      onClose: (reason) => reasons.push(reason),
    })
    held.document.body.append(trigger, view.element)
    trigger.focus()
    assert.equal(view.element.hasAttribute('data-ygg-ui-fallback'), true)
    assert.equal(view.element.hidden, true)

    view.open()
    assert.equal(view.element.hidden, false)
    assert.equal(view.element.open, true)
    assert.equal(held.document.activeElement, view.closeButton)
    view.element.dispatchEvent({ type: 'click', target: view.surface })
    assert.equal(view.opened, true)
    view.element.dispatchEvent({ type: 'click', target: view.element })
    assert.equal(view.opened, false)
    assert.equal(view.element.hidden, true)
    assert.deepEqual(reasons, ['backdrop'])
    assert.equal(held.document.activeElement, trigger)

    view.open()
    held.document.dispatchEvent({ type: 'keydown', key: 'Escape' })
    assert.equal(view.opened, false)
    assert.deepEqual(reasons, ['backdrop', 'escape'])
  } finally {
    held.restore()
  }
})

test('choice list exposes one pressed choice and rejects invalid indexes', async () => {
  const held = installDOM()
  try {
    const { choiceList } = await uiPromise
    const selected = []
    const view = choiceList({
      items: ['day', 'week'],
      render: (item) => item,
      selected: 0,
      onSelect: (item) => selected.push(item),
    })
    view.select(1)
    assert.equal(view.selectedIndex, 1)
    assert.deepEqual(view.buttons.map((node) => node.getAttribute('aria-pressed')), ['false', 'true'])
    assert.deepEqual(selected, ['week'])
    assert.throws(() => view.select(2), /outside/)
  } finally {
    held.restore()
  }
})

test('wire block separates displayed text from clipboard text and catches denial', async () => {
  const writes = []
  const held = installDOM({ navigator: { clipboard: { writeText: async (text) => writes.push(text) } } })
  try {
    const { wireBlock } = await uiPromise
    const wire = wireBlock({ display: '8=FIX.4.4␁', copy: '8=FIX.4.4\u0001', duration: 1 })
    assert.equal(await wire.copy(), true)
    assert.deepEqual(writes, ['8=FIX.4.4\u0001'])
    assert.equal(wire.body.textContent, '8=FIX.4.4␁')
  } finally {
    held.restore()
  }

  const fallback = installDOM({ navigator: { clipboard: { writeText: async () => { throw new Error('denied') } } } })
  try {
    fallback.document.copyResult = true
    const { wireBlock } = await uiPromise
    assert.equal(await wireBlock({ display: 'shown', copy: 'copied', duration: 1 }).copy(), true)
    assert.equal(fallback.document.selection, 'copied')
  } finally {
    fallback.restore()
  }
})

test('search precomputes haystacks and searchable lists cap visible DOM honestly', async () => {
  const held = installDOM()
  try {
    const { createSearchIndex, make, searchableList } = await uiPromise
    let indexed = 0
    const items = Array.from({ length: 7 }, (_, id) => ({ id, name: `Trade ${id}` }))
    const index = createSearchIndex({
      items,
      haystack: (item) => {
        indexed += 1
        return `${item.id} ${item.name}`
      },
    })
    const view = searchableList({
      index,
      pageSize: 3,
      noun: 'trades',
      renderPage: (page) => page.map((item) => make('p', null, item.name)),
    })
    assert.equal(indexed, 7)
    assert.equal(byTag(view.list, 'p').length, 3)
    assert.match(view.count.textContent, /7 of 7 trades; 3 drawn, 4 not drawn/)
    view.more.dispatchEvent('click')
    assert.equal(byTag(view.list, 'p').length, 6)
    view.input.value = 'Trade 2'
    view.input.dispatchEvent('input')
    assert.equal(byTag(view.list, 'p').length, 1)
    assert.equal(indexed, 7, 'typing never rebuilds haystacks')
    assert.throws(
      () => searchableList({ index, pageSize: 0, renderPage: () => null }),
      /positive finite integer/,
    )
  } finally {
    held.restore()
  }
})

test('tree builds child branches lazily and marks cycles', async () => {
  const held = installDOM()
  try {
    const { tree } = await uiPromise
    const root = { id: 'root', children: [] }
    root.children.push(root)
    let reads = 0
    const view = tree({
      items: [root],
      key: (item) => item.id,
      describe: (item) => ({
        label: item.id,
        branch: true,
        children: () => {
          reads += 1
          return item.children
        },
      }),
    })
    assert.equal(reads, 0)
    const details = byTag(view, 'details')[0]
    details.open = true
    details.dispatchEvent('toggle')
    assert.equal(reads, 1)
    assert.equal(byClass(view, 'ygg-ui__tree-cycle').length, 1)
  } finally {
    held.restore()
  }
})

test('lazy indexes build once, retain failures and refuse recursion', async () => {
  const { createLazyIndex } = await uiPromise
  let builds = 0
  const index = createLazyIndex({ build: () => ({ build: ++builds }) })
  assert.equal(index.built, false)
  assert.strictEqual(index.get(), index.get())
  assert.equal(builds, 1)

  const failure = new Error('bad index')
  const broken = createLazyIndex({ build: () => { throw failure } })
  assert.throws(() => broken.get(), (error) => error === failure)
  assert.throws(() => broken.get(), (error) => error === failure)
  let recursive
  recursive = createLazyIndex({ build: () => recursive.get() })
  assert.throws(() => recursive.get(), /called itself/)
})

test('asset loader fetches each URL once, prefetches on its scheduler and explains failures', async () => {
  const held = installDOM()
  try {
    const { assetLoader } = await uiPromise
    const calls = []
    const scheduled = []
    const fetcher = async (url) => {
      calls.push(url)
      return url.endsWith('bad.json')
        ? { ok: false, status: 404, statusText: 'Missing' }
        : { ok: true, json: async () => ({ url }) }
    }
    const loader = assetLoader({
      baseURL: 'https://example.test/assets/app.js',
      assets: { good: 'good.json', bad: 'bad.json' },
      regenerate: 'node scripts/build.js',
      fetch: fetcher,
      schedule: (run) => scheduled.push(run),
    })
    const first = loader.json('good')
    const second = loader.json('good')
    assert.strictEqual(first, second)
    assert.equal((await first).url, 'https://example.test/assets/good.json')
    assert.equal(calls.length, 1)
    const prefetch = loader.prefetch('bad')
    assert.equal(scheduled.length, 1)
    scheduled[0]()
    await assert.rejects(prefetch, /404 Missing/)
    const failure = loader.failure({ asset: 'bad', error: new Error('offline') })
    assert.match(failure.textContent, /bad\.json.*offline.*node scripts\/build\.js/)
    assert.throws(() => loader.address('missing'), /unknown asset/)
  } finally {
    held.restore()
  }
})

test('document readiness supports immediate, DOMContentLoaded and navigation lifecycles', async () => {
  const held = installDOM()
  try {
    const { onDocumentReady } = await uiPromise
    let runs = 0
    onDocumentReady({ document: held.document, run: () => { runs += 1 } })
    assert.equal(runs, 1)

    held.document.readyState = 'loading'
    const stop = onDocumentReady({ document: held.document, run: () => { runs += 1 } })
    assert.equal(runs, 1)
    held.document.dispatchEvent('DOMContentLoaded')
    assert.equal(runs, 2)
    stop()

    let subscriber
    let unsubscribed = false
    const navigation = {
      subscribe(run) {
        subscriber = run
        return { unsubscribe: () => { unsubscribed = true } }
      },
    }
    const end = onDocumentReady({ document: held.document, navigation, run: () => { runs += 1 } })
    subscriber()
    assert.equal(runs, 3)
    end()
    assert.equal(unsubscribed, true)
  } finally {
    held.restore()
  }
})
