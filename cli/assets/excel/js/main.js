// Boot and wiring: the token, the workbook document, the sheet shown (its
// tab strip is sheets.js's), the ribbon frame and its command registry, the
// one edit pipeline every module sends §8.4 ops through, and the change
// feed (sync.js) that brings other tabs' edits here. `app.events` tells the
// modules what happened: `workbook` (a document arrived), `sheet` (another
// sheet is shown), `edited` ({ edit, answer } from this tab), `changed`
// (one change's ranges were invalidated, from here or elsewhere) and
// `reset`.

import { Api, ApiError, takeToken, toast } from './api.js'
import { Tiles } from './tiles.js'
import { Styles } from './styles.js'
import { Selection } from './selection.js'
import { Grid } from './grid.js'
import { Keyboard } from './keyboard.js'
import { Mirror } from './a11y.js'
import { Editor } from './editor.js'
import { Menus } from './menus.js'
import { Ribbon } from './ribbon.js'
import { Dialogs } from './dialogs.js'
import { Status } from './status.js'
import { Clipboard } from './clipboard.js'
import { Sync } from './sync.js'
import { Sheets } from './sheets.js'
import { parseRange } from './geometry.js'

const $ = (id) => document.getElementById(id)

const api = new Api(new URL('../api/', import.meta.url), takeToken())
const tiles = new Tiles(api)
const styles = new Styles(api)
const selection = new Selection()
const grid = new Grid({
  host: $('grid'),
  canvas: $('grid-canvas'),
  vertical: $('vscroll'),
  horizontal: $('hscroll'),
  tiles,
  styles,
  selection
})
const mirror = new Mirror($('grid'), { grid, selection })

const state = {
  workbook: null,
  sheet: null,
  layouts: new Map(), // key -> { etag, layout }
  saved: new Map() // key -> the scroll and selection a sheet had
}

const sheetOf = (key) => state.workbook && state.workbook.sheets.find((sheet) => sheet.key === key)
const shownSheets = () => (state.workbook ? state.workbook.sheets.filter((sheet) => sheet.state === 'visible') : [])

// Commands ---------------------------------------------------------------------

// Every ribbon control and menu item names a command; the phases that
// implement one register it here, and an unregistered name does nothing (a
// menu shows it disabled). A command takes one options object: `{ element,
// value }` from a control, a menu item's `args` otherwise.
const commands = new Map()
// Commands that write into the entry being typed rather than act on cells.
const INTO_ENTRY = new Set(['insertFunction'])

const events = new EventTarget()
const emit = (name, detail) => events.dispatchEvent(new CustomEvent(name, { detail }))

const app = {
  events,
  sheetKey: () => state.sheet,
  sheet: () => sheetOf(state.sheet),
  workbook: () => state.workbook,
  editable: () => Boolean(state.workbook && !state.workbook.readOnly && grid.geometry && grid.sheet === state.sheet),
  switchSheet: (delta) => app.sheets.step(delta),
  showSheet: (key, focus) => showSheet(key, focus),
  focusGrid: () => mirror.focus(),
  gridFocused: () => document.activeElement === mirror.element,
  command: (name, run) => commands.set(name, run),
  has: (name) => commands.has(name),
  run: (name, options) => runCommand(name, options),
  edit: (edit, options) => applyEdit(edit, options),
  send: (request) => settle(request),
  confirm: (message, action) => app.dialogs.confirm(message, action)
}

