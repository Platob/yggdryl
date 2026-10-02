// Node smoke of the UI's pure modules: axis math, merges, selection, tiles,
// the fill offset, edit serialization; P3: number-format codes, the width
// conversion, the copy marker, the zoom slider and the change-feed rules;
// P4: the fill handle's target and corner, tab moves, the instance marker;
// P5: the sheet gate, the formula assist's text rules, the function search.
// node unit.mjs
import assert from 'node:assert/strict'
import { Axis, Geometry, Merges, columnWidthPx, parseRange, rangeName, refName, parseRef, columnName, DEFAULT_COLUMN_WIDTH } from '/home/user/yggdryl/cli/assets/excel/js/geometry.js'
import { fillText } from '/home/user/yggdryl/cli/assets/excel/js/render.js'
import { Api, ApiError } from '/home/user/yggdryl/cli/assets/excel/js/api.js'
import { Tiles, CAPACITY, IN_FLIGHT } from '/home/user/yggdryl/cli/assets/excel/js/tiles.js'
import { Selection } from '/home/user/yggdryl/cli/assets/excel/js/selection.js'

let passed = 0
const test = async (name, run) => {
  await run()
  passed++
  console.log('ok  ', name)
}

await test('names: columns, references, ranges', () => {
  assert.equal(columnName(0), 'A')
  assert.equal(columnName(25), 'Z')
  assert.equal(columnName(26), 'AA')
  assert.equal(columnName(16383), 'XFD')
  assert.deepEqual(parseRef('XFD1048576'), { row: 1048575, column: 16383 })
  assert.equal(parseRef('XFE1'), null)
  assert.equal(parseRef('A0'), null)
  assert.equal(parseRef('A1048577'), null)
  assert.deepEqual(parseRange('$B$3:A1'), { r0: 0, c0: 0, r1: 2, c1: 1 })
  assert.deepEqual(parseRange('C:D'), { r0: 0, c0: 2, r1: 1048575, c1: 3 })
  assert.deepEqual(parseRange('2:4'), { r0: 1, c0: 0, r1: 3, c1: 16383 })
  assert.equal(rangeName({ r0: 1, c0: 1, r1: 1, c1: 1 }), 'B2')
  assert.equal(refName(1048575, 16383), 'XFD1048576')
})

await test('widths: the §4.2 conversion', () => {
  assert.equal(columnWidthPx(9.140625, 7), 64)
  assert.equal(columnWidthPx(8.43, 7), 59)
  assert.equal(columnWidthPx(20.7, 7), 145)
})

await test('axis: start and indexAt agree across runs and hidden spans', () => {
  const runs = [
    { first: 2, last: 4, size: 30, style: null },
    { first: 5, last: 7, size: 0, style: null },
    { first: 10, last: 10, size: 5, style: 3 },
    { first: 1000000, last: 1048575, size: 0, style: null }
  ]
  const axis = new Axis(1048576, 20, runs)
  let at = 0
  for (let i = 0; i < 1200; i++) {
    assert.equal(axis.start(i), at, 'start ' + i)
    const size = axis.sizeOf(i)
    if (size > 0) {
      assert.equal(axis.indexAt(at), i, 'indexAt start of ' + i)
      assert.equal(axis.indexAt(at + size - 1), i, 'indexAt end of ' + i)
    }
    at += size
  }
  assert.equal(axis.indexAt(axis.start(5)), 8, 'a position on a hidden span lands past it')
  assert.equal(axis.styleOf(10), 3)
  assert.equal(axis.firstVisible(5, 1), 8)
  assert.equal(axis.firstVisible(7, -1), 4)
  assert.equal(axis.firstVisible(1000000, 1), -1)
  assert.equal(axis.advance(4, 1), 8)
  assert.equal(axis.advance(999990, 50), 999999, 'stops at the last shown index')
  assert.equal(axis.total, axis.start(1048576))
  assert.equal(axis.indexAt(1e12), 999999)
})

await test('merges: lookup and expansion to a fixed point', () => {
  const merges = new Merges(['B2:C3', 'C4:D70', 'F1:F1'])
  assert.deepEqual(merges.at(2, 2), { r0: 1, c0: 1, r1: 2, c1: 2 })
  assert.equal(merges.at(0, 0), null)
  assert.deepEqual(merges.at(69, 3), { r0: 3, c0: 2, r1: 69, c1: 3 })
  // B2 touches B2:C3, which touches nothing below; C3:C4 chains to C4:D70.
  assert.deepEqual(merges.expand({ r0: 1, c0: 1, r1: 1, c1: 1 }), { r0: 1, c0: 1, r1: 2, c1: 2 })
  assert.deepEqual(merges.expand({ r0: 2, c0: 2, r1: 3, c1: 2 }), { r0: 1, c0: 1, r1: 69, c1: 3 })
})

const geometry = new Geometry({
  defaults: { columnWidth: 8.43, rowHeight: 15, maxDigitWidth: 7 },
  columns: [[5, 6, null, true, 4]],
  rows: [[7, 9, null, true, null]],
  merges: ['D1:E1', 'B22:D24'],
  frozen: { rows: 1, columns: 0 },
  dimension: 'A1:J501'
})

