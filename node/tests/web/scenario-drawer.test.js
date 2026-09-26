// `node/web/scenario-drawer.js`: the saved scenarios and the active one's
// events, each change emitted as a command for the app to carry out.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { ScenarioDrawer } from '/web/scenario-drawer.js'
const saved = await (await fetch('/fixtures/scenarios.json')).json()
window.saved = saved
window.commands = []
document.addEventListener('ygg:scenario', (event) => commands.push(event.detail))
window.drawer = new ScenarioDrawer().mount(document.body)
drawer.update(saved)
drawer.open()
window.q = (selector) => drawer.el.querySelector(selector)
window.names = () => [...drawer.el.querySelectorAll('.ygg-ui__scenario-name')].map((node) => [node.textContent, node.getAttribute('aria-current')])
`

test('the drawer lists the scenarios and the active one events, and emits each change', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.frames()
    assert.deepEqual(await page.evaluate('names()'), [
      ['spike (2 events)', 'true'],
      ['empty (0 events)', 'false'],
    ])
    assert.deepEqual(await page.evaluate("[...drawer.el.querySelectorAll('.ygg-ui__scenario-event')].map((row) => row.querySelector('.ygg-ui__scenario-when').textContent + ' ' + row.querySelector('.ygg-ui__scenario-kind').textContent)"), [
      '2023-11-14T22:13:21.200000000Z order_event',
      '2023-11-14T22:13:22.200000000Z execution_event',
    ])
    // An event is the leaf's served row: every fact it states, in column order,
    // and never its native text; an empty map or a null states nothing.
    const facts = await page.evaluate("[...drawer.el.querySelectorAll('.ygg-ui__scenario-event')[1].querySelectorAll('dt')].map((term) => [term.textContent, term.nextElementSibling.textContent])")
    assert.deepEqual(facts, [
      ['curruuid', '0192f0a0-0000-7000-8000-900000000002'],
      ['crossuuid', '0192f0a0-0000-7000-8000-c00000000002'],
      ['crosscode', 'ALPHA-I-2'],
      ['currhashcode', '1000000000002'],
      ['state', '80FILLED'],
      ['currency', 'USD'],
      ['side', 'SELL'],
      ['lastpx', '98.5'],
      ['lastqty', '10'],
      ['ticker', 'ALPHA'],
      ['stableHash', '1000000000002'],
    ])
    await page.evaluate("drawer.update({ ...saved, scenarios: [{ ...saved.scenarios[0], events: [{ ...saved.scenarios[0].events[0], metadata: [['tech.clientid', 'C-7'], ['31027', 'X']] }] }] })")
    await page.frames()
    assert.equal(await page.evaluate("[...drawer.el.querySelectorAll('.ygg-ui__scenario-event dt')].find((term) => term.textContent === 'metadata').nextElementSibling.textContent"), 'tech.clientid=C-7, 31027=X')
    await page.evaluate('drawer.update(saved)')
    await page.frames()
    await page.screenshot('scenario-drawer')
    await page.click('.ygg-ui__scenario-event:nth-child(2) button[aria-label^="Remove"]')
    assert.deepEqual(await page.evaluate('commands.at(-1)'), { command: 'remove-event', name: 'spike', curruuid: '0192f0a0-0000-7000-8000-900000000002' })
    await page.click('.ygg-ui__scenario-list li:nth-child(2) .ygg-ui__scenario-name')
    assert.deepEqual(await page.evaluate('commands.at(-1)'), { command: 'select', name: 'empty' })
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('create, rename and a confirmed delete are commands; a taken name is refused here', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.frames()
    await page.evaluate("q('.ygg-ui__scenario-new input').focus()")
    await page.type('spike')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('commands'), [])
    assert.equal(await page.evaluate("q('.ygg-ui__scenario-new input').getAttribute('aria-invalid')"), 'true')
    assert.match(await page.evaluate("q('.ygg-ui__scenario-new .ygg-ui__error').textContent"), /a scenario named spike exists/)
    await page.evaluate("q('.ygg-ui__scenario-new input').value = ''")
    await page.type(' shock ')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('commands.at(-1)'), { command: 'create', name: 'shock' })
    await page.click('.ygg-ui__scenario-list li:nth-child(2) button[aria-label="Rename empty"]')
    await page.frames()
    assert.equal(await page.evaluate('document.activeElement.value'), 'empty', 'the rename field opens on the old name, focused')
    await page.evaluate("document.activeElement.value = ''")
    await page.type('calm')
    await page.press('Enter')
    assert.deepEqual(await page.evaluate('commands.at(-1)'), { command: 'rename', name: 'empty', to: 'calm' })
    await page.frames()
    const before = await page.evaluate('commands.length')
    await page.click('.ygg-ui__scenario-list li:nth-child(2) button[aria-label="Delete empty"]')
    await page.frames()
    assert.equal(await page.evaluate('commands.length'), before, 'the first press only asks')
    assert.equal(await page.evaluate("q('.ygg-ui__scenario-list li:nth-child(2) button[aria-label=\"Confirm deleting empty\"]').textContent"), 'Confirm delete')
    await page.click('.ygg-ui__scenario-list li:nth-child(2) button[aria-label="Confirm deleting empty"]')
    assert.deepEqual(await page.evaluate('commands.at(-1)'), { command: 'delete', name: 'empty' })
    // Focus survives a redraw of the list.
    await page.evaluate("q('.ygg-ui__scenario-list li:nth-child(1) .ygg-ui__scenario-name').focus(); drawer.update({ ...saved })")
    await page.frames()
    assert.equal(await page.evaluate('document.activeElement.textContent'), 'spike (2 events)')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
