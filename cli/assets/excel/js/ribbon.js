// The Home tab (§10). Every control is a command that issues §8.4 ops
// against the selection; its pressed states and pickers follow the active
// cell's style. Toggles read the active cell, as Excel's do: Bold on a bold
// cell makes the selection not bold. The clipboard commands are
// clipboard.js's; Format Cells, the sizing dialogs, Find and Replace and
// Custom Sort dialogs.js's; the sheet commands sheets.js's. A drag of the
// fill handle (grid.js) fills through here.

import { toast } from './api.js'
import { MAX_ROWS, MAX_COLUMNS, columnName, contains, rangeName, refName, parseRange, parseRef } from './geometry.js'
import { KIND_NUMBER, KIND_TEXT } from './tiles.js'
import { colorItems, pasteItems, sortItems } from './menus.js'

// Number formats by the dropdown's option values. Built-ins go as the en-US
// codes their ids render (§3.4); Accounting and Comma are custom codes.
export const FORMATS = {
  general: 'General',
  number: '0.00',
  currency: '"$"#,##0.00',
  accounting: '_("$"* #,##0.00_);_("$"* \\(#,##0.00\\);_("$"* "-"??_);_(@_)',
  shortDate: 'm/d/yyyy',
  longDate: 'dddd, mmmm d, yyyy',
  time: 'h:mm:ss AM/PM',
  percentage: '0.00%',
  fraction: '# ?/?',
  scientific: '0.00E+00',
  text: '@',
  percent: '0%',
  comma: '_(* #,##0.00_);_(* \\(#,##0.00\\);_(* "-"??_);_(@_)'
}

const MAX_INDENT = 250
const MAX_TOTALS = 4096

const isWholeRows = (range) => range.c0 === 0 && range.c1 === MAX_COLUMNS - 1
const isWholeColumns = (range) => range.r0 === 0 && range.r1 === MAX_ROWS - 1

// What a sort acts on (§8.4 `sort`): the selection, or on one cell its
// current region (the service's `edge`), whole rows or columns cut to the
// used range; and whether its first row reads as a header - text over a
// second row that is not, the way Excel guesses one. Null when the range
// is one row, with nothing to sort.
export const sortTarget = async ({ api, grid, selection, tiles, sheet }) => {
  const { active } = selection
  let range = selection.range
  if (selection.isSingle()) {
    const answer = await api.get(`sheets/${sheet}/edge?from=${refName(active.row, active.column)}&direction=region`)
    range = parseRange((answer && answer.range) || '') || range
  } else if (isWholeRows(range) || isWholeColumns(range)) {
    const used = grid.geometry.dimension
    range = {
      r0: Math.max(range.r0, used.r0), c0: Math.max(range.c0, used.c0),
      r1: Math.min(range.r1, used.r1), c1: Math.min(range.c1, used.c1)
    }
  }
  if (range.r0 >= range.r1) return null
  const column = Math.min(Math.max(active.column, range.c0), range.c1)
  const first = tiles.cell(sheet, range.r0, column)
  const second = tiles.cell(sheet, range.r0 + 1, column)
  const header = Boolean(first && first.kind === KIND_TEXT && first.text !== '' && !(second && second.kind === KIND_TEXT))
  return { range, header }
}

// Merged, sorted spans [first, last] of the ranges on one axis, last first,
// so a batch of inserts or removals never moves a span still to come.
const spans = (ranges, axis) => {
  const list = ranges.map((range) => axis === 'rows' ? [range.r0, range.r1] : [range.c0, range.c1]).sort((a, b) => a[0] - b[0])
  const out = []
  for (const span of list) {
    const last = out[out.length - 1]
    if (last && span[0] <= last[1] + 1) last[1] = Math.max(last[1], span[1])
    else out.push([...span])
  }
  return out.reverse()
}

