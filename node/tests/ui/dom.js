'use strict'

class FakeEventTarget {
  constructor() {
    this.listeners = new Map()
  }

  addEventListener(type, listener, options = {}) {
    const held = this.listeners.get(type) ?? []
    held.push({ listener, once: options === true || options?.once === true })
    this.listeners.set(type, held)
  }

  removeEventListener(type, listener) {
    const held = this.listeners.get(type) ?? []
    this.listeners.set(
      type,
      held.filter((entry) => entry.listener !== listener),
    )
  }

  dispatchEvent(event) {
    const value = typeof event === 'string' ? { type: event } : event
    value.target ??= this
    value.currentTarget = this
    value.preventDefault ??= () => {
      value.defaultPrevented = true
    }
    for (const entry of [...(this.listeners.get(value.type) ?? [])]) {
      entry.listener.call(this, value)
      if (entry.once) this.removeEventListener(value.type, entry.listener)
    }
    return !value.defaultPrevented
  }
}

class FakeNode extends FakeEventTarget {
  constructor(ownerDocument, nodeType) {
    super()
    this.ownerDocument = ownerDocument
    this.nodeType = nodeType
    this.parentNode = null
  }

  remove() {
    if (this.parentNode === null) return
    const at = this.parentNode.childNodes.indexOf(this)
    if (at !== -1) this.parentNode.childNodes.splice(at, 1)
    this.parentNode = null
  }
}

class FakeText extends FakeNode {
  constructor(ownerDocument, text) {
    super(ownerDocument, 3)
    this.data = String(text)
  }

  get textContent() {
    return this.data
  }

  set textContent(value) {
    this.data = String(value ?? '')
  }
}

class FakeElement extends FakeNode {
  constructor(ownerDocument, tag) {
    super(ownerDocument, 1)
    this.tagName = String(tag).toUpperCase()
    this.childNodes = []
    this.attributes = new Map()
    this.style = {}
    this.className = ''
    this.value = ''
    this.open = false
    this.hidden = false
    this.disabled = false
    this.selected = false
  }

  get children() {
    return this.childNodes.filter((node) => node.nodeType === 1)
  }

  get firstChild() {
    return this.childNodes[0] ?? null
  }

  get parentElement() {
    return this.parentNode?.nodeType === 1 ? this.parentNode : null
  }

  get textContent() {
    return this.childNodes.map((node) => node.textContent).join('')
  }

  set textContent(value) {
    for (const child of this.childNodes) child.parentNode = null
    this.childNodes = []
    if (value !== null && value !== undefined && String(value) !== '') {
      this.append(this.ownerDocument.createTextNode(String(value)))
    }
  }

  get classList() {
    return {
      add: (...names) => {
        const held = new Set(this.className.split(/\s+/).filter(Boolean))
        for (const name of names) held.add(name)
        this.className = [...held].join(' ')
      },
      contains: (name) => this.className.split(/\s+/).includes(name),
    }
  }

  append(...values) {
    for (let value of values) {
      if (value === null || value === undefined) continue
      if (typeof value !== 'object' || typeof value.nodeType !== 'number') {
        value = this.ownerDocument.createTextNode(String(value))
      }
      value.remove?.()
      value.parentNode = this
      this.childNodes.push(value)
    }
  }

  appendChild(value) {
    this.append(value)
    return value
  }

  replaceChildren(...values) {
    this.textContent = ''
    this.append(...values)
  }

  setAttribute(name, value) {
    this.attributes.set(String(name), String(value))
  }

  getAttribute(name) {
    return this.attributes.get(String(name)) ?? null
  }

  hasAttribute(name) {
    return this.attributes.has(String(name))
  }

  removeAttribute(name) {
    this.attributes.delete(String(name))
  }

  focus() {
    this.ownerDocument.activeElement = this
  }

  select() {
    this.ownerDocument.selection = this.value
  }
}

class FakeDocument extends FakeEventTarget {
  constructor({ dialog = false } = {}) {
    super()
    this.nodeType = 9
    this.readyState = 'complete'
    this.activeElement = null
    this.selection = null
    this.body = new FakeElement(this, 'body')
    this.copyResult = false
    this.copyCalls = 0
    this.dialog = dialog
  }

  createElement(tag) {
    const element = new FakeElement(this, tag)
    if (this.dialog && String(tag).toLowerCase() === 'dialog') {
      element.returnValue = ''
      element.showModalCalls = 0
      element.showModal = () => {
        if (element.open) throw new Error('dialog is already open')
        element.showModalCalls += 1
        element.open = true
      }
      element.close = (returnValue = '') => {
        if (!element.open) return
        element.returnValue = String(returnValue)
        element.open = false
        element.dispatchEvent({ type: 'close' })
      }
    }
    return element
  }

  createTextNode(text) {
    return new FakeText(this, text)
  }

  execCommand(command) {
    if (command !== 'copy') return false
    this.copyCalls += 1
    return this.copyResult
  }
}

const findAll = (root, predicate) => {
  const found = []
  const visit = (node) => {
    if (predicate(node)) found.push(node)
    for (const child of node.childNodes ?? []) visit(child)
  }
  visit(root)
  return found
}

const byTag = (root, tag) =>
  findAll(root, (node) => node.tagName === String(tag).toUpperCase())

const byClass = (root, name) =>
  findAll(root, (node) => node.className?.split(/\s+/).includes(name))

const replaceGlobal = (name, value) => {
  const before = Object.getOwnPropertyDescriptor(globalThis, name)
  Object.defineProperty(globalThis, name, {
    value,
    writable: true,
    configurable: true,
    enumerable: before?.enumerable ?? false,
  })
  return () => {
    if (before === undefined) delete globalThis[name]
    else Object.defineProperty(globalThis, name, before)
  }
}

const installDOM = ({ navigator = {}, dialog = false } = {}) => {
  const document = new FakeDocument({ dialog })
  const restoreDocument = replaceGlobal('document', document)
  const restoreNavigator = replaceGlobal('navigator', navigator)
  return {
    document,
    restore() {
      restoreNavigator()
      restoreDocument()
    },
  }
}

module.exports = { FakeDocument, FakeElement, byClass, byTag, findAll, installDOM }
