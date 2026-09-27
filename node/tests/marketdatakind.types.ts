import {
  MarketDataKind,
  fields,
  type MarketDataKindField,
  type MarketDataKindName,
} from '..'

const code: number = MarketDataKind.ORDR
const name: MarketDataKindName = 'QUOT'
const trade: 21 = MarketDataKind.TRAD
const field: MarketDataKindField = fields.marketdatakind('kind', { nullable: false })

// @ts-expect-error a member is read-only
MarketDataKind.ORDR = 7
// @ts-expect-error no member goes by that name
const missing: number = MarketDataKind.NOT_A_KIND

void [code, name, trade, field, missing]
