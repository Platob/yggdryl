// Dialogs (§10): the one modal frame every dialog is built in - a
// `<dialog>` with a title, a body, a row of buttons, a focus trap, Escape
// to cancel and Enter for the default button - and, on it, the confirm
// question, Format Cells (number, alignment, font, border and fill tabs,
// one `setStyle` per OK, the number sample rendered by the service), Row
// Height, Column Width, Go To, Find and Replace, Custom Sort and Insert
// Function. Later phases build theirs (open, save as) on `Dialog`, and
// send an OK button's edit through `commit`.

import { ApiError, toast } from './api.js'
import { columnName, columnWidthPx, parseRef, rangeName, DEFAULT_COLUMN_WIDTH } from './geometry.js'
import { THEME_SWATCHES, STANDARD_SWATCHES } from './menus.js'
import { sortTarget } from './ribbon.js'
import { FONT_STACK } from './styles.js'
import { KIND_NUMBER } from './tiles.js'

// DOM ------------------------------------------------------------------------

// `el('label', { className, text, attrs, dataset, on }, ...children)`:
// markup built through the DOM, so nothing reaches `innerHTML` and every
// style goes through the CSSOM (§8.2's CSP).
export const el = (tag, props = {}, ...children) => {
  const node = document.createElement(tag)
  const { className, text, attrs, dataset, on, ...rest } = props
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  for (const [name, value] of Object.entries(attrs || {})) {
    if (value === false || value === null || value === undefined) continue
    node.setAttribute(name, value === true ? '' : String(value))
  }
  for (const [name, value] of Object.entries(dataset || {})) node.dataset[name] = String(value)
  for (const [name, handler] of Object.entries(on || {})) node.addEventListener(name, handler)
  for (const [name, value] of Object.entries(rest)) node[name] = value
  for (const child of children.flat()) if (child !== null && child !== undefined && child !== false) node.append(child)
  return node
}

let ids = 0
const uid = (prefix) => `${prefix}-${++ids}`

// A labelled field: `<label for>` beside its control.
const field = (label, control, hint) => {
  if (!control.id) control.id = uid('field')
  return el('div', { className: 'field' },
    el('label', { text: label, attrs: { for: control.id } }),
    control,
    hint ? el('span', { className: 'field-hint', text: hint }) : null)
}

const select = (options, value, attrs = {}) => {
  const node = el('select', { attrs })
  for (const option of options) {
    const [key, label] = Array.isArray(option) ? option : [option, option]
    node.append(el('option', { value: key, text: label }))
  }
  if (value !== undefined) node.value = value
  return node
}

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

// The frame -------------------------------------------------------------------

export class Dialog {
  // buttons: [{ label, value, primary, id, action }]; an `action` may keep
  // the dialog open by answering false, or give the value it closes with.
  // Closing returns focus where it was, or to `restore()` when given (a
  // dialog over the cells gives the sheet its keys back, however opened).
  constructor ({ title, id = uid('dialog'), className = '', body = [], buttons = [], initial = null, restore = null }) {
    this.titleId = id + '-title'
    this.initial = initial
    this.restore = restore
    this.closed = false
    this.element = el('dialog', { className: ('dialog ' + className).trim(), id, attrs: { 'aria-labelledby': this.titleId } })
    const close = el('button', {
      type: 'button',
      className: 'dialog-close',
      text: '×',
      attrs: { 'aria-label': 'Close' },
      on: { click: () => this.close(null) }
    })
    this.body = el('div', { className: 'dialog-body' }, body)
    this.error = el('p', { className: 'dialog-error', attrs: { role: 'alert' }, hidden: true })
    this.buttons = buttons.map((spec) => {
      const button = el('button', {
        type: 'button',
        text: spec.label,
        className: spec.primary ? 'primary' : '',
        id: spec.id || '',
        on: { click: () => this.press(spec) }
      })
      if (spec.primary) this.primary = button
      return button
    })
    this.element.append(
      el('div', { className: 'dialog-head' }, el('h2', { className: 'dialog-title', id: this.titleId, text: title }), close),
      this.body,
      this.error,
      el('div', { className: 'dialog-buttons' }, this.buttons))
    this.element.addEventListener('cancel', (event) => {
      event.preventDefault()
      this.close(null)
    })
    this.element.addEventListener('keydown', (event) => this.key(event))
    this.result = new Promise((resolve) => {
      this.resolve = resolve
    })
  }

  open () {
    this.previous = document.activeElement
    document.body.append(this.element)
    this.element.showModal()
    const first = this.initial || this.body.querySelector(FOCUSABLE) || this.primary
    if (first) {
      first.focus()
      if (first.select && first.tagName === 'INPUT' && first.type === 'text') first.select()
    }
    return this.result
  }

  // A message in the dialog itself, for a value it cannot take.
  fail (message, focus = null) {
    this.error.textContent = message
    this.error.hidden = !message
    if (focus) {
      focus.focus()
      if (focus.select) focus.select()
    }
  }

  async press (spec) {
    if (this.busy) return
    if (!spec.action) {
      this.close(spec.value ?? null)
      return
    }
    this.busy = true
    let value
    try {
      value = await spec.action()
    } finally {
      this.busy = false
    }
    if (value === false) return
    this.close(value === undefined ? spec.value ?? true : value)
  }

  close (value) {
    if (this.closed) return
    this.closed = true
    if (this.element.open) this.element.close()
    this.element.remove()
    const back = this.previous
    if (this.restore) this.restore()
    else if (back && document.contains(back) && !back.closest('dialog')) back.focus({ preventScroll: true })
    this.resolve(value)
  }

  // Enter takes the default button from a one-line field; Tab stays in the
  // dialog, wrapping at either end.
  key (event) {
    if (event.key === 'Enter' && !event.shiftKey && !event.altKey && this.primary) {
      const target = event.target
      const line = target.tagName === 'INPUT' && !['button', 'checkbox', 'radio', 'color', 'range'].includes(target.type)
      const list = target.tagName === 'SELECT'
      if (line || list) {
        event.preventDefault()
        this.primary.click()
      }
      return
    }
    if (event.key !== 'Tab') return
    const focusable = [...this.element.querySelectorAll(FOCUSABLE)].filter((node) => !node.closest('[hidden]') && node.offsetParent !== null)
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }
}

// Number formats (the Number tab) ---------------------------------------------

// What the tab builds codes from: its categories and each one's choices. A
// code is the service's to render; here it is only spelled.
export const CATEGORIES = [
  ['general', 'General'], ['number', 'Number'], ['currency', 'Currency'], ['accounting', 'Accounting'],
  ['date', 'Date'], ['time', 'Time'], ['percentage', 'Percentage'], ['fraction', 'Fraction'],
  ['scientific', 'Scientific'], ['text', 'Text'], ['custom', 'Custom']
]

export const SYMBOLS = [
  ['$', '$', '"$"'], ['none', 'None', ''], ['€', '€ Euro', '"€"'], ['£', '£ English (United Kingdom)', '"£"'], ['¥', '¥ Japanese', '"¥"']
]

