// Menus: the one popup every drop-down and context menu opens - role=menu,
// roving focus, submenus, typeahead, colour swatches - and the context menus
// of §10 for cells, row and column headers and sheet tabs. An item names a
// command; one no phase has registered yet shows disabled.

import { MAX_ROWS, MAX_COLUMNS } from './geometry.js'

const SUBMENU_DELAY = 180
const SWATCH_COLUMNS = 10

// The Office theme's ten colours, then Excel's standard row.
const THEME = [
  ['White', '#FFFFFF'], ['Black', '#000000'], ['Light Gray', '#E7E6E6'], ['Blue-Gray', '#44546A'],
  ['Blue', '#4472C4'], ['Orange', '#ED7D31'], ['Gray', '#A5A5A5'], ['Gold', '#FFC000'],
  ['Light Blue', '#5B9BD5'], ['Green', '#70AD47']
]
const STANDARD = [
  ['Dark Red', '#C00000'], ['Red', '#FF0000'], ['Orange', '#FFC000'], ['Yellow', '#FFFF00'],
  ['Light Green', '#92D050'], ['Green', '#00B050'], ['Light Blue', '#00B0F0'], ['Blue', '#0070C0'],
  ['Dark Blue', '#002060'], ['Purple', '#7030A0']
]
const SHADES = [['Lighter 80%', 0.8], ['Lighter 60%', 0.6], ['Lighter 40%', 0.4], ['Darker 25%', -0.25], ['Darker 50%', -0.5]]

const shade = (hex, amount) => {
  const value = parseInt(hex.slice(1), 16)
  const channel = (shift) => {
    const c = (value >> shift) & 255
    return Math.round(amount >= 0 ? c + (255 - c) * amount : c * (1 + amount))
  }
  return '#' + ((channel(16) << 16) | (channel(8) << 8) | channel(0)).toString(16).padStart(6, '0').toUpperCase()
}

// The swatches of a colour menu: the theme row, five shades of each, the
// standard row. Each is `{ name, color }`.
export const THEME_SWATCHES = [
  ...THEME.map(([name, color]) => ({ name, color })),
  ...SHADES.flatMap(([label, amount]) => THEME.map(([name, color]) => ({ name: `${name}, ${label}`, color: shade(color, amount) })))
]
export const STANDARD_SWATCHES = STANDARD.map(([name, color]) => ({ name, color }))

// A colour menu: `none` is the item clearing the colour ("No Fill",
// "Automatic"); a swatch runs `command` with `{ color }`.
export const colorItems = (command, none) => [
  { label: none, command, args: { color: null } },
  { heading: 'Theme Colors' },
  { swatches: THEME_SWATCHES, command },
  { heading: 'Standard Colors' },
  { swatches: STANDARD_SWATCHES, command }
]

export const pasteItems = () => [
  { label: 'Paste', command: 'paste', shortcut: 'Ctrl+V' },
  { label: 'Values', command: 'pasteValues' },
  { label: 'Formulas', command: 'pasteFormulas' },
  { label: 'Formatting', command: 'pasteFormats' },
  { separator: true },
  { label: 'Paste Special…', command: 'pasteSpecial', shortcut: 'Ctrl+Alt+V' }
]

export const sortItems = () => [
  { label: 'Sort A to Z', command: 'sortAscending' },
  { label: 'Sort Z to A', command: 'sortDescending' },
  { label: 'Custom Sort…', command: 'sortDialog' }
]

const isWholeRows = (range) => range.c0 === 0 && range.c1 === MAX_COLUMNS - 1
const isWholeColumns = (range) => range.r0 === 0 && range.r1 === MAX_ROWS - 1