export class Ribbon {
  constructor ({ app, api, grid, selection, tiles, styles, menus, keyboard, panel }) {
    this.app = app
    this.api = api
    this.grid = grid
    this.selection = selection
    this.tiles = tiles
    this.styles = styles
    this.menus = menus
    this.panel = panel
    this.frame = 0
    this.register()
    keyboard.use((event) => this.key(event))
    selection.addEventListener('change', () => this.sync())
    styles.addEventListener('change', () => this.sync())
    tiles.addEventListener('tile', (event) => {
      if (event.detail.sheet === this.app.sheetKey()) this.sync()
    })
  }

  // The selection as the service names it, on the sheet shown.
  get sheet () {
    return this.app.sheetKey()
  }

  // Cell commands need a worksheet on screen.
  cells () {
    if (this.grid.geometry && this.grid.sheet === this.sheet) return true
    toast('Select cells on a worksheet first.', 'info')
    return false
  }

  single () {
    if (this.selection.ranges.length === 1) return true
    toast("This action won't work on multiple selections.", 'info')
    return false
  }

  edit (edit, ranges = this.selection.ranges) {
    return this.app.edit(edit, { sheet: this.sheet, ranges })
  }

  // One or several ops as one edit, one undo step.
  edits (list, ranges) {
    if (list.length === 0) return null
    return this.edit(list.length === 1 ? list[0] : { op: 'batch', edits: list }, ranges)
  }

  style (patch) {
    if (!this.cells()) return null
    return this.edit({ op: 'setStyle', sheet: this.sheet, ranges: this.selection.refs, patch })
  }

  // The active cell's style, normalized (styles.js).
  activeStyle () {
    const { row, column } = this.selection.active
    const found = this.tiles.cell(this.sheet, row, column)
    const id = found ? found.style : (this.grid.geometry && this.grid.geometry.blankStyle(row, column)) || 0
    return this.styles.get(id || 0)
  }

