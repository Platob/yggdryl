import { DataType, Field } from '..'

const type = DataType.from('struct<id: bigint not null>')
const clonedType: DataType = DataType.from(type)
const child: Field | null = type.get('id')
const indexedChild: Field | null = type.get(0)
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

