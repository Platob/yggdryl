// The clipboard (§10): copy, cut and paste, Paste Special, the marquee
// round what was copied, and the Format Painter.
//
// Copy writes the selection's `export` as text/plain (TSV) and text/html
// through the asynchronous clipboard; the `copy` event is the fallback when
// the browser refuses that. The HTML carries the service's
// `data-yggdryl-copy` marker. A paste reads the clipboard in the `paste`
// event (Ctrl+V), or through `navigator.clipboard.read` from a button: our
// marker - this service instance (addendum 8) and workbook generation -
// over cells unchanged since the copy, is the service's own `paste` (a cut
// from this tab becomes the move); any other text is `pasteText`, another
// service's copy included. The Format Painter is `paste` with `what:
// formats` onto the next selection the pointer makes.

import { ApiError, toast } from './api.js'
import { MAX_ROWS, MAX_COLUMNS, parseRange, rangeName, refName, intersects } from './geometry.js'
import { Dialog, el } from './dialogs.js'

const MARKER = /data-yggdryl-copy\s*=\s*"([^"]*)"/i
const MAX_TILES = 256
// §8.3: an export answers at most a million cells.
const MAX_EXPORT = 1000000

// `{instance}:{generation}:{revision}:{key}:{range}` from an export's HTML
// (addendum 8), or null.
export const markerOf = (html) => {
  const match = MARKER.exec(html || '')
  if (!match) return null
  const parts = match[1].split(':')
  if (parts.length < 5) return null
  const instance = parts[0]
  const [generation, revision, sheet] = parts.slice(1, 4).map(Number)
  const range = parts.slice(4).join(':')
  if (!/^[0-9A-Za-z-]+$/.test(instance) || ![generation, revision, sheet].every(Number.isInteger) || !parseRange(range)) return null
  return { raw: match[1], instance, generation, revision, sheet, range }
}

// The rows and columns a TSV spans, for the hatch while `pasteText` is out.
export const textExtent = (text) => {
  const lines = text.replace(/\r\n?/g, '\n').replace(/\n$/, '').split('\n')
  let columns = 1
  for (const line of lines) columns = Math.max(columns, line.split('\t').length)
  return { rows: lines.length, columns }
}

// These edits end a copy's marquee, as typing does in Excel.
const ENDS_COPY = new Set(['setEntries', 'fillEntry', 'clear', 'insertRows', 'removeRows', 'insertColumns',
  'removeColumns', 'sort', 'fill', 'removeSheet', 'undo', 'redo', 'import'])

const clamp = (range) => ({
  r0: Math.max(0, range.r0),
  c0: Math.max(0, range.c0),
  r1: Math.min(MAX_ROWS - 1, range.r1),
  c1: Math.min(MAX_COLUMNS - 1, range.c1)
})

export class Clipboard {
  constructor ({ app, api, grid, selection, keyboard, status, host }) {
    this.app = app
    this.api = api
    this.grid = grid
    this.selection = selection
    this.status = status
    this.host = host
    // The copy or cut this tab made: { sheet, range, cut, marker, written }.
    this.source = null
    // The Format Painter: { sheet, range, sticky }.
    this.painter = null
    // A cut already moved: its marker pastes nothing again.
    this.consumed = null
    // Text for the `copy` event when the asynchronous clipboard refused.
    this.prepared = null

    this.clip = el('div', { className: 'marquee-clip', hidden: true, attrs: { 'aria-hidden': 'true' } })
    this.ants = el('div', { className: 'marquee' })
    this.clip.append(this.ants)
    host.append(this.clip)
    grid.addEventListener('frame', () => this.place())

    const command = (name, run) => app.command(name, run)
    command('copy', () => this.copy(false))
    command('cut', () => this.copy(true))
    command('paste', () => this.paste('all'))
    command('pasteValues', () => this.paste('values'))
    command('pasteFormulas', () => this.paste('formulas'))
    command('pasteFormats', () => this.paste('formats'))
    command('pasteSpecial', () => this.pasteSpecial())
    command('pasteData', (data) => this.pasteData(data))
    command('formatPainter', () => (this.painter ? this.endPainter() : this.startPainter(false)))
    const painterButton = document.querySelector('[data-command="formatPainter"]')
    if (painterButton) painterButton.addEventListener('dblclick', () => this.startPainter(true))

    keyboard.use((event) => this.key(event))
    document.addEventListener('copy', (event) => this.copyEvent(event, false))
    document.addEventListener('cut', (event) => this.copyEvent(event, true))
    document.addEventListener('paste', (event) => this.pasteEvent(event))
    grid.addEventListener('dragging', (event) => {
      if (!event.detail && this.painter) this.paint()
    })
    app.events.addEventListener('edited', (event) => {
      const { edit } = event.detail
      if (this.source && edit && ENDS_COPY.has(edit.op)) this.clear()
    })
    app.events.addEventListener('reset', () => {
      this.clear()
      this.endPainter()
    })
  }