  register () {
    const command = (name, run) => this.app.command(name, run)
    const toggle = (name, items, label) => command(name, ({ element }) => {
      if (element) this.menus.toggle(typeof items === 'function' ? items() : items, element, label)
    })

    // Clipboard: the buttons name the commands clipboard.js registers.
    toggle('pasteMenu', pasteItems, 'Paste')

    // Font.
    command('bold', () => this.style({ bold: !this.activeStyle().font.bold }))
    command('italic', () => this.style({ italic: !this.activeStyle().font.italic }))
    command('underline', () => {
      const underline = this.activeStyle().font.underline
      return this.style({ underline: underline && underline !== 'none' ? 'none' : 'single' })
    })
    command('strike', () => this.style({ strike: !this.activeStyle().font.strike }))
    command('fontName', ({ value }) => value && this.style({ fontName: value }))
    command('fontSize', ({ value }) => {
      const size = Number(value)
      if (size > 0 && size <= 409) return this.style({ fontSize: size })
    })
    toggle('bordersMenu', () => [
      { label: 'Bottom Border', command: 'borders', args: { preset: 'bottom' } },
      { label: 'Top Border', command: 'borders', args: { preset: 'top' } },
      { label: 'Left Border', command: 'borders', args: { preset: 'left' } },
      { label: 'Right Border', command: 'borders', args: { preset: 'right' } },
      { separator: true },
      { label: 'No Border', command: 'borders', args: { preset: 'none' } },
      { label: 'All Borders', command: 'borders', args: { preset: 'all' } },
      { label: 'Outside Borders', command: 'borders', args: { preset: 'outside' } },
      { label: 'Thick Outside Borders', command: 'borders', args: { preset: 'thickOutside' } }
    ], 'Borders')
    command('borders', ({ preset = 'bottom' }) => this.style({
      borders: { preset, style: preset === 'thickOutside' ? 'thick' : 'thin', color: null }
    }))
    toggle('fillMenu', () => colorItems('fill', 'No Fill'), 'Fill Color')
    command('fill', ({ color = null }) => this.style({ fill: color }))
    toggle('fontColorMenu', () => colorItems('fontColor', 'Automatic'), 'Font Color')
    command('fontColor', ({ color = null }) => this.style({ fontColor: color }))

    // Alignment: a pressed horizontal alignment returns to General, a
    // pressed vertical one to Bottom, as Excel's buttons do.
    const horizontal = (value) => () => this.style({ horizontal: this.activeStyle().horizontal === value ? 'general' : value })
    const vertical = (value) => () => this.style({ vertical: this.activeStyle().vertical === value && value !== 'bottom' ? 'bottom' : value })
    command('alignLeft', horizontal('left'))
    command('alignCenter', horizontal('center'))
    command('alignRight', horizontal('right'))
    command('alignTop', vertical('top'))
    command('alignMiddle', vertical('center'))
    command('alignBottom', vertical('bottom'))
    command('wrap', () => this.style({ wrap: !this.activeStyle().wrap }))
    command('indentMore', () => {
      const style = this.activeStyle()
      const patch = { indent: Math.min(MAX_INDENT, style.indent + 1) }
      if (style.horizontal !== 'left' && style.horizontal !== 'right' && style.horizontal !== 'distributed') patch.horizontal = 'left'
      return this.style(patch)
    })
    command('indentLess', () => {
      const style = this.activeStyle()
      if (style.indent > 0) return this.style({ indent: style.indent - 1 })
    })
    toggle('mergeMenu', () => [
      { label: 'Merge & Center', command: 'merge', args: { center: true } },
      { label: 'Merge Across', command: 'merge', args: { across: true } },
      { label: 'Merge Cells', command: 'merge', args: {} },
      { label: 'Unmerge Cells', command: 'unmerge' }
    ], 'Merge')
    command('merge', (options) => this.merge(options))
    command('unmerge', () => this.unmerge())

    // Number.
    command('numberFormat', ({ element, value }) => {
      if (value === 'more') {
        if (element) this.sync()
        return this.app.run('formatCells', { tab: 'number' })
      }
      if (FORMATS[value]) return this.style({ numberFormat: FORMATS[value] })
    })
    command('currency', () => this.style({ numberFormat: FORMATS.accounting }))
    command('percent', () => this.style({ numberFormat: FORMATS.percent }))
    command('comma', () => this.style({ numberFormat: FORMATS.comma }))
    // One decimal more or fewer: a delta the service applies once per
    // distinct source style (`FormatCode::with_decimals`), never beside a
    // `numberFormat` in one patch (contract addendum 5).
    command('decimalsMore', () => this.style({ decimals: 1 }))
    command('decimalsLess', () => this.style({ decimals: -1 }))

    // Cells.
    toggle('insertMenu', () => [
      { label: 'Insert Sheet Rows', command: 'insertRows' },
      { label: 'Insert Sheet Columns', command: 'insertColumns' },
      { separator: true },
      { label: 'Insert Sheet', command: 'insertSheet', shortcut: 'Shift+F11' }
    ], 'Insert')
    toggle('deleteMenu', () => [
      { label: 'Delete Sheet Rows', command: 'deleteRows' },
      { label: 'Delete Sheet Columns', command: 'deleteColumns' },
      { separator: true },
      { label: 'Delete Sheet', command: 'deleteSheet' }
    ], 'Delete')
    toggle('formatMenu', () => this.formatItems(), 'Format')
    command('insertRows', () => this.structure('insertRows'))
    command('insertColumns', () => this.structure('insertColumns'))
    command('deleteRows', () => this.structure('removeRows'))
    command('deleteColumns', () => this.structure('removeColumns'))
    command('hideRows', () => this.hide('rows', true))
    command('hideColumns', () => this.hide('columns', true))
    command('unhideRows', () => this.hide('rows', false))
    command('unhideColumns', () => this.hide('columns', false))

    // Editing.
    toggle('autoSumMenu', () => [
      { label: 'Sum', command: 'autoSum', args: { name: 'SUM' }, shortcut: 'Alt+=' },
      { label: 'Average', command: 'autoSum', args: { name: 'AVERAGE' } },
      { label: 'Count Numbers', command: 'autoSum', args: { name: 'COUNT' } },
      { label: 'Max', command: 'autoSum', args: { name: 'MAX' } },
      { label: 'Min', command: 'autoSum', args: { name: 'MIN' } }
    ], 'AutoSum')
    command('autoSum', ({ name = 'SUM' } = {}) => this.autoSum(name))
    toggle('fillMenu2', () => [
      { label: 'Down', command: 'fillDown', shortcut: 'Ctrl+D' },
      { label: 'Right', command: 'fillRight', shortcut: 'Ctrl+R' },
      { label: 'Up', command: 'fillUp' },
      { label: 'Left', command: 'fillLeft' }
    ], 'Fill')
    command('fillHandle', (drag) => this.fillHandle(drag))
    command('fillToRegion', () => this.fillToRegion())
    this.grid.addEventListener('fill', (event) => this.app.run('fillHandle', event.detail))
    this.grid.addEventListener('filldouble', () => this.app.run('fillToRegion'))
    command('fillDown', () => this.fill('down'))
    command('fillRight', () => this.fill('right'))
    command('fillUp', () => this.fill('up'))
    command('fillLeft', () => this.fill('left'))
    toggle('clearMenu', () => [
      { label: 'Clear All', command: 'clearAll' },
      { label: 'Clear Formats', command: 'clearFormats' },
      { label: 'Clear Contents', command: 'clearContents', shortcut: 'Delete' }
    ], 'Clear')
    const clear = (what) => () => this.cells() && this.edit({ op: 'clear', sheet: this.sheet, ranges: this.selection.refs, what })
    command('clearAll', clear('all'))
    command('clearFormats', clear('formats'))
    command('clearContents', clear('contents'))
    toggle('sortMenu', sortItems, 'Sort')
    command('sortAscending', () => this.sort(false))
    command('sortDescending', () => this.sort(true))
    toggle('findMenu', () => [
      { label: 'Find…', command: 'find', shortcut: 'Ctrl+F' },
      { label: 'Replace…', command: 'replace', shortcut: 'Ctrl+H' },
      { label: 'Go To…', command: 'goTo', shortcut: 'Ctrl+G' }
    ], 'Find & Select')
  }

