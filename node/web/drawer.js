// A non-modal side panel: the page beside it stays live. Its toggles carry
// `aria-expanded` and `aria-controls`; opening moves focus into the panel,
// Escape or the close button closes it and gives focus back to the toggle
// that opened it. Emits `ygg:open` and `ygg:close`.

import { Component, element } from './component.js'

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

let serial = 0

export class Drawer extends Component {
  constructor({ side = 'right', title = '' } = {}) {
    if (side !== 'right' && side !== 'left') throw new TypeError(`expected side 'right' or 'left', got ${JSON.stringify(side)}`)
    super({ side, title })
    this.toggles = new Set()
    this.opener = null
  }

  render() {
    const id = `ygg-ui-drawer-${++serial}`
    const panel = element('aside', `ygg-ui__drawer ygg-ui__drawer--${this.props.side}`)
    panel.id = id
    panel.hidden = true
    panel.tabIndex = -1
    panel.setAttribute('aria-labelledby', `${id}-title`)
    const header = element('header', 'ygg-ui__drawer-header')
    this.title = element('h2', 'ygg-ui__drawer-title', this.props.title)
    this.title.id = `${id}-title`
    this.closer = element('button', 'ygg-ui__button', '×')
    this.closer.type = 'button'
    this.closer.setAttribute('aria-label', 'Close')
    header.append(this.title, this.closer)
    this.body = element('div', 'ygg-ui__drawer-body')
    panel.append(header, this.body)
    return panel
  }

  onMount() {
    this.listen(this.closer, 'click', () => this.close())
    this.listen(this.el, 'keydown', (event) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      this.close()
    })
  }

  /** Make `toggle` open and close this drawer; it reports the state in `aria-expanded`. */
  bind(toggle) {
    toggle.setAttribute('aria-controls', this.el.id)
    toggle.setAttribute('aria-expanded', String(this.isOpen))
    this.toggles.add(toggle)
    this.listen(toggle, 'click', () => this.toggle(toggle))
    return this
  }

  get isOpen() {
    return Boolean(this.el && !this.el.hidden)
  }

  toggle(opener) {
    if (this.isOpen) this.close()
    else this.open(opener)
  }

  open(opener = document.activeElement) {
    if (!this.el || this.isOpen) return
    this.opener = opener instanceof HTMLElement ? opener : null
    this.el.hidden = false
    this.expanded(true)
    const first = [...this.body.querySelectorAll(FOCUSABLE)].find((node) => node.getClientRects().length)
    ;(first ?? this.el).focus()
    this.emit('ygg:open', {})
  }

  close() {
    if (!this.isOpen) return
    const hadFocus = this.el.contains(document.activeElement)
    this.el.hidden = true
    this.expanded(false)
    const opener = this.opener
    this.opener = null
    if (hadFocus && opener?.isConnected) opener.focus()
    this.emit('ygg:close', {})
  }

  expanded(value) {
    for (const toggle of this.toggles) toggle.setAttribute('aria-expanded', String(value))
  }
}
