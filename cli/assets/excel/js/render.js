// Drawing one frame of the grid: per quadrant, fills, gridlines (hidden under
// fills, merges and overflowing text), merges, borders, text, the hatch over
// edits the service has not answered yet, then the selection, a fill
// handle's drag and the references of a formula being edited; the headers
// and the frozen split last. Only font metrics are decided here: overflow,
// `####`, the shorter General spellings, `*` fills, wrap and shrink. Every
// value and format arrives as text from the service.

import { KIND_NUMBER, KIND_BOOLEAN, KIND_ERROR } from './tiles.js'
import { columnName, contains, intersects } from './geometry.js'

const PAD = 2
const INDENT = 9 // pixels per indent level at 100%
const LOOK = 64 // columns searched beyond the frame for text overflowing into it
const MEASURED = 20000

const HEADER_FONT = '"Segoe UI", Carlito, Calibri, Arial, sans-serif'
// The colours a formula's references take in turn (`--ref-1`…), in the
// editor's text and on the grid alike.
export const REFERENCE_COLORS = 7

// A format's `*x` fill: `count` copies of `char` inserted at `offset`, the
// character index in the text where the format placed them (§8.3).
export const fillText = (text, char, offset, count) => {
  const at = Math.min(Math.max(0, Math.trunc(offset) || 0), text.length)
  return text.slice(0, at) + char.repeat(Math.max(0, count)) + text.slice(at)
}

export class Painter {
  constructor (canvas, styles) {
    this.canvas = canvas
    this.ctx = canvas.getContext('2d', { alpha: false })
    this.styles = styles
    this.widths = new Map()
    this.palette = null
    this.hatch = null
    this.last = { ms: 0, cells: 0 }
  }

  // Colours come from CSS custom properties, so forced-colors mode and the
  // stylesheet decide them; each is resolved to a concrete colour once.
  readPalette (host) {
    const probe = document.createElement('span')
    probe.className = 'palette-probe'
    host.append(probe)
    const names = ['sheet', 'grid', 'header', 'header-text', 'header-line', 'header-active', 'header-active-text',
      'header-full', 'accent', 'selection', 'frozen', 'text', 'pending']
    for (let n = 1; n <= REFERENCE_COLORS; n++) names.push('ref-' + n)
    const palette = {}
    for (const name of names) {
      probe.className = 'palette-probe palette-' + name
      palette[name] = getComputedStyle(probe).color
    }
    probe.remove()
    this.palette = palette
    this.hatch = null
  }

  measure (font, text) {
    const key = font + '\n' + text
    let width = this.widths.get(key)
    if (width === undefined) {
      if (this.widths.size > MEASURED) this.widths.clear()
      this.ctx.font = font
      width = this.ctx.measureText(text).width
      this.widths.set(key, width)
    }
    return width
  }

  headerFont (zoom) {
    return `${(11 * (96 / 72) * zoom * 0.9).toFixed(2)}px ${HEADER_FONT}`
  }

  // frame: { viewport, geometry, cell(row, column), selection, zoom, dpr,
  // gridlines, message, pending: ranges whose edit is unanswered, fill: a
  // fill-handle drag's { source, target, clear } or null, references: a
  // formula's { range, color, dashed } }
  paint (frame) {
    const started = performance.now()
    const { ctx, palette } = this
    const { viewport, dpr } = frame
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.fillStyle = palette.sheet
    ctx.fillRect(0, 0, viewport.width, viewport.height)
    this.cells = 0
    if (frame.message) {
      this.paintMessage(frame)
    } else {
      for (const quadrant of viewport.quadrants) {
        if (quadrant.w <= 0 || quadrant.h <= 0) continue
        ctx.save()
        ctx.beginPath()
        ctx.rect(quadrant.x, quadrant.y, quadrant.w, quadrant.h)
        ctx.clip()
        this.paintBody(frame, quadrant)
        this.paintPending(frame, quadrant)
        this.paintSelection(frame, quadrant)
        this.paintFill(frame, quadrant)
        this.paintReferences(frame, quadrant)
        ctx.restore()
      }
      this.paintHeaders(frame)
      this.paintSplit(frame)
    }
    this.last = { ms: performance.now() - started, cells: this.cells }
  }

