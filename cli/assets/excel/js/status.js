// The status bar (§10): the mode (Ready, Enter, Edit, Point), what the
// sheet is doing (loading cells, a copy waiting for its destination),
// Calculate with the uncomputed count, circular references, the aggregates
// of the selection from `summary` (debounced 150 ms), the workbook's clock,
// saved or unsaved, and the zoom slider. Which aggregates show is the
// viewer's choice (right click), kept in this browser. A bar too short for
// everything shortens the note first, then drops whole aggregates from
// the left, so Sum is the last to go and none is cut mid-number.

import { ApiError } from './api.js'
import { parseRange, intersects } from './geometry.js'

export const SUMMARY_DELAY = 150

// The aggregates in the order the bar shows them, Excel's.
export const AGGREGATES = [
  { key: 'average', label: 'Average' },
  { key: 'count', label: 'Count' },
  { key: 'numbers', label: 'Numerical Count' },
  { key: 'min', label: 'Min' },
  { key: 'max', label: 'Max' },
  { key: 'sum', label: 'Sum' }
]

const SHOWN_KEY = 'yggdryl-status-aggregates'
const MIN_ZOOM = 0.25
const MAX_ZOOM = 4

// The slider runs 0..100 with 100% at its middle: 25%..100% on the left
// half, 100%..400% on the right, as Excel's does.
export const zoomOfSlider = (position) => {
  const p = Math.min(100, Math.max(0, Number(position) || 0))
  const zoom = p <= 50 ? MIN_ZOOM + (p / 50) * (1 - MIN_ZOOM) : 1 + ((p - 50) / 50) * (MAX_ZOOM - 1)
  return Math.round(zoom * 100) / 100
}

export const sliderOfZoom = (zoom) => {
  const z = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom))
  return z <= 1 ? ((z - MIN_ZOOM) / (1 - MIN_ZOOM)) * 50 : 50 + ((z - 1) / (MAX_ZOOM - 1)) * 50
}

// A count as the bar shows it; any other number is the service's text.
const countText = (n) => (typeof n === 'number' ? n.toLocaleString('en-US') : '')

const readShown = () => {
  try {
    const saved = JSON.parse(localStorage.getItem(SHOWN_KEY) || 'null')
    if (Array.isArray(saved)) return new Set(saved.filter((key) => AGGREGATES.some((a) => a.key === key)))
  } catch {}
  return new Set(AGGREGATES.map((a) => a.key))
}

export class Status {
  constructor ({ app, api, grid, selection, tiles, editor, menus, element }) {
    this.app = app
    this.api = api
    this.grid = grid
    this.selection = selection
    this.menus = menus
    this.element = element
    this.shown = readShown()
    this.timer = 0
    this.controller = null
    this.asked = ''
    this.workbook = null
    this.calc = null
    this.copying = null
    this.loading = ''
    this.fitting = 0

    const $ = (id) => document.getElementById(id)
    this.mode = $('status-mode')
    this.note = $('status-note')
    this.calculate = $('status-calculate')
    this.circular = $('status-circular')
    this.aggregates = $('status-aggregates')
    this.clock = $('status-clock')
    this.saved = $('status-saved')
    this.zoomOut = $('status-zoom-out')
    this.zoomIn = $('status-zoom-in')
    this.slider = $('status-zoom-slider')
    this.zoomLabel = $('status-zoom')

    editor.addEventListener('mode', (event) => {
      this.mode.textContent = event.detail
      this.element.dataset.mode = event.detail.toLowerCase()
    })
    selection.addEventListener('change', () => this.schedule())
    app.events.addEventListener('workbook', (event) => this.setWorkbook(event.detail))
    app.events.addEventListener('edited', (event) => {
      const answer = event.detail.answer
      if (answer && answer.calc) this.setCalc(answer.calc)
    })
    // A change under the selection (here or in another tab) moves its sums.
    app.events.addEventListener('changed', (event) => {
      const change = event.detail
      const sheet = this.app.sheetKey()
      for (const item of change.changed || []) {
        if (item.sheet !== sheet) continue
        const range = parseRange(item.range)
        if (change.structural || !range || this.selection.ranges.some((r) => intersects(r, range))) {
          this.schedule(true)
          return
        }
      }
    })
    app.events.addEventListener('sheet', () => {
      this.schedule(true)
      if (this.calc) this.setCalc({})
    })
    grid.addEventListener('frame', () => {
      const pending = this.grid.host.dataset.pending
      this.loading = pending && pending !== '0' ? 'Loading cells…' : ''
      this.showNote()
    })
    tiles.addEventListener('failed', () => {
      this.loading = 'Some cells could not be loaded'
      this.showNote()
    })

    this.calculate.addEventListener('click', () => this.app.run('calculate'))
    this.circular.addEventListener('click', () => this.goToCircular())
    this.zoomOut.addEventListener('click', () => this.step(-1))
    this.zoomIn.addEventListener('click', () => this.step(1))
    this.slider.addEventListener('input', () => this.setZoom(zoomOfSlider(this.slider.value), false))
    this.slider.addEventListener('change', () => this.app.focusGrid())
    element.addEventListener('contextmenu', (event) => {
      event.preventDefault()
      this.customize(event.clientX, event.clientY)
    })
    new ResizeObserver(() => this.refit()).observe(element)

    app.command('zoom', ({ value }) => this.setZoom(Number(value)))
    app.command('calculate', ({ full = false } = {}) => this.recalculate(full))
    app.command('statusAggregate', ({ key }) => this.toggle(key))
    app.keyboard.use((event) => {
      if (event.key === 'F9' && (event.ctrlKey || event.metaKey) && event.altKey && !event.shiftKey) {
        event.preventDefault()
        this.app.run('calculate', { full: true })
        return true
      }
      return false
    })
    this.showZoom(grid.zoom)
  }

