'use strict'

/*
 * The FIX explorer: docs/assets/fix.json and fix-codes.json, made usable.
 *
 * The package is a native Node addon, so a browser cannot load it. Everything
 * this page *states* about the dictionary therefore comes from the manifests
 * `scripts/build_docs_fix.js` wrote by running the real package: every tag,
 * name, datatype, code, lineage entry, projected column, and every decoded
 * frame in the corpus is what a real call answered.
 *
 * What this file does compute is the reading of text you type: splitting a
 * frame into pairs, naming each tag from the manifest, resolving a value
 * through its code set, gathering occurrences under their counter, and
 * checking BodyLength and CheckSum. That is dictionary lookup and FIX
 * arithmetic over the shipped answers - it types no value and derives no
 * facet, and the corpus frames show the package's own answer beside it so the
 * two can be compared on the same line.
 *
 * No framework, no CDN, no build step. The script is loaded on every page and
 * does nothing where no container asks for it.
 */

;(() => {
  const SOURCE = document.currentScript ? document.currentScript.src : ''
  const COMMAND = 'node scripts/build_docs_fix.js'
  const SOH = ''

  // ---------------------------------------------------------------- fetching

  let index = null
  let detail = null

  /** One asset beside this script, fetched once per page load. */
  const asset = (name) => {
    const address = SOURCE
      ? new URL(name, SOURCE)
      : new URL(`../assets/${name}`, document.baseURI)
    return fetch(address).then((answer) => {
      if (!answer.ok) throw new Error(`${answer.status} ${answer.statusText}`)
      return answer.json()
    })
  }

  /** The index: the counts, the fields, the layouts, the corpus. */
  const manifest = () => {
    if (index === null) index = asset('fix.json').then(model)
    return index
  }

  /**
   * The code sets and lineages, fetched behind the first paint.
   *
   * Two thirds of the bytes explain one field at a time, and nothing on the
   * page needs them until a reader opens a field or decodes a value - so the
   * index renders first and this lands under it.
   */
  const codes = () => {
    if (detail === null) detail = asset('fix-codes.json')
    return detail
  }

  // ------------------------------------------------------------------- model

  /** ASCII case folding, the one the crate applies to a name. */
  const folded = (text) => text.toLowerCase().replace(/[\s_-]+/g, '')

  /**
   * The manifest, indexed the way every view asks about it.
   *
   * Built once per page load: a tag or a name reaches a field in one lookup,
   * a message type reaches its layout, and a group identifier reaches the
   * members a `groupRef` names.
   */
  function model(data) {
    const byTag = new Map()
    const byName = new Map()
    // What a counter introduces on the wire, from the layouts: the registry
    // holds only the members it can type, so a group inside a group is not
    // among them and a group of nothing but groups is not nested there at all.
    const groupsByTag = new Map(
      Object.entries(data.wire).map(([tag, held]) => [Number(tag), held]),
    )
    for (const record of data.fields) {
      // The standard branch wins a tag, which is the registry's own order.
      if (!byTag.has(record.t) || (record.b ?? '') === '') byTag.set(record.t, record)
      for (const name of [record.n, record.d, ...(record.a ?? [])]) {
        if (name === undefined) continue
        const key = folded(name)
        if (!byName.has(key)) byName.set(key, record)
      }
    }
    const messages = new Map(data.messages.map((held) => [held.y, held]))
    const components = new Map(data.components.map((held) => [held.i, held]))
    const layoutGroups = new Map(data.groups.map((held) => [held.i, held]))
    // What every field is searched on, built once: six thousand template
    // strings per keystroke is the difference between a filter that keeps up
    // with typing and one that does not.
    const haystacks = new Map(
      data.fields.map((record) => [
        record,
        `${record.t} ${record.n} ${record.d ?? ''} ${(record.a ?? []).join(' ')} ${record.x ?? ''}`.toLowerCase(),
      ]),
    )
    let carriers = null
    const scopes = new Map()
    const held = {
      data,
      byTag,
      byName,
      groupsByTag,
      messages,
      components,
      layoutGroups,
      haystacks,
      header: new Set(data.header),
      trailer: new Set(data.trailer),
      /** The field a wire key names, whatever spelling it arrived in. */
      field(key) {
        const bare = key.startsWith('#') ? key.slice(1) : key
        if (/^\d+$/.test(bare)) return byTag.get(Number(bare)) ?? null
        return byName.get(folded(bare)) ?? null
      },
      /**
       * What each counter introduces *in one message type*.
       *
       * `wire` unions every context a counter is declared in - Orchestra
       * declares `NoRelatedSym` eleven times - and a union swallows a
       * message-level field that some other context happens to put in the
       * group. One message's own layout says exactly which members its
       * instance holds, so that is what an occurrence ends on; the union
       * stays the answer for a counter the layout does not declare, because a
       * venue sends groups no message states.
       */
      groupsOf(msgtype) {
        if (msgtype === null) return new Map()
        const kept = scopes.get(msgtype)
        if (kept !== undefined) return kept
        const message = messages.get(msgtype)
        const found = new Map()
        if (message === undefined) {
          scopes.set(msgtype, found)
          return found
        }
        const flatten = (members, into, seen) => {
          for (const [kind, id] of members) {
            if (kind === 'f') {
              if (!into.includes(id)) into.push(id)
            } else if (kind === 'c') {
              const component = components.get(id)
              if (component !== undefined && !seen.has(id)) {
                seen.add(id)
                flatten(component.m, into, seen)
              }
            } else {
              const nested = layoutGroups.get(id)
              if (nested !== undefined && !into.includes(nested.t)) into.push(nested.t)
            }
          }
        }
        const walk = (members, seen) => {
          for (const [kind, id] of members) {
            if (kind === 'c') {
              const component = components.get(id)
              if (component === undefined || seen.has(`c${id}`)) continue
              seen.add(`c${id}`)
              walk(component.m, seen)
            } else if (kind === 'g') {
              const group = layoutGroups.get(id)
              if (group === undefined || seen.has(`g${id}`)) continue
              seen.add(`g${id}`)
              const into = []
              flatten(group.m, into, new Set())
              if (!found.has(group.t)) found.set(group.t, { n: group.n, m: into })
              walk(group.m, seen)
            }
          }
        }
        walk(message.m, new Set())
        scopes.set(msgtype, found)
        return found
      },
      /**
       * The message types whose layout reaches one tag.
       *
       * Walking all hundred and eighty layouts answers this for every tag at
       * once, so it is done once for the page rather than once per field a
       * reader opens - one of those layouts alone reaches three thousand tags.
       */
      carriers(tag) {
        if (carriers === null) {
          carriers = new Map()
          for (const message of data.messages) {
            for (const entry of layoutTags(held, message)) {
              const kept = carriers.get(entry.tag)
              if (kept === undefined) carriers.set(entry.tag, [message])
              else if (kept[kept.length - 1] !== message) kept.push(message)
            }
          }
        }
        return carriers.get(tag) ?? []
      },
    }
    return held
  }

  /**
   * Every tag one message layout reaches, flattened through its members.
   *
   * `required` is the specification's own reading: a field required inside an
   * optional component is required only once that component is sent, so the
   * flag is carried down the chain rather than read off the member alone.
   */
  function layoutTags(held, message, seen = new Set()) {
    const tags = []
    const walk = (members, depth, inside) => {
      for (const [kind, id, required] of members) {
        const held_ = inside && required === 1
        if (kind === 'f') {
          tags.push({ tag: id, required: held_, depth })
        } else if (kind === 'c') {
          const component = held.components.get(id)
          if (component && !seen.has(`c${id}`)) {
            seen.add(`c${id}`)
            walk(component.m, depth + 1, held_)
          }
        } else if (kind === 'g') {
          const group = held.layoutGroups.get(id)
          if (group && !seen.has(`g${id}`)) {
            seen.add(`g${id}`)
            tags.push({ tag: group.t, required: held_, depth, group })
            walk(group.m, depth + 1, held_)
          }
        }
      }
    }
    walk(message.m, 0, true)
    return tags
  }

  // ------------------------------------------------------------------ making

  const make = (tag, className, text) => {
    const node = document.createElement(tag)
    if (className) node.className = className
    if (text !== undefined) node.textContent = text
    return node
  }

  const code = (text) => make('code', 'ygg-fx__code', text)

  /** A KPI card: one number, what it counts, and where it came from. */
  function card(value, label, note) {
    const held = make('div', 'ygg-fx__card')
    held.append(make('span', 'ygg-fx__card-value', value))
    held.append(make('span', 'ygg-fx__card-label', label))
    if (note) held.append(make('span', 'ygg-fx__card-note', note))
    return held
  }

  /** A row of cards. */
  function cards(entries) {
    const held = make('div', 'ygg-fx__cards')
    for (const [value, label, note] of entries) held.append(card(value, label, note))
    return held
  }

  /**
   * A collapsible panel.
   *
   * `<details>` rather than a scripted toggle: it opens without JavaScript,
   * it is a landmark a screen reader already announces, and the browser's own
   * find-in-page opens it.
   */
  function panel(title, aside, open = false) {
    const held = make('details', 'ygg-fx__panel')
    held.open = open
    const summary = make('summary', 'ygg-fx__summary')
    summary.append(make('span', 'ygg-fx__summary-title', title))
    if (aside !== undefined) summary.append(make('span', 'ygg-fx__summary-aside', aside))
    const body = make('div', 'ygg-fx__panel-body')
    held.append(summary, body)
    return { held, body, summary }
  }

  /** A label-and-value table: the shape one thing's facts are shown in. */
  function facts(rows) {
    const table = make('table', 'ygg-fx__facts')
    const body = make('tbody')
    for (const [name, value, literal] of rows) {
      if (value === null || value === undefined || value === '') continue
      const line = make('tr')
      const key = make('th', null, name)
      key.setAttribute('scope', 'row')
      const cell = make('td')
      if (literal === false) cell.textContent = value
      else if (value instanceof Node) cell.append(value)
      else cell.append(code(String(value)))
      line.append(key, cell)
      body.append(line)
    }
    table.append(body)
    return table
  }

  /** A scrolling grid with a header row. */
  function grid(headings, rows, className) {
    const holder = make('div', 'ygg-fx__scroll')
    const table = make('table', className ? `ygg-fx__grid ${className}` : 'ygg-fx__grid')
    const head = make('thead')
    const heading = make('tr')
    for (const name of headings) {
      const cell = make('th', null, name)
      cell.setAttribute('scope', 'col')
      heading.append(cell)
    }
    head.append(heading)
    const body = make('tbody')
    for (const row of rows) {
      const line = make('tr')
      for (const value of row) {
        const cell = make('td')
        if (value instanceof Node) cell.append(value)
        else if (value === null || value === undefined) cell.textContent = '—'
        else cell.append(code(String(value)))
        line.append(cell)
      }
      body.append(line)
    }
    table.append(head, body)
    holder.append(table)
    return holder
  }

  /** A pill: a short fact with a colour that means something. */
  const pill = (text, kind) => make('span', kind ? `ygg-fx__pill ygg-fx__pill--${kind}` : 'ygg-fx__pill', text)

  /** A labelled control. */
  function control(id, label, node) {
    const held = make('div', 'ygg-fx__control')
    const tag = make('label', 'ygg-fx__label', label)
    tag.setAttribute('for', id)
    node.id = id
    held.append(tag, node)
    return held
  }

  const button = (text, kind) => {
    const held = make('button', kind ? `ygg-fx__button ygg-fx__button--${kind}` : 'ygg-fx__button', text)
    held.type = 'button'
    return held
  }

  /**
   * A block of text kept exactly as it is, with a button that copies it.
   *
   * `copyable` is what the button hands over where that differs from what the
   * block shows: an SOH is drawn as `␁` so a reader can see it, and copied as
   * the byte, because a page that shows the wire has to be able to give it.
   */
  function wire(text, className, copyable) {
    const held = make('div', className ? `ygg-fx__wire ${className}` : 'ygg-fx__wire')
    const body = make('pre', 'ygg-fx__wire-text')
    body.textContent = text
    const copy = button('Copy')
    copy.className = 'ygg-fx__copy'
    copy.addEventListener('click', () => {
      const done = () => {
        copy.textContent = 'Copied'
        setTimeout(() => {
          copy.textContent = 'Copy'
        }, 1200)
      }
      const wanted = copyable ?? body.textContent
      if (navigator.clipboard) navigator.clipboard.writeText(wanted).then(done, () => {})
      else done()
    })
    held.append(body, copy)
    return { held, body }
  }

  /** The expression that answered something, kept beside it. */
  function call(text) {
    const block = make('pre', 'ygg-fx__call')
    block.append(make('code', null, text))
    return block
  }

  const note = (text) => make('p', 'ygg-fx__note', text)

  // ------------------------------------------------------------------ values

  /**
   * The escape the manifest writes a control byte as, read back.
   *
   * One left-to-right pass, because two would eat a real backslash that
   * happens to be followed by `x` and two hex digits - a Windows path in a
   * `Text(58)` is exactly that.
   */
  const unescaped = (text) =>
    text.replace(/\\\\|\\x([0-9a-f]{2})/g, (_, hex) =>
      hex === undefined ? '\\' : String.fromCharCode(Number.parseInt(hex, 16)),
    )

  /** The same, for showing: a control byte a reader can see. */
  const showable = (text) =>
    [...text]
      .map((letter) => {
        const at = letter.codePointAt(0)
        if (at === 1) return '␁'
        if (at < 0x20 || at === 0x7f) return `\\x${at.toString(16).padStart(2, '0')}`
        return letter
      })
      .join('')

  /**
   * What a wire spelling says, where FIX spells it its own way.
   *
   * A reading rather than a typing: the row the package builds is what types
   * a value, and the corpus frames show it. This names what the text is so a
   * timestamp does not have to be read digit by digit.
   */
  function reading(record, text) {
    if (record === null) return null
    const dtype = record.y
    if (dtype === 'boolean') {
      if (text === 'Y') return 'true'
      if (text === 'N') return 'false'
      return null
    }
    if (dtype.startsWith('datetime64')) {
      // Three FIX datatypes land here and only their spelling differs:
      // UTCTimestamp states a date and no zone, TZTimestamp states both, and
      // TZTimeOnly states a zone and no date - so the epoch day supplies one,
      // the seconds it may omit are filled, and a stated zone is kept rather
      // than written over. This is the page's own reading of the shape, not
      // the package's answer: it names what the text is and does not check
      // the calendar, so a value the package nulls can still be named here.
      const held =
        /^(?:(\d{4})(\d{2})(\d{2})-)?(\d{2}):(\d{2})(?::(\d{2}))?(\.\d+)?([Zz]|[+-]\d{2}:?(?:\d{2})?)?$/.exec(
          text,
        )
      if (held === null) return null
      const date = held[1] === undefined ? '1970-01-01' : `${held[1]}-${held[2]}-${held[3]}`
      const zone = held[8] === undefined ? 'Z' : held[8]
      return `${date}T${held[4]}:${held[5]}:${held[6] ?? '00'}${held[7] ?? ''}${zone}`
    }
    if (dtype === 'date32') {
      const held = /^(\d{4})(\d{2})(\d{2})$/.exec(text)
      return held === null ? null : `${held[1]}-${held[2]}-${held[3]}`
    }
    return null
  }

  /** The direction verb in front of a payload, and the payload without it. */
  const VERBS = [
    ['SENT', ['sending', 'sent', 'send', 'outbound', 'outgoing', 'out']],
    ['RECV', ['receiving', 'received', 'receive', 'recv', 'inbound', 'incoming', 'in']],
  ]

  function split_direction(line) {
    const held = /^([A-Za-z]+)\b[\s>:<-]*/.exec(line)
    if (held === null) return [null, line]
    const word = held[1].toLowerCase()
    for (const [direction, verbs] of VERBS) {
      if (verbs.includes(word)) return [direction, line.slice(held[0].length)]
    }
    return [null, line]
  }

  /**
   * The separator a frame uses, chosen from the frame rather than guessed.
   *
   * Each candidate has to be followed by something that looks like the next
   * key, so a pipe inside a `Text(58)` does not win a frame written one pair
   * to a line. SOH is the wire's own and needs no such proof.
   */
  function separatorOf(body) {
    if (body.includes(SOH)) return SOH
    if (body.includes('^A')) return '^A'
    // Counted rather than ranked: a pipe frame that wrapped across two lines
    // holds a newline, and a frame written one pair to a line holds a pipe
    // inside a `Text(58)`. Whichever candidate reads the most pairs is the
    // one the frame was written with; a tie keeps the earlier, commoner one.
    let best = '|'
    let most = 0
    for (const candidate of ['|', '\n', ';', ' ']) {
      const pairs = body
        .split(candidate)
        .filter((segment) => /^\s*[#\w.[\]]+=/.test(segment)).length
      if (pairs > most) {
        most = pairs
        best = candidate
      }
    }
    return best
  }

  /**
   * The frame inside a log line, without whatever the emitter put in front.
   *
   * A timestamp, a level or a bracketed tag is not part of the message and
   * must not be counted into its checksum. The frame opens at `8=`, which is
   * where the package's own classifier looks for it, and a tag merely ending
   * in 8 is not it.
   */
  function framed(line) {
    let at = line.indexOf('8=')
    while (at > 0 && /\d/.test(line[at - 1])) at = line.indexOf('8=', at + 1)
    return at > 0 ? line.slice(at) : line
  }

  // ----------------------------------------------------------------- reading

  /**
   * One frame, read against the dictionary.
   *
   * Pairs in arrival order, each named from the manifest; a counter gathers
   * the members that follow it into occurrences; and the two self-describing
   * tags are recomputed from the bytes so a frame states whether it adds up.
   */
  function read(held, text) {
    const trimmed = text.trim()
    const [direction, verbless] = split_direction(trimmed)
    const rest = framed(verbless)
    const separator = separatorOf(rest)
    const entries = []
    let unknown = 0
    for (const segment of rest.split(separator)) {
      // The key is trimmed and the value is not: a log wraps its lines and
      // pads them, but a trailing space inside a `Text(58)` is a byte the
      // sender counted into its own checksum.
      const piece = segment.replace(/\r$/, '')
      const at = piece.indexOf('=')
      if (at <= 0) continue
      const key = piece.slice(0, at).trim()
      if (key === '') continue
      const record = held.field(key)
      if (record === null) unknown += 1
      entries.push({ key, value: piece.slice(at + 1), record, tag: record === null ? null : record.t })
    }

    // A counter takes the members that follow it, which is what makes a
    // repeating group a tree rather than a run of flat pairs. An occurrence
    // ends where a member repeats or a tag the group does not declare
    // arrives; a nested counter is a group of its own inside it.
    const msgtype = entries.find((entry) => entry.tag === 35)?.value ?? null
    const scoped = held.groupsOf(msgtype)
    const groupFor = (tag) => scoped.get(tag) ?? held.groupsByTag.get(tag)

    const tree = []
    let at = 0
    // An occurrence opens on the group's first declared member - FIX's own
    // delimiter rule - and again on any tag the occurrence already holds, for
    // a frame that left the delimiter out.
    const gather = (group, into) => {
      const members = new Set(group.m)
      const delimiter = group.m[0]
      let seen = new Set()
      let occurrence = null
      while (at < entries.length) {
        const entry = entries[at]
        if (entry.tag === null || !members.has(entry.tag)) break
        if (occurrence === null || entry.tag === delimiter || seen.has(entry.tag)) {
          occurrence = { members: [] }
          into.push(occurrence)
          seen = new Set()
        }
        seen.add(entry.tag)
        at += 1
        const nested = groupFor(entry.tag)
        if (nested !== undefined) {
          const inner = []
          gather(nested, inner)
          occurrence.members.push({ ...entry, occurrences: inner })
        } else {
          occurrence.members.push(entry)
        }
      }
    }
    while (at < entries.length) {
      const entry = entries[at]
      at += 1
      const group = entry.tag === null ? undefined : groupFor(entry.tag)
      if (group !== undefined) {
        const occurrences = []
        gather(group, occurrences)
        tree.push({ ...entry, occurrences })
      } else {
        tree.push(entry)
      }
    }

    return {
      direction,
      separator,
      body: rest,
      pairs: entries,
      tree,
      unknown,
      msgtype,
      version: entries.find((entry) => entry.tag === 8)?.value ?? null,
      checks: checks(entries),
    }
  }

  /**
   * What a frame says about its own bytes.
   *
   * BodyLength counts from the byte after its own separator to the one before
   * `10=`, and CheckSum sums every byte up to and including the separator in
   * front of it. Both are computed over the frame with its separator written
   * as one SOH, which is what a real session sends whatever a log printed.
   */
  function checks(entries) {
    const held = []
    const stated = (tag) => entries.find((entry) => entry.tag === tag)?.value ?? null
    // The frame the pairs make, separated by one SOH: what a session sends,
    // whatever a log wrote it as, and the only bytes the two tags describe.
    // Counted as bytes, because that is what they count - a `CAFÉ` in a
    // symbol is five bytes and four characters, and the wire carries five.
    const frame = bytes(entries.map((entry) => `${entry.key}=${entry.value}${SOH}`).join(''))
    const start = locate(frame, '35=')
    const end = locate(frame, '10=')

    const length = stated(9)
    if (length !== null && start !== -1 && end !== -1) {
      const counted = end - start
      held.push({
        name: 'BodyLength (9)',
        stated: length,
        computed: String(counted),
        ok: Number(length) === counted,
      })
    }
    const sum = stated(10)
    if (sum !== null && end !== -1) {
      const computed = checksum(frame, end - 1)
      held.push({ name: 'CheckSum (10)', stated: sum, computed, ok: sum === computed })
    }
    return held
  }

  /** One string as the bytes a frame carrying it holds. */
  const bytes = (text) => new TextEncoder().encode(text)

  /** Where a tag opens in a frame, as a byte offset, or -1. */
  function locate(frame, opening) {
    const wanted = bytes(`${SOH}${opening}`)
    outer: for (let at = 0; at + wanted.length <= frame.length; at += 1) {
      for (let step = 0; step < wanted.length; step += 1) {
        if (frame[at + step] !== wanted[step]) continue outer
      }
      return at + 1
    }
    return -1
  }

  /**
   * Every byte up to and including `stop`, summed, modulo 256, three digits.
   *
   * `stop` is the separator in front of `10=`, which the sum includes: the
   * checksum covers the whole message and the delimiter that closed the field
   * before it.
   */
  function checksum(frame, stop) {
    let total = 0
    for (let at = 0; at <= stop; at += 1) total += frame[at]
    return String(total % 256).padStart(3, '0')
  }

  /** The tags a message type requires, and every tag its layout reaches. */
  function expectations(held, msgtype) {
    const message = msgtype === null ? undefined : held.messages.get(msgtype)
    if (message === undefined) return { message: null, required: new Set(), known: new Set() }
    const tags = layoutTags(held, message)
    return {
      message,
      required: new Set(tags.filter((entry) => entry.required).map((entry) => entry.tag)),
      known: new Set(tags.map((entry) => entry.tag)),
    }
  }

  // ------------------------------------------------------------- field views

  /** One field's name, as a reader recognises it. */
  const titleOf = (record) => record.d ?? record.n

  /** One code set, filtered as you type, because some hold two hundred. */
  function codeTable(entries) {
    const holder = make('div', 'ygg-fx__codes')
    const box = make('input', 'ygg-fx__input')
    box.type = 'search'
    box.placeholder = 'Filter the codes…'
    box.autocomplete = 'off'
    const counter = make('p', 'ygg-fx__counter')
    counter.setAttribute('aria-live', 'polite')
    const view = make('div')
    const show = () => {
      const wanted = box.value.trim().toLowerCase()
      const kept = entries.filter(
        ([value, name, doc]) =>
          wanted === '' ||
          value.toLowerCase().includes(wanted) ||
          name.toLowerCase().includes(wanted) ||
          doc.toLowerCase().includes(wanted),
      )
      counter.textContent = `${kept.length} of ${entries.length} codes`
      view.textContent = ''
      view.append(
        grid(
          ['value', 'name', 'since', 'deprecated', 'meaning'],
          kept.map(([value, name, doc, since, ep, deprecated]) => [
            value,
            name,
            since === '' ? null : ep ? `${since} EP${ep}` : since,
            deprecated === '' ? null : deprecated,
            doc === '' ? null : make('span', 'ygg-fx__prose', doc),
          ]),
        ),
      )
    }
    box.addEventListener('input', show)
    if (entries.length > 12) holder.append(box)
    holder.append(counter, view)
    show()
    return holder
  }

  /** A lineage, as the versions it names, oldest first. */
  function lineageTable(entries) {
    return grid(
      ['since', 'named', 'typed', 'until'],
      entries.map(([since, name, type, until]) => [since, name, type, until === '' ? null : until]),
    )
  }

  /**
   * Everything one field is, opened on demand.
   *
   * The code set and the lineage come from the second manifest, so the panel
   * renders what it has and fills the rest in when that lands.
   */
  function fieldDetail(held, record) {
    const body = make('div', 'ygg-fx__detail')
    const id = `${record.t}:${record.b ?? ''}`
    const rows = [
      ['identifier', id],
      ['tag', String(record.t)],
      ['name', record.n],
      ['display', record.d],
      ['datatype', record.k === 'group' ? `list of ${record.m.length} members` : record.y],
      ['introduces', held.groupsByTag.get(record.t)?.n],
      ['branch', (record.b ?? '') === '' ? 'standard' : record.b],
      ['aliases', (record.a ?? []).join(', ')],
      ['alternate tags', (record.g ?? []).join(', ')],
      ['since', record.s],
      ['until', record.e],
    ]
    if (record.x) rows.push(['wording', make('span', 'ygg-fx__prose', record.x)])
    body.append(facts(rows))

    const wire = held.groupsByTag.get(record.t)
    if (wire !== undefined) {
      const members = wire.m.map((tag) => held.byTag.get(tag)).filter(Boolean)
      const group = panel('Group members', `${wire.n} · ${wire.m.length}`)
      group.body.append(
        grid(
          ['tag', 'field', 'type', 'meaning'],
          members.map((member) => [
            member.t,
            titleOf(member),
            held.groupsByTag.has(member.t) ? 'group' : member.y,
            member.x ? make('span', 'ygg-fx__prose', member.x) : null,
          ]),
        ),
      )
      body.append(group.held)
    }

    // Which messages carry it: the question a reader has in front of a tag
    // they met in a capture, and one the layouts answer.
    const carriers = held.carriers(record.t)
    if (carriers.length > 0) {
      const where = panel('Carried by', `${carriers.length} message types`)
      const list = make('div', 'ygg-fx__chips')
      for (const message of carriers.slice(0, 80)) {
        list.append(pill(`${message.y} ${message.n}`))
      }
      if (carriers.length > 80) list.append(pill(`+${carriers.length - 80} more`, 'quiet'))
      where.body.append(list)
      body.append(where.held)
    }

    if (record.c || record.s !== undefined) {
      const extra = make('div')
      extra.append(note('Loading the code set…'))
      body.append(extra)
      codes().then(
        (all) => {
          extra.textContent = ''
          const found = all[id]
          if (found === undefined) return
          if (found.c) {
            const set = panel('Code set', `${found.c.length} values`, found.c.length <= 12)
            set.body.append(codeTable(found.c))
            extra.append(set.held)
          }
          if (found.l) {
            const line = panel('Lineage', `${found.l.length} versions`)
            line.body.append(lineageTable(found.l))
            extra.append(line.held)
          }
        },
        (error) => {
          extra.textContent = ''
          extra.append(note(`The code sets could not be loaded (${error.message}).`))
        },
      )
    }

    body.append(
      call(
        `registry.fieldByTag(${record.t})            // JavaScript\n` +
          `registry.field_by_tag(${record.t})?         // Rust\n` +
          `registry.field_by_tag(${record.t})          # Python\n` +
          `cargo run -p yggdryl-cli -- --root config/fix show ${record.t}`,
      ),
    )
    return body
  }

  // --------------------------------------------------------------- renderers

  /** The dictionary in one row of numbers. */
  function renderKpi(root, held) {
    const kpi = held.data.kpi
    const spec = held.data.spec
    root.append(
      cards([
        [kpi.fields.toLocaleString(), 'fields', `${kpi.primitives.toLocaleString()} flat`],
        [kpi.groups.toLocaleString(), 'repeating groups', 'nested tree'],
        [kpi.codes.toLocaleString(), 'codes', `in ${kpi.codeSets.toLocaleString()} sets`],
        [kpi.messages.toLocaleString(), 'message types', `${kpi.components} components`],
        [kpi.lineage.toLocaleString(), 'dated fields', `${kpi.versions} versions`],
        [kpi.columns.toLocaleString(), 'fixed columns', 'one capture row'],
        [String(spec.version), 'specification', `EP${spec.ep}`],
        [String(spec.shards), 'shards', `${spec.sources.length} sources`],
      ]),
    )

    const shape = panel('Datatypes the dictionary uses', `${kpi.datatypes} distinct`)
    shape.body.append(
      grid(
        ['datatype', 'fields', 'share'],
        kpi.dtypes
          .slice(0, 24)
          .map(([name, count]) => [
            name,
            count,
            `${((count / kpi.fields) * 100).toFixed(1)}%`,
          ]),
      ),
    )
    root.append(shape.held)

    const branches = panel('Branches', `${kpi.branches}`)
    branches.body.append(
      grid(
        ['branch', 'fields'],
        kpi.branchSizes.map(([name, count]) => [name === '' ? 'standard' : name, count]),
      ),
    )
    root.append(branches.held)

    const versions = panel('Versions a lineage names', `${kpi.versions}`)
    const chips = make('div', 'ygg-fx__chips')
    for (const version of kpi.versionList) chips.append(pill(version))
    versions.body.append(chips)
    root.append(versions.held)
  }

  /** Where the dictionary came from, with the checksums that pin it. */
  function renderSources(root, held) {
    const spec = held.data.spec
    root.append(
      grid(
        ['source', 'format', 'version', 'sha256', 'licence'],
        spec.sources.map((source) => {
          const link = make('a', null, source.id)
          link.href = source.url
          link.rel = 'noreferrer'
          const licence = make('a', null, 'licence')
          licence.href = source.license
          licence.rel = 'noreferrer'
          return [link, source.format, source.version, source.sha256.slice(0, 16), licence]
        }),
      ),
    )
    root.append(
      note(
        `Every source is pinned to a commit and checked by its digest, so FIX ${spec.version} EP${spec.ep} regenerates byte for byte.`,
      ),
    )
  }

  /**
   * The dictionary, searched.
   *
   * Six thousand fields is more than anyone scrolls, so the filter is the
   * view: tag, name, alias and wording alike, narrowed by datatype, by kind
   * and by whether the field carries a vocabulary.
   */
  function renderFields(root, held) {
    const controls = make('div', 'ygg-fx__controls')
    const box = make('input', 'ygg-fx__input ygg-fx__input--wide')
    box.type = 'search'
    box.placeholder = 'symbol, 55, ClOrdID, "settlement date"…'
    box.autocomplete = 'off'
    const kind = make('select', 'ygg-fx__select')
    for (const [value, label] of [
      ['all', 'Every field'],
      ['group', 'Repeating groups'],
      ['coded', 'With a code set'],
      ['dated', 'With a lineage'],
      ['crate', 'The capture’s own'],
    ]) {
      kind.append(new Option(label, value))
    }
    const dtype = make('select', 'ygg-fx__select')
    dtype.append(new Option('Any datatype', 'any'))
    for (const [name, count] of held.data.kpi.dtypes.slice(0, 30)) {
      dtype.append(new Option(`${name} (${count})`, name))
    }
    controls.append(
      control('ygg-fx-search', 'Search', box),
      control('ygg-fx-kind', 'Kind', kind),
      control('ygg-fx-dtype', 'Datatype', dtype),
    )

    const counter = make('p', 'ygg-fx__counter')
    counter.setAttribute('aria-live', 'polite')
    const view = make('div', 'ygg-fx__list')
    const more = button('Show more')
    more.hidden = true

    const LIMIT = 60
    let shown = LIMIT
    let kept = []

    const matches = () => {
      const wanted = box.value.trim().toLowerCase()
      const words = wanted.split(/\s+/).filter(Boolean)
      const wantKind = kind.value
      const wantType = dtype.value
      return held.data.fields.filter((record) => {
        if (wantKind === 'group' && record.k !== 'group') return false
        if (wantKind === 'coded' && !record.c) return false
        if (wantKind === 'dated' && record.s === undefined) return false
        if (wantKind === 'crate' && (record.b ?? '') === '') return false
        if (wantType !== 'any' && record.y !== wantType) return false
        if (words.length === 0) return true
        const hay = held.haystacks.get(record)
        return words.every((word) => hay.includes(word))
      })
    }

    const rowFor = (record) => {
      const entry = panel(
        `${record.t}`,
        titleOf(record),
      )
      entry.held.classList.add('ygg-fx__entry')
      entry.summary.classList.add('ygg-fx__entry-summary')
      const marks = make('span', 'ygg-fx__marks')
      const isGroup = record.k === 'group' || held.groupsByTag.has(record.t)
      marks.append(pill(isGroup ? 'group' : record.y, isGroup ? 'group' : null))
      if (record.c) marks.append(pill(`${record.c} codes`, 'codes'))
      if ((record.b ?? '') !== '') marks.append(pill(record.b, 'branch'))
      if (record.s !== undefined) marks.append(pill(`since ${record.s}`, 'quiet'))
      entry.summary.append(marks)
      let filled = false
      entry.held.addEventListener('toggle', () => {
        if (!entry.held.open || filled) return
        filled = true
        entry.body.append(fieldDetail(held, record))
      })
      return entry.held
    }

    const show = () => {
      kept = matches()
      counter.textContent = `${kept.length.toLocaleString()} of ${held.data.kpi.fields.toLocaleString()} fields`
      view.textContent = ''
      for (const record of kept.slice(0, shown)) view.append(rowFor(record))
      more.hidden = kept.length <= shown
      more.textContent = `Show ${Math.min(LIMIT, kept.length - shown)} more of ${kept.length - shown}`
    }

    const reset = () => {
      shown = LIMIT
      show()
    }
    box.addEventListener('input', reset)
    kind.addEventListener('change', reset)
    dtype.addEventListener('change', reset)
    more.addEventListener('click', () => {
      shown += LIMIT
      show()
    })

    root.append(controls, counter, view, more)
    show()
  }

  /** One message layout as a tree, opened where a reader looks. */
  function layoutTree(held, members, depth = 0, seen = new Set()) {
    const list = make('ul', 'ygg-fx__tree')
    list.setAttribute('role', 'list')
    for (const [kind, id, required] of members) {
      const item = make('li', 'ygg-fx__tree-item')
      if (kind === 'f') {
        const record = held.byTag.get(id)
        const line = make('div', 'ygg-fx__tree-line')
        line.append(code(String(id)))
        line.append(make('span', 'ygg-fx__tree-name', record ? titleOf(record) : 'unknown'))
        if (record) line.append(pill(record.k === 'group' ? 'group' : record.y))
        if (required === 1) line.append(pill('required', 'required'))
        item.append(line)
      } else if (kind === 'c') {
        const component = held.components.get(id)
        if (component === undefined || seen.has(`c${id}`)) continue
        const block = panel(component.n, `component · ${component.m.length} members`)
        block.held.classList.add('ygg-fx__block')
        const next = new Set(seen).add(`c${id}`)
        let filled = false
        block.held.addEventListener('toggle', () => {
          if (!block.held.open || filled) return
          filled = true
          block.body.append(layoutTree(held, component.m, depth + 1, next))
        })
        if (required === 1) block.summary.append(pill('required', 'required'))
        item.append(block.held)
      } else {
        const group = held.layoutGroups.get(id)
        if (group === undefined || seen.has(`g${id}`)) continue
        const record = held.byTag.get(group.t)
        const block = panel(group.n, `group · counter ${group.t} · ${group.m.length} members`)
        block.held.classList.add('ygg-fx__block', 'ygg-fx__block--group')
        const next = new Set(seen).add(`g${id}`)
        let filled = false
        block.held.addEventListener('toggle', () => {
          if (!block.held.open || filled) return
          filled = true
          if (record && record.x) block.body.append(make('p', 'ygg-fx__prose', record.x))
          block.body.append(layoutTree(held, group.m, depth + 1, next))
        })
        if (required === 1) block.summary.append(pill('required', 'required'))
        item.append(block.held)
      }
      list.append(item)
    }
    return list
  }

  /** Every message type, and the layout each one declares. */
  function renderMessages(root, held) {
    const controls = make('div', 'ygg-fx__controls')
    const box = make('input', 'ygg-fx__input')
    box.type = 'search'
    box.placeholder = 'NewOrderSingle, D, execution…'
    box.autocomplete = 'off'
    const chooser = make('select', 'ygg-fx__select ygg-fx__select--wide')
    controls.append(control('ygg-fx-message-filter', 'Filter', box))
    controls.append(control('ygg-fx-message', 'Message', chooser))

    const summary = make('div')
    const view = make('div')

    const fill = () => {
      const wanted = box.value.trim().toLowerCase()
      const kept = held.data.messages.filter(
        (message) =>
          wanted === '' ||
          message.n.toLowerCase().includes(wanted) ||
          message.y.toLowerCase() === wanted,
      )
      const current = chooser.value
      chooser.textContent = ''
      for (const message of kept) chooser.append(new Option(`${message.n} (${message.y})`, message.y))
      if (kept.some((message) => message.y === current)) chooser.value = current
      show()
    }

    const show = () => {
      const message = held.messages.get(chooser.value)
      summary.textContent = ''
      view.textContent = ''
      if (message === undefined) {
        summary.append(note('No message type matches that filter.'))
        return
      }
      const tags = layoutTags(held, message)
      const groups = tags.filter((entry) => entry.group !== undefined)
      summary.append(
        cards([
          [message.y, 'message type', message.n],
          [String(tags.length), 'fields reached', 'through every block'],
          [String(tags.filter((entry) => entry.required).length), 'required', 'by the layout'],
          [String(groups.length), 'repeating groups', 'nested'],
          [String(message.m.filter(([kind]) => kind === 'c').length), 'components', 'at the top level'],
        ]),
      )
      view.append(layoutTree(held, message.m))
    }

    box.addEventListener('input', fill)
    chooser.addEventListener('change', show)
    root.append(controls, summary, view)
    fill()
  }

  /** The fixed row a capture lands in, filtered as you type. */
  function renderRow(root, held) {
    const row = held.data.row
    const controls = make('div', 'ygg-fx__controls')
    const box = make('input', 'ygg-fx__input ygg-fx__input--wide')
    box.type = 'search'
    box.placeholder = 'symbol, 55, price…'
    box.autocomplete = 'off'
    controls.append(control('ygg-fx-row', 'Filter', box))
    const counter = make('p', 'ygg-fx__counter')
    counter.setAttribute('aria-live', 'polite')
    const view = make('div')

    const show = () => {
      const wanted = box.value.trim().toLowerCase()
      const kept = row.columns.filter(
        (column) =>
          wanted === '' ||
          column.c.includes(wanted) ||
          column.n.toLowerCase().includes(wanted) ||
          column.x.toLowerCase().includes(wanted),
      )
      const own = row.columns.filter(
        (column) => (held.byTag.get(column.t)?.b ?? '') !== '',
      ).length
      counter.textContent =
        `${kept.length} of ${row.columns.length} columns` +
        `, ${own} of them the capture's own`
      view.textContent = ''
      view.append(
        grid(
          ['column', 'field', 'type', 'wording'],
          kept.map((column) => [
            column.c,
            column.n,
            column.y,
            column.x === '' ? null : make('span', 'ygg-fx__prose', column.x),
          ]),
        ),
      )
    }
    box.addEventListener('input', show)
    root.append(controls, counter, view, call(row.call))
    show()
  }

  // ----------------------------------------------------------------- decoder

  /** One entry of a read frame, as a row of the decode grid. */
  function decodeRows(held, entries, all, expected, depth = 0) {
    const rows = []
    for (const entry of entries) {
      const record = entry.record
      const name = make('span', depth > 0 ? 'ygg-fx__tree-name ygg-fx__nested' : 'ygg-fx__tree-name')
      if (depth > 1) name.style.marginLeft = `${(depth - 1) * 0.8}em`
      name.textContent = record === null ? entry.key : titleOf(record)
      const marks = make('span', 'ygg-fx__marks')
      if (record === null) marks.append(pill('unknown', 'unknown'))
      else {
        if (held.header.has(record.t)) marks.append(pill('header', 'quiet'))
        if (held.trailer.has(record.t)) marks.append(pill('trailer', 'quiet'))
        if (expected.required.has(record.t)) marks.append(pill('required', 'required'))
      }
      // What was gathered, not what the registry nests: a counter the
      // dictionary stores flat still introduced a group on the wire.
      if (entry.occurrences !== undefined) {
        marks.append(pill(`${entry.occurrences.length} occurrences`, 'group'))
      }
      const meaning = make('span', 'ygg-fx__meaning')
      const found = record === null ? undefined : all?.[`${record.t}:${record.b ?? ''}`]
      const codeFor = found?.c?.find(([value]) => value === entry.value)
      if (codeFor !== undefined) {
        meaning.append(make('strong', null, codeFor[1]))
        if (codeFor[2]) meaning.append(document.createTextNode(` — ${codeFor[2]}`))
      } else {
        const said = reading(record, entry.value)
        if (said !== null) meaning.append(code(said))
        else if (record !== null && record.x) meaning.append(make('span', 'ygg-fx__prose', record.x))
      }
      rows.push([
        entry.tag === null ? entry.key : String(entry.tag),
        name,
        showable(entry.value),
        record === null ? 'unknown' : record.k === 'group' ? 'group' : record.y,
        marks,
        meaning,
      ])
      for (const occurrence of entry.occurrences ?? []) {
        rows.push(...decodeRows(held, occurrence.members, all, expected, depth + 1))
      }
    }
    return rows
  }

  /** The live decoder: a frame in, an explanation out. */
  function renderDecode(root, held) {
    const form = make('div', 'ygg-fx__editor')
    const box = make('textarea', 'ygg-fx__textarea')
    box.rows = 4
    box.spellcheck = false
    box.placeholder = '8=FIX.4.4|35=D|55=AAPL|54=1|38=100|44=10.5|10=000|'
    form.append(control('ygg-fx-decode', 'A FIX frame — pipes, SOH, ^A or newlines', box))

    const presets = make('div', 'ygg-fx__chips')
    for (const frame of held.data.frames) {
      const chip = button(frame.label, 'chip')
      chip.addEventListener('click', () => {
        box.value = unescaped(frame.line)
        show()
      })
      presets.append(chip)
    }

    const summary = make('div')
    const view = make('div')
    let all = null
    codes().then((held_) => {
      all = held_
      show()
    }, () => {})

    const show = () => {
      const text = box.value
      summary.textContent = ''
      view.textContent = ''
      if (text.trim() === '') {
        summary.append(note('Paste a frame, or pick one of the captures above.'))
        return
      }
      const frame = read(held, text)
      const expected = expectations(held, frame.msgtype)
      const missing = [...expected.required].filter(
        (tag) => !frame.pairs.some((entry) => entry.tag === tag),
      )
      const groups = frame.tree.filter((entry) => entry.occurrences !== undefined)

      summary.append(
        cards([
          [frame.msgtype ?? '—', 'message type', expected.message ? expected.message.n : 'not in the dictionary'],
          [frame.version ?? '—', 'begin string', frame.direction ?? 'no direction stated'],
          [String(frame.pairs.length), 'pairs', `${frame.pairs.length - frame.unknown} named`],
          [String(frame.unknown), 'unexplained', frame.unknown === 0 ? 'every tag resolved' : 'no dictionary names them'],
          [String(groups.length), 'repeating groups', `${groups.reduce((total, entry) => total + entry.occurrences.length, 0)} occurrences`],
          [String(missing.length), 'required missing', expected.message ? 'against its layout' : 'no layout to check'],
        ]),
      )

      // The two self-describing tags, recomputed from the bytes.
      if (frame.checks.length > 0) {
        const holder = make('div', 'ygg-fx__checks')
        for (const check of frame.checks) {
          const item = make('div', check.ok ? 'ygg-fx__check ygg-fx__check--ok' : 'ygg-fx__check ygg-fx__check--bad')
          item.append(make('span', 'ygg-fx__check-name', check.name))
          item.append(
            make(
              'span',
              'ygg-fx__check-body',
              check.ok ? `${check.stated} — agrees` : `stated ${check.stated}, computed ${check.computed}`,
            ),
          )
          holder.append(item)
        }
        summary.append(holder)
      }

      const decoded = panel('Field by field', `${frame.pairs.length} pairs`, true)
      decoded.body.append(
        grid(
          ['tag', 'field', 'value', 'type', '', 'meaning'],
          decodeRows(held, frame.tree, all, expected),
          'ygg-fx__grid--decode',
        ),
      )
      view.append(decoded.held)

      if (missing.length > 0) {
        const gap = panel('Required by the layout and not sent', `${missing.length}`)
        gap.body.append(
          grid(
            ['tag', 'field', 'type'],
            missing.map((tag) => {
              const record = held.byTag.get(tag)
              return [tag, record ? titleOf(record) : 'unknown', record ? record.y : null]
            }),
          ),
        )
        view.append(gap.held)
      }

      if (frame.unknown > 0) {
        const strange = panel('What no dictionary explains', `${frame.unknown}`)
        strange.body.append(
          grid(
            ['key', 'value'],
            frame.pairs.filter((entry) => entry.record === null).map((entry) => [entry.key, showable(entry.value)]),
          ),
        )
        view.append(strange.held)
      }

      // Where the corpus holds this exact line, the package's own answer sits
      // beside the reading, which is what makes the reading checkable.
      const escaped = held.data.frames.find((frame_) => unescaped(frame_.line) === text.trim())
      if (escaped !== undefined) view.append(packageAnswer(escaped))
      else {
        view.append(
          note(
            'This frame is not in the generated corpus, so the typed row, the digest,' +
              ' the derived facets and the anomalies above are not shown: those are the' +
              ' package’s answers and this page computes none of them. Read it with the' +
              ' package to see them.',
          ),
        )
        view.append(call(`new fix.FixCodec(registry).text(${JSON.stringify(text.trim())})`))
      }
    }

    box.addEventListener('input', show)
    root.append(form, presets, summary, view)

    // A frame handed over by the composer, so the two pages are one workflow.
    const handed = sessionStorage.getItem('ygg-fx-frame')
    if (handed !== null) {
      sessionStorage.removeItem('ygg-fx-frame')
      box.value = handed
    } else if (held.data.frames.length > 0) {
      box.value = unescaped(held.data.frames[0].line)
    }
    show()
  }

  /** What the package answered for one corpus frame. */
  function packageAnswer(frame) {
    const held = panel('What the package answered', frame.label, true)
    held.held.classList.add('ygg-fx__package')
    held.body.append(
      facts([
        ['message', frame.root],
        ['media type', frame.mime],
        ['message type', frame.msgtype],
        ['direction', frame.direction],
        ['branch', frame.branch === '' ? 'standard' : frame.branch],
        ['fields', String(frame.size)],
        ['digest', frame.digest],
        ['symbol ticker', frame.ticker],
        ['market timestamp', frame.clock],
        ['unix partition', frame.partition],
      ]),
    )
    if (frame.columns.length > 0) {
      const row = panel('The columns it filled', `${frame.columns.length} of 89`)
      row.body.append(
        grid(
          ['tag', 'column', 'type', 'value'],
          frame.columns.map((column) => [column.t, column.n, column.y, column.v]),
        ),
      )
      held.body.append(row.held)
    }
    if (frame.lift.length > 0) {
      const lifted = panel('What it derived', `${frame.lift.length} facets`)
      lifted.body.append(
        grid(
          ['facet', 'value', 'read from'],
          frame.lift.map(([facet, value, source]) => [facet, value, source]),
        ),
      )
      held.body.append(lifted.held)
    }
    if (frame.anomalies.length > 0) {
      const odd = panel('What does not add up', `${frame.anomalies.length}`)
      odd.body.append(
        grid(['anomaly'], frame.anomalies.map((text) => [make('span', 'ygg-fx__prose', text)])),
      )
      held.body.append(odd.held)
    }
    if (frame.emitted !== '') {
      const again = panel('Re-emitted from its entries', 'the wire record')
      again.body.append(wire(unescaped(frame.emitted).replaceAll(SOH, '|')).held)
      held.body.append(again.held)
    }
    held.body.append(call(frame.call))
    return held.held
  }

  /** Every frame the package read, one at a time. */
  function renderFrames(root, held) {
    const chips = make('div', 'ygg-fx__chips')
    const view = make('div')
    const buttons = []
    const show = (at) => {
      buttons.forEach((chip, index) => chip.setAttribute('aria-pressed', index === at ? 'true' : 'false'))
      const frame = held.data.frames[at]
      view.textContent = ''
      view.append(wire(unescaped(frame.line).replaceAll(SOH, '|')).held)
      view.append(packageAnswer(frame))
    }
    held.data.frames.forEach((frame, at) => {
      const chip = button(frame.label, 'chip')
      chip.setAttribute('aria-pressed', 'false')
      chip.addEventListener('click', () => show(at))
      buttons.push(chip)
      chips.append(chip)
    })
    root.append(chips, view)
    show(0)
  }

  // ---------------------------------------------------------------- composer

  /**
   * The composer: a message type, its layout, and the frame it makes.
   *
   * BodyLength and CheckSum are computed from the bytes the composer wrote,
   * which is the arithmetic FIX states about itself; every name, datatype and
   * code offered comes from the manifest.
   */
  function renderEncode(root, held) {
    const controls = make('div', 'ygg-fx__controls')
    const chooser = make('select', 'ygg-fx__select ygg-fx__select--wide')
    for (const message of held.data.messages) {
      chooser.append(new Option(`${message.n} (${message.y})`, message.y))
    }
    chooser.value = 'D'
    const scope = make('select', 'ygg-fx__select')
    // Not "every field the layout reaches": a NewOrderSingle reaches nearly
    // three thousand tags through its components, and a form of three
    // thousand inputs is not a form. Anything outside these is added by name
    // or by tag through the box beside them.
    for (const [value, label] of [
      ['common', 'Required and common'],
      ['required', 'Required only'],
      ['declared', 'Declared by the message itself'],
    ]) {
      scope.append(new Option(label, value))
    }
    const separator = make('select', 'ygg-fx__select')
    for (const [value, label] of [
      ['|', 'Pipe (readable)'],
      ['soh', 'SOH (the wire)'],
      ['^A', 'Caret-A (logs)'],
    ]) {
      separator.append(new Option(label, value))
    }
    const extra = make('input', 'ygg-fx__input')
    extra.type = 'search'
    extra.placeholder = 'a tag or a name, then Enter'
    extra.autocomplete = 'off'
    const added = new Set()
    const said = make('p', 'ygg-fx__counter')
    said.setAttribute('aria-live', 'polite')
    extra.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return
      event.preventDefault()
      const wanted = extra.value.trim()
      if (wanted === '') return
      const record = held.field(wanted)
      extra.value = ''
      if (record === null) {
        said.textContent = `No field is named ${wanted}.`
        return
      }
      if (held.groupsByTag.has(record.t)) {
        said.textContent = `${titleOf(record)} (${record.t}) is a repeating group; the composer writes flat tags.`
        return
      }
      said.textContent = `${titleOf(record)} (${record.t}) added.`
      added.add(record.t)
      values.set(record.t, values.get(record.t) ?? '')
      build()
    })
    controls.append(
      control('ygg-fx-encode-message', 'Message', chooser),
      control('ygg-fx-encode-scope', 'Show', scope),
      control('ygg-fx-encode-extra', 'Add a field', extra),
      control('ygg-fx-encode-separator', 'Separator', separator),
    )

    const form = make('div', 'ygg-fx__form')
    const out = make('div')
    const values = new Map()
    // The tags the form currently draws. A value is remembered when a field
    // leaves the form and written again when it comes back, but never written
    // into a frame the reader cannot see it in.
    let shown = []

    // A frame states its version, type, parties and clock whatever it is, so
    // the composer opens with them rather than with an empty form.
    const SEEDS = new Map([
      [8, 'FIX.4.4'],
      [49, 'BUYSIDE'],
      [56, 'VENUE'],
      [34, '1'],
      [52, '20240201-12:34:56.000'],
      [11, 'ORDER-1'],
      [55, 'AAPL'],
      [54, '1'],
      [38, '100'],
      [44, '10.5'],
      [40, '2'],
      [60, '20240201-12:34:56.123'],
    ])

    /** The fields the form offers for one message type. */
    const offered = () => {
      const message = held.messages.get(chooser.value)
      if (message === undefined) return []
      const wanted = scope.value
      const tags =
        wanted === 'declared'
          ? message.m
              .filter(([kind]) => kind === 'f')
              .map(([, id, required]) => ({ tag: id, required: required === 1 }))
          : layoutTags(held, message)
      const seen = new Set()
      const kept = []
      for (const entry of tags) {
        if (seen.has(entry.tag)) continue
        const record = held.byTag.get(entry.tag)
        if (record === undefined || held.groupsByTag.has(entry.tag)) continue
        // The two self-describing tags are computed from the bytes, so they
        // are shown in the frame and never asked for.
        if (entry.tag === 9 || held.trailer.has(entry.tag)) continue
        // A tag asked for by name is offered wherever the layout puts it -
        // marking it seen before deciding to keep it is what used to make the
        // control answer only for tags the layout never reaches.
        const wants =
          added.has(entry.tag) ||
          wanted === 'declared' ||
          entry.required ||
          (wanted === 'common' && SEEDS.has(entry.tag))
        if (!wants) continue
        seen.add(entry.tag)
        kept.push({ tag: entry.tag, required: entry.required })
      }
      // And appended where the layout does not reach it at all.
      for (const tag of added) {
        if (seen.has(tag)) continue
        const record = held.byTag.get(tag)
        if (record === undefined || held.groupsByTag.has(tag)) continue
        seen.add(tag)
        kept.push({ tag, required: false })
      }
      return kept
    }

    const build = () => {
      form.textContent = ''
      const entries = offered()
      const message = held.messages.get(chooser.value)
      if (message !== undefined && !values.has(35)) values.set(35, message.y)
      values.set(35, chooser.value)
      codes().then((all) => paint(entries, all), () => paint(entries, null))
    }

    const paint = (entries, all) => {
      form.textContent = ''
      shown = entries.map((entry) => entry.tag)
      for (const entry of entries) {
        const record = held.byTag.get(entry.tag)
        const row = make('div', 'ygg-fx__field')
        const label = make('label', 'ygg-fx__field-label')
        label.setAttribute('for', `ygg-fx-tag-${entry.tag}`)
        label.append(code(String(entry.tag)), make('span', null, titleOf(record)))
        if (entry.required) label.append(pill('required', 'required'))
        const set = all?.[`${record.t}:${record.b ?? ''}`]?.c
        let input
        if (set !== undefined && set.length <= 60) {
          input = make('select', 'ygg-fx__select ygg-fx__select--wide')
          input.append(new Option('—', ''))
          for (const [value, name, doc] of set) {
            input.append(new Option(`${value} · ${name}${doc ? ` — ${doc}` : ''}`, value))
          }
        } else {
          input = make('input', 'ygg-fx__input ygg-fx__input--wide')
          input.type = 'text'
          input.autocomplete = 'off'
          input.placeholder = record.y
        }
        input.id = `ygg-fx-tag-${entry.tag}`
        const held_ = values.has(entry.tag) ? values.get(entry.tag) : (SEEDS.get(entry.tag) ?? '')
        input.value = held_
        values.set(entry.tag, held_)
        if (entry.tag === 35) {
          input.value = chooser.value
          input.disabled = true
        }
        input.addEventListener('input', () => {
          values.set(entry.tag, input.value)
          compose()
        })
        input.addEventListener('change', () => {
          values.set(entry.tag, input.value)
          compose()
        })
        row.append(label, input)
        if (record.x) row.append(make('p', 'ygg-fx__prose ygg-fx__field-note', record.x))
        form.append(row)
      }
      compose()
    }

    /** The frame, and the two tags that describe it. */
    const compose = () => {
      const message = held.messages.get(chooser.value)
      // Only what the form shows: a value typed under one message type must
      // not survive invisibly into a frame whose form has no row for it.
      const ordered = shown
        .filter((tag) => tag !== 8 && tag !== 9 && tag !== 10 && (values.get(tag) ?? '') !== '')
        .sort((left, right) => {
          const rank = (tag) => (held.header.has(tag) ? held.data.header.indexOf(tag) : 1000 + tag)
          return rank(left) - rank(right)
        })
      const body = ordered.map((tag) => `${tag}=${values.get(tag)}`)
      const begin = values.get(8) || 'FIX.4.4'
      const payload = `${body.join(SOH)}${SOH}`
      // Bytes, not characters: `CAFÉ` is five bytes on the wire and four
      // characters here, and the two tags count what the wire carries.
      const length = bytes(payload).length
      const framed = `8=${begin}${SOH}9=${length}${SOH}${payload}`
      const sum = checksum(bytes(framed), bytes(framed).length - 1)
      const full = `${framed}10=${sum}${SOH}`
      const written =
        separator.value === 'soh' ? showable(full) : full.replaceAll(SOH, separator.value)

      out.textContent = ''
      out.append(
        cards([
          [chooser.value, 'message type', message ? message.n : ''],
          [String(ordered.length + 3), 'tags', 'header, body, trailer'],
          [String(bytes(full).length), 'bytes', `body ${length}`],
          [sum, 'checksum', 'computed here'],
        ]),
      )
      // A value carrying the separator makes a boundary nobody meant, and the
      // frame stops reading back. Said once, where it is written.
      const clashes = ordered.filter((tag) => values.get(tag).includes(separator.value))
      if (separator.value !== 'soh' && clashes.length > 0) {
        out.append(
          note(
            `A value of ${clashes.join(', ')} carries the separator “${separator.value}”, so the` +
              ' frame below reads back as more fields than it holds. On the wire the separator' +
              ' is SOH and a value never carries one.',
          ),
        )
      }
      out.append(
        wire(written, 'ygg-fx__wire--out', separator.value === 'soh' ? full : undefined).held,
      )

      const actions = make('div', 'ygg-fx__actions')
      const decode = button('Read it in the decoder', 'primary')
      decode.addEventListener('click', () => {
        // The frame itself, SOH and all: the decoder reads that natively, and
        // handing over a substituted separator would hand over the clash.
        sessionStorage.setItem('ygg-fx-frame', full)
        // The two pages are siblings under `fix/`, so the decoder is one
        // segment across whatever prefix the site is served under.
        window.location.href = new URL('../decode/', window.location.href).href
      })
      actions.append(decode)
      out.append(
        actions,
        call(
          `const reader = new fix.FixCodec(registry)\n` +
            `reader.transformLine(Buffer.from(${JSON.stringify(full)}, 'binary')).toBytes(0x01)`,
        ),
      )
    }

    chooser.addEventListener('change', build)
    scope.addEventListener('change', build)
    separator.addEventListener('change', compose)
    root.append(controls, said, out, form)
    build()
  }

  // ------------------------------------------------------------------- start

  const RENDERERS = {
    kpi: renderKpi,
    sources: renderSources,
    fields: renderFields,
    messages: renderMessages,
    row: renderRow,
    decode: renderDecode,
    frames: renderFrames,
    encode: renderEncode,
  }

  /** Say what failed and how to put it back, rather than showing nothing. */
  function fail(roots, reason) {
    for (const root of roots) {
      root.textContent = ''
      root.append(
        note(
          `The generated manifest assets/fix.json could not be loaded (${reason}).` +
            ' It is committed, and a local build writes it with:',
        ),
        call(COMMAND),
      )
    }
  }

  function start() {
    const roots = [...document.querySelectorAll('[data-fix]')]
    if (roots.length === 0) return
    manifest().then(
      (held) => {
        for (const root of roots) {
          const renderer = RENDERERS[root.dataset.fix]
          if (renderer === undefined) continue
          root.textContent = ''
          root.classList.add('ygg-fx__ready')
          renderer(root, held)
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
