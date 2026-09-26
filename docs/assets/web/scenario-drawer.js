// The scenarios drawer: the saved scenarios, the active one's inserted events
// in the order it holds them - each the leaf's served row, every fact it
// states listed in column order - and the changes a person can ask for - create,
// select, rename, delete (asked twice), remove an event. Each is emitted as
// `ygg:scenario` `{ command, name, curruuid }` (a rename adds `to`); the app
// carries it out against the service and updates the drawer with what it
// answers. A new name already taken is refused here, before anything is sent.

import { element } from './component.js'
import { Drawer } from './drawer.js'
import { formatInstant } from './instant.js'

/** What an event row shows apart from its facts: the kind and instant lead, the native text never shows. */
const APART = new Set(['kind', 'currunix', 'native'])

/** Whether a served cell states anything: not null, not an empty text, list or map. */
function states(value) {
  return value !== null && value !== undefined && value !== '' && !(Array.isArray(value) && !value.length)
}

/** A served cell as text: a map's `[key, value]` pairs as `key=value`, a list joined. */
function cellText(value) {
  if (Array.isArray(value)) {
    return value.map((item) => (Array.isArray(item) && item.length === 2 ? `${item[0]}=${cellText(item[1])}` : cellText(item))).join(', ')
  }
  return value !== null && typeof value === 'object' ? JSON.stringify(value) : String(value)
}

export class ScenarioDrawer extends Drawer {
  constructor({ side = 'right', title = 'Scenarios' } = {}) {
    super({ side, title })
    this.renaming = null
    this.confirming = null
    this.drawn = undefined
  }

  render() {
    const panel = super.render()
    panel.classList.add('ygg-ui__scenarios')
    this.creator = element('form', 'ygg-ui__scenario-new')
    this.creator.noValidate = true
    const label = element('label', 'ygg-ui__insert-label', 'New scenario')
    this.nameInput = element('input')
    this.nameInput.type = 'text'
    this.nameInput.autocomplete = 'off'
    this.nameInput.id = `${panel.id}-new`
    label.htmlFor = this.nameInput.id
    this.nameError = element('span', 'ygg-ui__error ygg-ui__field-error')
    this.nameError.id = `${panel.id}-new-error`
    this.nameInput.setAttribute('aria-describedby', this.nameError.id)
    this.nameInput.setAttribute('aria-invalid', 'false')
    const create = element('button', 'ygg-ui__button', 'Create')
    create.type = 'submit'
    this.creator.append(label, this.nameInput, create, this.nameError)
    this.list = element('ul', 'ygg-ui__scenario-list')
    this.list.setAttribute('aria-label', 'Saved scenarios')
    this.eventsTitle = element('h3', 'ygg-ui__scenario-title')
    this.events = element('ol', 'ygg-ui__scenario-events')
    this.body.append(this.creator, this.list, this.eventsTitle, this.events)
    return panel
  }

  onMount() {
    super.onMount()
    this.listen(this.creator, 'submit', (event) => {
      event.preventDefault()
      const name = this.nameInput.value.trim()
      const refusal = this.refuseName(name)
      this.nameInput.setAttribute('aria-invalid', String(Boolean(refusal)))
      this.nameError.textContent = refusal ?? ''
      if (refusal) return
      this.nameInput.value = ''
      this.command({ command: 'create', name })
    })
    this.listen(this.body, 'click', (event) => {
      const button = event.target.closest?.('button[data-action]')
      if (button) this.act(button.dataset.action, button.dataset.name, button.dataset.curruuid)
    })
    this.listen(this.body, 'submit', (event) => {
      const form = event.target.closest?.('.ygg-ui__scenario-rename')
      if (!form) return
      event.preventDefault()
      const from = form.dataset.name
      const input = form.querySelector('input')
      const to = input.value.trim()
      const refusal = to === from ? null : this.refuseName(to)
      if (refusal) {
        input.setAttribute('aria-invalid', 'true')
        form.querySelector('.ygg-ui__error').textContent = refusal
        return
      }
      this.renaming = null
      if (to !== from) this.command({ command: 'rename', name: from, to })
      this.schedule()
    })
  }

  refuseName(name) {
    if (!name) return 'a scenario needs a name'
    if ((this.state?.scenarios ?? []).some((scenario) => scenario.name === name)) return `a scenario named ${name} exists`
    return null
  }

