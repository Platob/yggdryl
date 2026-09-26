'use strict'

// The browser modules the service shares with the page, imported once: they
// are ES modules (`node/web/package.json` says so) and this package is
// CommonJS, so they cross by dynamic `import()`. The diff and the scenario
// format are theirs; the service calls them rather than keeping a copy.

let loaded

/** `{ ...diff.js, ...scenario.js }`, the one promise every caller awaits. */
function web() {
  loaded ??= Promise.all([import('../web/diff.js'), import('../web/scenario.js')]).then(
    ([diff, scenario]) => Object.freeze({ ...diff, ...scenario }),
  )
  return loaded
}

module.exports = web
