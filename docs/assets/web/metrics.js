// The readings of one served book, exactly as the service answered them: the
// row's `spread`, `crossed` and `locked`; the readings the service asked of
// the native book - the imbalance and each side's depth at 1, 5 and 10
// levels, `bboMidpoint`, `medianQuantity`, `isTick`; and the counts of the
// served executions and deltas. Nothing is derived here. Only the BBO line
// is a live region, so a replay does not read the whole list aloud on every
// step.

import { Component, element } from './component.js'
import { formatDecimal } from './decimal.js'

const LEVELS = ['1', '5', '10']
const ABSENT = '—'

function decimal(text) {
  return text === null || text === undefined ? ABSENT : formatDecimal(text)
}

function flag(value) {
  return value === null || value === undefined ? ABSENT : value ? 'yes' : 'no'
}

function levels(count) {
  return `${count} ${count === '1' ? 'level' : 'levels'}`
}

function best(side) {
  return side?.price === null || side?.price === undefined ? 'none' : `${decimal(side.price)} x ${decimal(side.quantity)}`
}

export class Metrics extends Component {
  render() {
    const root = element('section', 'ygg-ui__metrics')
    root.setAttribute('aria-label', 'Book readings')
    this.bbo = element('p', 'ygg-ui__metrics-bbo ygg-ui__num')
    this.bbo.setAttribute('aria-live', 'polite')
    this.list = element('dl', 'ygg-ui__metrics-list')
    root.append(this.bbo, this.list)
    return root
  }

  draw(state) {
    const book = state?.book
    const bbo = book ? `Bid ${best(book.bid)}, ask ${best(book.ask)}` : 'No book'
    if (this.bbo.textContent !== bbo) this.bbo.textContent = bbo
    if (!book) {
      this.list.replaceChildren()
      return
    }
    const rows = [
      ['Spread', decimal(book.spread)],
      ['Midpoint', decimal(book.bboMidpoint)],
      ['Median quantity', decimal(book.medianQuantity)],
      ['Crossed', flag(book.crossed)],
      ['Locked', flag(book.locked)],
      ...LEVELS.map((count) => [`Imbalance, ${levels(count)}`, decimal(book.imbalance?.[count])]),
      ...LEVELS.map((count) => [`Bid depth, ${levels(count)}`, decimal(book.bid?.depth?.[count])]),
      ...LEVELS.map((count) => [`Ask depth, ${levels(count)}`, decimal(book.ask?.depth?.[count])]),
      ['Executions', String(book.executions?.length ?? 0)],
      ['Bid deltas', String(book.bid?.deltas?.length ?? 0)],
      ['Ask deltas', String(book.ask?.deltas?.length ?? 0)],
      ['Grid tick', flag(book.isTick)],
    ]
    const nodes = []
    for (const [term, value] of rows) {
      const dt = element('dt', 'ygg-ui__muted', term)
      const dd = element('dd', `ygg-ui__num${value === ABSENT ? ' ygg-ui__muted' : ''}`, value)
      nodes.push(dt, dd)
    }
    this.list.replaceChildren(...nodes)
  }
}
