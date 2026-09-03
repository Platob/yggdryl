import { DataType, Field } from '..'

const type = DataType.from('struct<id: bigint not null>')
const clonedType: DataType = DataType.from(type)
const child: Field | null = type.getField('id')
const indexedChild: Field | null = type.getField(0)
const pathChild: Field | null = type.getFieldByPath('id')
const positionalChild: Field = type.fieldAt(0)
const raisingChild: Field = type.field('id')
void pathChild
void positionalChild
void raisingChild
const children: Field[] = [...type]
const typeHash: bigint = clonedType.stableHash()
const typeJson: unknown = type.toJSON()
const arrowType: DataType = DataType.fromArrow({
  toString: () => type.toString(),
})

// The parenthesis disambiguates: a bare variant() is the Variant datatype
// with its own literal id, and variant(fields) stays the dense-union sugar.
const bareVariant = DataType.variant()
const bareVariantId: 'variant' = bareVariant.id
const bareVariantKind: 'variant' = bareVariant.kind
const geometryType: DataType = DataType.geometry()
const projectedGeometry: DataType = DataType.geometry('EPSG:3857')
const geographyType: DataType = DataType.geography()
const vincentyGeography: DataType = DataType.geography('OGC:CRS84', 'vincenty')
const asciiType: DataType = DataType.ascii(3)
const asciiWidth: number | null = asciiType.asciiWidth
const currencyType: DataType = DataType.fromLogicalName('currency')
const logicalNames: Record<string, DataType> = DataType.logicalNames()

void child
void indexedChild
void children
void typeHash
void typeJson
void arrowType
void bareVariantId
void bareVariantKind
void geometryType
void projectedGeometry
void geographyType
void vincentyGeography
void asciiWidth
void currencyType
void logicalNames