await test('selection: arrows step over hidden lines and merges', () => {
  const s = new Selection()
  s.bind(geometry)
  assert.deepEqual(s.active, { row: 1, column: 0 }, 'opens past the frozen row')
  s.select(6, 0)
  s.move(1, 0)
  assert.deepEqual(s.active, { row: 10, column: 0 }, 'rows 8-10 hidden')
  s.select(0, 2)
  s.move(0, 1)
  assert.deepEqual(s.active, { row: 0, column: 3 }, 'into the merge: its anchor')
  assert.equal(s.refs[0], 'D1:E1')
  s.move(0, 1)
  assert.deepEqual(s.active, { row: 0, column: 7 }, 'past the merge and hidden F:G')
})

await test('selection: Shift+arrows grow past a merge; Tab and Enter walk', () => {
  const s = new Selection()
  s.bind(geometry)
  s.select(20, 0)
  s.extendBy(1, 0)
  s.extendBy(0, 1)
  assert.equal(s.refs[0], 'A21:D24', 'B22:D24 joined whole')
  s.extendBy(0, 1)
  assert.equal(s.refs[0], 'A21:E24')
  s.extendBy(0, -1)
  assert.equal(s.refs[0], 'A21:D24')
  s.select(1, 0)
  s.walk(0, 1)
  s.walk(0, 1)
  s.walk(1, 0)
  assert.deepEqual(s.active, { row: 2, column: 0 }, 'Enter after Tab returns to the first column')
  s.selectRange(parseRange('A2:B3'))
  s.walk(0, 1)
  s.walk(0, 1)
  assert.deepEqual(s.active, { row: 2, column: 0 })
  s.walk(-1, 0)
  assert.deepEqual(s.active, { row: 1, column: 0 }, 'Shift+Enter moves up')
  s.walk(-1, 0)
  assert.deepEqual(s.active, { row: 2, column: 1 }, 'and wraps to the end of the selection')
  assert.equal(s.refs[0], 'A2:B3', 'the selection stays')
})

await test('selection: multi-range and whole lines', () => {
  const s = new Selection()
  s.bind(geometry)
  s.select(4, 1)
  s.extendTo(11, 2)
  s.add(13, 7)
  assert.deepEqual(s.refs, ['B5:C12', 'H14'])
  assert.equal(s.isSelected(5, 2), true)
  assert.equal(s.isSelected(12, 2), false)
  s.selectColumns(9, 9, {})
  assert.deepEqual(s.refs, ['J1:J1048576'])
  // Column C crosses B22:D24, which reaches D1:E1: whole merges, as Excel.
  s.selectColumns(2, 2, {})
  assert.deepEqual(s.refs, ['B1:E1048576'])
  s.selectRows(4, 6, { add: true })
  assert.deepEqual(s.refs, ['B1:E1048576', 'A5:XFD7'])
  s.selectAll()
  assert.deepEqual(s.refs, ['A1:XFD1048576'])
})

// A fake service: counts requests in flight, answers 304 to a matching tag.
const fakeApi = () => {
  const api = { inFlight: 0, peak: 0, calls: [], release: [], closed: new Set() }
  api.isOpen = (key) => !api.closed.has(key)
  api.request = (method, path, { etag, signal } = {}) => {
    api.calls.push({ path, etag })
    api.inFlight++
    api.peak = Math.max(api.peak, api.inFlight)
    return new Promise((resolve, reject) => {
      const done = () => {
        api.inFlight--
        const [, , , tr, tc] = path.split('/')
        const tag = `"t1.1.${tr}.${tc}.1"`
        if (etag === tag) resolve({ status: 304, etag: tag })
        else resolve({ status: 200, etag: tag, data: { cells: [[0, 0, `${tr}/${tc}`, 0, 0]], merges: [] } })
      }
      api.release.push(done)
      if (signal) signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })))
    })
  }
  api.flush = async () => {
    while (api.release.length) {
      api.release.shift()()
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
  }
  return api
}

await test('tiles: at most six in flight, shown before ring, then held', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  const shown = [[0, 0], [0, 1], [1, 0], [1, 1]]
  const ring = [[2, 0], [2, 1], [0, 2], [1, 2], [2, 2]]
  tiles.want(1, shown, ring)
  assert.equal(api.inFlight, IN_FLIGHT)
  assert.deepEqual(api.calls.slice(0, 4).map((c) => c.path), shown.map(([r, c]) => `sheets/1/tiles/${r}/${c}`))
  await api.flush()
  assert.equal(api.peak, IN_FLIGHT)
  assert.equal(api.calls.length, 9)
  assert.equal(tiles.cell(1, 64, 32).text, '1/1')
  assert.equal(tiles.cell(1, 65, 32), null, 'a held tile answers null for an empty cell')
  assert.equal(tiles.cell(1, 64 * 9, 0), undefined, 'an absent tile answers undefined')
  assert.equal(tiles.pending(1, shown), 0)
  tiles.want(1, shown, ring)
  assert.equal(api.calls.length, 9, 'held tiles are not fetched again')
})

await test('tiles: a stale tile revalidates with If-None-Match and keeps its cells on 304', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  tiles.want(1, [[0, 0]], [])
  await api.flush()
  tiles.invalidate(1, { r0: 0, c0: 0, r1: 0, c1: 0 })
  assert.equal(tiles.pending(1, [[0, 0]]), 1)
  tiles.want(1, [], [[0, 0]])
  assert.equal(api.calls.length, 1, 'a stale ring tile waits until shown')
  tiles.want(1, [[0, 0]], [])
  assert.equal(api.calls[1].etag, '"t1.1.0.0.1"')
  await api.flush()
  assert.equal(tiles.pending(1, [[0, 0]]), 0)
  assert.equal(tiles.cell(1, 0, 0).text, '0/0')
})

