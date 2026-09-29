'use strict'

// The book display as the package ships it: the assets `yggdryl market serve` embeds
// and serves, and a spawner of that command for a Node program that wants the
// display up beside it.
//
// `serve` runs the `yggdryl` binary - `YGGDRYL_BIN`, or the one on the path -
// with `serve` and the tables, bind, path and captures it is given, and
// resolves once the endpoint the command prints first on its own line has
// been read; a process that exits first, or cannot start, rejects with what
// it wrote on stderr. The display's own state lives in `book/app.js`.

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

/** The most stderr kept from a process that failed, in bytes. */
const STDERR_LIMIT = 64 * 1024

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
 * `args` (further command-line arguments), `env`. Rejects with the process's
 * stderr when it exits before the endpoint, or with the spawn error when it
 * cannot start.
 */
function serve(options = {}) {
  const { bin = process.env.YGGDRYL_BIN ?? 'yggdryl', env = process.env } = options
  const argv = serveArguments(options)
  return new Promise((resolve, reject) => {
    const child = spawn(bin, argv, { stdio: ['ignore', 'pipe', 'pipe'], env, windowsHide: true })
    let stderr = ''
    let settled = false
    let status = null
    // `close` rather than `exit`: it fires once the stdio streams have ended,
    // so the stderr a refusal wrote is whole when the rejection quotes it.
    const exit = new Promise((done) => {
      child.once('close', (code, signal) => {
        status = { code, signal }
        done(status)
      })
    })
    const settle = (outcome, value) => {
      if (settled) return
      settled = true
      outcome(value)
    }
    child.stderr.setEncoding('utf8')
    child.stderr.on('data', (chunk) => {
      if (stderr.length < STDERR_LIMIT) stderr += chunk
    })
    const close = () => {
      if (status === null && child.exitCode === null && child.signalCode === null) child.kill()
      return exit
    }
    const lines = createInterface({ input: child.stdout })
    lines.on('line', (line) => {
      if (settled) return
      const text = line.trim()
      if (text === '') return
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
    child.once('error', (cause) => {
      settle(reject, new Error(`cannot start ${bin}: ${cause.message}`, { cause }))
    })
    exit.then(({ code, signal }) => {
      const reason = code !== null ? `exited with ${code}` : `ended by ${signal}`
      const detail = stderr.trim()
      settle(reject, new Error(`yggdryl market serve ${reason} before printing its endpoint${detail ? `: ${detail}` : ''}`))
    })
  })
}

exports.assets = assets
exports.assetFiles = assetFiles
exports.serveArguments = serveArguments
exports.serve = serve
