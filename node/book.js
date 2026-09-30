'use strict'

// The book display as the package ships it: the assets `yggdryl market serve` embeds
// and serves, and a spawner of that command for a Node program that wants the
// display up beside it.
//
// `serve` runs the `yggdryl` binary - `YGGDRYL_BIN`, or the one on the path -
// with `market serve` and the tables, bind, path and captures it is given, and
// resolves once the endpoint the command prints first on its own line has
// been read. The command prints a refusal on stdout in the endpoint's place,
// as a `✗` line after the report of what it warned about, and exits 1; the
// argument parser writes its own on stderr and exits 2. A process that exits
// first rejects with what it printed - that refusal, then its stderr - and
// one that cannot start with the spawn error. Until then the caller holds
// only the promise, so its `signal` ends the process and rejects, and a
// parent that exits first takes the process with it. The display's own state
// lives in `book/app.js`.

const { spawn } = require('node:child_process')
const { join } = require('node:path')
const { createInterface } = require('node:readline')

/** The absolute folder holding the display's files. */
const assets = join(__dirname, 'book')

/** Every file `yggdryl market serve` embeds, in the order the design lists them. */
const assetFiles = Object.freeze([
  'index.html',
  'theme.css',
  'theme.js',
  'api.js',
  'chart.js',
  'audit.js',
  'app.js',
  'favicon.svg',
])

/** The most kept of a refusal and of stderr from a process that failed, in characters each. */
const OUTPUT_LIMIT = 64 * 1024

/**
 * A line of the warning report the command prints ahead of a refusal
 * (`cli/src/warnings.rs`): its `!` count, one `·` note per warning, or one
 * `::warning` workflow command each under `GITHUB_ACTIONS`.
 */
const WARNING = /^(?:! |· |::warning )/

function tableSpec(table) {
  if (typeof table === 'string' && table !== '') return table
  if (table !== null && typeof table === 'object' && typeof table.location === 'string' && table.location !== '') {
    return typeof table.name === 'string' && table.name !== '' ? `${table.name}=${table.location}` : table.location
  }
  throw new TypeError(`a table is 'name=location', a location, or { name, location }, got ${JSON.stringify(table)}`)
}

function listOf(value) {
  if (value === undefined || value === null) return []
  return Array.isArray(value) ? value : [value]
}

/**
 * The argument vector of `yggdryl market serve` for the options `serve` takes, the
 * binary left out: the tables first, then `--bind`, `--path`, one `--capture`
 * per log, then `args` verbatim.
 */
function serveArguments({ tables = [], bind = '127.0.0.1:0', path = '/', capture = [], args = [] } = {}) {
  return [
    'market',
    'serve',
    ...listOf(tables).map(tableSpec),
    '--bind',
    String(bind),
    '--path',
    String(path),
    ...listOf(capture).flatMap((log) => ['--capture', String(log)]),
    ...listOf(args).map(String),
  ]
}

/**
 * Start `yggdryl market serve` and resolve `{ endpoint, process, close }` once it has
 * printed its endpoint: `endpoint` the URL the display answers at, `process`
 * the child, `close()` a promise that ends the process and resolves with its
 * exit status. Options: `tables` (`name=location` strings or `{ name,
 * location }`), `bind` (`127.0.0.1:0`), `path` (`/`), `capture` (FIX bridge
 * logs folded into the first table), `bin` (`YGGDRYL_BIN`, else `yggdryl`),
 * `args` (further command-line arguments), `env`, `signal` (an `AbortSignal`
 * that ends the process whenever it aborts - `AbortSignal.timeout(ms)` bounds
 * the wait for the endpoint). When the process exits before the endpoint, the
 * rejection's message is what it printed: the refusal the command writes on
 * stdout in the endpoint's place - its `✗` line, `✗ invalid record value at
 * $.capture: a capture needs a table to land in: ...`, with any line the
 * refusal runs on to - then its stderr, where the argument parser refuses;
 * a process that printed neither rejects naming its exit status. It rejects
 * with the spawn error when the process cannot start, with the line when its
 * first is neither the endpoint, a refusal nor the warning report ahead of
 * one, and with an `AbortError` quoting what it printed once the process
 * `signal` ended has closed. Until it resolves, the process is ended if this
 * one exits.
 */