  paintMessage (frame) {
    const { ctx, palette } = this
    const { viewport } = frame
    ctx.fillStyle = palette['header-text']
    ctx.font = this.headerFont(1)
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText(frame.message, viewport.width / 2, viewport.height / 2)
    ctx.textAlign = 'left'
  }

  // The shown lines of one quadrant, as flat [index, position, size] triples.
  lines (axis, first, last, origin) {
    const out = []
    let at = origin + axis.start(first)
    for (let i = first; i <= last; i++) {
      const size = axis.sizeOf(i)
      if (size > 0) out.push(i, at, size)
      at += size
    }
    return out
  }

  paintBody (frame, q) {
    const { ctx, palette, styles } = this
    const { geometry, cell } = frame
    const R = this.lines(geometry.rows, q.r0, q.r1, q.oy)
    const C = this.lines(geometry.columns, q.c0, q.c1, q.ox)
    if (R.length === 0 || C.length === 0) return
    const area = { r0: q.r0, r1: q.r1, c0: q.c0, c1: q.c1 }
    const merges = geometry.merges.within(area)
    const inMerge = merges.length > 0 ? (r, c) => geometry.merges.at(r, c) : () => null
    const styleOf = (r, c, found) => {
      if (found) return found.style
      return geometry.blankStyle(r, c)
    }

    // Gridlines first: fills and merges paint over the lines they hide.
    const bottom = R[R.length - 2] + R[R.length - 1]
    const right = C[C.length - 2] + C[C.length - 1]
    if (frame.gridlines) {
      ctx.fillStyle = palette.grid
      for (let j = 0; j < C.length; j += 3) ctx.fillRect(C[j + 1] + C[j + 2] - 1, q.y, 1, bottom - q.y)
      for (let i = 0; i < R.length; i += 3) ctx.fillRect(q.x, R[i + 1] + R[i + 2] - 1, right - q.x, 1)
    }

    // Fills, and the text each shown cell carries.
    const filled = new Set()
    const texts = []
    for (let i = 0; i < R.length; i += 3) {
      const r = R[i]
      const y = R[i + 1]
      const h = R[i + 2]
      for (let j = 0; j < C.length; j += 3) {
        const c = C[j]
        const found = cell(r, c)
        if (inMerge(r, c)) continue
        const id = styleOf(r, c, found)
        if (id) {
          const style = styles.get(id)
          if (style.fill) {
            ctx.fillStyle = style.fill
            ctx.fillRect(C[j + 1] - 1, y - 1, C[j + 2] + 1, h + 1)
            filled.add(r * 16384 + c)
          }
        }
        if (found && found.text !== '') texts.push({ cell: found, x: C[j + 1], y, w: C[j + 2], h })
      }
    }
    this.cells += texts.length

    // Text entering the frame from cells beyond its edges.
    for (let i = 0; i < R.length; i += 3) {
      const r = R[i]
      const entering = this.entering(frame, q, r, R[i + 1], R[i + 2])
      for (const item of entering) texts.push(item)
    }

    // Merges: one fill over the whole range, interior gridlines erased.
    const mergedTexts = []
    for (const merge of merges) {
      const x = q.ox + geometry.columns.start(merge.c0)
      const y = q.oy + geometry.rows.start(merge.r0)
      const w = geometry.columns.start(merge.c1 + 1) - geometry.columns.start(merge.c0)
      const h = geometry.rows.start(merge.r1 + 1) - geometry.rows.start(merge.r0)
      if (w <= 0 || h <= 0) continue
      const anchor = cell(merge.r0, merge.c0)
      const id = styleOf(merge.r0, merge.c0, anchor)
      const style = id ? styles.get(id) : null
      if (style && style.fill) {
        ctx.fillStyle = style.fill
        ctx.fillRect(x - 1, y - 1, w + 1, h + 1)
      } else {
        ctx.fillStyle = palette.sheet
        ctx.fillRect(x, y, w - 1, h - 1)
      }
      if (anchor && anchor.text !== '') mergedTexts.push({ cell: anchor, x, y, w, h, merge })
    }

    // Lay the text out, so the lines overflowing text crosses are known
    // before borders are drawn over them.
    const laid = []
    for (const item of texts) {
      const out = this.layout(frame, q, item, filled, inMerge)
      if (out) laid.push(out)
    }
    for (const item of mergedTexts) {
      const out = this.layout(frame, q, item, filled, inMerge)
      if (out) laid.push(out)
    }
    if (frame.gridlines) {
      ctx.fillStyle = palette.sheet
      for (const out of laid) for (const [x, y, h] of out.erase) ctx.fillRect(x, y, 1, h - 1)
    }

    // Borders, per cell, never across the inside of a merge.
    for (let i = 0; i < R.length; i += 3) {
      const r = R[i]
      const y = R[i + 1]
      const h = R[i + 2]
      for (let j = 0; j < C.length; j += 3) {
        const c = C[j]
        const id = styleOf(r, c, cell(r, c))
        if (!id) continue
        const style = styles.get(id)
        if (!style.bordered) continue
        const merge = inMerge(r, c)
        this.borders(style.edges, C[j + 1], y, C[j + 2], h, merge, r, c)
      }
    }

    for (const out of laid) this.drawText(out)
  }

