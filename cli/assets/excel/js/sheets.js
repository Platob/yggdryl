// The sheet tab strip (§10): one tab per visible sheet, the one shown
// selected. A click shows a sheet; a double click renames it in place; a
// drag moves it, a mark showing where it lands; the tab's context menu and
// the Home tab's Format menu reach the rest - insert, delete, rename, move,
// hide, unhide - each one §8.4 op the service answers. Ctrl+PgUp/PgDn step
// through the tabs, and the arrows beside the strip scroll it a tab at a
// time.

import { ApiError, toast } from './api.js'
import { Dialog, el } from './dialogs.js'

// A press that travels this far is a drag, not a click.
const DRAG_START = 5
// How near the strip's end a drag scrolls it, and how fast.
const EDGE = 24
const EDGE_STEP = 12

// The `moveSheet` position that puts sheet `key` before sheet `before`, or
// after every sheet when `before` is null: its index in the workbook's list
// once it has left it, hidden sheets counted (§8.4 `to`).
export const movePosition = (sheets, key, before) => {
  const rest = sheets.filter((sheet) => sheet.key !== key)
  const at = before === null ? -1 : rest.findIndex((sheet) => sheet.key === before)
  return at < 0 ? rest.length : at
}

// Where a tab dropped at `x` goes: before the first tab whose middle is past
// `x`, or at the end. `boxes` are the tabs' `{ key, left, right }` in strip
// order.
export const dropBefore = (boxes, x) => {
  for (const box of boxes) if (x < (box.left + box.right) / 2) return box.key
  return null
}

export class Sheets {
  constructor ({ app, host, bar }) {
    this.app = app
    this.host = host
    this.bar = bar
    // A tab being renamed: { key, input, done }.
    this.renaming = null
    // A press on a tab that may become a drag: { key, tab, x, id, moved }.
    this.press = null
    this.suppressClick = false
    // The sheet a click is showing: a double click renames once it is shown,
    // so the sheet taking the keys cannot take them from the name field.
    this.showing = null
    this.mark = el('div', { className: 'sheet-drop', hidden: true, attrs: { 'aria-hidden': 'true' } })
    bar.append(this.mark)

    const command = (name, run) => app.command(name, run)
    command('insertSheet', (options) => this.insert(options))
    command('addSheet', () => this.insert({ after: app.sheetKey() }))
    command('deleteSheet', (options) => this.remove(options))
    command('renameSheet', (options) => this.rename(options))
    command('moveSheet', (options) => this.moveDialog(options))
    command('hideSheet', (options) => this.hide(options))
    command('unhideSheet', (options) => this.unhide(options))
    command('sheetsBack', () => this.scrollTabs(-1))
    command('sheetsForward', () => this.scrollTabs(1))

    host.addEventListener('click', (event) => {
      const tab = event.target.closest('.sheet-tab')
      if (!tab || tab.tagName !== 'BUTTON') return
      if (this.suppressClick) {
        this.suppressClick = false
        return
      }
      // The second click of a double click is the rename's.
      if (event.detail > 1) return
      // A sheet the service could not show said so where it was asked.
      this.showing = Promise.resolve(app.showSheet(Number(tab.dataset.key), true)).catch(() => {})
    })
    host.addEventListener('dblclick', (event) => {
      const tab = event.target.closest('.sheet-tab')
      if (!tab || tab.tagName !== 'BUTTON') return
      const key = Number(tab.dataset.key)
      Promise.resolve(this.showing).then(() => this.rename({ key }))
    })
    host.addEventListener('keydown', (event) => this.key(event))
    host.addEventListener('pointerdown', (event) => this.pointerDown(event))
    host.addEventListener('pointermove', (event) => this.pointerMove(event))
    host.addEventListener('pointerup', (event) => this.pointerUp(event, false))
    host.addEventListener('pointercancel', (event) => this.pointerUp(event, true))
    host.addEventListener('lostpointercapture', (event) => this.pointerUp(event, true))
    host.addEventListener('wheel', (event) => {
      event.preventDefault()
      host.scrollBy({ left: event.deltaX || event.deltaY })
    }, { passive: false })
    host.addEventListener('scroll', () => this.arrows())
    new ResizeObserver(() => this.arrows()).observe(host)
    // Escape drops a tab drag before anything else hears the key.
    window.addEventListener('keydown', (event) => {
      if (event.key !== 'Escape' || !this.press || !this.press.moved) return
      event.preventDefault()
      event.stopPropagation()
      this.endDrag(true)
    }, true)
  }

