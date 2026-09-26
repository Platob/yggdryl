// The symbol a replay follows: the symbols the service listed, in its order,
// the consolidated `GLOBAL` book named as such. Choosing emits `ygg:symbol`
// `{ symbol, global }`, `global` true exactly for the consolidated book.

import { Component, element } from './component.js'

/** The ticker of the consolidated book the core's iterator emits in global mode. */
export const GLOBAL = 'GLOBAL'

export class SymbolSelect extends Component {
  constructor({ label = 'Symbol' } = {}) {
    super({ label })
    this.symbols = undefined
  }

  render() {
    const root = element('label', 'ygg-ui__symbol')
    root.append(element('span', 'ygg-ui__symbol-label', `${this.props.label} `))
    this.select = element('select', 'ygg-ui__symbol-select')
    root.append(this.select)
    return root
  }

  onMount() {
    this.listen(this.select, 'change', () => {
      const symbol = this.select.value
      this.emit('ygg:symbol', { symbol, global: symbol === GLOBAL })
    })
  }

  draw(state) {
    const symbols = state?.symbols ?? []
    if (symbols !== this.symbols) {
      this.symbols = symbols
      this.select.replaceChildren(...symbols.map((symbol) => new Option(symbol === GLOBAL ? `${GLOBAL} (consolidated)` : symbol, symbol)))
    }
    const chosen = state?.global ? GLOBAL : state?.symbol
    if (chosen !== undefined && this.select.value !== chosen) this.select.value = chosen
  }
}
