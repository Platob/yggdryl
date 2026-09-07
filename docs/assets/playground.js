/* Render the generated ASCII manifest; every answer came from the Node package. */

import {
  assetLoader,
  button,
  callBlock,
  choiceList,
  code,
  control,
  facts,
  grid,
  make,
  note,
  onDocumentReady,
  panel,
  search,
  select,
} from './ui/index.js'

const COMMAND = 'node scripts/build_docs_playground.js'
const loader = assetLoader({
  baseURL: import.meta.url,
  assets: { manifest: 'playground.json' },
  regenerate: COMMAND,
})

const renderWidths = (root, data) => {
  root.append(
    grid({
      className: 'ygg-pg__widths',
      columns: [
        { label: 'datatype', key: 'dtype', rowHeader: true },
        { label: 'asciiWidth', key: 'asciiWidth' },
        { label: 'kind', key: 'kind' },
        { label: 'Arrow storage', key: 'arrow' },
        { label: 'extension name', key: 'extensionName' },
        { label: 'extension document', key: 'extensionDocument' },
      ],
      rows: data.widths.map((width) => ({
        ...width,
        extensionDocument: width.extensionDocument === '' ? '(empty)' : width.extensionDocument,
      })),
    }),
    panel({
      title: 'The calls that answered this',
      render: () => callBlock({ code: data.widths.map((width) => width.call).join('\n') }),
    }).element,
  )
}

const renderCases = (root, data, kind) => {
  const entries = data[kind]
  const widths = data.widths.map((width) => width.dtype)
  const view = make('div')
  let values = null

  const show = (dtype, index) => {
    const entry = entries.filter((item) => item.dtype === dtype)[index]
    view.textContent = ''
    if (entry === undefined) return
    const rows =
      kind === 'encode'
        ? [
            { label: 'input', value: entry.inputLiteral },
            ...(entry.ok
              ? [
                  { label: 'storage', value: entry.storageHex },
                  { label: 'storage as text', value: entry.storageEscaped },
                  { label: 'read back', value: entry.readBack === '' ? '(empty)' : entry.readBack },
                ]
              : [{ label: 'refused', value: entry.error, format: (value) => value }]),
          ]
        : [
            { label: 'storage', value: entry.storageHex },
            { label: 'storage as text', value: entry.storageEscaped },
            { label: 'text', value: entry.text === '' ? '(empty)' : entry.text },
          ]
    view.append(facts({ rows }), callBlock({ code: entry.call }))
  }

  const swap = (dtype, index = 0) => {
    const corpus = entries.filter((entry) => entry.dtype === dtype)
    const choices = choiceList({
      items: corpus,
      selected: index,
      render: (entry) => [
        code({ text: kind === 'encode' ? entry.inputLiteral : entry.storageEscaped }),
        make('span', 'ygg-pg__tag', entry.label),
      ],
      onSelect: (_, position) => show(dtype, position),
    })
    if (values === null) root.append(choices.element, view)
    else values.replaceWith(choices.element)
    values = choices.element
    show(dtype, index)
  }

  const chooser = select({
    options: widths.map((width) => ({ label: width, value: width })),
    onChange: (event) => swap(event.currentTarget.value),
  })
  const controls = make('div', 'ygg-ui__controls')
  controls.append(control({ id: 'ygg-' + kind + '-width', label: 'Width', node: chooser }))
  root.append(controls)
  swap(widths[0])

  return {
    select(dtype, index) {
      chooser.value = dtype
      swap(dtype, index)
    },
  }
}

