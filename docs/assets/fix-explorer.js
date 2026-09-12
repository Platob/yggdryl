'use strict'

// Native catalog documents and recorded codec results, presented without
// interpreting FIX bytes, resolving references, or rebuilding datatypes.
;(() => {
  const SOURCE = document.currentScript ? document.currentScript.src : ''
  const CATEGORIES = ['fields', 'components', 'groups']
  const PAGE_SIZE = 60
  let pending = null
  let controlId = 0

  const make = (tag, className, text) => {
    const node = document.createElement(tag)
    if (className) node.className = className
    if (text !== undefined) node.textContent = text
    return node
  }
  const code = (text) => make('code', 'ygg-fx__code', String(text))
  const note = (text) => make('p', 'ygg-fx__note', text)
  const shown = (value) => value === null || value === undefined ? 'null' : String(value)
  const button = (text) => {
    const node = make('button', 'ygg-fx__button', text)
    node.type = 'button'
    return node
  }
  const call = (text) => {
    const block = make('pre', 'ygg-fx__call')
    block.append(code(text))
    return block
  }
  const panel = (title, aside = '') => {
    const element = make('details', 'ygg-fx__panel')
    const summary = make('summary', 'ygg-fx__summary')
    summary.append(make('span', 'ygg-fx__summary-title', title))
    summary.append(make('span', 'ygg-fx__summary-aside', aside))
    const body = make('div', 'ygg-fx__panel-body')
    element.append(summary, body)
    return { element, body }
  }
  const control = (label, node) => {
    const wrapper = make('label', 'ygg-fx__control')
    node.id = `ygg-fx-control-${++controlId}`
    wrapper.htmlFor = node.id
    wrapper.append(make('span', 'ygg-fx__label', label), node)
    return wrapper
  }
  const grid = (headings, rows) => {
    const wrapper = make('div', 'ygg-fx__scroll')
    const table = make('table', 'ygg-fx__grid')
    const header = make('tr')
    for (const name of headings) {
      const cell = make('th', null, name)
      cell.scope = 'col'
      header.append(cell)
    }
    const head = make('thead')
    head.append(header)
    const body = make('tbody')
    for (const row of rows) {
      const line = make('tr')
      for (const value of row) {
        const cell = make('td')
        cell.append(value instanceof Node ? value : code(shown(value)))
        line.append(cell)
      }
      body.append(line)
    }
    table.append(head, body)
    wrapper.append(table)
    return wrapper
  }
  const jsonPanel = (title, value) => {
    const opened = panel(title)
    let filled = false
    opened.element.addEventListener('toggle', () => {
      if (filled || !opened.element.open) return
      filled = true
      opened.body.append(call(JSON.stringify(value, null, 2)))
    })
    return opened.element
  }
  const wire = (text) => {
    const wrapper = make('div', 'ygg-fx__wire')
    wrapper.append(make('pre', 'ygg-fx__wire-text', text))
    const copy = button('Copy displayed text')
    copy.addEventListener('click', async () => {
      try {
        if (!navigator.clipboard) throw new Error('clipboard unavailable')
        await navigator.clipboard.writeText(text)
        copy.textContent = 'Copied'
      } catch {
        copy.textContent = 'Select the text to copy'
      }
    })
    wrapper.append(copy)
    return wrapper
  }
  const metadata = (field) => field.metadata ?? {}
  // `fix:branches` is membership: the dictionaries that contributed a field,
  // comma-separated, lowercase and sorted. Empty for every field the
  // specification alone defines; no lookup consults it.
  const memberships = (field) => (metadata(field)['fix:branches'] ?? '').split(',').filter(Boolean)
  const title = (field) => metadata(field).display ?? field.name
  const searchText = (text) => String(text).toLowerCase()
  const words = (text) => searchText(text).trim().split(/\s+/).filter(Boolean)

  const manifest = () => {
    if (pending === null) {
      const address = SOURCE ? new URL('fix.json', SOURCE) : new URL('../assets/fix.json', document.baseURI)
      pending = fetch(address).then((answer) => {
        if (!answer.ok) throw new Error(`${answer.status} ${answer.statusText}`)
        return answer.json()
      }).then((data) => ({
        data,
        // Search text is presentation state. Names keep their stored spelling;
        // this index does not implement registry lookup or the one fold.
        definitions: CATEGORIES.flatMap((category) => data.catalog[category].map((field) => ({
          category,
          field,
          text: searchText([category, field.name, ...['display', 'description', 'fix:tag', 'fix:aliases', 'fix:tags', 'fix:counter', 'fix:component', 'fix:msgtype', 'fix:branches'].map((key) => metadata(field)[key] ?? '')].join(' ')),
        }))),
      }))
    }
    return pending
  }

  function renderKpi(root, { data }) {
    const entries = [
      [data.kpi.fields, 'scalar fields'], [data.kpi.messages, 'messages'],
      [data.kpi.components, 'components'], [data.kpi.groups, 'groups'],
      [data.kpi.codes, 'inline codes'], [data.kpi.enumFields, 'fields with codes'],
      [data.kpi.columns, 'capture columns'], [data.spec.version, 'FIX version'],
    ]
    const cards = make('div', 'ygg-fx__cards')
    for (const [value, label] of entries) {
      const card = make('div', 'ygg-fx__card')
      card.append(make('span', 'ygg-fx__card-value', typeof value === 'number' ? value.toLocaleString() : value))
      card.append(make('span', 'ygg-fx__card-label', label))
      cards.append(card)
    }
    root.append(cards, note('Counts from the native registry, including this crate\'s capture fields.'))
  }

  function fieldDetail(field, category, navigate) {
    const body = make('div')
    const meta = metadata(field)
    const held = memberships(field)
    body.append(grid(['Property', 'Native value'], [
      ['category', category], ['name', field.name],
      ['datatype', field.dtype.type], ['nullable', field.nullable],
      ...['fix:tag', 'fix:tags', 'fix:aliases', 'fix:counter', 'fix:component', 'fix:msgtype', 'description'].filter((key) => meta[key] !== undefined).map((key) => [key, meta[key]]),
      // Membership is provenance, shown only where a dictionary recorded it.
      ...(held.length ? [['membership', held.join(', ')]] : []),
    ]))
    // Direct occurrences are already present in the native Field document.
    // A reference button changes the search; it never expands a target schema.
    const members = field.dtype.fields ?? (field.dtype.field ? [field.dtype.field] : [])
    if (members.length) {
      body.append(make('h4', null, 'Declared occurrences'))
      body.append(grid(['Occurrence', 'Presence', 'Reference'], members.map((member) => {
        const held = metadata(member)
        const refs = make('span', 'ygg-fx__chips')
        for (const [key, target] of [['fix:field', 'fields'], ['fix:component', 'components'], ['fix:group', 'groups']]) {
          if (held[key] === undefined) continue
          const link = button(`${target}: ${held[key]}`)
          link.addEventListener('click', () => navigate(target, held[key]))
          refs.append(link)
        }
        if (!refs.childNodes.length) refs.textContent = member.dtype.type
        return [member.name, member.nullable ? 'optional' : 'required', refs]
      })))
    }
    if (meta['fix:codes']) {
      // This is a stored native metadata document, not a second enum registry.
      const codes = JSON.parse(meta['fix:codes']).codes
      const details = panel('Inline codes', `${codes.length}`)
      let filled = false
      details.element.addEventListener('toggle', () => {
        if (filled || !details.element.open) return
        filled = true
        details.body.append(grid(['Value', 'Name', 'Description'], codes.map((entry) => [entry.value, entry.name, entry.doc ?? ''])))
      })
      body.append(details.element)
    }
    if (meta['fix:lineage']) body.append(jsonPanel('Native lineage metadata', JSON.parse(meta['fix:lineage'])))
    body.append(jsonPanel('Native Field document', field))
    return body
  }

  function renderCatalog(root, held) {
    const controls = make('div', 'ygg-fx__controls')
    const query = make('input', 'ygg-fx__input ygg-fx__input--wide')
    query.type = 'search'
    query.placeholder = '453, Parties, Party, symbol, D...'
    const category = make('select', 'ygg-fx__select')
    category.append(new Option('All four categories', 'all'))
    for (const name of CATEGORIES) category.append(new Option(name, name))
    const coded = make('select', 'ygg-fx__select')
    coded.append(new Option('All definitions', 'all'), new Option('Fields with inline codes', 'codes'))
    controls.append(control('Search catalog text', query), control('Category', category), control('Vocabulary', coded))
    const count = make('p', 'ygg-fx__counter')
    count.setAttribute('aria-live', 'polite')
    const view = make('div', 'ygg-fx__list')
    const more = button('Show more')
    let limit = PAGE_SIZE
    const navigate = (target, name) => {
      category.value = target
      query.value = name
      coded.value = 'all'
      limit = PAGE_SIZE
      show()
      query.focus()
      controls.scrollIntoView({ block: 'nearest' })
    }
    const show = () => {
      const wanted = words(query.value)
      const matches = held.definitions.filter((entry) =>
        (category.value === 'all' || entry.category === category.value) &&
        (coded.value !== 'codes' || metadata(entry.field)['fix:codes'] !== undefined) &&
        wanted.every((word) => entry.text.includes(word)))
      count.textContent = `${matches.length.toLocaleString()} matching definitions; showing ${Math.min(limit, matches.length).toLocaleString()}`
      view.replaceChildren()
      for (const { field, category: kind } of matches.slice(0, limit)) {
        const meta = metadata(field)
        const identity = meta['fix:tag'] ? `tag ${meta['fix:tag']}` : meta['fix:counter'] ? `counter ${meta['fix:counter']}` : meta['fix:msgtype'] ? `wire ${meta['fix:msgtype']}` : ''
        const entry = panel(title(field), [kind, identity, memberships(field).join(', ')].filter(Boolean).join(' / '))
        let filled = false
        entry.element.addEventListener('toggle', () => {
          if (filled || !entry.element.open) return
          filled = true
          entry.body.append(fieldDetail(field, kind, navigate))
        })
        view.append(entry.element)
      }
      more.hidden = matches.length <= limit
      more.textContent = `Show ${Math.min(PAGE_SIZE, Math.max(0, matches.length - limit))} more`
    }
    const reset = () => { limit = PAGE_SIZE; show() }
    query.addEventListener('input', reset)
    category.addEventListener('change', reset)
    coded.addEventListener('change', reset)
    more.addEventListener('click', () => { limit += PAGE_SIZE; show() })
    root.append(note('Search filters the stored catalog text. Reference buttons select another search; schemas remain the native documents.'), controls, count, view, more)
    show()
  }

  function frameDetail(frame, emittedFirst = false) {
    const body = make('div')
    const output = () => {
      body.append(make('h4', null, 'Native emitted bytes (escaped text)'), wire(frame.emitted || '(empty)'))
      body.append(call(`const [message] = ${frame.call}\nmessage.intoBytes(0x7c)`))
    }
    if (emittedFirst) output()
    body.append(make('h4', null, 'Recorded input (escaped text)'), wire(frame.line))
    body.append(grid(['Native answer', 'Value'], [
      ['MIME type', frame.mime], ['message code', frame.msgtype], ['direction', frame.direction],
      ['root', frame.root], ['field count', frame.size],
      ['ticker', frame.ticker], ['market timestamp', frame.clock], ['partition', frame.partition], ['digest', frame.digest],
    ]))
    body.append(make('h4', null, 'Native anomalies'), note(frame.anomalies.length ? frame.anomalies.join(', ') : 'No anomalies reported.'))
    const arrivals = panel('Raw arrivals', `${frame.arrivals.length} entries`)
    arrivals.body.append(grid(['Tag', 'Original key', 'Original value'], frame.arrivals))
    body.append(arrivals.element)
    const columns = panel('Populated capture columns', `${frame.columns.length}`)
    columns.body.append(grid(['Tag', 'Name', 'Datatype', 'Value'], frame.columns.map((column) => [column.t, column.n, column.y, column.v])))
    body.append(columns.element)
    const lifted = panel('Native facets', `${frame.lift.length}`)
    lifted.body.append(grid(['Facet', 'Value', 'Source tag'], frame.lift))
    body.append(lifted.element, jsonPanel('Native message Field', frame.field), jsonPanel('Native message Scalar', frame.value))
    if (!emittedFirst) output()
    return body
  }

  function renderSamples(root, { data }, mode = 'decode') {
    const query = make('input', 'ygg-fx__input ygg-fx__input--wide')
    query.type = 'search'
    query.placeholder = 'parties, order, untyped...'
    const select = make('select', 'ygg-fx__select ygg-fx__select--wide')
    const controls = make('div', 'ygg-fx__controls')
    controls.append(control('Filter recorded samples', query), control('Native sample', select))
    const view = make('div')
    const show = () => {
      const frame = data.frames.find((entry) => entry.key === select.value)
      view.replaceChildren(frame ? frameDetail(frame, mode === 'encode') : note('No recorded sample matches this search.'))
    }
    const filter = () => {
      const previous = select.value
      const wanted = words(query.value)
      select.replaceChildren()
      for (const frame of data.frames) {
        if (!wanted.every((word) => searchText(`${frame.key} ${frame.label} ${frame.line}`).includes(word))) continue
        select.append(new Option(frame.label, frame.key))
      }
      if ([...select.options].some((option) => option.value === previous)) select.value = previous
      select.disabled = !select.options.length
      show()
    }
    query.addEventListener('input', filter)
    select.addEventListener('change', show)
    root.append(note('Every result below was produced by the native codec. Search selects recorded samples; run the shown package call to process your own input.'), controls, view)
    filter()
  }

  function renderFrames(root, { data }) {
    root.append(note(`${data.frames.length} samples generated through the native codec.`))
    for (const frame of data.frames) {
      const entry = panel(frame.label, frame.msgtype ?? frame.mime)
      let filled = false
      entry.element.addEventListener('toggle', () => {
        if (filled || !entry.element.open) return
        filled = true
        entry.body.append(frameDetail(frame))
      })
      root.append(entry.element)
    }
  }

  function renderRow(root, { data }) {
    const query = make('input', 'ygg-fx__input ygg-fx__input--wide')
    query.type = 'search'
    query.placeholder = '35, Symbol, timestamp...'
    const count = make('p', 'ygg-fx__counter')
    count.setAttribute('aria-live', 'polite')
    const view = make('div')
    const show = () => {
      const wanted = words(query.value)
      const columns = data.row.columns.filter((column) => wanted.every((word) => searchText(`${column.c} ${column.t} ${column.n} ${column.y} ${column.x}`).includes(word)))
      count.textContent = `${columns.length} of ${data.row.columns.length} native capture columns`
      view.replaceChildren(grid(['Column', 'Tag', 'Name', 'Datatype', 'Description'], columns.map((column) => [column.c, column.t, column.n, column.y, column.x])))
    }
    query.addEventListener('input', show)
    root.append(control('Find a capture column', query), count, view, call(data.row.call))
    show()
  }

  function renderSources(root, { data }) {
    const link = (text, href) => {
      const node = make('a', null, text)
      node.href = href
      node.rel = 'noreferrer'
      return node
    }
    root.append(grid(['Source', 'Format', 'Version', 'SHA-256', 'License'], data.spec.sources.map((source) => [
      link(source.id, source.url), source.format, source.version, source.sha256, link('License', source.license),
    ])))
    // Membership travels on each field's `fix:branches`; a registry lists the
    // distinct names through `dialects()`. The shipped dictionary names none.
    const dialects = data.kpi.dialectSizes ?? []
    if (dialects.length) {
      const held = panel('Dialects', `${dialects.length}`)
      held.body.append(grid(['Dialect', 'Member fields'], dialects))
      root.append(held.element)
    } else {
      root.append(note('No field carries a membership: every field is the specification\'s own, and registry.dialects() answers an empty list.'))
    }
  }

  const renderers = {
    kpi: renderKpi, catalog: renderCatalog, sources: renderSources, row: renderRow,
    decode: renderSamples, encode: (root, data) => renderSamples(root, data, 'encode'), frames: renderFrames,
  }
  function start() {
    const roots = [...document.querySelectorAll('[data-fix]')]
    if (!roots.length) return
    manifest().then((held) => {
      for (const root of roots) {
        root.replaceChildren()
        const render = renderers[root.dataset.fix]
        if (render) render(root, held)
        root.classList.add('ygg-fx__ready')
      }
    }).catch((error) => {
      for (const root of roots) root.replaceChildren(note(`The generated FIX manifest could not be displayed: ${error.message}`), call('node scripts/build_docs_fix.js'))
    })
  }
  if (typeof document$ !== 'undefined' && document$ && typeof document$.subscribe === 'function') document$.subscribe(start)
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start)
  else start()
})()