  formatItems () {
    const workbook = this.app.workbook()
    const hidden = workbook ? workbook.sheets.filter((sheet) => sheet.state === 'hidden') : []
    return [
      { heading: 'Cell Size' },
      { label: 'Row Height…', command: 'rowHeightDialog' },
      { label: 'Column Width…', command: 'columnWidthDialog' },
      { heading: 'Visibility' },
      {
        label: 'Hide & Unhide',
        submenu: [
          { label: 'Hide Rows', command: 'hideRows' },
          { label: 'Hide Columns', command: 'hideColumns' },
          { label: 'Hide Sheet', command: 'hideSheet' },
          { separator: true },
          { label: 'Unhide Rows', command: 'unhideRows' },
          { label: 'Unhide Columns', command: 'unhideColumns' },
          hidden.length
            ? { label: 'Unhide Sheet', submenu: hidden.map((sheet) => ({ label: sheet.name, command: 'unhideSheet', args: { key: sheet.key } })) }
            : { label: 'Unhide Sheet', disabled: true }
        ]
      },
      { heading: 'Organize Sheets' },
      { label: 'Rename Sheet', command: 'renameSheet', args: { key: this.sheet } },
      { label: 'Move Sheet…', command: 'moveSheet', args: { key: this.sheet } },
      { separator: true },
      { label: 'Format Cells…', command: 'formatCells', shortcut: 'Ctrl+1' }
    ]
  }