const renderVocabulary = (root, data) => {
  const group = data.vocabulary
  const steps = group.steps
  let at = 0
  const back = button({ label: '← Previous' })
  const forward = button({ label: 'Next →' })
  const counter = make('span', 'ygg-ui__counter')
  const controls = make('div', 'ygg-ui__controls')
  const view = make('div')
  counter.setAttribute('aria-live', 'polite')
  controls.append(back, counter, forward)

  const show = () => {
    const step = steps[at]
    const first = at === 0
    const last = at === steps.length - 1
    counter.textContent = 'Member ' + (at + 1) + ' of ' + steps.length
    if (first && document.activeElement === back) forward.focus()
    if (last && document.activeElement === forward) back.focus()
    back.disabled = first
    forward.disabled = last
    view.textContent = ''
    view.append(
      facts({
        rows: [
          { label: 'value', value: step.value },
          { label: 'member', value: step.member },
          { label: 'generated name', value: step.generated },
          { label: 'code', value: step.code },
          { label: 'storage', value: step.storageHex },
          {
            label: 'in the prebuilt listing',
            value: step.isPrebuilt ? 'yes' : 'no, this one is the declaration’s own',
            format: (value) => value,
          },
        ],
      }),
      callBlock({ code: step.call }),
    )
  }
  back.addEventListener('click', () => {
    at = Math.max(0, at - 1)
    show()
  })
  forward.addEventListener('click', () => {
    at = Math.min(steps.length - 1, at + 1)
    show()
  })
  show()

  const after = make('div', 'ygg-pg__after')
  after.append(
    make('h3', null, 'The listing the package ships'),
    facts({
      rows: [
        { label: 'datatype', value: group.dtype },
        { label: 'codes', value: String(group.prebuilt.size) },
        { label: 'first twelve', value: group.prebuilt.sample.join(', ') },
      ],
    }),
    callBlock({ code: group.prebuilt.call }),
    make('h3', null, 'The declaration on the field'),
    facts({
      rows: [
        {
          label: 'enum ' + group.enum.name,
          value: group.enum.members.map(([name, value]) => name + ' = ' + value).join(', '),
        },
        { label: 'ARROW:extension:name', value: group.declaration.extensionName },
        { label: 'field:enum', value: group.declaration.carried },
      ],
    }),
    callBlock({ code: group.declaration.call }),
  )
  root.append(controls, view, after)
}

const renderLookup = (root, data, views) => {
  const form = make('form', 'ygg-pg__lookup')
  const box = search({ placeholder: 'USD', autocomplete: 'off' })
  const submit = button({ label: 'Look it up', type: 'submit' })
  const answer = make('div')
  answer.setAttribute('aria-live', 'polite')
  form.append(control({ id: 'ygg-lookup-value', label: 'Value', node: box }), submit)

  form.addEventListener('submit', (event) => {
    event.preventDefault()
    const wanted = box.value
    answer.textContent = ''
    const encoded = data.encode.findIndex((entry) => entry.input === wanted)
    const decoded = data.decode.findIndex((entry) => entry.text === wanted)
    const kind = encoded !== -1 ? 'encode' : decoded !== -1 ? 'decode' : null
    if (kind === null) {
      answer.append(
        note({
          content: [
            code({ text: wanted }),
            ' is not in the generated corpus, and this page evaluates nothing: every output' +
              ' above was produced by the package at build time. Add the value to the corpus' +
              ' in scripts/build_docs_playground.js and regenerate:',
          ],
        }),
        callBlock({ code: COMMAND }),
      )
      return
    }
    const entry = data[kind][kind === 'encode' ? encoded : decoded]
    const index = data[kind].filter((other) => other.dtype === entry.dtype).indexOf(entry)
    views[kind]?.select(entry.dtype, index)
    const hit = make('p', 'ygg-pg__hit')
    hit.append(
      document.createTextNode('Found in ' + kind + ', under '),
      code({ text: entry.dtype }),
      document.createTextNode(' (' + entry.label + '); the ' + kind + ' view above now shows it.'),
    )
    answer.append(hit)
  })
  root.append(form, answer)
}

const RENDERERS = {
  widths: renderWidths,
  vocabulary: renderVocabulary,
  encode: renderCases,
  decode: renderCases,
}

onDocumentReady({
  run: (document) => {
    const roots = [...document.querySelectorAll('[data-playground]')]
    if (roots.length === 0) return
    loader.json('manifest').then(
      (data) => {
        const views = {}
        for (const root of roots) {
          root.textContent = ''
          root.classList.add('ygg-ui')
          const role = root.dataset.playground
          if (role === 'lookup') continue
          const renderer = RENDERERS[role]
          if (renderer === undefined) continue
          const result =
            role === 'encode' || role === 'decode'
              ? renderer(root, data, role)
              : renderer(root, data)
          if (result !== undefined) views[role] = result
        }
        for (const root of roots.filter((item) => item.dataset.playground === 'lookup')) {
          renderLookup(root, data, views)
        }
      },
      (error) => {
        for (const root of roots) {
          root.textContent = ''
          root.classList.add('ygg-ui')
          root.append(loader.failure({ asset: 'manifest', error }))
        }
      },
    )
  },
})
