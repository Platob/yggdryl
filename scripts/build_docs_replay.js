'use strict'

/**
 * Generate the replay page's manifest from the real package, and copy the
 * component and the page beside it.
 *
 * The replay service is Node over a native addon, so a documentation page
 * cannot run it. What the page shows is therefore recorded here: this runs
 * `yggdryl/replay` over its two fixed sources - the synthetic scenario, and
 * the ULBridge capture the Rust suite pins, read from its bytes under the
 * committed dictionary and the seed clock - and writes what each route would
 * answer: the sources listing, and every book of the walk per symbol and
 * consolidated. `docs/assets/replay.js` mounts the page over these answers
 * and computes nothing of its own; `calls` holds, beside the books, the
 * JavaScript and the route that give them.
 *
 * The component and the page are the package's own files, so they are
 * copied, never rewritten: `docs/assets/web/` holds what the package ships
 * under `web/` - `*.js`, `*.css`, `package.json` and `app/*` - byte for
 * byte, and a copy whose source is gone is drift, removed on regeneration.
 *
 * Everything is committed, so the same build runs on any machine: fixed
 * sources, fixed instants, fixed key order, two-space JSON, LF, bigints as
 * decimal text, no timestamps and no paths. `--check` proves the tree still
 * matches what a regeneration writes, comparing the text rather than the
 * endings a checkout imposed on it, and the capture's line identities by the
 * part the capture decides (`comparable`).
 *
 * Usage:
 *     node scripts/build_docs_replay.js            # regenerate
 *     node scripts/build_docs_replay.js --check    # report drift, write nothing
 */

const fs = require('node:fs')
const path = require('node:path')

const { graph } = require('../node/binding.js')
const { booksBetween, booksJson, createReplayServer, indexBooks, loadSource, toJson, walk } = require('../node/replay.js')

const ROOT = path.join(__dirname, '..')
const ASSETS = path.join(ROOT, 'docs', 'assets')
const MANIFEST = path.join(ASSETS, 'replay.json')
const WEB = path.join(ROOT, 'node', 'web')
const COPIES = path.join(ASSETS, 'web')
const VERSION = require('../node/package.json').version

// The capture the Rust suite pins (`rust/tests/fix/ulbridge.rs`): read from its
// bytes under the committed dictionary, an undated line taking the seed clock.
const CAPTURE = 'rust/tests/fix/ulbridge.log'
const REGISTRY = 'config/fix'
const SENDING = '2024-01-02T10:15:30Z'

/** What the service itself answers at `GET /api/sources`, served over both sources on an ephemeral port. */
async function listing(sources) {
  const service = createReplayServer({ sources })
  try {
    const url = await service.listen(0)
    const answer = await fetch(new URL('/api/sources', url))
    if (!answer.ok) throw new Error(`/api/sources: ${answer.status} ${await answer.text()}`)
    return answer.json()
  } finally {
    await service.close()
  }
}

/**
 * Every walk a request can name without a grid: the per-symbol walk's books of
 * each symbol, and the consolidated walk's `GLOBAL` books, each list in walk
 * order - what `GET .../books?symbol=` streams.
 */
function booksBySymbol(operations, symbols) {
  const index = indexBooks(walk(operations))
  const books = {}
  for (const symbol of symbols) books[symbol] = booksJson(booksBetween(index, symbol))
  books[graph.GLOBAL_SYMBOL] = booksJson(walk(operations, { global: true }))
  return books
}

/** One source's recorded books, and the JavaScript and the route that answer them. */
function recorded(source, load) {
  return {
    books: booksBySymbol(source.operations, source.symbols),
    calls: {
      books: {
        call: `${load}\nbooksJson(booksBetween(indexBooks(walk(source.operations)), symbol))`,
        route: `GET /api/sources/${source.id}/books?symbol=<symbol>`,
      },
    },
  }
}

/** Build the whole manifest, in the order it is written. */
async function manifest() {
  const synthetic = loadSource('synthetic')
  const ulbridge = loadSource(path.join(ROOT, CAPTURE), {
    registry: path.join(ROOT, REGISTRY),
    sendingTime: SENDING,
    id: 'ulbridge',
  })
  return {
    version: VERSION,
    // `GET /api/sources`: the sources and their symbols.
    listing: await listing([synthetic, ulbridge]),
    sources: {
      synthetic: recorded(synthetic, "const source = loadSource('synthetic')"),
      ulbridge: recorded(
        ulbridge,
        `const source = loadSource('${CAPTURE}', { registry: '${REGISTRY}', sendingTime: '${SENDING}', id: 'ulbridge' })`,
      ),
    },
  }
}

/** What the package ships under `web/`: `*.js`, `*.css`, `package.json` and `app/*`, by path under it. */
function shipped() {
  const files = []
  const list = (folder) => {
    try {
      return fs.readdirSync(folder, { withFileTypes: true }).filter((entry) => entry.isFile())
    } catch (error) {
      if (error.code === 'ENOENT') return []
      throw error
    }
  }
  for (const entry of list(WEB)) {
    if (/\.(js|css)$/.test(entry.name) || entry.name === 'package.json') files.push(entry.name)
  }
  for (const entry of list(path.join(WEB, 'app'))) files.push(`app/${entry.name}`)
  return files.sort()
}