  act(action, name, curruuid) {
    if (action === 'select') this.command({ command: 'select', name })
    else if (action === 'rename') {
      this.renaming = name
      this.confirming = null
      this.schedule()
      return
    } else if (action === 'cancel-rename') {
      this.renaming = null
    } else if (action === 'delete') {
      // A delete is asked twice: the first press turns the button into its confirmation.
      if (this.confirming !== name) {
        this.confirming = name
        this.schedule()
        return
      }
      this.confirming = null
      this.command({ command: 'delete', name })
    } else if (action === 'remove-event') this.command({ command: 'remove-event', name, curruuid })
    this.schedule()
  }

  command(detail) {
    this.emit('ygg:scenario', detail)
  }

  draw(state) {
    const scenarios = state?.scenarios ?? []
    const active = scenarios.find((scenario) => scenario.name === state?.active) ?? null
    // A redraw keeps focus on the control it was on, found again by its key.
    const focused = this.body.contains(document.activeElement) ? document.activeElement.dataset?.key : undefined
    this.list.replaceChildren(...scenarios.map((scenario) => this.item(scenario, scenario === active)))
    this.eventsTitle.textContent = active ? `Events in ${active.name}` : 'No scenario selected'
    this.events.replaceChildren(...(active?.events ?? []).map((event) => this.eventItem(active.name, event)))
    if (active && !active.events.length) this.events.append(element('li', 'ygg-ui__muted', 'No inserted events'))
    const renaming = this.list.querySelector('.ygg-ui__scenario-rename input')
    if (renaming && this.renaming !== this.drawnRenaming) {
      renaming.focus()
      renaming.select()
    } else if (focused) this.body.querySelector(`[data-key="${CSS.escape(focused)}"]`)?.focus()
    this.drawnRenaming = this.renaming
  }

  item(scenario, active) {
    const li = element('li', 'ygg-ui__scenario-item')
    const count = scenario.events?.length ?? 0
    if (this.renaming === scenario.name) {
      const form = element('form', 'ygg-ui__scenario-rename')
      form.dataset.name = scenario.name
      const input = element('input')
      input.type = 'text'
      input.value = scenario.name
      input.setAttribute('aria-label', `New name for ${scenario.name}`)
      const save = element('button', 'ygg-ui__button', 'Save')
      save.type = 'submit'
      const cancel = button('Cancel', 'cancel-rename', scenario.name, `Cancel renaming ${scenario.name}`)
      form.append(input, save, cancel, element('span', 'ygg-ui__error ygg-ui__field-error'))
      li.append(form)
      return li
    }
    const name = button(`${scenario.name} (${count} ${count === 1 ? 'event' : 'events'})`, 'select', scenario.name)
    name.className = 'ygg-ui__scenario-name'
    name.setAttribute('aria-current', String(active))
    const rename = button('Rename', 'rename', scenario.name, `Rename ${scenario.name}`)
    const confirming = this.confirming === scenario.name
    const remove = button(confirming ? 'Confirm delete' : 'Delete', 'delete', scenario.name, confirming ? `Confirm deleting ${scenario.name}` : `Delete ${scenario.name}`)
    if (confirming) remove.classList.add('ygg-ui__button--danger')
    li.append(name, rename, remove)
    return li
  }

  eventItem(name, event) {
    const li = element('li', 'ygg-ui__scenario-event')
    const details = element('details', 'ygg-ui__scenario-facts')
    details.append(element('summary', 'ygg-ui__muted', 'Facts'))
    const list = element('dl', 'ygg-ui__metrics-list')
    for (const [column, value] of Object.entries(event)) {
      if (APART.has(column) || !states(value)) continue
      list.append(element('dt', 'ygg-ui__muted ygg-ui__mono', column), element('dd', 'ygg-ui__mono', cellText(value)))
    }
    details.append(list)
    li.append(
      element('span', 'ygg-ui__scenario-when ygg-ui__mono', formatInstant(event.currunix)),
      ' ',
      element('span', 'ygg-ui__scenario-kind', event.kind ?? ''),
      button('×', 'remove-event', name, `Remove the ${event.kind ?? 'event'} at ${formatInstant(event.currunix)}`, event.curruuid),
      details,
    )
    return li
  }
}

function button(text, action, name, label, curruuid) {
  const node = element('button', 'ygg-ui__button', text)
  node.type = 'button'
  node.dataset.action = action
  node.dataset.name = name
  if (curruuid) node.dataset.curruuid = curruuid
  node.dataset.key = `${action}:${name}:${curruuid ?? ''}`
  if (label) node.setAttribute('aria-label', label)
  return node
}
