'use strict'

function metadataEntry(key, value) {
  if (typeof key !== 'string' || typeof value !== 'string') {
    throw new TypeError('field metadata keys and values must be strings')
  }
  return { key, value }
}

function normalizeMetadata(values) {
  if (values === undefined) return undefined
  if (values instanceof Map) {
    return Array.from(values, ([key, value]) => metadataEntry(key, value))
  }
  if (Array.isArray(values)) {
    return values.map((entry) => {
      if (Array.isArray(entry)) {
        if (entry.length !== 2) {
          throw new TypeError('field metadata tuples must contain two items')
        }
        return metadataEntry(entry[0], entry[1])
      }
      if (entry === null || typeof entry !== 'object') {
        throw new TypeError('field metadata entries must be objects or tuples')
      }
      return metadataEntry(entry.key, entry.value)
    })
  }
  if (isOptions(values)) {
    return Object.entries(values).map(([key, value]) => metadataEntry(key, value))
  }
  throw new TypeError('field metadata must be an object, Map, or entry array')
}

function isOptions(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return false
  }
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

// Datatype-specific factories are a language-level typing convenience. They
// return the one native Field class and call captured direct native datatype
// constructors, so nested children are never formatted and reparsed. The
// loader removes those internal constructors from the public class.
function createFields(DataType, Field, native) {
  const simpleTypes = new Map()

  function simpleType(kind) {
    let value = simpleTypes.get(kind)
    if (value === undefined) {
      value = native.simple(kind)
      simpleTypes.set(kind, value)
    }
    return value
  }

  // Fields are nullable unless the caller says otherwise, which is what the
  // Python factories and the native Field constructor already do. A helper
  // that defaulted the other way would give the same declared schema a
  // different default value in each language.
  function options(value) {
    if (value === undefined) return { nullable: true, metadata: undefined }
    if (!isOptions(value)) {
      throw new TypeError('field options must be a plain object')
    }
    const nullable = value.nullable === undefined ? true : value.nullable
    if (typeof nullable !== 'boolean') {
      throw new TypeError('field options nullable must be a boolean')
    }
    return { nullable, metadata: normalizeMetadata(value.metadata) }
  }

  function field(name, dataType, value) {
    const { nullable, metadata } = options(value)
    return new Field(name, dataType, nullable, metadata)
  }

  function temporalField(kind, fallback, name, unit, value) {
    if (isOptions(unit)) {
      value = unit
      unit = fallback
    }
    return field(name, native.temporal(kind, unit), value)
  }

  function decimalField(kind, name, precision, scale, value) {
    if (isOptions(scale)) {
      value = scale
      scale = 0
    }
    return field(name, native.decimal(kind, precision, scale), value)
  }

  function simple(kind) {
    return (name, value) => field(name, simpleType(kind), value)
  }

  function list(kind) {
    return (name, item, value) =>
      field(name, native.list(kind, item), value)
  }

  const fields = {
    null: simple('null'),
    boolean: simple('boolean'),
    int8: simple('int8'),
    int16: simple('int16'),
    int32: simple('int32'),
    int64: simple('int64'),
    uint8: simple('uint8'),
    uint16: simple('uint16'),
    uint32: simple('uint32'),
    uint64: simple('uint64'),
    float16: simple('float16'),
    float32: simple('float32'),
    float64: simple('float64'),

    timestamp(name, unit = 'microsecond', timezone, value) {
      // Omitting timezone makes the third argument the options object.
      if (isOptions(timezone)) {
        value = timezone
        timezone = undefined
      }
      return field(
        name,
        native.temporal('timestamp', unit, timezone),
        value,
      )
    },
    date32: simple('date32'),
    date64: simple('date64'),
    time(name, unit, value) {
      return field(name, DataType.time(unit), value)
    },
    time32(name, unit = 'millisecond', value) {
      return temporalField('time32', 'millisecond', name, unit, value)
    },
    time64(name, unit = 'microsecond', value) {
      return temporalField('time64', 'microsecond', name, unit, value)
    },
    duration(name, unit = 'microsecond', value) {
      return temporalField('duration', 'microsecond', name, unit, value)
    },
    interval(name, unit = 'month_day_nano', value) {
      return temporalField('interval', 'month_day_nano', name, unit, value)
    },

    binary: simple('binary'),
    fixedSizeBinary(name, byteWidth, value) {
      return field(name, native.fixedSizeBinary(byteWidth), value)
    },
    largeBinary: simple('large_binary'),
    binaryView: simple('binary_view'),
    utf8: simple('utf8'),
    largeUtf8: simple('large_utf8'),
    utf8View: simple('utf8_view'),

    list: list('list'),
    listView: list('list_view'),
    fixedSizeList(name, item, length, value) {
      return field(
        name,
        native.list('fixed_size_list', item, length),
        value,
      )
    },
    largeList: list('large_list'),
    largeListView: list('large_list_view'),
    struct(name, children, value) {
      return field(name, DataType.fromFields(children), value)
    },
    union(name, members, mode = 'sparse', value) {
      if (isOptions(mode)) {
        value = mode
        mode = 'sparse'
      }
      const collected = Array.from(members)
      return field(
        name,
        native.union(
          collected.map((member) => member[0]),
          collected.map((member) => member[1]),
          mode,
        ),
        value,
      )
    },
    denseUnion(name, members, value) {
      return field(name, DataType.variant(members), value)
    },
    variant(name, value) {
      return field(name, DataType.variant(), value)
    },
    geometry(name, crs, value) {
      if (isOptions(crs)) {
        value = crs
        crs = undefined
      }
      return field(name, DataType.geometry(crs), value)
    },
    geography(name, crs, algorithm, value) {
      if (isOptions(crs)) {
        value = crs
        crs = undefined
        algorithm = undefined
      } else if (isOptions(algorithm)) {
        value = algorithm
        algorithm = undefined
      }
      return field(name, DataType.geography(crs, algorithm), value)
    },
    dictionary(name, key, encodedValue, value) {
      return field(
        name,
        native.dictionary(key, encodedValue),
        value,
      )
    },

    decimal(name, precision, scale = 0, value) {
      return decimalField('decimal', name, precision, scale, value)
    },
    decimal32(name, precision, scale = 0, value) {
      return decimalField('decimal32', name, precision, scale, value)
    },
    decimal64(name, precision, scale = 0, value) {
      return decimalField('decimal64', name, precision, scale, value)
    },
    decimal128(name, precision, scale = 0, value) {
      return decimalField('decimal128', name, precision, scale, value)
    },
    decimal256(name, precision, scale = 0, value) {
      return decimalField('decimal256', name, precision, scale, value)
    },

    map(name, entries, keysSorted = false, value) {
      if (isOptions(keysSorted)) {
        value = keysSorted
        keysSorted = false
      }
      return field(name, native.map(entries, keysSorted), value)
    },
    mapOf(name, key, mappedValue, keysSorted = false, value) {
      if (isOptions(keysSorted)) {
        value = keysSorted
        keysSorted = false
      }
      return field(name, native.mapOf(key, mappedValue, keysSorted), value)
    },
    runEndEncoded(name, runEnds, values, value) {
      return field(
        name,
        native.runEndEncoded(runEnds, values),
        value,
      )
    },
  }

  return Object.freeze(fields)
}

module.exports = { createFields, normalizeMetadata }