  // The workbook's sheets, and those with a tab.
  sheets () {
    const workbook = this.app.workbook()
    return workbook ? workbook.sheets : []
  }

  shown () {
    return this.sheets().filter((sheet) => sheet.state === 'visible')
  }

  sheet (key) {
    return this.sheets().find((sheet) => sheet.key === key) || null
  }

  tabs () {
    return [...this.host.querySelectorAll('.sheet-tab, .sheet-rename')]
  }

  // The strip ------------------------------------------------------------------

  render () {
    if (this.renaming) this.renaming.done = true
    this.renaming = null
    const tabs = this.shown().map((sheet) => el('button', {
      type: 'button',
      className: 'sheet-tab' + (sheet.kind === 'worksheet' ? '' : ' chartsheet'),
      id: 'sheet-tab-' + sheet.key,
      text: sheet.name,
      title: sheet.name,
      dataset: { key: sheet.key },
      attrs: { role: 'tab', 'aria-controls': 'grid-mirror', 'aria-haspopup': 'menu' }
    }))
    this.host.replaceChildren(...tabs)
    this.markSelected()
  }

  // The tab of the sheet shown, selected and scrolled into view.
  markSelected () {
    const key = this.app.sheetKey()
    for (const tab of this.tabs()) {
      if (tab.tagName !== 'BUTTON') continue
      const selected = Number(tab.dataset.key) === key
      tab.setAttribute('aria-selected', String(selected))
      tab.tabIndex = selected ? 0 : -1
      if (selected) this.reveal(tab)
    }
    this.arrows()
  }

  // Scroll the strip the least that shows a whole tab.
  reveal (tab) {
    const left = tab.offsetLeft - this.host.offsetLeft
    const right = left + tab.offsetWidth
    if (left < this.host.scrollLeft) this.host.scrollLeft = left
    else if (right > this.host.scrollLeft + this.host.clientWidth) this.host.scrollLeft = right - this.host.clientWidth
    this.arrows()
  }

  // The arrows beside the strip: the tab edge before or after the view's.
  scrollTabs (direction) {
    const host = this.host
    const at = host.scrollLeft
    const edges = this.tabs().map((tab) => tab.offsetLeft - host.offsetLeft)
    const end = host.scrollWidth - host.clientWidth
    const to = direction < 0
      ? [...edges].reverse().find((left) => left < at - 1) ?? 0
      : edges.find((left) => left > at + 1) ?? end
    host.scrollLeft = Math.max(0, Math.min(to, end))
    this.arrows()
  }

  // Each arrow says whether it has anywhere to go.
  arrows () {
    const host = this.host
    const back = this.bar.querySelector('[data-command="sheetsBack"]')
    const forward = this.bar.querySelector('[data-command="sheetsForward"]')
    const end = host.scrollWidth - host.clientWidth
    if (back) back.setAttribute('aria-disabled', String(host.scrollLeft <= 0))
    if (forward) forward.setAttribute('aria-disabled', String(host.scrollLeft >= end - 1))
  }

  // Ctrl+PgUp/PgDn: the tab before or after the one shown.
  step (delta, focus = false) {
    const shown = this.shown()
    const at = shown.findIndex((sheet) => sheet.key === this.app.sheetKey())
    const next = shown[at + delta]
    if (!next) return
    const done = this.app.showSheet(next.key)
    if (focus) {
      const tab = document.getElementById('sheet-tab-' + next.key)
      if (tab) tab.focus()
    }
    return done
  }