function serve(options = {}) {
  const { bin = process.env.YGGDRYL_BIN ?? 'yggdryl', env = process.env, signal } = options
  const argv = serveArguments(options)
  return new Promise((resolve, reject) => {
    const child = spawn(bin, argv, { stdio: ['ignore', 'pipe', 'pipe'], env, windowsHide: true, signal })
    let stderr = ''
    // The `✗` line the command printed in the endpoint's place, and every line after it.
    let refusal = ''
    let settled = false
    let status = null
    // A parent that exits while it waits would leave the child to init, still running.
    const orphaned = () => child.kill()
    process.once('exit', orphaned)
    // `close` rather than `exit`: it fires once the stdio streams have ended,
    // so the refusal and the stderr are whole when the rejection quotes them.
    const exit = new Promise((done) => {
      child.once('close', (code, signal) => {
        process.removeListener('exit', orphaned)
        status = { code, signal }
        done(status)
      })
    })
    const settle = (outcome, value) => {
      if (settled) return
      settled = true
      // Once resolved the caller holds the process and its `close()`.
      if (outcome === resolve) process.removeListener('exit', orphaned)
      outcome(value)
    }
    child.stderr.setEncoding('utf8')
    child.stderr.on('data', (chunk) => {
      if (stderr.length < OUTPUT_LIMIT) stderr += chunk
    })
    const close = () => {
      if (status === null && child.exitCode === null && child.signalCode === null) child.kill()
      return exit
    }
    const lines = createInterface({ input: child.stdout })
    lines.on('line', (line) => {
      if (settled) return
      if (refusal !== '') {
        if (refusal.length < OUTPUT_LIMIT) refusal += `\n${line.trimEnd()}`
        return
      }
      const text = line.trim()
      // The command exits once its refusal is printed, so the rejection waits for the close.
      if (text.startsWith('✗')) {
        refusal = text
        return
      }
      if (text === '' || WARNING.test(text)) return
      let endpoint
      try {
        endpoint = new URL(text).toString()
      } catch (cause) {
        settle(reject, new Error(`expected the endpoint on the first line of yggdryl market serve, got ${JSON.stringify(text)}`, { cause }))
        close()
        return
      }
      lines.removeAllListeners('line')
      child.stdout.resume()
      settle(resolve, { endpoint, process: child, close })
    })
    // An aborted `signal` has ended the process; the rejection waits for its close, so it quotes the whole stderr.
    let cancelled = null
    child.on('error', (cause) => {
      if (cause.name === 'AbortError') {
        cancelled = cause
        return
      }
      process.removeListener('exit', orphaned)
      settle(reject, new Error(`cannot start ${bin}: ${cause.message}`, { cause }))
    })
    exit.then(({ code, signal }) => {
      const printed = [refusal.trim(), stderr.trim()].filter((text) => text !== '').join('\n')
      if (cancelled !== null) {
        const detail = printed === '' ? '' : `: ${printed}`
        const error = new Error(`yggdryl market serve was cancelled before printing its endpoint${detail}`, { cause: cancelled })
        error.name = 'AbortError'
        settle(reject, error)
        return
      }
      if (printed !== '') {
        settle(reject, new Error(printed))
        return
      }
      const reason = code !== null ? `exited with ${code}` : `ended by ${signal}`
      settle(reject, new Error(`yggdryl market serve ${reason} before printing its endpoint`))
    })
  })
}

exports.assets = assets
exports.assetFiles = assetFiles
exports.serveArguments = serveArguments
exports.serve = serve
