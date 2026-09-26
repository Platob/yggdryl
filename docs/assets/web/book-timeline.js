// The order book along its timeline: one component over the books a walk
// answered - `bookJson` each, in walk order. A scrubber moves through their
// instants; the limits standing at the chosen instant are drawn side by
// side, each bar as deep as its quantity and marked where it changed since
// the book before; `Audit` opens the whole book in a dialog sized to the
// viewport, every side, limit, live entry, delta and execution a collapsible
// item. Everything shown is what the package answered: nothing is folded,
// ordered or computed here but the text it is drawn as.

const NANOS_PER_MILLI = 1_000_000n
/** At most this many instant marks are drawn; a longer walk marks every n-th book. */
const MAX_MARKS = 400

/** An instant - decimal nanoseconds since the epoch, UTC - as ISO 8601 with every digit kept. */
export function instantText(nanos) {
  if (nanos === null || nanos === undefined) return '-'
  const value = BigInt(nanos)
  const millis = value / NANOS_PER_MILLI - (value % NANOS_PER_MILLI < 0n ? 1n : 0n)
  const fraction = String(value - millis * NANOS_PER_MILLI).padStart(6, '0')
  return new Date(Number(millis)).toISOString().replace('Z', `${fraction}Z`)
}

/** A limit's price as drawn: the unpriced limit, where market orders rest, is `market`. */
export function priceText(price) {
  return price === null || price === undefined ? 'market' : price
}

/** The key a limit is compared across books by: its price text. */
function limitKey(limit) {
  return priceText(limit.price)
}

/**
 * The limit keys of `side` whose quantity or live entries differ from the
 * same side of `previous`, or that `previous` did not hold.
 */
export function changedLimits(previous, side) {
  const before = new Map((previous?.limits ?? []).map((limit) => [limitKey(limit), limit]))
  const changed = new Set()
  for (const limit of side?.limits ?? []) {
    const held = before.get(limitKey(limit))
    if (!held || held.quantity !== limit.quantity || held.uuids.join() !== limit.uuids.join()) changed.add(limitKey(limit))
  }
  return changed
}

/** `count` and `noun`, the noun plural unless the count is one. */
function counted(count, noun) {
  return `${count} ${noun}${count === 1 ? '' : 's'}`
}

/** The live entries of `side` by their `curruuid`. */
export function liveByUuid(side) {
  return new Map((side?.live ?? []).map((entry) => [entry.curruuid, entry]))
}

function node(tag, className, text) {
  const created = document.createElement(tag)
  if (className) created.className = className
  if (text !== undefined) created.textContent = text
  return created
}

function button(className, text, label) {
  const created = node('button', className, text)
  created.type = 'button'
  if (label) created.setAttribute('aria-label', label)
  return created
}

/** A two-column table of `[name, value]` facts, a null value drawn as `-`. */
function factsTable(facts) {
  const table = node('table', 'ygg-bt__facts')
  for (const [name, value] of facts) {
    const row = table.insertRow()
    row.append(node('th', '', name), node('td', '', value === null || value === undefined ? '-' : String(value)))
  }
  return table
}

/** A table of `rows` under `columns`, each a `[heading, read]` pair. */
function rowsTable(columns, rows) {
  const table = node('table', 'ygg-bt__rows')
  const head = table.createTHead().insertRow()
  for (const [heading] of columns) head.append(node('th', '', heading))
  const body = table.createTBody()
  for (const row of rows) {
    const line = body.insertRow()
    for (const [, read] of columns) line.append(node('td', '', read(row) ?? '-'))
  }
  return table
}

/** A collapsible item: `summary` always shown, `content` built only when it first opens. */
function collapsible(summary, content, open = false) {
  const details = node('details', 'ygg-bt__item')
  details.append(node('summary', '', summary))
  let built = false
  const build = () => {
    if (built) return
    built = true
    details.append(content())
  }
  details.addEventListener('toggle', () => details.open && build())
  if (open) {
    details.open = true
    build()
  }
  return details
}