  // Cells left of the frame whose left-aligned text runs into it, and cells
  // right of it whose right-aligned text does.
  entering (frame, q, r, y, h) {
    const { geometry, cell } = frame
    const out = []
    const low = q.c0 < geometry.frozen.columns ? 0 : geometry.frozen.columns
    for (let c = q.c0 - 1; c >= Math.max(low, q.c0 - LOOK); c--) {
      if (geometry.merges.at(r, c)) break
      const found = cell(r, c)
      if (found === undefined) break
      if (found && found.text !== '') {
        const w = geometry.columns.sizeOf(c)
        if (w > 0) out.push({ cell: found, x: q.ox + geometry.columns.start(c), y, w, h, outside: true })
        break
      }
    }
    const high = q.c0 < geometry.frozen.columns ? geometry.frozen.columns - 1 : 16383
    for (let c = q.c1 + 1; c <= Math.min(high, q.c1 + LOOK); c++) {
      if (geometry.merges.at(r, c)) break
      const found = cell(r, c)
      if (found === undefined) break
      if (found && found.text !== '') {
        const w = geometry.columns.sizeOf(c)
        if (w > 0) out.push({ cell: found, x: q.ox + geometry.columns.start(c), y, w, h, outside: true })
        break
      }
    }
    return out
  }