/** Every file under `folder`, as paths relative to it with `/` separators. */
function present(folder, prefix = '') {
  let entries
  try {
    entries = fs.readdirSync(folder, { withFileTypes: true })
  } catch (error) {
    if (error.code === 'ENOENT') return []
    throw error
  }
  return entries.flatMap((entry) =>
    entry.isDirectory()
      ? present(path.join(folder, entry.name), `${prefix}${entry.name}/`)
      : [`${prefix}${entry.name}`],
  )
}

const TEXT = /\.(js|css|html|json|svg|txt|md)$/

/** Report the first line two renderings differ on, so drift names itself. */
function difference(target, current, wanted) {
  if (current === null) return `${path.relative(ROOT, target)} is missing`
  const was = current.split('\n')
  const now = wanted.split('\n')
  for (let line = 0; line < Math.max(was.length, now.length); line += 1) {
    if (was[line] === now[line]) continue
    return (
      `${path.relative(ROOT, target)} is out of date at line ${line + 1}:\n` +
      `  committed: ${(was[line] ?? '<end of file>').slice(0, 160)}\n` +
      `  generated: ${(now[line] ?? '<end of file>').slice(0, 160)}`
    )
  }
  return `${path.relative(ROOT, target)} is out of date`
}

/**
 * The one answer that is not the same on every read: a capture is read into
 * an anonymous buffer the package names `mem://<process>/<address>`, a text
 * line's identity is seeded by that name, and an operation's `srcuuids` are
 * the identities of the lines it was read from. Their millisecond and row
 * sequence (`00000000-0000-7xxx`) are the capture's; the 64 bits after them
 * are the buffer's. A comparison reads the first part and leaves the second,
 * so the committed record is one real read and a new read of the same bytes
 * is not drift.
 */
const BUFFER_SEEDED = /"(00000000-0000-7[0-9a-f]{3})-[0-9a-f]{4}-[0-9a-f]{12}"/g

/** Text as the drift check compares it. */
function comparable(text) {
  return text.replace(BUFFER_SEEDED, '"$1-<buffer>"')
}

/** Write one text file, or report its drift; answers whether it was current. */
function settle(target, wanted, check, compare = (text) => text) {
  // A checkout under `core.autocrlf` materialises the committed LF file as
  // CRLF, which is not drift, so compare the text and keep the file's endings.
  const raw = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : null
  const current = raw === null ? null : raw.replace(/\r\n/g, '\n')
  if (current !== null && compare(current) === compare(wanted)) return true
  if (check) {
    console.log(`  ${difference(target, current === null ? null : compare(current), compare(wanted))}`)
    return false
  }
  fs.mkdirSync(path.dirname(target), { recursive: true })
  const eol = raw !== null && raw.includes('\r\n') ? '\r\n' : '\n'
  fs.writeFileSync(target, wanted.replace(/\n/g, eol))
  return true
}

/** Copy one file that is not text byte for byte, or report its drift. */
function settleBytes(target, wanted, check) {
  const current = fs.existsSync(target) ? fs.readFileSync(target) : null
  if (current !== null && current.equals(wanted)) return true
  if (check) {
    console.log(`  ${path.relative(ROOT, target)} ${current === null ? 'is missing' : 'is out of date'}`)
    return false
  }
  fs.mkdirSync(path.dirname(target), { recursive: true })
  fs.writeFileSync(target, wanted)
  return true
}

/** Copy the component and the page; a copy with no source is drift. */
function settleCopies(check) {
  const files = shipped()
  let current = true
  for (const name of files) {
    const source = path.join(WEB, ...name.split('/'))
    const target = path.join(COPIES, ...name.split('/'))
    let settled
    if (TEXT.test(name)) {
      const text = fs.readFileSync(source, 'utf8').replace(/\r\n/g, '\n')
      settled = settle(target, text, check)
    } else {
      settled = settleBytes(target, fs.readFileSync(source), check)
    }
    current = settled && current
  }
  const wanted = new Set(files)
  for (const name of present(COPIES)) {
    if (wanted.has(name)) continue
    const target = path.join(COPIES, ...name.split('/'))
    if (check) {
      console.log(`  ${path.relative(ROOT, target)} has no source in node/web`)
      current = false
    } else {
      fs.rmSync(target)
    }
  }
  return { current, count: files.length }
}

async function main(argv) {
  const check = argv.includes('--check')
  const built = await manifest()
  const { synthetic: made, ulbridge } = built.sources
  const count = (books) => Object.values(books).reduce((total, list) => total + list.length, 0)
  const current = settle(MANIFEST, `${toJson(built, 2)}\n`, check, comparable)
  const copies = settleCopies(check)
  console.log(
    `replay: synthetic ${count(made.books)} books, ulbridge ${count(ulbridge.books)} books; ` +
      `${copies.count} web files${check ? ' checked' : ' generated'}`,
  )
  return current && copies.current ? 0 : 1
}

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code
  },
  (error) => {
    console.error(error)
    process.exitCode = 1
  },
)