  // Keys on the sheet: Ctrl+C, Ctrl+X, Ctrl+V (the paste event follows),
  // Ctrl+Insert, Shift+Insert, Shift+Delete, Ctrl+Alt+V; Enter pastes what
  // was copied and ends the copy; Escape ends it, or the Format Painter.
  key (event) {
    const ctrl = event.ctrlKey || event.metaKey
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key
    const run = (name) => {
      event.preventDefault()
      this.app.run(name)
      return true
    }
    if (ctrl && !event.altKey && !event.shiftKey) {
      if (key === 'c' || key === 'Insert') return run('copy')
      if (key === 'x') return run('cut')
      // The browser's own paste event carries the clipboard: let it come.
      if (key === 'v') return true
      return false
    }
    if (ctrl && event.altKey && !event.shiftKey && key === 'v') return run('pasteSpecial')
    if (event.shiftKey && !ctrl && !event.altKey) {
      if (key === 'Delete') return run('cut')
      if (key === 'Insert') return true
    }
    if (ctrl || event.altKey || event.shiftKey) return false
    if (key === 'Escape' && (this.source || this.painter)) {
      event.preventDefault()
      this.clear()
      this.endPainter()
      return true
    }
    if (key === 'Enter' && this.painter) {
      event.preventDefault()
      this.paint()
      return true
    }
    if (key === 'Enter' && this.source) {
      event.preventDefault()
      this.app.run('pasteData', { local: true, end: true })
      return true
    }
    return false
  }

  cells () {
    if (this.grid.geometry && this.grid.sheet === this.app.sheetKey()) return true
    toast('Select cells on a worksheet first.', 'info')
    return false
  }

  single () {
    if (this.selection.ranges.length === 1) return true
    toast("This action won't work on multiple selections.", 'info')
    return false
  }

  // Copy and cut ------------------------------------------------------------------

  // The selection becomes the source, with its marquee; its export goes to
  // the clipboard. A selection past the export's bound (whole columns, the
  // sheet) exports the part the layout says holds cells, the whole
  // selection staying the source of a paste in this tab.
  async copy (cut) {
    if (!this.cells() || !this.single()) return
    this.endPainter()
    const sheet = this.app.sheetKey()
    const range = clamp(this.selection.range)
    const source = { sheet, range, cut, marker: null, exported: rangeName(this.exported(range)) }
    this.show(source)
    const path = (format) => `sheets/${sheet}/export?range=${encodeURIComponent(source.exported)}&format=${format}`
    // A refused export is said once, by the text form's toast.
    const tsv = this.api.text(path('tsv'))
    const html = this.api.text(path('html'), { quiet: true })
    html.then((text) => {
      const marker = markerOf(text)
      if (marker) source.marker = marker.raw
    }, () => {})
    document.documentElement.dataset.clipboard = 'writing'
    const written = await this.write(tsv, html)
    if (written === null) {
      // The service refused the export (and said why): nothing was copied.
      if (this.source === source) this.clear()
      document.documentElement.dataset.clipboard = 'refused'
      return
    }
    source.written = written
    document.documentElement.dataset.clipboard = written
  }

  exported (range) {
    if ((range.r1 - range.r0 + 1) * (range.c1 - range.c0 + 1) <= MAX_EXPORT) return range
    const used = this.grid.geometry.dimension
    if (!intersects(range, used)) return { r0: range.r0, c0: range.c0, r1: range.r0, c1: range.c0 }
    return { r0: Math.max(range.r0, used.r0), c0: Math.max(range.c0, used.c0), r1: Math.min(range.r1, used.r1), c1: Math.min(range.c1, used.c1) }
  }