await test('tiles: the LRU keeps at most CAPACITY, never evicting a shown tile', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  for (let tr = 0; tr < CAPACITY + 40; tr++) {
    tiles.want(1, [[tr, 0]], [])
    await api.flush()
  }
  assert.equal(tiles.entries.size, CAPACITY)
  assert.ok(tiles.entry(1, CAPACITY + 39, 0), 'the newest is held')
  assert.equal(tiles.entry(1, 0, 0), null, 'the oldest went first')
})

await test('tiles: reset drops a generation, in-flight answers included', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  tiles.want(1, [[0, 0]], [])
  tiles.reset(2)
  await api.flush()
  assert.equal(tiles.entries.size, 0)
})

await test('geometry: a sheet stating no width gets Excel\'s 64 px column', () => {
  const g = new Geometry({ defaults: { rowHeight: 15, maxDigitWidth: 7 } })
  assert.equal(g.columns.size, 64)
  assert.equal(new Geometry({ defaults: { columnWidth: 9.140625, maxDigitWidth: 7 } }).columns.size, 64)
  assert.equal(columnWidthPx(DEFAULT_COLUMN_WIDTH, 7), 64)
})

await test('render: a `*x` fill goes in at the offset the service names', () => {
  assert.equal(fillText(' $1,234.50 ', ' ', 2, 3), ' $   1,234.50 ')
  assert.equal(fillText('Total', '.', 5, 4), 'Total....')
  assert.equal(fillText('abc', '-', 99, 2), 'abc--', 'an offset past the end clamps to it')
  assert.equal(fillText('abc', '-', -1, 2), '--abc')
  assert.equal(fillText('abc', '-', 1, 0), 'abc')
})

await test('tiles: settled while none over the ranges is in flight or stale and shown', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  const a1 = [{ r0: 0, c0: 0, r1: 0, c1: 0 }]
  tiles.want(1, [[0, 0]], [])
  assert.equal(tiles.settled(1, a1), false, 'the first fetch is in flight')
  assert.equal(tiles.settled(1, [{ r0: 100, c0: 0, r1: 100, c1: 0 }]), true, 'a range under no tile')
  await api.flush()
  assert.equal(tiles.settled(1, a1), true)
  tiles.invalidate(1, a1[0])
  assert.equal(tiles.settled(1, a1), false, 'stale and shown')
  tiles.want(1, [[1, 0]], [])
  assert.equal(tiles.settled(1, a1), true, 'stale but no longer shown: left for later')
  tiles.want(1, [[0, 0]], [])
  assert.equal(tiles.settled(1, a1), false, 'revalidating')
  await api.flush()
  assert.equal(tiles.settled(1, a1), true)
  assert.equal(tiles.settled(2, a1), true, 'another sheet')
})

await test('selection: relayout grows ranges into new merges and moves the active cell to the anchor', () => {
  const s = new Selection()
  s.bind(geometry)
  s.select(40, 3)
  s.extendTo(40, 4)
  const merged = new Geometry({ defaults: {}, merges: ['C40:E42'] })
  s.relayout(merged)
  assert.deepEqual(s.active, { row: 39, column: 2 })
  assert.deepEqual(s.refs, ['C40:E42'])
})

await test('api: edits go one at a time, each carrying the revision the one before answered', async () => {
  const sent = []
  let revision = 42
  globalThis.fetch = async (url, init) => {
    const body = JSON.parse(init.body)
    sent.push({ path: new URL(url).pathname, body, token: init.headers.get('X-Yggdryl-Token'), type: init.headers.get('Content-Type') })
    await new Promise((resolve) => setTimeout(resolve, 5))
    if (body.edit && body.edit.stale) {
      return new Response(JSON.stringify({ status: 409, kind: 'stale_base', detail: 'stale', revision }), { status: 409, headers: { 'Content-Type': 'application/problem+json' } })
    }
    revision += 1
    return new Response(JSON.stringify({ revision, changed: [], structural: false, sheets: false, styles: false }), { status: 200, headers: { 'Content-Type': 'application/json' } })
  }
  const api = new Api(new URL('http://h/api/'), 'tok')
  api.adopt({ generation: 3, revision: 42 })
  const first = api.edit({ op: 'setEntries' })
  const second = api.edit({ op: 'clear' })
  const answers = await Promise.all([first, second])
  assert.deepEqual(sent.map((item) => item.body.base), [42, 43])
  assert.deepEqual(answers.map((answer) => answer.base), [42, 43])
  assert.equal(api.revision, 44)
  assert.equal(sent[0].path, '/api/edits')
  assert.equal(sent[0].token, 'tok')
  assert.equal(sent[0].type, 'application/json')
  const refused = await api.edit({ stale: true }).catch((error) => error)
  assert.ok(refused instanceof ApiError)
  assert.equal(refused.kind, 'stale_base')
  assert.equal(refused.base, 44, 'a refusal carries its base')
  assert.equal(api.revision, 44, 'a refusal moves no revision')
  const undo = await api.history('undo')
  assert.equal(sent[sent.length - 1].path, '/api/undo')
  assert.deepEqual(sent[sent.length - 1].body, { base: 44 })
  assert.equal(undo.revision, 45)
  api.adopt({ revision: 40 })
  assert.equal(api.revision, 45, 'a revision never goes back within a generation')
  api.adopt({ generation: 4, revision: 1 })
  assert.equal(api.revision, 1, 'a new generation starts over')
})