  key (event) {
    if (event.target.tagName !== 'BUTTON') return
    const ctrl = event.ctrlKey || event.metaKey
    if (ctrl && (event.key === 'PageUp' || event.key === 'PageDown')) {
      event.preventDefault()
      this.step(event.key === 'PageUp' ? -1 : 1, true)
      return
    }
    if (ctrl || event.altKey) return
    const tabs = this.tabs()
    const at = tabs.indexOf(document.activeElement)
    if (at < 0) return
    const next = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: tabs.length - 1 }[event.key]
    if (next === undefined || !tabs[next]) return
    event.preventDefault()
    tabs[next].focus()
    this.reveal(tabs[next])
    this.app.showSheet(Number(tabs[next].dataset.key))
  }

  // Dragging a tab ---------------------------------------------------------------

  pointerDown (event) {
    const tab = event.target.closest('.sheet-tab')
    if (!tab || tab.tagName !== 'BUTTON' || event.button !== 0 || event.pointerType === 'touch') return
    this.press = { key: Number(tab.dataset.key), tab, x: event.clientX, id: event.pointerId, moved: false, before: undefined }
  }

  pointerMove (event) {
    const press = this.press
    if (!press || event.pointerId !== press.id) return
    // A press released outside the strip never said so here.
    if (!(event.buttons & 1)) {
      if (press.moved) this.endDrag(true)
      else this.press = null
      return
    }
    if (!press.moved) {
      if (Math.abs(event.clientX - press.x) < DRAG_START) return
      if (!this.movable()) {
        this.press = null
        return
      }
      press.moved = true
      this.host.setPointerCapture(press.id)
      press.tab.classList.add('dragging')
      this.bar.dataset.dragging = 'true'
    }
    this.dragTo(event.clientX)
  }

  // The mark goes where the tab would land; near either end the strip
  // scrolls under the pointer.
  dragTo (x) {
    const press = this.press
    press.clientX = x
    const host = this.host.getBoundingClientRect()
    if (x < host.left + EDGE) this.host.scrollLeft -= EDGE_STEP
    else if (x > host.right - EDGE) this.host.scrollLeft += EDGE_STEP
    const boxes = this.tabs().map((tab) => {
      const box = tab.getBoundingClientRect()
      return { key: Number(tab.dataset.key), left: box.left, right: box.right }
    })
    const before = dropBefore(boxes, x)
    press.before = before
    const bar = this.bar.getBoundingClientRect()
    const anchor = before === null ? boxes[boxes.length - 1] : boxes.find((box) => box.key === before)
    const at = before === null ? anchor.right : anchor.left
    const clamped = Math.min(Math.max(at, host.left), host.right)
    this.mark.style.left = Math.round(clamped - bar.left - 1) + 'px'
    this.mark.hidden = false
    this.mark.dataset.before = before === null ? 'end' : String(before)
    clearTimeout(this.edgeTimer)
    if (x < host.left + EDGE || x > host.right - EDGE) {
      this.edgeTimer = setTimeout(() => {
        if (this.press && this.press.moved) this.dragTo(this.press.clientX)
      }, 40)
    }
  }

  pointerUp (event, cancelled) {
    const press = this.press
    if (!press || event.pointerId !== press.id) return
    if (!press.moved) {
      this.press = null
      return
    }
    // The click that follows a drag is not a click on the tab.
    this.suppressClick = !cancelled && event.type === 'pointerup'
    setTimeout(() => {
      this.suppressClick = false
    }, 0)
    this.endDrag(cancelled)
  }

  endDrag (cancelled) {
    const press = this.press
    this.press = null
    clearTimeout(this.edgeTimer)
    if (!press) return
    press.tab.classList.remove('dragging')
    delete this.bar.dataset.dragging
    this.mark.hidden = true
    if (this.host.hasPointerCapture(press.id)) this.host.releasePointerCapture(press.id)
    if (cancelled || press.before === undefined || press.before === press.key) return
    // The service's refusal was shown where it was answered.
    this.move(press.key, press.before).catch((error) => {
      if (!(error instanceof ApiError)) toast(String((error && error.message) || error))
    })
  }

  movable () {
    const workbook = this.app.workbook()
    if (workbook && workbook.readOnly) {
      toast('This workbook is open read-only.', 'info')
      return false
    }
    return this.shown().length > 1
  }

  // The verbs ------------------------------------------------------------------------

  // Sheet `key` before sheet `before` (null: at the end); nothing is sent
  // when it is already there.
  async move (key, before, options = {}) {
    const sheets = this.sheets()
    const to = movePosition(sheets, key, before)
    if (sheets.findIndex((sheet) => sheet.key === key) === to) return
    await this.app.edit({ op: 'moveSheet', sheet: key, to }, options)
  }

  // A new sheet before `before` (the sheet shown) or after `after` (the
  // sheet bar's +); it becomes the sheet shown.
  async insert ({ before = this.app.sheetKey(), after = null } = {}) {
    const sheets = this.sheets()
    const known = new Set(sheets.map((sheet) => sheet.key))
    const anchor = sheets.findIndex((sheet) => sheet.key === (after ?? before))
    const at = anchor < 0 ? sheets.length : anchor + (after === null ? 0 : 1)
    await this.app.edit({ op: 'addSheet', at })
    const added = this.sheets().find((sheet) => !known.has(sheet.key))
    if (added) await this.app.showSheet(added.key, true)
  }

  async remove ({ key = this.app.sheetKey() } = {}) {
    const sheet = this.sheet(key)
    if (!sheet) return
    const confirmed = await this.app.confirm(`Delete the sheet “${sheet.name}”? Undo brings it back.`, 'Delete')
    if (confirmed) await this.app.edit({ op: 'removeSheet', sheet: key })
  }

  hide ({ key = this.app.sheetKey() } = {}) {
    return this.app.edit({ op: 'sheetState', sheet: key, state: 'hidden' })
  }

  async unhide ({ key } = {}) {
    if (key === undefined) return
    await this.app.edit({ op: 'sheetState', sheet: key, state: 'visible' })
    await this.app.showSheet(key, true)
  }

  // Rename in place: the tab becomes a text field over its name. Enter or
  // leaving the field renames through the service, Escape keeps the name;
  // a name the service refuses keeps the field open with its reason.
  rename ({ key = this.app.sheetKey() } = {}) {
    const sheet = this.sheet(key)
    const tab = document.getElementById('sheet-tab-' + key)
    if (!sheet || !tab || this.renaming) return
    const workbook = this.app.workbook()
    if (workbook && workbook.readOnly) {
      toast('This workbook is open read-only.', 'info')
      return
    }
    const input = el('input', {
      type: 'text',
      className: 'sheet-rename',
      id: 'sheet-rename',
      value: sheet.name,
      spellcheck: false,
      dataset: { key },
      attrs: { 'aria-label': `Rename sheet ${sheet.name}`, autocomplete: 'off' }
    })
    input.size = Math.max(4, sheet.name.length + 2)
    const state = { key, input, done: false, tab }
    this.renaming = state
    tab.replaceWith(input)
    input.focus()
    input.select()
    input.addEventListener('input', () => {
      input.size = Math.max(4, input.value.length + 2)
    })
    input.addEventListener('keydown', (event) => {
      event.stopPropagation()
      if (event.key === 'Enter') {
        event.preventDefault()
        this.finishRename(state, true)
      } else if (event.key === 'Escape') {
        event.preventDefault()
        this.finishRename(state, false)
      }
    })
    input.addEventListener('blur', () => {
      if (!state.done && !state.sending) this.finishRename(state, true, false)
    })
  }

  async finishRename (state, save, focus = true) {
    if (state.done || state.sending) return
    const name = state.input.value
    const sheet = this.sheet(state.key)
    const end = () => {
      state.done = true
      if (this.renaming === state) this.renaming = null
      if (state.input.isConnected) state.input.replaceWith(state.tab)
      this.markSelected()
      if (focus) this.app.focusGrid()
    }
    if (!save || !sheet || name === sheet.name) {
      end()
      return
    }
    state.sending = true
    state.input.setAttribute('aria-busy', 'true')
    try {
      await this.app.edit({ op: 'renameSheet', sheet: state.key, name }, { quiet: true })
    } catch (error) {
      state.sending = false
      state.input.removeAttribute('aria-busy')
      if (!(error instanceof ApiError)) throw error
      if (state.done) return
      toast(error.kind === 'stale_base' ? 'Another tab changed the sheets first; try the name again.' : error.message)
      state.input.focus()
      state.input.select()
      return
    }
    state.sending = false
    // The answer re-rendered the strip; the field went with it.
    if (!state.done) end()
    else if (focus) this.app.focusGrid()
  }

  // Move (the context menu's Move…): before which sheet, or to the end.
  async moveDialog ({ key = this.app.sheetKey() } = {}) {
    const sheet = this.sheet(key)
    if (!sheet) return
    const shown = this.shown()
    const list = el('select', { id: 'move-sheet-before', attrs: { size: Math.min(10, shown.length + 1), 'aria-label': 'Before sheet' } },
      shown.map((item) => el('option', { value: String(item.key), text: item.name })),
      el('option', { value: 'end', text: '(move to end)' }))
    const at = shown.findIndex((item) => item.key === key)
    const next = shown[at + 1]
    list.value = next ? String(next.key) : 'end'
    const dialog = new Dialog({
      title: 'Move Sheet',
      id: 'move-sheet',
      className: 'small',
      body: [
        el('p', { text: `Move “${sheet.name}” before sheet:` }),
        list
      ],
      buttons: [{
        label: 'OK',
        primary: true,
        id: 'move-sheet-ok',
        action: async () => {
          const before = list.value === 'end' ? null : Number(list.value)
          if (before === key) return true
          try {
            await this.move(key, before, { quiet: true })
          } catch (error) {
            if (!(error instanceof ApiError)) throw error
            dialog.fail(error.message)
            return false
          }
          return true
        }
      }, { label: 'Cancel', value: null }],
      initial: list,
      restore: () => this.app.focusGrid()
    })
    list.addEventListener('dblclick', () => dialog.primary.click())
    await dialog.open()
  }
}