  // Where one cell's text goes: its lines, font, colour, anchor and clip.
  layout (frame, q, item, filled, inMerge) {
    const { styles } = this
    const { zoom } = frame
    const found = item.cell
    const style = styles.get(found.style)
    let font = styles.font(style, zoom)
    let px = styles.fontPx(style, zoom)
    const kind = found.kind
    let horizontal = style.horizontal
    if (horizontal === 'general') {
      horizontal = kind === KIND_NUMBER ? 'right' : kind === KIND_BOOLEAN || kind === KIND_ERROR ? 'center' : 'left'
    }
    let wrap = style.wrap
    if (horizontal === 'justify' || horizontal === 'distributed') {
      wrap = true
      horizontal = horizontal === 'justify' ? 'left' : 'center'
    }
    if (horizontal === 'centerContinuous') horizontal = 'center'
    const indent = (horizontal === 'left' || horizontal === 'right') ? style.indent * INDENT * zoom : 0
    const pad = PAD * zoom
    const available = Math.max(0, item.w - 2 * pad - indent)
    let text = found.text
    let width
    let clip = { x: item.x, y: item.y, w: item.w - 1, h: item.h - 1 }
    const erase = []
    let lines

    if (kind === KIND_NUMBER) {
      width = this.measure(font, text)
      if (width > available && style.shrink && !wrap) {
        px = Math.max(1, px * (available / width))
        font = font.replace(/[\d.]+px/, px.toFixed(2) + 'px')
        width = this.measure(font, text)
      } else if (width > available) {
        let chosen = null
        for (const shorter of found.shorter || []) {
          const w = this.measure(font, shorter)
          if (w <= available) {
            chosen = shorter
            width = w
            break
          }
        }
        if (chosen === null) {
          const hash = this.measure(font, '#')
          const count = Math.max(1, Math.floor(available / Math.max(1, hash)))
          text = '#'.repeat(count)
          width = this.measure(font, text)
        } else text = chosen
      }
      // The fill's offset indexes the service's text, not `####`.
      if (text === found.text) ({ text, width } = this.expandFill(font, found.fill, text, width, available))
      lines = [text]
    } else if (horizontal === 'fill') {
      const unit = this.measure(font, text)
      const count = unit > 0 ? Math.max(1, Math.floor(available / unit)) : 1
      text = text.repeat(count)
      width = this.measure(font, text)
      horizontal = 'left'
      lines = [text]
    } else if (wrap) {
      lines = this.wrapLines(font, text, Math.max(1, available))
      width = 0
      for (const line of lines) width = Math.max(width, this.measure(font, line))
    } else {
      text = text.replace(/\r?\n/g, ' ')
      width = this.measure(font, text)
      if (width > available && style.shrink) {
        px = Math.max(1, px * (available / Math.max(1, width)))
        font = font.replace(/[\d.]+px/, px.toFixed(2) + 'px')
        width = this.measure(font, text)
      }
      if (found.fill) ({ text, width } = this.expandFill(font, found.fill, text, width, available))
      lines = [text]
      // Plain text overflows into empty neighbours; nothing else does.
      if (width > available && kind === 0 && !item.merge && !found.fill) {
        const span = this.overflow(frame, q, item, horizontal, width + 2 * pad + indent, filled, inMerge)
        clip = span.clip
        for (const line of span.erase) erase.push(line)
      }
    }
    if (item.merge) clip = { x: item.x, y: item.y, w: item.w - 1, h: item.h - 1 }
    const lineHeight = px * 1.22
    const block = lineHeight * lines.length
    let top
    if (style.vertical === 'top') top = item.y + 1
    else if (style.vertical === 'center' || style.vertical === 'justify' || style.vertical === 'distributed') top = item.y + (item.h - 1 - block) / 2
    else top = item.y + item.h - 2 - block
    let x
    if (horizontal === 'right') x = item.x + item.w - 1 - pad - indent
    else if (horizontal === 'center') x = item.x + (item.w - 1) / 2
    else x = item.x + pad + indent
    const needsClip = width > clip.w - 2 * pad || block > item.h || (item.outside && !kind)
    return {
      lines,
      font,
      px,
      align: horizontal === 'right' ? 'right' : horizontal === 'center' ? 'center' : 'left',
      x,
      top,
      lineHeight,
      color: found.color || style.font.color || this.palette.text,
      underline: style.font.underline && style.font.underline !== 'none' ? style.font.underline : null,
      strike: style.font.strike,
      clip,
      needsClip: needsClip || Boolean(item.merge),
      erase,
      widths: lines.map((line) => this.measure(font, line))
    }
  }

  // A format's `*x` repeats x across the room the text leaves, at the offset
  // the service names: Accounting pads after its `$`.
  expandFill (font, fill, text, width, available) {
    if (!Array.isArray(fill) || width >= available) return { text, width }
    const [char, offset] = fill
    const unit = char ? this.measure(font, char) : 0
    if (unit <= 0) return { text, width }
    const expanded = fillText(text, char, offset, Math.floor((available - width) / unit))
    return { text: expanded, width: this.measure(font, expanded) }
  }

  // The run of empty cells text spills into, and the lines it crosses.
  overflow (frame, q, item, horizontal, need, filled, inMerge) {
    const { geometry, cell } = frame
    const { columns } = geometry
    const found = item.cell
    const row = found.row
    const first = q.c0 < geometry.frozen.columns ? 0 : geometry.frozen.columns
    const last = q.c0 < geometry.frozen.columns ? geometry.frozen.columns - 1 : 16383
    const free = (c) => {
      if (c < first || c > last) return false
      if (inMerge(row, c) || geometry.merges.at(row, c)) return false
      const neighbour = cell(row, c)
      return !neighbour || neighbour.text === ''
    }
    let left = found.column
    let right = found.column
    let span = item.w
    const grow = (direction) => {
      for (let guard = 0; guard < 256 && span < need; guard++) {
        const next = direction > 0 ? right + 1 : left - 1
        if (!free(next)) return
        const size = columns.sizeOf(next)
        if (direction > 0) right = next
        else left = next
        span += size
      }
    }
    if (horizontal === 'left') grow(1)
    else if (horizontal === 'right') grow(-1)
    else {
      // Centered text needs the same room on both sides.
      for (let guard = 0; guard < 256 && span < need; guard++) {
        const before = span
        if (free(left - 1) && free(right + 1)) {
          left--
          right++
          span += columns.sizeOf(left) + columns.sizeOf(right)
        }
        if (span === before) break
      }
    }
    const x0 = item.x - (columns.start(found.column) - columns.start(left))
    const x1 = item.x + (columns.start(right + 1) - columns.start(found.column))
    const erase = []
    for (let c = left; c < right; c++) {
      if (filled.has(row * 16384 + c) || filled.has(row * 16384 + c + 1)) continue
      erase.push([item.x + (columns.start(c + 1) - columns.start(found.column)) - 1, item.y, item.h])
    }
    return { clip: { x: x0, y: item.y, w: x1 - x0 - 1, h: item.h - 1 }, erase }
  }

