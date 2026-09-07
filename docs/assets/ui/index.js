/*
 * Pure ESM is the single browser contract: MkDocs accepts module scripts, so
 * this graph stays disconnected from the native CommonJS addon without a
 * generated global or bundling step.
 */

const ASSET_PROMISES = new Map()
let controlId = 0

const classNames = (base, extra) => (extra ? `${base} ${extra}` : base)

const documentOf = () => {
  const held = globalThis.document
  if (held === undefined) throw new Error('yggdryl/ui requires a browser document')
  return held
}

const isNode = (value) =>
  value !== null && typeof value === 'object' && typeof value.nodeType === 'number'

const appendContent = (parent, content) => {
  if (content === null || content === undefined) return
  if (Array.isArray(content)) {
    for (const value of content) appendContent(parent, value)
  } else if (isNode(content)) {
    parent.append(content)
  } else {
    parent.append(documentOf().createTextNode(String(content)))
  }
}

const defaultCell = (value) => {
  if (value === null || value === undefined) return value
  if (Array.isArray(value)) return value.map(defaultCell)
  return isNode(value) ? value : code({ text: value })
}

/** Create one element without interpreting its text as markup. */
export function make(tag, className, text) {
  const node = documentOf().createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = String(text)
  return node
}

export function code({ text, className }) {
  return make('code', classNames('ygg-ui__code', className), text)
}

export function card({ value, label, note: detail, className }) {
  const element = make('div', classNames('ygg-ui__card', className))
  const valueNode = make('span', 'ygg-ui__card-value')
  const labelNode = make('span', 'ygg-ui__card-label')
  appendContent(valueNode, value)
  appendContent(labelNode, label)
  element.append(valueNode, labelNode)
  if (detail !== null && detail !== undefined && detail !== '') {
    const noteNode = make('span', 'ygg-ui__card-note')
    appendContent(noteNode, detail)
    element.append(noteNode)
  }
  return element
}

export function cardRow({ cards, className }) {
  const element = make('div', classNames('ygg-ui__cards', className))
  for (const config of cards) element.append(card(config))
  return element
}

/** A native details panel whose body is constructed at most once. */
export function panel({
  title,
  aside,
  open = false,
  render,
  className,
  summaryClassName,
  bodyClassName,
}) {
  const element = make('details', classNames('ygg-ui__panel', className))
  const summary = make('summary', classNames('ygg-ui__summary', summaryClassName))
  const titleNode = make('span', 'ygg-ui__summary-title')
  appendContent(titleNode, title)
  summary.append(titleNode)
  if (aside !== null && aside !== undefined && aside !== '') {
    const asideNode = make('span', 'ygg-ui__summary-aside')
    appendContent(asideNode, aside)
    summary.append(asideNode)
  }
  const body = make('div', classNames('ygg-ui__panel-body', bodyClassName))
  element.append(summary, body)

  let filled = false
  const fill = () => {
    if (filled) return body
    filled = true
    appendContent(body, render?.(body))
    return body
  }
  element.addEventListener('toggle', () => {
    if (element.open) fill()
  })
  element.open = Boolean(open)
  if (element.open) fill()
  return { element, summary, body, fill }
}

export function facts({ rows, format = defaultCell, className }) {
  const table = make('table', classNames('ygg-ui__facts', className))
  const body = make('tbody')
  rows.forEach((row, index) => {
    if (row.value === null || row.value === undefined || row.value === '') return
    const line = make('tr')
    const key = make('th', null, row.label)
    key.setAttribute('scope', 'row')
    const cell = make('td')
    const formatter = row.format ?? format
    appendContent(cell, formatter(row.value, row, index))
    line.append(key, cell)
    body.append(line)
  })
  table.append(body)
  return table
}

