import {
  DataType,
  Field,
  fields,
  type JSValueHint,
} from '..'

const nullableId = fields.int64('id', { nullable: true })
const nullableValue: bigint | null = nullableId.defaultJSValue()
const dtypeValue: bigint = nullableId.dtype.defaultJSValue()
const idHint: JSValueHint<'int64', bigint> = nullableId.defaultJSHint()
// A hint reports the coarse family, not the variant that produced it.
const idKind: 'integer' = idHint.kind
const idConstructor: Function | null = idHint.constructor
const idNullable: boolean = idHint.nullable

const nullField = fields.null('nothing', { nullable: true })
const nullValue: null = nullField.defaultJSValue()
const nullDtypeValue: null = nullField.dtype.defaultJSValue()

const item = fields.struct(
  'item',
  [
    fields.int32('quantity', { nullable: false }),
    fields.utf8('sku', { nullable: true }),
  ],
  { nullable: false },
)
// A non-null Struct field is itself the schema, so its default is the ordered
// positional tuple of child defaults rather than a separate record value.
const itemValue: readonly [number, string | null] = item.defaultJSValue()
const quantity: number = itemValue[0]
const sku: string | null = itemValue[1]
const itemChildren: Field[] = [...item.dtype]

const fixed = fields.fixedSizeList('items', item, 2, { nullable: false })
const fixedDefault = fixed.defaultJSValue()
const nestedQuantity: number = fixedDefault[0][0]

// The factories default to nullable to match Python, so an unqualified Struct
// projects the null case and every unqualified child slot is nullable too.
const defaultedItem = fields.struct('defaulted_item', [fields.int32('quantity')])
const defaultedItemValue: readonly [number | null] | null =
  defaultedItem.defaultJSValue()

const genericDtype = new DataType('int32')
const unknownDtypeDefault: unknown = genericDtype.defaultJSValue()
const genericField = new Field('value', genericDtype, false)
const unknownFieldDefault: unknown = genericField.defaultJSValue()
const arrowScalar: unknown = fixed.defaultArrowScalar()
const sparkDtype: DataType = fixed.dtype.intoSchemeCompat('spark')
const arrowField: Field = fixed.intoSchemeCompat('arrow')

// @ts-expect-error nullable Field defaults are not narrowed to bigint
const nonNullableFieldValue: bigint = nullableId.defaultJSValue()
// @ts-expect-error a DataType default is non-null for non-Null datatypes
const nullableDtypeValue: null = nullableId.dtype.defaultJSValue()
// @ts-expect-error a defaulted Struct is nullable, so its default is not a bare tuple
const nonNullableItemValue: readonly [number | null] = defaultedItem.defaultJSValue()
// @ts-expect-error compatibility targets are a closed vocabulary
fixed.intoSchemeCompat('postgres')
// @ts-expect-error private native defaults bridges are not public declarations
fixed._defaultJSValueNative()
// @ts-expect-error private native hint-category bridge is hidden
fixed.dtype._defaultJSHintNative()
// @ts-expect-error private native Arrow bridge is hidden
fixed.dtype._defaultArrowScalarIpcNative()

void nullableValue
void dtypeValue
void idKind
void idConstructor
void idNullable
void nullValue
void nullDtypeValue
void quantity
void sku
void itemChildren
void nestedQuantity
void defaultedItemValue
void unknownDtypeDefault
void unknownFieldDefault
void arrowScalar
void sparkDtype
void arrowField
void nonNullableFieldValue
void nonNullableItemValue