  wrapLines (font, text, width) {
    const lines = []
    for (const paragraph of text.split(/\r?\n/)) {
      const words = paragraph.split(/(\s+)/)
      let line = ''
      for (const word of words) {
        if (word === '') continue
        const candidate = line + word
        if (line === '' || this.measure(font, candidate.trimEnd()) <= width) {
          line = candidate
          continue
        }
        lines.push(line.trimEnd())
        line = /^\s+$/.test(word) ? '' : word
        // A word wider than the cell breaks between characters.
        while (line && this.measure(font, line) > width && line.length > 1) {
          let cut = line.length - 1
          while (cut > 1 && this.measure(font, line.slice(0, cut)) > width) cut--
          lines.push(line.slice(0, cut))
          line = line.slice(cut)
        }
      }
      lines.push(line.trimEnd())
    }
    return lines
  }

  drawText (out) {
    const { ctx } = this
    if (out.needsClip) {
      ctx.save()
      ctx.beginPath()
      ctx.rect(out.clip.x, out.clip.y, Math.max(0, out.clip.w), Math.max(0, out.clip.h))
      ctx.clip()
    }
    ctx.font = out.font
    ctx.fillStyle = out.color
    ctx.textAlign = out.align
    ctx.textBaseline = 'middle'
    for (let i = 0; i < out.lines.length; i++) {
      const middle = out.top + out.lineHeight * i + out.lineHeight / 2
      ctx.fillText(out.lines[i], out.x, middle)
      if (out.underline || out.strike) {
        const width = out.widths[i]
        const start = out.align === 'right' ? out.x - width : out.align === 'center' ? out.x - width / 2 : out.x
        const thick = Math.max(1, Math.round(out.px / 14))
        if (out.underline) {
          const y = Math.round(middle + out.px * 0.42)
          ctx.fillRect(start, y, width, thick)
          if (out.underline.startsWith('double')) ctx.fillRect(start, y + thick + 1, width, thick)
        }
        if (out.strike) ctx.fillRect(start, Math.round(middle + out.px * 0.06), width, thick)
      }
    }
    ctx.textAlign = 'left'
    if (out.needsClip) ctx.restore()
  }

  // One cell's edges, each on the gridline it replaces.
  borders (edges, x, y, w, h, merge, r, c) {
    const draw = (edge, horizontal, at, from, length) => {
      if (!edge) return
      const { ctx } = this
      ctx.fillStyle = edge.color
      ctx.strokeStyle = edge.color
      const width = edge.width
      if (edge.double) {
        if (horizontal) {
          ctx.fillRect(from, at - 1, length, 1)
          ctx.fillRect(from, at + 1, length, 1)
        } else {
          ctx.fillRect(at - 1, from, 1, length)
          ctx.fillRect(at + 1, from, 1, length)
        }
        return
      }
      const offset = Math.floor((width - 1) / 2)
      if (edge.dash.length === 0) {
        if (horizontal) ctx.fillRect(from, at - offset, length, width)
        else ctx.fillRect(at - offset, from, width, length)
        return
      }
      ctx.save()
      ctx.lineWidth = width
      ctx.setLineDash(edge.dash)
      ctx.beginPath()
      if (horizontal) {
        ctx.moveTo(from, at - offset + width / 2)
        ctx.lineTo(from + length, at - offset + width / 2)
      } else {
        ctx.moveTo(at - offset + width / 2, from)
        ctx.lineTo(at - offset + width / 2, from + length)
      }
      ctx.stroke()
      ctx.restore()
    }
    const inside = (side) => merge && ((side === 'left' && c > merge.c0) || (side === 'right' && c < merge.c1) ||
      (side === 'top' && r > merge.r0) || (side === 'bottom' && r < merge.r1))
    if (!inside('top')) draw(edges.top, true, y - 1, x - 1, w + 1)
    if (!inside('bottom')) draw(edges.bottom, true, y + h - 1, x - 1, w + 1)
    if (!inside('left')) draw(edges.left, false, x - 1, y - 1, h + 1)
    if (!inside('right')) draw(edges.right, false, x + w - 1, y - 1, h + 1)
  }