export function grid({ columns, rows, empty = '—', className }) {
  const configured = columns.map((column) =>
    typeof column === 'string' ? { label: column } : column,
  )
  const holder = make('div', 'ygg-ui__scroll')
  const table = make('table', classNames('ygg-ui__grid', className))
  const head = make('thead')
  const heading = make('tr')
  for (const column of configured) {
    const cell = make('th', column.headerClassName)
    cell.setAttribute('scope', 'col')
    appendContent(cell, column.label)
    heading.append(cell)
  }
  head.append(heading)

  const body = make('tbody')
  rows.forEach((row, rowIndex) => {
    const line = make('tr')
    configured.forEach((column, columnIndex) => {
      const cell = make(column.rowHeader ? 'th' : 'td', column.className)
      if (column.rowHeader) cell.setAttribute('scope', 'row')
      const value = column.get
        ? column.get(row, rowIndex, columnIndex)
        : column.key !== undefined
          ? row[column.key]
          : row[columnIndex]
      if (value === null || value === undefined) {
        cell.textContent = empty
      } else {
        appendContent(
          cell,
          column.format ? column.format(value, row, rowIndex, columnIndex) : defaultCell(value),
        )
      }
      line.append(cell)
    })
    body.append(line)
  })
  table.append(head, body)
  holder.append(table)
  return holder
}

export function pill({ text, kind, className }) {
  const modifier = kind ? ` ygg-ui__pill--${kind}` : ''
  const element = make('span', classNames(`ygg-ui__pill${modifier}`, className))
  appendContent(element, text)
  return element
}

export function control({ id, label, node, className }) {
  const element = make('div', classNames('ygg-ui__control', className))
  const labelNode = make('label', 'ygg-ui__label')
  labelNode.setAttribute('for', id)
  appendContent(labelNode, label)
  node.id = id
  element.append(labelNode, node)
  return element
}

const applyTextControl = (node, config) => {
  if (config.value !== undefined) node.value = String(config.value)
  if (config.placeholder !== undefined) node.placeholder = config.placeholder
  if (config.name !== undefined) node.name = config.name
  if (config.autocomplete !== undefined) node.autocomplete = config.autocomplete
  if (config.disabled !== undefined) node.disabled = Boolean(config.disabled)
  if (config.ariaLabel !== undefined) node.setAttribute('aria-label', config.ariaLabel)
  if (config.onInput) node.addEventListener('input', config.onInput)
  if (config.onChange) node.addEventListener('change', config.onChange)
  return node
}

export function input(config = {}) {
  const wide = config.wide ? ' ygg-ui__input--wide' : ''
  const node = make('input', classNames(`ygg-ui__input${wide}`, config.className))
  node.type = config.type ?? 'text'
  return applyTextControl(node, config)
}

export function search(config = {}) {
  return input({ ...config, type: 'search' })
}

export function select(config = {}) {
  const wide = config.wide ? ' ygg-ui__select--wide' : ''
  const node = make('select', classNames(`ygg-ui__select${wide}`, config.className))
  for (const entry of config.options ?? []) {
    const option = make('option', null, entry.label)
    option.value = String(entry.value)
    if (entry.disabled !== undefined) option.disabled = Boolean(entry.disabled)
    if (entry.selected !== undefined) option.selected = Boolean(entry.selected)
    node.append(option)
  }
  if (config.value !== undefined) node.value = String(config.value)
  if (config.name !== undefined) node.name = config.name
  if (config.disabled !== undefined) node.disabled = Boolean(config.disabled)
  if (config.ariaLabel !== undefined) node.setAttribute('aria-label', config.ariaLabel)
  if (config.onChange) node.addEventListener('change', config.onChange)
  return node
}

export function textarea(config = {}) {
  const node = make('textarea', classNames('ygg-ui__textarea', config.className))
  if (config.rows !== undefined) node.rows = config.rows
  if (config.spellcheck !== undefined) node.spellcheck = Boolean(config.spellcheck)
  if (config.wrap !== undefined) node.wrap = config.wrap
  return applyTextControl(node, config)
}

export function button({
  label,
  kind,
  type = 'button',
  pressed,
  disabled,
  onClick,
  ariaLabel,
  className,
}) {
  const modifier = kind ? ` ygg-ui__button--${kind}` : ''
  const node = make('button', classNames(`ygg-ui__button${modifier}`, className))
  node.type = type
  appendContent(node, label)
  if (pressed !== undefined) node.setAttribute('aria-pressed', pressed ? 'true' : 'false')
  if (disabled !== undefined) node.disabled = Boolean(disabled)
  if (ariaLabel !== undefined) node.setAttribute('aria-label', ariaLabel)
  if (onClick) node.addEventListener('click', onClick)
  return node
}

