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

let report
try {
  const packed = JSON.parse(result.stdout)
  // npm answers with a list of packed packages up to npm 11 and with an object
  // keyed by package name from npm 12. The release job installs the newest npm
  // to reach trusted publishing while a checkout uses the one Node bundles, so
  // both shapes are read rather than the one this machine happens to have.
  report = Array.isArray(packed) ? packed[0] : Object.values(packed)[0]
} catch (cause) {
  throw new Error('npm pack --dry-run did not return its JSON report', { cause })
}
if (report?.files === undefined) {
  throw new Error(
    `npm pack --dry-run reported no files: ${result.stdout.slice(0, 200)}`,
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