app.keyboard = new Keyboard({ target: mirror.element, grid, selection, api, app })
app.editor = new Editor({
  app, api, grid, selection, tiles, styles,
  keyboard: app.keyboard,
  host: $('grid'),
  nameBox: $('namebox'),
  formulaBar: $('formula')
})
app.menus = new Menus({ app, grid, selection, keyboard: app.keyboard, tabs: $('sheet-tabs') })
app.ribbon = new Ribbon({
  app, api, grid, selection, tiles, styles,
  menus: app.menus,
  keyboard: app.keyboard,
  panel: $('panel-home')
})
app.dialogs = new Dialogs({ app, api, grid, selection, tiles, styles })
app.sheets = new Sheets({ app, host: $('sheet-tabs'), bar: document.querySelector('.sheetbar') })
app.status = new Status({
  app, api, grid, selection, tiles,
  editor: app.editor,
  menus: app.menus,
  element: $('statusbar')
})
app.clipboard = new Clipboard({ app, api, grid, selection, keyboard: app.keyboard, status: app.status, host: $('grid') })

const runCommand = async (name, options = {}) => {
  const run = commands.get(name)
  if (!run) return
  // A command acts on cells, not on text being typed: the entry commits
  // first, and its edit goes ahead of the command's (Insert Function aside,
  // which writes into it). Undo while typing drops the typing, as Excel's
  // does.
  if (app.editor.isOpen() && !INTO_ENTRY.has(name)) {
    if (name === 'undo') {
      app.editor.cancel()
      return
    }
    app.editor.commit()
  }
  try {
    await run(options)
  } catch (error) {
    // The service's refusals were shown where they were answered.
    if (!(error instanceof ApiError)) toast(String((error && error.message) || error))
  }
}

document.addEventListener('click', (event) => {
  const element = event.target.closest('[data-command]')
  if (!element || element.tagName === 'SELECT' || element.tagName === 'INPUT') return
  if (element.getAttribute('aria-disabled') === 'true') return
  runCommand(element.dataset.command, { element })
  // A pointer click hands the keys back to the sheet, as Excel does; a
  // keyboard activation (detail 0) keeps its place in the ribbon.
  if (event.detail > 0 && !element.hasAttribute('aria-haspopup')) mirror.focus()
})
document.addEventListener('change', (event) => {
  const element = event.target.closest('select[data-command], input[data-command]')
  if (element) runCommand(element.dataset.command, { element, value: element.value })
})

app.command('download', () => api.download('workbook/download', (state.workbook && state.workbook.name) || 'workbook.xlsx'))
app.command('gridlines', ({ element }) => {
  const shown = element.getAttribute('aria-pressed') !== 'true'
  element.setAttribute('aria-pressed', String(shown))
  grid.setGridlines(shown)
})
app.command('undo', () => history('undo'))
app.command('redo', () => history('redo'))

// The change feed starts once the workbook has loaded (boot).
const sync = new Sync({
  api,
  apply: (answer, from) => applyChanges(answer, from),
  reset: () => reloadAll()
})
sync.addEventListener('halted', (event) => {
  const error = event.detail
  if (error.status === 503 || error.kind === 'closed') toast('The workbook service has stopped: changes from other tabs no longer arrive.', 'info')
})
app.sync = sync

// Edits ------------------------------------------------------------------------

// One §8.4 edit: `ranges` are hatched from now until the service has
// answered and their tiles are current (§10: no optimistic values). A 409
// `stale_base` first reads what changed since the base, so the retry sees
// it (§8.6); the refusal then goes back to the caller.
const applyEdit = async (edit, { sheet = state.sheet, ranges = [], quiet = false } = {}) => {
  if (state.workbook && state.workbook.readOnly) {
    const refusal = new ApiError({ status: 403, kind: 'read_only', detail: 'This workbook is open read-only.' })
    if (!quiet) toast(refusal.message, 'info')
    throw refusal
  }
  // A sheet this edit removes is shut first: nothing more is asked of it,
  // and what is on its way lands (or is cancelled) before the edit goes.
  const removed = removedSheets(edit)
  if (removed.length) await api.closeSheets(removed)
  const mark = grid.addPending(sheet, ranges)
  let answer
  try {
    answer = await api.edit(edit)
  } catch (error) {
    api.openSheets(removed)
    grid.dropPending(mark)
    if (error instanceof ApiError && error.kind === 'stale_base') await catchUp(error.base ?? api.revision)
    if (!quiet && error instanceof ApiError) {
      toast(error.kind === 'stale_base' ? 'Another tab changed this sheet first; it now shows that change. Try again.' : error.message)
    }
    throw error
  }
  try {
    await absorb(answer)
  } finally {
    api.openSheets(removed)
  }
  grid.answered(mark)
  emit('edited', { edit, answer })
  return answer
}

