// The editor (§10): the in-cell editor, the formula bar, the Name Box, and
// the formula assist over them.
//
// Typing on the sheet replaces the active cell (Enter mode); F2, a double
// click or a click in the formula bar edit its entry (Edit mode). Enter, Tab
// and their Shift forms commit and move, Ctrl+Enter fills the selection,
// Alt+Enter breaks the line, Esc cancels, Backspace clears and edits. A
// commit is one `setEntries` or `fillEntry` edit carrying the base revision
// (a `fillEntry` names the active cell as `ref`, where the text is entered
// and from which its references translate, as Excel's Ctrl+Enter does);
// what was typed is never drawn as the cell's value - the cell is hatched
// until the service answers and its tile arrives. A refused entry reopens
// with the text kept, so nothing typed is lost.
//
// Writing a formula, the editor helps as Excel's does. Where the caret
// follows `( , + - * / ^ & = < >` (point mode), a click or a drag on the
// sheet, or an arrow in Enter mode, puts a reference to those cells at the
// caret, and the next one replaces it; the status bar says Point. F4 cycles
// the `$`s of the reference at the caret. The references are coloured in
// the text and framed in the same colours on the grid; a name being typed
// offers the functions (`GET functions`) and the workbook's names that
// start so, Tab taking the one highlighted; inside a call a tip shows the
// function's arguments, the one at the caret in bold. How a reference is
// spelled, where the references are and what F4 makes of one are the
// service's answers (`POST formula/assist`): the editor asks and places the
// text it is given.

import { ApiError, toast } from './api.js'
import { refName, rangeName, parseRange, parseRef, contains, normalize, MAX_ROWS, MAX_COLUMNS } from './geometry.js'
import { REFERENCE_COLORS } from './render.js'

const PAD = 2
const LOOKUP_DELAY = 60
const ENTRIES = 2048
// The references of a formula are asked for this long after the last key.
const TOKENS_DELAY = 60
// The most names the function list shows at once (it scrolls past them).
const LIST_ROWS = 8

const ARROWS = {
  ArrowUp: [-1, 0, 'up'],
  ArrowDown: [1, 0, 'down'],
  ArrowLeft: [0, -1, 'left'],
  ArrowRight: [0, 1, 'right']
}

// The formula text rules the assist reads locally -------------------------------

// What a pointed reference may follow: an operator, a separator or an
// opening parenthesis (spaces between them allowed).
const POINT_AFTER = '(,+-*/^&=<>'
const NAME_CHAR = /[A-Za-z0-9_.]/
const NAME_START = /[A-Za-z_]/

const isFormula = (text) => text.startsWith('=')

// Whether `at` is inside a string literal: an odd count of quotes before
// it (`""` inside a string pairs up).
export const inString = (text, at) => {
  let quotes = 0
  for (let i = 0; i < at; i++) if (text.charCodeAt(i) === 34) quotes++
  return quotes % 2 === 1
}

// Whether a reference may be pointed in at the caret `at` (point mode).
export const canPoint = (text, at) => {
  if (!isFormula(text) || at < 1 || inString(text, at)) return false
  let i = at - 1
  while (i > 0 && text[i] === ' ') i--
  return POINT_AFTER.includes(text[i])
}

// The name being typed at the caret `at` - letters, digits, `_` and `.`,
// begun by a letter or `_`, after an operator, a separator, a space or the
// `=` - as { start, word }, or null (inside a string, mid-word, already a
// call, or after `!`, `$` or `:`).
export const wordAt = (text, at) => {
  if (!isFormula(text) || inString(text, at)) return null
  if (at < text.length && (NAME_CHAR.test(text[at]) || text[at] === '(')) return null
  let start = at
  while (start > 1 && NAME_CHAR.test(text[start - 1])) start--
  if (start === at || !NAME_START.test(text[start])) return null
  const before = text[start - 1]
  if (!POINT_AFTER.includes(before) && before !== ' ' && before !== '\n') return null
  return { start, word: text.slice(start, at) }
}

// The innermost named call the caret `at` is inside, and which of its
// arguments (0-based) the caret is in; null outside every call.
export const callAt = (text, at) => {
  if (!isFormula(text)) return null
  const stack = []
  let quoted = false
  let sheet = false
  let braces = 0
  for (let i = 1; i < at && i < text.length; i++) {
    const ch = text[i]
    if (quoted) {
      if (ch === '"') quoted = false
      continue
    }
    if (sheet) {
      if (ch === "'") sheet = false
      continue
    }
    if (ch === '"') quoted = true
    else if (ch === "'") sheet = true
    else if (ch === '{') braces++
    else if (ch === '}') braces = Math.max(0, braces - 1)
    else if (ch === '(') {
      let j = i
      while (j > 1 && NAME_CHAR.test(text[j - 1])) j--
      const name = j < i && NAME_START.test(text[j]) ? text.slice(j, i).replace(/^_xl(fn|ws)\./i, '') : null
      stack.push({ name, index: 0 })
    } else if (ch === ')') stack.pop()
    else if (ch === ',' && braces === 0 && stack.length > 0) stack[stack.length - 1].index++
  }
  for (let k = stack.length - 1; k >= 0; k--) if (stack[k].name) return stack[k]
  return null
}

// A signature's parameters as written: `SUM(number1, [number2], ...)` is
// `number1`, `[number2]` and `...`; a bracketed group keeps its commas.
export const parameters = (signature) => {
  const open = signature.indexOf('(')
  const close = signature.lastIndexOf(')')
  if (open < 0 || close < open) return []
  const out = []
  let depth = 0
  let part = ''
  for (const ch of signature.slice(open + 1, close)) {
    if (ch === '[') depth++
    else if (ch === ']') depth--
    if (ch === ',' && depth === 0) {
      out.push(part.trim())
      part = ''
    } else part += ch
  }
  if (part.trim()) out.push(part.trim())
  return out
}

// Which parameter argument `index` fills: a bracketed group takes as many
// arguments as it names, and past the listed ones a trailing `...` repeats
// the parameter before it; -1 when none does.
export const parameterAt = (list, index) => {
  let at = 0
  for (let i = 0; i < list.length; i++) {
    if (list[i] === '...') return i > 0 ? i - 1 : -1
    const size = list[i].split(',').length
    if (index < at + size) return i
    at += size
  }
  return -1
}