  // Merge each selected range, or, for Merge & Center on a cell already
  // merged, unmerge it (the button toggles).
  merge ({ center = false, across = false } = {}) {
    if (!this.cells()) return null
    const { active } = this.selection
    const merged = this.grid.geometry.merges.at(active.row, active.column)
    if (center && !across && merged && this.selection.isSingle()) return this.unmerge()
    const list = this.selection.ranges
      .filter((range) => range.r0 !== range.r1 || range.c0 !== range.c1)
      .map((range) => ({ op: 'merge', sheet: this.sheet, range: rangeName(range), center, across }))
    return this.edits(list)
  }

  unmerge () {
    if (!this.cells()) return null
    const found = new Map()
    for (const range of this.selection.ranges) {
      for (const merge of this.grid.geometry.merges.within(range)) found.set(rangeName(merge), merge)
    }
    return this.edits([...found.keys()].map((range) => ({ op: 'unmerge', sheet: this.sheet, range })), [...found.values()])
  }

  // Insert or remove the rows or columns the selection spans.
  structure (op) {
    if (!this.cells()) return null
    const rows = op.endsWith('Rows')
    const insert = op.startsWith('insert')
    const list = spans(this.selection.ranges, rows ? 'rows' : 'columns').map(([first, last]) => ({
      op,
      sheet: this.sheet,
      [insert ? 'at' : 'start']: first,
      count: last - first + 1
    }))
    const marked = this.selection.ranges.map((range) => rows
      ? { r0: range.r0, c0: 0, r1: range.r1, c1: MAX_COLUMNS - 1 }
      : { r0: 0, c0: range.c0, r1: MAX_ROWS - 1, c1: range.c1 })
    return this.edits(list, marked)
  }

  hide (axis, hidden) {
    if (!this.cells()) return null
    const op = axis === 'rows' ? 'hideRows' : 'hideColumns'
    const list = spans(this.selection.ranges, axis).map(([first, last]) => ({
      op, sheet: this.sheet, start: first, count: last - first + 1, hidden
    }))
    return this.edits(list, [])
  }

  // AutoSum: under or beside a block of numbers. On one cell the formula
  // opens in the editor over the numbers above it, else those to its left;
  // on a range, each column gets its total in the row below.
  async autoSum (name) {
    if (!this.cells()) return
    const sheet = this.sheet
    const selection = this.selection
    if (!selection.isSingle()) {
      const entries = []
      for (const range of selection.ranges) {
        if (range.r1 >= MAX_ROWS - 1) continue
        for (let c = range.c0; c <= range.c1 && entries.length <= MAX_TOTALS; c++) {
          entries.push({ ref: refName(range.r1 + 1, c), text: `=${name}(${refName(range.r0, c)}:${refName(range.r1, c)})` })
        }
      }
      if (entries.length === 0) return
      if (entries.length > MAX_TOTALS) {
        toast(`AutoSum writes at most ${MAX_TOTALS} totals at once.`)
        return
      }
      return this.edit({ op: 'setEntries', sheet, entries }, entries.map((entry) => parseRange(entry.ref)))
    }
    const { row, column } = selection.active
    const range = await this.numbersBeside(sheet, row, column)
    if (sheet !== this.sheet) return
    const text = range ? `=${name}(${rangeName(range)})` : `=${name}()`
    this.app.editor.begin({ text, mode: 'edit', caret: range ? text.length : text.length - 1 })
  }