// The sheets an edit removes, its batch included.
const removedSheets = (edit) => edit.op === 'removeSheet'
  ? [edit.sheet]
  : edit.op === 'batch' ? (edit.edits || []).flatMap(removedSheets) : []

// A request answered as an edit (`undo`, `redo`, `calculate`): its answer
// absorbed the same way, a stale base caught up first.
const settle = async (request) => {
  let answer
  try {
    answer = await request()
  } catch (error) {
    if (error instanceof ApiError && error.kind === 'stale_base') await catchUp(error.base ?? api.revision)
    if (error instanceof ApiError && error.kind !== 'nothing_to_undo') toast(error.message)
    throw error
  }
  await absorb(answer)
  return answer
}

const history = async (kind) => {
  let answer
  try {
    answer = await settle(() => api.history(kind))
  } catch (error) {
    if (!(error instanceof ApiError)) throw error
    if (error.kind === 'nothing_to_undo') toast(`There is nothing to ${kind}.`, 'info')
    return
  }
  emit('edited', { edit: { op: kind }, answer })
}

// What an answer says changed: the style table grew, the sheet list moved,
// ranges to revalidate (a structural edit, the whole sheet), and the layout
// of the sheet shown. Changes other tabs made since the base come along.
const absorb = async (answer) => {
  if (answer.styles) await styles.load().catch(() => {})
  if (answer.sheets) {
    tiles.hold(true)
    try {
      await reloadWorkbook()
    } finally {
      tiles.hold(false)
    }
  }
  invalidate(answer)
  if (typeof answer.base === 'number' && answer.revision > answer.base + 1) await catchUp(answer.base, true)
  await revalidateLayout()
  grid.invalidate()
  scheduleWorkbook()
}

// A structural change stales its whole sheet; any other, the tiles over its
// ranges.
const invalidate = (change) => {
  const structural = new Set()
  for (const item of change.changed || []) {
    if (change.structural) structural.add(item.sheet)
    else tiles.invalidate(item.sheet, parseRange(item.range))
  }
  for (const key of structural) tiles.invalidate(key)
  emit('changed', change)
}

// Read the change ring from `since` (§8.3 `changes`) and revalidate what it
// names; a reset reloads everything.
const catchUp = async (since, known = false) => {
  let answer
  try {
    answer = await api.get(`changes?since=${since}&wait=0`, { quiet: true })
  } catch {
    tiles.invalidate(state.sheet)
    grid.invalidate()
    return
  }
  if (answer.reset) {
    await reloadAll()
    return
  }
  await applyChanges(answer, since, known)
}

// A `changes` answer: each change past revision `from`, in order - the
// style table, the sheet list (unless `known` already reread it), the
// tiles it names - then the revision it reached, the layout shown, and
// the workbook document a moment later (undo labels, dirty, calc).
const applyChanges = async (answer, from, known = false) => {
  // The revision first, so what reads by it (the formula bar's entry)
  // reads past these changes.
  api.adopt({ revision: answer.revision })
  let any = false
  for (const change of answer.changes || []) {
    if (change.revision <= from) continue
    any = true
    if (change.styles) await styles.load().catch(() => {})
    if (change.sheets && !known) {
      tiles.hold(true)
      try {
        await reloadWorkbook()
      } finally {
        tiles.hold(false)
      }
    }
    invalidate(change)
  }
  if (!any) return
  await revalidateLayout()
  grid.invalidate()
  scheduleWorkbook()
}

// Ribbon tabs ------------------------------------------------------------------