// Tokens answered for `before`, carried to `after` while the answer for it
// is on its way: those wholly before the change keep their place, those
// wholly after it move with the text, and those it touched - or grew, a
// character of a reference typed against one - go.
const REFERENCE_CHAR = /[A-Za-z0-9_.$:!']/
export const shiftTokens = (tokens, before, after) => {
  let head = 0
  const most = Math.min(before.length, after.length)
  while (head < most && before[head] === after[head]) head++
  let tail = 0
  while (tail < most - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++
  const delta = after.length - before.length
  const cut = before.length - tail
  const out = []
  for (const token of tokens) {
    if (token.end < head || (token.end === head && !REFERENCE_CHAR.test(after[head] || ''))) out.push(token)
    else if (token.start > cut || (token.start === cut && !REFERENCE_CHAR.test(after[cut + delta - 1] || ''))) {
      out.push({ ...token, start: token.start + delta, end: token.end + delta })
    }
  }
  return out
}

// Each token's colour: one per distinct range, in order of first
// appearance, cycling through the palette.
export const tokenColors = (tokens) => {
  const seen = new Map()
  return tokens.map((token) => {
    const range = parseRange(token.range)
    const key = token.sheet + '!' + (range ? rangeName(range) : token.range)
    if (!seen.has(key)) seen.set(key, seen.size % REFERENCE_COLORS)
    return seen.get(key)
  })
}

// The range a pointer's press and its current place span: cells, whole
// columns, whole rows, or the sheet from the corner.
export const pointRange = (from, to) => {
  if (from.kind === 'column') return normalize(0, from.column, MAX_ROWS - 1, to.column ?? from.column)
  if (from.kind === 'row') return normalize(from.row, 0, to.row ?? from.row, MAX_COLUMNS - 1)
  if (from.kind === 'corner') return { r0: 0, c0: 0, r1: MAX_ROWS - 1, c1: MAX_COLUMNS - 1 }
  return normalize(from.row, from.column, to.row ?? from.row, to.column ?? from.column)
}

// A text field's references in colour ---------------------------------------------

// A backdrop behind a text field draws its text with each reference in its
// colour; the field's own glyphs go transparent over it while it shows,
// its caret and selection staying the field's. Forced colours bring the
// field's glyphs back and hide the backdrop.
const COPIED = ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'lineHeight', 'letterSpacing', 'textIndent',
  'textAlign', 'whiteSpace', 'overflowWrap', 'wordBreak', 'tabSize', 'paddingTop', 'paddingRight', 'paddingBottom',
  'paddingLeft', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'boxSizing']

class Backdrop {
  constructor (field, className) {
    this.field = field
    this.node = document.createElement('div')
    this.node.className = 'editor-backdrop ' + className
    this.node.setAttribute('aria-hidden', 'true')
    this.node.hidden = true
    field.before(this.node)
    field.addEventListener('scroll', () => this.scroll())
  }

  get shown () {
    return !this.node.hidden
  }

  // `runs`: [{ start, end, color }] in order, over `text`.
  show (text, runs, { color = '', background = '' } = {}) {
    const parts = []
    let at = 0
    for (const run of runs) {
      if (run.start < at || run.end > text.length) continue
      if (run.start > at) parts.push(document.createTextNode(text.slice(at, run.start)))
      const span = document.createElement('span')
      span.className = 'ref ref-' + ((run.color % REFERENCE_COLORS) + 1)
      span.dataset.color = String(run.color)
      span.textContent = text.slice(run.start, run.end)
      parts.push(span)
      at = run.end
    }
    // A trailing line break needs a character after it to hold its line.
    parts.push(document.createTextNode(text.slice(at) + (text.endsWith('\n') ? ' ' : '')))
    this.node.replaceChildren(...parts)
    this.node.style.color = color
    this.node.style.backgroundColor = background
    this.node.hidden = false
    this.field.classList.add('coloured')
    this.place()
  }

  hide () {
    if (this.node.hidden) return
    this.node.hidden = true
    this.node.replaceChildren()
    this.field.classList.remove('coloured')
  }

  // Over the field exactly: its box, its metrics, its scroll.
  place () {
    if (this.node.hidden) return
    const field = this.field
    const computed = getComputedStyle(field)
    const style = this.node.style
    for (const name of COPIED) style[name] = computed[name]
    style.left = field.offsetLeft + 'px'
    style.top = field.offsetTop + 'px'
    style.width = field.offsetWidth + 'px'
    style.height = field.offsetHeight + 'px'
    this.scroll()
  }

  scroll () {
    this.node.scrollTop = this.field.scrollTop
    this.node.scrollLeft = this.field.scrollLeft
  }
}

export class Editor extends EventTarget {
  constructor ({ app, api, grid, selection, tiles, styles, keyboard, host, nameBox, formulaBar }) {
    super()
    this.app = app
    this.api = api
    this.grid = grid
    this.selection = selection
    this.tiles = tiles
    this.styles = styles
    this.keyboard = keyboard
    this.host = host
    this.nameBox = nameBox
    this.bar = formulaBar
    // { sheet, row, column, ref, original, text, mode: 'enter'|'edit',
    // surface: 'cell'|'bar', ranges, selection, point, tokens }
    this.state = null
    this.entries = new Map()
    this.lookup = 0
    this.lookupTimer = 0
    this.opening = 0
    this.dragging = false
    this.warned = 0
    // The formula assist: the function catalogue (loaded once), the asks
    // on their way (a later one drops an earlier answer), the list offered.
    this.catalog = null
    this.byName = new Map()
    this.asks = 0
    this.tokensTimer = 0
    this.list = null
    this.dismissed = null

    this.box = document.createElement('textarea')
    this.box.className = 'cell-editor'
    this.box.id = 'cell-editor'
    this.box.hidden = true
    this.box.spellcheck = false
    this.box.setAttribute('autocomplete', 'off')
    this.box.setAttribute('aria-label', 'Cell editor')
    this.box.setAttribute('aria-multiline', 'true')
    host.append(this.box)
    this.backdrops = {
      cell: new Backdrop(this.box, 'cell-backdrop'),
      bar: new Backdrop(this.bar, 'formula-backdrop')
    }
    this.buildAssist()

    this.box.addEventListener('input', () => this.typed(this.box))
    this.bar.addEventListener('input', () => this.typed(this.bar))
    this.box.addEventListener('keydown', (event) => this.key(event))
    this.bar.addEventListener('keydown', (event) => this.barKey(event))
    this.box.addEventListener('focus', () => this.surface('cell'))
    this.bar.addEventListener('focus', () => this.barFocus())
    // The caret moving (a click in the text, an arrow in Edit mode) moves
    // the list and the tip with it.
    document.addEventListener('selectionchange', () => this.caretMoved())
    for (const field of [this.box, this.bar]) {
      field.addEventListener('keyup', () => this.caretMoved())
      field.addEventListener('pointerup', () => this.caretMoved())
    }
    window.addEventListener('resize', () => this.placeAssist())

    keyboard.use((event) => this.gridKey(event))
    grid.addEventListener('activate', () => this.editActive('cell'))
    // A press on the sheet commits first, as Excel does, then selects - or,
    // where the caret can take a reference, points at the cells instead.
    grid.addEventListener('press', (event) => {
      if (!this.state) return
      if (event.detail.hit && this.pointable()) {
        event.preventDefault()
        return
      }
      this.commit()
    })
    grid.addEventListener('point', (event) => this.pointer(event.detail))
    grid.addEventListener('frame', () => {
      if (this.state) this.place()
    })
    selection.addEventListener('change', () => {
      if (!this.dragging) this.showName()
      if (!this.state) this.showEntry()
    })
    // A change here or in another tab over the active cell: its entry again.
    app.events.addEventListener('changed', (event) => {
      if (this.state) return
      const change = event.detail
      const sheet = this.app.sheetKey()
      const { row, column } = this.selection.active
      const over = (item) => {
        const range = parseRange(item.range)
        return item.sheet === sheet && (change.structural || !range || contains(range, row, column))
      }
      if ((change.changed || []).some(over)) this.showEntry()
    })
    grid.addEventListener('dragsize', (event) => this.showName(event.detail))
    grid.addEventListener('dragging', (event) => {
      this.dragging = event.detail
      if (!event.detail) this.showName()
    })

    nameBox.addEventListener('focus', () => {
      if (this.state) this.commit()
      nameBox.select()
    })
    nameBox.addEventListener('blur', () => this.showName())
    nameBox.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault()
        this.go(nameBox.value)
      } else if (event.key === 'Escape') {
        event.preventDefault()
        this.showName()
        this.app.focusGrid()
      }
    })
  }

  isOpen () {
    return this.state !== null
  }

  get mode () {
    return this.state ? this.state.mode : 'ready'
  }

  announceMode () {
    const s = this.state
    const pointing = Boolean(s && s.point && !s.point.done)
    const mode = s ? (pointing ? 'Point' : s.mode === 'enter' ? 'Enter' : 'Edit') : 'Ready'
    if (s) this.host.dataset.editor = pointing ? 'point' : s.mode
    else delete this.host.dataset.editor
    this.dispatchEvent(new CustomEvent('mode', { detail: mode }))
  }

  // Entries -------------------------------------------------------------------

  entryKey (sheet, ref) {
    return `${this.api.generation}:${this.api.revision}:${sheet}:${ref}`
  }

  // A cell's entry: `=` and its formula, `12%`, `'text` (§3.5). A cell its
  // current tile shows as empty needs no lookup.
  async entryOf (sheet, row, column) {
    const merge = this.grid.geometry && this.grid.geometry.merges.at(row, column)
    if (merge) {
      row = merge.r0
      column = merge.c0
    }
    if (this.tiles.current(sheet, row, column) === null) return ''
    const ref = refName(row, column)
    const key = this.entryKey(sheet, ref)
    if (this.entries.has(key)) return this.entries.get(key)
    const answer = await this.api.get(`sheets/${sheet}/cells/${ref}`, { quiet: true })
    const entry = (answer && answer.entry) ?? ''
    if (this.entries.size > ENTRIES) this.entries.clear()
    this.entries.set(key, entry)
    return entry
  }

  // The formula bar follows the active cell; a slow answer for a cell no
  // longer active never lands.
  showEntry () {
    clearTimeout(this.lookupTimer)
    const ask = ++this.lookup
    const sheet = this.app.sheetKey()
    if (!this.grid.geometry || this.grid.sheet !== sheet) {
      this.bar.value = ''
      return
    }
    const { row, column } = this.selection.active
    const known = this.entries.get(this.entryKey(sheet, refName(row, column)))
    if (known !== undefined) {
      this.bar.value = known
      return
    }
    // Blank until the entry arrives: never the previous cell's.
    this.bar.value = ''
    if (this.tiles.current(sheet, row, column) === null) return
    this.lookupTimer = setTimeout(async () => {
      try {
        const entry = await this.entryOf(sheet, row, column)
        if (ask === this.lookup && !this.state) this.bar.value = entry
      } catch {
        if (ask === this.lookup && !this.state) this.bar.value = ''
      }
    }, LOOKUP_DELAY)
  }

  clearEntry () {
    clearTimeout(this.lookupTimer)
    this.lookup++
    this.bar.value = ''
  }

  // The Name Box: the active cell, or rows by columns while a drag sizes.
  showName (range) {
    if (document.activeElement === this.nameBox) return
    if (range) {
      const rows = range.r1 - range.r0 + 1
      const columns = range.c1 - range.c0 + 1
      this.nameBox.value = rows === 1 && columns === 1 ? refName(range.r0, range.c0) : `${rows}R x ${columns}C`
      return
    }
    const sheet = this.app.sheet()
    this.nameBox.value = sheet && sheet.kind === 'worksheet' ? this.selection.activeRef : ''
  }

  // Go to a reference or a defined name, resolved by the service
  // (`resolve`), on its own sheet: the Name Box's Enter, Go To, a status
  // bar link. Answers null once there, else why not; from the Name Box the
  // reason is also a toast and the box stays selected.
  async go (text, { nameBox = true } = {}) {
    const wanted = text.trim()
    if (!wanted) return 'Type a reference or a name.'
    const refuse = (reason, shown = true) => {
      if (nameBox) {
        if (shown) toast(reason)
        this.nameBox.select()
      }
      return reason
    }
    let answer
    try {
      answer = await this.api.get('resolve?text=' + encodeURIComponent(wanted), { quiet: !nameBox })
    } catch (error) {
      return refuse((error && error.message) || `${wanted} is not a reference or a name.`, false)
    }
    const range = answer && parseRange(answer.range)
    if (!range) return refuse(`${wanted} is not a reference or a name.`)
    const key = answer.sheet ?? this.app.sheetKey()
    const target = ((this.app.workbook() || {}).sheets || []).find((sheet) => sheet.key === key)
    if (!target || target.state !== 'visible' || target.kind !== 'worksheet') return refuse(`${wanted} is not on a visible worksheet.`)
    if (key !== this.app.sheetKey()) await this.app.showSheet(key)
    if (!this.grid.geometry || this.app.sheetKey() !== key) return `${wanted} could not be shown.`
    if (range.r0 === range.r1 && range.c0 === range.c1) this.selection.select(range.r0, range.c0)
    else this.selection.selectRange(range)
    this.grid.ensureVisible(range.r0, range.c0)
    if (nameBox) this.nameBox.blur()
    this.app.focusGrid()
    this.showName()
    return null
  }

  // Opening ------------------------------------------------------------------

  editable () {
    if (this.app.editable()) return true
    const now = Date.now()
    if (now - this.warned > 4000) {
      this.warned = now
      const workbook = this.app.workbook()
      toast(workbook && workbook.readOnly ? 'This workbook is open read-only.' : 'This sheet has no cells to edit.', 'info')
    }
    return false
  }

  // Open on the active cell. `original` is the entry a commit compares
  // against (unknown when typing replaced it unseen).
  begin ({ text = '', mode = 'enter', surface = 'cell', original, caret = null, ranges = null } = {}) {
    if (!this.editable()) return false
    const { row, column } = this.selection.active
    this.grid.ensureVisible(row, column)
    this.state = {
      sheet: this.app.sheetKey(),
      row,
      column,
      ref: refName(row, column),
      original,
      text,
      mode,
      surface,
      ranges: (ranges || this.selection.ranges).map((range) => ({ ...range })),
      selection: this.selection.snapshot(),
      // The reference pointed last, and those whose spelling is on its way.
      point: null,
      spelling: new Set(),
      tokens: null
    }
    this.box.value = text
    this.bar.value = text
    this.box.setAttribute('aria-label', `Editing ${this.state.ref}`)
    this.box.hidden = false
    this.place()
    const target = this.field()
    if (document.activeElement !== target) target.focus({ preventScroll: true })
    const at = caret === null ? text.length : caret
    target.setSelectionRange(at, at)
    this.announceMode()
    this.assistSoon(true)
    return true
  }

  // F2, a double click: the entry, caret at its end.
  async editActive (surface) {
    if (this.state) {
      if (surface === 'cell') this.box.focus({ preventScroll: true })
      return
    }
    if (!this.editable()) return
    const sheet = this.app.sheetKey()
    const { row, column } = this.selection.active
    const ask = ++this.opening
    let entry
    try {
      entry = await this.entryOf(sheet, row, column)
    } catch {
      return
    }
    const now = this.selection.active
    if (ask !== this.opening || this.state || sheet !== this.app.sheetKey() || now.row !== row || now.column !== column) return
    this.begin({ text: entry, mode: 'edit', surface, original: entry })
  }

  // The formula bar took the focus: edit the active cell there.
  async barFocus () {
    if (this.state) {
      this.surface('bar')
      return
    }
    if (!this.app.editable()) return
    const sheet = this.app.sheetKey()
    const { row, column } = this.selection.active
    const ask = ++this.opening
    const shown = this.bar.value
    let entry
    try {
      entry = await this.entryOf(sheet, row, column)
    } catch {
      return
    }
    if (ask !== this.opening || this.state || document.activeElement !== this.bar) return
    // Keys typed while the entry was on its way stay; the caret too.
    const text = this.bar.value !== shown ? this.bar.value : entry
    const caret = this.bar.value === text ? this.bar.selectionStart : text.length
    this.begin({ text, mode: 'edit', surface: 'bar', original: entry, caret })
  }

  // After the sheet or the workbook changed: the formula bar takes typing
  // only where cells can be edited.
  refresh () {
    this.bar.readOnly = !this.app.editable()
    this.showName()
    if (!this.state) this.showEntry()
  }

  surface (which) {
    if (!this.state || this.state.surface === which) return
    this.state.surface = which
    if (which === 'bar') this.state.mode = 'edit'
    this.announceMode()
    this.caretMoved()
  }

  // The field being typed in, and its caret.
  field () {
    return this.state && this.state.surface === 'bar' ? this.bar : this.box
  }

  caret () {
    return this.field().selectionEnd
  }

  // Back to the text being edited, its caret where it was (after a dialog
  // or a pointer on the sheet took the focus).
  focus () {
    if (!this.state) return
    const target = this.field()
    const start = target.selectionStart
    const end = target.selectionEnd
    if (document.activeElement !== target) target.focus({ preventScroll: true })
    target.setSelectionRange(start, end)
  }

  // A printable key on the sheet: the first opens the editor, the rest
  // (queued behind a key still resolving) join it.
  type (text) {
    this.keyboard.queue(() => {
      if (this.state) {
        this.insert(text)
        return
      }
      this.begin({ text, mode: 'enter' })
    })
  }

  insert (text) {
    const s = this.state
    if (!s) return
    const target = this.field()
    target.setRangeText(text, target.selectionStart, target.selectionEnd, 'end')
    this.typed(target)
  }

  // What the user typed: the reference pointed last is done with (its
  // spelling, still on its way, lands all the same).
  typed (source) {
    const s = this.state
    if (!s) return
    s.text = source.value
    if (source !== this.box) this.box.value = s.text
    if (source !== this.bar) this.bar.value = s.text
    this.dismissed = null
    if (s.point && !s.point.done) {
      s.point.done = true
      this.announceMode()
    }
    this.place()
    this.paintReferences()
    this.assistSoon()
  }

  // Text the assist wrote, and the caret after it.
  setText (text, caret) {
    const s = this.state
    if (!s) return
    s.text = text
    this.box.value = text
    this.bar.value = text
    const target = this.field()
    if (document.activeElement !== target) target.focus({ preventScroll: true })
    target.setSelectionRange(caret, caret)
    this.place()
    this.paintReferences()
    this.assistSoon()
  }

  // Where the box sits: over the cell (its merge), as wide as the text needs
  // up to the pane's edge, then wrapping downward. Font, colour and fill are
  // the cell's own, at the zoom.
  place () {
    const s = this.state
    const viewport = this.grid.viewport
    if (!s || !viewport || !this.grid.geometry || this.grid.sheet !== s.sheet) {
      this.box.hidden = true
      this.backdrops.cell.hide()
      return
    }
    this.box.hidden = false
    const geometry = this.grid.geometry
    const zoom = this.grid.zoom
    const merge = geometry.merges.at(s.row, s.column)
    const rect = viewport.rect(merge || { r0: s.row, c0: s.column, r1: s.row, c1: s.column })
    const style = this.styles.get(this.styleOf(s))
    const font = this.styles.font(style, zoom)
    const px = this.styles.fontPx(style, zoom)
    const pad = Math.max(1, Math.round(PAD * zoom))
    const line = Math.ceil(px * 1.22)
    const box = this.box.style
    box.font = font
    box.lineHeight = line + 'px'
    box.color = style.font.color || ''
    box.caretColor = style.font.color || ''
    box.backgroundColor = style.fill || ''
    box.paddingLeft = pad + 'px'
    box.paddingRight = pad + 'px'
    const lines = s.text.split('\n')
    let widest = 0
    for (const text of lines) widest = Math.max(widest, this.grid.painter.measure(font, text || ' '))
    const minWidth = Math.max(8, rect.w + 1)
    const minHeight = Math.max(line, rect.h + 1)
    const maxWidth = Math.max(minWidth, viewport.width - rect.x)
    const width = Math.min(maxWidth, Math.max(minWidth, Math.ceil(widest + 2 * pad + px)))
    box.left = (rect.x - 1) + 'px'
    box.top = (rect.y - 1) + 'px'
    box.width = width + 'px'
    // Text sits where the cell draws it: at the bottom unless aligned up.
    const block = line * Math.max(1, lines.length)
    const room = Math.max(0, minHeight - 2 - block)
    box.paddingTop = (style.vertical === 'top' ? 0 : style.vertical === 'center' ? Math.floor(room / 2) : room) + 'px'
    box.paddingBottom = '0px'
    box.height = minHeight + 'px'
    const need = this.box.scrollHeight
    if (need > minHeight) {
      box.paddingTop = '0px'
      box.height = Math.min(Math.max(need, block + 2), Math.max(minHeight, viewport.height - rect.y)) + 'px'
    }
    this.backdrops.cell.place()
    this.placeAssist()
  }

  styleOf (s) {
    const found = this.tiles.cell(s.sheet, s.row, s.column)
    if (found) return found.style
    return (this.grid.geometry && this.grid.geometry.blankStyle(s.row, s.column)) || 0
  }

  // Closing ------------------------------------------------------------------

  close () {
    this.state = null
    this.opening++
    this.asks++
    clearTimeout(this.tokensTimer)
    this.hideList()
    this.hideTip()
    this.backdrops.cell.hide()
    this.backdrops.bar.hide()
    if (this.grid.references.length > 0) this.grid.setReferences([])
    this.box.hidden = true
    this.box.value = ''
    if (document.activeElement === this.box || document.activeElement === this.bar) this.app.focusGrid()
    this.showEntry()
    this.announceMode()
  }

  cancel () {
    if (!this.state) return
    this.close()
    this.app.focusGrid()
  }

  // Commit what was typed, then `move`. With `fill`, the text goes into every
  // cell of the selection it was typed over (Ctrl+Enter). The editor closes
  // at once; the answer arrives later and clears the hatch.
  commit ({ move = null, fill = false } = {}) {
    const s = this.state
    if (!s) return Promise.resolve(null)
    const text = s.text
    this.close()
    this.app.focusGrid()
    if (move) {
      move()
      this.grid.ensureVisible(this.selection.active.row, this.selection.active.column)
    }
    if (!fill && s.original !== undefined && text === s.original) return Promise.resolve(null)
    const merge = this.grid.geometry && this.grid.geometry.merges.at(s.row, s.column)
    const edit = fill
      ? { op: 'fillEntry', sheet: s.sheet, ranges: s.ranges.map(rangeName), text, ref: s.ref }
      : { op: 'setEntries', sheet: s.sheet, entries: [{ ref: s.ref, text }] }
    const ranges = fill ? s.ranges : [merge || { r0: s.row, c0: s.column, r1: s.row, c1: s.column }]
    return this.app.edit(edit, { sheet: s.sheet, ranges, quiet: true }).catch((error) => {
      this.refused(s, text, fill, error)
      return null
    })
  }

  // The service refused the entry. A stale base (another tab went first) or
  // an entry it could not take reopens the editor on its cell with the text
  // kept, unless the user has moved on to another edit; then the toast
  // carries the text.
  refused (s, text, fill, error) {
    if (!(error instanceof ApiError)) {
      toast(String((error && error.message) || error))
      return
    }
    // Kept: a stale base, an entry the service could not take, no answer.
    const keep = error.kind === 'stale_base' || error.status === 0 || error.status === 400 || error.status === 413 || error.status === 422
    const message = error.kind === 'stale_base'
      ? 'Another tab changed this sheet first. Your entry is kept: press Enter to apply it again.'
      : error.message
    if (!keep || this.state || this.app.sheetKey() !== s.sheet || !this.app.editable()) {
      toast(keep ? `${message} (${s.ref}: ${text})` : message)
      return
    }
    toast(message)
    this.selection.bind(this.grid.geometry, s.selection)
    this.begin({ text, mode: 'edit', surface: 'cell', original: s.original, ranges: s.ranges })
    this.host.dataset.kept = error.kind || String(error.status)
  }

  // Point mode -----------------------------------------------------------------

  // The reference pointed last, while it is still what the caret follows.
  pointed () {
    const s = this.state
    const p = s && s.point
    return p && !p.done && p.text === s.text && this.caret() === p.end ? p : null
  }

  // Whether a click or an arrow now points at cells: a formula whose caret
  // takes a reference, on the sheet the grid shows.
  pointable () {
    const s = this.state
    if (!s || !this.grid.geometry || this.grid.sheet !== s.sheet) return false
    return Boolean(this.pointed()) || canPoint(s.text, this.caret())
  }

  // A press, a drag or its end on the sheet (grid.js `point` events),
  // taken in order: Shift grows the reference pointed last, Ctrl adds
  // another after a comma once the last one's spelling has landed, as
  // Excel's Ctrl+click does.
  pointer (detail) {
    this.pointing = (this.pointing || Promise.resolve()).then(() => this.pointerNow(detail)).catch(() => {})
  }

  async pointerNow ({ phase, hit, extend = false, add = false }) {
    const s = this.state
    if (!s || !hit) return
    if (phase === 'end') {
      this.focus()
      return
    }
    let last = this.pointed()
    if (phase === 'start' && add && last) {
      // After the reference pointed last, once it is spelled; anything
      // typed after it meanwhile moves along.
      const before = last
      await before.landed
      if (this.state !== s || s.text.slice(0, before.end) !== before.text.slice(0, before.end)) return
      before.done = true
      this.setText(s.text.slice(0, before.end) + ',' + s.text.slice(before.end), before.end + 1)
      last = null
    }
    const from = phase === 'start' ? (extend && last ? last.from : hit) : (last ? last.from : hit)
    if (phase === 'move' && !last) return
    this.pointTo(this.selection.merged(pointRange(from, hit)), from, hit)
    this.focus()
  }

  // An arrow in Enter mode on a formula taking a reference: the pointed
  // cell moves (from the edited cell at first), Shift grows the pointed
  // range from where it began, Ctrl goes to the data edge (`edge`).
  async pointKey (dRow, dColumn, direction, { extend = false, jump = false } = {}) {
    const s = this.state
    const last = this.pointed()
    // From the cell pointed last (a header pointed at starts again from
    // the edited cell).
    const cells = last && last.to.kind === 'cell' && last.from.kind === 'cell'
    const at = cells ? last.to : { kind: 'cell', row: s.row, column: s.column }
    const from = cells && extend ? last.from : null
    let to
    if (jump) {
      const asked = this.asks
      let answer
      try {
        answer = await this.api.get(`sheets/${s.sheet}/edge?from=${refName(at.row, at.column)}&direction=${direction}`, { quiet: true })
      } catch {
        return
      }
      const found = answer && parseRef(answer.ref || '')
      if (this.state !== s || !found || this.asks !== asked) return
      to = { kind: 'cell', row: found.row, column: found.column }
    } else {
      const next = this.selection.step(at.row, at.column, dRow, dColumn)
      to = { kind: 'cell', row: next.row, column: next.column }
    }
    const anchor = from || to
    this.pointTo(this.selection.merged(pointRange(anchor, to)), anchor, to)
    this.grid.ensureVisible(to.row, to.column)
  }

  // Point at `range`: it is framed at once, and the service spells the
  // reference (`assist` `reference`), which replaces the one pointed before
  // or goes in at the caret.
  pointTo (range, from, to) {
    const s = this.state
    const last = this.pointed()
    const target = this.field()
    const start = last ? last.start : target.selectionStart
    const end = last ? last.end : target.selectionEnd
    const point = { start, end, text: s.text, from, to, range, ask: ++this.asks, done: false }
    s.point = point
    s.spelling.add(point)
    point.landed = this.spell(s, point).finally(() => s.spelling.delete(point))
    this.announceMode()
    this.paintReferences()
  }

  // The service's spelling of a pointed range, put in its place. A later
  // point at the same place supersedes it; text typed after it meanwhile
  // does not (it lands before that text, which moves along).
  async spell (s, p) {
    let answer
    try {
      answer = await this.api.post('formula/assist', {
        sheet: s.sheet,
        ref: s.ref,
        text: p.text,
        caret: p.start,
        action: 'reference',
        range: { sheet: this.grid.sheet, range: rangeName(p.range) }
      }, { quiet: true })
    } catch (error) {
      if (this.state === s && s.point === p && error instanceof ApiError) toast(error.message, 'info')
      return
    }
    if (this.state !== s || (s.point !== p && !p.done) || p.ask === null || typeof (answer && answer.text) !== 'string') return
    // Only where the text up to the place is still what it was.
    if (s.text.slice(0, p.end) !== p.text.slice(0, p.end)) return
    const text = s.text.slice(0, p.start) + answer.text + s.text.slice(p.end)
    const end = p.start + answer.text.length
    const delta = end - p.end
    // A reference pointed after this one, spelled or on its way, moves
    // with the text; each keeps the text it was pointed in as far as its
    // place, which is what its own landing compares.
    for (const q of new Set([...s.spelling, s.point])) {
      if (!q || q === p || q.start < p.end) continue
      q.text = text.slice(0, q.end + delta) + q.text.slice(q.end)
      q.start += delta
      q.end += delta
    }
    const caret = this.caret()
    const after = p.done ? (caret >= p.end ? caret + delta : caret) : end
    p.text = p.done ? text.slice(0, end) + p.text.slice(p.end) : text
    p.end = end
    p.ask = null
    this.setText(text, after)
  }

  // F4: the reference at the caret with its `$`s cycled (`assist` `cycle`).
  async cycle () {
    const s = this.state
    if (!s || !isFormula(s.text)) return
    const text = s.text
    const caret = this.caret()
    const point = this.pointed()
    const ask = ++this.asks
    let answer
    try {
      answer = await this.api.post('formula/assist', { sheet: s.sheet, ref: s.ref, text, caret, action: 'cycle' }, { quiet: true })
    } catch (error) {
      if (this.state === s && error instanceof ApiError) toast(error.message, 'info')
      return
    }
    if (this.state !== s || s.text !== text || this.asks !== ask || !answer || typeof answer.text !== 'string') return
    const at = Math.min(answer.text.length, Math.max(0, answer.caret ?? answer.text.length))
    // The pointed reference, cycled, stays the one the arrows move.
    if (point) {
      point.end = at
      point.text = answer.text
      point.ask = null
    }
    this.setText(answer.text, at)
    if (point) {
      this.announceMode()
      this.paintReferences()
    }
  }

  // The assist -------------------------------------------------------------------

  // The functions the service evaluates (`GET functions`), asked once.
  functions () {
    if (!this.catalog) {
      this.catalog = this.api.get('functions', { quiet: true }).then((list) => {
        this.byName = new Map((list || []).map((item) => [item.name.toUpperCase(), item]))
        return list || []
      }, (error) => {
        this.catalog = null
        throw error
      })
    }
    return this.catalog
  }

  // After the text or the caret moved: the list and the tip at once, the
  // references a moment after the last key (`now`: at once).
  assistSoon (now = false) {
    const s = this.state
    if (!s) return
    if (isFormula(s.text) && !this.catalog) this.functions().then(() => this.caretMoved(), () => {})
    this.caretMoved()
    clearTimeout(this.tokensTimer)
    if (!isFormula(s.text)) {
      s.tokens = null
      this.paintReferences()
      return
    }
    this.tokensTimer = setTimeout(() => this.askTokens(), now ? 0 : TOKENS_DELAY)
  }

  // The references in the text (`assist` `tokens`), for their colours.
  async askTokens () {
    const s = this.state
    if (!s || !isFormula(s.text)) return
    const text = s.text
    let answer
    try {
      answer = await this.api.post('formula/assist', { sheet: s.sheet, ref: s.ref, text, caret: this.caret(), action: 'tokens' }, { quiet: true })
    } catch {
      return
    }
    if (this.state !== s || s.text !== text) return
    s.tokens = { text, list: (answer && answer.tokens) || [] }
    this.paintReferences()
  }

  // The references in colour: in both text fields, and framed on the grid
  // where they are on the sheet it shows, the pointed range dashed.
  paintReferences () {
    const s = this.state
    if (!s) return
    const tokens = !s.tokens || !isFormula(s.text) ? [] : s.tokens.text === s.text ? s.tokens.list : shiftTokens(s.tokens.list, s.tokens.text, s.text)
    const colors = tokenColors(tokens)
    const runs = tokens.map((token, i) => ({ start: token.start, end: token.end, color: colors[i] }))
    if (runs.length > 0) {
      const style = this.styles.get(this.styleOf(s))
      if (this.box.hidden) this.backdrops.cell.hide()
      else this.backdrops.cell.show(s.text, runs, { color: style.font.color || '', background: style.fill || '' })
      this.backdrops.bar.show(s.text, runs)
    } else {
      this.backdrops.cell.hide()
      this.backdrops.bar.hide()
    }
    const references = []
    const point = s.point && !s.point.done ? s.point : null
    let pointColor = null
    tokens.forEach((token, i) => {
      const range = parseRange(token.range)
      if (!range || token.sheet !== this.grid.sheet) return
      const pointedHere = point && point.text === s.text && token.start === point.start
      if (pointedHere) pointColor = colors[i]
      else references.push({ range, color: colors[i], dashed: false })
    })
    if (point && this.grid.sheet === s.sheet) {
      if (pointColor === null) {
        const same = tokens.findIndex((token) => token.sheet === this.grid.sheet && sameArea(parseRange(token.range), point.range))
        pointColor = same >= 0 ? colors[same] : new Set(colors).size % REFERENCE_COLORS
      }
      references.push({ range: point.range, color: pointColor, dashed: true })
    }
    this.grid.setReferences(references)
  }

  buildAssist () {
    this.listBox = document.createElement('div')
    this.listBox.className = 'function-list'
    this.listBox.hidden = true
    this.options = document.createElement('ul')
    this.options.id = 'function-options'
    this.options.setAttribute('role', 'listbox')
    this.options.setAttribute('aria-label', 'Functions and names')
    this.listNote = document.createElement('p')
    this.listNote.className = 'function-note'
    this.listNote.id = 'function-note'
    this.listBox.append(this.options, this.listNote)
    // The list never takes the focus: the text keeps it.
    this.listBox.addEventListener('pointerdown', (event) => event.preventDefault())
    this.options.addEventListener('click', (event) => {
      const option = event.target.closest('[role="option"]')
      if (!option || !this.list) return
      this.list.index = Number(option.dataset.index)
      this.pick()
    })
    this.tip = document.createElement('div')
    this.tip.className = 'argument-tip'
    this.tip.id = 'argument-tip'
    this.tip.setAttribute('role', 'tooltip')
    this.tip.hidden = true
    this.tip.addEventListener('pointerdown', (event) => event.preventDefault())
    document.body.append(this.listBox, this.tip)
    for (const field of [this.box, this.bar]) {
      field.setAttribute('aria-autocomplete', 'list')
      field.setAttribute('aria-controls', 'function-options')
    }
  }

  // The caret moved or the text changed: a pointed reference no longer at
  // the caret is done with; the list and the tip follow.
  caretMoved () {
    const s = this.state
    if (!s) return
    const active = document.activeElement
    if (active !== this.box && active !== this.bar && !this.grid.drag) {
      this.hideList()
      this.hideTip()
      return
    }
    if (s.point && !s.point.done && !this.pointed()) {
      s.point.done = true
      this.announceMode()
      this.paintReferences()
    }
    this.updateList()
    this.updateTip()
  }

  // The functions and names starting as the name at the caret does.
  updateList () {
    const s = this.state
    const at = this.caret()
    const found = s && wordAt(s.text, at)
    if (!found || !this.catalog || (this.dismissed && this.dismissed.text === s.text)) {
      this.hideList()
      return
    }
    const wanted = found.word.toUpperCase()
    const items = []
    for (const item of this.byName.values()) {
      if (item.name.toUpperCase().startsWith(wanted)) items.push({ name: item.name, kind: 'function', note: item.description || '' })
    }
    const workbook = this.app.workbook()
    for (const name of (workbook && workbook.names) || []) {
      if ((name.scope === null || name.scope === undefined || name.scope === s.sheet) && name.name.toUpperCase().startsWith(wanted)) {
        items.push({ name: name.name, kind: 'name', note: name.text || '' })
      }
    }
    if (items.length === 0) {
      this.hideList()
      return
    }
    items.sort((a, b) => a.name.localeCompare(b.name, 'en', { sensitivity: 'base' }))
    const before = this.list && this.list.items[this.list.index]
    const kept = before ? items.findIndex((item) => item.name === before.name && item.kind === before.kind) : -1
    this.list = { start: found.start, end: at, items, index: kept >= 0 ? kept : 0 }
    this.options.replaceChildren(...items.map((item, i) => {
      const option = document.createElement('li')
      option.id = 'function-option-' + i
      option.className = 'function-option'
      option.setAttribute('role', 'option')
      option.dataset.index = String(i)
      option.dataset.name = item.name
      option.dataset.kind = item.kind
      const tag = document.createElement('span')
      tag.className = 'function-kind'
      tag.textContent = item.kind === 'function' ? 'fx' : 'name'
      tag.setAttribute('aria-hidden', 'true')
      option.append(tag, document.createTextNode(item.name))
      return option
    }))
    this.listBox.hidden = false
    this.highlight()
    this.hideTip()
    this.placeAssist()
  }

  highlight () {
    const list = this.list
    if (!list) return
    for (const option of this.options.children) {
      const on = Number(option.dataset.index) === list.index
      option.setAttribute('aria-selected', String(on))
      if (!on) continue
      // Scrolled into the list's own view, never the page's.
      const top = option.offsetTop
      const bottom = top + option.offsetHeight
      if (top < this.options.scrollTop) this.options.scrollTop = top
      else if (bottom > this.options.scrollTop + this.options.clientHeight) this.options.scrollTop = bottom - this.options.clientHeight
    }
    const item = list.items[list.index]
    this.listNote.textContent = item ? item.note : ''
    this.field().setAttribute('aria-activedescendant', 'function-option-' + list.index)
  }

  moveList (delta) {
    const list = this.list
    if (!list) return
    list.index = Math.min(list.items.length - 1, Math.max(0, list.index + delta))
    this.highlight()
  }

  // Tab or a click on the list: the name typed so far becomes the one
  // highlighted, a function's with its opening parenthesis.
  pick () {
    const s = this.state
    const list = this.list
    if (!s || !list) return
    const item = list.items[list.index]
    if (!item) return
    const written = item.kind === 'function' ? item.name + '(' : item.name
    const text = s.text.slice(0, list.start) + written + s.text.slice(list.end)
    this.hideList()
    this.setText(text, list.start + written.length)
  }

  hideList () {
    this.list = null
    if (this.listBox.hidden) return
    this.listBox.hidden = true
    this.options.replaceChildren()
    for (const field of [this.box, this.bar]) field.removeAttribute('aria-activedescendant')
  }

  // Escape with the list open closes it until the text changes.
  dismissList () {
    this.dismissed = { text: this.state ? this.state.text : '' }
    this.hideList()
    this.updateTip()
  }

  // The arguments of the call the caret is in, the one at the caret bold.
  updateTip () {
    const s = this.state
    const call = s && this.list === null ? callAt(s.text, this.caret()) : null
    const found = call && this.byName.get(call.name.toUpperCase())
    if (!found || !found.signature) {
      this.hideTip()
      return
    }
    const list = parameters(found.signature)
    const current = parameterAt(list, call.index)
    const parts = [document.createTextNode(found.signature.slice(0, found.signature.indexOf('(') + 1))]
    list.forEach((parameter, i) => {
      if (i > 0) parts.push(document.createTextNode(', '))
      const span = document.createElement(i === current ? 'b' : 'span')
      span.className = 'argument' + (i === current ? ' current' : '')
      span.textContent = parameter
      parts.push(span)
    })
    parts.push(document.createTextNode(')'))
    this.tip.replaceChildren(...parts)
    this.tip.dataset.name = found.name
    this.tip.dataset.argument = String(current)
    this.tip.hidden = false
    this.field().setAttribute('aria-describedby', 'argument-tip')
    this.placeAssist()
  }

  hideTip () {
    if (this.tip.hidden) return
    this.tip.hidden = true
    this.tip.replaceChildren()
    for (const field of [this.box, this.bar]) field.removeAttribute('aria-describedby')
  }

  // The list under the name being typed, the tip under the text; both in
  // the window, above the text where there is no room below.
  placeAssist () {
    if (!this.state || (this.listBox.hidden && this.tip.hidden)) return
    const field = this.field()
    if (field === this.box && this.box.hidden) return
    const rect = field.getBoundingClientRect()
    const computed = getComputedStyle(field)
    const left = rect.left + (parseFloat(computed.paddingLeft) || 0) + (parseFloat(computed.borderLeftWidth) || 0) - field.scrollLeft
    const place = (node, x) => {
      const style = node.style
      const width = node.offsetWidth
      const height = node.offsetHeight
      const below = rect.bottom + 2
      style.left = Math.max(4, Math.min(x, window.innerWidth - width - 4)) + 'px'
      style.top = (below + height > window.innerHeight - 4 ? Math.max(4, rect.top - height - 2) : below) + 'px'
    }
    if (!this.listBox.hidden && this.list) {
      const text = this.state.text
      const lineStart = text.lastIndexOf('\n', this.list.start - 1) + 1
      place(this.listBox, left + this.grid.painter.measure(computed.font, text.slice(lineStart, this.list.start)))
    }
    if (!this.tip.hidden) place(this.tip, rect.left)
  }

  // Insert Function (dialogs.js): `NAME()` at the caret with the caret
  // between the parentheses; on a cell not being edited, or one holding no
  // formula, the entry becomes `=NAME()`.
  insertFunction (name) {
    if (!this.state && !this.begin({ text: '', mode: 'enter' })) return false
    const s = this.state
    const target = this.field()
    let text = s.text
    let start = target.selectionStart
    let end = target.selectionEnd
    if (!isFormula(text)) {
      text = '='
      start = end = 1
    }
    const written = name + '()'
    this.hideList()
    this.setText(text.slice(0, start) + written + text.slice(end), start + written.length - 1)
    return true
  }

  // Keys -----------------------------------------------------------------------

  // Keys on the sheet that open or feed the editor, and the history keys.
  gridKey (event) {
    if (event.defaultPrevented) return false
    const ctrl = event.ctrlKey || event.metaKey
    const { key } = event
    if (ctrl && !event.altKey) {
      if (key === 'z' || key === 'Z') {
        event.preventDefault()
        this.app.run(event.shiftKey ? 'redo' : 'undo')
        return true
      }
      if (key === 'y' || key === 'Y') {
        event.preventDefault()
        this.app.run('redo')
        return true
      }
      if (key === ';' || key === ':') {
        event.preventDefault()
        if (this.editable()) this.type(this.clock(key === ':' || event.shiftKey))
        return true
      }
      return false
    }
    if (event.altKey) return false
    if (key === 'F2' && !event.shiftKey) {
      event.preventDefault()
      this.editActive('cell')
      return true
    }
    if (key === 'F3' && event.shiftKey) {
      event.preventDefault()
      this.app.run('insertFunction')
      return true
    }
    if (key === 'Backspace') {
      event.preventDefault()
      if (this.editable()) this.keyboard.queue(() => this.begin({ text: '', mode: 'enter' }))
      return true
    }
    // An input method or a dead key composes into the editor itself.
    if (event.isComposing || key === 'Process' || key === 'Dead') {
      if (!this.state && this.editable()) this.begin({ text: '', mode: 'enter' })
      return true
    }
    if (key.length === 1 && !(key === ' ' && event.shiftKey)) {
      event.preventDefault()
      if (this.editable()) this.type(key)
      return true
    }
    return false
  }

  // Today (Ctrl+;) or now (Ctrl+Shift+;) as en-US entry text, on the
  // workbook's clock (`workbook.timezone`).
  clock (time) {
    const workbook = this.app.workbook()
    const zone = workbook && workbook.timezone
    const now = new Date()
    const format = (options) => {
      try {
        return new Intl.DateTimeFormat('en-US', { ...options, timeZone: zone || undefined }).format(now)
      } catch {
        return new Intl.DateTimeFormat('en-US', options).format(now)
      }
    }
    return time
      ? format({ hour: 'numeric', minute: '2-digit', hour12: true }).replace(/ /g, ' ')
      : format({ year: 'numeric', month: 'numeric', day: 'numeric' })
  }

  // Keys inside the in-cell editor.
  key (event) {
    const s = this.state
    if (!s) return
    const ctrl = event.ctrlKey || event.metaKey
    const { key } = event
    const plain = !ctrl && !event.altKey && !event.shiftKey
    // The function list, while it shows, takes the keys that choose.
    if (this.list) {
      if (plain && (key === 'ArrowDown' || key === 'ArrowUp' || key === 'PageDown' || key === 'PageUp')) {
        event.preventDefault()
        this.moveList({ ArrowDown: 1, ArrowUp: -1, PageDown: LIST_ROWS, PageUp: -LIST_ROWS }[key])
        return
      }
      if (plain && key === 'Tab') {
        event.preventDefault()
        this.pick()
        return
      }
      if (plain && key === 'Escape') {
        event.preventDefault()
        this.dismissList()
        return
      }
    }
    if (key === 'Enter') {
      event.preventDefault()
      if (event.altKey) this.insert('\n')
      else if (ctrl) this.commit({ fill: true })
      else this.commit({ move: () => this.selection.walk(event.shiftKey ? -1 : 1, 0) })
      return
    }
    if (key === 'Tab') {
      event.preventDefault()
      this.commit({ move: () => this.selection.walk(0, event.shiftKey ? -1 : 1) })
      return
    }
    if (key === 'Escape') {
      event.preventDefault()
      this.cancel()
      return
    }
    if (key === 'F2') {
      event.preventDefault()
      s.mode = s.mode === 'enter' ? 'edit' : 'enter'
      this.announceMode()
      return
    }
    if (key === 'F4' && plain) {
      event.preventDefault()
      this.cycle()
      return
    }
    if (key === 'F3' && event.shiftKey && !ctrl && !event.altKey) {
      event.preventDefault()
      this.app.run('insertFunction')
      return
    }
    if (ctrl && !event.altKey && (key === ';' || key === ':')) {
      event.preventDefault()
      this.insert(this.clock(key === ':' || event.shiftKey))
      return
    }
    // In Enter mode an arrow points where a formula's caret takes a
    // reference, and otherwise commits and moves as it would on the sheet;
    // in Edit mode it moves the caret.
    if (key in ARROWS && s.mode === 'enter' && s.surface === 'cell' && !event.altKey) {
      const [dRow, dColumn, direction] = ARROWS[key]
      if (this.pointable()) {
        event.preventDefault()
        this.pointKey(dRow, dColumn, direction, { extend: event.shiftKey, jump: ctrl })
        return
      }
      if (ctrl) return
      event.preventDefault()
      this.commit({
        move: () => {
          if (event.shiftKey) this.selection.extendBy(dRow, dColumn)
          else this.selection.move(dRow, dColumn)
        }
      })
    }
  }

  // Keys in the formula bar: the same, except that arrows always move the
  // caret (the list's aside), and a bar not yet editing opens on the first
  // key.
  barKey (event) {
    if (!this.state) {
      if (event.key === 'Escape') {
        event.preventDefault()
        this.showEntry()
        this.app.focusGrid()
      }
      return
    }
    this.surface('bar')
    if (event.key in ARROWS && !(this.list && (event.key === 'ArrowDown' || event.key === 'ArrowUp'))) return
    this.key(event)
  }
}

const sameArea = (a, b) => Boolean(a && b) && a.r0 === b.r0 && a.c0 === b.c0 && a.r1 === b.r1 && a.c1 === b.c1