  // `navigator.clipboard.write` with the export's two forms, the item
  // built at once so the gesture still counts; else the `copy` event.
  // Answers 'written', 'event', 'blocked', or null when the export failed.
  async write (tsv, html) {
    const both = Promise.allSettled([tsv, html])
    try {
      if (!navigator.clipboard || typeof navigator.clipboard.write !== 'function' || typeof ClipboardItem !== 'function') {
        throw new Error('no asynchronous clipboard')
      }
      const blob = (text, type) => text.then((value) => new Blob([value], { type }))
      await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob(tsv, 'text/plain'), 'text/html': blob(html, 'text/html') })])
      return 'written'
    } catch {
      const [plain, rich] = await both
      if (plain.status === 'rejected' || rich.status === 'rejected') return null
      this.prepared = { tsv: plain.value, html: rich.value }
      let copied = false
      try {
        copied = document.execCommand('copy')
      } catch {}
      if (this.prepared) {
        this.prepared = null
        copied = false
      }
      if (!copied) toast('The browser kept the clipboard closed: this copy pastes within this tab only.', 'info')
      return copied ? 'event' : 'blocked'
    }
  }

  // The `copy` and `cut` events: the fallback's text, or the browser's own
  // Copy on the sheet run as ours.
  copyEvent (event, cut) {
    if (this.prepared) {
      event.clipboardData.setData('text/plain', this.prepared.tsv)
      event.clipboardData.setData('text/html', this.prepared.html)
      event.preventDefault()
      this.prepared = null
      return
    }
    if (!this.app.gridFocused()) return
    event.preventDefault()
    this.app.run(cut ? 'cut' : 'copy')
  }

  // Paste ------------------------------------------------------------------------

  pasteEvent (event) {
    if (!this.app.gridFocused()) return
    event.preventDefault()
    const data = event.clipboardData
    this.app.run('pasteData', { html: data.getData('text/html'), text: data.getData('text/plain') })
  }

  // A button or menu: the clipboard as the browser lets the page read it,
  // else this tab's own copy.
  async paste (what) {
    if (!this.cells() || !this.single()) return
    let data = null
    try {
      if (!navigator.clipboard || typeof navigator.clipboard.read !== 'function') throw new Error('no asynchronous clipboard')
      data = { html: '', text: '' }
      for (const item of await navigator.clipboard.read()) {
        if (!data.html && item.types.includes('text/html')) data.html = await (await item.getType('text/html')).text()
        if (!data.text && item.types.includes('text/plain')) data.text = await (await item.getType('text/plain')).text()
      }
    } catch {
      data = null
    }
    if (data) return this.pasteData({ ...data, what })
    if (this.source) return this.pasteData({ local: true, what })
    toast('The browser did not let this page read the clipboard: press Ctrl+V to paste.', 'info')
  }

  // Paste Special (Ctrl+Alt+V): what to paste, then the paste.
  async pasteSpecial () {
    if (!this.cells() || !this.single()) return
    const choices = [['all', 'All'], ['formulas', 'Formulas'], ['values', 'Values'], ['formats', 'Formats']]
    const name = 'paste-special-what'
    const radios = choices.map(([value, label], i) => {
      const input = el('input', { type: 'radio', name, value, id: `paste-special-${value}`, checked: i === 0 })
      return el('div', { className: 'field check' }, input, el('label', { text: label, attrs: { for: input.id } }))
    })
    const dialog = new Dialog({
      title: 'Paste Special',
      id: 'paste-special',
      className: 'small',
      body: [el('fieldset', {}, el('legend', { text: 'Paste' }), radios)],
      buttons: [{
        label: 'OK',
        primary: true,
        action: () => (dialog.element.querySelector(`input[name="${name}"]:checked`) || {}).value || 'all'
      }, { label: 'Cancel', value: null }],
      restore: () => this.app.focusGrid()
    })
    const what = await dialog.open()
    if (what) await this.paste(what)
  }

  // What a paste carries: `{ html, text }` from the clipboard, or `local`
  // for this tab's own copy (Enter, a refused read, and any paste while the
  // browser kept the copy off the clipboard). `end` closes the copy.
  async pasteData ({ html = '', text = '', what = 'all', local = false, end = false } = {}) {
    if (!this.cells() || !this.single()) return
    const own = this.source
    if (own && own.written === 'blocked') local = true
    let marker = local ? null : markerOf(html)
    if (local) {
      if (!own) return
      marker = {
        raw: own.marker, instance: this.api.instance, generation: this.api.generation,
        revision: this.api.revision, sheet: own.sheet, range: rangeName(own.range)
      }
    }
    if (marker && marker.instance === this.api.instance && marker.generation === this.api.generation && this.sheetExists(marker.sheet)) {
      if (marker.raw && marker.raw === this.consumed) return
      const mine = own && (local || (own.marker && own.marker === marker.raw)) ? own : null
      const range = mine ? mine.range : parseRange(marker.range)
      if (mine || await this.unchanged(marker, range)) {
        const cut = Boolean(mine && mine.cut && what === 'all')
        await this.pasteRange({ sheet: marker.sheet, range }, what, cut)
        if (cut) this.consumed = mine.marker
        if (cut || end) this.clear()
        return
      }
    }
    if (what === 'formats') {
      toast('Formatting pastes only from cells copied in this workbook.', 'info')
      return
    }
    if (!text) {
      toast('There is nothing on the clipboard to paste.', 'info')
      return
    }
    await this.pasteText(text)
  }

  sheetExists (key) {
    const workbook = this.app.workbook()
    return Boolean(workbook && workbook.sheets.some((sheet) => sheet.key === key && sheet.kind === 'worksheet'))
  }

  // Whether nothing since the copy touched its cells (the ring's word): the
  // service's `paste` then moves what the clipboard shows.
  async unchanged (marker, range) {
    if (!range) return false
    if (marker.revision >= this.api.revision) return true
    let answer
    try {
      answer = await this.api.get(`changes?since=${marker.revision}&wait=0`, { quiet: true })
    } catch {
      return false
    }
    if (!answer || answer.reset) return false
    for (const change of answer.changes || []) {
      if (change.sheets) return false
      for (const item of change.changed || []) {
        if (item.sheet !== marker.sheet) continue
        if (change.structural) return false
        const touched = parseRange(item.range)
        if (!touched || intersects(touched, range)) return false
      }
    }
    return true
  }

  // The service's `paste` at the selection's top-left; the pasted range is
  // selected once it has landed.
  async pasteRange (from, what, cut) {
    const sheet = this.app.sheetKey()
    const at = { row: this.selection.range.r0, column: this.selection.range.c0 }
    const target = clamp({
      r0: at.row,
      c0: at.column,
      r1: at.row + from.range.r1 - from.range.r0,
      c1: at.column + from.range.c1 - from.range.c0
    })
    const edit = {
      op: 'paste',
      from: { sheet: from.sheet, range: rangeName(from.range) },
      to: { sheet, ref: refName(at.row, at.column) },
      what,
      cut
    }
    const marks = [target]
    if (cut && from.sheet === sheet) marks.push(from.range)
    await this.app.edit(edit, { sheet, ranges: marks })
    if (this.app.sheetKey() === sheet) this.selection.selectRange(target, at)
  }

  async pasteText (text) {
    const sheet = this.app.sheetKey()
    const at = { row: this.selection.range.r0, column: this.selection.range.c0 }
    const { rows, columns } = textExtent(text)
    const guess = clamp({ r0: at.row, c0: at.column, r1: at.row + rows - 1, c1: at.column + columns - 1 })
    const answer = await this.app.edit({ op: 'pasteText', sheet, ref: refName(at.row, at.column), text }, { sheet, ranges: [guess] })
    if (this.app.sheetKey() !== sheet) return
    const landed = (answer.changed || [])
      .filter((item) => item.sheet === sheet)
      .map((item) => parseRange(item.range))
      .find((range) => range && range.r0 === at.row && range.c0 === at.column)
    if (landed) this.selection.selectRange(landed, at)
  }

  // Format Painter ----------------------------------------------------------------

  // One click paints once; a double click keeps painting until Escape or
  // the button again.
  startPainter (sticky) {
    if (!this.cells() || !this.single()) return
    this.clear()
    this.painter = { sheet: this.app.sheetKey(), range: clamp(this.selection.range), sticky, painter: true }
    this.pressPainter(true)
    this.status.setCopying(sticky ? 'Format Painter: select cells to format; Esc stops' : 'Format Painter: select cells to format')
    this.host.dataset.painter = sticky ? 'sticky' : 'once'
    this.grid.invalidate()
  }

  endPainter () {
    if (!this.painter) return
    this.painter = null
    this.pressPainter(false)
    delete this.host.dataset.painter
    this.status.setCopying(null)
    this.grid.invalidate()
  }

  pressPainter (on) {
    const button = document.querySelector('[data-command="formatPainter"]')
    if (button) button.setAttribute('aria-pressed', String(on))
  }

  // The source's formats onto the selection: once at its top-left, or
  // tiled when the selection is a whole multiple of the source.
  async paint () {
    const painter = this.painter
    if (!painter || !this.grid.geometry || this.grid.sheet !== this.app.sheetKey()) return
    if (!painter.sticky) this.endPainter()
    const sheet = this.app.sheetKey()
    const target = clamp(this.selection.range)
    const height = painter.range.r1 - painter.range.r0 + 1
    const width = painter.range.c1 - painter.range.c0 + 1
    const rows = target.r1 - target.r0 + 1
    const columns = target.c1 - target.c0 + 1
    const tiled = rows % height === 0 && columns % width === 0 && (rows / height) * (columns / width) <= MAX_TILES
    const edits = []
    const from = { sheet: painter.sheet, range: rangeName(painter.range) }
    for (let r = target.r0; r <= (tiled ? target.r1 : target.r0); r += height) {
      for (let c = target.c0; c <= (tiled ? target.c1 : target.c0); c += width) {
        edits.push({ op: 'paste', from, to: { sheet, ref: refName(r, c) }, what: 'formats', cut: false })
      }
    }
    const painted = tiled ? target : clamp({ r0: target.r0, c0: target.c0, r1: target.r0 + height - 1, c1: target.c0 + width - 1 })
    try {
      await this.app.edit(edits.length === 1 ? edits[0] : { op: 'batch', edits }, { sheet, ranges: [painted] })
    } catch (error) {
      if (!(error instanceof ApiError)) throw error
    }
  }

  // The marquee ---------------------------------------------------------------------

  show (source) {
    this.source = source
    this.status.setCopying('Select destination and press ENTER or choose Paste')
    this.grid.invalidate()
  }

  // A copy and the Format Painter never stand together: each ends the other.
  clear () {
    if (!this.source) return
    this.source = null
    this.status.setCopying(null)
    this.grid.invalidate()
  }

  // Marching ants round the source on its sheet, clipped to the cells'
  // pane; the animation is CSS's, still under reduced motion.
  place () {
    const source = this.painter || this.source
    const viewport = this.grid.viewport
    if (!source || !viewport || !this.grid.geometry || this.grid.sheet !== source.sheet) {
      if (!this.clip.hidden) this.clip.hidden = true
      delete this.host.dataset.marquee
      return
    }
    const rect = viewport.rect(source.range)
    const width = Math.max(0, viewport.width - viewport.headerWidth)
    const height = Math.max(0, viewport.height - viewport.headerHeight)
    const x0 = Math.max(-4, rect.x - viewport.headerWidth - 1)
    const y0 = Math.max(-4, rect.y - viewport.headerHeight - 1)
    const x1 = Math.min(width + 4, rect.x - viewport.headerWidth + rect.w)
    const y1 = Math.min(height + 4, rect.y - viewport.headerHeight + rect.h)
    const clip = this.clip.style
    clip.left = viewport.headerWidth + 'px'
    clip.top = viewport.headerHeight + 'px'
    clip.width = width + 'px'
    clip.height = height + 'px'
    const ants = this.ants.style
    ants.left = x0 + 'px'
    ants.top = y0 + 'px'
    ants.width = Math.max(0, x1 - x0) + 'px'
    ants.height = Math.max(0, y1 - y0) + 'px'
    const kind = source.painter ? 'painter' : source.cut ? 'cut' : 'copy'
    this.ants.dataset.kind = kind
    this.host.dataset.marquee = kind
    this.clip.hidden = x1 <= x0 || y1 <= y0
  }
}