const modalContains = (root, node) => {
  for (let held = node; held !== null && held !== undefined; held = held.parentNode) {
    if (held === root) return true
  }
  return false
}

const modalFocusables = (surface, closeButton) => {
  if (typeof surface.querySelectorAll !== 'function') return [closeButton]
  return [...surface.querySelectorAll(
    'a[href], button, input, select, textarea, [tabindex]',
  )].filter(
    (node) =>
      !node.disabled &&
      !node.hidden &&
      node.getAttribute('aria-hidden') !== 'true' &&
      node.getAttribute('tabindex') !== '-1',
  )
}

/** A modal dialog with native top-layer behavior and an accessible fallback. */
export function modal({
  title,
  content,
  actions,
  closeLabel = 'Close',
  closeOnEscape = true,
  closeOnBackdrop = true,
  initialFocus,
  onOpen,
  onClose,
  className,
  surfaceClassName,
  bodyClassName,
}) {
  const doc = documentOf()
  const element = make('dialog', classNames('ygg-ui ygg-ui__modal', className))
  const surface = make('div', classNames('ygg-ui__modal-surface', surfaceClassName))
  const header = make('header', 'ygg-ui__modal-header')
  const titleNode = make('h2', 'ygg-ui__modal-title')
  const titleId = `ygg-ui-modal-title-${++controlId}`
  titleNode.id = titleId
  appendContent(titleNode, title)
  const closeButton = button({ label: closeLabel, className: 'ygg-ui__modal-close' })
  header.append(titleNode, closeButton)
  const body = make('div', classNames('ygg-ui__modal-body', bodyClassName))
  appendContent(body, content)
  surface.append(header, body)
  if (actions !== null && actions !== undefined) {
    const footer = make('footer', 'ygg-ui__modal-actions')
    appendContent(footer, actions)
    surface.append(footer)
  }
  element.append(surface)
  element.setAttribute('role', 'dialog')
  element.setAttribute('aria-modal', 'true')
  element.setAttribute('aria-labelledby', titleId)
  element.setAttribute('aria-hidden', 'true')

  const native =
    typeof element.showModal === 'function' && typeof element.close === 'function'
  if (!native) {
    element.setAttribute('data-ygg-ui-fallback', '')
    element.hidden = true
  }

  let opened = false
  let previousFocus = null

  const restoreFocus = () => {
    const target = previousFocus
    previousFocus = null
    if (!target || target.isConnected === false || typeof target.focus !== 'function') return
    try {
      target.focus({ preventScroll: true })
    } catch {
      target.focus()
    }
  }

  const finishClose = (reason) => {
    if (!opened) return false
    opened = false
    element.setAttribute('aria-hidden', 'true')
    if (!native) {
      doc.removeEventListener('keydown', onFallbackKeydown)
      element.open = false
      element.removeAttribute('open')
      element.hidden = true
    }
    restoreFocus()
    onClose?.(reason)
    return true
  }

  const close = (reason = 'api') => {
    if (!opened) return
    if (native && element.open) element.close(String(reason))
    finishClose(reason)
  }

  const onFallbackKeydown = (event) => {
    if (!opened) return
    if (event.key === 'Escape') {
      if (!closeOnEscape) return
      event.preventDefault()
      close('escape')
      return
    }
    if (event.key !== 'Tab') return
    const focusable = modalFocusables(surface, closeButton)
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    const active = doc.activeElement
    if (event.shiftKey && (active === first || !modalContains(surface, active))) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && (active === last || !modalContains(surface, active))) {
      event.preventDefault()
      first.focus()
    }
  }

  const open = () => {
    if (opened) return
    previousFocus = doc.activeElement
    if (native) {
      element.showModal()
    } else {
      element.hidden = false
      element.open = true
      element.setAttribute('open', '')
      doc.addEventListener('keydown', onFallbackKeydown)
    }
    opened = true
    element.removeAttribute('aria-hidden')
    const target = initialFocus && modalContains(surface, initialFocus) ? initialFocus : closeButton
    target.focus?.()
    onOpen?.()
  }

  closeButton.addEventListener('click', () => close('button'))
  element.addEventListener('click', (event) => {
    if (!opened || !closeOnBackdrop || event.target !== element) return
    if (native && typeof element.getBoundingClientRect === 'function') {
      const box = element.getBoundingClientRect()
      if (
        Number.isFinite(event.clientX) &&
        event.clientX >= box.left &&
        event.clientX <= box.right &&
        event.clientY >= box.top &&
        event.clientY <= box.bottom
      ) return
    }
    close('backdrop')
  })
  element.addEventListener('cancel', (event) => {
    event.preventDefault()
    if (closeOnEscape) close('escape')
  })
  element.addEventListener('close', () => {
    finishClose(element.returnValue || 'native')
  })

  return {
    element,
    surface,
    body,
    closeButton,
    open,
    close,
    get opened() {
      return opened
    },
  }
}