const ENTRY_COLUMNS = [
  ['kind', (entry) => entry.kind],
  ['crosscode', (entry) => entry.crosscode],
  ['side', (entry) => entry.side],
  ['price', (entry) => entry.price],
  ['quantity', (entry) => entry.quantity],
  ['at', (entry) => instantText(entry.currunix)],
  ['curruuid', (entry) => entry.curruuid],
]

export class BookTimeline {
  constructor({ books = [], index, title = '' } = {}) {
    this.books = books
    this.index = index ?? Math.max(0, books.length - 1)
    this.title = title
    this.el = null
    this.dialog = null
  }

  /** Attach under `parent`; answers this. */
  mount(parent) {
    if (this.el) throw new Error('BookTimeline is already mounted')
    const root = node('section', 'ygg-bt')
    const head = node('header', 'ygg-bt__head')
    this.symbolEl = node('strong', 'ygg-bt__symbol')
    this.instantEl = node('time', 'ygg-bt__instant')
    this.statsEl = node('span', 'ygg-bt__stats')
    this.auditButton = button('ygg-bt__button', 'Audit book')
    head.append(this.symbolEl, this.instantEl, this.statsEl, this.auditButton)

    const timeline = node('div', 'ygg-bt__timeline')
    this.previousButton = button('ygg-bt__button', '‹', 'Previous book')
    this.nextButton = button('ygg-bt__button', '›', 'Next book')
    this.scrubber = node('input', 'ygg-bt__scrubber')
    this.scrubber.type = 'range'
    this.scrubber.min = '0'
    this.scrubber.step = '1'
    this.scrubber.setAttribute('aria-label', 'Book instant')
    this.positionEl = node('output', 'ygg-bt__position')
    timeline.append(this.previousButton, this.scrubber, this.nextButton, this.positionEl)
    this.marksEl = node('div', 'ygg-bt__marks')
    this.marksEl.setAttribute('aria-hidden', 'true')

    const book = node('div', 'ygg-bt__book')
    this.bidEl = node('div', 'ygg-bt__side ygg-bt__side--bid')
    this.askEl = node('div', 'ygg-bt__side ygg-bt__side--ask')
    book.append(this.bidEl, this.askEl)

    this.dialog = node('dialog', 'ygg-bt__dialog')
    root.append(head, timeline, this.marksEl, book, this.dialog)

    this.scrubber.addEventListener('input', () => this.select(Number(this.scrubber.value)))
    this.previousButton.addEventListener('click', () => this.select(this.index - 1))
    this.nextButton.addEventListener('click', () => this.select(this.index + 1))
    this.auditButton.addEventListener('click', () => this.openAudit())
    this.marksEl.addEventListener('click', (event) => {
      const at = event.target?.dataset?.index
      if (at !== undefined) this.select(Number(at))
    })
    root.addEventListener('keydown', (event) => {
      if (event.target === this.scrubber || this.dialog.open) return
      if (event.key === 'ArrowLeft') this.select(this.index - 1)
      else if (event.key === 'ArrowRight') this.select(this.index + 1)
      else if (event.key === 'Home') this.select(0)
      else if (event.key === 'End') this.select(this.books.length - 1)
      else return
      event.preventDefault()
    })
    parent.append(root)
    this.el = root
    this.draw()
    return this
  }

  /** New books and, optionally, the index to stand at (the last book by default). */
  update({ books = this.books, index, title = this.title } = {}) {
    const fresh = books !== this.books
    this.books = books
    this.title = title
    this.index = index ?? (fresh ? Math.max(0, books.length - 1) : this.index)
    if (this.el) this.draw()
    return this
  }

  /** The book standing now, or `null` for an empty walk. */
  get book() {
    return this.books[this.index] ?? null
  }

  /** Stand at book `index`, clamped to the walk; emits `ygg:book-select` when it moves. */
  select(index) {
    const next = Math.min(Math.max(0, index), Math.max(0, this.books.length - 1))
    if (next === this.index && this.el?.dataset.index === String(next)) return
    this.index = next
    this.draw()
    const book = this.book
    this.el?.dispatchEvent(
      new CustomEvent('ygg:book-select', {
        bubbles: true,
        detail: { index: next, curruuid: book?.curruuid ?? null, currunix: book?.currunix ?? null },
      }),
    )
  }

