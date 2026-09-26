// WAI-ARIA tabs: one `role="tablist"` of `role="tab"` buttons, each
// `aria-controls` its `role="tabpanel"`. Focus roves - only the selected tab
// is in the Tab order - the arrows move and wrap, Home and End go to the
// ends, and a tab is selected as it is focused. Choosing a tab emits `ygg:tab`
// `{ id }`; the app choosing one through `update` emits nothing.

import { Component, element } from './component.js'

let serial = 0

function checked(tabs) {
  const seen = new Set()
  for (const tab of tabs) {
    if (typeof tab?.id !== 'string' || !tab.id) throw new TypeError('expected every tab to have an id')
    if (seen.has(tab.id)) throw new TypeError(`tab id ${JSON.stringify(tab.id)} is listed twice`)
    seen.add(tab.id)
  }
  return tabs
}

export class Tabs extends Component {
  constructor({ tabs = [], selected, label = 'Tabs' } = {}) {
    super({ tabs: checked(tabs), label })
    this.base = `ygg-ui-tabs-${++serial}`
    this.tabs = tabs
    this.selected = selected ?? tabs[0]?.id ?? null
    this.buttons = new Map()
    this.panels = new Map()
  }

  render() {
    const root = element('div', 'ygg-ui__tabs')
    this.list = element('div', 'ygg-ui__tablist')
    this.list.setAttribute('role', 'tablist')
    this.list.setAttribute('aria-label', this.props.label)
    this.stack = element('div', 'ygg-ui__tabpanels')
    root.append(this.list, this.stack)
    this.build(this.tabs)
    this.apply()
    return root
  }

  onMount() {
    this.listen(this.list, 'click', (event) => {
      const tab = event.target.closest?.('[role="tab"]')
      if (tab) this.choose(tab.dataset.id, false)
    })
    this.listen(this.list, 'keydown', (event) => {
      const ids = this.tabs.map((tab) => tab.id)
      const at = ids.indexOf(this.selected)
      let next
      if (event.key === 'ArrowRight') next = ids[(at + 1) % ids.length]
      else if (event.key === 'ArrowLeft') next = ids[(at - 1 + ids.length) % ids.length]
      else if (event.key === 'Home') next = ids[0]
      else if (event.key === 'End') next = ids.at(-1)
      else return
      event.preventDefault()
      event.stopPropagation()
      this.choose(next, true)
    })
  }

  /** The panel element of tab `id`, where its content goes. */
  panel(id) {
    return this.panels.get(id) ?? null
  }

  /** Select `id` as the person did: focus it if asked, and emit. */
  choose(id, focus) {
    if (!this.buttons.has(id)) return
    const changed = id !== this.selected
    this.selected = id
    this.apply()
    if (focus) this.buttons.get(id).focus()
    if (changed) this.emit('ygg:tab', { id })
  }

  build(tabs) {
    const buttons = new Map()
    const panels = new Map()
    for (const { id, label } of tabs) {
      const button = this.buttons.get(id) ?? element('button', 'ygg-ui__tab')
      button.type = 'button'
      button.setAttribute('role', 'tab')
      button.id = `${this.base}-tab-${id}`
      button.dataset.id = id
      button.textContent = label ?? id
      const panel = this.panels.get(id) ?? element('div', 'ygg-ui__tabpanel')
      panel.id = `${this.base}-panel-${id}`
      panel.setAttribute('role', 'tabpanel')
      panel.setAttribute('aria-labelledby', button.id)
      panel.tabIndex = 0
      button.setAttribute('aria-controls', panel.id)
      buttons.set(id, button)
      panels.set(id, panel)
    }
    this.buttons = buttons
    this.panels = panels
    this.list.replaceChildren(...buttons.values())
    this.stack.replaceChildren(...panels.values())
  }

  apply() {
    if (!this.buttons.has(this.selected)) this.selected = this.tabs[0]?.id ?? null
    for (const [id, button] of this.buttons) {
      const selected = id === this.selected
      button.setAttribute('aria-selected', String(selected))
      button.tabIndex = selected ? 0 : -1
      this.panels.get(id).hidden = !selected
    }
  }

  draw(state) {
    if (state?.tabs && state.tabs !== this.tabs) {
      this.tabs = checked(state.tabs)
      this.build(this.tabs)
    }
    if (state?.selected !== undefined) this.selected = state.selected
    this.apply()
  }
}