  // The run of numbers ending next to (row, column): the service's `edge`
  // bounds it, the tiles' kinds trim it to numbers.
  async numbersBeside (sheet, row, column) {
    const number = (r, c) => {
      const cell = this.tiles.cell(sheet, r, c)
      return cell === undefined ? true : Boolean(cell && cell.kind === KIND_NUMBER)
    }
    const known = (r, c) => {
      const cell = this.tiles.cell(sheet, r, c)
      return Boolean(cell && cell.kind === KIND_NUMBER)
    }
    // `edge` from a cell whose neighbour is filled answers the block's end;
    // from one whose neighbour is empty it would jump past the gap.
    if (row > 0 && known(row - 1, column)) {
      let top = row - 1
      if (row > 1 && this.tiles.cell(sheet, row - 2, column) !== null) {
        const answer = await this.api.get(`sheets/${sheet}/edge?from=${refName(row - 1, column)}&direction=up`, { quiet: true })
        const bound = parseRef((answer && answer.ref) || '')
        while (bound && top - 1 >= bound.row && number(top - 1, column)) top--
      }
      return { r0: top, c0: column, r1: row - 1, c1: column }
    }
    if (column > 0 && known(row, column - 1)) {
      let left = column - 1
      if (column > 1 && this.tiles.cell(sheet, row, column - 2) !== null) {
        const answer = await this.api.get(`sheets/${sheet}/edge?from=${refName(row, column - 1)}&direction=left`, { quiet: true })
        const bound = parseRef((answer && answer.ref) || '')
        while (bound && left - 1 >= bound.column && number(row, left - 1)) left--
      }
      return { r0: row, c0: left, r1: row, c1: column - 1 }
    }
    return null
  }

  // Fill Down/Right/Up/Left: the edge row or column of the selection copied
  // across it; on one cell, the neighbour copied into it (Ctrl+D, Ctrl+R).
  fill (direction) {
    if (!this.cells() || !this.single()) return null
    const range = { ...this.selection.range }
    let source
    let target = range
    if (direction === 'down' || direction === 'up') {
      if (range.r0 === range.r1) {
        const from = direction === 'down' ? range.r0 - 1 : range.r0 + 1
        if (from < 0 || from >= MAX_ROWS) return null
        source = { ...range, r0: from, r1: from }
        target = { ...range, r0: Math.min(from, range.r0), r1: Math.max(from, range.r1) }
      } else {
        const at = direction === 'down' ? range.r0 : range.r1
        source = { ...range, r0: at, r1: at }
      }
    } else {
      if (range.c0 === range.c1) {
        const from = direction === 'right' ? range.c0 - 1 : range.c0 + 1
        if (from < 0 || from >= MAX_COLUMNS) return null
        source = { ...range, c0: from, c1: from }
        target = { ...range, c0: Math.min(from, range.c0), c1: Math.max(from, range.c1) }
      } else {
        const at = direction === 'right' ? range.c0 : range.c1
        source = { ...range, c0: at, c1: at }
      }
    }
    return this.edit({ op: 'fill', sheet: this.sheet, source: rangeName(source), target: rangeName(target), mode: 'copy' }, [target])
  }

  // A drag of the fill handle (grid.js): a `fill` of the source over the
  // target, a series unless Ctrl was held at the release (§10); drawn back
  // over the selection, a clear of the contents left behind. The target is
  // selected at once, the active cell staying where it was.
  fillHandle ({ source, target, clear, copy }) {
    if (!this.cells()) return null
    const active = { ...this.selection.active }
    this.selection.selectRange(target, contains(target, active.row, active.column) ? active : { row: target.r0, column: target.c0 })
    if (clear) return this.edit({ op: 'clear', sheet: this.sheet, ranges: [rangeName(clear)], what: 'contents' }, [clear])
    return this.edit({
      op: 'fill', sheet: this.sheet, source: rangeName(source), target: rangeName(target), mode: copy ? 'copy' : 'series'
    }, [target])
  }

  // A double click on the fill handle: a series down to the bottom of the
  // current region the selection stands in.
  async fillToRegion () {
    if (!this.cells() || !this.single()) return
    const sheet = this.sheet
    const source = { ...this.selection.range }
    const { active } = this.selection
    const answer = await this.api.get(`sheets/${sheet}/edge?from=${refName(active.row, active.column)}&direction=region`)
    const region = parseRange((answer && answer.range) || '')
    if (sheet !== this.sheet || !region || region.r1 <= source.r1) return
    return this.fillHandle({ source, target: { ...source, r1: region.r1 }, clear: null, copy: false })
  }