  draw() {
    const book = this.book
    const count = this.books.length
    this.el.dataset.index = String(this.index)
    this.el.setAttribute('aria-label', `Order book timeline${book ? `, ${book.crosscode}` : ''}`)
    this.symbolEl.textContent = book?.crosscode ?? (this.title || 'No book')
    this.instantEl.textContent = book ? instantText(book.currunix) : ''
    if (book) this.instantEl.dateTime = instantText(book.currunix)
    this.statsEl.replaceChildren(...this.stats(book))
    this.scrubber.max = String(Math.max(0, count - 1))
    this.scrubber.value = String(this.index)
    this.scrubber.disabled = count < 2
    this.scrubber.setAttribute('aria-valuetext', book ? instantText(book.currunix) : 'no book')
    this.positionEl.textContent = count ? `${this.index + 1} / ${count}` : '0 / 0'
    this.previousButton.disabled = this.index <= 0
    this.nextButton.disabled = this.index >= count - 1
    this.auditButton.disabled = !book
    this.drawMarks()
    const previous = this.books[this.index - 1] ?? null
    const deepest = Math.max(1, ...[book?.bid, book?.ask].flatMap((side) => (side?.limits ?? []).map((limit) => Number(limit.quantity))))
    this.drawSide(this.bidEl, 'Bid', book?.bid, previous?.bid, deepest)
    this.drawSide(this.askEl, 'Ask', book?.ask, previous?.ask, deepest)
  }

  /** The book's headline facts: spread, midpoint, a crossed or locked badge. */
  stats(book) {
    if (!book) return [node('span', '', 'The walk answered no book.')]
    const parts = [node('span', '', `spread ${book.spread ?? '-'}`), node('span', '', `mid ${book.bboMidpoint ?? '-'}`)]
    if (book.crossed) parts.push(node('span', 'ygg-bt__badge ygg-bt__badge--alert', 'crossed'))
    else if (book.locked) parts.push(node('span', 'ygg-bt__badge ygg-bt__badge--alert', 'locked'))
    if (book.executions?.length) parts.push(node('span', 'ygg-bt__badge', `${book.executions.length} executed`))
    return parts
  }

  /** One mark per book at its instant's place on the walk's span; a click stands there. */
  drawMarks() {
    const count = this.books.length
    const step = Math.max(1, Math.ceil(count / MAX_MARKS))
    const first = count ? BigInt(this.books[0].currunix) : 0n
    const span = count ? BigInt(this.books[count - 1].currunix) - first : 0n
    const marks = []
    for (let at = 0; at < count; at += step) {
      const mark = node('span', 'ygg-bt__mark')
      const offset = span > 0n ? Number(((BigInt(this.books[at].currunix) - first) * 10_000n) / span) / 100 : 0
      mark.style.left = `${offset}%`
      mark.dataset.index = String(at)
      mark.title = instantText(this.books[at].currunix)
      if (at <= this.index && this.index < at + step) mark.classList.add('ygg-bt__mark--current')
      marks.push(mark)
    }
    this.marksEl.replaceChildren(...marks)
  }

  /** A side's live limits, best first as served, each bar as deep as its quantity. */
  drawSide(host, label, side, previous, deepest) {
    const limits = side?.limits ?? []
    const changed = changedLimits(previous, side)
    const caption = node('div', 'ygg-bt__caption', `${label} · ${counted(limits.length, 'limit')}`)
    const columns = node('div', 'ygg-bt__columns')
    columns.setAttribute('aria-hidden', 'true')
    columns.append(node('span', '', 'price'), node('span', '', 'quantity'), node('span', '', 'live'))
    const list = node('ol', 'ygg-bt__limits')
    list.setAttribute('aria-label', `${label} limits`)
    for (const limit of limits) {
      const row = node('li', 'ygg-bt__limit')
      if (changed.has(limitKey(limit))) row.classList.add('ygg-bt__limit--changed')
      if (limit.price === null) row.classList.add('ygg-bt__limit--market')
      const bar = node('span', 'ygg-bt__bar')
      bar.style.width = `${Math.min(100, (Number(limit.quantity) / deepest) * 100)}%`
      row.append(
        bar,
        node('span', 'ygg-bt__price', priceText(limit.price)),
        node('span', 'ygg-bt__quantity', limit.quantity),
        node('span', 'ygg-bt__orders', `${limit.uuids.length}`),
      )
      row.title = `${priceText(limit.price)} × ${limit.quantity}, ${limit.uuids.length} live`
      list.append(row)
    }
    if (!limits.length) list.append(node('li', 'ygg-bt__empty', 'no limit'))
    host.replaceChildren(caption, columns, list)
  }