// P3 --------------------------------------------------------------------------

const { numberCode, classifyCode, charactersOfWidth, widthOfCharacters, DATE_TYPES, TIME_TYPES, FRACTION_TYPES } = await import('/home/user/yggdryl/cli/assets/excel/js/dialogs.js')
const { markerOf, textExtent } = await import('/home/user/yggdryl/cli/assets/excel/js/clipboard.js')
const { zoomOfSlider, sliderOfZoom } = await import('/home/user/yggdryl/cli/assets/excel/js/status.js')
const { Sync } = await import('/home/user/yggdryl/cli/assets/excel/js/sync.js')
const { FORMATS } = await import('/home/user/yggdryl/cli/assets/excel/js/ribbon.js')

await test('number codes: the dialog spells what the ribbon spells, and reads every code back', () => {
  assert.equal(numberCode('accounting', { decimals: 2, symbol: '$' }), FORMATS.accounting)
  assert.equal(numberCode('accounting', { decimals: 2, symbol: 'none' }), FORMATS.comma)
  assert.equal(numberCode('currency', { decimals: 2, symbol: '$' }), FORMATS.currency)
  assert.equal(numberCode('percentage', { decimals: 2 }), FORMATS.percentage)
  assert.equal(numberCode('percentage', { decimals: 0 }), FORMATS.percent)
  assert.equal(numberCode('scientific', { decimals: 2 }), FORMATS.scientific)
  assert.equal(numberCode('number', { decimals: 2 }), FORMATS.number)
  assert.equal(numberCode('number', { decimals: 3, separator: true, negative: 3 }), '#,##0.000_);[Red](#,##0.000)')
  assert.equal(numberCode('accounting', { decimals: 0, symbol: '$' }), '_("$"* #,##0_);_("$"* \\(#,##0\\);_("$"* "-"_);_(@_)')
  for (const [key] of [...DATE_TYPES, ...TIME_TYPES, ...FRACTION_TYPES]) assert.notEqual(classifyCode(key).category, 'custom', key)
  assert.deepEqual(classifyCode('#,##0.000_);[Red](#,##0.000)'), { category: 'number', options: { decimals: 3, separator: true, negative: 3 } })
  assert.equal(classifyCode(FORMATS.longDate).category, 'date')
  assert.equal(classifyCode(FORMATS.time).category, 'time')
  assert.equal(classifyCode('@').category, 'text')
  assert.equal(classifyCode('General').category, 'general')
  assert.deepEqual(classifyCode('0.0"kg"'), { category: 'custom', options: { code: '0.0"kg"' } })
  // A category spells the same code its choices were read from.
  for (const code of ['0', '#,##0.00', '"€"#,##0.00_);("€"#,##0.00)', '0.000%', '0E+00', FORMATS.accounting]) {
    const known = classifyCode(code)
    assert.equal(numberCode(known.category, known.options), code)
  }
})

await test('widths: a dialog\'s characters and the file\'s <col width> (addendum 2)', () => {
  assert.equal(charactersOfWidth(9.140625, 7), 8.43)
  assert.equal(widthOfCharacters(8.43, 7), 9.140625)
  assert.equal(widthOfCharacters(12, 7), 12.7109375)
  assert.equal(charactersOfWidth(widthOfCharacters(12, 7), 7), 12)
  assert.equal(widthOfCharacters(0, 7), 0)
  for (const characters of [1, 10, 20.71, 100, 255]) {
    assert.equal(charactersOfWidth(widthOfCharacters(characters, 7), 7), characters, String(characters))
  }
  // A width lands on whole pixels, as Excel's does: 2.5 characters reads back as 2.43.
  assert.equal(charactersOfWidth(widthOfCharacters(2.5, 7), 7), 2.43)
})

const INSTANCE = '0190a6e4-7b3c-7d2e-9f10-123456789abc'

await test('clipboard: the export marker names the service instance (addendum 8); the extent of pasted text', () => {
  assert.deepEqual(markerOf(`<table data-yggdryl-copy="${INSTANCE}:3:42:1:A1:C3"><tr></tr></table>`),
    { raw: `${INSTANCE}:3:42:1:A1:C3`, instance: INSTANCE, generation: 3, revision: 42, sheet: 1, range: 'A1:C3' })
  assert.equal(markerOf(`<table data-yggdryl-copy="${INSTANCE}:3:42:1:B7">`).range, 'B7')
  assert.equal(markerOf('<table data-yggdryl-copy="3:42:1:A1:C3">'), null, 'a marker naming no instance is not ours')
  assert.equal(markerOf('<table data-yggdryl-copy="3:42:1:B7">'), null)
  assert.equal(markerOf(`<table data-yggdryl-copy="${INSTANCE}:x:42:1:A1">`), null)
  assert.equal(markerOf(`<table data-yggdryl-copy="${INSTANCE}:3:42:1:nothing">`), null)
  assert.equal(markerOf('<table data-yggdryl-copy="a b:3:42:1:A1">'), null)
  assert.equal(markerOf('<table>'), null)
  assert.equal(markerOf(''), null)
  assert.deepEqual(textExtent('1\t2\t3\r\n4\r\n'), { rows: 2, columns: 3 })
  assert.deepEqual(textExtent('x'), { rows: 1, columns: 1 })
})