export function choiceList({ items, render, onSelect, selected, className }) {
  const held = Array.from(items)
  const element = make('div', classNames('ygg-ui__choices', className))
  element.setAttribute('role', 'group')
  let current = selected === undefined ? -1 : selected

  const buttons = held.map((item, index) => {
    const node = button({ label: render(item, index), kind: 'chip', pressed: index === current })
    node.addEventListener('click', () => selectAt(index))
    element.append(node)
    return node
  })

  const selectAt = (index) => {
    if (!Number.isInteger(index) || index < 0 || index >= held.length) {
      throw new RangeError(`choice index ${index} is outside 0..${Math.max(held.length - 1, 0)}`)
    }
    current = index
    buttons.forEach((node, at) => node.setAttribute('aria-pressed', at === index ? 'true' : 'false'))
    onSelect?.(held[index], index)
  }

  return {
    element,
    buttons,
    select: selectAt,
    get selectedIndex() {
      return current
    },
  }
}

const fallbackCopy = (text) => {
  const doc = documentOf()
  if (!doc.body || typeof doc.execCommand !== 'function') return false
  const held = doc.createElement('textarea')
  held.value = text
  held.setAttribute('readonly', '')
  held.style.position = 'fixed'
  held.style.opacity = '0'
  doc.body.append(held)
  try {
    held.focus?.()
    held.select?.()
    return doc.execCommand('copy') === true
  } catch {
    return false
  } finally {
    held.remove()
  }
}

const copyText = async (text) => {
  try {
    const clipboard = globalThis.navigator?.clipboard
    if (clipboard && typeof clipboard.writeText === 'function') {
      await clipboard.writeText(text)
      return true
    }
  } catch {
    // A denied Clipboard API may still leave the document copy command usable.
  }
  return fallbackCopy(text)
}

export function wireBlock({ display, copy, duration = 1200, labels = {}, className }) {
  const element = make('div', classNames('ygg-ui__wire', className))
  const body = make('pre', 'ygg-ui__wire-text', display)
  const idle = labels.copy ?? 'Copy'
  const copied = labels.copied ?? 'Copied'
  const unavailable = labels.unavailable ?? 'Copy unavailable'
  const copyButton = button({ label: idle, className: 'ygg-ui__copy' })
  let timer = null

  const resetLater = () => {
    if (timer !== null) globalThis.clearTimeout(timer)
    timer = globalThis.setTimeout(() => {
      copyButton.textContent = idle
      timer = null
    }, duration)
  }
  const perform = async () => {
    const ok = await copyText(String(copy ?? display))
    copyButton.textContent = ok ? copied : unavailable
    resetLater()
    return ok
  }
  copyButton.addEventListener('click', () => {
    void perform()
  })
  element.append(body, copyButton)
  return { element, body, button: copyButton, copy: perform }
}