  // A range's rectangle in one quadrant, clamped near it: a whole column is
  // 20M pixels tall.
  boxIn (q, geometry, range) {
    const x = q.ox + geometry.columns.start(range.c0)
    const y = q.oy + geometry.rows.start(range.r0)
    const x1 = q.ox + geometry.columns.start(range.c1 + 1)
    const y1 = q.oy + geometry.rows.start(range.r1 + 1)
    const cx = Math.max(x, q.x - 8)
    const cy = Math.max(y, q.y - 8)
    return { x: cx, y: cy, w: Math.min(x1, q.x + q.w + 8) - cx, h: Math.min(y1, q.y + q.h + 8) - cy, right: x1, bottom: y1 }
  }

  // Cells whose edit the service has not answered: the old value stays
  // under a hatch until the tile holding the new one arrives. Nothing typed
  // is drawn as though it were accepted.
  paintPending (frame, q) {
    const ranges = frame.pending
    if (!ranges || ranges.length === 0) return
    const { ctx, palette } = this
    const area = { r0: q.r0, r1: q.r1, c0: q.c0, c1: q.c1 }
    if (!this.hatch) {
      const tile = document.createElement('canvas')
      tile.width = 6
      tile.height = 6
      const pen = tile.getContext('2d')
      pen.strokeStyle = palette.pending
      pen.lineWidth = 1
      pen.beginPath()
      pen.moveTo(-1, 7)
      pen.lineTo(7, -1)
      pen.stroke()
      this.hatch = ctx.createPattern(tile, 'repeat')
    }
    ctx.save()
    ctx.lineWidth = 1
    ctx.setLineDash([3, 2])
    ctx.strokeStyle = palette.pending
    for (const range of ranges) {
      if (!intersects(range, area)) continue
      const box = this.boxIn(q, frame.geometry, range)
      if (box.w <= 1 || box.h <= 1) continue
      ctx.fillStyle = this.hatch
      ctx.fillRect(box.x, box.y, box.w - 1, box.h - 1)
      ctx.strokeRect(box.x + 0.5, box.y + 0.5, box.w - 2, box.h - 2)
    }
    ctx.restore()
  }

  paintSelection (frame, q) {
    const { ctx, palette } = this
    const { selection, geometry } = frame
    if (!selection) return
    const area = { r0: q.r0, r1: q.r1, c0: q.c0, c1: q.c1 }
    const rectOf = (range) => this.boxIn(q, geometry, range)
    const active = selection.active
    const activeRange = geometry.merges.at(active.row, active.column) ||
      { r0: active.row, c0: active.column, r1: active.row, c1: active.column }
    const single = selection.isSingle()
    if (!single) {
      ctx.beginPath()
      for (const range of selection.ranges) {
        if (!intersects(range, area)) continue
        const box = rectOf(range)
        ctx.rect(box.x, box.y, box.w - 1, box.h - 1)
      }
      if (intersects(activeRange, area)) {
        const box = rectOf(activeRange)
        ctx.rect(box.x, box.y, box.w - 1, box.h - 1)
      }
      ctx.fillStyle = palette.selection
      ctx.fill('evenodd')
    }
    ctx.strokeStyle = palette.accent
    if (selection.ranges.length === 1 && intersects(selection.range, area)) {
      const range = selection.range
      const box = rectOf(range)
      ctx.lineWidth = 2
      ctx.strokeRect(box.x - 1, box.y - 1, box.w, box.h)
      // The fill handle, where the range's bottom-right corner is shown.
      if (contains(area, range.r1, range.c1) && box.right <= q.x + q.w + 8 && box.bottom <= q.y + q.h + 8) {
        ctx.fillStyle = palette.sheet
        ctx.fillRect(box.right - 5, box.bottom - 5, 8, 8)
        ctx.fillStyle = palette.accent
        ctx.fillRect(box.right - 4, box.bottom - 4, 6, 6)
      }
    } else if (intersects(activeRange, area)) {
      const box = rectOf(activeRange)
      ctx.lineWidth = 1
      ctx.strokeRect(box.x - 0.5, box.y - 0.5, box.w, box.h)
    }
  }

