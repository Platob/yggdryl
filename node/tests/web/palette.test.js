// `node/web/palette.js`: the command palette - a fuzzy filter over command
// labels, a listbox the arrows walk, Enter choosing, the chord beside each.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { fuzzy } from '../../web/palette.js'
import { openPage } from './browser.js'

const COMMANDS = [
  { id: 'transport.toggle', label: 'Play or pause', chord: 'Space' },
  { id: 'transport.grid', label: 'Snapshot grid on or off', chord: 'g' },
  { id: 'scenario.insert', label: 'Insert an event', chord: 'i' },
  { id: 'theme.toggle', label: 'Switch the theme', chord: 't' },
]

test('the filter keeps substring matches first, then subsequences, in list order', () => {
  assert.deepEqual(fuzzy(COMMANDS, '').map((command) => command.id), COMMANDS.map((command) => command.id))
  assert.deepEqual(fuzzy(COMMANDS, 'grid').map((command) => command.id), ['transport.grid'])
  assert.deepEqual(fuzzy(COMMANDS, 'THE').map((command) => command.id), ['theme.toggle'])
  // "ie" is no label's substring, and a subsequence of two: they keep list order.
  assert.deepEqual(fuzzy(COMMANDS, 'ie').map((command) => command.id), ['scenario.insert', 'theme.toggle'])
  // A substring match ranks above a subsequence match wherever it is listed.
  assert.deepEqual(fuzzy(COMMANDS, 'ev').map((command) => command.id), ['scenario.insert'])
  assert.deepEqual(fuzzy([...COMMANDS].reverse(), 'in').map((command) => command.id), ['scenario.insert', 'transport.grid'])
  assert.deepEqual(fuzzy(COMMANDS, 'swt').map((command) => command.id), ['theme.toggle'])
  assert.deepEqual(fuzzy(COMMANDS, 'or').map((command) => command.id), ['transport.toggle', 'transport.grid'])
  assert.deepEqual(fuzzy(COMMANDS, 'zzz'), [])
})

test('the palette filters, walks with the arrows and emits the chosen command after it closes', async () => {
  const { page, close } = await openPage(`
import { Palette } from '/web/palette.js'
const root = document.getElementById('root')
root.innerHTML = '<button id="opener" class="ygg-ui__button">Commands</button>'
window.palette = new Palette({ commands: ${JSON.stringify(COMMANDS)} }).mount(root)
window.commands = []
document.addEventListener('ygg:command', (event) => commands.push([event.detail.id, document.activeElement.id]))
document.getElementById('opener').addEventListener('click', (event) => palette.open(event.currentTarget))
window.options = () => [...palette.el.querySelectorAll('[role=option]')].map((option) => [option.querySelector('.ygg-ui__palette-label').textContent, option.querySelector('kbd')?.textContent ?? '', option.getAttribute('aria-selected')])
`)
  try {
    await page.click('#opener')
    assert.equal(await page.evaluate("document.activeElement.getAttribute('role')"), 'combobox')
    assert.deepEqual(await page.evaluate('options()'), [
      ['Play or pause', 'Space', 'true'],
      ['Snapshot grid on or off', 'g', 'false'],
      ['Insert an event', 'i', 'false'],
      ['Switch the theme', 't', 'false'],
    ])
    const wired = await page.evaluate("(() => { const input = palette.el.querySelector('[role=combobox]'); const list = palette.el.querySelector('[role=listbox]'); return [input.getAttribute('aria-controls') === list.id, document.getElementById(input.getAttribute('aria-activedescendant')).textContent.startsWith('Play')] })()")
    assert.deepEqual(wired, [true, true])
    await page.type('or')
    assert.deepEqual(await page.evaluate('options().map((option) => option[0])'), ['Play or pause', 'Snapshot grid on or off'])
    await page.press('ArrowDown')
    assert.deepEqual(await page.evaluate('options().map((option) => option[2])'), ['false', 'true'])
    await page.press('ArrowDown')
    assert.deepEqual(await page.evaluate('options().map((option) => option[2])'), ['true', 'false'], 'the arrows wrap')
    await page.press('ArrowUp')
    await page.screenshot('palette')
    await page.press('Enter')
    await page.waitFor('commands.length === 1')
    assert.deepEqual(await page.evaluate('commands'), [['transport.grid', 'opener']], 'emitted once focus is back on the opener')
    assert.equal(await page.evaluate('palette.el.open'), false)
    // Reopened, the query is empty again; a click chooses too; nothing matching says so.
    await page.click('#opener')
    assert.equal(await page.evaluate("palette.el.querySelector('[role=combobox]').value"), '')
    await page.click('[role=option]:nth-child(4)')
    await page.waitFor('commands.length === 2')
    assert.equal(await page.evaluate('commands[1][0]'), 'theme.toggle')
    await page.click('#opener')
    await page.type('zzz')
    assert.equal(await page.evaluate("palette.el.querySelector('.ygg-ui__palette-empty').hidden"), false)
    await page.press('Enter')
    assert.equal(await page.evaluate('palette.el.open'), true, 'Enter on no match chooses nothing')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