const ribbonTabs = [...document.querySelectorAll('#ribbon-tabs [role="tab"]')]
const selectRibbonTab = (tab, focus = false) => {
  for (const other of ribbonTabs) {
    const selected = other === tab
    other.setAttribute('aria-selected', String(selected))
    other.tabIndex = selected ? 0 : -1
    $(other.getAttribute('aria-controls')).hidden = !selected
  }
  if (focus) tab.focus()
}
for (const tab of ribbonTabs) tab.addEventListener('click', () => selectRibbonTab(tab))
$('ribbon-tabs').addEventListener('keydown', (event) => {
  const at = ribbonTabs.indexOf(document.activeElement)
  if (at < 0) return
  const next = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: ribbonTabs.length - 1 }[event.key]
  if (next === undefined) return
  event.preventDefault()
  selectRibbonTab(ribbonTabs[(next + ribbonTabs.length) % ribbonTabs.length], true)
})

// The sheet shown ----------------------------------------------------------------

// A sheet's layout is revalidated by its ETag each time it is shown and
// after every edit; `changed` says whether a new one arrived.
const loadLayout = async (key) => {
  const held = state.layouts.get(key)
  const answer = await api.request('GET', `sheets/${key}/layout`, { etag: held && held.etag })
  if (answer.status === 304 && held) return { layout: held.layout, changed: false }
  state.layouts.set(key, { etag: answer.etag, layout: answer.data })
  return { layout: answer.data, changed: true }
}

const revalidateLayout = async () => {
  const key = state.sheet
  const sheet = sheetOf(key)
  if (!sheet || sheet.kind !== 'worksheet' || grid.sheet !== key || !grid.geometry) return
  let found
  try {
    found = await loadLayout(key)
  } catch {
    return
  }
  if (found.changed && state.sheet === key && grid.sheet === key) grid.setLayout(found.layout)
}

const showSheet = async (key, focus = false) => {
  const sheet = sheetOf(key)
  if (!sheet) return
  if (app.editor.isOpen()) app.editor.commit()
  if (state.sheet !== null && grid.geometry && state.sheet !== key) state.saved.set(state.sheet, grid.saved())
  state.sheet = key
  app.sheets.markSelected()
  mirror.setSheet(key, sheet.name)
  if (sheet.kind !== 'worksheet') {
    grid.showMessage(key, `${sheet.name} is a chart sheet: it is kept in the file and not drawn here.`)
    app.editor.refresh()
    app.editor.clearEntry()
    mirror.announce(sheet.name + ', chart sheet')
    return
  }
  // `data-loading` marks a sheet whose layout is on its way.
  $('grid').dataset.loading = 'true'
  let found
  try {
    found = await loadLayout(key)
  } catch (error) {
    // Shut by the gate on its way: an edit is removing the sheet, and the
    // sheet list that follows shows another.
    if (error.name === 'AbortError') return
    throw error
  } finally {
    if (state.sheet === key) delete $('grid').dataset.loading
  }
  if (state.sheet !== key) return
  grid.show(key, found.layout, state.saved.get(key))
  app.editor.refresh()
  mirror.announce(sheet.name)
  emit('sheet', key)
  if (focus) mirror.focus()
}

grid.addEventListener('focus', () => {
  if (!app.editor.isOpen()) mirror.focus()
})

// The workbook document --------------------------------------------------------