  /** The whole book standing now, in a dialog: each side, limit and list a collapsible item. */
  openAudit() {
    const book = this.book
    if (!book) return
    const header = node('header', 'ygg-bt__dialog-head')
    const title = node('h2', '', `${book.crosscode} at ${instantText(book.currunix)}`)
    const expand = button('ygg-bt__button', 'Expand all')
    const collapse = button('ygg-bt__button', 'Collapse all')
    const close = button('ygg-bt__button', '×', 'Close')
    header.append(title, expand, collapse, close)
    const body = node('div', 'ygg-bt__dialog-body')
    body.append(
      collapsible('Book', () =>
        factsTable([
          ['currunix', instantText(book.currunix)],
          ['snapunix', book.snapunix === null ? null : instantText(book.snapunix)],
          ['curruuid', book.curruuid],
          ['state', book.state],
          ['spread', book.spread],
          ['midpoint', book.bboMidpoint],
          ['median quantity', book.medianQuantity],
          ['imbalance 1 / 5 / 10', ['1', '5', '10'].map((depth) => book.imbalance?.[depth] ?? '-').join(' / ')],
          ['crossed', book.crossed],
          ['locked', book.locked],
          ['stableHash', book.stableHash],
        ]), true),
      this.auditSide('Bid', book.bid),
      this.auditSide('Ask', book.ask),
      collapsible(`Executions · ${book.executions?.length ?? 0}`, () => rowsTable(ENTRY_COLUMNS, book.executions ?? [])),
      collapsible(`Deltas · ${(book.bid?.deltas?.length ?? 0) + (book.ask?.deltas?.length ?? 0)}`, () =>
        rowsTable(ENTRY_COLUMNS, [...(book.bid?.deltas ?? []), ...(book.ask?.deltas ?? [])]),
      ),
    )
    const items = () => [...body.querySelectorAll('details')]
    expand.addEventListener('click', () => {
      // Opening builds a nested item's content, which holds more items: open until none is closed.
      for (let closed = items().filter((item) => !item.open); closed.length; closed = items().filter((item) => !item.open)) {
        for (const item of closed) {
          item.open = true
          item.dispatchEvent(new Event('toggle'))
        }
      }
    })
    collapse.addEventListener('click', () => {
      for (const item of items()) item.open = false
    })
    close.addEventListener('click', () => this.dialog.close())
    this.dialog.replaceChildren(header, body)
    this.dialog.setAttribute('aria-label', `Audit of the ${book.crosscode} book`)
    if (!this.dialog.open) this.dialog.showModal()
  }

  /** One side in the audit: its limits, each holding the live entries resting at it. */
  auditSide(label, side) {
    const limits = side?.limits ?? []
    const live = liveByUuid(side)
    return collapsible(
      `${label} · ${counted(limits.length, 'limit')}`,
      () => {
        const list = node('div', 'ygg-bt__audit-limits')
        for (const limit of limits) {
          list.append(
            collapsible(`${priceText(limit.price)} × ${limit.quantity} · ${limit.uuids.length} live`, () =>
              rowsTable(ENTRY_COLUMNS, limit.uuids.map((uuid) => live.get(uuid) ?? { curruuid: uuid })),
            ),
          )
        }
        if (!limits.length) list.append(node('p', 'ygg-bt__empty', 'no limit'))
        return list
      },
      true,
    )
  }

  /** Detach; the component can be mounted again. */
  destroy() {
    if (this.dialog?.open) this.dialog.close()
    this.el?.remove()
    this.el = null
    this.dialog = null
  }
}
