'use strict'

const { readFileSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')

const declarationPath = join(__dirname, '..', 'index.d.ts')
const privateTypes = []

function occurrences(source, needle) {
  let count = 0
  let offset = 0
  while ((offset = source.indexOf(needle, offset)) !== -1) {
    count += 1
    offset += needle.length
  }
  return count
}

function declarationRange(source, className) {
  const marker = `export declare class ${className} {`
  if (occurrences(source, marker) !== 1) {
    throw new Error(`expected exactly one generated ${className} declaration`)
  }
  const classStart = source.indexOf(marker)
  let start = classStart
  const commentStart = source.lastIndexOf('/**', classStart)
  if (commentStart !== -1) {
    const between = source.slice(commentStart, classStart)
    if (/^\/\*\*[\s\S]*\*\/\r?\n$/.test(between)) start = commentStart
  }

  const open = source.indexOf('{', classStart)
  let depth = 0
  let end = -1
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1
    if (source[index] === '}') {
      depth -= 1
      if (depth === 0) {
        end = index + 1
        break
      }
    }
  }
  if (end === -1) throw new Error(`unterminated generated ${className} declaration`)
  if (source[end] === '\r') end += 1
  if (source[end] === '\n') end += 1
  return [start, end]
}

function sanitize(source) {
  for (const [className, alias] of privateTypes) {
    const aliasDeclaration = `export type ${alias} = ${className}`
    if (occurrences(source, aliasDeclaration) !== 1) {
      throw new Error(`expected exactly one generated ${alias} alias`)
    }
    const [start, end] = declarationRange(source, className)
    source = source.slice(0, start) + source.slice(end)

    const aliasStart = source.indexOf(aliasDeclaration)
    let aliasEnd = aliasStart + aliasDeclaration.length
    if (source[aliasEnd] === '\r') aliasEnd += 1
    if (source[aliasEnd] === '\n') aliasEnd += 1
    source = source.slice(0, aliasStart) + source.slice(aliasEnd)
  }
  for (const names of privateTypes) {
    for (const name of names) {
      if (source.includes(name)) {
        throw new Error(`generated declarations still expose private type ${name}`)
      }
    }
  }
  return source
}

function check(source) {
  for (const names of privateTypes) {
    for (const name of names) {
      if (source.includes(name)) {
        throw new Error(`generated declarations expose private type ${name}`)
      }
    }
  }
}

const source = readFileSync(declarationPath, 'utf8')
if (process.argv.includes('--check')) {
  check(source)
} else {
  writeFileSync(declarationPath, sanitize(source))
}