  // A fill-handle drag: what a release fills, in a grey dashed frame; drawn
  // back over the selection, the cells it would clear shaded.
  paintFill (frame, q) {
    const fill = frame.fill
    if (!fill) return
    const { ctx, palette } = this
    const area = { r0: q.r0, r1: q.r1, c0: q.c0, c1: q.c1 }
    ctx.save()
    ctx.strokeStyle = palette.frozen
    ctx.fillStyle = palette.frozen
    if (fill.clear && intersects(fill.clear, area)) {
      const box = this.boxIn(q, frame.geometry, fill.clear)
      ctx.globalAlpha = 0.3
      ctx.fillRect(box.x, box.y, box.w - 1, box.h - 1)
      ctx.globalAlpha = 1
    }
    if (intersects(fill.target, area)) {
      const box = this.boxIn(q, frame.geometry, fill.target)
      ctx.lineWidth = 2
      ctx.setLineDash([3, 2])
      ctx.strokeRect(box.x - 1, box.y - 1, box.w, box.h)
    }
    ctx.restore()
  }

  // A formula's references while it is edited, as Excel draws them: each
  // range framed in its colour with a square at every corner; the range
  // being pointed at, dashed.
  paintReferences (frame, q) {
    const references = frame.references
    if (!references || references.length === 0) return
    const { ctx, palette } = this
    const area = { r0: q.r0, r1: q.r1, c0: q.c0, c1: q.c1 }
    ctx.save()
    ctx.lineWidth = 2
    for (const item of references) {
      if (!intersects(item.range, area)) continue
      const color = palette['ref-' + ((item.color % REFERENCE_COLORS) + 1)]
      const box = this.boxIn(q, frame.geometry, item.range)
      ctx.strokeStyle = color
      ctx.setLineDash(item.dashed ? [4, 3] : [])
      ctx.strokeRect(box.x, box.y, box.w - 1, box.h - 1)
      if (item.dashed) continue
      // The squares sit on the range's own corners, where the frame shows.
      ctx.fillStyle = color
      const left = q.ox + frame.geometry.columns.start(item.range.c0)
      const top = q.oy + frame.geometry.rows.start(item.range.r0)
      const right = box.right - 1
      const bottom = box.bottom - 1
      for (const [x, y] of [[left, top], [right, top], [left, bottom], [right, bottom]]) {
        if (x < q.x - 3 || x > q.x + q.w + 3 || y < q.y - 3 || y > q.y + q.h + 3) continue
        ctx.fillRect(x - 2, y - 2, 5, 5)
      }
    }
    ctx.restore()
  }

