// Resizable panes: two or more panes side by side (`horizontal`) or stacked
// (`vertical`), each pair split by a `role="separator"` that the pointer drags
// and the keyboard moves - the arrows by a step, Home and End to the limits -
// reporting the first pane's share as `aria-valuenow`. Sizes are shares of
// one hundred. Redrawing what is inside a pane when it changes size is the
// app's work (a chart watches its own box).

import { Component, element } from './component.js'

let serial = 0

export class Split extends Component {
  constructor({ direction = 'horizontal', sizes = [50, 50], min = 10, step = 2, label = 'panes' } = {}) {
    if (direction !== 'horizontal' && direction !== 'vertical') {
      throw new TypeError(`expected direction 'horizontal' or 'vertical', got ${JSON.stringify(direction)}`)
    }
    super({ direction, min, step, label })
    this.sizes = normalized(sizes)
    this.base = `ygg-ui-split-${++serial}`
    this.drag = null
  }

  render() {
    const { direction, label } = this.props
    const root = element('div', `ygg-ui__split ygg-ui__split--${direction}`)
    this.panes = []
    this.separators = []
    this.sizes.forEach((_, index) => {
      if (index > 0) {
        const separator = element('div', 'ygg-ui__split-separator')
        separator.setAttribute('role', 'separator')
        separator.setAttribute('aria-orientation', direction === 'horizontal' ? 'vertical' : 'horizontal')
        separator.setAttribute('aria-controls', `${this.base}-pane-${index - 1}`)
        separator.setAttribute('aria-label', `Resize ${label}`)
        separator.tabIndex = 0
        separator.dataset.index = String(index - 1)
        this.separators.push(separator)
        root.append(separator)
      }
      const pane = element('div', 'ygg-ui__split-pane')
      pane.id = `${this.base}-pane-${index}`
      this.panes.push(pane)
      root.append(pane)
    })
    return root
  }

  onMount() {
    this.apply()
    for (const separator of this.separators) {
      const index = Number(separator.dataset.index)
      this.listen(separator, 'keydown', (event) => this.onKey(event, index))
      this.listen(separator, 'pointerdown', (event) => this.start(event, index))
      this.listen(separator, 'pointermove', (event) => this.move(event))
      this.listen(separator, 'pointerup', (event) => this.end(event))
      this.listen(separator, 'pointercancel', (event) => this.end(event))
    }
  }

  /** Set pane `index`'s share, its neighbour taking the rest of the pair; clamped. */
  resize(index, size) {
    const pair = this.sizes[index] + this.sizes[index + 1]
    const { min } = this.props
    const held = Math.min(pair - min, Math.max(min, size))
    if (held === this.sizes[index]) return false
    this.sizes[index] = held
    this.sizes[index + 1] = pair - held
    this.apply()
    return true
  }

  onKey(event, index) {
    const { step, min } = this.props
    const pair = this.sizes[index] + this.sizes[index + 1]
    const targets = {
      ArrowRight: this.sizes[index] + step,
      ArrowDown: this.sizes[index] + step,
      ArrowLeft: this.sizes[index] - step,
      ArrowUp: this.sizes[index] - step,
      Home: min,
      End: pair - min,
    }
    if (!(event.key in targets)) return
    event.preventDefault()
    event.stopPropagation()
    if (this.resize(index, targets[event.key])) this.emit('ygg:resize', { sizes: [...this.sizes] })
  }

  start(event, index) {
    const horizontal = this.props.direction === 'horizontal'
    const first = this.panes[index].getBoundingClientRect()
    const second = this.panes[index + 1].getBoundingClientRect()
    this.drag = {
      index,
      origin: horizontal ? event.clientX : event.clientY,
      size: this.sizes[index],
      pixels: horizontal ? first.width + second.width : first.height + second.height,
      pair: this.sizes[index] + this.sizes[index + 1],
      moved: false,
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }

  move(event) {
    const drag = this.drag
    if (!drag || drag.pixels <= 0) return
    const at = this.props.direction === 'horizontal' ? event.clientX : event.clientY
    const share = ((at - drag.origin) / drag.pixels) * drag.pair
    if (this.resize(drag.index, drag.size + share)) drag.moved = true
  }

  end(event) {
    const drag = this.drag
    if (!drag) return
    this.drag = null
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
    if (drag.moved) this.emit('ygg:resize', { sizes: [...this.sizes] })
  }

  apply() {
    if (!this.el) return
    const template = this.sizes.map((size) => `minmax(0, ${size}fr)`).join(' var(--ygg-ui-split, 6px) ')
    if (this.props.direction === 'horizontal') this.el.style.gridTemplateColumns = template
    else this.el.style.gridTemplateRows = template
    this.separators.forEach((separator, index) => {
      const pair = this.sizes[index] + this.sizes[index + 1]
      separator.setAttribute('aria-valuenow', String(Math.round(this.sizes[index])))
      separator.setAttribute('aria-valuemin', String(this.props.min))
      separator.setAttribute('aria-valuemax', String(Math.round(pair - this.props.min)))
    })
  }

  draw(state) {
    if (state?.sizes && state.sizes.length === this.sizes.length) {
      this.sizes = normalized(state.sizes)
      this.apply()
    }
  }
}

/** Positive shares scaled to sum to one hundred. */
function normalized(sizes) {
  if (!Array.isArray(sizes) || sizes.length < 2 || sizes.some((size) => !(size > 0))) {
    throw new TypeError('expected two or more positive pane sizes')
  }
  const total = sizes.reduce((sum, size) => sum + size, 0)
  return sizes.map((size) => (size * 100) / total)
}
