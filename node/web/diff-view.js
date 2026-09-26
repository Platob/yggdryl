// A scenario's difference from the base replay, instant by instant, as the
// diff route served it: `[{ at, base, scenario, changes }]` - the two
// `stableHash`es and `diff.js`'s answer. A row shows its instant, both hashes
// and how many changes `diffBooks` found (or that a book is only in one
// stream); expanding it lists them keyed exactly as `diff.js` keyed them - a
// limit by its price text, an entry, a delta and an execution by `curruuid` -
// never by what is drawn. Choosing an instant emits `ygg:seek` `{ at }`.

import { Component, element } from './component.js'
import { formatInstant } from './instant.js'

const ABSENT = '—'
const UNPRICED = '∅'

function shown(value) {
  if (value === null || value === undefined) return ABSENT
  return typeof value === 'object' ? JSON.stringify(value) : String(value)
}

function keyedCount(keyed) {
  return keyed ? keyed.added.length + keyed.removed.length + keyed.changed.length : 0
}

/** Every change `diffBooks` answered, counted once. */
export function countChanges(changes) {
  if (!changes) return 0
  let count = changes.facts?.length ?? 0
  for (const side of [changes.bid, changes.ask]) {
    if (side) count += keyedCount(side.limits) + keyedCount(side.live) + keyedCount(side.deltas) + (side.facts?.length ?? 0)
  }
  return count + keyedCount(changes.executions) + keyedCount(changes.snapshotpartitions)
}

/** One line per change, in the order `diffBooks` answers them. */
export function changeLines(changes) {
  const lines = []
  const facts = (prefix, list) => {
    for (const fact of list ?? []) lines.push(`${prefix}${fact.name}: ${shown(fact.base)} → ${shown(fact.scenario)}`)
  }
  const factsOf = (list) => (list ?? []).map((fact) => `${fact.name} ${shown(fact.base)} → ${shown(fact.scenario)}`).join(', ')
  facts('fact ', changes?.facts)
  for (const name of ['bid', 'ask']) {
    const side = changes?.[name]
    if (!side) continue
    facts(`${name} fact `, side.facts)
    const limit = (held) => `${held.key ?? UNPRICED}: ${shown(held.quantity)} (${held.uuids?.length ?? 0} ${held.uuids?.length === 1 ? 'entry' : 'entries'})`
    for (const held of side.limits?.added ?? []) lines.push(`${name} limit added ${limit(held)}`)
    for (const held of side.limits?.removed ?? []) lines.push(`${name} limit removed ${limit(held)}`)
    for (const held of side.limits?.changed ?? []) lines.push(`${name} limit changed ${held.key ?? UNPRICED}: ${factsOf(held.facts)}`)
    for (const list of ['live', 'deltas']) {
      const label = list === 'live' ? 'live' : 'delta'
      for (const held of side[list]?.added ?? []) lines.push(`${name} ${label} added ${held.curruuid}`)
      for (const held of side[list]?.removed ?? []) lines.push(`${name} ${label} removed ${held.curruuid}`)
      for (const held of side[list]?.changed ?? []) lines.push(`${name} ${label} changed ${held.curruuid}: ${factsOf(held.facts)}`)
    }
  }
  for (const held of changes?.executions?.added ?? []) lines.push(`execution added ${held.curruuid}`)
  for (const held of changes?.executions?.removed ?? []) lines.push(`execution removed ${held.curruuid}`)
  for (const held of changes?.executions?.changed ?? []) lines.push(`execution changed ${held.curruuid}: ${factsOf(held.facts)}`)
  const scope = (held) => `${held.symbol ?? UNPRICED} ${held.scope}`
  for (const held of changes?.snapshotpartitions?.added ?? []) lines.push(`snapshot partition added ${scope(held)}`)
  for (const held of changes?.snapshotpartitions?.removed ?? []) lines.push(`snapshot partition removed ${scope(held)}`)
  return lines
}

let serial = 0

export class DiffView extends Component {
  constructor(props = {}) {
    super(props)
    this.base = `ygg-ui-diff-${++serial}`
    this.expanded = new Set()
    this.unchanged = false
  }

