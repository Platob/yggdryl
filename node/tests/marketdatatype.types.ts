import {
  MarketDataType,
  fields,
  marketDataTypeFromFix,
  type MarketDataTypeField,
  type MarketDataTypeName,
} from '..'

const code: number = MarketDataType.ORDLIMIT
const name: MarketDataTypeName = 'QUOOTHER'
const field: MarketDataTypeField = fields.marketdatatype('type', { nullable: false })
const read: MarketDataTypeName | null = marketDataTypeFromFix(40, '2')

// @ts-expect-error a member is read-only
MarketDataType.ORDLIMIT = 7
// @ts-expect-error no member goes by that name
const missing: number = MarketDataType.NOT_A_TYPE

void [code, name, field, read, missing]
