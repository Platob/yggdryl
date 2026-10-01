// Data tiles: 64 rows by 32 columns of display text, fetched on demand, kept
// in a least-recently-used map, revalidated with their ETag, and prefetched one
// tile around what is shown. At most six requests are in flight, and none for
// a sheet the API's gate has shut (one being removed, or gone).

import { TILE_ROWS, TILE_COLUMNS } from './geometry.js'

export const CAPACITY = 256
export const IN_FLIGHT = 6

// A tile cell as the service writes it: [dRow, dCol, text, style, flags,
// color?, fill?, shorter?] (§8.3). `fill` is `[char, offset]`: a format's
// `*x` repeats `char` at character `offset` of `text`.
export const KIND_TEXT = 0
export const KIND_NUMBER = 1
export const KIND_BOOLEAN = 2
export const KIND_ERROR = 3
export const FLAG_FORMULA = 4
export const FLAG_UNCOMPUTED = 8
export const FLAG_STYLED_BLANK = 16
export const FLAG_ARRAY = 32
export const FLAG_PIVOT = 64

const decode = (row0, column0, item) => {
  const flags = item[4] || 0
  return {
    row: row0 + item[0],
    column: column0 + item[1],
    text: item[2] ?? '',
    style: item[3] || 0,
    flags,
    kind: flags & 3,
    color: item[5] ?? null,
    fill: item[6] ?? null,
    shorter: item[7] ?? null
  }
}

export const tileOf = (row, column) => [Math.floor(row / TILE_ROWS), Math.floor(column / TILE_COLUMNS)]

const keyOf = (sheet, tr, tc) => sheet + ':' + tr + ':' + tc

export class Tiles extends EventTarget {
  constructor (api) {
    super()
    this.api = api
    this.entries = new Map() // key -> { sheet, tr, tc, etag, cells, stale }
    this.flight = new Map() // key -> { controller, stale }
    this.queue = []
    this.wanted = new Set()
    this.generation = 0
    // While the sheet list is being reread nothing new is sent: a sheet an
    // edit just removed answers 404. Holds nest (an edit's answer and the
    // change feed may reread it at once); the last release sends.
    this.holds = 0
  }

  // Everything held belongs to one workbook generation.
  reset (generation) {
    for (const request of this.flight.values()) request.controller.abort()
    this.flight.clear()
    this.entries.clear()
    this.queue = []
    this.wanted.clear()
    this.generation = generation
  }

  entry (sheet, tr, tc) {
    const key = keyOf(sheet, tr, tc)
    const entry = this.entries.get(key)
    if (entry) {
      this.entries.delete(key)
      this.entries.set(key, entry)
    }
    return entry || null
  }

  // A cell, `null` when its tile is known and holds none, `undefined` when the
  // tile has not arrived.
  cell (sheet, row, column) {
    const entry = this.entry(sheet, Math.floor(row / TILE_ROWS), Math.floor(column / TILE_COLUMNS))
    if (!entry) return undefined
    return entry.cells.get((row % TILE_ROWS) * TILE_COLUMNS + (column % TILE_COLUMNS)) || null
  }

  // As `cell`, but only from a tile known current: `undefined` while it is
  // absent or stale, so "this cell is empty" is never read off an old tile.
  current (sheet, row, column) {
    const entry = this.entry(sheet, Math.floor(row / TILE_ROWS), Math.floor(column / TILE_COLUMNS))
    if (!entry || entry.stale) return undefined
    return entry.cells.get((row % TILE_ROWS) * TILE_COLUMNS + (column % TILE_COLUMNS)) || null
  }

  // A reader over one block of tiles, resolving each tile once per frame.
  view (sheet) {
    const seen = new Map()
    return (row, column) => {
      const tr = Math.floor(row / TILE_ROWS)
      const tc = Math.floor(column / TILE_COLUMNS)
      const key = tr * 1024 + tc
      let entry = seen.get(key)
      if (entry === undefined) {
        entry = this.entry(sheet, tr, tc)
        seen.set(key, entry)
      }
      if (!entry) return undefined
      return entry.cells.get((row % TILE_ROWS) * TILE_COLUMNS + (column % TILE_COLUMNS)) || null
    }
  }

  // Ask for the tiles a frame shows, then the ring around them. Queued tiles
  // no longer wanted are dropped; requests already sent finish.
  want (sheet, shown, ring) {
    this.wanted = new Set(shown.map(([tr, tc]) => keyOf(sheet, tr, tc)))
    this.queue = []
    for (const [tr, tc] of shown) this.enqueue(sheet, tr, tc, true)
    for (const [tr, tc] of ring) this.enqueue(sheet, tr, tc, false)
    this.pump()
  }