  paintHeaders (frame) {
    const { ctx, palette } = this
    const { viewport, geometry, selection, zoom } = frame
    const { headerWidth: hw, headerHeight: hh } = viewport
    const font = this.headerFont(zoom)
    const selectedColumns = (c) => {
      let any = false
      let full = false
      for (const range of selection.ranges) {
        if (c >= range.c0 && c <= range.c1) {
          any = true
          if (range.r0 === 0 && range.r1 === 1048575) full = true
        }
      }
      return full ? 2 : any ? 1 : 0
    }
    const selectedRows = (r) => {
      let any = false
      let full = false
      for (const range of selection.ranges) {
        if (r >= range.r0 && r <= range.r1) {
          any = true
          if (range.c0 === 0 && range.c1 === 16383) full = true
        }
      }
      return full ? 2 : any ? 1 : 0
    }
    ctx.font = font
    ctx.textBaseline = 'middle'
    ctx.textAlign = 'center'

    // Column letters: the frozen columns, then the scrolled ones.
    const columnSpans = [
      [0, geometry.frozen.columns - 1, hw, hw, viewport.bodyX],
      [viewport.scrolled.c0, viewport.scrolled.c1, viewport.ox, viewport.bodyX, viewport.width]
    ]
    ctx.fillStyle = palette.header
    ctx.fillRect(hw, 0, viewport.width - hw, hh)
    for (const [c0, c1, origin, clipLeft, clipRight] of columnSpans) {
      if (c1 < c0 || clipRight <= clipLeft) continue
      ctx.save()
      ctx.beginPath()
      ctx.rect(clipLeft, 0, clipRight - clipLeft, hh)
      ctx.clip()
      let x = origin + geometry.columns.start(c0)
      for (let c = c0; c <= c1; c++) {
        const w = geometry.columns.sizeOf(c)
        if (w > 0) {
          const state = selectedColumns(c)
          if (state) {
            ctx.fillStyle = state === 2 ? palette['header-full'] : palette['header-active']
            ctx.fillRect(x, 0, w - 1, hh - 1)
            ctx.fillStyle = palette.accent
            ctx.fillRect(x - 1, hh - 2, w + 1, 2)
          }
          ctx.fillStyle = palette['header-line']
          ctx.fillRect(x + w - 1, 0, 1, hh)
          ctx.fillStyle = state ? palette['header-active-text'] : palette['header-text']
          ctx.fillText(columnName(c), x + w / 2, hh / 2)
        }
        x += w
      }
      ctx.restore()
    }

    // Row numbers.
    const rowSpans = [
      [0, geometry.frozen.rows - 1, hh, hh, viewport.bodyY],
      [viewport.scrolled.r0, viewport.scrolled.r1, viewport.oy, viewport.bodyY, viewport.height]
    ]
    ctx.fillStyle = palette.header
    ctx.fillRect(0, hh, hw, viewport.height - hh)
    for (const [r0, r1, origin, clipTop, clipBottom] of rowSpans) {
      if (r1 < r0 || clipBottom <= clipTop) continue
      ctx.save()
      ctx.beginPath()
      ctx.rect(0, clipTop, hw, clipBottom - clipTop)
      ctx.clip()
      let y = origin + geometry.rows.start(r0)
      for (let r = r0; r <= r1; r++) {
        const h = geometry.rows.sizeOf(r)
        if (h > 0) {
          const state = selectedRows(r)
          if (state) {
            ctx.fillStyle = state === 2 ? palette['header-full'] : palette['header-active']
            ctx.fillRect(0, y, hw - 1, h - 1)
            ctx.fillStyle = palette.accent
            ctx.fillRect(hw - 2, y - 1, 2, h + 1)
          }
          ctx.fillStyle = palette['header-line']
          ctx.fillRect(0, y + h - 1, hw, 1)
          ctx.fillStyle = state ? palette['header-active-text'] : palette['header-text']
          ctx.fillText(String(r + 1), hw / 2, y + h / 2)
        }
        y += h
      }
      ctx.restore()
    }
    ctx.textAlign = 'left'

    // The corner, with the select-all triangle.
    ctx.fillStyle = palette.header
    ctx.fillRect(0, 0, hw, hh)
    ctx.fillStyle = palette['header-line']
    ctx.fillRect(hw - 1, 0, 1, viewport.height)
    ctx.fillRect(0, hh - 1, viewport.width, 1)
    ctx.beginPath()
    ctx.moveTo(hw - 4, hh - 4)
    ctx.lineTo(hw - 4, Math.max(4, hh - 14 * zoom))
    ctx.lineTo(Math.max(4, hw - 14 * zoom), hh - 4)
    ctx.closePath()
    ctx.fillStyle = palette['header-text']
    ctx.fill()
  }

  paintSplit (frame) {
    const { ctx, palette } = this
    const { viewport, geometry } = frame
    ctx.fillStyle = palette.frozen
    if (geometry.frozen.columns > 0) ctx.fillRect(viewport.bodyX - 1, 0, 1, viewport.height)
    if (geometry.frozen.rows > 0) ctx.fillRect(0, viewport.bodyY - 1, viewport.width, 1)
  }
}