  render() {
    const root = element('section', 'ygg-ui__diff')
    root.setAttribute('aria-label', 'Scenario against the base replay')
    const bar = element('div', 'ygg-ui__diff-bar')
    this.summary = element('p', 'ygg-ui__diff-summary ygg-ui__muted')
    this.summary.setAttribute('aria-live', 'polite')
    const toggle = element('label', 'ygg-ui__diff-unchanged')
    this.toggle = element('input')
    this.toggle.type = 'checkbox'
    toggle.append(this.toggle, ' Show unchanged instants')
    bar.append(this.summary, toggle)
    this.table = element('table', 'ygg-ui__table ygg-ui__diff-table')
    const head = element('thead')
    const row = element('tr')
    for (const [name, numeric] of [['Instant', true], ['Base', true], ['Scenario', true], ['Changes', true], ['', false]]) {
      const cell = element('th', numeric ? 'ygg-ui__num' : '', name)
      cell.scope = 'col'
      row.append(cell)
    }
    head.append(row)
    this.body = element('tbody')
    this.table.append(element('caption', 'ygg-ui__sr-only', 'Instants, their two hashes and their changes'), head, this.body)
    root.append(bar, this.table)
    return root
  }

  onMount() {
    this.listen(this.toggle, 'change', () => {
      this.unchanged = this.toggle.checked
      this.schedule()
    })
    this.listen(this.body, 'click', (event) => {
      const seek = event.target.closest?.('.ygg-ui__diff-seek')
      if (seek) {
        this.emit('ygg:seek', { at: seek.dataset.at })
        return
      }
      const expand = event.target.closest?.('button[aria-expanded]')
      if (!expand) return
      const at = expand.dataset.at
      if (this.expanded.has(at)) this.expanded.delete(at)
      else this.expanded.add(at)
      this.schedule()
    })
  }

  draw(state) {
    const rows = state?.rows ?? []
    const differing = rows.filter((row) => !row.changes?.same)
    this.summary.textContent = rows.length ? `${differing.length} of ${rows.length} instants differ` : 'No instants to compare'
    const focused = this.body.contains(document.activeElement) ? document.activeElement.dataset?.key : undefined
    const nodes = []
    for (const row of this.unchanged ? rows : differing) nodes.push(...this.rowNodes(row))
    this.body.replaceChildren(...nodes)
    if (focused) this.body.querySelector(`[data-key="${CSS.escape(focused)}"]`)?.focus()
  }

  rowNodes(row) {
    const at = String(row.at)
    const instant = formatInstant(at)
    const detailId = `${this.base}-${at}`
    const tr = element('tr', 'ygg-ui__diff-row')
    if (!row.changes?.same) tr.classList.add('ygg-ui__diff-changed')
    const seek = element('button', 'ygg-ui__diff-seek ygg-ui__plain', instant)
    seek.type = 'button'
    seek.dataset.at = at
    seek.dataset.key = `seek:${at}`
    seek.setAttribute('aria-label', `Seek to ${instant}`)
    const count = row.base === null || row.base === undefined
      ? 'added'
      : row.scenario === null || row.scenario === undefined
        ? 'removed'
        : row.changes?.same
          ? 'same'
          : String(countChanges(row.changes))
    const open = this.expanded.has(at)
    const expand = element('button', 'ygg-ui__button ygg-ui__diff-expand', open ? 'Hide' : 'Show')
    expand.type = 'button'
    expand.dataset.at = at
    expand.dataset.key = `expand:${at}`
    expand.setAttribute('aria-expanded', String(open))
    expand.setAttribute('aria-controls', detailId)
    expand.setAttribute('aria-label', `${open ? 'Hide' : 'Show'} the changes at ${instant}`)
    const first = element('td', 'ygg-ui__num')
    first.append(seek)
    const last = element('td')
    last.append(expand)
    tr.append(
      first,
      element('td', 'ygg-ui__num ygg-ui__muted', shown(row.base)),
      element('td', 'ygg-ui__num ygg-ui__muted', shown(row.scenario)),
      element('td', `ygg-ui__num${row.changes?.same ? ' ygg-ui__muted' : ''}`, count),
      last,
    )
    const detail = element('tr', 'ygg-ui__diff-detail')
    detail.id = detailId
    detail.hidden = !open
    if (open) {
      const cell = element('td')
      cell.colSpan = 5
      const lines =
        row.base === null || row.base === undefined
          ? ['book only in the scenario']
          : row.scenario === null || row.scenario === undefined
            ? ['book only in the base replay']
            : changeLines(row.changes)
      const list = element('ul', 'ygg-ui__diff-lines ygg-ui__mono')
      for (const line of lines.length ? lines : ['no change']) list.append(element('li', undefined, line))
      cell.append(list)
      detail.append(cell)
    }
    return [tr, detail]
  }
}
