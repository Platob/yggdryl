'use strict'

// Preloaded (`node --require`) by the release smoke step when a test file
// fails: it writes each test's name to stderr as the test starts, and
// synchronously, so a process that dies on a signal names the test it died in.
// The runner's own reporter cannot: it writes on a later turn of the event loop,
// and a file of synchronous tests never gives it one before the crash.

const fs = require('node:fs')
const Module = require('node:module')

function traced(run) {
  const wrapped = function (name, options, fn) {
    const body = [name, options, fn].find((argument) => typeof argument === 'function')
    const title = typeof name === 'string' ? name : body?.name || '<anonymous>'
    const args = [name, options, fn].map((argument) =>
      argument === body
        ? function (...inner) {
            fs.writeSync(2, `> ${title}\n`)
            return Reflect.apply(body, this, inner)
          }
        : argument,
    )
    return Reflect.apply(run, this, args)
  }
  return Object.assign(wrapped, run)
}

const load = Module._load
Module._load = function (request, ...rest) {
  const loaded = Reflect.apply(load, this, [request, ...rest])
  if (request !== 'node:test' && request !== 'test') return loaded
  const test = traced(loaded)
  test.test = test
  for (const name of ['it', 'describe', 'suite']) {
    if (typeof loaded[name] === 'function') test[name] = traced(loaded[name])
  }
  return test
}
