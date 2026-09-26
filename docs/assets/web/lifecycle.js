// One element's chain as the lifecycle view served it: its statements in the
// served order (the view orders by `currunix`), each with its state, kind and
// instant, and `prevuuid` as a link to the statement it follows. The chosen
// statement's `metadata` and `securityids` - each served as `[key, value]`
// pairs in the map's own order - are key-value tables in that order: never
// re-keyed, never sorted here.

import { Component, element } from './component.js'
import { formatInstant } from './instant.js'

const NONE = '—'

export class Lifecycle extends Component {
  constructor(props = {}) {
    super(props)
    this.selected = null
    this.highlight = null
    this.rowsById = new Map()
    this.drawn = undefined
  }

  render() {
    const root = element('section', 'ygg-ui__lifecycle')
    root.setAttribute('aria-label', 'Lifecycle')
    this.heading = element('h3', 'ygg-ui__lifecycle-title')
    this.table = element('table', 'ygg-ui__table ygg-ui__lifecycle-chain')
    const caption = element('caption', 'ygg-ui__sr-only', 'Statements, oldest first')
    const head = element('thead')
    const headRow = element('tr')
    for (const [name, numeric] of [['#', true], ['State', false], ['Kind', false], ['Instant', true], ['Id', true], ['Previous', true]]) {
      const cell = element('th', numeric ? 'ygg-ui__num' : '', name)
      cell.scope = 'col'
      headRow.append(cell)
    }
    head.append(headRow)
    this.body = element('tbody')
    this.table.append(caption, head, this.body)
    this.empty = element('p', 'ygg-ui__lifecycle-empty ygg-ui__muted')
    this.empty.hidden = true
    this.details = element('div', 'ygg-ui__lifecycle-details')
    this.metadata = keyValues('metadata', 'Metadata')
    this.securityids = keyValues('securityids', 'Security ids')
    this.details.append(this.metadata.section, this.securityids.section)
    root.append(this.heading, this.table, this.empty, this.details)
    return root
  }

  onMount() {
    this.listen(this.body, 'click', (event) => {
      const link = event.target.closest?.('.ygg-ui__link')
      if (link) {
        this.follow(link.dataset.target)
        return
      }
      const select = event.target.closest?.('.ygg-ui__lifecycle-select')
      if (select) this.select(select.dataset.id)
    })
  }

  /** Highlight the statement `id` names and move focus to it. */
  follow(id) {
    const row = this.rowsById.get(id)
    if (!row) return
    this.highlight = id
    for (const [key, held] of this.rowsById) held.classList.toggle('ygg-ui__highlight', key === id)
    row.scrollIntoView({ block: 'nearest' })
    row.querySelector('.ygg-ui__lifecycle-select')?.focus()
  }

  select(id) {
    const row = (this.state?.rows ?? []).find((held) => held.curruuid === id)
    if (!row) return
    this.selected = id
    this.emit('ygg:select-element', { crosscode: row.crosscode ?? this.state.crosscode, curruuid: row.curruuid })
    this.schedule()
  }

  draw(state) {
    const rows = state?.rows ?? []
    if (state !== this.drawn) {
      if (state?.crosscode !== this.drawn?.crosscode) {
        this.selected = null
        this.highlight = null
      }
      this.drawn = state
      this.heading.textContent = state?.crosscode ?? ''
      this.el.setAttribute('aria-label', `Lifecycle of ${state?.crosscode ?? 'nothing'}`)
      this.rebuild(rows)
      this.table.hidden = !rows.length
      this.empty.hidden = rows.length > 0
      this.empty.textContent = rows.length ? '' : `No statements for ${state?.crosscode ?? 'this element'}`
    }
    const chosen = rows.find((row) => row.curruuid === this.selected) ?? rows.at(-1)
    for (const [id, tr] of this.rowsById) {
      tr.setAttribute('aria-selected', String(id === chosen?.curruuid))
      tr.classList.toggle('ygg-ui__selected', id === chosen?.curruuid)
      tr.classList.toggle('ygg-ui__highlight', id === this.highlight)
    }
    this.details.hidden = !chosen
    this.metadata.fill(chosen?.metadata)
    this.securityids.fill(chosen?.securityids)
  }

  rebuild(rows) {
    const ids = new Set(rows.map((row) => row.curruuid))
    this.rowsById = new Map()
    const nodes = rows.map((row, index) => {
      const tr = element('tr')
      const instant = formatInstant(row.currunix)
      const select = element('button', 'ygg-ui__lifecycle-select ygg-ui__plain ygg-ui__num', short(row.curruuid))
      select.type = 'button'
      select.dataset.id = row.curruuid
      select.title = row.curruuid
      select.setAttribute('aria-label', `Select ${row.state ?? 'statement'} at ${instant}, ${row.curruuid}`)
      const previous = element('td', 'ygg-ui__num')
      if (row.prevuuid && ids.has(row.prevuuid)) {
        const link = element('button', 'ygg-ui__link', short(row.prevuuid))
        link.type = 'button'
        link.dataset.target = row.prevuuid
        link.title = row.prevuuid
        link.setAttribute('aria-label', `Go to the statement this follows, ${row.prevuuid}`)
        previous.append(link)
      } else {
        previous.textContent = row.prevuuid ? short(row.prevuuid) : NONE
        if (row.prevuuid) previous.title = row.prevuuid
      }
      const idCell = element('td', 'ygg-ui__num')
      idCell.append(select)
      tr.append(
        element('td', 'ygg-ui__num ygg-ui__muted', String(index + 1)),
        element('td', undefined, row.state ?? ''),
        element('td', 'ygg-ui__muted', row.kind ?? ''),
        element('td', 'ygg-ui__num', instant),
        idCell,
        previous,
      )
      this.rowsById.set(row.curruuid, tr)
      return tr
    })
    this.body.replaceChildren(...nodes)
  }
}

/** A titled two-column table of one served map - `[key, value]` pairs - in their order. */
function keyValues(name, title) {
  const section = element('section', 'ygg-ui__lifecycle-map')
  const table = element('table', 'ygg-ui__table')
  table.dataset.table = name
  const caption = element('caption', 'ygg-ui__lifecycle-caption', title)
  const body = element('tbody')
  const empty = element('p', 'ygg-ui__muted', `No ${title.toLowerCase()}`)
  table.append(caption, body)
  section.append(table, empty)
  return {
    section,
    fill(pairs) {
      const entries = Array.isArray(pairs) ? pairs : []
      body.replaceChildren(
        ...entries.map(([key, value]) => {
          const tr = element('tr')
          const th = element('th', 'ygg-ui__muted', key)
          th.scope = 'row'
          const td = element('td', 'ygg-ui__mono', value === null || value === undefined ? '' : String(value))
          tr.append(th, td)
          return tr
        }),
      )
      empty.hidden = entries.length > 0
    },
  }
}

function short(uuid) {
  return String(uuid).slice(0, 8)
}