await test('api: another service instance starts over, as a new generation does', () => {
  const api = new Api(new URL('http://h/api/'), 'tok')
  api.adopt({ instance: INSTANCE, generation: 3, revision: 42 })
  assert.equal(api.instance, INSTANCE)
  api.adopt({ revision: 45 })
  api.adopt({ instance: INSTANCE, generation: 3, revision: 44 })
  assert.equal(api.revision, 45, 'the same instance and generation never go back')
  api.adopt({ instance: 'another', generation: 3, revision: 2 })
  assert.deepEqual([api.instance, api.generation, api.revision], ['another', 3, 2], 'a restarted service at the same generation')
})

await test('status: the zoom slider has 100% in its middle, 25% and 400% at its ends', () => {
  assert.equal(zoomOfSlider(50), 1)
  assert.equal(zoomOfSlider(0), 0.25)
  assert.equal(zoomOfSlider(100), 4)
  assert.equal(zoomOfSlider(-5), 0.25)
  for (const zoom of [0.25, 0.5, 0.75, 1, 1.5, 2, 4]) assert.equal(zoomOfSlider(sliderOfZoom(zoom)), zoom, String(zoom))
})

const feedApi = () => {
  const api = { base: new URL('http://127.0.0.1:1/api/'), revision: 10, generation: 3, asked: [] }
  api.adopt = ({ revision }) => { api.revision = Math.max(api.revision, revision) }
  api.get = async (path) => {
    api.asked.push(path)
    return { revision: 14, changes: [{ revision: 11, changed: [] }, { revision: 12, changed: [] }, { revision: 13, changed: [] }, { revision: 14, changed: [] }] }
  }
  return api
}

await test('sync: a relayed answer applies past this tab\'s revision; a gap is read once; resets and other workbooks', async () => {
  const api = feedApi()
  const applied = []
  let resets = 0
  const sync = new Sync({
    api,
    apply: async (answer, from) => {
      applied.push([from, answer.changes.filter((c) => c.revision > from).map((c) => c.revision)])
      api.adopt(answer)
    },
    reset: async () => { resets++ }
  })
  const base = String(api.base)
  const relay = (since, revision, extra = {}) => sync.receive({ base, generation: 3, since, revision, changes: Array.from({ length: revision - since }, (_, i) => ({ revision: since + i + 1, changed: [] })), ...extra })
  await relay(9, 11)
  assert.deepEqual(applied.pop(), [10, [11]], 'only past the revision held')
  await relay(10, 11)
  assert.equal(applied.length, 0, 'nothing new: nothing applied')
  await relay(12, 14)
  assert.deepEqual(api.asked, ['changes?since=11&wait=0'], 'a gap is read from this tab\'s revision')
  assert.deepEqual(applied.pop(), [11, [12, 13, 14]])
  await sync.receive({ base: 'http://elsewhere/api/', generation: 3, since: 14, revision: 20, changes: [] })
  assert.equal(api.revision, 14, 'another workbook\'s feed is not ours')
  await sync.receive({ base, generation: 2, since: 14, revision: 30, reset: true })
  assert.equal(resets, 0, 'a leader on an older generation says nothing new')
  await sync.receive({ base, generation: 4, since: 1, revision: 2, changes: [] })
  assert.equal(resets, 1, 'a later generation means a reset was missed')
  await sync.receive({ base, generation: 3, since: 14, revision: 1, reset: true })
  assert.equal(resets, 2)
})

await test('sync: the leader polls from the revision it holds, and afresh after a reset to another generation', async () => {
  globalThis.document = globalThis.document || { documentElement: { dataset: {} } }
  const api = { base: new URL('http://127.0.0.1:1/api/'), revision: 10, generation: 3, asked: [] }
  api.adopt = ({ revision }) => { api.revision = Math.max(api.revision, revision) }
  const answers = [
    { revision: 12, changes: [{ revision: 11, changed: [] }, { revision: 12, changed: [] }] },
    { revision: 40, reset: true },
    { revision: 3, changes: [{ revision: 3, changed: [] }] }
  ]
  let sync = null
  api.get = async (path) => {
    api.asked.push(path)
    const answer = answers.shift()
    if (answers.length === 0) sync.stopped = true
    return answer
  }
  sync = new Sync({ api, apply: async (answer) => api.adopt(answer), reset: async () => { api.generation = 4; api.revision = 2 } })
  await sync.lead('alone')
  assert.deepEqual(api.asked, ['changes?since=10&wait=25', 'changes?since=12&wait=25', 'changes?since=2&wait=25'],
    'a new generation polls from its own revision, never the old one')
})

await test('tiles: an invalidation while a tile is on its way lands it stale', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  tiles.want(1, [[0, 0]], [])
  assert.equal(api.inFlight, 1)
  tiles.invalidate(1, { r0: 0, c0: 0, r1: 0, c1: 0 })
  await api.flush()
  assert.equal(tiles.pending(1, [[0, 0]]), 1, 'the answer may predate the change')
  assert.equal(tiles.settled(1, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]), false)
  tiles.want(1, [[0, 0]], [])
  assert.equal(api.calls.length, 2, 'the next frame asks again')
  await api.flush()
  assert.equal(tiles.pending(1, [[0, 0]]), 0)
})