export function callBlock({ code: source, answer, className }) {
  const element = make('div', classNames('ygg-ui__call-block', className))
  if (answer !== null && answer !== undefined) {
    const answerNode = make('div', 'ygg-ui__call-answer')
    appendContent(answerNode, answer)
    element.append(answerNode)
  }
  const block = make('pre', 'ygg-ui__call')
  block.append(make('code', null, source))
  element.append(block)
  return element
}

export function note({ content, kind, className }) {
  const modifier = kind ? ` ygg-ui__note--${kind}` : ''
  const element = make('p', classNames(`ygg-ui__note${modifier}`, className))
  appendContent(element, content)
  return element
}

export function createSearchIndex({ items, haystack }) {
  const values = Array.from(items)
  const entries = values.map((item, index) => ({
    item,
    haystack: String(haystack(item, index)).toLowerCase(),
  }))
  return {
    entries,
    size: entries.length,
    search(query, filter) {
      const words = String(query).trim().toLowerCase().split(/\s+/).filter(Boolean)
      const found = []
      for (const entry of entries) {
        if (filter && !filter(entry.item)) continue
        if (words.every((word) => entry.haystack.includes(word))) found.push(entry.item)
      }
      return found
    },
  }
}

const checkedPageSize = (value) => {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`pageSize must be a positive finite integer, got ${value}`)
  }
  return value
}

export function searchableList({
  index,
  renderPage,
  pageSize = 60,
  noun = 'items',
  filter,
  label = 'Search',
  id,
  placeholder,
  query = '',
  controls: extraControls,
  className,
}) {
  const page = checkedPageSize(pageSize)
  const element = make('div', classNames('ygg-ui__searchable', className))
  const searchBox = search({ value: query, placeholder, autocomplete: 'off', wide: true })
  const searchId = id ?? `ygg-ui-search-${++controlId}`
  const searchControl = control({
    id: searchId,
    label,
    node: searchBox,
    className: 'ygg-ui__search-controls',
  })
  const controls = make('div', 'ygg-ui__controls')
  appendContent(controls, [searchControl, extraControls])
  const count = make('p', 'ygg-ui__counter')
  count.setAttribute('aria-live', 'polite')
  const list = make('div', 'ygg-ui__list')
  const more = button({ label: 'Show more' })
  let shown = page

  const refresh = ({ reset = false } = {}) => {
    if (reset) shown = page
    const matches = index.search(searchBox.value, filter)
    const visible = matches.slice(0, shown)
    const remaining = matches.length - visible.length
    count.textContent =
      `${matches.length.toLocaleString()} of ${index.size.toLocaleString()} ${noun}; ` +
      `${visible.length.toLocaleString()} drawn, ${remaining.toLocaleString()} not drawn`
    list.textContent = ''
    appendContent(list, renderPage(visible, { matches: matches.length, remaining }))
    more.hidden = remaining === 0
    more.textContent = `Show ${Math.min(page, remaining).toLocaleString()} more of ${remaining.toLocaleString()}`
    return { matches: matches.length, drawn: visible.length, remaining }
  }
  const reset = () => refresh({ reset: true })
  searchBox.addEventListener('input', reset)
  more.addEventListener('click', () => {
    shown += page
    refresh()
  })
  element.append(controls, count, list, more)
  refresh()
  return { element, controls, input: searchBox, count, list, more, refresh, reset }
}

