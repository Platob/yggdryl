// The change feed (§8.3 `changes`, §8.6): what other tabs, and other
// browsers, did to the workbook reaches this tab without a request of its
// own. One tab per browser leads: it holds the Web Lock and the one long
// poll (`GET changes?since=N&wait=25`) and relays every answer over
// `BroadcastChannel("yggdryl-excel")`; the others follow and never poll.
// When the leader goes, the lock passes to a follower, which polls from
// the revision it has.
//
// Every tab absorbs a relayed answer the same way, the leader included:
// the changes past the revision it already holds, in order, each
// invalidating the tiles it names (the whole sheet when structural), the
// style table or the sheet list; a `reset` reloads everything. A follower
// that finds a gap between its revision and the relayed `since` reads the
// ring once from its own revision (`wait=0`) instead.

import { ApiError } from './api.js'

export const CHANNEL = 'yggdryl-excel'
export const LOCK = 'yggdryl-changes'
export const WAIT = 25

const FIRST_PAUSE = 1000
const MAX_PAUSE = 30000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export class Sync extends EventTarget {
  // `apply(answer, from)` absorbs a `changes` answer's changes past
  // revision `from`; `reset()` reloads the workbook.
  constructor ({ api, apply, reset }) {
    super()
    this.api = api
    this.apply = apply
    this.reset = reset
    // Messages are tagged with the API they came from: two services on one
    // origin never absorb each other's changes.
    this.base = String(api.base)
    this.chain = Promise.resolve()
    this.channel = null
    this.controller = null
    this.stopped = false
    this.state = 'idle'
  }

  // `data-sync` on the document says which this tab is: leader, follower,
  // alone (no Web Locks here), stopped.
  mark (state) {
    this.state = state
    document.documentElement.dataset.sync = state
  }

  start () {
    if (this.controller) return
    this.stopped = false
    this.controller = new AbortController()
    window.addEventListener('pagehide', () => this.stop(), { once: true })
    // A page restored from the back-forward cache takes its place again; a
    // gap to the relayed answers is read once, as for any follower.
    if (!this.resumes) {
      this.resumes = true
      window.addEventListener('pageshow', (event) => {
        if (event.persisted && this.state === 'stopped' && !this.halted) this.start()
      })
    }
    const relay = typeof BroadcastChannel === 'function'
    const locks = relay && navigator.locks && typeof navigator.locks.request === 'function'
    if (!locks) {
      // Nothing to share the poll with: this tab polls for itself.
      this.lead('alone')
      return
    }
    this.channel = new BroadcastChannel(CHANNEL)
    this.channel.addEventListener('message', (event) => this.receive(event.data))
    this.mark('follower')
    // The lock is held for as long as the returned promise is pending: for
    // the life of this page, or until the service stops answering.
    navigator.locks.request(LOCK + ' ' + new URL(this.base).pathname, { signal: this.controller.signal }, () => this.lead('leader'))
      .catch(() => {})
  }

  stop () {
    this.stopped = true
    if (this.controller) this.controller.abort()
    if (this.channel) this.channel.close()
    this.channel = null
    this.controller = null
    this.mark('stopped')
  }

  // The long poll, from the revision this tab holds. Each answer goes to
  // the followers first, then to this tab.
  async lead (state) {
    this.mark(state)
    let generation = this.api.generation
    let since = this.api.revision
    let pause = 0
    while (!this.stopped) {
      // A workbook opened, made or uploaded since the last poll (a relayed
      // reset, or this tab's own reload) numbers its revisions afresh: the
      // poll starts again from the revision this tab now holds.
      if (this.api.generation !== generation) {
        generation = this.api.generation
        since = this.api.revision
      }
      let answer
      try {
        answer = await this.api.get(`changes?since=${since}&wait=${WAIT}`, { quiet: true, signal: this.controller && this.controller.signal })
        pause = 0
      } catch (error) {
        if (this.stopped || (error && error.name === 'AbortError')) return
        // The service stopped, or this tab may not read it: nothing a
        // retry changes. The lock is let go by returning.
        if (error instanceof ApiError && (error.status === 401 || error.status === 403 || error.status === 503 || error.kind === 'closed')) {
          this.halt(error)
          return
        }
        pause = Math.min(MAX_PAUSE, pause ? pause * 2 : FIRST_PAUSE)
        await sleep(pause)
        continue
      }
      if (!answer || typeof answer.revision !== 'number') {
        await sleep(FIRST_PAUSE)
        continue
      }
      const message = { base: this.base, generation, since, ...answer }
      if (this.channel) this.channel.postMessage(message)
      await this.receive(message)
      // A reset absorbed moved this tab to another generation; the loop's
      // head takes its revision.
      if (this.api.generation === generation) since = answer.revision
    }
  }

  halt (error) {
    this.stopped = true
    this.halted = true
    this.mark('stopped')
    this.dispatchEvent(new CustomEvent('halted', { detail: error }))
  }

  // Answers are absorbed one at a time, in the order they arrived.
  receive (message) {
    const run = () => this.absorb(message)
    this.chain = this.chain.then(run, run).catch(() => {})
    return this.chain
  }

  async absorb (message) {
    if (!message || message.base !== this.base || this.stopped) return
    const api = this.api
    // A leader still on a generation this tab has left says nothing new;
    // one on a later generation means this tab missed the reset.
    if (message.generation !== api.generation) {
      if (message.generation > api.generation) await this.reset()
      return
    }
    if (message.reset) {
      await this.reset()
      return
    }
    const from = api.revision
    if (message.revision <= from) return
    if (message.since > from) {
      // Changes this tab never saw lie before the relayed ones.
      let answer
      try {
        answer = await api.get(`changes?since=${from}&wait=0`, { quiet: true })
      } catch {
        return
      }
      if (answer.reset) await this.reset()
      else await this.apply(answer, from)
      return
    }
    await this.apply(message, from)
    this.dispatchEvent(new CustomEvent('absorbed', { detail: message.revision }))
  }
}
