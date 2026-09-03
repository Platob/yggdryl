import { Vector as ArrowVector } from 'apache-arrow'

import { AsciiDictionary, DataType, Field } from '..'

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

const currencies = new AsciiDictionary('ascii32')
const seededCurrencies: AsciiDictionary = AsciiDictionary.fromValues(
  DataType.ascii(3),
  ['USD', 'EUR'],
  'int64',
)
const currencyCode: number = currencies.push('USD')
const currencyValue: string | null = currencies.get(0)
const currencyLookup: number | null = currencies.getCode('USD')
const currencyValues: string[] = currencies.values()
const currencyCount: number = currencies.length
const currencyDtype: DataType = currencies.dtype
const currencyKey: DataType = currencies.key
const currencyWidth: DataType = currencies.valuesDtype
const currencyEquals: boolean = currencies.equals(seededCurrencies)
const currencyText: string = currencies.toString()
const currencyClone: AsciiDictionary = currencies.clone()
const currencyEnum: Readonly<Record<string, number>> = currencies.intoEnum('Currency')
const currencyColumn: ArrowVector = currencies.intoArrowArray(['USD', null])
const recoveredCurrencies: AsciiDictionary =
  AsciiDictionary.fromArrowArray(currencyColumn)

void currencyCode
void currencyValue
void currencyLookup
void currencyValues
void currencyCount
void currencyDtype
void currencyKey
void currencyWidth
void currencyEquals
void currencyText
void currencyClone
void currencyEnum
void recoveredCurrencies