const showWorkbook = (workbook) => {
  const before = state.workbook
  // A document read before one already shown (a delayed refresh passed by
  // an edit's reread) says nothing newer: it could bring back a sheet the
  // edit removed.
  if (before && workbook.generation === before.generation && workbook.instance === before.instance &&
    workbook.revision < before.revision) return
  const sheetsBefore = before && JSON.stringify(before.sheets)
  state.workbook = workbook
  api.setSheets(workbook.sheets)
  const name = workbook.name || 'Book1'
  document.title = `${name} — yggdryl workbook`
  $('title').textContent = name
  $('title-state').textContent = workbook.readOnly ? 'Read-only' : workbook.dirty ? 'Unsaved changes' : ''
  document.documentElement.dataset.generation = String(workbook.generation)
  document.documentElement.dataset.revision = String(workbook.revision)
  mirror.setReadOnly(Boolean(workbook.readOnly))
  const label = (button, verb, entry) => {
    const text = entry && entry.label
    button.setAttribute('aria-label', text ? `${verb} ${text}` : verb)
    button.title = text ? `${verb} ${text}` : `Nothing to ${verb.toLowerCase()}`
    button.setAttribute('aria-disabled', text && !workbook.readOnly ? 'false' : 'true')
  }
  label($('undo'), 'Undo', workbook.undo)
  label($('redo'), 'Redo', workbook.redo)
  if (sheetsBefore !== JSON.stringify(workbook.sheets)) app.sheets.render()
  emit('workbook', workbook)
}

// The sheet list moved: tabs again, and a sheet no longer shown gives way to
// the one beside it. The document's revision is not adopted here: this
// tab's revision is the changes it has absorbed, and the feed brings the
// rest.
const reloadWorkbook = async () => {
  const before = shownSheets().map((sheet) => sheet.key)
  const workbook = await api.get('workbook', { quiet: true })
  if (workbook.generation !== api.generation || workbook.instance !== api.instance) {
    await reloadAll()
    return
  }
  for (const key of state.workbook ? state.workbook.sheets.map((sheet) => sheet.key) : []) {
    if (!workbook.sheets.some((sheet) => sheet.key === key)) {
      tiles.drop(key)
      state.layouts.delete(key)
      state.saved.delete(key)
    }
  }
  showWorkbook(workbook)
  const current = sheetOf(state.sheet)
  if (!current || current.state !== 'visible') {
    const shown = shownSheets()
    const at = Math.max(0, before.indexOf(state.sheet))
    const next = shown[Math.min(at, shown.length - 1)]
    if (next) await showSheet(next.key, true)
    else grid.showMessage(null, 'This workbook has no visible sheets.')
  }
}

// Undo labels and the dirty mark follow edits, a moment after the last.
let workbookTimer = 0
const scheduleWorkbook = () => {
  clearTimeout(workbookTimer)
  workbookTimer = setTimeout(async () => {
    try {
      showWorkbook(await api.get('workbook', { quiet: true }))
    } catch {}
  }, 120)
}

const reloadAll = async () => {
  const workbook = await api.workbook()
  tiles.reset(workbook.generation)
  state.layouts.clear()
  styles.reset()
  await styles.load()
  emit('reset', workbook)
  showWorkbook(workbook)
  const current = sheetOf(state.sheet)
  const next = current && current.state === 'visible' ? current : firstSheet(workbook)
  if (next) {
    state.sheet = null
    await showSheet(next.key)
  }
}

// `activeSheet` names a sheet by key; a hidden or unknown one falls back to
// the first sheet shown.
const firstSheet = (workbook) => {
  const shown = workbook.sheets.filter((sheet) => sheet.state === 'visible')
  return (shown.find((sheet) => sheet.key === workbook.activeSheet) || shown[0] || workbook.sheets[0] || null)
}

// Boot -----------------------------------------------------------------------

const boot = async () => {
  try {
    const workbook = await api.workbook()
    tiles.reset(workbook.generation)
    styles.reset()
    await styles.load()
    showWorkbook(workbook)
    const sheet = firstSheet(workbook)
    if (sheet) await showSheet(sheet.key)
    else grid.showMessage(null, 'This workbook has no sheets.')
    mirror.focus()
    sync.start()
  } catch (error) {
    grid.showMessage(null, 'The workbook could not be loaded: ' + ((error && error.message) || error))
  } finally {
    document.documentElement.dataset.ready = 'true'
  }
}

boot()