  // The workbook document -------------------------------------------------------

  setWorkbook (workbook) {
    this.workbook = workbook
    this.clock.textContent = workbook.timezone || 'UTC'
    this.clock.title = `TODAY() and NOW() read the clock in ${workbook.timezone || 'UTC'}`
    const state = workbook.readOnly ? 'Read-only' : workbook.dirty ? 'Unsaved' : 'Saved'
    this.saved.textContent = state
    this.saved.dataset.state = state.toLowerCase()
    this.saved.title = workbook.readOnly ? 'This workbook is open read-only' : workbook.dirty
      ? 'Changes since the last save' : 'No changes since the last save'
    if (workbook.calc) this.setCalc(workbook.calc)
  }

  // Calculate (n) while formulas wait; the first circular reference, and
  // how many there are.
  setCalc (calc) {
    this.calc = { ...(this.calc || {}), ...calc }
    // An edit's answer lists the circular references without the
    // workbook document's count; an older count says nothing about them.
    if ('circular' in calc && !('circularCount' in calc)) delete this.calc.circularCount
    const uncomputed = this.calc.uncomputed || 0
    this.calculate.hidden = uncomputed === 0
    this.calculate.textContent = `Calculate (${countText(uncomputed)})`
    this.calculate.title = uncomputed === 1 ? 'One formula waits for a value: calculate now (F9)' : `${countText(uncomputed)} formulas wait for a value: calculate now (F9)`
    this.calculate.setAttribute('aria-label', this.calculate.title)
    const circular = this.calc.circular || []
    const count = Math.max(circular.length, this.calc.circularCount ?? 0)
    this.circular.hidden = circular.length === 0
    if (circular.length) {
      const first = circular[0]
      const sheet = ((this.workbook && this.workbook.sheets) || []).find((item) => item.key === first.sheet)
      const where = first.sheet === this.app.sheetKey() || !sheet ? first.ref : `${sheet.name}!${first.ref}`
      this.circular.textContent = `Circular References: ${where}`
      this.circular.title = count > 1 ? `${count} circular references; go to the first` : 'Go to the circular reference'
      this.circular.dataset.sheet = String(first.sheet)
      this.circular.dataset.ref = first.ref
    }
    this.refit()
  }

  async recalculate (full) {
    let answer
    try {
      answer = await this.app.send(() => this.api.calculate(full))
    } catch (error) {
      if (!(error instanceof ApiError)) throw error
      return
    }
    if (answer && answer.calc) this.setCalc(answer.calc)
  }

  goToCircular () {
    const { sheet, ref } = this.circular.dataset
    if (!ref) return
    const target = ((this.workbook && this.workbook.sheets) || []).find((item) => item.key === Number(sheet))
    this.app.editor.go(target ? `'${target.name.replace(/'/g, "''")}'!${ref}` : ref)
  }

  // Notes: a copy waiting for its destination, else the tiles' state.
  setCopying (text) {
    this.copying = text
    this.showNote()
  }

  showNote () {
    const text = this.copying || this.loading
    if (this.note.textContent === text) return
    this.note.textContent = text
    this.note.title = text
    this.refit()
  }

  // Aggregates --------------------------------------------------------------------

  // Sums follow the selection a moment after it stops moving; a single cell
  // shows none. `force` asks again for the same ranges (they changed).
  schedule (force = false) {
    clearTimeout(this.timer)
    this.timer = setTimeout(() => this.summarize(force), SUMMARY_DELAY)
  }

