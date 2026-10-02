// The workbook's style table as the service resolves it (§8.3 `styles`):
// append-only, so an id once read never changes. Each entry is normalized once
// into what drawing needs, and CSS font strings are cached per zoom.

export const FONT_STACK = 'Calibri, Carlito, "Segoe UI", Arial, sans-serif'

const DEFAULT = {
  font: { name: 'Calibri', size: 11, bold: false, italic: false, underline: 'none', strike: false, color: '#000000' },
  fill: { pattern: 'none' },
  border: {},
  alignment: { horizontal: 'general', vertical: 'bottom', wrap: false, indent: 0, rotation: 0, shrink: false },
  numberFormat: 'General'
}

// Pixel width and dash per border style (ECMA-376 §18.18.3).
const BORDERS = {
  hair: { width: 1, dash: [1, 1] },
  thin: { width: 1, dash: [] },
  dotted: { width: 1, dash: [1, 2] },
  dashed: { width: 1, dash: [3, 1] },
  dashDot: { width: 1, dash: [3, 1, 1, 1] },
  dashDotDot: { width: 1, dash: [3, 1, 1, 1, 1, 1] },
  medium: { width: 2, dash: [] },
  mediumDashed: { width: 2, dash: [6, 2] },
  mediumDashDot: { width: 2, dash: [6, 2, 2, 2] },
  mediumDashDotDot: { width: 2, dash: [6, 2, 2, 2, 2, 2] },
  slantDashDot: { width: 2, dash: [6, 1, 2, 1] },
  thick: { width: 3, dash: [] },
  double: { width: 3, dash: [], double: true }
}

// How much of the foreground a pattern fill shows over its background.
const PATTERN_SHARE = {
  solid: 1,
  darkGray: 0.75,
  mediumGray: 0.5,
  lightGray: 0.25,
  gray125: 0.125,
  gray0625: 0.0625
}

const hex = (color) => {
  const match = /^#?([0-9a-f]{6})$/i.exec(color || '')
  return match ? parseInt(match[1], 16) : null
}

const mix = (foreground, background, share) => {
  const f = hex(foreground)
  const b = hex(background) ?? 0xffffff
  if (f === null) return null
  const channel = (shift) => Math.round(((f >> shift) & 255) * share + ((b >> shift) & 255) * (1 - share))
  return '#' + ((channel(16) << 16) | (channel(8) << 8) | channel(0)).toString(16).padStart(6, '0')
}

const fillColor = (fill) => {
  if (!fill || !fill.pattern || fill.pattern === 'none') return null
  const share = PATTERN_SHARE[fill.pattern] ?? 0.5
  return mix(fill.color || '#000000', fill.background || '#FFFFFF', share)
}

const edge = (side) => {
  if (!side || !side.style || side.style === 'none') return null
  const shape = BORDERS[side.style] || BORDERS.thin
  return { ...shape, color: side.color || '#000000' }
}

// Only the service's resolved facts reach this: colours are `#RRGGBB`.
const normalize = (style) => {
  const font = { ...DEFAULT.font, ...(style.font || {}) }
  const alignment = { ...DEFAULT.alignment, ...(style.alignment || {}) }
  const border = style.border || {}
  const edges = {
    left: edge(border.left),
    right: edge(border.right),
    top: edge(border.top),
    bottom: edge(border.bottom)
  }
  return {
    font,
    fill: fillColor(style.fill),
    edges,
    bordered: Boolean(edges.left || edges.right || edges.top || edges.bottom),
    horizontal: alignment.horizontal || 'general',
    vertical: alignment.vertical || 'bottom',
    wrap: Boolean(alignment.wrap),
    indent: alignment.indent || 0,
    rotation: alignment.rotation || 0,
    shrink: Boolean(alignment.shrink),
    numberFormat: style.numberFormat || 'General',
    fonts: new Map()
  }
}

const quoteFamily = (name) => (/^[\w -]+$/.test(name) ? `"${name}"` : `"${name.replace(/["\\]/g, '')}"`)

export class Styles extends EventTarget {
  constructor (api) {
    super()
    this.api = api
    this.table = []
    this.count = 0
    this.loading = null
    this.fallback = normalize(DEFAULT)
  }

  reset () {
    this.table = []
    this.count = 0
    this.loading = null
  }

  // Read the ids past those already held; the answer's `count` is the total.
  load () {
    if (this.loading) return this.loading
    const from = this.table.length
    this.loading = this.api.get(`styles?from=${from}`).then((answer) => {
      for (const style of answer.styles || []) this.table.push(normalize(style))
      this.count = answer.count ?? this.table.length
      this.dispatchEvent(new CustomEvent('change'))
    }).finally(() => {
      this.loading = null
    })
    return this.loading
  }

  get (id) {
    const style = this.table[id]
    if (style) return style
    if (id >= this.table.length && !this.loading) this.load().catch(() => {})
    return this.table[0] || this.fallback
  }

  // The CSS font a style draws with at `zoom`; points become 96-dpi pixels.
  font (style, zoom) {
    let font = style.fonts.get(zoom)
    if (!font) {
      const px = Math.max(1, (style.font.size || 11) * (96 / 72) * zoom)
      font = (style.font.italic ? 'italic ' : '') + (style.font.bold ? 'bold ' : '') +
        px.toFixed(2) + 'px ' + quoteFamily(style.font.name || 'Calibri') + ', ' + FONT_STACK
      style.fonts.set(zoom, font)
    }
    return font
  }

  fontPx (style, zoom) {
    return Math.max(1, (style.font.size || 11) * (96 / 72) * zoom)
  }
}
