'use strict'

// Post-build patch for the napi-generated `index.d.ts` (wired into `npm run build` right
// after `napi build`, so a plain rebuild keeps the file correct).
//
// napi-rs emits every class/enum reference **unqualified**, even when the referenced type
// is declared in a *different* `export declare namespace` block — e.g. `local.LocalIO.seek`
// takes a `Whence`, but `Whence` is declared in `namespace memory`, so the reference does
// not resolve and the typing silently degrades. This script:
//
//   1. scans `index.d.ts` for its `export declare namespace X { ... }` blocks and collects
//      which type names each namespace (and the top level) declares;
//   2. inside each namespace block, qualifies every reference to a name that is declared in
//      exactly one *other* namespace (and neither in the referencing namespace nor at the
//      top level) — `Whence` → `memory.Whence`, `Uri` → `uri.Uri`, `Headers` →
//      `headers.Headers`, `IOMode`/`IOKind` → `io.IOMode`/`io.IOKind`, and any future case
//      the same way, with no hard-coded list.
//
// Comment lines and enum bodies are left untouched (doc prose keeps its plain names, and an
// enum member such as `IOKind.Heap` *declares* a name rather than referencing one), and
// already-qualified references are skipped (the `(?<![.\w$])` guard), so the script is
// idempotent.

const fs = require('node:fs')
const path = require('node:path')

const dtsPath = path.join(__dirname, '..', 'index.d.ts')
const source = fs.readFileSync(dtsPath, 'utf8')
const lines = source.split('\n')

// A namespace opens at column 0 and closes at the next column-0 `}` (they never nest).
const NAMESPACE_OPEN = /^export declare namespace (\w+) \{/
// A declaration introduces a type name at the top level or directly inside a namespace.
const DECLARATION =
  /^\s*export (?:declare )?(?:const enum|abstract class|class|interface|function|enum|type|const|let|var) (\w+)/
// An enum body declares member names (no type references) — never rewrite inside one.
const ENUM_OPEN = /^\s*export (?:declare )?(?:const )?enum \w+ \{/

// ---- pass 1: which names does each scope declare? -----------------------------------

/** @type {Map<string, Set<string>>} namespace name -> type names it declares */
const declared = new Map()
/** @type {Set<string>} names declared at the module's top level (visible everywhere) */
const topLevel = new Set()

let current = null // the namespace name while inside its block
for (const line of lines) {
  const open = line.match(NAMESPACE_OPEN)
  if (open) {
    current = open[1]
    if (!declared.has(current)) declared.set(current, new Set())
    continue
  }
  if (current !== null && line === '}') {
    current = null
    continue
  }
  const decl = line.match(DECLARATION)
  if (decl) {
    if (current === null) topLevel.add(decl[1])
    else declared.get(current).add(decl[1])
  }
}

// A name is qualifiable when exactly one namespace declares it and the top level does not
// (an ambiguous or top-level name resolves — or must be resolved — some other way).
/** @type {Map<string, string>} type name -> the one namespace that declares it */
const owner = new Map()
for (const [ns, names] of declared) {
  for (const name of names) {
    if (topLevel.has(name)) continue
    owner.set(name, owner.has(name) ? null : ns) // null marks an ambiguous name
  }
}
for (const [name, ns] of owner) if (ns === null) owner.delete(name)

// ---- pass 2: qualify cross-namespace references -------------------------------------

let replaced = 0
let inComment = false
let inEnum = false
current = null
const fixed = lines.map((line) => {
  const open = line.match(NAMESPACE_OPEN)
  if (open) {
    current = open[1]
    return line
  }
  if (current !== null && line === '}') {
    current = null
    return line
  }
  // Track (and skip) comment lines — doc prose keeps its plain names.
  const trimmed = line.trim()
  if (inComment) {
    if (trimmed.includes('*/')) inComment = false
    return line
  }
  if (trimmed.startsWith('/*')) {
    if (!trimmed.includes('*/')) inComment = true
    return line
  }
  if (trimmed.startsWith('//') || current === null) return line
  // Track (and skip) enum bodies — a member such as `Heap = 3` declares, not references.
  if (inEnum) {
    if (trimmed === '}') inEnum = false
    return line
  }
  if (ENUM_OPEN.test(line)) {
    inEnum = true
    return line
  }

  let out = line
  for (const [name, ns] of owner) {
    if (ns === current || declared.get(current).has(name)) continue
    const reference = new RegExp(`(?<![.\\w$])${name}(?![\\w$])`, 'g')
    out = out.replace(reference, () => {
      replaced += 1
      return `${ns}.${name}`
    })
  }
  return out
})

const result = fixed.join('\n')
if (result !== source) {
  fs.writeFileSync(dtsPath, result)
  console.log(`fix-dts: qualified ${replaced} cross-namespace reference(s) in index.d.ts`)
} else {
  console.log('fix-dts: index.d.ts already fully qualified')
}