  // Sort A to Z or Z to A by the active cell's column (`sortTarget`).
  async sort (descending) {
    if (!this.cells() || !this.single()) return
    const sheet = this.sheet
    const active = { ...this.selection.active }
    const single = this.selection.isSingle()
    const found = await sortTarget({ api: this.api, grid: this.grid, selection: this.selection, tiles: this.tiles, sheet })
    if (!found || sheet !== this.sheet) return
    const { range, header } = found
    const keys = [{ column: columnName(active.column), descending }]
    await this.edit({ op: 'sort', sheet, range: rangeName(range), header, keys }, [range])
    if (sheet === this.sheet && single) this.selection.selectRange(range, active)
  }

  // Keys (§10): Ctrl+B/I/U/5, Delete, Ctrl+D, Ctrl+R, Alt+=, Shift+F11; and
  // Ctrl+1, Ctrl+F/H/G, F5, Ctrl+S and F9 for the commands later phases
  // register - a key whose command is not there yet stays the browser's.
  key (event) {
    const ctrl = event.ctrlKey || event.metaKey
    const run = (name) => {
      if (!this.app.has(name)) return false
      event.preventDefault()
      this.app.run(name)
      return true
    }
    const plain = !event.altKey && !event.shiftKey
    if (ctrl && plain) {
      switch (event.key.toLowerCase()) {
        case 'b': return run('bold')
        case 'i': return run('italic')
        case 'u': return run('underline')
        case '5': return run('strike')
        case 'd': return run('fillDown')
        case 'r': return run('fillRight')
        case '1': return run('formatCells')
        case 'f': return run('find')
        case 'h': return run('replace')
        case 'g': return run('goTo')
        case 's': return run('save')
      }
      return false
    }
    if (ctrl) return false
    if (plain && event.key === 'Delete') return run('clearContents')
    if (plain && event.key === 'F5') return run('goTo')
    if (plain && event.key === 'F9') return run('calculate')
    if (event.altKey && !event.shiftKey && event.key === '=') return run('autoSum')
    if (event.shiftKey && !event.altKey && event.key === 'F11') return run('insertSheet')
    return false
  }

  // Pressed states and pickers from the active cell, once per frame.
  sync () {
    if (this.frame) return
    this.frame = requestAnimationFrame(() => {
      this.frame = 0
      this.update()
    })
  }

  update () {
    const panel = this.panel
    const style = this.activeStyle()
    const press = (name, on) => {
      const element = panel.querySelector(`[data-command="${name}"]`)
      if (element) element.setAttribute('aria-pressed', on ? 'true' : 'false')
    }
    press('bold', style.font.bold)
    press('italic', style.font.italic)
    press('underline', style.font.underline && style.font.underline !== 'none')
    press('strike', style.font.strike)
    press('alignLeft', style.horizontal === 'left')
    press('alignCenter', style.horizontal === 'center' || style.horizontal === 'centerContinuous')
    press('alignRight', style.horizontal === 'right')
    press('alignTop', style.vertical === 'top')
    press('alignMiddle', style.vertical === 'center')
    press('alignBottom', style.vertical === 'bottom')
    press('wrap', style.wrap)
    this.pick(panel.querySelector('[data-command="fontName"]'), style.font.name || 'Calibri')
    this.pick(panel.querySelector('[data-command="fontSize"]'), String(style.font.size || 11))
    const format = panel.querySelector('[data-command="numberFormat"]')
    if (format && document.activeElement !== format) {
      const key = Object.keys(FORMATS).find((name) => FORMATS[name] === style.numberFormat &&
        format.querySelector(`option[value="${name}"]`))
      format.value = key || 'custom'
    }
    panel.dataset.active = this.selection.activeRef
  }

  // Show `value` in a picker, adding it when the list lacks it.
  pick (select, value) {
    if (!select || document.activeElement === select) return
    if (![...select.options].some((option) => option.value === value)) {
      const option = document.createElement('option')
      option.value = value
      option.textContent = value
      option.dataset.added = 'true'
      select.append(option)
    }
    select.value = value
  }
}
