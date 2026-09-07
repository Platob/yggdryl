'use strict'

/**
 * Copy the published browser UI into the documentation asset tree.
 *
 * MkDocs only serves files below `docs/`, while `node/ui/` is the package's
 * single source of truth. These committed copies keep the documentation on
 * that exact implementation without adding a page-script build step.
 *
 * Usage:
 *     node scripts/build_docs_ui.js            # regenerate
 *     node scripts/build_docs_ui.js --check    # report drift, write nothing
 */

const fs = require('node:fs')
const path = require('node:path')

const ROOT = path.join(__dirname, '..')
const SOURCE = path.join(ROOT, 'node', 'ui')
const TARGET = path.join(ROOT, 'docs', 'assets', 'ui')
const FILES = [
  { source: 'fix.mjs', target: 'fix.js' },
  { source: 'index.mjs', target: 'index.js' },
  { source: 'yggdryl.mjs', target: 'yggdryl.js' },
  { source: 'styles.css', target: 'styles.css' },
]

/**
 * Python's stock static server labels `.mjs` as text/plain on supported
 * Windows installs, which browsers correctly reject as a module. The package
 * remains explicit ESM; its docs copy uses `.js` under `type="module"` and
 * rewrites relative module edges to the served names.
 */
function browserSource(name, text) {
  if (!name.endsWith('.mjs')) return text
  const rewritten = text.replace(/(['"]\.\/[^'"]+)\.mjs(['"])/g, '$1.js$2')
  if (/['"]\.\/[^'"]+\.mjs['"]/.test(rewritten)) {
    throw new Error(`${name} has a relative module import the docs bridge did not rewrite`)
  }
  return rewritten
}

/** Compare source text independently of checkout newline materialization. */
const normalized = (text) => text.replace(/\r\n/g, '\n')

/** Report the first line whose text differs. */
function difference(target, current, wanted) {
  if (current === null) return `${path.relative(ROOT, target)} is missing`
  const was = current.split('\n')
  const now = wanted.split('\n')
  for (let line = 0; line < Math.max(was.length, now.length); line += 1) {
    if (was[line] === now[line]) continue
    return (
      `${path.relative(ROOT, target)} is out of date at line ${line + 1}:\n` +
      `  committed: ${(was[line] ?? '<end of file>').slice(0, 160)}\n` +
      `  source:    ${(now[line] ?? '<end of file>').slice(0, 160)}`
    )
  }
  return `${path.relative(ROOT, target)} is out of date`
}

/** Copy one file, or report whether its committed copy is current. */
function settle(file, check) {
  const source = path.join(SOURCE, file.source)
  const target = path.join(TARGET, file.target)
  let sourceRaw
  try {
    sourceRaw = fs.readFileSync(source, 'utf8')
  } catch (error) {
    if (error.code === 'ENOENT') {
      console.log(`  ${path.relative(ROOT, source)} is missing`)
      return false
    }
    throw error
  }

  const wanted = normalized(browserSource(file.source, sourceRaw))
  const targetRaw = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : null
  const current = targetRaw === null ? null : normalized(targetRaw)
  if (current === wanted) return true
  if (check) {
    console.log(`  ${difference(target, current, wanted)}`)
    return false
  }

  fs.mkdirSync(path.dirname(target), { recursive: true })
  const eol =
    (targetRaw !== null && targetRaw.includes('\r\n')) ||
    (targetRaw === null && sourceRaw.includes('\r\n'))
      ? '\r\n'
      : '\n'
  fs.writeFileSync(target, wanted.replace(/\n/g, eol))
  return true
}

function main(argv) {
  const check = argv.includes('--check')
  const current = FILES.map((file) => settle(file, check)).every(Boolean)
  console.log(`ui assets: ${FILES.length} files ${check ? 'checked' : 'generated'}`)
  return current ? 0 : 1
}

process.exitCode = main(process.argv.slice(2))