export class Menus {
  constructor ({ app, grid, selection, keyboard, tabs }) {
    this.app = app
    this.grid = grid
    this.selection = selection
    this.stack = []
    this.suppressed = null
    this.timer = 0
    this.typed = ''
    this.typedAt = 0

    // A press outside every open menu closes them; a press on the button
    // that opened the menu closes it without reopening it on the click.
    document.addEventListener('pointerdown', (event) => {
      if (this.stack.length === 0) return
      if (this.stack.some((menu) => menu.element.contains(event.target))) return
      const root = this.stack[0]
      if (root.invoker && root.invoker.contains(event.target)) this.suppressed = root.invoker
      this.closeAll(false)
    }, true)
    window.addEventListener('blur', () => this.closeAll(false))
    window.addEventListener('resize', () => this.closeAll(false))

    grid.addEventListener('context', (event) => {
      const { kind, x, y } = event.detail
      this.open(this.itemsFor(kind), { x, y, label: kind === 'cell' ? 'Cell' : kind === 'row' ? 'Row' : 'Column' })
    })
    keyboard.use((event) => this.gridKey(event))

    tabs.addEventListener('contextmenu', (event) => {
      const tab = event.target.closest('.sheet-tab')
      if (!tab) return
      event.preventDefault()
      const key = Number(tab.dataset.key)
      Promise.resolve(app.showSheet(key)).finally(() => {
        this.open(this.tabItems(key), { x: event.clientX, y: event.clientY, label: 'Sheet' })
      })
    })
    tabs.addEventListener('keydown', (event) => {
      if (!(event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))) return
      const tab = event.target.closest('.sheet-tab')
      if (!tab) return
      event.preventDefault()
      event.stopPropagation()
      this.open(this.tabItems(Number(tab.dataset.key)), { anchor: tab, label: 'Sheet', invoker: tab })
    })
  }

  isOpen () {
    return this.stack.length > 0
  }

  // A ribbon drop-down: opens under its button, or closes when open.
  toggle (items, invoker, label) {
    if (this.suppressed === invoker) {
      this.suppressed = null
      return
    }
    if (this.stack.length && this.stack[0].invoker === invoker) {
      this.closeAll(true)
      return
    }
    this.open(items, { anchor: invoker, invoker, label })
  }

  // Keys the grid hands to menus: the context-menu key, Shift+F10, and the
  // insert and delete chords (§10), which act at once on whole rows or
  // columns and otherwise ask which.
  gridKey (event) {
    const ctrl = event.ctrlKey || event.metaKey
    if (event.key === 'ContextMenu' || (event.shiftKey && !ctrl && event.key === 'F10')) {
      event.preventDefault()
      const kind = this.selectionKind()
      const box = this.activeBox()
      this.open(this.itemsFor(kind), { x: box.x, y: box.y, label: 'Cell' })
      return true
    }
    if (ctrl && !event.altKey && (event.key === '+' || (event.shiftKey && event.key === '='))) {
      event.preventDefault()
      this.structural('insert')
      return true
    }
    if (ctrl && !event.altKey && !event.shiftKey && event.key === '-') {
      event.preventDefault()
      this.structural('delete')
      return true
    }
    return false
  }

  structural (verb) {
    const kind = this.selectionKind()
    const insert = verb === 'insert'
    if (kind === 'row') return this.app.run(insert ? 'insertRows' : 'deleteRows')
    if (kind === 'column') return this.app.run(insert ? 'insertColumns' : 'deleteColumns')
    const box = this.activeBox()
    this.open([
      { label: insert ? 'Insert Entire Row' : 'Delete Entire Row', command: insert ? 'insertRows' : 'deleteRows' },
      { label: insert ? 'Insert Entire Column' : 'Delete Entire Column', command: insert ? 'insertColumns' : 'deleteColumns' }
    ], { x: box.x, y: box.y, label: insert ? 'Insert' : 'Delete' })
  }

  selectionKind () {
    const ranges = this.selection.ranges
    if (ranges.every(isWholeRows) && !ranges.every(isWholeColumns)) return 'row'
    if (ranges.every(isWholeColumns) && !ranges.every(isWholeRows)) return 'column'
    return 'cell'
  }

  // The active cell's bottom-left corner in client pixels.
  activeBox () {
    const viewport = this.grid.viewport
    const canvas = this.grid.canvas.getBoundingClientRect()
    if (!viewport) return { x: canvas.left + 40, y: canvas.top + 40 }
    const { row, column } = this.selection.active
    const merge = this.grid.geometry.merges.at(row, column)
    const box = viewport.rect(merge || { r0: row, c0: column, r1: row, c1: column })
    const x = Math.min(Math.max(canvas.left + box.x, canvas.left), canvas.right - 20)
    const y = Math.min(Math.max(canvas.top + box.y + box.h, canvas.top), canvas.bottom - 20)
    return { x, y }
  }

  itemsFor (kind) {
    const rows = kind === 'row'
    const columns = kind === 'column'
    const items = [
      { label: 'Cut', command: 'cut', shortcut: 'Ctrl+X' },
      { label: 'Copy', command: 'copy', shortcut: 'Ctrl+C' },
      { label: 'Paste', command: 'paste', shortcut: 'Ctrl+V' },
      { label: 'Paste Special', submenu: pasteItems().slice(1) },
      { separator: true }
    ]
    if (rows || columns) {
      items.push(
        { label: 'Insert', command: rows ? 'insertRows' : 'insertColumns' },
        { label: 'Delete', command: rows ? 'deleteRows' : 'deleteColumns' },
        { label: 'Clear Contents', command: 'clearContents', shortcut: 'Delete' },
        { separator: true },
        { label: 'Format Cells…', command: 'formatCells', shortcut: 'Ctrl+1' },
        rows ? { label: 'Row Height…', command: 'rowHeightDialog' } : { label: 'Column Width…', command: 'columnWidthDialog' },
        { label: 'Hide', command: rows ? 'hideRows' : 'hideColumns' },
        { label: 'Unhide', command: rows ? 'unhideRows' : 'unhideColumns' }
      )
      return items
    }
    items.push(
      { label: 'Insert', submenu: [{ label: 'Entire Row', command: 'insertRows' }, { label: 'Entire Column', command: 'insertColumns' }] },
      { label: 'Delete', submenu: [{ label: 'Entire Row', command: 'deleteRows' }, { label: 'Entire Column', command: 'deleteColumns' }] },
      { label: 'Clear Contents', command: 'clearContents', shortcut: 'Delete' },
      { separator: true },
      { label: 'Sort', submenu: sortItems() },
      { separator: true },
      { label: 'Format Cells…', command: 'formatCells', shortcut: 'Ctrl+1' }
    )
    return items
  }

  tabItems (key) {
    const workbook = this.app.workbook()
    const sheets = workbook ? workbook.sheets : []
    const hidden = sheets.filter((sheet) => sheet.state === 'hidden')
    return [
      { label: 'Insert Sheet', command: 'insertSheet', args: { before: key }, shortcut: 'Shift+F11' },
      { label: 'Delete', command: 'deleteSheet', args: { key } },
      { label: 'Rename', command: 'renameSheet', args: { key } },
      { label: 'Move…', command: 'moveSheet', args: { key } },
      { separator: true },
      { label: 'Hide', command: 'hideSheet', args: { key } },
      hidden.length
        ? { label: 'Unhide', submenu: hidden.map((sheet) => ({ label: sheet.name, command: 'unhideSheet', args: { key: sheet.key } })) }
        : { label: 'Unhide', disabled: true }
    ]
  }

  // Rendering ------------------------------------------------------------------

  open (items, { anchor = null, x = 0, y = 0, label = 'Menu', invoker = null, parent = null } = {}) {
    if (!parent) this.closeAll(false)
    const element = document.createElement('div')
    element.className = 'menu'
    element.setAttribute('role', 'menu')
    element.setAttribute('aria-label', label)
    element.tabIndex = -1
    const menu = { element, entries: [], invoker, parent, child: null }
    for (const item of items) this.render(menu, item)
    document.body.append(element)
    this.place(element, anchor, x, y, parent)
    this.stack.push(menu)
    if (parent) parent.child = menu
    if (invoker && invoker.hasAttribute('aria-expanded')) invoker.setAttribute('aria-expanded', 'true')
    element.addEventListener('keydown', (event) => this.key(menu, event))
    element.addEventListener('click', (event) => {
      const entry = menu.entries.find((candidate) => candidate.element.contains(event.target))
      if (entry) this.activate(menu, entry, event.detail === 0)
    })
    element.addEventListener('pointerover', (event) => {
      const entry = menu.entries.find((candidate) => candidate.element.contains(event.target))
      if (entry) this.hover(menu, entry)
    })
    const first = menu.entries.find((entry) => !entry.disabled)
    ;(first ? first.element : element).focus({ preventScroll: true })
    return menu
  }

  render (menu, item) {
    if (!item) return
    const { element } = menu
    if (item.separator) {
      const line = document.createElement('div')
      line.className = 'menu-separator'
      line.setAttribute('role', 'separator')
      element.append(line)
      return
    }
    if (item.heading) {
      const heading = document.createElement('div')
      heading.className = 'menu-heading'
      heading.setAttribute('role', 'presentation')
      heading.textContent = item.heading
      element.append(heading)
      return
    }
    if (item.swatches) {
      const group = document.createElement('div')
      group.className = 'menu-swatches'
      group.setAttribute('role', 'group')
      const disabled = !this.app.has(item.command)
      const members = []
      for (const swatch of item.swatches) {
        const button = document.createElement('button')
        button.type = 'button'
        button.className = 'menu-swatch'
        button.setAttribute('role', 'menuitem')
        button.setAttribute('aria-label', swatch.name)
        button.title = swatch.name
        button.tabIndex = -1
        button.dataset.color = swatch.color
        button.style.backgroundColor = swatch.color
        if (disabled) button.setAttribute('aria-disabled', 'true')
        group.append(button)
        const entry = { element: button, item: { command: item.command, args: { ...(item.args || {}), color: swatch.color } }, disabled, members }
        members.push(entry)
        menu.entries.push(entry)
      }
      element.append(group)
      return
    }
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'menu-item'
    button.tabIndex = -1
    button.setAttribute('role', item.checked === undefined ? 'menuitem' : 'menuitemcheckbox')
    if (item.checked !== undefined) button.setAttribute('aria-checked', String(Boolean(item.checked)))
    const check = document.createElement('span')
    check.className = 'menu-check'
    check.setAttribute('aria-hidden', 'true')
    check.textContent = item.checked ? '✓' : ''
    const text = document.createElement('span')
    text.className = 'menu-label'
    text.textContent = item.label
    button.append(check, text)
    if (item.shortcut) {
      const keys = document.createElement('span')
      keys.className = 'menu-shortcut'
      keys.textContent = item.shortcut
      button.setAttribute('aria-keyshortcuts', item.shortcut.replace(/Ctrl/g, 'Control'))
      button.append(keys)
    }
    if (item.submenu) {
      button.setAttribute('aria-haspopup', 'menu')
      button.setAttribute('aria-expanded', 'false')
      const arrow = document.createElement('span')
      arrow.className = 'menu-arrow'
      arrow.setAttribute('aria-hidden', 'true')
      arrow.textContent = '›'
      button.append(arrow)
    }
    const disabled = Boolean(item.disabled) || (!item.submenu && !this.app.has(item.command)) ||
      (item.submenu && item.submenu.length === 0)
    if (disabled) button.setAttribute('aria-disabled', 'true')
    element.append(button)
    menu.entries.push({ element: button, item, disabled })
  }

  // Under an anchor, at a point, or beside a parent item; flipped and
  // clamped to stay on screen.
  place (element, anchor, x, y, parent) {
    element.style.left = '0px'
    element.style.top = '0px'
    const box = element.getBoundingClientRect()
    const width = window.innerWidth
    const height = window.innerHeight
    let left = x
    let top = y
    if (anchor) {
      const rect = anchor.getBoundingClientRect()
      if (parent) {
        left = rect.right - 2
        top = rect.top - 4
        if (left + box.width > width - 4) left = rect.left - box.width + 2
      } else {
        left = rect.left
        top = rect.bottom + 2
        if (top + box.height > height - 4 && rect.top - box.height - 2 > 4) top = rect.top - box.height - 2
      }
    } else if (top + box.height > height - 4) {
      top = Math.max(4, top - box.height)
    }
    left = Math.max(4, Math.min(left, width - box.width - 4))
    top = Math.max(4, Math.min(top, height - box.height - 4))
    element.style.left = Math.round(left) + 'px'
    element.style.top = Math.round(top) + 'px'
  }

  // Closing ----------------------------------------------------------------------

  closeFrom (menu) {
    const at = this.stack.indexOf(menu)
    if (at < 0) return
    for (const open of this.stack.splice(at)) {
      open.element.remove()
      if (open.invoker && open.invoker.hasAttribute('aria-expanded')) open.invoker.setAttribute('aria-expanded', 'false')
      if (open.parent) open.parent.child = null
    }
  }

  // Close every menu; `restore` hands the keys back to where they were:
  // the ribbon button that opened the menu, else the sheet.
  closeAll (restore = true) {
    clearTimeout(this.timer)
    if (this.stack.length === 0) return
    const root = this.stack[0]
    this.closeFrom(root)
    if (!restore) return
    if (root.invoker && document.contains(root.invoker)) root.invoker.focus()
    else this.app.focusGrid()
  }

  // Items -------------------------------------------------------------------------

  activate (menu, entry, byKey) {
    if (entry.disabled) return
    const { item } = entry
    if (item.submenu) {
      this.openSubmenu(menu, entry, byKey)
      return
    }
    const root = this.stack[0]
    const invoker = root && root.invoker
    this.closeAll(false)
    // A ribbon menu used by the keyboard gives the keys back to its button;
    // otherwise they go to the sheet, as a ribbon click does.
    if (byKey && invoker && document.contains(invoker) && invoker.closest('.ribbon')) invoker.focus()
    else this.app.focusGrid()
    this.app.run(item.command, item.args || {})
  }

  openSubmenu (menu, entry, focus) {
    clearTimeout(this.timer)
    if (menu.child && menu.child.invoker === entry.element) {
      if (focus) this.focusEntry(menu.child, 0)
      return
    }
    if (menu.child) this.closeFrom(menu.child)
    const child = this.open(entry.item.submenu, {
      anchor: entry.element,
      invoker: entry.element,
      parent: menu,
      label: entry.item.label
    })
    if (!focus) entry.element.focus({ preventScroll: true })
    return child
  }

  hover (menu, entry) {
    if (menu.child && menu.child.invoker !== entry.element) {
      clearTimeout(this.timer)
      this.timer = setTimeout(() => {
        if (menu.child) this.closeFrom(menu.child)
      }, SUBMENU_DELAY)
    }
    if (entry.disabled) return
    entry.element.focus({ preventScroll: true })
    if (entry.item.submenu) {
      clearTimeout(this.timer)
      this.timer = setTimeout(() => this.openSubmenu(menu, entry, false), SUBMENU_DELAY)
    }
  }

  focusEntry (menu, index) {
    const enabled = menu.entries.filter((entry) => !entry.disabled)
    if (enabled.length === 0) return
    const at = ((index % enabled.length) + enabled.length) % enabled.length
    enabled[at].element.focus({ preventScroll: true })
  }

  key (menu, event) {
    const enabled = menu.entries.filter((entry) => !entry.disabled)
    const current = enabled.findIndex((entry) => entry.element === document.activeElement)
    const entry = enabled[current]
    const inSwatches = entry && entry.members
    const move = (to) => {
      event.preventDefault()
      this.focusEntry(menu, to)
    }
    switch (event.key) {
      case 'ArrowDown': {
        if (inSwatches) {
          const at = entry.members.indexOf(entry)
          if (at + SWATCH_COLUMNS < entry.members.length) {
            event.preventDefault()
            entry.members[at + SWATCH_COLUMNS].element.focus({ preventScroll: true })
            return
          }
          const last = enabled.indexOf(entry.members[entry.members.length - 1])
          return move(last + 1)
        }
        return move(current + 1)
      }
      case 'ArrowUp': {
        if (inSwatches) {
          const at = entry.members.indexOf(entry)
          if (at - SWATCH_COLUMNS >= 0) {
            event.preventDefault()
            entry.members[at - SWATCH_COLUMNS].element.focus({ preventScroll: true })
            return
          }
          const first = enabled.indexOf(entry.members[0])
          return move(first - 1)
        }
        return move(current - 1)
      }
      case 'Home':
        return move(0)
      case 'End':
        return move(enabled.length - 1)
      case 'ArrowRight':
        if (inSwatches) return move(current + 1)
        if (entry && entry.item.submenu) {
          event.preventDefault()
          this.openSubmenu(menu, entry, true)
        }
        return
      case 'ArrowLeft':
        if (inSwatches) return move(current - 1)
        if (menu.parent) {
          event.preventDefault()
          const back = menu.invoker
          this.closeFrom(menu)
          back.focus({ preventScroll: true })
        }
        return
      case 'Enter':
      case ' ':
        event.preventDefault()
        if (entry) this.activate(menu, entry, true)
        return
      case 'Escape': {
        event.preventDefault()
        event.stopPropagation()
        if (menu.parent) {
          const back = menu.invoker
          this.closeFrom(menu)
          back.focus({ preventScroll: true })
        } else this.closeAll(true)
        return
      }
      case 'Tab':
        event.preventDefault()
        this.closeAll(true)
        return
    }
    // Typeahead: the next item whose label starts with what was typed.
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      const now = performance.now()
      this.typed = now - this.typedAt < 700 ? this.typed + event.key.toLowerCase() : event.key.toLowerCase()
      this.typedAt = now
      const labelled = enabled.filter((candidate) => candidate.item.label)
      const from = Math.max(0, labelled.indexOf(entry))
      for (let step = this.typed.length > 1 ? 0 : 1; step <= labelled.length; step++) {
        const candidate = labelled[(from + step) % labelled.length]
        if (candidate && candidate.item.label.toLowerCase().startsWith(this.typed)) {
          event.preventDefault()
          candidate.element.focus({ preventScroll: true })
          return
        }
      }
    }
  }
}
