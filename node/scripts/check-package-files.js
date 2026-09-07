'use strict'

const { spawnSync } = require('node:child_process')
const { existsSync, readdirSync } = require('node:fs')
const { dirname, join } = require('node:path')

const root = join(__dirname, '..')
const npmCli = process.env.npm_execpath ?? join(
  dirname(process.execPath),
  'node_modules',
  'npm',
  'bin',
  'npm-cli.js',
)
if (!existsSync(npmCli)) {
  throw new Error('cannot locate npm-cli.js for the package dry-run')
}
const result = spawnSync(process.execPath, [npmCli, 'pack', '--dry-run', '--json'], {
  cwd: root,
  encoding: 'utf8',
})

if (result.status !== 0) {
  process.stderr.write(result.stderr ?? `${result.error}\n`)
  process.exit(result.status ?? 1)
}

// npm answers `pack --json` in two envelopes, and the release job installs
// whatever `npm@latest` is on the day it runs: through npm 11 `pack` buffers
// each tarball under its array index and prints `[report]`; from npm 12 it
// keys by package name and prints `{ "<name>": report }`. The report inside
// is the same object either way. Reading only the array left `report`
// undefined under npm 12, and the audit died on a TypeError reading `.files`.
let packed
try {
  packed = JSON.parse(result.stdout)
} catch (cause) {
  throw new Error('npm pack --dry-run did not return its JSON report', { cause })
}

// Chosen by shape rather than by position: npm merges an `error` key into the
// object envelope beside the reports, and that key has no `files`.
const report = (Array.isArray(packed) ? packed : Object.values(packed ?? {}))
  .find((entry) => entry && Array.isArray(entry.files))
if (!report) {
  throw new Error(
    `npm pack --dry-run named no packed files: ${result.stdout.slice(0, 400)}`,
  )
}

const files = new Set(report.files.map(({ path }) => path.replaceAll('\\', '/')))
for (const required of [
  'binding.js',
  'binding.d.ts',
  'defaults.js',
  'fields.js',
  'index.d.ts',
  'index.js',
  'records.js',
  'values.js',
]) {
  if (!files.has(required)) {
    throw new Error(`npm package is missing required runtime file ${required}`)
  }
}

const nativeFiles = readdirSync(root).filter((path) => path.endsWith('.node'))
if (nativeFiles.length === 0) {
  throw new Error('npm package audit found no native module in the package root')
}
for (const nativeFile of nativeFiles) {
  if (!files.has(nativeFile)) {
    throw new Error(`npm package excludes native module ${nativeFile}`)
  }
}

console.log(
  `package dry-run: ${report.files.length} files, ${nativeFiles.length} native module(s), shasum ${report.shasum}`,
)
