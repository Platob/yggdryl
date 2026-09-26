// One shared tooltip: a `role="tooltip"` element placed beside its target's
// rectangle, shown on hover and focus, hidden on leave, blur and Escape, and
// named by the target's `aria-describedby` while it shows.

import { Component, element } from './component.js'

let serial = 0

export class Tooltip extends Component {
  render() {
    const tip = element('div', 'ygg-ui__tooltip')
    tip.id = `ygg-ui-tooltip-${++serial}`
    tip.setAttribute('role', 'tooltip')
    tip.hidden = true
    this.target = null
    return tip
  }

  onMount() {
    this.listen(document, 'keydown', (event) => {
      if (event.key === 'Escape' && this.target) this.hide()
    })
  }

  /** Show `content` - text or a node - beside `target`, which it then describes. */
  show(target, content) {
    if (!this.el) return
    if (this.target && this.target !== target) this.release()
    this.el.replaceChildren(typeof content === 'string' ? document.createTextNode(content) : content)
    this.target = target
    const described = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean)
    if (!described.includes(this.el.id)) target.setAttribute('aria-describedby', [...described, this.el.id].join(' '))
    this.el.hidden = false
    this.place(target)
  }

  /** Hide the tooltip and stop describing its target. */
  hide() {
    if (!this.el) return
    this.release()
    this.el.hidden = true
  }

  /** Show on hover and focus of `target`, hide on leave and blur; answers the detach function. */
  attach(target, content) {
    const show = () => this.show(target, typeof content === 'function' ? content() : content)
    const hide = () => {
      if (this.target === target) this.hide()
    }
    const pairs = [
      ['pointerenter', show],
      ['focus', show],
      ['pointerleave', hide],
      ['blur', hide],
    ]
    for (const [type, handler] of pairs) target.addEventListener(type, handler)
    return () => {
      for (const [type, handler] of pairs) target.removeEventListener(type, handler)
      hide()
    }
  }

  release() {
    const target = this.target
    this.target = null
    if (!target) return
    const described = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter((id) => id && id !== this.el.id)
    if (described.length) target.setAttribute('aria-describedby', described.join(' '))
    else target.removeAttribute('aria-describedby')
  }

  /** Below the target when it fits, else above; kept inside the viewport. */
  place(target) {
    const rect = target.getBoundingClientRect()
    const tip = this.el.getBoundingClientRect()
    const gap = 6
    const below = rect.bottom + gap + tip.height <= window.innerHeight
    const top = below ? rect.bottom + gap : Math.max(gap, rect.top - gap - tip.height)
    const left = Math.min(Math.max(gap, rect.left), Math.max(gap, window.innerWidth - tip.width - gap))
    this.el.style.top = `${Math.round(top)}px`
    this.el.style.left = `${Math.round(left)}px`
  }

  onDestroy() {
    this.release()
  }
}