// Each type shown with the example Excel's dialog shows (3/14/2012 1:30:55 PM).
export const DATE_TYPES = [
  ['m/d/yyyy', '3/14/2012'], ['dddd, mmmm d, yyyy', 'Wednesday, March 14, 2012'], ['m/d/yy', '3/14/12'],
  ['mm/dd/yy', '03/14/12'], ['d-mmm', '14-Mar'], ['d-mmm-yy', '14-Mar-12'], ['dd-mmm-yy', '14-Mar-12 (two-digit day)'],
  ['mmm-yy', 'Mar-12'], ['mmmm-yy', 'March-12'], ['mmmm d, yyyy', 'March 14, 2012'], ['m/d/yy h:mm', '3/14/12 13:30'],
  ['m/d/yyyy h:mm', '3/14/2012 13:30'], ['yyyy-mm-dd', '2012-03-14']
]
export const TIME_TYPES = [
  ['h:mm:ss AM/PM', '1:30:55 PM'], ['h:mm', '13:30'], ['h:mm AM/PM', '1:30 PM'], ['h:mm:ss', '13:30:55'],
  ['mm:ss', '30:55'], ['mm:ss.0', '30:55.2'], ['[h]:mm:ss', '37:30:55'], ['m/d/yy h:mm AM/PM', '3/14/12 1:30 PM']
]
export const FRACTION_TYPES = [
  ['# ?/?', 'Up to one digit (1/4)'], ['# ??/??', 'Up to two digits (21/25)'], ['# ???/???', 'Up to three digits (312/943)'],
  ['# ?/2', 'As halves (1/2)'], ['# ?/4', 'As quarters (2/4)'], ['# ?/8', 'As eighths (4/8)'],
  ['# ??/16', 'As sixteenths (8/16)'], ['# ?/10', 'As tenths (3/10)'], ['# ??/100', 'As hundredths (30/100)']
]
// ECMA-376 §18.8.30's en-US built-ins the Custom list offers first.
export const BUILTIN_CODES = [
  'General', '0', '0.00', '#,##0', '#,##0.00', '0%', '0.00%', '0.00E+00', '##0.0E+0', '# ?/?', '# ??/??',
  'm/d/yyyy', 'd-mmm-yy', 'd-mmm', 'mmm-yy', 'h:mm AM/PM', 'h:mm:ss AM/PM', 'h:mm', 'h:mm:ss', 'm/d/yyyy h:mm',
  '#,##0 ;(#,##0)', '#,##0 ;[Red](#,##0)', '#,##0.00;(#,##0.00)', '#,##0.00;[Red](#,##0.00)', 'mm:ss', '[h]:mm:ss', 'mmss.0', '@'
]
export const MAX_DECIMALS = 30

const decimalsOf = (d) => (d > 0 ? '.' + '0'.repeat(d) : '')
const symbolCode = (symbol) => (SYMBOLS.find(([key]) => key === symbol) || SYMBOLS[0])[2]
const negatives = (base) => [base, `${base};[Red]${base}`, `${base}_);(${base})`, `${base}_);[Red](${base})`]

// The code a category's choices spell, as Excel's dialog writes it.
export const numberCode = (category, options = {}) => {
  const d = Math.min(MAX_DECIMALS, Math.max(0, Math.trunc(options.decimals ?? 2)))
  const negative = Math.min(3, Math.max(0, options.negative || 0))
  switch (category) {
    case 'general': return 'General'
    case 'number': return negatives((options.separator ? '#,##0' : '0') + decimalsOf(d))[negative]
    case 'currency': return negatives(symbolCode(options.symbol) + '#,##0' + decimalsOf(d))[negative]
    case 'accounting': {
      const s = symbolCode(options.symbol)
      const n = '#,##0' + decimalsOf(d)
      return `_(${s}* ${n}_);_(${s}* \\(${n}\\);_(${s}* "-"${'?'.repeat(d)}_);_(@_)`
    }
    case 'percentage': return '0' + decimalsOf(d) + '%'
    case 'scientific': return '0' + decimalsOf(d) + 'E+00'
    case 'date': return options.type || DATE_TYPES[0][0]
    case 'time': return options.type || TIME_TYPES[0][0]
    case 'fraction': return options.type || FRACTION_TYPES[0][0]
    case 'text': return '@'
    default: return options.code || 'General'
  }
}

let codes = null

// The category and choices a code was spelled from; anything else is Custom.
export const classifyCode = (code) => {
  if (!codes) {
    codes = new Map()
    const add = (category, options) => {
      const spelled = numberCode(category, options)
      if (!codes.has(spelled)) codes.set(spelled, { category, options })
    }
    add('general', {})
    for (let decimals = 0; decimals <= MAX_DECIMALS; decimals++) {
      for (const separator of [false, true]) for (let negative = 0; negative < 4; negative++) add('number', { decimals, separator, negative })
      for (const [symbol] of SYMBOLS) {
        for (let negative = 0; negative < 4; negative++) add('currency', { decimals, symbol, negative })
        add('accounting', { decimals, symbol })
      }
      add('percentage', { decimals })
      add('scientific', { decimals })
    }
    for (const [type] of DATE_TYPES) add('date', { type })
    for (const [type] of TIME_TYPES) add('time', { type })
    for (const [type] of FRACTION_TYPES) add('fraction', { type })
    add('text', {})
  }
  return codes.get(code) || { category: 'custom', options: { code } }
}

// Borders and alignment (the other tabs) ----------------------------------------

export const LINE_STYLES = [
  ['thin', 'Thin'], ['hair', 'Hair'], ['dotted', 'Dotted'], ['dashed', 'Dashed'], ['dashDot', 'Dash dot'],
  ['dashDotDot', 'Dash dot dot'], ['double', 'Double'], ['medium', 'Medium'], ['mediumDashed', 'Medium dashed'],
  ['mediumDashDot', 'Medium dash dot'], ['mediumDashDotDot', 'Medium dash dot dot'], ['slantDashDot', 'Slanted dash dot'],
  ['thick', 'Thick']
]
const LINE_CSS = {
  thin: '1px solid', hair: '1px dotted', dotted: '1px dotted', dashed: '1px dashed', dashDot: '1px dashed',
  dashDotDot: '1px dashed', double: '3px double', medium: '2px solid', mediumDashed: '2px dashed',
  mediumDashDot: '2px dashed', mediumDashDotDot: '2px dashed', slantDashDot: '2px dashed', thick: '3px solid'
}
const HORIZONTAL = [
  ['general', 'General'], ['left', 'Left (Indent)'], ['center', 'Center'], ['right', 'Right (Indent)'], ['fill', 'Fill'],
  ['justify', 'Justify'], ['centerContinuous', 'Center Across Selection'], ['distributed', 'Distributed (Indent)']
]
const VERTICAL = [['top', 'Top'], ['center', 'Center'], ['bottom', 'Bottom'], ['justify', 'Justify'], ['distributed', 'Distributed']]
const UNDERLINES = [['none', 'None'], ['single', 'Single'], ['double', 'Double'], ['singleAccounting', 'Single Accounting'], ['doubleAccounting', 'Double Accounting']]
const FONT_STYLES = [['regular', 'Regular'], ['italic', 'Italic'], ['bold', 'Bold'], ['boldItalic', 'Bold Italic']]
const FONTS = ['Calibri', 'Aptos', 'Arial', 'Cambria', 'Consolas', 'Courier New', 'Georgia', 'Segoe UI', 'Tahoma', 'Times New Roman', 'Verdana']
const SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72]
const INDENTED = new Set(['left', 'right', 'distributed'])
const TABS = [['number', 'Number'], ['alignment', 'Alignment'], ['font', 'Font'], ['border', 'Border'], ['fill', 'Fill']]

export const MAX_ROW_HEIGHT = 409
export const MAX_COLUMN_CHARACTERS = 255
export const MAX_INDENT = 250
const MAX_RECENT = 4
// The number sample asks the service this long after the last change.
export const PREVIEW_DELAY = 120
// A `*x` fill in the sample: enough copies to cross it, clipped by CSS.
const SAMPLE_FILL = 80
// Custom Sort: Excel's 64 levels, and the columns a level's list offers.
export const MAX_SORT_LEVELS = 64
const MAX_SORT_COLUMNS = 1024
// Insert Function: what Most Recently Used lists before anything was
// inserted (Excel's own first list; a name the service does not list is
// left out), and how many it keeps.
const FIRST_RECENT = ['SUM', 'AVERAGE', 'IF', 'HYPERLINK', 'COUNT', 'MAX', 'SIN', 'SUMIF', 'PMT', 'STDEV']
const MAX_RECENT_FUNCTIONS = 10