await test('tiles: holds nest; the last release sends', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  tiles.hold(true)
  tiles.hold(true)
  tiles.want(1, [[0, 0]], [])
  tiles.hold(false)
  assert.equal(api.calls.length, 0)
  tiles.hold(false)
  assert.equal(api.calls.length, 1)
  await api.flush()
})

await test('tiles: a sheet the gate has shut is not asked for', async () => {
  const api = fakeApi()
  const tiles = new Tiles(api)
  tiles.reset(1)
  api.closed.add(1)
  tiles.want(1, [[0, 0], [0, 1]], [[1, 0]])
  assert.equal(api.calls.length, 0, 'nothing for a shut sheet')
  tiles.want(2, [[0, 0]], [])
  assert.deepEqual(api.calls.map((call) => call.path), ['sheets/2/tiles/0/0'])
  await api.flush()
})

await test('api: the sheet gate - only listed sheets, a removal waits for what is on its way, then cancels, answers after it dropped', async () => {
  const sent = []
  const pending = []
  globalThis.fetch = (url, init) => {
    sent.push(new URL(url).pathname)
    return new Promise((resolve, reject) => {
      pending.push(() => resolve(new Response('{"cells":[]}', { status: 200, headers: { 'Content-Type': 'application/json' } })))
      if (init.signal) init.signal.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
    })
  }
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
  const api = new Api(new URL('http://h/api/'), 'tok')
  const rejected = (promise) => promise.then(() => 'answered', (error) => error.name)
  assert.equal(api.isOpen(9), true, 'before a document, every sheet')
  api.setSheets([{ key: 1 }, { key: 2 }])
  assert.equal(await rejected(api.get('sheets/9/tiles/0/0')), 'AbortError', 'a sheet the document does not list')
  assert.deepEqual(sent, [], 'is never sent')
  // A removal waits for sheet 1's request, which lands and is dropped.
  const first = rejected(api.get('sheets/1/tiles/0/0', { quiet: true }))
  let closed = false
  const closing = api.closeSheets([1]).then(() => { closed = true })
  await settle()
  assert.equal(api.isOpen(1), false)
  assert.equal(await rejected(api.get('sheets/1/cells/A1')), 'AbortError', 'nothing new while it closes')
  assert.equal(closed, false, 'waits for the one on its way')
  pending.shift()()
  await closing
  assert.equal(await first, 'AbortError', 'an answer landing after the sheet shut is dropped')
  assert.deepEqual(sent, ['/api/sheets/1/tiles/0/0'])
  // Refused: the sheet opens again.
  api.openSheets([1])
  assert.equal(api.isOpen(1), true)
  // Past its limit a removal cancels what is still on its way.
  const slow = rejected(api.get('sheets/2/layout'))
  await api.closeSheets([2], 20)
  assert.equal(await slow, 'AbortError')
  // The next document drops sheet 2; opening it after the edit keeps it shut.
  api.setSheets([{ key: 1 }])
  api.openSheets([2])
  assert.equal(api.isOpen(2), false)
  // What names no sheet goes out whatever the gate says.
  pending.length = 0
  const other = rejected(api.get('workbook'))
  await settle()
  pending.shift()()
  assert.equal(await other, 'answered')
})


// P4 --------------------------------------------------------------------------

const { fillTarget, Viewport } = await import('/home/user/yggdryl/cli/assets/excel/js/geometry.js')
const { movePosition, dropBefore } = await import('/home/user/yggdryl/cli/assets/excel/js/sheets.js')
const { sortTarget } = await import('/home/user/yggdryl/cli/assets/excel/js/ribbon.js')

await test('fill handle: the target grows along the axis the pointer left farther on; back inside it clears', () => {
  const source = parseRange('B2:C3')
  const t = (ref) => {
    const at = parseRef(ref)
    const { target, clear } = fillTarget(source, at.row, at.column)
    return [rangeName(target), clear && rangeName(clear)]
  }
  assert.deepEqual(t('C7'), ['B2:C7', null], 'down')
  assert.deepEqual(t('B1'), ['B1:C3', null], 'up')
  assert.deepEqual(t('F3'), ['B2:F3', null], 'right')
  assert.deepEqual(t('A2'), ['A2:C3', null], 'left')
  assert.deepEqual(t('E9'), ['B2:C9', null], 'six rows down beats two columns right')
  assert.deepEqual(t('H4'), ['B2:H3', null], 'five columns right beats one row down')
  assert.deepEqual(t('D4'), ['B2:C4', null], 'a tie goes down')
  assert.deepEqual(t('C3'), ['B2:C3', null], 'the corner itself: nothing')
  assert.deepEqual(t('C2'), ['B2:C2', 'B3:C3'], 'back up a row clears it')
  assert.deepEqual(t('B3'), ['B2:B3', 'C2:C3'], 'back a column clears it')
  const tall = parseRange('A1:A10')
  assert.deepEqual(fillTarget(tall, 3, 0), { target: parseRange('A1:A4'), clear: parseRange('A5:A10') })
})

