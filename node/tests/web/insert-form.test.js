// `node/web/insert-form.js`: a form over the columns the service served,
// emitting an insert, and a native refusal shown verbatim beside the field
// its path names.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { openPage } from './browser.js'

const PAGE = `
import { InsertForm } from '/web/insert-form.js'
const field = await (await fetch('/fixtures/field.json')).json()
window.field = field
window.inserts = []
document.addEventListener('ygg:insert', (event) => inserts.push(event.detail))
const root = document.getElementById('root')
root.style.width = '560px'
window.form = new InsertForm().mount(root)
form.update({ columns: field.columns, kinds: field.kinds, at: '1700000001200000000' })
window.q = (selector) => form.el.querySelector(selector)
window.choose = (name) => { q('.ygg-ui__insert-column').value = name; q('.ygg-ui__insert-add').click() }
window.fact = (name) => q('[data-fact="' + name + '"] input')
window.errorOf = (control) => document.getElementById(control.getAttribute('aria-describedby') ?? '')?.textContent ?? null
`

test('the form offers the served columns and emits an insert of the chosen facts', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    assert.deepEqual(await page.evaluate("[...q('.ygg-ui__insert-kind').options].map((option) => option.value)"), await page.evaluate('field.kinds'))
    assert.equal(await page.evaluate("q('.ygg-ui__insert-at').value"), '1700000001200000000')
    const offered = await page.evaluate("[...q('.ygg-ui__insert-column').options].map((option) => option.value)")
    assert.deepEqual(offered, (await page.evaluate('field.columns.map((column) => column.name)')).filter((name) => name !== 'kind' && name !== 'currunix'), 'every served column but the two the form asks for itself, in served order')
    await page.evaluate("choose('price'); choose('side'); choose('price')")
    assert.deepEqual(await page.evaluate("[...form.el.querySelectorAll('[data-fact]')].map((row) => [row.dataset.fact, row.querySelector('.ygg-ui__insert-dtype').textContent])"), [['price', 'decimal'], ['side', 'side']], 'a column is chosen once')
    assert.equal(await page.evaluate("q('.ygg-ui__insert-column option[value=price]').disabled"), true)
    await page.evaluate("fact('price').focus()")
    await page.type('99.5')
    await page.evaluate("fact('side').focus()")
    await page.type('BUY')
    await page.evaluate("q('.ygg-ui__insert-kind').value = 'order_event'")
    await page.screenshot('insert-form')
    await page.click('.ygg-ui__insert-submit')
    assert.deepEqual(await page.evaluate('inserts'), [{ kind: 'order_event', currunix: '1700000001200000000', facts: { price: '99.5', side: 'BUY' } }])
    // An empty fact is not stated; a removed one is gone.
    await page.evaluate("fact('side').value = ''; choose('ticker')")
    await page.click('[data-fact="price"] button[aria-label="Remove price"]')
    await page.evaluate("fact('ticker').value = 'ALPHA'")
    await page.click('.ygg-ui__insert-submit')
    assert.deepEqual(await page.evaluate('inserts.at(-1).facts'), { ticker: 'ALPHA' })
    assert.equal(await page.evaluate("q('.ygg-ui__insert-column option[value=price]').disabled"), false, 'a removed column can be chosen again')
    // An instant that is not decimal nanoseconds is refused here, beside its field.
    await page.evaluate("q('.ygg-ui__insert-at').value = '2024-01-02'")
    await page.click('.ygg-ui__insert-submit')
    assert.equal(await page.evaluate('inserts.length'), 2)
    assert.equal(await page.evaluate("q('.ygg-ui__insert-at').getAttribute('aria-invalid')"), 'true')
    assert.match(await page.evaluate("errorOf(q('.ygg-ui__insert-at'))"), /expected an instant as decimal nanoseconds/)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('reset: an instant typed for one insert gives way to the one shown, and follows it again', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    const at = () => page.evaluate("q('.ygg-ui__insert-at').value")
    const show = async (instant) => {
      await page.evaluate(`form.update({ columns: field.columns, kinds: field.kinds, at: '${instant}' })`)
      await page.frames()
    }
    await page.evaluate("q('.ygg-ui__insert-at').focus(); q('.ygg-ui__insert-at').select()")
    await page.type('1700000009000000000')
    await show('1700000002000000000')
    assert.equal(await at(), '1700000009000000000', 'a typed instant is kept while the form is open')
    await page.evaluate('form.reset()')
    assert.equal(await at(), '1700000002000000000')
    await show('1700000003000000000')
    assert.equal(await at(), '1700000003000000000')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a native refusal is shown verbatim beside the field its path names', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.evaluate("choose('price')")
    const price = '$.price: expected a decimal representable at scale 18 within 38 digits, got "abc"'
    await page.evaluate(`form.showRefusal(${JSON.stringify(price)})`)
    assert.equal(await page.evaluate("fact('price').getAttribute('aria-invalid')"), 'true')
    assert.equal(await page.evaluate("errorOf(fact('price'))"), price)
    const kind = '$.operations[1].kind: expected an operation event, got book_side'
    await page.evaluate(`form.showRefusal(${JSON.stringify(kind)})`)
    assert.equal(await page.evaluate("fact('price').getAttribute('aria-invalid')"), 'false', 'a new refusal clears the last')
    assert.equal(await page.evaluate("errorOf(q('.ygg-ui__insert-kind'))"), kind)
    await page.screenshot('insert-form-refusal')
    // A path naming no field the form shows, or no path at all: the form's own line.
    const elsewhere = '$.bidside.live[0].miccode: expected a MIC, got "?"'
    await page.evaluate(`form.showRefusal(${JSON.stringify(elsewhere)})`)
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').textContent"), elsewhere)
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').hidden"), false)
    await page.evaluate("form.showRefusal('the service did not answer')")
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').textContent"), 'the service did not answer')
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').getAttribute('role')"), 'alert')
    await page.evaluate('form.showRefusal(null)')
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').hidden"), true, 'null clears it')
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a map, a serie or a struct fact is typed as JSON and sent as the value it spells', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    const dtypes = await page.evaluate("Object.fromEntries(field.columns.map((column) => [column.name, column.dtype]))")
    // The fixture is the native field as the service serves it: the display the native package writes.
    assert.match(dtypes.securityids, /^map\(/)
    assert.match(dtypes.srcuuids, /^serie\(/)
    assert.match(dtypes.bidside, /^struct\(/)
    await page.evaluate("choose('securityids'); choose('srcuuids'); choose('metadata'); choose('ticker')")
    await page.evaluate(`fact('securityids').value = '[["ISIN", "US0378331005"]]'`)
    await page.evaluate(`fact('srcuuids').value = '["0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"]'`)
    await page.evaluate(`fact('metadata').value = '{"note": "hand typed"}'`)
    // A text column stays the text typed, even when it reads as JSON.
    await page.evaluate(`fact('ticker').value = '["ALPHA"]'`)
    await page.evaluate("q('.ygg-ui__insert-kind').value = 'order_event'")
    await page.screenshot('insert-form-json')
    await page.click('.ygg-ui__insert-submit')
    assert.deepEqual(await page.evaluate('inserts'), [
      {
        kind: 'order_event',
        currunix: '1700000001200000000',
        facts: {
          securityids: [['ISIN', 'US0378331005']],
          srcuuids: ['0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b'],
          metadata: { note: 'hand typed' },
          ticker: '["ALPHA"]',
        },
      },
    ])
    // Text that is not JSON is refused here, naming the fact, and nothing is sent.
    await page.evaluate("fact('securityids').value = 'ISIN=US0378331005'")
    await page.click('.ygg-ui__insert-submit')
    assert.equal(await page.evaluate('inserts.length'), 1)
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').hidden"), false)
    assert.match(await page.evaluate("q('.ygg-ui__insert-refusal').textContent"), /^securityids: expected JSON for a map, as \[\["key", "value"\]\]: /)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})

test('a refusal on one operation, or inside a fact, lands beside that fact', async () => {
  const { page, close } = await openPage(PAGE)
  try {
    await page.evaluate("choose('price'); choose('metadata')")
    // The admission walk names one operation `$.operation.<name>`.
    const price = '$.operation.price: expected a decimal, got "abc"'
    await page.evaluate(`form.showRefusal(${JSON.stringify(price)})`)
    assert.equal(await page.evaluate("errorOf(fact('price'))"), price)
    // A path below a fact names that fact.
    const metadata = 'invalid record value at $.metadata[1].key: map keys are not sorted'
    await page.evaluate(`form.showRefusal(${JSON.stringify(metadata)})`)
    assert.equal(await page.evaluate("errorOf(fact('metadata'))"), metadata)
    assert.equal(await page.evaluate("q('.ygg-ui__insert-refusal').hidden"), true)
    assert.deepEqual(page.consoleLines, [])
  } finally {
    await close()
  }
})