// The functions a search finds: every word in the name or the description,
// a name matching best first, then by name.
export const searchFunctions = (list, text) => {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return []
  const found = []
  for (const item of list) {
    const name = item.name.toLowerCase()
    const about = (item.description || '').toLowerCase()
    if (!words.every((word) => name.includes(word) || about.includes(word))) continue
    const rank = words.includes(name) ? 0 : words.some((word) => name.startsWith(word)) ? 1 : words.some((word) => name.includes(word)) ? 2 : 3
    found.push({ item, rank })
  }
  found.sort((a, b) => a.rank - b.rank || a.item.name.localeCompare(b.item.name))
  return found.map((entry) => entry.item)
}

// Column widths in the characters a dialog shows (Excel's 8.43), and the
// file's `<col width>` the API takes (addendum 2), padding included.
export const charactersOfWidth = (width, digit) => {
  const px = columnWidthPx(width, digit)
  return Math.max(0, Math.trunc(((px - 5) / digit) * 100 + 0.5) / 100)
}
export const widthOfCharacters = (characters, digit) =>
  characters <= 0 ? 0 : Math.trunc(((characters * digit + 5) / digit) * 256) / 256

// Merged spans [first, last] of the ranges on one axis.
const spans = (ranges, rows) => {
  const list = ranges.map((range) => rows ? [range.r0, range.r1] : [range.c0, range.c1]).sort((a, b) => a[0] - b[0])
  const out = []
  for (const span of list) {
    const last = out[out.length - 1]
    if (last && span[0] <= last[1] + 1) last[1] = Math.max(last[1], span[1])
    else out.push([...span])
  }
  return out
}

const runAt = (runs, index) => (runs || []).find((run) => run[0] <= index && index <= run[1]) || null

// The number a user typed, or null.
const numberIn = (text) => {
  const value = Number(String(text).trim())
  return String(text).trim() !== '' && Number.isFinite(value) ? value : null
}

// Send a dialog's edit from its OK button: the dialog closes once the
// service has taken it, and stays open with the service's reason when it
// refuses (the text typed kept for a fix, as a refused entry is).
export const commit = async (app, dialog, edit, options) => {
  try {
    await app.edit(edit, { ...options, quiet: true })
    return true
  } catch (error) {
    if (!(error instanceof ApiError)) throw error
    dialog.fail(error.kind === 'stale_base' ? 'Another tab changed these cells first; they now show that change. Choose OK to apply yours again.' : error.message)
    return false
  }
}

export class Dialogs {
  constructor ({ app, api, grid, selection, tiles, styles }) {
    this.app = app
    this.api = api
    this.grid = grid
    this.selection = selection
    this.tiles = tiles
    this.styles = styles
    this.lastTab = 'number'
    this.recent = []
    // What Find and Replace asked last, asked again next time.
    this.search = { text: '', replacement: '', scope: 'sheet', in: 'formulas', matchCase: false, entireCell: false }
    // The functions Insert Function inserted, the latest first.
    this.recentFunctions = [...FIRST_RECENT]
    this.toSheet = () => app.focusGrid()
    app.command('formatCells', (options) => this.formatCells(options))
    app.command('rowHeightDialog', () => this.rowHeight())
    app.command('columnWidthDialog', () => this.columnWidth())
    app.command('goTo', () => this.goTo())
    app.command('find', () => this.findReplace('find'))
    app.command('replace', () => this.findReplace('replace'))
    app.command('sortDialog', () => this.sortDialog())
    app.command('insertFunction', () => this.insertFunction())
  }

  // A yes-or-no question: true on the action button, false on Cancel or
  // Escape.
  async confirm (message, action = 'OK') {
    const cancel = { label: 'Cancel', value: false, id: 'confirm-cancel' }
    const dialog = new Dialog({
      title: 'Confirm',
      id: 'confirm',
      className: 'confirm',
      body: [el('p', { id: 'confirm-text', text: message })],
      buttons: [{ label: action, value: true, primary: true, id: 'confirm-ok' }, cancel]
    })
    dialog.initial = dialog.buttons[1]
    return (await dialog.open()) === true
  }

  // The selection on a worksheet shown, or a word saying why not.
  cells () {
    if (this.grid.geometry && this.grid.sheet === this.app.sheetKey()) return true
    toast('Select cells on a worksheet first.', 'info')
    return false
  }

  activeStyle () {
    const { row, column } = this.selection.active
    const found = this.tiles.cell(this.app.sheetKey(), row, column)
    const id = found ? found.style : (this.grid.geometry && this.grid.geometry.blankStyle(row, column)) || 0
    return this.styles.get(id || 0)
  }

  // Format Cells ------------------------------------------------------------------

