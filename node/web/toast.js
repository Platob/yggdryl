// Notices stacked in a corner. Information is announced politely and leaves
// by itself; a refusal - the service's text, verbatim - is announced as an
// alert and stays until it is dismissed. The two live regions exist from the
// start, so what is added to them is announced.

import { Component, element } from './component.js'

const KINDS = new Set(['info', 'refusal'])

export class Toasts extends Component {
  constructor({ timeoutMs = 5000 } = {}) {
    super({ timeoutMs })
    this.timers = new Map()
  }

  render() {
    const root = element('div', 'ygg-ui__toasts')
    root.setAttribute('aria-label', 'Notices')
    this.assertive = element('div', 'ygg-ui__toast-region')
    this.assertive.setAttribute('role', 'alert')
    this.polite = element('div', 'ygg-ui__toast-region')
    this.polite.setAttribute('role', 'status')
    this.polite.setAttribute('aria-live', 'polite')
    root.append(this.assertive, this.polite)
    return root
  }

  /** Show `text`; answers `{ dismiss }`. */
  push({ text, kind = 'info' } = {}) {
    if (!KINDS.has(kind)) throw new TypeError(`expected kind 'info' or 'refusal', got ${JSON.stringify(kind)}`)
    const toast = element('div', `ygg-ui__toast ygg-ui__toast--${kind}`)
    const body = element('span', 'ygg-ui__toast-text', String(text ?? ''))
    const dismiss = element('button', 'ygg-ui__toast-dismiss', '×')
    dismiss.type = 'button'
    dismiss.setAttribute('aria-label', 'Dismiss')
    dismiss.addEventListener('click', () => this.dismiss(toast))
    toast.append(body, dismiss)
    ;(kind === 'refusal' ? this.assertive : this.polite).append(toast)
    if (kind === 'info') this.timers.set(toast, setTimeout(() => this.dismiss(toast), this.props.timeoutMs))
    return { dismiss: () => this.dismiss(toast) }
  }

  dismiss(toast) {
    clearTimeout(this.timers.get(toast))
    this.timers.delete(toast)
    toast.remove()
  }

  onDestroy() {
    for (const timer of this.timers.values()) clearTimeout(timer)
    this.timers.clear()
  }
}