  async summarize (force) {
    const sheet = this.app.sheetKey()
    const selection = this.selection
    const multiple = selection.ranges.length > 1 || !selection.isSingle()
    if (!this.grid.geometry || this.grid.sheet !== sheet || !multiple) {
      this.clear()
      return
    }
    const ranges = selection.refs.join(',')
    const asked = `${sheet}|${ranges}`
    if (!force && asked === this.asked) return
    this.asked = asked
    if (this.controller) this.controller.abort()
    const controller = new AbortController()
    this.controller = controller
    this.element.dataset.summary = 'pending'
    let answer
    try {
      answer = await this.api.get(`sheets/${sheet}/summary?range=${encodeURIComponent(ranges)}`, { quiet: true, signal: controller.signal })
    } catch {
      if (controller !== this.controller) return
      this.controller = null
      this.asked = ''
      this.clear()
      return
    }
    if (controller !== this.controller) return
    this.controller = null
    this.show(answer)
  }

  clear () {
    if (this.controller) this.controller.abort()
    this.controller = null
    this.asked = ''
    this.aggregates.replaceChildren()
    this.element.dataset.summary = 'none'
  }

  // Excel's rule: nothing under two filled cells; the count alone over text;
  // the numeric aggregates once one of them is a number.
  show (answer) {
    const items = []
    const count = answer.count || 0
    const numbers = answer.numbers || 0
    if (count >= 2) {
      const text = answer.text || {}
      const value = (key) => text[key] ?? (answer[key] === null || answer[key] === undefined ? '' : String(answer[key]))
      for (const { key, label } of AGGREGATES) {
        if (!this.shown.has(key)) continue
        if (key === 'count') items.push([key, label, countText(count)])
        else if (numbers === 0) continue
        else if (key === 'numbers') items.push([key, label, countText(numbers)])
        else items.push([key, label, value(key)])
      }
    }
    const nodes = items.map(([key, label, text]) => {
      const node = document.createElement('span')
      node.className = 'status-aggregate'
      node.dataset.key = key
      node.textContent = `${label}: ${text}`
      return node
    })
    this.aggregates.replaceChildren(...nodes)
    this.element.dataset.summary = items.length ? 'shown' : 'none'
    this.fit()
  }

  // Once a frame at most: every aggregate back, then the first ones hidden
  // while the bar still overflows (the note has already shrunk to its least).
  refit () {
    if (this.fitting) return
    this.fitting = requestAnimationFrame(() => {
      this.fitting = 0
      this.fit()
    })
  }

  fit () {
    const items = [...this.aggregates.children]
    for (const item of items) item.hidden = false
    for (const item of items) {
      if (this.element.scrollWidth <= this.element.clientWidth) break
      item.hidden = true
    }
  }

  // Customize Status Bar: which aggregates show.
  customize (x, y) {
    this.menus.open([
      { heading: 'Customize Status Bar' },
      ...AGGREGATES.map(({ key, label }) => ({ label, command: 'statusAggregate', args: { key }, checked: this.shown.has(key) }))
    ], { x, y, label: 'Customize Status Bar' })
  }

  toggle (key) {
    if (this.shown.has(key)) this.shown.delete(key)
    else this.shown.add(key)
    try {
      localStorage.setItem(SHOWN_KEY, JSON.stringify([...this.shown]))
    } catch {}
    this.schedule(true)
  }

  // Zoom ------------------------------------------------------------------------

  setZoom (zoom, focus = true) {
    const value = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Number(zoom) || 1))
    this.grid.setZoom(value)
    this.showZoom(value)
    if (focus) this.app.focusGrid()
  }

  // Ten percent a press, landing on the tens.
  step (direction) {
    const percent = Math.round(this.grid.zoom * 100)
    const next = direction > 0 ? Math.floor(percent / 10) * 10 + 10 : Math.ceil(percent / 10) * 10 - 10
    this.setZoom(next / 100, false)
  }

  showZoom (zoom) {
    const percent = Math.round(zoom * 100) + '%'
    this.zoomLabel.textContent = percent
    this.slider.value = String(sliderOfZoom(zoom))
    this.slider.setAttribute('aria-valuetext', percent)
    // The View tab's picker shows the zoom too: a zoom it does not list
    // takes the one added option.
    const select = document.getElementById('zoom')
    if (select && document.activeElement !== select) {
      const value = String(zoom)
      const listed = [...select.options].some((option) => option.value === value && !option.dataset.added)
      for (const option of select.querySelectorAll('option[data-added]')) option.remove()
      if (!listed) {
        const option = document.createElement('option')
        option.value = value
        option.textContent = percent
        option.dataset.added = 'true'
        select.append(option)
      }
      select.value = value
    }
  }
}