await test('fill handle: the corner is where the range ends in the frame, across frozen panes, or nowhere', () => {
  const frozen = new Geometry({
    defaults: { columnWidth: DEFAULT_COLUMN_WIDTH, rowHeight: 15, maxDigitWidth: 7 },
    columns: [], rows: [], merges: [], frozen: { rows: 1, columns: 1 }, dimension: 'A1'
  }, 1)
  const header = { width: 30, height: 20 }
  const view = new Viewport(frozen, { topRow: 10, rowOffset: 0, leftColumn: 5, columnOffset: 0 }, 800, 500, header)
  // B11:C12 in the scrolled pane: its corner is C12's bottom-right.
  const box = view.rect(parseRange('C12'))
  assert.deepEqual(view.corner(parseRange('F11:G12')), { x: view.x(6) + 64, y: view.y(11) + 20 })
  assert.equal(view.corner(parseRange('B2:C3')), null, 'scrolled away above and left')
  assert.deepEqual(view.corner(parseRange('A1')), { x: 30 + 64, y: 20 + 20 }, 'the frozen corner')
  assert.equal(view.corner(parseRange('F11:G5000')), null, 'below the frame')
  assert.ok(box.w === 64 && box.h === 20)
})

await test('sheets: a move lands before a sheet or at the end, hidden sheets counted', () => {
  const sheets = [
    { key: 1, state: 'visible' }, { key: 2, state: 'visible' }, { key: 9, state: 'hidden' },
    { key: 3, state: 'visible' }, { key: 4, state: 'visible' }
  ]
  assert.equal(movePosition(sheets, 1, 3), 2, 'before 3: past the hidden sheet')
  assert.equal(movePosition(sheets, 4, 1), 0)
  assert.equal(movePosition(sheets, 1, null), 4, 'the end')
  assert.equal(movePosition(sheets, 1, 2), 0, 'before the next one: where it is')
  assert.equal(movePosition(sheets, 3, 4), 3, 'already there')
  const boxes = [{ key: 1, left: 0, right: 60 }, { key: 2, left: 60, right: 100 }, { key: 3, left: 100, right: 200 }]
  assert.equal(dropBefore(boxes, 10), 1)
  assert.equal(dropBefore(boxes, 31), 2, 'past the middle of the first tab')
  assert.equal(dropBefore(boxes, 149), 3)
  assert.equal(dropBefore(boxes, 151), null, 'past the last middle: the end')
})

await test('sort: the region of one cell, a header guessed from text over a value, nothing for one row', async () => {
  const selection = new Selection()
  selection.bind(geometry)
  selection.select(4, 1)
  const cells = new Map([['0,1', { kind: 0, text: 'Price' }], ['1,1', { kind: 1, text: '3' }]])
  const tiles = { cell: (sheet, r, c) => cells.get(r + ',' + c) ?? null }
  const api = { get: async (path) => (assert.match(path, /^sheets\/7\/edge\?from=B5&direction=region$/), { range: 'A1:C9' }) }
  const found = await sortTarget({ api, grid: {}, selection, tiles, sheet: 7 })
  assert.deepEqual([rangeName(found.range), found.header], ['A1:C9', true])
  cells.set('1,1', { kind: 0, text: 'x' })
  assert.equal((await sortTarget({ api, grid: {}, selection, tiles, sheet: 7 })).header, false, 'text over text is data')
  const flat = { get: async () => ({ range: 'A5:C5' }) }
  assert.equal(await sortTarget({ api: flat, grid: {}, selection, tiles, sheet: 7 }), null)
})


// P5 --------------------------------------------------------------------------

const {
  inString, canPoint, wordAt, callAt, parameters, parameterAt, tokenColors, shiftTokens, pointRange
} = await import('/home/user/yggdryl/cli/assets/excel/js/editor.js')
const { searchFunctions } = await import('/home/user/yggdryl/cli/assets/excel/js/dialogs.js')

await test('assist: point mode after an operator, a separator or a parenthesis, never inside a string', () => {
  for (const [text, at] of [['=', 1], ['=SUM(', 5], ['=A1+', 4], ['=A1+ ', 5], ['=SUM(A1,', 8], ['=A1<', 4], ['=A1<>', 5], ['=2^', 3], ['="a"&', 5]]) {
    assert.equal(canPoint(text, at), true, `${text} at ${at}`)
  }
  for (const [text, at] of [['abc', 3], ['=A1', 3], ['=SUM(A1', 7], ['=SUM(A1)', 8], ['="(', 3], ['=1', 0], ['+A1+', 4]]) {
    assert.equal(canPoint(text, at), false, `${text} at ${at}`)
  }
  assert.equal(inString('="a""b', 5), true)
  assert.equal(inString('="a""b"', 7), false)
})

await test('assist: the name being typed, and nothing mid-word, after a sheet or inside a string', () => {
  assert.deepEqual(wordAt('=SU', 3), { start: 1, word: 'SU' })
  assert.deepEqual(wordAt('=A1+va', 6), { start: 4, word: 'va' })
  assert.deepEqual(wordAt('=SUM(A1, st', 11), { start: 9, word: 'st' })
  assert.deepEqual(wordAt('=MODE.S', 7), { start: 1, word: 'MODE.S' })
  for (const [text, at] of [['SU', 2], ['=SU', 2], ['=SUM(', 5], ['=Data!A', 7], ['="su', 4], ['=1SU', 4], ['=$A', 3], ['=SUM(', 4], ['=A1:B', 5]]) {
    assert.equal(wordAt(text, at), null, `${text} at ${at}`)
  }
})

