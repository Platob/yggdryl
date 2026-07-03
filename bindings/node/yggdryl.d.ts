// Type declarations for the package entry (`yggdryl.js`): the per-crate
// namespaces over the generated native declarations (`index.d.ts`), aliasing the
// prefixed native class names (`DtypeInt64`, `FieldInt64`, `ScalarInt64`, ...)
// to their bare names inside the `dtype` / `field` / `scalar` namespaces —
// mirroring the crate tree with the same names as Rust and Python.

import * as native from './index'

export import core = native.core

export namespace dtype {
  export import Union = native.DtypeUnion
  export import Null = native.DtypeNull
  export import Binary = native.DtypeBinary
  export import OptionalBinary = native.DtypeOptionalBinary
  export import Int8 = native.DtypeInt8
  export import OptionalInt8 = native.DtypeOptionalInt8
  export import Int16 = native.DtypeInt16
  export import OptionalInt16 = native.DtypeOptionalInt16
  export import Int32 = native.DtypeInt32
  export import OptionalInt32 = native.DtypeOptionalInt32
  export import Int64 = native.DtypeInt64
  export import OptionalInt64 = native.DtypeOptionalInt64
  export import UInt8 = native.DtypeUInt8
  export import OptionalUInt8 = native.DtypeOptionalUInt8
  export import UInt16 = native.DtypeUInt16
  export import OptionalUInt16 = native.DtypeOptionalUInt16
  export import UInt32 = native.DtypeUInt32
  export import OptionalUInt32 = native.DtypeOptionalUInt32
  export import UInt64 = native.DtypeUInt64
  export import OptionalUInt64 = native.DtypeOptionalUInt64
}

export namespace field {
  export import Union = native.FieldUnion
  export import Null = native.FieldNull
  export import Binary = native.FieldBinary
  export import OptionalBinary = native.FieldOptionalBinary
  export import Int8 = native.FieldInt8
  export import OptionalInt8 = native.FieldOptionalInt8
  export import Int16 = native.FieldInt16
  export import OptionalInt16 = native.FieldOptionalInt16
  export import Int32 = native.FieldInt32
  export import OptionalInt32 = native.FieldOptionalInt32
  export import Int64 = native.FieldInt64
  export import OptionalInt64 = native.FieldOptionalInt64
  export import UInt8 = native.FieldUInt8
  export import OptionalUInt8 = native.FieldOptionalUInt8
  export import UInt16 = native.FieldUInt16
  export import OptionalUInt16 = native.FieldOptionalUInt16
  export import UInt32 = native.FieldUInt32
  export import OptionalUInt32 = native.FieldOptionalUInt32
  export import UInt64 = native.FieldUInt64
  export import OptionalUInt64 = native.FieldOptionalUInt64
}

export namespace scalar {
  export import Null = native.ScalarNull
  export import Binary = native.ScalarBinary
  export import OptionalBinary = native.ScalarOptionalBinary
  export import Int8 = native.ScalarInt8
  export import OptionalInt8 = native.ScalarOptionalInt8
  export import Int16 = native.ScalarInt16
  export import OptionalInt16 = native.ScalarOptionalInt16
  export import Int32 = native.ScalarInt32
  export import OptionalInt32 = native.ScalarOptionalInt32
  export import Int64 = native.ScalarInt64
  export import OptionalInt64 = native.ScalarOptionalInt64
  export import UInt8 = native.ScalarUInt8
  export import OptionalUInt8 = native.ScalarOptionalUInt8
  export import UInt16 = native.ScalarUInt16
  export import OptionalUInt16 = native.ScalarOptionalUInt16
  export import UInt32 = native.ScalarUInt32
  export import OptionalUInt32 = native.ScalarOptionalUInt32
  export import UInt64 = native.ScalarUInt64
  export import OptionalUInt64 = native.ScalarOptionalUInt64
}
