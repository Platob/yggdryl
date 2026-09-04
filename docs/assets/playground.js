'use strict'

/*
 * Render docs/assets/playground.json.
 *
 * The package is a native Node addon, so this file evaluates nothing: it fetches
 * the manifest that scripts/build_docs_playground.js wrote by running the real
 * package, and shows what the package answered. There is no encode, decode, or
 * vocabulary logic here, and there never will be - a value that is not in the
 * manifest is a value nobody has asked the package about.
 *
 * The script is loaded on every page and does nothing where no container asks
 * for it.
 */

;(() => {
  const SOURCE = document.currentScript ? document.currentScript.src : ''
  const COMMAND = 'node scripts/build_docs_playground.js'

  let pending = null

  /** Fetch the manifest once per page load, whatever asks for it first. */
  const manifest = () => {
    if (pending === null) {
      const address = SOURCE
        ? new URL('playground.json', SOURCE)
        : new URL('../assets/playground.json', document.baseURI)
      pending = fetch(address).then((answer) => {
        if (!answer.ok) throw new Error(`${answer.status} ${answer.statusText}`)
        return answer.json()
      })
    }
    return pending
  }

  const make = (tag, className, text) => {
    const node = document.createElement(tag)
    if (className) node.className = className
    if (text !== undefined) node.textContent = text
    return node
  }

  /** One `<code>` holding text that must survive as it is, padding included. */
  const code = (text) => make('code', 'ygg-pg__code', text)

  /** A label-and-value table: the shape every case is shown in. */
  const detail = (rows) => {
    const table = make('table', 'ygg-pg__detail')
    const body = make('tbody')
    for (const [name, value, literal] of rows) {
      const line = make('tr')
      const key = make('th', null, name)
      key.setAttribute('scope', 'row')
      const cell = make('td')
      cell.append(literal === false ? document.createTextNode(value) : code(value))
      line.append(key, cell)
      body.append(line)
    }
    table.append(body)
    return table
  }

  /** The expression that produced a case, kept beside it so it can be rerun. */
  const call = (text) => {
    const block = make('pre', 'ygg-pg__call')
    block.append(make('code', null, text))
    return block
  }

  /** A `<select>` over the fixed ASCII types, labelled for a screen reader. */
  const chooser = (id, widths, onChange) => {
    const holder = make('div', 'ygg-pg__controls')
    const label = make('label', null, 'Width')
    label.setAttribute('for', id)
    const select = make('select', 'ygg-pg__select')
    select.id = id
    for (const width of widths) select.append(new Option(width, width))
    select.addEventListener('change', () => onChange(select.value))
    holder.append(label, select)
    return { holder, select }
  }

  /**
   * The corpus of one width as a list of buttons: the whole corpus stays
   * visible, and every case is one tab stop away.
   */
  const list = (entries, name, onPick) => {
    const items = make('ul', 'ygg-pg__values')
    items.setAttribute('role', 'list')
    const buttons = entries.map((entry, index) => {
      const item = make('li')
      const button = make('button', 'ygg-pg__value')
      button.type = 'button'
      button.setAttribute('aria-pressed', 'false')
      button.append(code(name(entry)), make('span', 'ygg-pg__tag', entry.label))
      button.addEventListener('click', () => onPick(index))
      item.append(button)
      items.append(item)
      return button
    })
    const press = (index) => {
      buttons.forEach((button, position) => {
        button.setAttribute('aria-pressed', position === index ? 'true' : 'false')
      })
    }
    return { items, press }
  }

  /** Each fixed ASCII type, what it stores, and the extension identity it carries. */
  const renderWidths = (root, data) => {
    const table = make('table', 'ygg-pg__widths')
    const head = make('thead')
    const heading = make('tr')
    for (const name of [
      'datatype',
      'asciiWidth',
      'kind',
      'Arrow storage',
      'extension name',
      'extension document',
    ]) {
      const cell = make('th', null, name)
      cell.setAttribute('scope', 'col')
      heading.append(cell)
    }
    head.append(heading)
    const body = make('tbody')
    for (const width of data.widths) {
      const line = make('tr')
      const first = make('th')
      first.setAttribute('scope', 'row')
      first.append(code(width.dtype))
      line.append(first)
      for (const value of [
        String(width.asciiWidth),
        width.kind,
        width.arrow,
        width.extensionName,
        width.extensionDocument === '' ? '(empty)' : width.extensionDocument,
      ]) {
        const cell = make('td')
        cell.append(code(value))
        line.append(cell)
      }
      body.append(line)
    }
    table.append(head, body)

    const calls = make('details', 'ygg-pg__calls')
    calls.append(make('summary', null, 'The calls that answered this'))
    calls.append(call(data.widths.map((width) => width.call).join('\n')))

    root.append(table, calls)
  }

  /** Both value views: a width, its corpus, and the selected case. */
  const renderCases = (root, data, kind) => {
    const entries = data[kind]
    const widths = data.widths.map((width) => width.dtype)
    const view = make('div', 'ygg-pg__view')

    const show = (dtype, index) => {
      const corpus = entries.filter((entry) => entry.dtype === dtype)
      const entry = corpus[index]
      view.textContent = ''
      if (entry === undefined) return
      view.append(
        detail(
          kind === 'encode'
            ? [
                ['input', entry.inputLiteral],
                ...(entry.ok
                  ? [
                      ['storage', entry.storageHex],
                      ['storage as text', entry.storageEscaped],
                      ['read back', entry.readBack === '' ? '(empty)' : entry.readBack],
                    ]
                  : [['refused', entry.error, false]]),
              ]
            : [
                ['storage', entry.storageHex],
                ['storage as text', entry.storageEscaped],
                ['text', entry.text === '' ? '(empty)' : entry.text],
              ],
        ),
        call(entry.call),
      )
    }

    let values = null
    const swap = (dtype, index = 0) => {
      const corpus = entries.filter((entry) => entry.dtype === dtype)
      const built = list(
        corpus,
        (entry) => (kind === 'encode' ? entry.inputLiteral : entry.storageEscaped),
        (position) => {
          built.press(position)
          show(dtype, position)
        },
      )
      if (values === null) root.append(built.items, view)
      else values.replaceWith(built.items)
      values = built.items
      built.press(index)
      show(dtype, index)
    }

    const { holder, select } = chooser(`ygg-${kind}-width`, widths, (dtype) => swap(dtype))
    root.append(holder)
    swap(widths[0])

    // The free-text box drives these views, so it needs to reach a case.
    return {
      select: (dtype, index) => {
        select.value = dtype
        swap(dtype, index)
      },
    }
  }

  /** The auto-registering vocabulary, one push at a time. */
  const renderDictionary = (root, data) => {
    const group = data.dictionary
    const steps = group.steps
    let at = 0

    const controls = make('div', 'ygg-pg__controls')
    const back = make('button', 'ygg-pg__step', '← Previous')
    back.type = 'button'
    const forward = make('button', 'ygg-pg__step', 'Next →')
    forward.type = 'button'
    const counter = make('span', 'ygg-pg__counter')
    counter.setAttribute('aria-live', 'polite')
    controls.append(back, counter, forward)

    const view = make('div', 'ygg-pg__view')
    const show = () => {
      const step = steps[at]
      counter.textContent = `Push ${at + 1} of ${steps.length}`
      const first = at === 0
      const last = at === steps.length - 1
      // Disabling the focused control drops focus to <body>, so move it first.
      if (first && document.activeElement === back) forward.focus()
      if (last && document.activeElement === forward) back.focus()
      back.disabled = first
      forward.disabled = last
      view.textContent = ''
      view.append(
        detail([
          ['pushed', step.value],
          ['code', String(step.code)],
          ['new', step.isNew ? 'yes' : 'no, it keeps its first code', false],
          ['vocabulary', step.vocabulary.join(', ')],
          ['datatype', step.dtype],
        ]),
        call(step.call),
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
    after.append(make('h3', null, 'After the last push'))
    const column = group.column
    after.append(
      detail([
        ['column', column.input.map((value) => (value === null ? 'null' : value)).join(', ')],
        ['codes', column.codes.map((value) => (value === null ? 'null' : String(value))).join(', ')],
      ]),
      call(column.call),
      detail([
        [
          `enum ${group.enum.name}`,
          group.enum.members.map(([name, value]) => `${name} = ${value}`).join(', '),
        ],
      ]),
      call(group.enum.call),
    )

    root.append(controls, view, after)
  }

  /** Look a typed value up in the corpus, and say plainly when it is not there. */
  const renderLookup = (root, data, views) => {
    const form = make('form', 'ygg-pg__lookup')
    const label = make('label', null, 'Value')
    label.setAttribute('for', 'ygg-lookup-value')
    const box = make('input', 'ygg-pg__input')
    box.id = 'ygg-lookup-value'
    box.type = 'search'
    box.placeholder = 'USD'
    box.autocomplete = 'off'
    const submit = make('button', 'ygg-pg__step', 'Look it up')
    submit.type = 'submit'
    form.append(label, box, submit)

    const answer = make('div', 'ygg-pg__answer')
    answer.setAttribute('aria-live', 'polite')

    form.addEventListener('submit', (event) => {
      event.preventDefault()
      const wanted = box.value
      answer.textContent = ''
      const encoded = data.encode.findIndex((entry) => entry.input === wanted)
      const decoded = data.decode.findIndex((entry) => entry.text === wanted)
      const kind = encoded !== -1 ? 'encode' : decoded !== -1 ? 'decode' : null
      if (kind === null) {
        const miss = make('p', 'ygg-pg__miss')
        miss.append(
          code(wanted),
          document.createTextNode(
            ' is not in the generated corpus, and this page evaluates nothing: every' +
              ' output above was produced by the package at build time. Add the value to' +
              ' the corpus in scripts/build_docs_playground.js and regenerate:',
          ),
        )
        answer.append(miss, call(COMMAND))
        return
      }
      const entry = data[kind][kind === 'encode' ? encoded : decoded]
      const index = data[kind]
        .filter((other) => other.dtype === entry.dtype)
        .indexOf(entry)
      if (views[kind]) views[kind].select(entry.dtype, index)
      const hit = make('p', 'ygg-pg__hit')
      hit.append(
        document.createTextNode(`Found in ${kind}, under `),
        code(entry.dtype),
        document.createTextNode(` (${entry.label}); the ${kind} view above now shows it.`),
      )
      answer.append(hit)
    })

    root.append(form, answer)
  }

  /** Say what failed and how to put it back, rather than showing nothing. */
  const fail = (roots, reason) => {
    for (const root of roots) {
      root.textContent = ''
      const note = make('p', 'ygg-pg__error')
      note.append(
        document.createTextNode(
          `The generated manifest assets/playground.json could not be loaded (${reason}).` +
            ' It is committed, and a local build writes it with:',
        ),
      )
      root.append(note, call(COMMAND))
    }
  }

  const start = () => {
    const roots = [...document.querySelectorAll('[data-playground]')]
    if (roots.length === 0) return
    manifest().then(
      (data) => {
        const views = {}
        for (const root of roots) {
          const role = root.dataset.playground
          if (role === 'lookup') continue
          root.textContent = ''
          if (role === 'widths') renderWidths(root, data)
          else if (role === 'dictionary') renderDictionary(root, data)
          else if (role === 'encode' || role === 'decode') {
            views[role] = renderCases(root, data, role)
          }
        }
        for (const root of roots.filter((node) => node.dataset.playground === 'lookup')) {
          root.textContent = ''
          renderLookup(root, data, views)
        }
      },
      (error) => fail(roots, error.message),
    )
  }

  // Material's instant navigation swaps the document without reloading this
  // script, so the render is driven by its document observable where it exists.
  if (typeof document$ !== 'undefined' && document$ && typeof document$.subscribe === 'function') {
    document$.subscribe(start)
  } else if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start)
  } else {
    start()
  }
})()
