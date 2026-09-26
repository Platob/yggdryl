// A modal panel over the native `<dialog>`: opened with `showModal()`, named
// by its title, closed by Escape (the browser's own cancel), a backdrop click
// or its close button. While it is open the rest of the document is `inert`
// and Tab cycles inside it; on close the opener has focus again and
// `ygg:close` is emitted once - destroying an open modal included, which
// undoes all of it at once, since `destroy` forgets the `close` listener first.

import { Component, element } from './component.js'

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

let serial = 0

export class Modal extends Component {
  constructor({ title = '' } = {}) {
    super({ title })
    this.opener = null
    this.inerted = []
    this.opened = false
  }

  render() {
    const id = `ygg-ui-modal-${++serial}`
    const dialog = element('dialog', 'ygg-ui__modal')
    dialog.setAttribute('aria-labelledby', `${id}-title`)
    const header = element('header', 'ygg-ui__modal-header')
    this.title = element('h2', 'ygg-ui__modal-title', this.props.title)
    this.title.id = `${id}-title`
    this.closer = element('button', 'ygg-ui__button ygg-ui__modal-close', '×')
    this.closer.type = 'button'
    this.closer.setAttribute('aria-label', 'Close')
    header.append(this.title, this.closer)
    this.body = element('div', 'ygg-ui__modal-body')
    dialog.append(header, this.body)
    return dialog
  }

  onMount() {
    this.listen(this.closer, 'click', () => this.close())
    this.listen(this.el, 'close', () => this.restore())
    // A click on the backdrop lands on the dialog itself, outside its box.
    this.listen(this.el, 'click', (event) => {
      if (event.target !== this.el) return
      const box = this.el.getBoundingClientRect()
      const inside = event.clientX >= box.left && event.clientX <= box.right && event.clientY >= box.top && event.clientY <= box.bottom
      if (!inside) this.close()
    })
    this.listen(this.el, 'keydown', (event) => this.trap(event))
  }

  /** Open over the page; `opener` has focus again when it closes. */
  open(opener = document.activeElement) {
    if (!this.el || this.el.open) return
    this.opener = opener instanceof HTMLElement ? opener : null
    this.opened = true
    this.inertOutside()
    this.el.showModal()
  }

  close() {
    if (this.el?.open) this.el.close()
  }

  get isOpen() {
    return Boolean(this.el?.open)
  }

  /** Every element beside the dialog's ancestors becomes inert until it closes. */
  inertOutside() {
    for (let node = this.el; node && node !== document.body && node.parentElement; node = node.parentElement) {
      for (const sibling of node.parentElement.children) {
        if (sibling === node || sibling.inert || sibling.tagName === 'SCRIPT' || sibling.tagName === 'STYLE') continue
        sibling.inert = true
        this.inerted.push(sibling)
      }
    }
  }

  /** Undo what `open` did, once per open: the page, the opener's focus, `ygg:close`. */
  restore() {
    if (!this.opened) return
    this.opened = false
    for (const node of this.inerted.splice(0)) node.inert = false
    const opener = this.opener
    this.opener = null
    if (opener?.isConnected) opener.focus()
    this.emit('ygg:close', {})
  }

  /** Tab and Shift+Tab wrap at the dialog's edges. */
  trap(event) {
    if (event.key !== 'Tab') return
    const controls = [...this.el.querySelectorAll(FOCUSABLE)].filter((node) => !node.closest('[hidden], [inert]') && node.getClientRects().length)
    if (!controls.length) return
    const first = controls[0]
    const last = controls.at(-1)
    if (event.shiftKey && (document.activeElement === first || !this.el.contains(document.activeElement))) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && (document.activeElement === last || !this.el.contains(document.activeElement))) {
      event.preventDefault()
      first.focus()
    }
  }

  onDestroy() {
    // The dialog's `close` event would come a task later, to no listener.
    this.close()
    this.restore()
  }
}
