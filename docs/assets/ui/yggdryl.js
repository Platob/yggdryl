import {
  code,
  createLazyIndex,
  createSearchIndex,
  facts,
  make,
  pill,
  tree,
} from './index.js'

const folded = (text) => String(text).toLowerCase().replace(/[\s_-]+/g, '')

/** Display a package-produced DataType string without parsing or normalizing it. */
export function dataType({ value, className }) {
  return code({ text: value, className: className ? `ygg-ui__datatype ${className}` : 'ygg-ui__datatype' })
}

const nestedFields = (dtype) => {
  if (dtype === null || typeof dtype !== 'object') return []
  const children = []
  if (Array.isArray(dtype.fields)) {
    for (const entry of dtype.fields) {
      const field = entry && typeof entry === 'object' && 'field' in entry ? entry.field : entry
      if (field !== null && typeof field === 'object') children.push(field)
    }
  }
  for (const name of ['field', 'entries', 'run_ends', 'values']) {
    const field = dtype[name]
    if (field !== null && typeof field === 'object') children.push(field)
  }
  return children
}

const shownValue = (value) => {
  if (value === null || typeof value !== 'object') return value
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

const fieldFacts = (field) => {
  const dtype = field.dtype
  const rows = [
    { label: 'name', value: field.name },
    {
      label: 'datatype',
      value: dtype !== null && typeof dtype === 'object' ? dtype.type : dtype,
    },
    { label: 'nullable', value: field.nullable },
    { label: 'dictionary ID', value: field.dictionary_id },
    { label: 'dictionary ordered', value: field.dictionary_is_ordered },
  ]
  if (dtype !== null && typeof dtype === 'object') {
    for (const [name, value] of Object.entries(dtype)) {
      if (['type', 'fields', 'field', 'entries', 'run_ends', 'values'].includes(name)) continue
      rows.push({ label: `datatype.${name}`, value: shownValue(value) })
    }
  }
  if (field.metadata !== null && typeof field.metadata === 'object') {
    for (const [name, value] of Object.entries(field.metadata)) {
      rows.push({ label: `metadata.${name}`, value })
    }
  }
  return facts({ rows })
}

/** Map the structural object returned by Field.toJSON() onto a lazy field tree. */
export function fieldDocument({ document, open = false, className }) {
  const identities = new WeakMap()
  let nextIdentity = 0
  const identity = (field) => {
    let held = identities.get(field)
    if (held === undefined) {
      held = String(++nextIdentity)
      identities.set(field, held)
    }
    return held
  }
  return tree({
    items: [document],
    key: identity,
    className: className ? `ygg-ui__field-document ${className}` : 'ygg-ui__field-document',
    describe: (field) => {
      const dtype = field?.dtype
      const type = dtype !== null && typeof dtype === 'object' ? dtype.type : dtype
      return {
        label: [
          code({ text: field?.name ?? '(unnamed)', className: 'ygg-ui__field-name' }),
          pill({
            text: field?.nullable === false ? 'required' : 'nullable',
            kind: field?.nullable === false ? 'required' : 'quiet',
          }),
        ],
        aside: type ?? 'unknown datatype',
        branch: true,
        open: field === document && open,
        beforeChildren: () => fieldFacts(field ?? {}),
        children: () => nestedFields(dtype),
      }
    },
  })
}

/**
 * Index the compact generated FIX manifests without copying their 6,210 field
 * records. Every expensive secondary index is either built here once or behind
 * one createLazyIndex gate.
 */
export function fixManifest({ index, details = null }) {
  const byTag = new Map()
  const byName = new Map()
  const groupsByTag = new Map(
    Object.entries(index.wire).map(([tag, held]) => [Number(tag), held]),
  )
  for (const record of index.fields) {
    if (!byTag.has(record.t) || (record.b ?? '') === '') byTag.set(record.t, record)
    for (const name of [record.n, record.d, ...(record.a ?? [])]) {
      if (name === undefined) continue
      const key = folded(name)
      if (!byName.has(key)) byName.set(key, record)
    }
  }

  const messages = new Map(index.messages.map((held) => [held.y, held]))
  const components = new Map(index.components.map((held) => [held.i, held]))
  const layoutGroups = new Map(index.groups.map((held) => [held.i, held]))
  const header = new Set(index.header)
  const trailer = new Set(index.trailer)
  const scopes = new Map()
  const searchIndex = createSearchIndex({
    items: index.fields,
    haystack: (record) =>
      `${record.t} ${record.n} ${record.d ?? ''} ${(record.a ?? []).join(' ')} ${record.x ?? ''}`,
  })

  const field = (key) => {
    const text = String(key)
    const bare = text.startsWith('#') ? text.slice(1) : text
    if (/^\d+$/.test(bare)) return byTag.get(Number(bare)) ?? null
    return byName.get(folded(bare)) ?? null
  }

  const groupsOf = (msgtype) => {
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
  }

  const layoutTags = (message, seen = new Set()) => {
    const held = typeof message === 'string' ? messages.get(message) : message
    if (held === undefined) return []
    const tags = []
    const walk = (members, depth, inside) => {
      for (const [kind, id, required] of members) {
        const requiredInside = inside && required === 1
        if (kind === 'f') {
          tags.push({ tag: id, required: requiredInside, depth })
        } else if (kind === 'c') {
          const component = components.get(id)
          if (component && !seen.has(`c${id}`)) {
            seen.add(`c${id}`)
            walk(component.m, depth + 1, requiredInside)
          }
        } else if (kind === 'g') {
          const group = layoutGroups.get(id)
          if (group && !seen.has(`g${id}`)) {
            seen.add(`g${id}`)
            tags.push({ tag: group.t, required: requiredInside, depth, group })
            walk(group.m, depth + 1, requiredInside)
          }
        }
      }
    }
    walk(held.m, 0, true)
    return tags
  }

  const carrierIndex = createLazyIndex({
    build: () => {
      const found = new Map()
      for (const message of index.messages) {
        for (const entry of layoutTags(message)) {
          const kept = found.get(entry.tag)
          if (kept === undefined) found.set(entry.tag, [message])
          else if (kept[kept.length - 1] !== message) kept.push(message)
        }
      }
      return found
    },
  })
  const carriers = (tag) => carrierIndex.get().get(tag) ?? []
  const titleOf = (record) => record.d ?? record.n
  const detail = (record, source = details) => {
    if (source === null || source === undefined) return null
    const id =
      typeof record === 'string'
        ? record
        : `${typeof record === 'number' ? record : record.t}:${typeof record === 'object' ? (record.b ?? '') : ''}`
    return source[id] ?? null
  }

  const layoutTree = (members) =>
    tree({
      items: members,
      key: ([kind, id]) => `${kind}${id}`,
      describe: ([kind, id, required]) => {
        if (kind === 'f') {
          const record = byTag.get(id)
          const label = make('span', 'ygg-ui__fix-field')
          label.append(code({ text: String(id) }))
          label.append(record ? titleOf(record) : 'unknown')
          if (record) {
            const isGroup = record.k === 'group' || groupsByTag.has(record.t)
            label.append(pill({ text: isGroup ? 'group' : record.y, kind: isGroup ? 'group' : undefined }))
          }
          if (required === 1) label.append(pill({ text: 'required', kind: 'required' }))
          return { label }
        }
        if (kind === 'c') {
          const component = components.get(id)
          if (component === undefined) return null
          return {
            label:
              required === 1
                ? [component.n, pill({ text: 'required', kind: 'required' })]
                : component.n,
            aside: `component · ${component.m.length} members`,
            branch: true,
            children: () => component.m,
          }
        }
        const group = layoutGroups.get(id)
        if (group === undefined) return null
        const record = byTag.get(group.t)
        return {
          label:
            required === 1
              ? [group.n, pill({ text: 'required', kind: 'required' })]
              : group.n,
          aside: `group · counter ${group.t} · ${group.m.length} members`,
          branch: true,
          className: 'ygg-ui__fix-group',
          children: () => group.m,
          beforeChildren: () => {
            const content = []
            if (record?.x) content.push(make('p', 'ygg-ui__prose', record.x))
            return content
          },
        }
      },
    })

  return {
    data: index,
    details,
    byTag,
    byName,
    groupsByTag,
    messages,
    components,
    layoutGroups,
    header,
    trailer,
    searchIndex,
    field,
    groupsOf,
    carriers,
    layoutTags,
    layoutTree,
    detail,
    titleOf,
  }
}


