// The one shape every component has: a class over one root element, mounted
// once, updated with state, destroyed once, emitting DOM `CustomEvent`s for
// what the person wants. It holds no market logic; it lays out what the
// native package answered.

export class Component {
  constructor(props = {}) {
    this.props = props
    this.el = null
    this.state = undefined
    this._frame = 0
    this._listeners = []
  }

  /** Build the root element. Subclasses override. */
  render() {
    return document.createElement('div')
  }

  /** Attach under `parent` and return this. */
  mount(parent) {
    if (this.el) throw new Error(`${this.constructor.name} is already mounted`)
    this.el = this.render()
    this.el.classList.add('ygg-ui')
    parent.append(this.el)
    this.onMount?.()
    if (this.state !== undefined) this.schedule()
    return this
  }

  /** Take new state and redraw at most once per frame. */
  update(state) {
    this.state = state
    if (this.el) this.schedule()
    return this
  }

  /** Redraw on the next animation frame, once however often it is asked. */
  schedule() {
    if (this._frame) return
    const raf = globalThis.requestAnimationFrame ?? ((fn) => setTimeout(fn, 16))
    this._frame = raf(() => {
      this._frame = 0
      this.draw?.(this.state)
    })
  }

  /** Emit a bubbling `CustomEvent` from the root element. */
  emit(name, detail) {
    if (!this.el) return false
    return this.el.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }))
  }

  /** Listen on a target and forget the listener at destroy. */
  listen(target, type, handler, options) {
    target.addEventListener(type, handler, options)
    this._listeners.push(() => target.removeEventListener(type, handler, options))
  }

  /** Detach and release everything this component holds. */
  destroy() {
    if (this._frame) {
      const caf = globalThis.cancelAnimationFrame ?? clearTimeout
      caf(this._frame)
      this._frame = 0
    }
    for (const forget of this._listeners.splice(0)) forget()
    this.onDestroy?.()
    this.el?.remove()
    this.el = null
  }
}

/** One element with a class list and optional text, the shape every renderer uses. */
export function element(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}