  async formatCells ({ tab } = {}) {
    if (!this.cells()) return
    const sheet = this.app.sheetKey()
    const ranges = this.selection.ranges.map((range) => ({ ...range }))
    const refs = this.selection.refs
    const style = this.activeStyle()
    // What the user changed, by patch key; an untouched key is left out of
    // the patch, so the cells keep what they have.
    const patch = {}
    const borders = []
    const tabs = {
      number: this.numberTab(style, patch, sheet, this.selection.activeRef),
      alignment: this.alignmentTab(style, patch),
      font: this.fontTab(style, patch),
      border: this.borderTab(style, borders),
      fill: this.fillTab(style, patch)
    }
    const list = el('div', { className: 'dialog-tabs', attrs: { role: 'tablist', 'aria-label': 'Format Cells' } })
    const panels = []
    const buttons = []
    const choose = (key, focus = false) => {
      this.lastTab = key
      for (const button of buttons) {
        const selected = button.dataset.tab === key
        button.setAttribute('aria-selected', String(selected))
        button.tabIndex = selected ? 0 : -1
        if (selected && focus) button.focus()
      }
      for (const panel of panels) panel.hidden = panel.dataset.tab !== key
    }
    for (const [key, label] of TABS) {
      const id = uid('fc-tab')
      const panel = el('div', { className: 'dialog-panel', id: id + '-panel', attrs: { role: 'tabpanel', 'aria-labelledby': id }, dataset: { tab: key } }, tabs[key])
      const button = el('button', {
        type: 'button',
        id,
        className: 'dialog-tab',
        text: label,
        attrs: { role: 'tab', 'aria-controls': panel.id },
        dataset: { tab: key },
        on: { click: () => choose(key) }
      })
      buttons.push(button)
      panels.push(panel)
      list.append(button)
    }
    list.addEventListener('keydown', (event) => {
      const at = buttons.indexOf(document.activeElement)
      const next = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: buttons.length - 1 }[event.key]
      if (at < 0 || next === undefined) return
      event.preventDefault()
      choose(buttons[(next + buttons.length) % buttons.length].dataset.tab, true)
    })
    const dialog = new Dialog({
      title: 'Format Cells',
      id: 'format-cells',
      className: 'format-cells',
      body: [list, ...panels],
      buttons: [{
        label: 'OK',
        primary: true,
        id: 'format-cells-ok',
        action: () => {
          const edits = []
          if (Object.keys(patch).length) edits.push({ op: 'setStyle', sheet, ranges: refs, patch })
          for (const border of borders) edits.push({ op: 'setStyle', sheet, ranges: refs, patch: { borders: border } })
          if (edits.length === 0) return true
          return commit(this.app, dialog, edits.length === 1 ? edits[0] : { op: 'batch', edits }, { sheet, ranges })
        }
      }, { label: 'Cancel', value: null }],
      restore: this.toSheet
    })
    const first = TABS.some(([key]) => key === tab) ? tab : this.lastTab
    choose(first)
    dialog.initial = buttons.find((button) => button.dataset.tab === first)
    await dialog.open()
  }

  // The sample is the service's rendering of the code chosen against the
  // active cell's value, or 1234.5 on a blank cell (addendum 9), asked a
  // moment after the last change; an answer to an earlier code is dropped.
  // A code the service cannot read shows its reason under the sample.
  numberTab (style, patch, sheet, ref) {
    const original = style.numberFormat || 'General'
    const known = classifyCode(original)
    let category = known.category
    const choices = { decimals: 2, separator: false, negative: 0, symbol: '$', type: '', code: original, ...known.options }
    const categories = select(CATEGORIES, category, { size: CATEGORIES.length, 'aria-label': 'Category' })
    categories.id = 'fc-category'
    const options = el('div', { className: 'number-options' })
    const code = el('input', { type: 'text', id: 'fc-code', spellcheck: false, attrs: { autocomplete: 'off' } })
    const sample = el('output', { className: 'number-sample', id: 'fc-sample', attrs: { 'aria-live': 'polite' } })
    const refused = el('p', { className: 'field-hint sample-refused', id: 'fc-sample-refused', attrs: { role: 'alert' }, hidden: true })
    const described = el('p', { className: 'field-hint' })
    const codeField = field('Type', code)
    const show = (spelled, answer, refusal) => {
      sample.style.color = ''
      sample.dataset.code = spelled
      if (refusal) {
        sample.replaceChildren()
        sample.dataset.state = 'refused'
        refused.textContent = refusal.message
        refused.hidden = false
        return
      }
      refused.hidden = true
      sample.dataset.state = 'shown'
      const text = String(answer.text ?? '')
      if (Array.isArray(answer.fill)) {
        const at = Math.min(Math.max(0, Math.trunc(answer.fill[1]) || 0), text.length)
        sample.replaceChildren(
          el('span', { text: text.slice(0, at) }),
          el('span', { className: 'number-fill', text: String(answer.fill[0]).repeat(SAMPLE_FILL) }),
          el('span', { text: text.slice(at) }))
      } else {
        sample.textContent = text
      }
      if (answer.color) sample.style.color = answer.color
    }
    let asked = 0
    let timer = 0
    const preview = (spelled, delay) => {
      clearTimeout(timer)
      const id = ++asked
      sample.dataset.state = 'asking'
      timer = setTimeout(async () => {
        let answer = null
        let refusal = null
        try {
          answer = await this.api.get(`format?code=${encodeURIComponent(spelled)}&sheet=${sheet}&ref=${ref}`, { quiet: true })
        } catch (error) {
          refusal = error
        }
        if (id === asked && sample.isConnected) show(spelled, answer, refusal)
      }, delay)
    }
    let asking = false
    const update = () => {
      relabel()
      const spelled = numberCode(category, choices)
      if (document.activeElement !== code) code.value = spelled
      code.readOnly = category !== 'custom'
      if (spelled !== original) patch.numberFormat = spelled
      else delete patch.numberFormat
      if (sample.dataset.code !== spelled || sample.dataset.state === 'asking') preview(spelled, asking ? PREVIEW_DELAY : 0)
      asking = true
    }
    const decimals = () => {
      const input = el('input', { type: 'number', min: 0, max: MAX_DECIMALS, step: 1, value: String(choices.decimals), id: 'fc-decimals' })
      input.addEventListener('input', () => {
        const value = numberIn(input.value)
        if (value === null) return
        choices.decimals = Math.min(MAX_DECIMALS, Math.max(0, Math.trunc(value)))
        update()
      })
      return field('Decimal places', input)
    }
    const listOf = (types, label) => {
      const list = select(types, types.some(([key]) => key === choices.type) ? choices.type : types[0][0], { size: Math.min(8, types.length) })
      list.id = 'fc-type'
      choices.type = list.value
      list.addEventListener('change', () => {
        choices.type = list.value
        update()
      })
      return field(label, list)
    }
    // Excel's four spellings of a negative number, over 1234.10 at the
    // decimals and separator chosen.
    let negatives = null
    const relabel = () => {
      if (!negatives) return
      const symbol = category === 'currency' ? (SYMBOLS.find(([key]) => key === choices.symbol) || SYMBOLS[0])[2].replace(/"/g, '') : ''
      const digits = (category === 'currency' || choices.separator ? '1,234' : '1234') + (choices.decimals > 0 ? '.' + '10'.padEnd(choices.decimals, '0').slice(0, choices.decimals) : '')
      const labels = [`-${symbol}${digits}`, `${symbol}${digits} (red)`, `(${symbol}${digits})`, `(${symbol}${digits}) (red)`]
      labels.forEach((label, i) => {
        negatives.options[i].textContent = label
      })
    }
    const negative = () => {
      negatives = select([0, 1, 2, 3].map((i) => [String(i), '']), String(choices.negative), { size: 4 })
      negatives.id = 'fc-negative'
      negatives.addEventListener('change', () => {
        choices.negative = Number(negatives.value)
        update()
      })
      return field('Negative numbers', negatives)
    }
    const symbol = () => {
      const list = select(SYMBOLS.map(([key, label]) => [key, label]), choices.symbol)
      list.id = 'fc-symbol'
      list.addEventListener('change', () => {
        choices.symbol = list.value
        update()
      })
      return field('Symbol', list)
    }
    const render = () => {
      const parts = []
      negatives = null
      if (category === 'general') parts.push(el('p', { text: 'General format cells have no specific number format.' }))
      if (category === 'number') {
        const separator = el('input', { type: 'checkbox', checked: Boolean(choices.separator), id: 'fc-separator' })
        separator.addEventListener('change', () => {
          choices.separator = separator.checked
          update()
        })
        parts.push(decimals(), el('div', { className: 'field check' }, separator, el('label', { text: 'Use 1000 Separator (,)', attrs: { for: separator.id } })), negative())
      }
      if (category === 'currency') parts.push(decimals(), symbol(), negative())
      if (category === 'accounting') parts.push(decimals(), symbol())
      if (category === 'percentage' || category === 'scientific') parts.push(decimals())
      if (category === 'date') parts.push(listOf(DATE_TYPES, 'Type'))
      if (category === 'time') parts.push(listOf(TIME_TYPES, 'Type'))
      if (category === 'fraction') parts.push(listOf(FRACTION_TYPES, 'Type'))
      if (category === 'text') parts.push(el('p', { text: 'Text format cells are treated as text even when a number is in the cell.' }))
      if (category === 'custom') {
        const seen = new Set()
        const all = [...BUILTIN_CODES, ...this.styles.table.map((entry) => entry.numberFormat), original]
          .filter((item) => item && !seen.has(item) && seen.add(item))
        const list = select(all, all.includes(choices.code) ? choices.code : undefined, { size: 8, 'aria-label': 'Custom formats' })
        list.id = 'fc-custom-list'
        list.addEventListener('change', () => {
          choices.code = list.value
          code.value = list.value
          update()
        })
        parts.push(list)
      }
      options.replaceChildren(...parts)
      codeField.hidden = category === 'general' || category === 'text'
      described.textContent = category === 'custom' ? 'Type the number format code, using one of the existing codes as a starting point.' : ''
      update()
    }
    code.addEventListener('input', () => {
      if (category !== 'custom') return
      choices.code = code.value
      update()
    })
    categories.addEventListener('change', () => {
      category = categories.value
      if (category === 'custom') choices.code = code.value
      render()
    })
    render()
    return el('div', { className: 'number-tab' },
      field('Category', categories),
      el('div', { className: 'number-right' },
        el('div', { className: 'field' }, el('span', { className: 'field-label', text: 'Sample' }), sample, refused),
        options, codeField, described))
  }

  alignmentTab (style, patch) {
    const horizontal = select(HORIZONTAL, style.horizontal || 'general')
    horizontal.id = 'fc-horizontal'
    const indent = el('input', { type: 'number', min: 0, max: MAX_INDENT, step: 1, value: String(style.indent || 0), id: 'fc-indent' })
    const vertical = select(VERTICAL, style.vertical || 'bottom')
    vertical.id = 'fc-vertical'
    const wrap = el('input', { type: 'checkbox', checked: Boolean(style.wrap), id: 'fc-wrap' })
    const sync = () => {
      indent.disabled = !INDENTED.has(horizontal.value)
    }
    horizontal.addEventListener('change', () => {
      patch.horizontal = horizontal.value
      sync()
    })
    indent.addEventListener('input', () => {
      const value = numberIn(indent.value)
      if (value !== null) patch.indent = Math.min(MAX_INDENT, Math.max(0, Math.trunc(value)))
    })
    vertical.addEventListener('change', () => {
      patch.vertical = vertical.value
    })
    wrap.addEventListener('change', () => {
      patch.wrap = wrap.checked
    })
    sync()
    return el('div', { className: 'dialog-columns' },
      el('fieldset', {}, el('legend', { text: 'Text alignment' }), field('Horizontal', horizontal), field('Indent', indent), field('Vertical', vertical)),
      el('fieldset', {}, el('legend', { text: 'Text control' }),
        el('div', { className: 'field check' }, wrap, el('label', { text: 'Wrap text', attrs: { for: wrap.id } }))))
  }

  fontTab (style, patch) {
    const font = style.font
    const list = el('datalist', { id: uid('fonts') }, FONTS.map((name) => el('option', { value: name })))
    const name = el('input', { type: 'text', value: font.name || 'Calibri', id: 'fc-font', spellcheck: false, attrs: { list: list.id, autocomplete: 'off' } })
    const kind = font.bold && font.italic ? 'boldItalic' : font.bold ? 'bold' : font.italic ? 'italic' : 'regular'
    const weight = select(FONT_STYLES, kind)
    weight.id = 'fc-font-style'
    const sizes = el('datalist', { id: uid('sizes') }, SIZES.map((size) => el('option', { value: String(size) })))
    const size = el('input', { type: 'number', min: 1, max: 409, step: 0.5, value: String(font.size || 11), id: 'fc-size', attrs: { list: sizes.id } })
    const underline = select(UNDERLINES, font.underline || 'none')
    underline.id = 'fc-underline'
    const color = el('input', { type: 'color', value: (font.color || '#000000').slice(0, 7).toLowerCase(), id: 'fc-font-color' })
    const automatic = el('input', { type: 'checkbox', checked: !font.color || font.color.toUpperCase() === '#000000', id: 'fc-font-auto' })
    const strike = el('input', { type: 'checkbox', checked: Boolean(font.strike), id: 'fc-strike' })
    const preview = el('div', { className: 'font-preview', text: 'AaBbCcYyZz', attrs: { 'aria-hidden': 'true' } })
    const show = () => {
      const s = preview.style
      s.fontFamily = `"${name.value.replace(/["\\]/g, '')}", ${FONT_STACK}`
      s.fontSize = Math.min(36, Math.max(6, numberIn(size.value) || 11)) + 'pt'
      s.fontWeight = weight.value.startsWith('bold') ? '700' : '400'
      s.fontStyle = weight.value === 'italic' || weight.value === 'boldItalic' ? 'italic' : 'normal'
      const lines = []
      if (underline.value !== 'none') lines.push('underline')
      if (strike.checked) lines.push('line-through')
      s.textDecorationLine = lines.join(' ') || 'none'
      s.textDecorationStyle = underline.value.startsWith('double') ? 'double' : 'solid'
      s.color = automatic.checked ? '#000000' : color.value
    }
    name.addEventListener('input', () => {
      if (name.value.trim()) patch.fontName = name.value.trim()
      show()
    })
    weight.addEventListener('change', () => {
      patch.bold = weight.value.startsWith('bold')
      patch.italic = weight.value === 'italic' || weight.value === 'boldItalic'
      show()
    })
    size.addEventListener('input', () => {
      const value = numberIn(size.value)
      if (value !== null && value >= 1 && value <= 409) patch.fontSize = value
      show()
    })
    underline.addEventListener('change', () => {
      patch.underline = underline.value
      show()
    })
    color.addEventListener('input', () => {
      automatic.checked = false
      patch.fontColor = color.value.toUpperCase()
      show()
    })
    automatic.addEventListener('change', () => {
      patch.fontColor = automatic.checked ? null : color.value.toUpperCase()
      show()
    })
    strike.addEventListener('change', () => {
      patch.strike = strike.checked
      show()
    })
    show()
    return el('div', { className: 'dialog-columns' },
      el('div', {}, field('Font', name), list, field('Font style', weight), field('Size', size), sizes, field('Underline', underline)),
      el('div', {},
        field('Color', color),
        el('div', { className: 'field check' }, automatic, el('label', { text: 'Automatic', attrs: { for: automatic.id } })),
        el('fieldset', {}, el('legend', { text: 'Effects' }),
          el('div', { className: 'field check' }, strike, el('label', { text: 'Strikethrough', attrs: { for: strike.id } }))),
        el('div', { className: 'field' }, el('span', { className: 'field-label', text: 'Preview' }), preview)))
  }

  // Each preset pressed is one `setStyle` of its own (a patch carries one
  // preset), all in one batch with the rest of the dialog.
  borderTab (style, borders) {
    const line = select(LINE_STYLES, 'thin', { size: 7 })
    line.id = 'fc-line'
    const color = el('input', { type: 'color', value: '#000000', id: 'fc-border-color' })
    const automatic = el('input', { type: 'checkbox', checked: true, id: 'fc-border-auto' })
    color.addEventListener('input', () => {
      automatic.checked = false
    })
    // The preview: four cells of the selection, their outer and inner lines.
    const sides = {}
    for (const side of ['top', 'bottom', 'left', 'right']) {
      const edge = style.edges[side]
      sides[side] = edge ? { css: `${edge.width}px ${edge.double ? 'double' : edge.dash.length ? 'dashed' : 'solid'}`, color: edge.color } : null
    }
    sides.inside = null
    const cells = [0, 1, 2, 3].map(() => el('div', { className: 'border-cell', text: 'Text' }))
    const preview = el('div', { className: 'border-preview', attrs: { 'aria-hidden': 'true' } }, cells)
    const draw = () => {
      const edge = (value) => value ? `${value.css} ${value.color}` : ''
      cells.forEach((cell, i) => {
        const top = i < 2
        const left = i % 2 === 0
        cell.style.borderTop = edge(top ? sides.top : sides.inside)
        cell.style.borderBottom = edge(top ? sides.inside : sides.bottom)
        cell.style.borderLeft = edge(left ? sides.left : sides.inside)
        cell.style.borderRight = edge(left ? sides.inside : sides.right)
      })
    }
    const applied = el('p', { className: 'field-hint', id: 'fc-borders-applied' })
    const press = (preset) => {
      const lineStyle = preset === 'thickOutside' ? 'thick' : line.value
      const value = { preset, style: lineStyle, color: automatic.checked ? null : color.value.toUpperCase() }
      borders.push(value)
      const side = { css: LINE_CSS[lineStyle] || '1px solid', color: value.color || '#000000' }
      if (preset === 'none') for (const key of Object.keys(sides)) sides[key] = null
      if (preset === 'all' || preset === 'outside' || preset === 'thickOutside') {
        sides.top = sides.bottom = sides.left = sides.right = side
        if (preset === 'all') sides.inside = side
      }
      if (['top', 'bottom', 'left', 'right'].includes(preset)) sides[preset] = side
      applied.textContent = 'Applies: ' + borders.map((b) => `${b.preset} (${b.style})`).join(', ')
      draw()
    }
    const presets = [
      ['none', 'None'], ['outside', 'Outline'], ['all', 'All'], ['thickOutside', 'Thick Outline'],
      ['top', 'Top'], ['bottom', 'Bottom'], ['left', 'Left'], ['right', 'Right']
    ].map(([key, label]) => el('button', {
      type: 'button',
      className: 'preset',
      text: label,
      dataset: { preset: key },
      attrs: { 'aria-label': `${label} border` },
      on: { click: () => press(key) }
    }))
    draw()
    return el('div', { className: 'dialog-columns' },
      el('div', {}, field('Line style', line), field('Color', color),
        el('div', { className: 'field check' }, automatic, el('label', { text: 'Automatic', attrs: { for: automatic.id } }))),
      el('div', {},
        el('div', { className: 'presets', attrs: { role: 'group', 'aria-label': 'Presets' } }, presets),
        preview, applied))
  }

  fillTab (style, patch) {
    const sample = el('div', { className: 'fill-sample', attrs: { 'aria-hidden': 'true' } })
    const current = el('output', { className: 'field-hint', id: 'fc-fill-value' })
    const show = (color) => {
      sample.style.backgroundColor = color || ''
      current.textContent = color ? color.toUpperCase() : 'No Color'
    }
    const choose = (color) => {
      patch.fill = color ? color.toUpperCase() : null
      show(color)
      for (const swatch of swatches.querySelectorAll('.menu-swatch')) swatch.setAttribute('aria-pressed', String(swatch.dataset.color === color))
    }
    const swatch = ({ name, color }) => {
      const button = el('button', {
        type: 'button',
        className: 'menu-swatch',
        title: name,
        dataset: { color },
        attrs: { 'aria-label': name, 'aria-pressed': 'false' },
        on: { click: () => choose(color) }
      })
      button.style.backgroundColor = color
      return button
    }
    const swatches = el('div', { className: 'dialog-swatches', attrs: { role: 'group', 'aria-label': 'Background color' } },
      [...THEME_SWATCHES, ...STANDARD_SWATCHES].map(swatch))
    const none = el('button', { type: 'button', className: 'secondary', text: 'No Color', id: 'fc-no-fill', on: { click: () => choose(null) } })
    const more = el('input', { type: 'color', value: (style.fill || '#ffffff').slice(0, 7).toLowerCase(), id: 'fc-fill-more' })
    more.addEventListener('input', () => choose(more.value))
    show(style.fill)
    return el('div', { className: 'dialog-columns' },
      el('div', {}, none, swatches, field('More Colors', more)),
      el('div', {}, el('span', { className: 'field-label', text: 'Sample' }), sample, current))
  }

  // Row Height and Column Width ------------------------------------------------------

  // The size asked for, once for every span of the selection: points for a
  // row, characters for a column (sent as the file's width).
  async size (rows) {
    if (!this.cells()) return
    const sheet = this.app.sheetKey()
    const layout = this.grid.layout || {}
    const defaults = layout.defaults || {}
    const digit = defaults.maxDigitWidth || 7
    const { row, column } = this.selection.active
    const run = runAt(rows ? layout.rows : layout.columns, rows ? row : column)
    let shown
    if (rows) shown = run && run[2] !== null && run[2] !== undefined ? run[2] : defaults.rowHeight ?? 15
    else shown = charactersOfWidth(run && run[2] !== null && run[2] !== undefined ? run[2] : defaults.columnWidth ?? DEFAULT_COLUMN_WIDTH, digit)
    const input = el('input', { type: 'text', value: String(Math.round(shown * 100) / 100), id: rows ? 'row-height' : 'column-width', spellcheck: false, attrs: { inputmode: 'decimal', autocomplete: 'off' } })
    const max = rows ? MAX_ROW_HEIGHT : MAX_COLUMN_CHARACTERS
    const lines = spans(this.selection.ranges, rows)
    const dialog = new Dialog({
      title: rows ? 'Row Height' : 'Column Width',
      id: rows ? 'row-height-dialog' : 'column-width-dialog',
      className: 'small',
      body: [field(rows ? 'Row height' : 'Column width', input, rows ? 'points' : 'characters')],
      buttons: [{
        label: 'OK',
        primary: true,
        action: () => {
          const value = numberIn(input.value)
          if (value === null || value < 0 || value > max) {
            dialog.fail(`The ${rows ? 'row height' : 'column width'} must be a number from 0 to ${max}.`, input)
            return false
          }
          const size = rows ? value : widthOfCharacters(value, digit)
          const edits = lines.map(([first, last]) => ({ op: rows ? 'rowHeight' : 'columnWidth', sheet, start: first, count: last - first + 1, size }))
          return commit(this.app, dialog, edits.length === 1 ? edits[0] : { op: 'batch', edits }, { sheet, ranges: [] })
        }
      }, { label: 'Cancel', value: null }],
      initial: input,
      restore: this.toSheet
    })
    await dialog.open()
  }

  rowHeight () {
    return this.size(true)
  }

  columnWidth () {
    return this.size(false)
  }

  // Go To ------------------------------------------------------------------------

  // The workbook's names and the last references gone to; OK goes through
  // `resolve`, as the Name Box does, and a reference it refuses keeps the
  // dialog open with the reason.
  async goTo () {
    const workbook = this.app.workbook()
    const names = ((workbook && workbook.names) || []).map((name) => name.name)
    const items = [...new Set([...this.recent, ...names])]
    const list = select(items, undefined, { size: 8, 'aria-label': 'Go to' })
    list.id = 'goto-list'
    const reference = el('input', { type: 'text', id: 'goto-reference', spellcheck: false, attrs: { autocomplete: 'off' } })
    list.addEventListener('change', () => {
      reference.value = list.value
    })
    const dialog = new Dialog({
      title: 'Go To',
      id: 'goto-dialog',
      className: 'small',
      body: [field('Go to', list), field('Reference', reference)],
      buttons: [{
        label: 'OK',
        primary: true,
        id: 'goto-ok',
        action: async () => {
          const text = reference.value.trim() || list.value
          if (!text) {
            dialog.fail('Type a reference or choose a name.', reference)
            return false
          }
          const refused = await this.app.editor.go(text, { nameBox: false })
          if (refused) {
            dialog.fail(refused, reference)
            return false
          }
          this.recent = [text, ...this.recent.filter((item) => item !== text)].slice(0, MAX_RECENT)
          return true
        }
      }, { label: 'Cancel', value: null }],
      initial: reference,
      restore: this.toSheet
    })
    list.addEventListener('dblclick', () => dialog.primary.click())
    await dialog.open()
  }

  // Find and Replace -------------------------------------------------------------

  // One dialog with a Find and a Replace tab (Ctrl+F, Ctrl+H). Find Next
  // asks the service for the next match after the active cell (addendum 10)
  // and selects it, the dialog staying open; Replace All is the `replace`
  // op, and says how many cells it changed. The Replace tab looks in
  // formulas, as Excel's does, so Find Next there stops where Replace All
  // would change.
  async findReplace (mode = 'find') {
    const asked = this.search
    const text = el('input', { type: 'text', id: 'find-text', value: asked.text, spellcheck: false, attrs: { autocomplete: 'off' } })
    const replacement = el('input', { type: 'text', id: 'find-replacement', value: asked.replacement, spellcheck: false, attrs: { autocomplete: 'off' } })
    const scope = select([['sheet', 'Sheet'], ['workbook', 'Workbook']], asked.scope)
    scope.id = 'find-scope'
    const lookIn = select([['formulas', 'Formulas'], ['values', 'Values']], asked.in)
    lookIn.id = 'find-in'
    const matchCase = el('input', { type: 'checkbox', id: 'find-case', checked: asked.matchCase })
    const entireCell = el('input', { type: 'checkbox', id: 'find-entire', checked: asked.entireCell })
    const status = el('p', { className: 'find-status', id: 'find-status', attrs: { role: 'status', 'aria-live': 'polite' } })
    const replaceField = field('Replace with', replacement)
    const check = (input, label) => el('div', { className: 'field check' }, input, el('label', { text: label, attrs: { for: input.id } }))
    const tabs = [['find', 'Find'], ['replace', 'Replace']].map(([key, label]) => el('button', {
      type: 'button',
      className: 'dialog-tab',
      id: 'find-tab-' + key,
      text: label,
      dataset: { tab: key },
      attrs: { role: 'tab', 'aria-controls': 'find-panel' },
      on: { click: () => choose(key) }
    }))
    const list = el('div', { className: 'dialog-tabs', attrs: { role: 'tablist', 'aria-label': 'Find and Replace' } }, tabs)
    const panel = el('div', { id: 'find-panel', className: 'find-panel', attrs: { role: 'tabpanel' } },
      field('Find what', text), replaceField,
      el('div', { className: 'find-options' },
        field('Within', scope), field('Look in', lookIn),
        el('div', {}, check(matchCase, 'Match case'), check(entireCell, 'Match entire cell contents'))),
      status)
    const remember = () => {
      this.search = {
        text: text.value,
        replacement: replacement.value,
        scope: scope.value,
        in: mode === 'replace' ? this.search.in : lookIn.value,
        matchCase: matchCase.checked,
        entireCell: entireCell.checked
      }
    }
    const say = (message) => {
      dialog.fail('')
      status.textContent = message
    }
    const wanted = () => {
      if (text.value) return true
      status.textContent = ''
      dialog.fail('Type what to find.', text)
      return false
    }
    // The worksheet a search starts on: the one shown, or in the workbook
    // scope, from a chart sheet, the next worksheet.
    const start = () => {
      const sheets = ((this.app.workbook() || {}).sheets || []).filter((item) => item.state === 'visible')
      const shown = this.app.sheetKey()
      const at = sheets.findIndex((item) => item.key === shown)
      if (sheets[at] && sheets[at].kind === 'worksheet') return shown
      if (scope.value === 'workbook') {
        const next = [...sheets.slice(at + 1), ...sheets.slice(0, Math.max(0, at))].find((item) => item.kind === 'worksheet')
        if (next) return next.key
      }
      status.textContent = ''
      dialog.fail('Show a worksheet to search it, or search the workbook.')
      return null
    }
    const dialog = new Dialog({
      title: 'Find and Replace',
      id: 'find-dialog',
      className: 'find',
      body: [list, panel],
      buttons: [{
        label: 'Replace All',
        id: 'find-replace-all',
        action: async () => {
          if (!wanted()) return false
          remember()
          const sheet = start()
          if (sheet === null) return false
          const edit = {
            op: 'replace', scope: scope.value, sheet, text: text.value, replacement: replacement.value,
            in: 'formulas', matchCase: matchCase.checked, entireCell: entireCell.checked
          }
          let answer
          try {
            answer = await this.app.edit(edit, { sheet, ranges: [], quiet: true })
          } catch (error) {
            if (!(error instanceof ApiError)) throw error
            status.textContent = ''
            dialog.fail(error.kind === 'stale_base' ? 'Another tab changed the workbook first; it now shows that change. Choose Replace All again.' : error.message)
            return false
          }
          const count = (answer.result && answer.result.replaced) || 0
          say(count ? `All done. We made ${count} replacement${count === 1 ? '' : 's'}.` : 'We couldn\'t find anything to replace.')
          return false
        }
      }, {
        label: 'Find Next',
        id: 'find-next',
        primary: true,
        action: async () => {
          if (!wanted()) return false
          remember()
          const sheet = start()
          if (sheet === null) return false
          const query = new URLSearchParams({
            text: text.value,
            scope: scope.value,
            sheet: String(sheet),
            in: lookIn.value,
            matchCase: String(matchCase.checked),
            entireCell: String(entireCell.checked)
          })
          if (this.grid.geometry && this.grid.sheet === sheet) query.set('from', this.selection.activeRef)
          let match
          try {
            match = await this.api.get('find?' + query, { quiet: true })
          } catch (error) {
            if (!(error instanceof ApiError)) throw error
            status.textContent = ''
            dialog.fail(error.message)
            return false
          }
          if (!match) {
            say('We couldn\'t find what you were looking for.')
            return false
          }
          const found = ((this.app.workbook() || {}).sheets || []).find((item) => item.key === match.sheet)
          const shown = await this.reveal(match.sheet, match.ref)
          say(shown ? `Found at ${found ? found.name + '!' : ''}${match.ref}.` : `${match.ref} could not be shown.`)
          return false
        }
      }, { label: 'Close', id: 'find-close', value: null }],
      initial: text,
      restore: this.toSheet
    })
    const replaceAll = dialog.buttons[0]
    // The Replace tab looks in formulas only; the Find tab's choice waits.
    const choose = (key, focus = false) => {
      mode = key
      for (const tab of tabs) {
        const selected = tab.dataset.tab === key
        tab.setAttribute('aria-selected', String(selected))
        tab.tabIndex = selected ? 0 : -1
        if (selected && focus) tab.focus()
      }
      panel.setAttribute('aria-labelledby', 'find-tab-' + key)
      replaceField.hidden = key !== 'replace'
      replaceAll.hidden = key !== 'replace'
      lookIn.disabled = key === 'replace'
      lookIn.value = key === 'replace' ? 'formulas' : this.search.in
      dialog.element.dataset.mode = key
    }
    list.addEventListener('keydown', (event) => {
      if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
      event.preventDefault()
      choose(mode === 'find' ? 'replace' : 'find', true)
    })
    lookIn.addEventListener('change', () => {
      if (mode === 'find') this.search = { ...this.search, in: lookIn.value }
    })
    dialog.element.addEventListener('keydown', (event) => {
      const ctrl = event.ctrlKey || event.metaKey
      if (!ctrl || event.altKey || event.shiftKey) return
      const key = event.key.toLowerCase()
      if (key !== 'f' && key !== 'h') return
      event.preventDefault()
      choose(key === 'f' ? 'find' : 'replace')
      text.focus()
      text.select()
    })
    choose(mode)
    await dialog.open()
  }

  // Show a cell the service named: its sheet, then the cell, selected and
  // scrolled into view. Answers whether it could.
  async reveal (key, ref) {
    const at = parseRef(ref)
    if (!at) return false
    if (key !== this.app.sheetKey()) await this.app.showSheet(key)
    if (this.app.sheetKey() !== key || !this.grid.geometry || this.grid.sheet !== key) return false
    this.selection.select(at.row, at.column)
    this.grid.ensureVisible(at.row, at.column)
    return true
  }

  // Custom Sort -------------------------------------------------------------------

  // Levels of a column and an order over the range `sortTarget` names (the
  // selection, or the current region of one cell), and whether its first
  // row is a header; OK is one `sort` op. A column is named by its header's
  // text when the first row is one, as Excel names it, and its order reads
  // as numbers or as text by the first value under the header.
  async sortDialog () {
    if (!this.cells()) return
    if (this.selection.ranges.length !== 1) {
      toast('This action won\'t work on multiple selections.', 'info')
      return
    }
    const sheet = this.app.sheetKey()
    const active = { ...this.selection.active }
    const single = this.selection.isSingle()
    const found = await sortTarget({ api: this.api, grid: this.grid, selection: this.selection, tiles: this.tiles, sheet })
    if (sheet !== this.app.sheetKey()) return
    if (!found) {
      toast('Select at least two rows to sort.', 'info')
      return
    }
    const { range } = found
    const columns = []
    for (let c = range.c0; c <= range.c1 && columns.length < MAX_SORT_COLUMNS; c++) columns.push(c)
    const most = Math.min(MAX_SORT_LEVELS, columns.length)
    const header = el('input', { type: 'checkbox', id: 'sort-header', checked: found.header })
    const label = (c) => {
      const cell = header.checked ? this.tiles.cell(sheet, range.r0, c) : null
      return cell && cell.text !== '' ? cell.text : `Column ${columnName(c)}`
    }
    const orders = (c) => {
      const cell = this.tiles.cell(sheet, range.r0 + (header.checked ? 1 : 0), c)
      return cell && cell.kind === KIND_NUMBER ? ['Smallest to Largest', 'Largest to Smallest'] : ['A to Z', 'Z to A']
    }
    const levels = [{ column: columns.includes(active.column) ? active.column : columns[0], descending: false }]
    const list = el('div', { className: 'sort-levels', id: 'sort-levels', attrs: { role: 'list', 'aria-label': 'Sort levels' } })
    const add = el('button', { type: 'button', className: 'secondary', id: 'sort-add', text: 'Add Level' })
    const draw = (focus = null) => {
      list.replaceChildren(...levels.map((level, i) => {
        const name = i === 0 ? 'Sort by' : 'Then by'
        const column = select(columns.map((c) => [String(c), label(c)]), String(level.column), { 'aria-label': `${name}, column` })
        column.id = `sort-column-${i}`
        const order = select([['asc', ''], ['desc', '']], level.descending ? 'desc' : 'asc', { 'aria-label': `${name}, order` })
        order.id = `sort-order-${i}`
        const relabel = () => {
          orders(level.column).forEach((text, n) => {
            order.options[n].textContent = text
          })
        }
        relabel()
        column.addEventListener('change', () => {
          level.column = Number(column.value)
          relabel()
        })
        order.addEventListener('change', () => {
          level.descending = order.value === 'desc'
        })
        const button = (text, id, aria, disabled, act) => el('button', {
          type: 'button', className: 'secondary sort-move', id, text, disabled, attrs: { 'aria-label': aria }, on: { click: act }
        })
        const move = (to) => () => {
          levels.splice(to, 0, levels.splice(i, 1)[0])
          draw(`#sort-column-${to}`)
        }
        return el('div', { className: 'sort-level', attrs: { role: 'listitem' }, dataset: { level: i } },
          el('span', { className: 'sort-level-name', text: name }), column, order,
          button('↑', `sort-up-${i}`, `Move level ${i + 1} up`, i === 0, move(i - 1)),
          button('↓', `sort-down-${i}`, `Move level ${i + 1} down`, i === levels.length - 1, move(i + 1)),
          button('×', `sort-delete-${i}`, `Delete level ${i + 1}`, levels.length === 1, () => {
            levels.splice(i, 1)
            draw(`#sort-column-${Math.min(i, levels.length - 1)}`)
          }))
      }))
      add.disabled = levels.length >= most
      const target = focus && list.querySelector(focus)
      if (target) target.focus()
    }
    add.addEventListener('click', () => {
      if (levels.length >= most) return
      const unused = columns.find((c) => !levels.some((level) => level.column === c))
      levels.push({ column: unused ?? columns[0], descending: false })
      draw(`#sort-column-${levels.length - 1}`)
    })
    header.addEventListener('change', () => draw())
    draw()
    const dialog = new Dialog({
      title: 'Sort',
      id: 'sort-dialog',
      className: 'sort',
      body: [
        el('div', { className: 'sort-head' }, add,
          el('div', { className: 'field check' }, header, el('label', { text: 'My data has headers', attrs: { for: header.id } }))),
        list,
        el('p', { className: 'field-hint', id: 'sort-range', text: `Sorts ${rangeName(range)}.` })
      ],
      buttons: [{
        label: 'OK',
        primary: true,
        id: 'sort-ok',
        action: async () => {
          const keys = levels.map((level) => ({ column: columnName(level.column), descending: level.descending }))
          const edit = { op: 'sort', sheet, range: rangeName(range), header: header.checked, keys }
          const done = await commit(this.app, dialog, edit, { sheet, ranges: [range] })
          if (done && single && sheet === this.app.sheetKey()) this.selection.selectRange(range, active)
          return done
        }
      }, { label: 'Cancel', value: null }],
      initial: list.querySelector('#sort-column-0'),
      restore: this.toSheet
    })
    await dialog.open()
  }

  // Insert Function ---------------------------------------------------------------

  // The service's functions (`GET functions`) by category, or those a
  // search finds (the category then reads Recommended); the one chosen shows
  // its signature and what it does, and OK writes `NAME()` into the entry
  // (editor.js), the caret between the parentheses - into the formula being
  // typed, or as the active cell's new formula.
  async insertFunction () {
    if (!this.cells()) return
    const editor = this.app.editor
    let list
    try {
      list = await editor.functions()
    } catch (error) {
      toast(error instanceof ApiError ? error.message : 'The list of functions could not be read.')
      return
    }
    const byName = new Map(list.map((item) => [item.name, item]))
    const categories = []
    for (const item of list) if (!categories.includes(item.category)) categories.push(item.category)
    const search = el('input', {
      type: 'search',
      id: 'function-search',
      spellcheck: false,
      attrs: { autocomplete: 'off', placeholder: 'Type a brief description of what you want to do' }
    })
    const go = el('button', { type: 'button', className: 'secondary', id: 'function-go', text: 'Go' })
    const category = select([['recent', 'Most Recently Used'], ['all', 'All'], ...categories.map((name) => [name, name])], 'recent')
    category.id = 'function-category'
    const recommended = el('option', { value: 'search', text: 'Recommended' })
    const choices = el('select', { id: 'function-choices', size: 10, attrs: { 'aria-label': 'Select a function' } })
    const signature = el('p', { className: 'function-signature', id: 'function-signature', attrs: { 'aria-live': 'polite' } })
    const about = el('p', { className: 'function-about', id: 'function-about' })
    const shown = () => {
      const item = byName.get(choices.value)
      if (!item) {
        signature.replaceChildren()
        about.textContent = choices.options.length ? '' : 'No function matches.'
        return
      }
      const open = item.signature.indexOf('(')
      signature.replaceChildren(el('b', { text: open > 0 ? item.signature.slice(0, open) : item.name }), open > 0 ? item.signature.slice(open) : '')
      about.textContent = item.description || ''
    }
    const fill = (items) => {
      choices.replaceChildren(...items.map((item) => el('option', { value: item.name, text: item.name })))
      if (items.length) choices.value = items[0].name
      shown()
    }
    const listed = () => {
      if (category.value === 'search') return searchFunctions(list, search.value)
      if (category.value === 'recent') return this.recentFunctions.map((name) => byName.get(name)).filter(Boolean)
      if (category.value === 'all') return [...list].sort((a, b) => a.name.localeCompare(b.name))
      return list.filter((item) => item.category === category.value).sort((a, b) => a.name.localeCompare(b.name))
    }
    const find = () => {
      if (!search.value.trim()) return
      if (!recommended.isConnected) category.prepend(recommended)
      category.value = 'search'
      fill(listed())
    }
    go.addEventListener('click', find)
    // Enter in the search field searches, as Go does; OK stays for the list.
    search.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return
      event.preventDefault()
      event.stopPropagation()
      find()
    })
    category.addEventListener('change', () => {
      if (category.value !== 'search') recommended.remove()
      fill(listed())
    })
    choices.addEventListener('change', shown)
    fill(listed())
    const dialog = new Dialog({
      title: 'Insert Function',
      id: 'function-dialog',
      className: 'functions',
      body: [
        el('div', { className: 'field' },
          el('label', { text: 'Search for a function', attrs: { for: search.id } }),
          el('div', { className: 'function-find' }, search, go)),
        field('Or select a category', category),
        field('Select a function', choices),
        signature,
        about
      ],
      buttons: [
        { label: 'OK', primary: true, id: 'function-ok', action: () => choices.value || false },
        { label: 'Cancel', value: null, id: 'function-cancel' }
      ],
      initial: search,
      restore: () => (editor.isOpen() ? editor.focus() : this.app.focusGrid())
    })
    choices.addEventListener('dblclick', () => dialog.primary.click())
    const name = await dialog.open()
    if (!name) return
    this.recentFunctions = [name, ...this.recentFunctions.filter((other) => other !== name)].slice(0, MAX_RECENT_FUNCTIONS)
    editor.insertFunction(name)
  }
}