export function tree({ items, key, describe, className }) {
  const level = (values, path, root = false) => {
    const list = make('ul', classNames('ygg-ui__tree', root ? className : undefined))
    list.setAttribute('role', 'list')
    for (const item of values) {
      const description = describe(item)
      if (description === null || description === undefined) continue
      const identity = key(item)
      const entry = make('li', 'ygg-ui__tree-item')
      if (path.has(identity)) {
        const line = make('div', 'ygg-ui__tree-line ygg-ui__tree-cycle')
        const labelNode = make('span', 'ygg-ui__tree-label')
        appendContent(labelNode, description.label)
        line.append(labelNode, pill({ text: 'cycle', kind: 'warning' }))
        entry.append(line)
        list.append(entry)
        continue
      }

      const branch = description.branch ?? typeof description.children === 'function'
      if (!branch) {
        const line = make('div', classNames('ygg-ui__tree-line', description.className))
        const labelNode = make('span', 'ygg-ui__tree-label')
        appendContent(labelNode, description.label)
        line.append(labelNode)
        if (description.aside !== null && description.aside !== undefined) {
          const asideNode = make('span', 'ygg-ui__tree-aside')
          appendContent(asideNode, description.aside)
          line.append(asideNode)
        }
        entry.append(line)
      } else {
        const next = new Set(path)
        next.add(identity)
        const block = panel({
          title: description.label,
          aside: description.aside,
          open: description.open,
          className: classNames('ygg-ui__tree-branch', description.className),
          render: () => {
            const content =
              typeof description.beforeChildren === 'function'
                ? description.beforeChildren(item)
                : description.beforeChildren
            const children = description.children ? Array.from(description.children(item)) : []
            return [content, children.length > 0 ? level(children, next) : null]
          },
        })
        entry.append(block.element)
      }
      list.append(entry)
    }
    return list
  }
  return level(Array.from(items), new Set(), true)
}

export function createLazyIndex({ build }) {
  let state = 'empty'
  let value
  let failure
  return {
    get built() {
      return state !== 'empty'
    },
    get() {
      if (state === 'ready') return value
      if (state === 'failed') throw failure
      if (state === 'building') throw new Error('lazy index build called itself')
      state = 'building'
      try {
        value = build()
        state = 'ready'
        return value
      } catch (error) {
        failure = error
        state = 'failed'
        throw error
      }
    },
  }
}

const afterPaint = (run) => {
  if (typeof globalThis.requestAnimationFrame === 'function') {
    globalThis.requestAnimationFrame(() => globalThis.requestAnimationFrame(run))
  } else {
    globalThis.setTimeout(run, 0)
  }
}

export function assetLoader({
  baseURL,
  assets,
  regenerate,
  fetch: fetcher = globalThis.fetch,
  schedule = afterPaint,
}) {
  const address = (name) => {
    if (!Object.hasOwn(assets, name)) throw new Error(`unknown asset ${name}`)
    return new URL(assets[name], baseURL).href
  }

  const json = (name) => {
    const url = address(name)
    let pending = ASSET_PROMISES.get(url)
    if (pending === undefined) {
      pending = Promise.resolve().then(async () => {
        if (typeof fetcher !== 'function') throw new Error('fetch is unavailable')
        const response = await fetcher(url)
        if (!response.ok) {
          const reason = `${response.status} ${response.statusText ?? ''}`.trim()
          throw new Error(reason)
        }
        return response.json()
      })
      ASSET_PROMISES.set(url, pending)
    }
    return pending
  }

  const prefetch = (name) => {
    const pending = new Promise((resolve, reject) => {
      schedule(() => json(name).then(resolve, reject))
    })
    pending.catch(() => {})
    return pending
  }

  const failure = ({ asset, error }) => {
    const reason = error instanceof Error ? error.message : String(error)
    const element = make('div')
    element.append(
      note({
        content: `The generated asset ${assets[asset] ?? asset} could not be loaded (${reason}). ` +
          'Regenerate it with:',
        kind: 'warning',
      }),
      callBlock({ code: regenerate }),
    )
    return element
  }

  return { address, json, prefetch, failure }
}

export function onDocumentReady({ run, document: doc = globalThis.document, navigation }) {
  let observable = navigation
  if (observable === undefined) {
    try {
      observable = globalThis.document$
    } catch {
      observable = undefined
    }
  }
  if (observable && typeof observable.subscribe === 'function') {
    const subscription = observable.subscribe(() => run(doc))
    return () => subscription?.unsubscribe?.()
  }
  if (!doc) throw new Error('yggdryl/ui requires a browser document')
  if (doc.readyState === 'loading') {
    const start = () => run(doc)
    doc.addEventListener('DOMContentLoaded', start, { once: true })
    return () => doc.removeEventListener('DOMContentLoaded', start)
  }
  run(doc)
  return () => {}
}
