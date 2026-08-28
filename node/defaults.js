'use strict'

// Runtime defaults are native, schema-directed projections. This facade owns
// only JavaScript identity/caching policy and the optional Apache Arrow JS
// materialization step; it never switches on the datatype kind to rebuild
// core schema behavior.
const { arrowScalarFromIPC } = require('./values.js')

function installDefaults({ DataType, Field, NativeDataType, NativeField }) {
  const dataTypeDefault = NativeDataType.prototype._defaultJSValueNative
  const dataTypeDefaultHint = NativeDataType.prototype._defaultJSHintNative
  const fieldDefault = NativeField.prototype._defaultJSValueNative
  const dataTypeArrowDefault = NativeDataType.prototype._defaultArrowScalarIpcNative
  const fieldArrowDefault = NativeField.prototype._defaultArrowScalarIpcNative

  for (const [name, method] of [
    ['DataType._defaultJSValueNative', dataTypeDefault],
    ['DataType._defaultJSHintNative', dataTypeDefaultHint],
    ['Field._defaultJSValueNative', fieldDefault],
    ['DataType._defaultArrowScalarIpcNative', dataTypeArrowDefault],
    ['Field._defaultArrowScalarIpcNative', fieldArrowDefault],
  ]) {
    if (typeof method !== 'function') {
      throw new TypeError(`native binding is missing ${name}`)
    }
  }

  delete NativeDataType.prototype._defaultJSValueNative
  delete NativeDataType.prototype._defaultJSHintNative
  delete NativeField.prototype._defaultJSValueNative
  delete NativeDataType.prototype._defaultArrowScalarIpcNative
  delete NativeField.prototype._defaultArrowScalarIpcNative

  const dataTypeEntries = new WeakMap()
  const fieldEntries = new WeakMap()

  function dataTypeEntry(dataType) {
    let entry = dataTypeEntries.get(dataType)
    if (entry === undefined) {
      entry = {
        hint: undefined,
      }
      dataTypeEntries.set(dataType, entry)
    }
    return entry
  }

  function fieldEntry(field) {
    let entry = fieldEntries.get(field)
    if (entry === undefined) {
      entry = {
        hint: undefined,
        hintDataType: undefined,
        hintNullable: undefined,
      }
      fieldEntries.set(field, entry)
    }
    return entry
  }

  // Indexed by the native JsValueHint discriminant. A struct projects as an
  // array and a union as a plain `{ typeId, value }` object, so only 8 is
  // unused rather than reserved for a record class.
  const hintConstructors = Object.freeze([
    null,
    Boolean,
    Number,
    BigInt,
    Array,
    Buffer,
    String,
    Object,
    undefined,
    Map,
  ])

  function frozenHint(dataType, nullable) {
    const code = Reflect.apply(dataTypeDefaultHint, dataType, [])
    const constructor = hintConstructors[code]
    if (constructor === undefined) {
      throw new TypeError(`native binding returned unknown JS hint category ${code}`)
    }
    return Object.freeze({
      kind: dataType.kind,
      constructor,
      nullable,
    })
  }

  Object.defineProperties(DataType.prototype, {
    defaultJSValue: {
      configurable: true,
      value() {
        return Reflect.apply(dataTypeDefault, this, [])
      },
    },
    defaultJSHint: {
      configurable: true,
      value() {
        const entry = dataTypeEntry(this)
        return (entry.hint ??= frozenHint(this, false))
      },
    },
    defaultArrowScalar: {
      configurable: true,
      value() {
        return arrowScalarFromIPC(
          Reflect.apply(dataTypeArrowDefault, this, []),
          `DataType(${this}) default Arrow scalar`,
        )
      },
    },
  })

  Object.defineProperties(Field.prototype, {
    defaultJSValue: {
      configurable: true,
      value() {
        return Reflect.apply(fieldDefault, this, [])
      },
    },
    defaultJSHint: {
      configurable: true,
      value() {
        const entry = fieldEntry(this)
        const dataType = this.dataType
        const nullable = this.nullable
        if (
          entry.hint === undefined ||
          entry.hintNullable !== nullable ||
          !dataType.equals(entry.hintDataType, false)
        ) {
          entry.hintDataType = dataType
          entry.hintNullable = nullable
          entry.hint = frozenHint(dataType, nullable)
        }
        return entry.hint
      },
    },
    defaultArrowScalar: {
      configurable: true,
      value() {
        return arrowScalarFromIPC(
          Reflect.apply(fieldArrowDefault, this, []),
          `Field(${this.name}) default Arrow scalar`,
        )
      },
    },
  })
}

module.exports = { installDefaults }