  enqueue (sheet, tr, tc, shown) {
    const key = keyOf(sheet, tr, tc)
    if (this.flight.has(key)) return
    const entry = this.entries.get(key)
    // A stale tile is revalidated only while it is shown; the ring waits.
    if (entry && (!entry.stale || !shown)) return
    this.queue.push({ sheet, tr, tc, key })
  }

  pending (sheet, shown) {
    let count = 0
    for (const [tr, tc] of shown) {
      const entry = this.entries.get(keyOf(sheet, tr, tc))
      if (!entry || entry.stale) count++
    }
    return count
  }

  hold (held) {
    this.holds = Math.max(0, this.holds + (held ? 1 : -1))
    if (this.holds === 0) this.pump()
  }

  // A sheet gone from the workbook: its queue and its tiles go. Answers
  // already on their way land and age out of the LRU.
  drop (sheet) {
    this.queue = this.queue.filter((item) => item.sheet !== sheet)
    for (const [key, entry] of this.entries) if (entry.sheet === sheet) this.entries.delete(key)
  }

  pump () {
    while (this.holds === 0 && this.flight.size < IN_FLIGHT && this.queue.length > 0) {
      const next = this.queue.shift()
      if (this.api.isOpen(next.sheet)) this.fetch(next)
    }
  }

  // An invalidation arriving while the tile is on its way marks the request
  // (`stale`): the answer may predate the change, so it lands stale and the
  // next frame showing it asks again.
  async fetch ({ sheet, tr, tc, key }) {
    const controller = new AbortController()
    const request = { controller, stale: false }
    this.flight.set(key, request)
    const generation = this.generation
    const held = this.entries.get(key)
    try {
      const answer = await this.api.request('GET', `sheets/${sheet}/tiles/${tr}/${tc}`, {
        etag: held && held.etag,
        signal: controller.signal,
        quiet: true
      })
      if (generation !== this.generation) return
      if (answer.status === 304 && held) {
        held.stale = request.stale
        if (answer.etag) held.etag = answer.etag
      } else {
        const row0 = tr * TILE_ROWS
        const column0 = tc * TILE_COLUMNS
        const cells = new Map()
        for (const item of answer.data.cells || []) {
          const cell = decode(row0, column0, item)
          cells.set(item[0] * TILE_COLUMNS + item[1], cell)
        }
        this.entries.delete(key)
        this.entries.set(key, { sheet, tr, tc, etag: answer.etag, cells, stale: request.stale })
        this.evict()
      }
      this.dispatchEvent(new CustomEvent('tile', { detail: { sheet, tr, tc } }))
    } catch (error) {
      // A cancellation says nothing, nor does a failure for a sheet gone
      // meanwhile (another tab removed it).
      if (error.name !== 'AbortError' && generation === this.generation && this.api.isOpen(sheet)) {
        this.dispatchEvent(new CustomEvent('failed', { detail: { sheet, tr, tc, error } }))
      }
    } finally {
      if (this.flight.get(key) === request) this.flight.delete(key)
      this.pump()
    }
  }

  evict () {
    for (const [key] of this.entries) {
      if (this.entries.size <= CAPACITY) break
      if (!this.wanted.has(key)) this.entries.delete(key)
    }
  }

  // Whether every held tile over `ranges` is current: none stale while shown
  // (the next frame revalidates those) and none in flight. A stale tile no
  // frame shows is left for when one does.
  settled (sheet, ranges) {
    const over = (tr, tc) => {
      const r0 = tr * TILE_ROWS
      const c0 = tc * TILE_COLUMNS
      return ranges.some((area) => !(area.r1 < r0 || area.r0 >= r0 + TILE_ROWS || area.c1 < c0 || area.c0 >= c0 + TILE_COLUMNS))
    }
    for (const key of this.flight.keys()) {
      const [owner, tr, tc] = key.split(':')
      if (owner === String(sheet) && over(Number(tr), Number(tc))) return false
    }
    for (const [key, entry] of this.entries) {
      if (entry.sheet === sheet && entry.stale && this.wanted.has(key) && over(entry.tr, entry.tc)) return false
    }
    return true
  }

  // Mark tiles stale so the next frame showing them revalidates by ETag.
  invalidate (sheet, area) {
    const over = (tr, tc) => {
      if (!area) return true
      const r0 = tr * TILE_ROWS
      const c0 = tc * TILE_COLUMNS
      return !(area.r1 < r0 || area.r0 >= r0 + TILE_ROWS || area.c1 < c0 || area.c0 >= c0 + TILE_COLUMNS)
    }
    for (const entry of this.entries.values()) {
      if (entry.sheet === sheet && over(entry.tr, entry.tc)) entry.stale = true
    }
    for (const [key, request] of this.flight) {
      const [owner, tr, tc] = key.split(':')
      if (owner === String(sheet) && over(Number(tr), Number(tc))) request.stale = true
    }
  }
}