await test('assist: the call the caret is in and its argument, past strings, arrays and quoted sheets', () => {
  assert.deepEqual(callAt('=SUM(', 5), { name: 'SUM', index: 0 })
  assert.deepEqual(callAt('=SUM(A1,B', 9), { name: 'SUM', index: 1 })
  assert.deepEqual(callAt('=IF(A1>0,ROUND(B1,', 18), { name: 'ROUND', index: 1 })
  assert.deepEqual(callAt('=IF(A1>0,ROUND(B1,2),', 21), { name: 'IF', index: 2 })
  assert.deepEqual(callAt('=SUM((A1,B1),', 13), { name: 'SUM', index: 1 }, 'a bare parenthesis is not a call')
  assert.deepEqual(callAt('=CONCAT("a,(b",', 15), { name: 'CONCAT', index: 1 })
  assert.deepEqual(callAt('=SUM({1,2;3,4},', 15), { name: 'SUM', index: 1 })
  assert.deepEqual(callAt("=SUM('a,(b'!A1,", 15), { name: 'SUM', index: 1 })
  assert.deepEqual(callAt('=_xlfn.CONCAT(', 14), { name: 'CONCAT', index: 0 })
  assert.equal(callAt('=SUM(A1)', 8), null)
  assert.equal(callAt('SUM(', 4), null)
})

await test('assist: a signature\'s parameters and the one an argument fills', () => {
  const sum = parameters('SUM(number1, [number2], ...)')
  assert.deepEqual(sum, ['number1', '[number2]', '...'])
  assert.deepEqual([0, 1, 2, 7].map((i) => parameterAt(sum, i)), [0, 1, 1, 1])
  const ifs = parameters('SUMIFS(sum_range, criteria_range1, criteria1, [criteria_range2, criteria2], ...)')
  assert.deepEqual(ifs, ['sum_range', 'criteria_range1', 'criteria1', '[criteria_range2, criteria2]', '...'])
  assert.deepEqual([0, 2, 3, 4, 5, 9].map((i) => parameterAt(ifs, i)), [0, 2, 3, 3, 3, 3])
  const round = parameters('ROUND(number, num_digits)')
  assert.deepEqual([0, 1, 2].map((i) => parameterAt(round, i)), [0, 1, -1])
  assert.deepEqual(parameters('PI()'), [])
  assert.equal(parameterAt([], 0), -1)
})

await test('assist: one colour per distinct range, in order, cycling; tokens carried over an edit', () => {
  const tokens = [
    { start: 1, end: 3, sheet: 1, range: 'A1:A1' },
    { start: 4, end: 9, sheet: 1, range: 'B1:C2' },
    { start: 10, end: 12, sheet: 1, range: 'A1' },
    { start: 13, end: 22, sheet: 3, range: 'A1:A1' }
  ]
  assert.deepEqual(tokenColors(tokens), [0, 1, 0, 2])
  const many = Array.from({ length: 9 }, (_, i) => ({ sheet: 1, range: `A${i + 1}` }))
  assert.deepEqual(tokenColors(many), [0, 1, 2, 3, 4, 5, 6, 0, 1])
  // '=A1+B1:C2' becomes '=A1*10+B1:C2': A1 stays, B1:C2 moves by three.
  const before = '=A1+B1:C2'
  const after = '=A1*10+B1:C2'
  assert.deepEqual(shiftTokens([{ start: 1, end: 3 }, { start: 4, end: 9 }], before, after), [{ start: 1, end: 3 }, { start: 7, end: 12 }])
  // A reference the edit touched or grew goes until the answer comes; an
  // operator typed after one keeps it.
  assert.deepEqual(shiftTokens([{ start: 1, end: 3 }], '=A1', '=A12'), [])
  assert.deepEqual(shiftTokens([{ start: 1, end: 3 }], '=A1', '=A1+'), [{ start: 1, end: 3 }])
  assert.deepEqual(shiftTokens([{ start: 1, end: 3 }], '=A1', '=A'), [])
  assert.deepEqual(shiftTokens([{ start: 2, end: 4 }], '=+A1', '=5+A1'), [{ start: 3, end: 5 }])
  assert.deepEqual(shiftTokens([{ start: 2, end: 4 }], '=+A1', '=+$A1'), [])
})

await test('assist: a pointer\'s range, from cells, headers or the corner', () => {
  assert.deepEqual(pointRange({ kind: 'cell', row: 4, column: 1 }, { kind: 'cell', row: 2, column: 3 }), { r0: 2, c0: 1, r1: 4, c1: 3 })
  assert.equal(rangeName(pointRange({ kind: 'column', column: 2 }, { kind: 'column', column: 1 })), 'B1:C1048576')
  assert.equal(rangeName(pointRange({ kind: 'row', row: 4 }, { kind: 'row', row: 4 })), 'A5:XFD5')
  assert.equal(rangeName(pointRange({ kind: 'corner' }, { kind: 'corner' })), 'A1:XFD1048576')
})

await test('insert function: a search finds every word in a name or a description, names first', () => {
  const list = [
    { name: 'AVERAGEIF', description: 'Finds the average for the cells a condition names.' },
    { name: 'SUM', description: 'Adds all the numbers.' },
    { name: 'AVERAGE', description: 'Returns the average of its arguments.' },
    { name: 'MEDIAN', description: 'Returns the median of the numbers.' }
  ]
  assert.deepEqual(searchFunctions(list, 'average').map((item) => item.name), ['AVERAGE', 'AVERAGEIF'])
  assert.deepEqual(searchFunctions(list, 'the numbers').map((item) => item.name), ['MEDIAN', 'SUM'])
  assert.deepEqual(searchFunctions(list, '  '), [])
})

console.log(`${passed} passed`)
