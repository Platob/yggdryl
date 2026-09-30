'use strict'

const { spawnSync } = require('node:child_process')
const { existsSync, readdirSync, readFileSync } = require('node:fs')
const { dirname, join } = require('node:path')

const root = join(__dirname, '..')
const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
// Under `npm run` the CLI is named by the environment; run bare, it is the one
// bundled beside the node binary (Windows) or under its `lib` (Unix).
const npmCli = process.env.npm_execpath ?? [
  join(dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js'),
  join(dirname(process.execPath), '..', 'lib', 'node_modules', 'npm', 'bin', 'npm-cli.js'),
].find((candidate) => existsSync(candidate))
if (npmCli === undefined || !existsSync(npmCli)) {
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
const required = new Set([
  'defaults.js',
  'fields.js',
  'index.d.ts',
  'index.js',
  'records.js',
  'values.js',
  manifest.main,
  manifest.types,
])
// TypeScript takes the first condition of an entry it recognizes, so an entry
// whose `require` came first would never reach its declarations.
for (const [entry, conditions] of Object.entries(manifest.exports)) {
  const names = typeof conditions === 'object' && conditions !== null ? Object.keys(conditions) : []
  if (names[0] !== 'types') {
    throw new Error(`exports['${entry}'] must lead with a types condition, got ${JSON.stringify(conditions)}`)
  }
  for (const target of Object.values(conditions)) required.add(target.replace(/^\.\//, ''))
}
// The book display's ES modules, and the `"type": "module"` marker that makes them ES modules.
const book = require('../book.js')
for (const name of [...book.assetFiles, 'package.json']) required.add(`book/${name}`)
for (const path of required) {
  if (!files.has(path)) {
    throw new Error(`npm package is missing required file ${path}`)
  }
}

// `book.d.ts` is written by hand beside `book.js`, so it is held to the values
// the module exports: one undeclared is `any` to a consumer, one declared and
// absent a runtime `undefined` the compiler vouched for.
const ts = require('typescript')
const bookDeclarations = join(root, 'book.d.ts')
const program = ts.createProgram([bookDeclarations], {
  module: ts.ModuleKind.Node16,
  moduleResolution: ts.ModuleResolutionKind.Node16,
  noEmit: true,
  strict: true,
  target: ts.ScriptTarget.ES2022,
})
const checker = program.getTypeChecker()
const declared = checker
  .getExportsOfModule(checker.getSymbolAtLocation(program.getSourceFile(bookDeclarations)))
  .filter((symbol) => symbol.flags & ts.SymbolFlags.Value)
  .map(({ name }) => name)
  .sort()
const exported = Object.keys(book).sort()
if (JSON.stringify(declared) !== JSON.stringify(exported)) {
  throw new Error(
    `book.d.ts declares ${JSON.stringify(declared)} but book.js exports ${JSON.stringify(exported)}`,
  )
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
  `package dry-run: ${report.files.length} files, ${nativeFiles.length} native module(s), ` +
    `shasum ${report.shasum}`,
)
